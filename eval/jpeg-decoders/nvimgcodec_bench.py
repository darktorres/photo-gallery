"""nvImageCodec (NVIDIA's codec library over nvJPEG) for docs/JPEG_DECODERS.md: one image at a time per backend, and
batches, each with the pixels copied to host memory; CPU time per image; output against libjpeg-turbo.

Run with its own venv (python -m pip install -r eval/jpeg-decoders/requirements.txt; bin/python on
Linux), not the shared eval venv:
    $JPEG_EVAL/venv/Scripts/python.exe eval/jpeg-decoders/nvimgcodec_bench.py \
        --ref-dir DIR FILE...
DIR holds the harness's libjpeg-turbo decodes (`harness dump turbo FILE DIR/NAME.ppm`), one per FILE.
Timing as the harness: warm-up, then samples until 5 and the budget; the median, and process CPU time per image.
"""

import argparse
import ctypes
import statistics
import time
from pathlib import Path

import env
import numpy as np
from PIL import Image

BUDGET_S = 2.0
MAX_BATCH_BYTES = 2.0e9  # decoded RGB per batch: the 2060 has 6 GB, ~1.5 GB taken by the desktop
CU_CTX_SCHED_BLOCKING_SYNC = 0x04  # cudaDeviceScheduleBlockingSync: a synchronize() sleeps instead of spinning


def backends(nv) -> dict:
    """The decoders to time: nvImageCodec's default (None), and each backend kind. Takes the imported module, which
    main() imports only after blocking_sync()."""
    return {
        "default": None,
        "GPU_ONLY": nv.BackendKind.GPU_ONLY,
        "HYBRID_CPU_GPU": nv.BackendKind.HYBRID_CPU_GPU,
        "CPU_ONLY": nv.BackendKind.CPU_ONLY,
    }


def blocking_sync() -> None:
    """Make the primary context of device 0 block on synchronize(), so waiting for the GPU doesn't bill CPU time.
    Must run before the first CUDA call. Done through the driver's DLL: this venv has no torch (see gpu.py)."""
    cuda = ctypes.CDLL("nvcuda.dll")
    if (rc := cuda.cuInit(0)) != 0:
        raise RuntimeError(f"cuInit returned {rc}")
    if (rc := cuda.cuDevicePrimaryCtxSetFlags(0, CU_CTX_SCHED_BLOCKING_SYNC)) != 0:
        raise RuntimeError(f"cuDevicePrimaryCtxSetFlags returned {rc} (the context is already active?)")


def read_ppm(path):
    """The libjpeg-turbo reference: uint8, shape (h, w, 3)."""
    return np.ascontiguousarray(np.asarray(Image.open(path)))


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
    blocking_sync()
    # Imported only now: if importing the library creates the CUDA primary context, the flags above must already be set
    # (cuDevicePrimaryCtxSetFlags fails on an active context).
    from nvidia import nvimgcodec as nv

    decoders = {}
    for name, kind in backends(nv).items():
        try:
            decoders[name] = nv.Decoder() if kind is None else nv.Decoder(backends=[nv.Backend(kind)])
        except Exception as e:  # noqa: BLE001 (reported, and the other backends still run)
            print(f"{name:15} cannot be created: {e}")
    print(env.line())
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
                decoded = dec.decode(data)
                out = None if decoded is None else np.asarray(decoded.cpu())
            except Exception as e:  # noqa: BLE001 (reported, and the run goes on)
                print(f"{f.name:34} {name:15} error: {e}")
                continue
            if out is None:
                print(f"{f.name:34} {name:15} error: returned no image")
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
