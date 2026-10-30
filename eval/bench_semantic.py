"""A second model for the first pass (PLAN §9 decision 2: keywords and semantic search): CLIP / SigLIP 2 image
encoders on the RTX 2060, torch fp16 vs TensorRT fp16, and how faithful TensorRT is to torch fp32.

Usage: python bench_semantic.py [--models clip_b32 siglip2_b16 ...] [--batch 64] [--iters 30] [--images 128]

Per model: image-encoder parameters, GPU-only throughput (synthetic batch) under torch fp16 autocast and as a
TensorRT 11 fp16 engine (accel.build_engine; ONNX exported from the fp16 encoder), and the cosine between the
TensorRT and torch fp32 embeddings of --images real images (open_clip's own preprocessing). Quality is the
published zero-shot ImageNet-1k top-1, as there are no keyword labels here: it ranks models, not this
library. Weights download to the cache dir (the SO400M model is ~4.5 GB). Writes WORK_DIR/bench_semantic.json.
"""

import argparse
import json
import random
import time

import numpy as np
import torch

import accel
from common import CACHE_DIR, DATA_ROOT, WORK_DIR, image_list, load_rgb

# name: (open_clip architecture, pretrained tag, published zero-shot ImageNet-1k top-1 %, source of that number)
CANDIDATES = {
    "clip_b32": ("ViT-B-32", "laion2b_s34b_b79k", 66.6, "open_clip results table"),
    "siglip2_b32_256": ("ViT-B-32-SigLIP2-256", "webli", 74.0, "SigLIP 2 paper, Table 1"),
    "siglip2_b16": ("ViT-B-16-SigLIP2", "webli", 78.2, "SigLIP 2 paper, Table 1"),
    "siglip2_l16_256": ("ViT-L-16-SigLIP2-256", "webli", 82.5, "SigLIP 2 paper, Table 1"),
    "siglip2_so400m14": ("ViT-SO400M-14-SigLIP2", "webli", 83.2, "SigLIP 2 paper, Table 1"),
}


def load(arch: str, pretrained: str):
    import open_clip

    model, _, preprocess = open_clip.create_model_and_transforms(arch, pretrained=pretrained)
    visual = model.visual.eval()
    del model  # the text tower isn't needed
    return visual, preprocess


def image_size(visual) -> int:
    size = visual.image_size
    return size[0] if isinstance(size, (tuple, list)) else int(size)


def autocast_fp16(model):
    def run(t: torch.Tensor) -> torch.Tensor:
        with torch.autocast("cuda", dtype=torch.float16):
            return model(t).float()

    return run


def throughput(fn, x: torch.Tensor, iters: int) -> float:
    with torch.inference_mode():
        for _ in range(3):
            fn(x)
        torch.cuda.synchronize()
        start = time.perf_counter()
        for _ in range(iters):
            fn(x)
        torch.cuda.synchronize()
    return len(x) * iters / (time.perf_counter() - start)


def engine(name: str, arch: str, pretrained: str, batch: int) -> tuple[accel.TRTRunner, int]:
    """TensorRT fp16 engine of the image encoder (exported and built on first use, cached)."""
    plan = accel.TRT_DIR / f"{name}_fp16_b{batch}_trt{accel._trt_major()}.plan"
    onnx = accel.ONNX_DIR / f"{name}_fp16.onnx"
    visual, _ = load(arch, pretrained)
    size = image_size(visual)
    if not plan.exists():
        if not onnx.exists():
            accel.ONNX_DIR.mkdir(parents=True, exist_ok=True)
            visual = visual.half().cuda()
            x = torch.zeros(2, 3, size, size, dtype=torch.float16, device="cuda")
            torch.onnx.export(visual, (x,), str(onnx), input_names=["x"], output_names=["y"], opset_version=17,
                              dynamic_axes={"x": {0: "batch"}, "y": {0: "batch"}}, dynamo=False)
        del visual
        torch.cuda.empty_cache()  # the build needs the GPU memory
        accel.build_engine(onnx, plan, ((1, 3, size, size), (batch, 3, size, size), (batch, 3, size, size)))
    return accel.TRTRunner(plan), size


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--models", nargs="+", choices=list(CANDIDATES), default=list(CANDIDATES))
    parser.add_argument("--batch", type=int, default=64)
    parser.add_argument("--iters", type=int, default=30)
    parser.add_argument("--images", type=int, default=128)
    args = parser.parse_args()

    files = random.Random(4).sample(image_list(), args.images)
    out_path = WORK_DIR / "bench_semantic.json"
    report = json.loads(out_path.read_text(encoding="utf-8")) if out_path.exists() else {}
    for name in args.models:
        arch, pretrained, in1k, source = CANDIDATES[name]
        trt_model, size = engine(name, arch, pretrained, args.batch)  # first: building needs free GPU memory
        visual, preprocess = load(arch, pretrained)
        params = sum(p.numel() for p in visual.parameters()) / 1e6
        visual = visual.cuda()
        x = torch.stack([preprocess(load_rgb(DATA_ROOT / f)) for f in files]).cuda()
        synthetic = torch.randn(args.batch, 3, size, size, device="cuda")
        torch_fp16 = autocast_fp16(visual)

        with torch.inference_mode():
            ref = torch.nn.functional.normalize(torch.cat([visual(x[s:s + 32]) for s in range(0, len(x), 32)]), dim=1)
            got = {"torch_fp16": torch.cat([torch_fp16(x[s:s + 32]) for s in range(0, len(x), 32)]),
                   "trt_fp16": torch.cat([trt_model(x[s:s + 32]) for s in range(0, len(x), 32)])}
        cosine = {k: (torch.nn.functional.normalize(v, dim=1) * ref).sum(1).cpu().numpy() for k, v in got.items()}
        rates = {"torch_fp16": throughput(torch_fp16, synthetic, args.iters),
                 "trt_fp16": throughput(trt_model, synthetic, args.iters)}
        report[name] = {"arch": arch, "pretrained": pretrained, "size": size, "params_m": round(params, 1),
                        "imagenet_zero_shot": in1k, "imagenet_source": source,
                        "img_s": {k: round(v) for k, v in rates.items()},
                        "cos_vs_fp32": {k: {"min": float(c.min()), "p1": float(np.percentile(c, 1))}
                                        for k, c in cosine.items()}}
        out_path.write_text(json.dumps(report, indent=1), encoding="utf-8")
        print(f"{name}: {report[name]}", flush=True)
        del visual, trt_model, x, synthetic
        torch.cuda.empty_cache()

    print(f"\n## Image encoders on the RTX 2060 (batch {args.batch}, GPU only); cache: {CACHE_DIR}")
    print("| model | input | params (M) | ImageNet zero-shot | torch fp16 img/s | TensorRT fp16 img/s "
          "| GPU h for 70M (TensorRT) | TensorRT vs fp32 cosine min |")
    print("|---|---|---|---|---|---|---|---|")
    for name, r in report.items():
        trt_rate = r["img_s"]["trt_fp16"]
        print(f"| {name} | {r['size']} px | {r['params_m']:.0f} | {r['imagenet_zero_shot']:.1f}% "
              f"| {r['img_s']['torch_fp16']:,} | {trt_rate:,} | {70e6 / trt_rate / 3600:.0f} "
              f"| {r['cos_vs_fp32']['trt_fp16']['min']:.4f} |")


if __name__ == "__main__":
    main()
