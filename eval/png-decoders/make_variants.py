"""Variants of the inputs, for the PNG decoder comparison. Most keep their source's pixels: each is the source file with one
chunk added or removed. The rest are re-encoded from decoded pixels: `gray16-2mp-trns` (from the 8-bit grey) and `*-filter0`.

    python make_variants.py [--out DIR]     (default: $PG_ROOT\\eval\\png-decoders\\inputs-variants)

- `*-trns`: a tRNS chunk added to a non-transparent file, so that the decoders must apply transparency. A palette gets
  one alpha per entry (the entries' alphas vary); 8-bit RGB gets a colour key (the colour of pixel (0, 0), which becomes
  transparent wherever it occurs); 8-bit grayscale gets a key (the grey of pixel (0, 0)).
- `*-notext`: the library file with its tEXt, zTXt, iTXt and iCCP chunks removed, for libspng, which can't skip text.
  The app's decoder and libpng skip text already, so the pixels and the output are unchanged.
- `gray16-2mp-trns`: 16-bit grayscale, the 8-bit grey of `gray8-2mp` times 257, with a 16-bit key (pixel (0, 0)'s
  grey). It is the only input that makes libspng decode to GA16, which is host-ordered like RGBA16.
- `real-flower_foveon-16bit-trns`: the 16-bit photograph with a 16-bit colour key (the photo's pixel (0, 0)), read
  from the app's dump of it (`ref/`), so `dump` must have run on `inputs-real16` first.
- `*-filter0`: two 8-bit RGB library images re-encoded with filter type 0 on every row, so a decode is nearly all inflate.
"""

import argparse
import struct
import sys
import zlib
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from make_inputs import RGB_SOURCE, chunk, encode, gray_of, ihdr, load, samples_bytes

from common import DATA_ROOT, pg_path

SIGNATURE = b"\x89PNG\r\n\x1a\n"
TEXT_KINDS = {b"tEXt", b"zTXt", b"iTXt", b"iCCP"}
# The library files of the comparison's table, by stem: the six from the sample and the 116 MP one.
LIBRARY_STEMS = [
    "pinterest_1141451468122730916",
    "pinterest_1141451468123671931",
    "pinterest_1141451468122225667",
    "pinterest_1141451468122732060",
    "pinterest_1141451468123671747",
    "pinterest_1141451468123718688",
    "db4f698cb287297b07d5a501a6595f51",
]


def split_chunks(data: bytes) -> tuple[bytes, list[tuple[bytes, bytes]]]:
    """A PNG as its signature and its (kind, data) chunks, CRCs dropped (they are recomputed)."""
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    chunks = []
    pos = 8
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        chunks.append((kind, data[pos + 8 : pos + 8 + length]))
        pos += 12 + length
        if kind == b"IEND":
            break
    return data[:8], chunks


def join(signature: bytes, chunks: list[tuple[bytes, bytes]]) -> bytes:
    return signature + b"".join(chunk(kind, body) for kind, body in chunks)


def with_trns(data: bytes, payload: bytes) -> bytes:
    """`data` with a tRNS chunk before its first IDAT (after PLTE, where a palette file has one)."""
    signature, chunks = split_chunks(data)
    index = next(i for i, (kind, _) in enumerate(chunks) if kind == b"IDAT")
    chunks.insert(index, (b"tRNS", payload))
    return join(signature, chunks)


def without_text(data: bytes) -> bytes:
    signature, chunks = split_chunks(data)
    return join(signature, [(k, d) for k, d in chunks if k not in TEXT_KINDS])


def palette_entries(data: bytes) -> int:
    _, chunks = split_chunks(data)
    return next(len(body) // 3 for kind, body in chunks if kind == b"PLTE")


def library_file(stem: str) -> Path:
    path = next(DATA_ROOT.rglob(f"{stem}.png"), None)
    if path is None:
        raise SystemExit(f"{stem}.png is not under {DATA_ROOT}")
    return path


def filter0_png(pixels: np.ndarray) -> bytes:
    """The 8-bit image with filter type 0 on every row. Unfiltering is then a copy, so a decode is nearly all inflate:
    a decoder's total time on this file is its inflate cost, which is what the PNG decoder comparison asks about."""
    h, w, c = pixels.shape
    raw = b"".join(b"\x00" + row.tobytes() for row in samples_bytes(pixels, 8))
    return SIGNATURE + ihdr(w, h, 8, c, False) + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b"")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()
    out = args.out or pg_path(None, "eval", "png-decoders", "inputs-variants")
    out.mkdir(parents=True, exist_ok=True)

    # tRNS: the palette's alphas vary by entry; the colour and grey keys are pixel (0, 0)'s.
    palette = library_file("pinterest_1141451468123718688").read_bytes()
    alphas = bytes((i * 37) % 256 for i in range(palette_entries(palette)))
    rgb = load(RGB_SOURCE)
    key = rgb[0, 0]
    gray_key = int(gray_of(rgb)[0, 0, 0])
    trns = {
        "palette-2mp-trns": with_trns(palette, alphas),
        "rgb8-3mp-trns": with_trns(library_file(RGB_SOURCE).read_bytes(), struct.pack(">HHH", *(int(v) for v in key))),
        "gray8-2mp-trns": with_trns((out.parent / "inputs" / "gray8-2mp.png").read_bytes(), struct.pack(">H", gray_key)),
    }
    for name, data in trns.items():
        (out / f"{name}.png").write_bytes(data)
        print(f"{out / (name + '.png')}  {len(data) / 1e6:.2f} MB")

    # A 16-bit colour key on a real photograph: the key is the photo's own pixel (0, 0), read from the app's dump of
    # it (make_real16.py's flower, ref/NAME.raw, big-endian samples), so the key is the photo's colour.
    flower = out.parent / "inputs-real16" / "real-flower_foveon-16bit.png"
    flower_ref = out.parent / "ref" / "real-flower_foveon-16bit.raw"
    key16 = flower_ref.read_bytes()[:6]  # R, G, B as 16-bit big-endian: the tRNS payload for RGB 16-bit
    data = with_trns(flower.read_bytes(), key16)
    (out / "real-flower_foveon-16bit-trns.png").write_bytes(data)
    print(f"{out / 'real-flower_foveon-16bit-trns.png'}  {len(data) / 1e6:.2f} MB")

    # 16-bit grey with a key: the 8-bit grey of gray8-2mp, times 257, with the key of pixel (0, 0). The only input that
    # makes libspng decode to GA16.
    grey = np.asarray(Image.open(out.parent / "inputs" / "gray8-2mp.png").convert("L"))[..., None].astype(np.uint16) * 257
    data = with_trns(encode(grey, depth=16), struct.pack(">H", int(grey[0, 0, 0])))
    (out / "gray16-2mp-trns.png").write_bytes(data)
    print(f"{out / 'gray16-2mp-trns.png'}  {len(data) / 1e6:.2f} MB")

    # Filter 0 on every row, from the two largest library RGB images (the 2.79 and 4.28 MP ones of the table).
    for name, stem in [("rgb8-3mp-filter0", RGB_SOURCE), ("rgb8-4mp-filter0", "pinterest_1141451468122225667")]:
        pixels = np.asarray(Image.open(library_file(stem)).convert("RGB"))
        data = filter0_png(pixels)
        (out / f"{name}.png").write_bytes(data)
        print(f"{out / (name + '.png')}  {len(data) / 1e6:.2f} MB")

    notext = pg_path(None, "eval", "png-decoders", "inputs-notext")
    notext.mkdir(parents=True, exist_ok=True)
    for stem in LIBRARY_STEMS:
        data = without_text(library_file(stem).read_bytes())
        (notext / f"{stem}.png").write_bytes(data)
        print(f"{notext / (stem + '.png')}  {len(data) / 1e6:.2f} MB")


if __name__ == "__main__":
    main()
