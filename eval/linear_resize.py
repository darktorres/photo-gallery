"""The app's resize, in numpy: Magic Kernel Sharp 2021 on the sRGB bytes, as the app's Resizer runs it.

Mirrors crates/codec/src/resize.rs (Resizer::fit_within, and the convolution with the MKS 2021 kernel) and
crates/codec/src/kernel.rs (the kernel k(x) = sum over n of s_n m(x - n)). The resize takes the sRGB bytes as they are,
with no conversion to linear light (the app's choice since commit 59d496ec). PDQ's input is made here, so the eval
hashes what the app hashes.

- Window: an output sample's centre is (o + 0.5) * scale; its window is the input samples within support * max(scale, 1)
  of the centre, clipped to the image; the kernel is evaluated at (x - (centre - 0.5)) / max(scale, 1); each row is
  normalised to sum 1. This is the window rule the numpy mirror uses, and _check measures its difference from the app.
- Two passes, vertical then horizontal, in float64, with the result rounded to bytes once (round half up, clamped to
  0-255).

Usage (a check against the app's 220 px outputs, given the inputs and the outputs of the Rust probe):
    python linear_resize.py <inputs_dir> <app_outputs_dir> [--height 220]
"""

import argparse
import functools
import math
import sys
from pathlib import Path

import numpy as np
from scipy import sparse

SUPPORT = 4.5
# Sharp 2021 taps for |n| = 0..3 (Costella 2021, eqs. 33-35); zero beyond.
TAPS = (17.0 / 12.0, -35.0 / 144.0, 1.0 / 24.0, -1.0 / 144.0)
FIT_SIDE = 512  # PDQ's input (Meta's recommendation); the app's PDQ_SIDE (crates/analysis/src/analyze.rs)


def _m(x: np.ndarray) -> np.ndarray:
    """The magic kernel m(x) (Costella 2021, eq. 27)."""
    ax = np.abs(x)
    return np.where(ax <= 0.5, 0.75 - ax * ax, np.where(ax <= 1.5, (ax - 1.5) ** 2 / 2.0, 0.0))


def _k(x: np.ndarray) -> np.ndarray:
    """The combined kernel k(x) = sum over n of s_n m(x - n)."""
    acc = np.zeros(np.shape(x), dtype=np.float64)
    for n in range(-3, 4):
        acc = acc + TAPS[abs(n)] * _m(x - n)
    return acc


@functools.cache
def _weights(n_in: int, n_out: int) -> sparse.csr_matrix:
    """The (n_out, n_in) resampling matrix: row o holds the weights of output sample o, the window rule above."""
    scale = n_in / n_out
    fs = max(scale, 1.0)
    radius = SUPPORT * fs
    centre = (np.arange(n_out, dtype=np.float64) + 0.5) * scale
    x0 = np.maximum(0, np.floor(centre - radius)).astype(np.int64)
    x1 = np.minimum(n_in, np.ceil(centre + radius)).astype(np.int64)
    width = int((x1 - x0).max())
    x = x0[:, None] + np.arange(width, dtype=np.int64)[None, :]  # (n_out, width): each row's window, from x0
    valid = x < x1[:, None]
    w = _k((np.where(valid, x, 0).astype(np.float64) - (centre - 0.5)[:, None]) / fs)
    w = np.where(valid, w, 0.0)
    total = w.sum(axis=1, keepdims=True)
    w = np.where(total != 0.0, w / np.where(total != 0.0, total, 1.0), w)
    rows = np.broadcast_to(np.arange(n_out, dtype=np.int64)[:, None], x.shape)
    return sparse.csr_matrix((w[valid], (rows[valid], x[valid])), shape=(n_out, n_in))


def _resize_axis0(a: np.ndarray, weights: sparse.csr_matrix) -> np.ndarray:
    """Resample the rows of an (H, W, 3) array."""
    h, w, c = a.shape
    return (weights @ a.reshape(h, w * c)).reshape(weights.shape[0], w, c)


def _resize_axis1(a: np.ndarray, weights: sparse.csr_matrix) -> np.ndarray:
    """Resample the columns of an (H, W, 3) array."""
    h, w, c = a.shape
    moved = a.transpose(1, 0, 2).reshape(w, h * c)
    out = weights @ moved
    return out.reshape(weights.shape[0], h, c).transpose(1, 0, 2)


def resize_srgb(rgb: np.ndarray, out_h: int, out_w: int) -> np.ndarray:
    """An (H, W, 3) uint8 sRGB image resized to (out_h, out_w), returned as uint8 sRGB."""
    h, w = rgb.shape[:2]
    z = _resize_axis1(_resize_axis0(rgb.astype(np.float64), _weights(h, out_h)), _weights(w, out_w))
    return np.floor(np.clip(z, 0.0, 255.0) + 0.5).astype(np.uint8)


def fit_within_size(w: int, h: int, side: int) -> tuple[int, int] | None:
    """The app's fit_within: (width, height) of the image fitted into side x side, or None if it already fits."""
    long = max(w, h)
    if long <= side:
        return None
    scale = side / long
    return max(1, math.floor(w * scale + 0.5)), max(1, math.floor(h * scale + 0.5))


def fit_within(rgb: np.ndarray, side: int = FIT_SIDE) -> np.ndarray:
    """The image as PDQ takes it: fitted into side x side with the app's resize (unchanged if it already fits)."""
    h, w = rgb.shape[:2]
    size = fit_within_size(w, h, side)
    if size is None:
        return rgb
    out_w, out_h = size
    return resize_srgb(rgb, out_h, out_w)


def _check(inputs: Path, outputs: Path, height: int) -> int:
    """Compare against the app's outputs (the Rust probe's MKS 2021 images at `height`): differences in levels."""
    from PIL import Image

    worst = 0
    total = 0
    over1 = 0
    over5 = 0
    for out_path in sorted(outputs.iterdir()):
        src = Image.open(inputs / out_path.name).convert("RGB")
        ref = np.asarray(Image.open(out_path).convert("RGB"), dtype=np.int16)
        # The probe's size rule is Rust's f64::round (half away from zero, crates/codec/src/resize.rs); Python's round() is
        # half to even. For positive values, floor(x + 0.5) is half away from zero.
        out_w = int(src.width * height / src.height + 0.5)
        mine = resize_srgb(np.asarray(src), height, out_w).astype(np.int16)
        if mine.shape != ref.shape:
            print(f"{out_path.name}: size {mine.shape} vs app {ref.shape}")
            return 1
        d = np.abs(mine - ref)
        worst = max(worst, int(d.max()))
        total += d.size
        over1 += int((d >= 1).sum())
        over5 += int((d >= 5).sum())
    print(
        f"{len(list(outputs.iterdir()))} images at height {height}: max {worst} levels; "
        f"samples >= 1 level {over1 / total:.4%}; >= 5 levels {over5}"
    )
    return 0 if worst <= 1 else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("inputs", type=Path)
    parser.add_argument("outputs", type=Path)
    parser.add_argument("--height", type=int, default=220)
    args = parser.parse_args()
    sys.exit(_check(args.inputs, args.outputs, args.height))
