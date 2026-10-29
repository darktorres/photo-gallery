"""Download each candidate model and measure pure GPU throughput (no decoding).

Usage: python bench_models.py [--models sscd dinov2_s ...] [--batch 64]
The number is an upper bound for ingestion speed: real throughput is
min(this, decode rate, disk rate).
"""

import argparse
import time

import torch

from models import SPECS, load_model, run


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--models", nargs="+", default=list(SPECS))
    parser.add_argument("--batch", type=int, default=64)
    parser.add_argument("--iters", type=int, default=20)
    args = parser.parse_args()

    device = torch.device("cuda")
    print(f"GPU: {torch.cuda.get_device_name(0)}, batch {args.batch}")
    print("| model | input | dim | params (M) | img/s | days for 70M (GPU only) |")
    print("|---|---|---|---|---|---|")
    for name in args.models:
        spec = SPECS[name]
        model = load_model(spec, device)
        params = sum(p.numel() for p in model.parameters()) / 1e6
        x = torch.randn(args.batch, 3, spec.size, spec.size, device=device)
        with torch.inference_mode():
            dim = run(model, spec, x).shape[1]
            for _ in range(3):
                run(model, spec, x)
            torch.cuda.synchronize()
            start = time.perf_counter()
            for _ in range(args.iters):
                run(model, spec, x)
            torch.cuda.synchronize()
        rate = args.batch * args.iters / (time.perf_counter() - start)
        print(f"| {name} | {spec.size}px {spec.precision} | {dim} | {params:.0f} | {rate:.0f} "
              f"| {70e6 / rate / 86400:.1f} |")
        del model
        torch.cuda.empty_cache()


if __name__ == "__main__":
    main()
