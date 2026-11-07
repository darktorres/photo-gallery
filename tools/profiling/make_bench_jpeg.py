"""The per-file benches' input (crates/bench/benches/ir/sample.jpg): a deterministic synthetic photo.

    python tools/profiling/make_bench_jpeg.py crates/bench/benches/ir/sample.jpg

The image is 1280x1017, encoded as a baseline JPEG at quality 80 with 4:2:0 subsampling, the parameters of the file the
benches used before (quantization tables of quality 80). Its content is 1/f noise per channel, which has the spectrum of
a natural photo: most of the energy in low frequencies, so the decoder and the thumbnail maker see photo-like work. The
seed and the contrast are fixed, so the file is the same on every run. Regenerating it changes the per-file baseline
(docs/perf/baseline.json): re-measure after any change here.
"""

import argparse
from pathlib import Path

import numpy as np
from PIL import Image

WIDTH, HEIGHT = 1280, 1017
SEED = 20261105
CONTRAST = 0.20  # the spread of the luminance, as a fraction of the range; sets the file size (~126 KB)
QUALITY = 80


def one_over_f(rng: np.random.Generator) -> np.ndarray:
    """A field in [0, 1] whose power spectrum falls as 1/f²."""
    white = rng.standard_normal((HEIGHT, WIDTH))
    spectrum = np.fft.fft2(white)
    fy = np.fft.fftfreq(HEIGHT)[:, None]
    fx = np.fft.fftfreq(WIDTH)[None, :]
    f = np.sqrt(fx * fx + fy * fy)
    f[0, 0] = 1.0  # the DC term keeps the mean; avoid dividing by zero
    field = np.real(np.fft.ifft2(spectrum / f)).astype(np.float64)
    field -= field.min()
    return field / field.max()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    parser.add_argument("out", type=Path)
    args = parser.parse_args()

    rng = np.random.default_rng(SEED)
    luma = one_over_f(rng)
    chroma = [one_over_f(rng) for _ in range(2)]
    base = 0.5 + CONTRAST * (luma - 0.5)
    rgb = np.stack(
        [
            base + 0.10 * (chroma[0] - 0.5),
            base,
            base - 0.10 * (chroma[1] - 0.5),
        ],
        axis=-1,
    )
    pixels = np.clip(rgb * 255.0 + 0.5, 0, 255).astype(np.uint8)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(pixels, "RGB").save(
        args.out, format="JPEG", quality=QUALITY, subsampling=2, optimize=False, progressive=False
    )
    print(f"wrote {args.out} ({args.out.stat().st_size:,} bytes)")


if __name__ == "__main__":
    main()
