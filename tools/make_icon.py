"""Make the app icon from mascot 03: the mascot centred on a square, written as a multi-size Windows icon.

Usage:
  python make_icon.py                    # writes crates/gallery/resources/app.ico and crates/desktop/resources/app-icon.png
  python make_icon.py --preview PNG      # also writes a PNG of the 256 px version, to look at

Needs Pillow. The mascot is the source picture (crates/desktop/resources/mascots/03.png); it is centred on a transparent
square with a margin and drawn at 256 px, and Pillow scales that down for each size in the .ico, so the small sizes stay
clean. Rerun after changing the mascot or the margin below. The .ico is embedded in gallery.exe for Explorer
(crates/gallery/build.rs); the PNG is a Qt resource that sets the window and taskbar icon (crates/desktop/cpp/app_icon.cpp).
"""

import argparse
from pathlib import Path

from PIL import Image

CRATES = Path(__file__).resolve().parents[1] / "crates"
MASCOT = CRATES / "desktop" / "resources" / "mascots" / "03.png"
OUT = CRATES / "gallery" / "resources" / "app.ico"
PNG_OUT = CRATES / "desktop" / "resources" / "app-icon.png"
SIZE = 256
# The square's side beyond the mascot's larger edge, as a fraction of it: the mascot keeps off the edges.
MARGIN = 0.04
SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


def make_icon():
    mascot = Image.open(MASCOT).convert("RGBA")
    side = round(max(mascot.size) * (1 + 2 * MARGIN))
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.alpha_composite(mascot, ((side - mascot.width) // 2, (side - mascot.height) // 2))
    return canvas.resize((SIZE, SIZE), Image.Resampling.LANCZOS)


def main():
    parser = argparse.ArgumentParser(description="Make the app icon from mascot 03 as a multi-size .ico.")
    parser.add_argument("--preview", type=Path, help="also write a 256 px PNG here")
    args = parser.parse_args()

    icon = make_icon()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    PNG_OUT.parent.mkdir(parents=True, exist_ok=True)
    icon.save(OUT, format="ICO", sizes=SIZES)
    print(f"wrote {OUT}")
    icon.save(PNG_OUT)
    print(f"wrote {PNG_OUT}")
    if args.preview:
        icon.save(args.preview)
        print(f"wrote {args.preview}")


if __name__ == "__main__":
    main()
