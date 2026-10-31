"""Draw the app icon: two photos on a dark tile, written as a multi-size Windows icon.

Usage:
  python make_icon.py                    # writes crates/ui/resources/app.ico and app-icon.png
  python make_icon.py --preview PNG      # also writes a PNG of the 256 px version, to look at

Needs Pillow. The icon is drawn at 1024 px and scaled down by Pillow for each size in the .ico, so the small sizes
stay clean. Rerun after changing the colours or shapes below. The .ico is embedded in gallery.exe for Explorer
(crates/ui/build.rs); the PNG is a Qt resource that sets the window and taskbar icon (crates/ui/cpp/app_icon.cpp).
"""

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

RESOURCES = Path(__file__).resolve().parents[1] / "crates" / "ui" / "resources"
OUT = RESOURCES / "app.ico"
PNG_OUT = RESOURCES / "app-icon.png"
SIZE = 1024
SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


def gradient(width, height, top, bottom):
    """A vertical gradient from `top` to `bottom` (RGB)."""
    img = Image.new("RGBA", (width, height))
    draw = ImageDraw.Draw(img)
    for y in range(height):
        t = y / max(height - 1, 1)
        rgb = (round(a + (b - a) * t) for a, b in zip(top, bottom, strict=True))
        draw.line([(0, y), (width, y)], fill=(*rgb, 255))
    return img


def rounded_mask(width, height, radius):
    mask = Image.new("L", (width, height), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, width - 1, height - 1], radius=radius, fill=255)
    return mask


def scene(width, height):
    """A landscape photo: a sky with a sun, and two ranges of mountains."""
    img = gradient(width, height, (92, 170, 214), (206, 236, 246))
    draw = ImageDraw.Draw(img)
    sun = 0.11 * width
    cx, cy = 0.70 * width, 0.30 * height
    draw.ellipse([cx - sun, cy - sun, cx + sun, cy + sun], fill=(255, 205, 96, 255))
    draw.polygon(
        [
            (0, 0.74 * height),
            (0.34 * width, 0.36 * height),
            (0.58 * width, 0.62 * height),
            (0.80 * width, 0.42 * height),
            (width, 0.70 * height),
            (width, height),
            (0, height),
        ],
        fill=(86, 124, 118, 255),
    )
    draw.polygon(
        [
            (0, 0.90 * height),
            (0.26 * width, 0.62 * height),
            (0.50 * width, 0.86 * height),
            (0.76 * width, 0.56 * height),
            (width, 0.90 * height),
            (width, height),
            (0, height),
        ],
        fill=(38, 78, 68, 255),
    )
    return img


def print_frame(width, height, photo_inset, bottom_extra, radius):
    """A white print with a photo in it: the thicker bottom edge is the Polaroid look."""
    frame = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    frame.paste((246, 247, 249, 255), (0, 0, width, height), rounded_mask(width, height, radius))
    photo_w = width - 2 * photo_inset
    photo_h = height - photo_inset - bottom_extra
    photo = scene(photo_w, photo_h)
    photo.putalpha(rounded_mask(photo_w, photo_h, radius // 3))
    frame.alpha_composite(photo, (photo_inset, photo_inset))
    return frame


def drop_shadow(frame, offset, blur, opacity):
    """The frame's alpha, blurred and moved, as a dark layer under it."""
    pad = blur * 3
    layer = Image.new("RGBA", (frame.width + 2 * pad, frame.height + 2 * pad), (0, 0, 0, 0))
    alpha = frame.getchannel("A").filter(ImageFilter.GaussianBlur(blur)).point(lambda v: v * opacity // 255)
    shadow = Image.new("RGBA", frame.size, (0, 0, 0, 255))
    shadow.putalpha(alpha)
    layer.alpha_composite(shadow, (pad + offset[0], pad + offset[1]))
    return layer, pad


def place(canvas, frame, center, angle, shadow_offset=(14, 22)):
    """Rotate `frame` by `angle` degrees, then paste it onto `canvas` centred on `center`, with a shadow."""
    turned = frame.rotate(angle, resample=Image.Resampling.BICUBIC, expand=True)
    shadow, pad = drop_shadow(turned, shadow_offset, blur=22, opacity=150)
    x = round(center[0] - turned.width / 2)
    y = round(center[1] - turned.height / 2)
    layer = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    layer.alpha_composite(shadow, (x - pad, y - pad))
    layer.alpha_composite(turned, (x, y))
    return Image.alpha_composite(canvas, layer)


def make_icon():
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    tile = gradient(SIZE, SIZE, (44, 62, 82), (16, 26, 38))
    tile.putalpha(rounded_mask(SIZE, SIZE, int(SIZE * 0.22)))
    canvas.alpha_composite(tile)

    # The back print is dimmed, so the front one reads as the one in front.
    back = print_frame(int(SIZE * 0.50), int(SIZE * 0.60), photo_inset=34, bottom_extra=110, radius=40)
    dim = Image.new("RGBA", back.size, (16, 26, 38, 110))
    back.alpha_composite(Image.composite(dim, Image.new("RGBA", back.size, (0, 0, 0, 0)), back.getchannel("A")))
    canvas = place(canvas, back, center=(0.42 * SIZE, 0.45 * SIZE), angle=-9)

    front = print_frame(int(SIZE * 0.56), int(SIZE * 0.66), photo_inset=40, bottom_extra=130, radius=44)
    canvas = place(canvas, front, center=(0.58 * SIZE, 0.56 * SIZE), angle=4)
    return canvas


def main():
    parser = argparse.ArgumentParser(description="Draw the app icon as a multi-size .ico.")
    parser.add_argument("--preview", type=Path, help="also write a 256 px PNG here")
    args = parser.parse_args()

    icon = make_icon()
    RESOURCES.mkdir(parents=True, exist_ok=True)
    icon.save(OUT, format="ICO", sizes=SIZES)
    print(f"wrote {OUT}")
    icon.resize((256, 256), Image.Resampling.LANCZOS).save(PNG_OUT)
    print(f"wrote {PNG_OUT}")
    if args.preview:
        icon.resize((256, 256), Image.Resampling.LANCZOS).save(args.preview)
        print(f"wrote {args.preview}")


if __name__ == "__main__":
    main()
