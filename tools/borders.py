"""Find and remove borders around images: solid (black, white, colored, JPEG-noisy) and blurred
padding bars (a blurred, enlarged copy of the image filling the letterbox).

Usage:
  python borders.py SRC [SRC ...] --out DIR                 # dry run: report only
  python borders.py SRC [SRC ...] --out DIR --apply         # also write cleaned copies

Dry run writes DIR/report.csv, DIR/report.md (summary and every file with a border),
DIR/report.html and DIR/_preview/*.jpg (the detected content box drawn on each image). Nothing is
modified. --apply writes cleaned copies to DIR/cleaned/<src folder name>/<relative path>;
images without borders are not copied, and the
originals are never touched. Rerunning with the same --out resumes (already reported files are
skipped); --apply after a dry run crops the borders that run found. Detections marked as false
positives in the viewer's Borders page (eval/viewer.py, saved to DIR/review.json) are not cropped.

Detection, per side (top, bottom, left, right; repeated so nested borders peel off):
  - A line is "solid" if nearly all its pixels lie within --tol of the line's median color, and
    that color changes only gradually from line to line (absorbs JPEG noise and ringing).
  - A line is "blurred" if its detail (mean gradient) is far below the image's central detail.
  - The border is the run of such lines from the edge; the exact seam is the first strong
    line-to-line change near the end of the run.
  - Accepted only if the seam is a straight edge across most of the line (a dark sky or a
    blurred bokeh background ends irregularly, a real border ends in a straight line), and for
    blurred bars, if the content past the seam is clearly sharper.

Output (--mode):
  lossless (default) - JPEG: lossless crop with jpegtran (no re-encode). The left/top cut is
                       moved inward to the next JPEG block boundary (8 or 16 px), so up to 15 px
                       of content can be lost on those sides. Other formats: as "exact".
  exact              - cut exactly at the seam; JPEG is re-encoded with the original
                       quantization tables and chroma subsampling (minimal generation loss).
Metadata (EXIF, ICC) is kept; cropping works on the stored pixels, so EXIF orientation stays valid.
"""

import argparse
import csv
import hashlib
import html
import json
import math
import os
import random
import shutil
import subprocess
import tempfile
import time
from concurrent.futures import ProcessPoolExecutor
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, JpegImagePlugin

Image.MAX_IMAGE_PIXELS = None  # local, trusted files

EXTS = {".jpg", ".jpeg", ".png", ".webp", ".bmp"}
JPEGS = {".jpg", ".jpeg"}
SIDES = ("top", "bottom", "left", "right")
PREVIEW_SIDE = 360
JPEGTRAN = shutil.which("jpegtran")
REVIEW_FILE = "review.json"  # written by eval/viewer.py's Borders page
FALSE_POSITIVE = "false_positive"
COLUMNS = [
    "path",
    "width",
    "height",
    "status",
    "crop_left",
    "crop_top",
    "crop_right",
    "crop_bottom",
    "sides",
    "rejected",
    "output",
    "error",
]


@dataclass(frozen=True)
class Params:
    tol: int = 24  # max per-channel distance from the line color that still counts as border
    solid_off: float = 0.02  # max fraction of a solid line's pixels beyond tol
    blur_rel: float = 0.15  # blurred line: detail below this fraction of the image's central detail
    blur_abs: float = 1.0  # ... or below this absolute mean gradient (gray levels per px)
    min_content_act: float = 3.0  # skip blur detection on images that are smooth overall
    min_px: int = 3  # thinner borders are ignored
    max_frac: float = 0.45  # a border can't cover more than this fraction of a side
    min_keep: float = 0.3  # the content must keep at least this fraction of each dimension
    straight: float = 0.5  # min fraction of the seam that must be a sharp straight edge
    blur_straight: float = 0.7  # ... for blurred bars, which have no color edge to rely on
    blur_sym: float = 0.1  # blurred bars on opposite sides may differ in depth by this fraction
    seam_step: int = 12  # per-pixel change that marks a column as crossing the seam
    # Real padding is digitally flat away from the seam; a wall or sky has shading. Median pixel
    # deviation, and drift of the line color across the border's depth (JPEG noise moves pixels,
    # not line medians, so noisy colored borders still pass):
    flat_dev: int = 4  # one-sided solid border
    flat_dev_paired: int = 8  # solid borders on both sides
    max_drift: int = 8
    margin: int = 1  # extra px removed past each seam (JPEG chroma blending at the edge)


@dataclass
class Side:
    name: str
    kind: str  # "solid" | "blur"
    depth: int  # content starts this many px from the edge
    color: str  # line color at the edge (hex)
    straight: float
    accepted: bool
    reason: str
    dev: float = 0.0  # solid only: median pixel deviation from the border color
    drift: float = 0.0  # solid only: how far the line color wanders across the border

    def describe(self) -> str:
        kind = f"solid {self.color} dev {self.dev:.0f} drift {self.drift:.0f}" if self.kind == "solid" else "blur"
        verdict = "" if self.accepted else f" [{self.reason}]"
        return f"{self.name}: {kind} {self.depth}px straight {self.straight:.2f}{verdict}"


# ---------- detection ----------


def gray(rows: np.ndarray) -> np.ndarray:
    return rows.mean(axis=2, dtype=np.float32)


def detail(g: np.ndarray) -> np.ndarray:
    """Mean horizontal gradient of each line."""
    return np.abs(np.diff(g, axis=1)).mean(axis=1)


def run_length(mask: np.ndarray, max_gap: int = 2, resume: int = 3) -> int:
    """Length of the leading run of True, bridging gaps of <= max_gap if the run resumes for >= resume."""
    i, n = 0, len(mask)
    while i < n:
        if mask[i]:
            i += 1
            continue
        rest = np.flatnonzero(mask[i : i + max_gap + 1])
        if not len(rest) or not mask[i + rest[0] : i + rest[0] + resume].all() or i + rest[0] + resume > n:
            break
        i += rest[0]
    return i


def solid_lines(rows: np.ndarray, p: Params) -> tuple[np.ndarray, np.ndarray]:
    """Per line: is it solid; plus its median color."""
    sub = rows[:, ::2].astype(np.int16)  # every other column is plenty for line statistics
    med = np.median(sub, axis=1)
    off = (np.abs(sub - med[:, None, :]).max(axis=2) > p.tol).mean(axis=1)
    jump = np.r_[0.0, np.abs(np.diff(med, axis=0)).max(axis=1)]
    return (off < p.solid_off) & (jump <= p.tol / 3), med


def refine_seam(a: np.ndarray, guess: int, before: int, after: int) -> int:
    """The first strong line-to-line change in [guess - before, guess + after]."""
    lo, hi = max(1, guess - before), min(a.shape[0] - 1, guess + after)
    win = a[lo - 1 : hi + 1, ::2].astype(np.int16)
    step = np.abs(np.diff(win, axis=0)).max(axis=2).mean(axis=1)  # step[k] = change into line lo+k
    strong = np.flatnonzero(step >= max(3.0, 0.5 * step.max()))
    return lo + int(strong[0]) if len(strong) else guess


def seam_straightness(sub: np.ndarray, hd_cum: np.ndarray, rows: np.ndarray, p: Params, k: int = 6) -> np.ndarray:
    """For each candidate first-content line i: the fraction of columns where i is a sharp edge.

    A column counts if its color jumps across i, or its detail jumps (sharp content below a
    smooth border). sub: (D, W, 3) int16; hd_cum: cumulative per-column horizontal detail,
    hd_cum[i] = sum over lines < i. Real borders end in a straight edge; skies and bokeh don't.
    """
    color_step = np.abs(sub[rows + 1] - sub[rows - 2]).max(axis=2) > p.seam_step  # (n, W)
    above = (hd_cum[rows - 1] - hd_cum[rows - 1 - k]) / k
    below = (hd_cum[rows + 1 + k] - hd_cum[rows + 1]) / k
    detail_step = below > 2 * above + 2  # (n, W - 1)
    return (color_step[:, :-1] | detail_step).mean(axis=1)


def detect_side(a: np.ndarray, name: str, p: Params, blur_thr: float | None) -> Side | None:
    """Border at the top edge of a (H, W, 3) view; other sides arrive flipped or transposed."""
    h = a.shape[0]
    depth_max = int(h * p.max_frac)
    if depth_max < p.min_px + 8:
        return None
    first, med0 = solid_lines(a[:4], p)
    color = "#{:02x}{:02x}{:02x}".format(*(int(c) for c in med0[0]))
    if first.all():
        return solid_side(a, name, p, depth_max, color)
    if blur_thr is not None and detail(gray(a[:4, ::2].astype(np.int16))).mean() < blur_thr:
        return blur_side(a, name, p, depth_max)
    return None


def solid_side(a: np.ndarray, name: str, p: Params, depth_max: int, color: str) -> Side | None:
    # Look at a few lines first and widen only while the run continues.
    n = 64
    while True:
        n_eff = min(n, depth_max)
        solid, _ = solid_lines(a[:n_eff], p)
        end = run_length(solid)
        if end < n_eff - 3 or n_eff == depth_max:
            break
        n *= 4
    if end < p.min_px:
        return None
    if end >= depth_max - 3:
        return Side(name, "solid", end, color, 0.0, False, "too deep")
    # JPEG ringing next to the content makes the run stop a few lines early.
    seam = refine_seam(a, end, 3, 12)
    sub = a[max(0, seam - 8) : seam + 9, ::2].astype(np.int16)
    hd = np.abs(np.diff(gray(sub), axis=1))
    hd_cum = np.concatenate([np.zeros((1, hd.shape[1]), np.float32), np.cumsum(hd, axis=0)])
    off = seam - max(0, seam - 8)
    k = min(6, off - 1)  # thin borders: compare detail over as many lines as the border has
    if k < 1 or len(sub) < off + 1 + k:
        return Side(name, "solid", seam, color, 0.0, False, "no seam")
    straight = float(seam_straightness(sub, hd_cum, np.array([off]), p, k)[0])
    if straight < p.straight:
        return Side(name, "solid", seam, color, straight, False, "seam not straight")
    core = a[: max(1, seam - 10), ::2].astype(np.int16)  # away from the seam's JPEG ringing
    line_color = np.median(core, axis=1)
    dev = float(np.median(np.abs(core - np.median(line_color, axis=0)).max(axis=2)))
    drift = float(np.ptp(line_color, axis=0).max())
    return Side(name, "solid", min(seam + p.margin, depth_max), color, straight, True, "", dev, drift)


def blur_side(a: np.ndarray, name: str, p: Params, depth_max: int) -> Side | None:
    """A blurred bar has no fixed color and may hold a blurred copy of the content, so instead of
    following a run of similar lines, find the first straight sharp seam below a smooth region."""
    sub = a[: depth_max + 72, ::2].astype(np.int16)
    hd = np.abs(np.diff(gray(sub), axis=1))  # (D, W - 1)
    act = hd.mean(axis=1)
    hd_cum = np.concatenate([np.zeros((1, hd.shape[1]), np.float32), np.cumsum(hd, axis=0)])
    act_cum = np.r_[0.0, np.cumsum(act)]
    rows = np.arange(max(p.min_px, 7), min(depth_max, len(sub) - 72))
    if not len(rows):
        return None
    straight = seam_straightness(sub, hd_cum, rows, p)
    bar = act_cum[rows] / rows  # mean detail of the bar above each candidate
    content = (act_cum[rows + 64] - act_cum[rows]) / 64  # ... and of the content below it
    ok = np.flatnonzero((straight >= p.blur_straight) & (bar < 0.35 * content))
    if not len(ok):
        return None
    seam = refine_seam(a, int(rows[ok[0]]), 1, 3)
    return Side(name, "blur", min(seam + p.margin, depth_max), "", float(straight[ok[0]]), True, "")


def judge(side: Side, other: Side | None, p: Params) -> None:
    """Rules that need the opposite side; rejects `side` in place.

    `other` is the opposite side's accepted detection (or None), captured before any judging,
    so the outcome doesn't depend on which side is judged first.
    """
    paired = other is not None
    if side.kind == "blur":
        # A smooth sky above a straight horizon, or a plain wall beside a door frame, looks like
        # a blurred bar. Padding tools center the content: real bars come in equal pairs.
        if not (
            other is not None
            and other.kind == "blur"
            and abs(side.depth - other.depth) <= max(4, p.blur_sym * max(side.depth, other.depth))
        ):
            side.accepted, side.reason = False, "blurred bar without an equal opposite bar"
        return
    if side.drift > p.max_drift or side.dev > (p.flat_dev_paired if paired else p.flat_dev):
        side.accepted, side.reason = False, "not flat (shading: wall or sky?)"


def side_view(a: np.ndarray, name: str) -> np.ndarray:
    if name == "top":
        return a
    if name == "bottom":
        return a[::-1]
    if name == "left":
        return a.transpose(1, 0, 2)
    return a.transpose(1, 0, 2)[::-1]


def detect(a: np.ndarray, p: Params) -> tuple[tuple[int, int, int, int], list[Side], list[Side]]:
    """Content box (left, top, right, bottom; right/bottom exclusive), accepted and rejected sides."""
    h, w = a.shape[:2]
    center = a[h // 3 : 2 * h // 3 : 4, w // 8 : 7 * w // 8 : 2]
    content_act = float(np.median(detail(gray(center.astype(np.int16))))) if center.size else 0.0
    blur_thr = max(p.blur_abs, p.blur_rel * content_act) if content_act >= p.min_content_act else None

    box = [0, 0, w, h]
    accepted: list[Side] = []
    rejected: list[Side] = []
    for _ in range(3):  # nested borders, e.g. a thin line inside a blurred bar
        changed = False
        # Top/bottom first, then left/right on what is left, so corners never mix two borders.
        for pair in (("top", "bottom"), ("left", "right")):
            l, t, r, b = box
            found = {n: detect_side(side_view(a[t:b, l:r], n), n, p, blur_thr) for n in pair}
            detected = {n: s if s is not None and s.accepted else None for n, s in found.items()}
            for side in detected.values():
                if side is not None:
                    judge(side, detected[pair[1] if side.name == pair[0] else pair[0]], p)
            for name, side in found.items():
                if side is None:
                    continue
                if not side.accepted:
                    if not any(s.name == side.name and s.depth == side.depth for s in rejected):
                        rejected.append(side)
                    continue
                accepted.append(side)
                changed = True
                if name == "top":
                    box[1] += side.depth
                elif name == "bottom":
                    box[3] -= side.depth
                elif name == "left":
                    box[0] += side.depth
                else:
                    box[2] -= side.depth
        if not changed:
            break
    l, t, r, b = box
    if accepted and (r - l < p.min_keep * w or b - t < p.min_keep * h):
        for s in accepted:
            s.accepted, s.reason = False, "too little content left"
        rejected, accepted, box = rejected + accepted, [], [0, 0, w, h]
    return (box[0], box[1], box[2], box[3]), accepted, rejected


# ---------- loading and writing ----------


def pixels(im: Image.Image) -> np.ndarray:
    """Stored pixels as RGB (no EXIF rotation: the crop applies to the stored image)."""
    if im.mode in ("RGBA", "LA", "PA") or (im.mode == "P" and "transparency" in im.info):
        rgba = im.convert("RGBA")
        flat = Image.new("RGB", rgba.size, (255, 255, 255))
        flat.paste(rgba, mask=rgba.getchannel("A"))
        return np.asarray(flat)
    return np.asarray(im.convert("RGB"))


def jpeg_block(im: Image.Image) -> tuple[int, int]:
    """Width and height of the JPEG's minimum coded unit, the granularity of lossless crops."""
    layers = getattr(im, "layer", None)
    if not layers or len(layers) == 1:
        return 8, 8
    return 8 * max(l[1] for l in layers), 8 * max(l[2] for l in layers)


def crop_lossless(src: Path, im: Image.Image, box, dest: Path, tmp_dir: Path) -> tuple[int, int, int, int]:
    """jpegtran crop; left/top move inward to block boundaries. Returns the box actually cut."""
    bw, bh = jpeg_block(im)
    l, t, r, b = box
    l, t = math.ceil(l / bw) * bw, math.ceil(t / bh) * bh
    # jpegtran (IJG) takes only file names, and its argv can't carry non-ASCII paths on
    # Windows: go through ASCII-named temporary files.
    with tempfile.TemporaryDirectory(dir=tmp_dir) as tmp:
        tin, tout = Path(tmp) / "in.jpg", Path(tmp) / "out.jpg"
        shutil.copyfile(src, tin)
        args = [JPEGTRAN, "-copy", "all", "-optimize", "-crop", f"{r - l}x{b - t}+{l}+{t}"]
        if im.info.get("progressive") or im.info.get("progression"):
            args.append("-progressive")
        result = subprocess.run([*args, "-outfile", str(tout), str(tin)], check=False, capture_output=True, text=True)
        if result.returncode != 0:
            raise RuntimeError(f"jpegtran failed: {result.stderr.strip()[:300]}")
        with Image.open(tout) as check:
            if check.size != (r - l, b - t):
                raise RuntimeError(f"jpegtran produced {check.size}, expected {(r - l, b - t)}")
        shutil.move(tout, dest)
    return l, t, r, b


def crop_exact(im: Image.Image, box, dest: Path, fmt: str) -> None:
    cropped = im.crop(box)
    extra = {k: im.info[k] for k in ("exif", "icc_profile") if im.info.get(k)}
    if fmt == "JPEG":
        sampling = JpegImagePlugin.get_sampling(im)
        if getattr(im, "quantization", None):
            extra["qtables"] = im.quantization
        if sampling >= 0:
            extra["subsampling"] = sampling
        extra["progressive"] = bool(im.info.get("progressive") or im.info.get("progression"))
        cropped.save(dest, "JPEG", optimize=True, **extra)
    elif fmt == "WEBP":
        cropped.save(dest, "WEBP", quality=95, method=6, **extra)
    else:
        cropped.save(dest, fmt, **extra)


def preview(a: np.ndarray, box, accepted: list[Side], rejected: list[Side], dest: Path) -> None:
    """Thumbnail with the removed area dimmed and the content box outlined."""
    h, w = a.shape[:2]
    s = min(1.0, PREVIEW_SIDE / max(h, w))
    im = Image.fromarray(a).resize((max(1, round(w * s)), max(1, round(h * s))), Image.Resampling.BILINEAR)
    l, t, r, b = (round(v * s) for v in box)
    if accepted:
        dim = Image.new("RGB", im.size, (255, 0, 200))
        mask = Image.new("L", im.size, 90)
        ImageDraw.Draw(mask).rectangle([l, t, r - 1, b - 1], fill=0)
        im = Image.composite(dim, im, mask)
    draw = ImageDraw.Draw(im)
    for side in rejected:  # where a rejected candidate would have cut
        d = round(side.depth * s)
        line = {
            "top": [0, d, im.width, d],
            "bottom": [0, im.height - d, im.width, im.height - d],
            "left": [d, 0, d, im.height],
            "right": [im.width - d, 0, im.width - d, im.height],
        }[side.name]
        draw.line(line, fill=(255, 210, 0), width=2)
    if accepted:
        draw.rectangle([l, t, r - 1, b - 1], outline=(255, 0, 200), width=2)
    im.save(dest, "JPEG", quality=82)


# ---------- driver ----------


@dataclass(frozen=True)
class Job:
    path: str
    root: str
    out: str
    apply: bool
    mode: str
    params: Params


def process(job: Job) -> dict:
    path, out = Path(job.path), Path(job.out)
    row: dict = {"path": job.path, "status": "none"}
    try:
        with Image.open(path) as im:
            im.load()
            row["width"], row["height"] = im.size
            if min(im.size) < 64 or getattr(im, "n_frames", 1) > 1:
                row["status"] = "skipped"
                return row
            a = pixels(im)
            box, accepted, rejected = detect(a, job.params)
            row["sides"] = "; ".join(s.describe() for s in accepted)
            row["rejected"] = "; ".join(s.describe() for s in rejected)
            if accepted:
                row["status"] = "border"
            elif rejected:
                row["status"] = "rejected"
            if accepted or rejected:
                key = hashlib.sha1(job.path.encode()).hexdigest()[:16]
                preview(a, box, accepted, rejected, out / "_preview" / f"{key}.jpg")
                row["preview"] = f"_preview/{key}.jpg"
            if accepted and job.apply:
                root = Path(job.root)
                dest = out / "cleaned" / root.name / path.relative_to(root)
                dest.parent.mkdir(parents=True, exist_ok=True)
                if job.mode == "lossless" and path.suffix.lower() in JPEGS and JPEGTRAN and im.format == "JPEG":
                    box = crop_lossless(path, im, box, dest, out / "_tmp")
                else:
                    crop_exact(im, box, dest, im.format or "PNG")
                st = path.stat()
                os.utime(dest, ns=(st.st_atime_ns, st.st_mtime_ns))
                row["output"] = str(dest)
            l, t, r, b = box
            row.update(crop_left=l, crop_top=t, crop_right=row["width"] - r, crop_bottom=row["height"] - b)
    except Exception as e:  # one unreadable file must not stop the run
        row.update(status="error", error=f"{type(e).__name__}: {e}")
    return row


def write_html(out: Path, rows: list[dict]) -> None:
    def card(r: dict) -> str:
        name = Path(r["path"]).name
        crop = f"L{r['crop_left']} T{r['crop_top']} R{r['crop_right']} B{r['crop_bottom']}"
        detail_text = r.get("sides") or r.get("rejected") or ""
        return (
            f'<figure><a href="{html.escape(Path(r["path"]).as_uri())}"><img loading="lazy" '
            f'src="{html.escape(r["preview"])}"></a><figcaption><b>{html.escape(name)}</b><br>'
            f"{r['width']}×{r['height']} · cut {crop}<br>{html.escape(detail_text)}</figcaption></figure>"
        )

    found = [r for r in rows if r["status"] == "border" and r.get("preview")]
    rejected = [r for r in rows if r["status"] == "rejected" and r.get("preview")]
    errors = [r for r in rows if r["status"] == "error"]
    body = [
        f"<h1>Borders</h1><p>{len(rows):,} images scanned · {len(found):,} with borders · "
        f"{len(rejected):,} rejected candidates · {len(errors):,} errors. Magenta: area removed. "
        f"Yellow line: a rejected candidate's cut. Click an image for the original.</p>",
        f"<h2>Borders found ({len(found):,})</h2><div class=grid>",
        *map(card, found),
        "</div>",
        f"<h2>Rejected candidates ({len(rejected):,})</h2><div class=grid>",
        *map(card, rejected),
        "</div>",
    ]
    if errors:
        body.append(f"<h2>Errors ({len(errors):,})</h2><ul>")
        body += [f"<li>{html.escape(r['path'])}: {html.escape(r.get('error', ''))}</li>" for r in errors]
        body.append("</ul>")
    page = f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Border report</title><style>
:root {{ --bg: #f5f5f2; --panel: #fff; --text: #1c1c1a; --muted: #6a6a64; --line: #e1e1dc; }}
@media (prefers-color-scheme: dark) {{
  :root {{ --bg: #141413; --panel: #1e1e1c; --text: #ebebe8; --muted: #9b9b94; --line: #33332f; }}
}}
body {{ margin: 0; padding: 16px; font: 14px/1.45 system-ui, "Segoe UI", sans-serif; background: var(--bg); color: var(--text); }}
h1 {{ font-size: 18px; margin: 0 0 6px; }} h2 {{ font-size: 15px; margin: 24px 0 8px; }} p {{ color: var(--muted); }}
.grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 260px), 1fr)); gap: 12px; }}
figure {{ margin: 0; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 8px; min-width: 0; }}
figure img {{ width: 100%; height: 280px; object-fit: contain; display: block; }}
figcaption {{ font-size: 11px; color: var(--muted); margin-top: 6px; overflow-wrap: anywhere; }}
figcaption b {{ color: var(--text); font-weight: 500; }}
</style></head><body>{"".join(body)}</body></html>"""
    (out / "report.html").write_text(page, encoding="utf-8")


def load_review(out: Path) -> dict[str, str]:
    """Verdicts saved by the viewer's Borders page: {absolute path: verdict}."""
    path = out / REVIEW_FILE
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}


def write_md(out: Path, rows: list[dict], roots: list[Path], review: dict[str, str]) -> None:
    """report.md: summary, every file with a border (path, cut per side, what was found), errors."""

    def rel(path: str) -> str:
        for root in roots:
            if Path(path).is_relative_to(root):
                return Path(path).relative_to(root).as_posix()
        return path

    def cell(text: str) -> str:
        return text.replace("|", "\\|")

    detected = sorted((r for r in rows if r["status"] == "border"), key=lambda r: rel(r["path"]))
    marked = [r for r in detected if review.get(r["path"]) == FALSE_POSITIVE]
    found = [r for r in detected if review.get(r["path"]) != FALSE_POSITIVE]
    errors = [r for r in rows if r["status"] == "error"]

    def kind(side: str) -> str:
        # Side.describe(): "top: solid #rrggbb dev .. drift .. 95px straight 1.00" or "top: blur 155px ..."
        words = side.split()
        if words[1] == "blur":
            return "blurred bar"
        rgb = [int(words[2][i : i + 2], 16) for i in (1, 3, 5)]
        return "black / near-black" if max(rgb) <= 40 else "white / near-white" if min(rgb) >= 215 else "other color"

    kinds: dict[str, int] = {}
    for s in (s for r in found for s in r["sides"].split("; ")):
        kinds[kind(s)] = kinds.get(kind(s), 0) + 1
    reasons: dict[str, int] = {}
    for r in rows:
        for s in filter(None, (r.get("rejected") or "").split("; ")):
            reason = s[s.rfind("[") + 1 : -1] if s.endswith("]") else "?"
            reasons[reason] = reasons.get(reason, 0) + 1

    counts = {s: sum(r["status"] == s for r in rows) for s in ("border", "rejected", "none", "skipped", "error")}
    lines = [
        "# Border detection report",
        "",
        f"Source: {', '.join(f'`{r}`' for r in roots)}  ",
        "Generated by `tools/borders.py` (dry run unless `--apply`). Visual review: `report.html` in this folder.",
        "",
        "## Summary",
        "",
        "| | Images |",
        "|---|---|",
        f"| Scanned | {len(rows):,} |",
        f"| **With borders** | **{len(found):,}** ({len(found) / max(1, len(rows)):.1%}) |",
        f"| Detected, but marked false positive in review | {len(marked):,} |",
        f"| Rejected candidates only (walls, skies, plain backgrounds) | {counts['rejected']:,} |",
        f"| No border | {counts['none']:,} |",
        f"| Skipped (tiny or animated) | {counts['skipped']:,} |",
        f"| Errors | {counts['error']:,} |",
        "",
        "Borders found, by side:",
        "",
        "| Kind | Sides |",
        "|---|---|",
        *(f"| {k} | {v:,} |" for k, v in sorted(kinds.items(), key=lambda kv: -kv[1])),
        "",
        "Rejected candidates, by reason:",
        "",
        "| Reason | Sides |",
        "|---|---|",
        *(f"| {k} | {v:,} |" for k, v in sorted(reasons.items(), key=lambda kv: -kv[1])),
        "",
        f"## Files with borders ({len(found):,})",
        "",
        "Cut = pixels removed from the left, top, right and bottom edges.",
        "",
        "| File | Size | Cut L / T / R / B | Found |",
        "|---|---|---|---|",
    ]

    def table_row(r: dict) -> str:
        cut = f"{r['crop_left']} / {r['crop_top']} / {r['crop_right']} / {r['crop_bottom']}"
        return f"| {cell(rel(r['path']))} | {r['width']}×{r['height']} | {cut} | {cell(r['sides'])} |"

    lines += map(table_row, found)
    if marked:
        lines += [
            "",
            f"## Marked false positive in review ({len(marked):,})",
            "",
            "Not cropped by `--apply`.",
            "",
            "| File | Size | Cut L / T / R / B | Found |",
            "|---|---|---|---|",
            *map(table_row, marked),
        ]
    if errors:
        lines += ["", f"## Errors ({len(errors):,})", "", "| File | Error |", "|---|---|"]
        lines += [f"| {cell(rel(r['path']))} | {cell(r.get('error') or '')} |" for r in errors]
    (out / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("src", nargs="+", type=Path, help="folders (scanned recursively) or files")
    parser.add_argument("--out", type=Path, required=True, help="report, previews and cleaned copies go here")
    parser.add_argument("--apply", action="store_true", help="write cleaned copies (default: report only)")
    parser.add_argument("--mode", choices=("lossless", "exact"), default="lossless")
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--sample", type=int, default=0, help="check only N random images")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--tol", type=int, default=Params.tol)
    parser.add_argument("--straight", type=float, default=Params.straight)
    parser.add_argument("--margin", type=int, default=Params.margin)
    args = parser.parse_args()
    params = Params(tol=args.tol, straight=args.straight, margin=args.margin)
    if args.apply and args.mode == "lossless" and not JPEGTRAN:
        raise SystemExit("jpegtran not found on PATH: install it or use --mode exact")

    jobs: list[tuple[Path, Path]] = []
    for src in args.src:
        src = src.resolve()
        if src.is_file():
            jobs.append((src, src.parent))
        else:
            jobs += [(p, src) for p in src.rglob("*") if p.is_file() and p.suffix.lower() in EXTS]
    jobs.sort()
    if args.sample and len(jobs) > args.sample:
        jobs = sorted(random.Random(args.seed).sample(jobs, args.sample))

    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "_preview").mkdir(exist_ok=True)
    (args.out / "_tmp").mkdir(exist_ok=True)
    report = args.out / "report.csv"
    done: list[dict] = []
    if report.exists():
        with open(report, newline="", encoding="utf-8") as f:
            done = list(csv.DictReader(f))
    review = load_review(args.out)
    false_positives = {p for p, verdict in review.items() if verdict == FALSE_POSITIVE}
    if args.apply:
        # Borders found by an earlier dry run still need their cleaned copy: process them again
        # (detection is deterministic), except those marked as false positives in the viewer.
        redo = {r["path"] for r in done if r["status"] == "border" and not r.get("output") and r["path"] not in false_positives}
        if redo:
            done = [r for r in done if r["path"] not in redo]
            with open(report, "w", newline="", encoding="utf-8") as f:
                w = csv.DictWriter(f, [*COLUMNS, "preview"], extrasaction="ignore")
                w.writeheader()
                w.writerows(done)
    seen = {r["path"] for r in done}
    todo = [
        Job(str(p), str(root), str(args.out), args.apply, args.mode, params)
        for p, root in jobs
        if str(p) not in seen and not (args.apply and str(p) in false_positives)
    ]
    in_scope = {str(p) for p, _ in jobs}
    print(
        f"{len(jobs):,} images, {len(seen & in_scope):,} already done, {len(todo):,} to process"
        + (f", {len(false_positives & in_scope):,} marked false positive (not cropped)" if args.apply else ""),
        flush=True,
    )

    new_file = not report.exists()
    start = time.time()
    with open(report, "a", newline="", encoding="utf-8") as f, ProcessPoolExecutor(args.workers) as pool:
        w = csv.DictWriter(f, [*COLUMNS, "preview"], extrasaction="ignore")
        if new_file:
            w.writeheader()
        for i, row in enumerate(pool.map(process, todo, chunksize=8), 1):
            w.writerow(row)
            done.append(row)
            if i % 500 == 0 or i == len(todo):
                f.flush()
                print(f"  {i:,}/{len(todo):,}  {i / (time.time() - start):.1f} img/s", flush=True)

    counts = {s: sum(r["status"] == s for r in done) for s in ("border", "rejected", "none", "skipped", "error")}
    print("  ".join(f"{k}: {v:,}" for k, v in counts.items()))
    write_html(args.out, done)
    write_md(args.out, done, [s.resolve() if s.is_dir() else s.resolve().parent for s in args.src], review)
    print(f"report: {args.out / 'report.html'}, {args.out / 'report.md'}")


if __name__ == "__main__":
    main()
