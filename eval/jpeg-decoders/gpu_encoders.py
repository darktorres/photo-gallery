"""GPU JPEG encoders for docs/JPEG_ENCODERS.md: nvJPEG (through nvImageCodec) and GPUJPEG (gpujpegtool), from the
PPMs encoders.py prepared in WSL, into its out/ directory, named as encoders.py names its own outputs. Then
encoders.py score rates them with the CPU encoders. Also the encode time per megapixel at Q75 (in-process for nvJPEG:
host pixels in, JPEG bytes on the host out; gpujpegtool's own per-iteration encode time for GPUJPEG).

    $JPEG_EVAL/venv/Scripts/python.exe eval/jpeg-decoders/gpu_encoders.py --gpujpeg EXE   (bin/python on Linux)
"""

import argparse
import os
import re
import statistics
import subprocess
import time
from pathlib import Path

import env
import numpy as np
from nvidia import nvimgcodec as nv
from PIL import Image

if not os.environ.get("PG_JPEG_ENC_DIR"):
    raise SystemExit(r"set PG_JPEG_ENC_DIR to the WSL work dir, e.g. \\wsl.localhost\<distro>\root\jpeg-enc")
W = Path(os.environ["PG_JPEG_ENC_DIR"])
QUALITIES = [50, 60, 70, 75, 80, 85, 90, 95]
BUDGET_S = 2.0


def read_ppm(path):
    """The PPM encoders.py wrote: uint8, shape (h, w, 3)."""
    return np.ascontiguousarray(np.asarray(Image.open(path)))


def nv_params(q):
    # 4:4:4 and optimized Huffman tables, baseline: ImageReduce's settings
    jp = nv.JpegEncodeParams(optimized_huffman=True, progressive=False)
    return nv.EncodeParams(
        quality_type=nv.QualityType.QUALITY,
        quality_value=float(q),
        chroma_subsampling=nv.ChromaSubsampling.CSS_444,
        jpeg_encode_params=jp,
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--gpujpeg", required=True, help="gpujpegtool.exe")
    a = ap.parse_args()
    enc = nv.Encoder()
    print(env.line())
    print("encode ms per megapixel at Q75: nvjpeg (in-process, median), gpujpeg (its own average, first iteration excluded)")
    for ppm in sorted((W / "src").glob("*.ppm")):
        name = ppm.stem
        img = read_ppm(ppm)
        mp = img.shape[0] * img.shape[1] / 1e6
        for q in QUALITIES:
            out = W / "out" / f"{name}.nvjpeg.q{q}.jpg"
            if not out.exists():
                out.write_bytes(enc.encode(img, "jpeg", params=nv_params(q)))
            out = W / "out" / f"{name}.gpujpeg.q{q}.jpg"
            if not out.exists():
                subprocess.run(
                    [a.gpujpeg, "-e", "-q", str(q), "--subsampled=4:4:4", "-i", str(ppm), str(out)],
                    check=True,
                    capture_output=True,
                )
        p = nv_params(75)
        for _ in range(3):
            enc.encode(img, "jpeg", params=p)
        ts, t0 = [], time.perf_counter()
        while len(ts) < 5 or time.perf_counter() - t0 < BUDGET_S:
            t = time.perf_counter()
            enc.encode(img, "jpeg", params=p)
            ts.append(time.perf_counter() - t)
        nvms = statistics.median(ts) * 1e3 / mp
        r = subprocess.run(
            [a.gpujpeg, "-v", "-n", "20", "-e", "-q", "75", "--subsampled=4:4:4", "-i", str(ppm), str(W / "speed-gpujpeg.jpg")],
            capture_output=True,
            text=True,
            check=True,
        )
        # its summary (on stderr): the whole encode with the copies to and from the GPU, first iteration excluded
        m = re.search(r"Avg w/o 1st Iter:\s*([\d.]+) ms", r.stdout + r.stderr)
        gj = f"{float(m.group(1)) / mp:.1f}" if m else "?"
        print(f"{name:22} nvjpeg {nvms:7.1f}   gpujpeg {gj:>7}", flush=True)


if __name__ == "__main__":
    main()
