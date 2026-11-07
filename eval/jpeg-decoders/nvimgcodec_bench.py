"""nvImageCodec (NVIDIA's codec library over nvJPEG) for docs/JPEG_DECODERS.md: one image at a time per backend, and
batches, each with the pixels copied to host memory; CPU time per image; output against libjpeg-turbo.

Run with its own venv (python -m pip install -r eval/jpeg-decoders/requirements.txt), not the shared eval venv:
    $JPEG_EVAL/venv/Scripts/python.exe -I eval/jpeg-decoders/nvimgcodec_bench.py \
        --ref-dir DIR FILE...
DIR holds the harness's libjpeg-turbo decodes (`harness dump turbo FILE DIR/NAME.ppm`), one per FILE.
Timing as the harness: warm-up, then samples until 5 and the budget; the median, and process CPU time per image.
"""

import argparse
import statistics
import time
from pathlib import Path

import numpy as np
from nvidia import nvimgcodec as nv

BUDGET_S = 2.0
MAX_BATCH_BYTES = 2.0e9  # decoded RGB per batch: the 2060 has 6 GB, ~1.5 GB taken by the desktop
BACKENDS = {
    "default": None,
    "GPU_ONLY": nv.BackendKind.GPU_ONLY,
    "HYBRID_CPU_GPU": nv.BackendKind.HYBRID_CPU_GPU,
    "CPU_ONLY": nv.BackendKind.CPU_ONLY,
}


def read_ppm(path):
    data = path.read_bytes()
    parts = data.split(maxsplit=4)  # P6 W H 255 pixels
    w, h = int(parts[1]), int(parts[2])
    return np.frombuffer(parts[4][: w * h * 3], np.uint8).reshape(h, w, 3)


def timed(fn, per_call, budget):
    for _ in range(3):
        fn()
    samples, c0, t0 = [], time.process_time(), time.perf_counter()
    while len(samples) < 5 or time.perf_counter() - t0 < budget:
        t = time.perf_counter()
        fn()
        samples.append((time.perf_counter() - t) * 1e3 / per_call)
    cpu = (time.process_time() - c0) * 1e3 / (len(samples) * per_call)
    return statistics.median(samples), cpu


def diff(a, ref):
    if a.shape != ref.shape:
        return f"{a.shape[1]}x{a.shape[0]}"
    d = np.abs(a.astype(np.int16) - ref.astype(np.int16))
    mse = float((d.astype(np.float64) ** 2).mean())
    psnr = float("inf") if mse == 0 else 10 * np.log10(255**2 / mse)
    return f"{d.mean():.3f} / {d.max()} / {psnr:.1f} dB"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref-dir", type=Path, required=True)
    ap.add_argument("--batches", default="1,8,64")
    ap.add_argument("--secs", type=float, default=BUDGET_S)
    ap.add_argument("files", nargs="+", type=Path)
    args = ap.parse_args()
    batches = [int(b) for b in args.batches.split(",")]
    decoders = {
        name: nv.Decoder() if kind is None else nv.Decoder(backends=[nv.Backend(kind)]) for name, kind in BACKENDS.items()
    }
    print(
        f"nvImageCodec {nv.__version__}: median wall ms per image (pixels on the host), CPU ms per image, "
        "output vs libjpeg-turbo (mean |diff| / max / PSNR)"
    )
    for f in args.files:
        data = f.read_bytes()
        ref = read_ppm(args.ref_dir / (f.stem + ".ppm"))
        h, w = ref.shape[:2]
        cap = max(1, int(MAX_BATCH_BYTES // (w * h * 3)))
        for name, dec in decoders.items():
            try:
                out = np.asarray(dec.decode(data).cpu())
            except Exception as e:  # noqa: BLE001 (reported, and the run goes on)
                print(f"{f.name:34} {name:15} error: {e}")
                continue
            quality = diff(out, ref)
            for b in [b for b in batches if b <= cap]:
                if b == 1:
                    fn = lambda dec=dec, data=data: dec.decode(data).cpu()  # noqa: E731
                else:
                    batch = [data] * b
                    fn = lambda dec=dec, batch=batch: [i.cpu() for i in dec.decode(batch)]  # noqa: E731
                try:
                    wall, cpu = timed(fn, b, args.secs)
                except Exception as e:  # noqa: BLE001 (reported, and the run goes on)
                    print(f"{f.name:34} {name:15} batch {b:3}: error: {e}")
                    continue
                print(f"{f.name:34} {name:15} batch {b:3}: {wall:9.3f} ms {cpu:9.3f} cpu ms   {quality}", flush=True)


if __name__ == "__main__":
    main()
