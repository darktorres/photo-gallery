"""Real 16-bit photographs for the PNG decoder comparison: the 16-bit RGB set of imagecompression.info (Sachin Garg,
"The New Image Compression Test Set", Jan 2008; redistribution granted with attribution, see its readme.txt).

    python make_real16.py --zip rgb16bit.zip [--out DIR]    (default: $PG_ROOT\\eval\\png-decoders\\inputs-real16)

The zip holds 16-bit binary PPMs (P6, maxval 65535, big-endian samples). They are read from the zip directly, not
extracted, and written as 16-bit RGB PNGs with make_inputs.py's encoder, so the pixels are the photographs' own
samples. The four chosen span 3.4 to 27.7 MP, the range the other inputs cover, up to a larger photograph.
"""

import argparse
import sys
import zipfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import pg_path  # noqa: E402
from make_inputs import encode  # noqa: E402

CHOSEN = ["flower_foveon", "leaves_iso_200", "nightshot_iso_1600", "big_tree"]


def read_ppm(data: bytes) -> np.ndarray:
    """A P6 binary PPM with maxval 65535: (height, width, 3) uint16."""
    parts = data[:64].split(maxsplit=4)
    if parts[0] != b"P6" or parts[3] != b"65535":
        raise SystemExit(f"not a 16-bit P6 PPM: header {parts[:4]!r}")
    width, height = int(parts[1]), int(parts[2])
    header_len = data.index(b"65535") + len(b"65535") + 1
    body = np.frombuffer(data, dtype=">u2", count=width * height * 3, offset=header_len)
    return body.reshape(height, width, 3).astype(np.uint16)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", type=Path, required=True)
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()
    out = args.out or pg_path(None, "eval", "png-decoders", "inputs-real16")
    out.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.zip) as archive:
        for name in CHOSEN:
            pixels = read_ppm(archive.read(f"{name}.ppm"))
            h, w, _ = pixels.shape
            data = encode(pixels, depth=16)
            (out / f"real-{name}-16bit.png").write_bytes(data)
            print(f"{out / ('real-' + name + '-16bit.png')}  {w}x{h}  {len(data) / 1e6:.2f} MB")


if __name__ == "__main__":
    main()
