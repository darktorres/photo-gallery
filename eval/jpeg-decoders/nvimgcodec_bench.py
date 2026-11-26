"""nvImageCodec (NVIDIA's codec library over nvJPEG): one image at a time per backend, and
batches, each with the pixels copied to host memory; CPU time per image; output against libjpeg-turbo.

Run with its own venv (python -m pip install -r eval/jpeg-decoders/requirements.txt; bin/python on
Linux), not the shared eval venv:
    $PG_JPEG_EVAL/venv/Scripts/python.exe eval/jpeg-decoders/nvimgcodec_bench.py \
        --ref-dir DIR FILE...
DIR holds the harness's libjpeg-turbo decodes (`harness dump turbo FILE DIR/NAME.ppm`), one per FILE.
Timing as the harness: warm-up, then samples until 5 and the budget; the median, and process CPU time per image.
"""

import argparse
from functools import partial
from pathlib import Path

import env
import numpy as np
from reference_io import read_ppm
from timing import BUDGET_S, blocking_sync, max_batch, timed_s


def backends(nv) -> dict:
    """The decoders to time: nvImageCodec's default (None), and each backend kind. Takes the imported module, which
    main() imports only after blocking_sync()."""
    return {
        "default": None,
        "GPU_ONLY": nv.BackendKind.GPU_ONLY,
        "HW_GPU_ONLY": nv.BackendKind.HW_GPU_ONLY,  # the JPEG engine: not on Turing, so it reports "cannot be created"
        "HYBRID_CPU_GPU": nv.BackendKind.HYBRID_CPU_GPU,
        "CPU_ONLY": nv.BackendKind.CPU_ONLY,
    }


def decode_one(dec, data):
    """One image, decoded and copied to the host (the timed call of a batch of one)."""
    return dec.decode(data).cpu()


def decode_batch(dec, batch):
    """A batch, decoded and each image copied to the host."""
    return [i.cpu() for i in dec.decode(batch)]


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
        f"nvImageCodec {nv.__version__}: median wall ms/image (pixels on the host), CPU ms/image, "
        "output vs libjpeg-turbo (mean |diff| / max / PSNR)"
    )
    for f in args.files:
        data = f.read_bytes()
        ref = read_ppm(args.ref_dir / (f.stem + ".ppm"))
        h, w = ref.shape[:2]
        cap = max_batch(w, h)
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
                fn = partial(decode_one, dec, data) if b == 1 else partial(decode_batch, dec, [data] * b)

                try:
                    wall, cpu = timed_s(fn, b, args.secs)
                except Exception as e:  # noqa: BLE001 (reported, and the run goes on)
                    print(f"{f.name:34} {name:15} batch {b:3}: error: {e}")
                    continue
                print(
                    f"{f.name:34} {name:15} batch {b:3}: {wall * 1e3:9.3f} ms {cpu * 1e3:9.3f} cpu ms   {quality}",
                    flush=True,
                )


if __name__ == "__main__":
    main()
