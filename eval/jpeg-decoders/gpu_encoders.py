"""GPU JPEG encoders: nvJPEG (through nvImageCodec) and GPUJPEG (gpujpegtool), from the
PPMs encoders.py prepared (in the container, in the host folder PG_JPEG_ENC_DIR), into its out/ directory, named as
encoders.py names its own outputs. Then encoders.py score rates them with the CPU encoders. Also the encode time per
megapixel at Q75 (in-process for nvJPEG: host pixels in, JPEG bytes on the host out; gpujpegtool's own per-iteration
encode time for GPUJPEG).

    $PG_JPEG_EVAL/venv/Scripts/python.exe eval/jpeg-decoders/gpu_encoders.py --gpujpeg EXE   (bin/python on Linux)
"""

import argparse
import os
import re
import subprocess
import uuid
from pathlib import Path

import env
from encoders import QUALITIES
from nvidia import nvimgcodec as nv
from reference_io import read_ppm
from timing import timed_s

if not os.environ.get("PG_JPEG_ENC_DIR"):
    raise SystemExit("set PG_JPEG_ENC_DIR (.env) to encoders.py's work folder, the one the container mounts")
W = Path(os.environ["PG_JPEG_ENC_DIR"])


def tmp_of(out):
    """A unique temp name beside `out`: an encode writes here and is renamed to `out` only when it has finished,
    so an interrupted run never leaves a truncated file under the name a later run takes as done."""
    return out.with_name(f"{out.name}.{uuid.uuid4().hex}.tmp")


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
                tmp = tmp_of(out)
                tmp.write_bytes(enc.encode(img, "jpeg", params=nv_params(q)))
                tmp.replace(out)
            out = W / "out" / f"{name}.gpujpeg.q{q}.jpg"
            if not out.exists():
                tmp = tmp_of(out)
                subprocess.run(
                    [a.gpujpeg, "-e", "-q", str(q), "--subsampled=4:4:4", "-i", str(ppm), str(tmp)],
                    check=True,
                    capture_output=True,
                )
                tmp.replace(out)
        p = nv_params(75)
        secs, _ = timed_s(lambda img=img, p=p: enc.encode(img, "jpeg", params=p))
        nvms = secs * 1e3 / mp
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
