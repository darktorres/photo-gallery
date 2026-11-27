"""Measure the merge layout rules of docs/plans/MERGE_PLAN.md (section 4) on images.

For each image, the image is cut into a grid of tiles (the cut is the truth), the tiles are optionally re-encoded as JPEG
(as split files are), and the layout the rules find is compared with the cut:

- correct: the rules place every tile where the cut had it;
- wrong:   the rules accept a layout that is not the cut (the dangerous case);
- refused: the rules find no layout (the honest outcome).

The rules and the tests use only the standard library. The command line reads and cuts images with Pillow, which is
imported only there.

Usage (Pillow installed):
    python tools/merge_measure.py IMAGE [IMAGE ...] [--grid 2x2 --grid 3x2] [--jpeg 90 --jpeg 75] [--reject 1.3]
"""

from __future__ import annotations

import argparse
import importlib.util
import io
import math
import sys
from dataclasses import dataclass
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Iterable

EPS = 1e-9
Key = tuple[int, int]  # (row, column) of a tile in the cut grid


@dataclass(frozen=True)
class Image:
    """An RGB image: three bytes a pixel, row after row."""

    width: int
    height: int
    rgb: bytes

    def pixel(self, x: int, y: int) -> bytes:
        i = (y * self.width + x) * 3
        return self.rgb[i : i + 3]


def _column(img: Image, x: int) -> list[bytes]:
    return [img.pixel(x, y) for y in range(img.height)]


def _row(img: Image, y: int) -> list[bytes]:
    return [img.pixel(x, y) for x in range(img.width)]


def _mean_abs(p: Iterable[bytes], q: Iterable[bytes]) -> float:
    total = 0
    count = 0
    for a, b in zip(p, q, strict=True):
        total += abs(a[0] - b[0]) + abs(a[1] - b[1]) + abs(a[2] - b[2])
        count += 3
    return total / count


def h_score(a: Image, b: Image) -> float:
    """Score of `a` placed left of `b` (the plan's relative seam score, section 4 step 2)."""
    if a.height != b.height or a.width < 2 or b.width < 2:
        return math.inf
    seam = _mean_abs(_column(a, a.width - 1), _column(b, 0))
    base = _mean_abs(_column(a, a.width - 1), _column(a, a.width - 2)) + _mean_abs(_column(b, 0), _column(b, 1))
    return seam / (base + EPS)


def v_score(a: Image, b: Image) -> float:
    """Score of `a` placed above `b`."""
    if a.width != b.width or a.height < 2 or b.height < 2:
        return math.inf
    seam = _mean_abs(_row(a, a.height - 1), _row(b, 0))
    base = _mean_abs(_row(a, a.height - 1), _row(a, a.height - 2)) + _mean_abs(_row(b, 0), _row(b, 1))
    return seam / (base + EPS)


def cut(img: Image, rows: int, cols: int) -> dict[Key, Image]:
    """The tiles of a rows x cols grid, each (width // cols) x (height // rows)."""
    tw, th = img.width // cols, img.height // rows
    tiles: dict[Key, Image] = {}
    for r in range(rows):
        for c in range(cols):
            data = bytearray()
            for y in range(r * th, (r + 1) * th):
                start = (y * img.width + c * tw) * 3
                data += img.rgb[start : start + tw * 3]
            tiles[(r, c)] = Image(tw, th, bytes(data))
    return tiles


def _best_table(tiles: dict[Key, Image], score) -> dict[Key, tuple[Key, float]]:
    """For each tile, its best candidate in one direction and the ratio second/best (1 candidate: unambiguous)."""
    table: dict[Key, tuple[Key, float]] = {}
    for i, tile_i in tiles.items():
        ranked = sorted(((score(tile_i, tiles[j]), j) for j in tiles if j != i), key=lambda t: t[0])
        best, best_key = ranked[0]
        if not math.isfinite(best):
            table[i] = (best_key, 0.0)
            continue
        second = ranked[1][0] if len(ranked) > 1 else math.inf
        table[i] = (best_key, (second + EPS) / (best + EPS))
    return table


def mutual_links(tiles: dict[Key, Image], score, reject: float) -> set[tuple[Key, Key]]:
    """Links (i, j), i left of (or above) j, where each is the other's best with ratio >= reject."""
    right = _best_table(tiles, score)
    left: dict[Key, tuple[Key, float]] = {}
    for j, tile_j in tiles.items():
        ranked = sorted(((score(tiles[i], tile_j), i) for i in tiles if i != j), key=lambda t: t[0])
        best, best_key = ranked[0]
        if not math.isfinite(best):
            left[j] = (best_key, 0.0)
            continue
        second = ranked[1][0] if len(ranked) > 1 else math.inf
        left[j] = (best_key, (second + EPS) / (best + EPS))
    accepted = set()
    for i, (j, ratio) in right.items():
        back, back_ratio = left[j]
        if ratio >= reject and back_ratio >= reject and back == i:
            accepted.add((i, j))
    return accepted


def assemble(pieces: list[Key], right: set[tuple[Key, Key]], down: set[tuple[Key, Key]]) -> dict[Key, tuple[int, int]] | None:
    """Positions (x, y) from the links, or None when they conflict or leave a piece unreached."""
    adj: dict[Key, list[tuple[Key, tuple[int, int]]]] = {p: [] for p in pieces}
    for a, b in right:
        adj[a].append((b, (1, 0)))
        adj[b].append((a, (-1, 0)))
    for a, b in down:
        adj[a].append((b, (0, 1)))
        adj[b].append((a, (0, -1)))
    start = pieces[0]
    pos = {start: (0, 0)}
    stack = [start]
    while stack:
        p = stack.pop()
        x, y = pos[p]
        for q, (dx, dy) in adj[p]:
            want = (x + dx, y + dy)
            if q in pos:
                if pos[q] != want:
                    return None
            else:
                pos[q] = want
                stack.append(q)
    if len(pos) != len(pieces):
        return None
    x0 = min(x for x, _ in pos.values())
    y0 = min(y for _, y in pos.values())
    pos = {p: (x - x0, y - y0) for p, (x, y) in pos.items()}
    if len(set(pos.values())) != len(pieces):
        return None
    return pos


def is_rectangle_or_ragged(pos: dict[Key, tuple[int, int]]) -> bool:
    """A full box, or empty cells only in the last row, or only in the last column (the plan's ragged rule)."""
    w = max(x for x, _ in pos.values()) + 1
    h = max(y for _, y in pos.values()) + 1
    cells = set(pos.values())
    empty = [(x, y) for y in range(h) for x in range(w) if (x, y) not in cells]
    if not empty:
        return True
    return all(y == h - 1 for _, y in empty) or all(x == w - 1 for x, _ in empty)


def adjacency_ok(pos: dict[Key, tuple[int, int]], right: set[tuple[Key, Key]], down: set[tuple[Key, Key]]) -> bool:
    """Every two pieces side by side in the layout have an accepted link between them in that direction."""
    at = {v: k for k, v in pos.items()}
    for (x, y), p in at.items():
        q = at.get((x + 1, y))
        if q is not None and (p, q) not in right:
            return False
        q = at.get((x, y + 1))
        if q is not None and (p, q) not in down:
            return False
    return True


def layout(tiles: dict[Key, Image], reject: float) -> dict[Key, tuple[int, int]] | None:
    """The layout the rules accept for these tiles, or None (refused)."""
    right = mutual_links(tiles, h_score, reject)
    down = mutual_links(tiles, v_score, reject)
    pos = assemble(sorted(tiles), right, down)
    if pos is None or not is_rectangle_or_ragged(pos) or not adjacency_ok(pos, right, down):
        return None
    return pos


def outcome(tiles: dict[Key, Image], rows: int, cols: int, reject: float) -> str:
    """'correct', 'wrong' or 'refused', against the cut grid of rows x cols."""
    pos = layout(tiles, reject)
    if pos is None:
        return "refused"
    truth = {(r, c): (c, r) for r in range(rows) for c in range(cols)}
    return "correct" if pos == truth else "wrong"


def _load(path: str, jpeg: int | None, rows: int, cols: int, max_side: int) -> dict[Key, Image]:
    from PIL import Image as PilImage  # Pillow is needed only on the command line

    pil = PilImage.open(path).convert("RGB")
    if max_side and max(pil.size) > max_side:
        scale = max_side / max(pil.size)
        pil = pil.resize((int(pil.width * scale), int(pil.height * scale)), PilImage.Resampling.LANCZOS)
    tw, th = pil.width // cols, pil.height // rows
    tiles: dict[Key, Image] = {}
    for r in range(rows):
        for c in range(cols):
            piece = pil.crop((c * tw, r * th, (c + 1) * tw, (r + 1) * th))
            if jpeg is not None:
                buf = io.BytesIO()
                piece.save(buf, "JPEG", quality=jpeg, subsampling=2)
                buf.seek(0)
                piece = PilImage.open(buf).convert("RGB")
            tiles[(r, c)] = Image(piece.width, piece.height, piece.tobytes())
    return tiles


def _grid(text: str) -> tuple[int, int]:
    rows, cols = text.lower().split("x")
    return int(rows), int(cols)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Measure the merge layout rules on images.")
    parser.add_argument("images", nargs="+")
    parser.add_argument("--grid", action="append", type=_grid, help="ROWSxCOLS, repeatable (default 2x2, 3x2, 2x4)")
    parser.add_argument("--jpeg", action="append", type=int, help="JPEG quality, repeatable (default 90, 75)")
    parser.add_argument("--no-png", action="store_true", help="skip the PNG (lossless) pieces")
    parser.add_argument("--reject", type=float, default=1.3, help="reject ratio (default 1.3, the plan's value)")
    parser.add_argument("--max-side", type=int, default=0, help="resize the longest side to this first (0: keep)")
    args = parser.parse_args(argv)
    grids = args.grid or [(2, 2), (3, 2), (2, 4)]
    qualities: list[int | None] = list(args.jpeg or [90, 75])
    if not args.no_png:
        qualities.append(None)  # None means PNG (lossless): the pieces are not re-encoded
    if importlib.util.find_spec("PIL") is None:
        print("Pillow is needed to read images: `uv sync` installs it (the tools group)", file=sys.stderr)
        return 2
    totals = {"correct": 0, "wrong": 0, "refused": 0}
    for path in args.images:
        for rows, cols in grids:
            for quality in qualities:
                tiles = _load(path, quality, rows, cols, args.max_side)
                result = outcome(tiles, rows, cols, args.reject)
                totals[result] += 1
                label = "png" if quality is None else f"jpeg{quality}"
                print(f"{path}  {rows}x{cols}  {label:8s} {result}")
    print(f"total: correct {totals['correct']}  wrong {totals['wrong']}  refused {totals['refused']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
