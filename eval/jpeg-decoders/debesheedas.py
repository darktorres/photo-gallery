"""debesheedas/GPU-JPEG-Decoder (a course project; 4:4:4 baseline only) for docs/JPEG_DECODERS.md.

It is a program, not a library: one decode per run, written as a text array to ../testing/cuda_decoder_output_arrays
relative to the working directory. So: the median of 3 whole runs, minus the median of 3 runs of the same program on
a 16x16 image (process start, CUDA context, allocation), and its output against libjpeg-turbo.
    python eval/jpeg-decoders/debesheedas.py --exe DECODER.exe --ref-dir DIR FILE...
DIR holds `harness dump turbo FILE DIR/NAME.ppm` for each FILE (NAME = FILE's stem).
"""

import argparse
import shutil
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

import env
import numpy as np
from PIL import Image


def read_ppm(path):
    """The libjpeg-turbo reference: uint8, shape (h, w, 3)."""
    return np.ascontiguousarray(np.asarray(Image.open(path)))


def tiny_jpeg(path):  # a 16x16 4:4:4 baseline JPEG, for the fixed cost
    Image.new("RGB", (16, 16), (128, 64, 32)).save(path, quality=90, subsampling=0)
    return path


def run(exe, jpeg, work):
    t = time.perf_counter()
    subprocess.run([exe, str(jpeg)], cwd=work / "run", check=True, capture_output=True, timeout=600)
    return time.perf_counter() - t


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", required=True)
    ap.add_argument("--ref-dir", type=Path, required=True)
    ap.add_argument("files", nargs="+", type=Path)
    a = ap.parse_args()
    print(env.line())
    # The program runs in a temp folder, so every path it is given must be absolute.
    exe = str(Path(a.exe).resolve())
    ref_dir = a.ref_dir.resolve()
    files = [f.resolve() for f in a.files]
    work = Path(tempfile.mkdtemp())
    try:
        (work / "run").mkdir()
        outdir = work / "testing" / "cuda_decoder_output_arrays"
        outdir.mkdir(parents=True)
        tiny = tiny_jpeg(work / "tiny.jpg")
        base = statistics.median(run(exe, tiny, work) for _ in range(3))
        print(f"fixed cost (16x16 image): {base * 1e3:.0f} ms")
        for f in files:
            ref = read_ppm(ref_dir / (f.stem + ".ppm"))
            try:
                wall = statistics.median(run(exe, f, work) for _ in range(3))
            except subprocess.CalledProcessError as e:
                print(f"{f.name:34} error: exit {e.returncode}")
                continue
            arr = (outdir / (f.stem + ".array")).read_text().split("\n")  # the file name, extension replaced
            h, w = map(int, arr[0].split())
            rgb = np.stack([np.array(arr[i + 1].split(), np.int32).reshape(h, w) for i in range(3)], -1)
            rgb = np.clip(rgb, 0, 255).astype(np.int16)
            if rgb.shape != ref.shape:
                print(f"{f.name:34} {wall * 1e3:9.0f} ms   {w}x{h}")
                continue
            d = np.abs(rgb - ref.astype(np.int16))
            mse = float((d.astype(np.float64) ** 2).mean())
            psnr = float("inf") if mse == 0 else 10 * np.log10(255**2 / mse)
            print(
                f"{f.name:34} {wall * 1e3:9.0f} ms whole run, {(wall - base) * 1e3:9.0f} ms past the fixed cost   "
                f"{d.mean():.3f} / {d.max()} / {psnr:.1f} dB",
                flush=True,
            )
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
