"""The PNGs the decoder comparison's sample lacks: 16-bit, Adam7-interlaced, grayscale and animated (APNG).

    python make_inputs.py [--out DIR]           write the files (default: $PG_ROOT\\eval\\png-decoders\\inputs)
    python make_inputs.py --verify-dump DIR     check the app's 16-bit dumps (png-decoders dump DIR ...) against the source

Each file is written from a library PNG (eval/common.py's DATA_ROOT, the same images as the sample), so its pixels are
the library's. The encoder is here because Pillow can't write 16-bit RGB(A), interlaced or APNG files. It picks one
filter per row (the least sum of absolute residuals, as libpng does) and compresses at zlib level 6. The APNG's frames
are the same size, and its frame 0 is the default image (IDAT).

Before a file is written, Pillow must decode its 8-bit source back to the same pixels (APNG: its first frame).
"""

import argparse
import io
import struct
import sys
import zlib
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import DATA_ROOT, pg_path  # noqa: E402

# The library images behind the inputs, by the names the decoder comparison's table gives them.
RGB_SOURCE = "pinterest_1141451468123671931"  # rgb-3mp
RGBA_SOURCE = "pinterest_1141451468123671747"  # rgba-2mp

# Adam7's seven passes: (x start, y start, x step, y step).
ADAM7 = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
COLOR_TYPE = {1: 0, 3: 2, 4: 6}  # channels -> PNG colour type: grayscale, RGB, RGBA


def chunk(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


def filter_rows(rows: np.ndarray, bpp: int) -> bytes:
    """`rows` (one row of bytes per row) under the filter with the least sum of absolute residuals, as PNG filter bytes."""
    out = bytearray()
    prev = np.zeros(rows.shape[1], dtype=np.int16)
    for row_u8 in rows:
        row = row_u8.astype(np.int16)
        left = np.zeros_like(row)
        up_left = np.zeros_like(row)
        if bpp < row.size:
            left[bpp:] = row[:-bpp]
            up_left[bpp:] = prev[:-bpp]
        p = left + prev - up_left
        pa, pb, pc = np.abs(p - left), np.abs(p - prev), np.abs(p - up_left)
        paeth = np.where((pa <= pb) & (pa <= pc), left, np.where(pb <= pc, prev, up_left))
        residuals = [row, row - left, row - prev, row - (left + prev) // 2, row - paeth]
        scored = []
        for code, residual in enumerate(residuals):
            res = (residual & 0xFF).astype(np.uint8)
            signed = res.astype(np.int16)
            signed = np.where(signed >= 128, signed - 256, signed)
            scored.append((int(np.abs(signed).sum()), code, res))
        _, code, res = min(scored, key=lambda s: s[0])
        out.append(code)
        out += res.tobytes()
        prev = row
    return bytes(out)


def samples_bytes(pixels: np.ndarray, depth: int) -> np.ndarray:
    """(h, w, c) pixels as rows of bytes; 16-bit samples big-endian, as PNG stores them."""
    h, w, c = pixels.shape
    if depth == 16:
        return pixels.astype(">u2").view(np.uint8).reshape(h, w * c * 2)
    return pixels.reshape(h, w * c)


def idat_data(pixels: np.ndarray, depth: int, interlaced: bool) -> bytes:
    """The zlib stream of the scanlines; with Adam7, the seven passes one after another."""
    bpp = pixels.shape[2] * depth // 8
    if not interlaced:
        return zlib.compress(filter_rows(samples_bytes(pixels, depth), bpp), 6)
    raw = bytearray()
    for x0, y0, dx, dy in ADAM7:
        sub = pixels[y0::dy, x0::dx]
        if sub.shape[0] and sub.shape[1]:
            raw += filter_rows(samples_bytes(sub, depth), bpp)
    return zlib.compress(bytes(raw), 6)


def ihdr(w: int, h: int, depth: int, channels: int, interlaced: bool) -> bytes:
    body = struct.pack(">IIBBBBB", w, h, depth, COLOR_TYPE[channels], 0, 0, 1 if interlaced else 0)
    return chunk(b"IHDR", body)


def encode(pixels: np.ndarray, depth: int = 8, interlaced: bool = False) -> bytes:
    h, w, c = pixels.shape
    return (
        b"\x89PNG\r\n\x1a\n"
        + ihdr(w, h, depth, c, interlaced)
        + chunk(b"IDAT", idat_data(pixels, depth, interlaced))
        + chunk(b"IEND", b"")
    )


def encode_apng(frames: list[np.ndarray], delay_ms: int = 100) -> bytes:
    """acTL; frame 0 as the default image (fcTL, then IDAT); each further frame as fcTL and fdAT. One sequence number
    counts the fcTL and fdAT chunks together."""
    h, w, c = frames[0].shape
    out = bytearray(b"\x89PNG\r\n\x1a\n" + ihdr(w, h, 8, c, False))
    out += chunk(b"acTL", struct.pack(">II", len(frames), 0))
    seq = 0
    for index, frame in enumerate(frames):
        out += chunk(b"fcTL", struct.pack(">IIIIIHHBB", seq, w, h, 0, 0, delay_ms, 1000, 0, 0))
        seq += 1
        data = idat_data(frame, 8, False)
        if index == 0:
            out += chunk(b"IDAT", data)
        else:
            out += chunk(b"fdAT", struct.pack(">I", seq) + data)
            seq += 1
    out += chunk(b"IEND", b"")
    return bytes(out)


def load(name: str) -> np.ndarray:
    path = next(DATA_ROOT.rglob(f"{name}.png"), None)
    if path is None:
        raise SystemExit(f"{name}.png is not under {DATA_ROOT}")
    mode = "RGBA" if name == RGBA_SOURCE else "RGB"
    return np.asarray(Image.open(path).convert(mode))


def gray_of(rgb: np.ndarray) -> np.ndarray:
    lum = rgb.astype(np.float64) @ np.array([0.299, 0.587, 0.114])
    return np.round(lum).astype(np.uint8)[..., None]


def build(rgb: np.ndarray, rgba: np.ndarray) -> dict[str, bytes]:
    frames = [rgb, np.roll(rgb, 97, axis=1), rgb[::-1]]
    return {
        "rgb16-3mp": encode(rgb.astype(np.uint16) * 257, depth=16),
        "rgba16-2mp": encode(rgba.astype(np.uint16) * 257, depth=16),
        "rgb16-interlaced-3mp": encode(rgb.astype(np.uint16) * 257, depth=16, interlaced=True),
        "rgb8-interlaced-3mp": encode(rgb, interlaced=True),
        "rgba8-interlaced-2mp": encode(rgba, interlaced=True),
        "gray8-2mp": encode(gray_of(rgb)),
        "apng-rgb8-3frames": encode_apng(frames),
    }


def check_pillow(name: str, data: bytes, want: np.ndarray) -> None:
    """Pillow's first frame of the 8-bit file must be the source's pixels."""
    got = np.asarray(Image.open(io.BytesIO(data)))
    if got.ndim == 2:
        got = got[..., None]
    if got.shape != want.shape or not np.array_equal(got, want):
        raise SystemExit(f"{name}: Pillow's pixels differ from the source")
    print(f"  {name}: Pillow decodes to the source's pixels")


def verify_dump(dump: Path) -> None:
    """The app's 16-bit output (NAME.raw: big-endian samples; NAME.txt: width height channels bytes-per-sample) must
    be the library's 8-bit pixels times 257, as the 16-bit inputs were written."""
    cases = {"rgb16-3mp": (RGB_SOURCE, 3), "rgba16-2mp": (RGBA_SOURCE, 4), "rgb16-interlaced-3mp": (RGB_SOURCE, 3)}
    for name, (source, channels) in cases.items():
        raw = (dump / f"{name}.raw").read_bytes()
        w, h, ch, bytes_per_sample = (int(x) for x in (dump / f"{name}.txt").read_text().split())
        if ch != channels or bytes_per_sample != 2:
            raise SystemExit(f"{name}: the dump says {ch} channels, {bytes_per_sample} bytes a sample")
        want = load(source).astype(">u2") * 257
        if not np.array_equal(np.frombuffer(raw, dtype=">u2").reshape(h, w, ch), want):
            raise SystemExit(f"{name}: the app's output differs from the source times 257")
        print(f"  {name}: the app's output is the source times 257")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=None)
    ap.add_argument("--verify-dump", type=Path, default=None)
    args = ap.parse_args()
    if args.verify_dump:
        verify_dump(args.verify_dump)
        return
    rgb, rgba = load(RGB_SOURCE), load(RGBA_SOURCE)
    files = build(rgb, rgba)
    check_pillow("rgb8-interlaced-3mp", files["rgb8-interlaced-3mp"], rgb)
    check_pillow("rgba8-interlaced-2mp", files["rgba8-interlaced-2mp"], rgba)
    check_pillow("gray8-2mp", files["gray8-2mp"], gray_of(rgb))
    check_pillow("apng-rgb8-3frames", files["apng-rgb8-3frames"], rgb)
    out = args.out or pg_path(None, "eval", "png-decoders", "inputs")
    out.mkdir(parents=True, exist_ok=True)
    for name, data in files.items():
        (out / f"{name}.png").write_bytes(data)
        print(f"{out / (name + '.png')}  {len(data) / 1e6:.2f} MB")


if __name__ == "__main__":
    main()
