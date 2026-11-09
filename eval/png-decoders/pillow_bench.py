"""A second opinion for docs/PNG_DECODERS.md: Pillow's own PNG reader (its `zip` decoder over zlib-ng, not libpng),
on the same files as `png-decoders time`.

    python pillow_bench.py --ref-dir DIR [--secs S] FILE...

DIR holds the app's output from `png-decoders dump DIR FILE...` (NAME.raw and NAME.txt). Each file is read into
memory first, so the timing is the decode alone, as the Rust harness times it: `Image.open` on the bytes, `load()`,
then a conversion only when the mode isn't RGB or RGBA (a palette: RGBA if it has transparency, RGB otherwise).
Pixels are compared with the app's output, sample by sample.
"""

import argparse
import contextlib
import io
import pathlib
import statistics
import time
from importlib import metadata

import numpy as np
from PIL import Image, features


def versions() -> str:
    found = []
    for dist in ("numpy", "pillow"):
        with contextlib.suppress(metadata.PackageNotFoundError):
            found.append(f"{dist} {metadata.version(dist)}")
    return ", ".join(found)


# The 116 MP file is the library's largest; Pillow's decompression-bomb guard is for untrusted input, and this is ours.
Image.MAX_IMAGE_PIXELS = None


def decode(data: bytes) -> Image.Image:
    im = Image.open(io.BytesIO(data))
    im.load()
    if im.mode == "P":
        im = im.convert("RGBA" if "transparency" in im.info else "RGB")
    elif im.mode not in ("RGB", "RGBA"):
        im = im.convert("RGBA" if "A" in im.getbands() else "RGB")
    return im


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref-dir", type=pathlib.Path, required=True)
    ap.add_argument("--secs", type=float, default=1.0)
    ap.add_argument("files", nargs="+", type=pathlib.Path)
    args = ap.parse_args()

    print(f"Packages: {versions()}")
    print(f"Pillow's PNG reader on zlib {features.version('zlib')} (its wheel, not libpng)")
    print("Median ms per decode, one thread, full size; then the samples against the app's fork (identical, or the")
    print("count that differ and the largest difference).")
    width = max(len(p.stem) for p in args.files)
    for path in args.files:
        name = path.stem
        data = path.read_bytes()
        ref_w, ref_h, ref_ch = (int(x) for x in (args.ref_dir / f"{name}.txt").read_text().split())
        ref = np.frombuffer((args.ref_dir / f"{name}.raw").read_bytes(), dtype=np.uint8)

        out = np.asarray(decode(data))
        for _ in range(2):
            decode(data)
        samples = []
        start = time.perf_counter()
        while len(samples) < 5 or time.perf_counter() - start < args.secs:
            t = time.perf_counter()
            decode(data)
            samples.append((time.perf_counter() - t) * 1e3)
        ms = statistics.median(samples)

        if out.shape != (ref_h, ref_w, ref_ch):
            check = f"{out.shape[1]}×{out.shape[0]} {out.shape[2]} ch vs {ref_w}×{ref_h} {ref_ch} ch"
        else:
            got = out.reshape(-1)
            if np.array_equal(got, ref):
                check = "identical"
            else:
                diff = np.abs(got.astype(np.int16) - ref.astype(np.int16))
                check = f"{int(np.count_nonzero(diff))} of {got.size} samples differ, largest {int(diff.max())}"
        print(f"{name:{width}}  {ms:9.2f} ms  {check}")


if __name__ == "__main__":
    main()
