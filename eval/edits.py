"""Synthetic edits used to test robustness (synth.py, pdq.py). Torch-free, so worker processes
that only hash images don't pay for importing it."""

import io
import math
import random
from collections.abc import Callable

from PIL import Image, ImageDraw, ImageEnhance, ImageFont, ImageOps


def crop_area(fraction: float) -> Callable[[Image.Image, random.Random], Image.Image]:
    """Random crop keeping `fraction` of the area, with mild aspect-ratio change."""

    def apply(im: Image.Image, rng: random.Random) -> Image.Image:
        w, h = im.size
        r = rng.uniform(0.8, 1.25)
        cw = max(1, min(w, round(w * fraction**0.5 * r)))
        ch = max(1, min(h, round(h * fraction / (cw / w))))
        x = rng.randint(0, w - cw)
        y = rng.randint(0, h - ch)
        return im.crop((x, y, x + cw, y + ch))

    return apply


def jpeg(quality: int) -> Callable[[Image.Image, random.Random], Image.Image]:
    def apply(im: Image.Image, _rng: random.Random) -> Image.Image:
        buf = io.BytesIO()
        im.save(buf, "JPEG", quality=quality)
        buf.seek(0)
        return Image.open(buf).convert("RGB")

    return apply


def downscale(im: Image.Image, _rng: random.Random) -> Image.Image:
    w, h = im.size
    return im.resize((max(1, w // 4), max(1, h // 4)), Image.Resampling.LANCZOS)


def hflip(im: Image.Image, _rng: random.Random) -> Image.Image:
    return ImageOps.mirror(im)


def rot90(im: Image.Image, _rng: random.Random) -> Image.Image:
    return im.rotate(90, expand=True)


def rotate(low: float, high: float) -> Callable[[Image.Image, random.Random], Image.Image]:
    """Rotation by an angle in [low, high) degrees (counter-clockwise) on an enlarged canvas, corners black."""

    def apply(im: Image.Image, rng: random.Random) -> Image.Image:
        return im.rotate(rng.uniform(low, high), resample=Image.Resampling.BICUBIC, expand=True, fillcolor=(0, 0, 0))

    return apply


def _inner_rect(w: int, h: int, angle: float) -> tuple[float, float]:
    """Largest axis-aligned rectangle inside a w x h rectangle rotated by `angle` radians."""
    long_side, short_side = max(w, h), min(w, h)
    sin_a, cos_a = abs(math.sin(angle)), abs(math.cos(angle))
    if short_side <= 2 * sin_a * cos_a * long_side or abs(sin_a - cos_a) < 1e-10:
        x = 0.5 * short_side
        return (x / sin_a, x / cos_a) if w >= h else (x / cos_a, x / sin_a)
    cos_2a = cos_a * cos_a - sin_a * sin_a
    return (w * cos_a - h * sin_a) / cos_2a, (h * cos_a - w * sin_a) / cos_2a


def tilt_crop(im: Image.Image, rng: random.Random) -> Image.Image:
    """A straightened (or tilted) copy: rotated 3-15 degrees either way, cropped to the inside of the picture."""
    deg = rng.uniform(3, 15) * rng.choice((-1, 1))
    rotated = im.rotate(deg, resample=Image.Resampling.BICUBIC, expand=True)
    cw, ch = _inner_rect(im.width, im.height, math.radians(deg))
    x, y = (rotated.width - cw) / 2, (rotated.height - ch) / 2
    return rotated.crop((round(x), round(y), round(x + cw), round(y + ch)))


def letterbox(im: Image.Image, _rng: random.Random) -> Image.Image:
    w, h = im.size
    return ImageOps.expand(im, border=(w // 10, h // 10), fill=(0, 0, 0))


def grayscale(im: Image.Image, _rng: random.Random) -> Image.Image:
    return ImageOps.grayscale(im).convert("RGB")


def color_shift(im: Image.Image, _rng: random.Random) -> Image.Image:
    return ImageEnhance.Brightness(ImageEnhance.Color(im).enhance(1.6)).enhance(1.15)


def text_overlay(im: Image.Image, _rng: random.Random) -> Image.Image:
    im = im.copy()
    w, h = im.size
    size = max(12, h // 10)
    try:
        font = ImageFont.truetype("arial.ttf", size)
    except OSError:
        font = ImageFont.load_default(size)
    draw = ImageDraw.Draw(im)
    for line, y in (("FOLLOW FOR MORE", 0.15), ("@some_account", 0.80)):
        draw.text(
            (w * 0.05, h * y), line, font=font, fill=(255, 255, 255), stroke_width=max(1, size // 12), stroke_fill=(0, 0, 0)
        )
    return im


def crop50_hflip(im: Image.Image, rng: random.Random) -> Image.Image:
    return hflip(crop_area(0.5)(im, rng), rng)


TRANSFORMS: dict[str, Callable[[Image.Image, random.Random], Image.Image]] = {
    "jpeg_q30": jpeg(30),
    "downscale_25pct": downscale,
    "grayscale": grayscale,
    "color_shift": color_shift,
    "text_overlay": text_overlay,
    "letterbox_10pct": letterbox,
    "hflip": hflip,
    "rot90": rot90,
    "crop_80pct": crop_area(0.80),
    "crop_60pct": crop_area(0.60),
    "crop_40pct": crop_area(0.40),
    "crop_25pct": crop_area(0.25),
    "crop_50pct_hflip": crop50_hflip,
    # Added 2026-10-30 (appended, so the edits above keep their seeds): rotations other than 90 degrees.
    "rot_tilt_crop": tilt_crop,
    "rot_30": rotate(30, 30),
    "rot_45": rotate(45, 45),
    "rot_135": rotate(135, 135),
    "rot_any": rotate(0, 360),
}


def edit_seed(transform_seed: int, item: int) -> int:
    """Per-image RNG seed (transform_seed = run seed + transform index), shared by every script so
    the edits are identical."""
    return transform_seed * 1_000_003 + item
