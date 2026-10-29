"""Second-stage verifier prototype: align two images, undo global edits, look at what is left.

Usage: python verify.py [--pairs labeled|cross_groups|knn] [--model sscd_fp16] [--floor 0.4] [--k 10] [--limit N]

Pair sets (only pairs whose embedding similarity from emb_<model>.npy clears --floor, i.e. the
pairs a k-NN candidate stage would actually hand to this verifier; labeled duplicates always):
  labeled      - every labeled duplicate pair (should match), and similiar_but_not_dups pairs
                 (not scored: that folder is mostly variants, which count as matches, mixed with
                 different pictures). Writes verify_<model>.*
  cross_groups - pairs from two different dups2/<NNN> groups: different pictures, so they should
                 be unrelated. The negative set. Writes verify_cross_<model>.*
  knn          - what the pipeline would really check: every image's --k nearest neighbors above
                 --floor, as unordered pairs. Labeled where the ground truth says something
                 (duplicate, cross_group, similar_folder), "unlabeled" otherwise. Writes verify_knn_<model>.*
Duplicates and variants (local edits of the same picture) are not told apart for now: both are
a "match".

Per pair:
  1. Match local features (DISK + LightGlue) between A and B, and between A and mirrored B.
  2. Fit a homography with MAGSAC. Undoes crops, zooms and mirrors.
  3. Warp the less-contained image onto the more-contained one (the "reference"), on a
     comparison canvas, with both blurred to the same effective resolution.
  4. Fit one global color mapping (quadratic in RGB, Tukey-reweighted so a locally edited
     region does not bias it). Undoes grading, grayscale, brightness and contrast.
  5. Residual = per-pixel color difference, tolerant to 2 px of misalignment. "noise" = its
     median over the overlap. Regions clearly above the noise level are "changed"; connected
     regions are measured, overall and away from the frame edges ("interior": a different frame of
     the same shoot changes the interior; overlays and status bars sit at the edges). Diagnostic
     only for now.

Decision (decide()):
  match     - after alignment and color mapping the pixels agree (noise <= --max-noise). Optionally
              (--strong-inliers, off by default) also when the alignment itself is overwhelming; that
              also matched different frames of one shoot (same background), which aren't matches.
  unrelated - otherwise. Inlier count alone is not enough: DISK + LightGlue + MAGSAC finds a
              "homography" with 20-120 inliers between unrelated pictures too, but their pixels
              don't agree once aligned.

--rescore re-applies the decision thresholds to an existing CSV without touching images.

Writes <prefix>_<model>.csv / .json and one diagnostic image per pair in <prefix>_<model>/
(reference | other, aligned and color-mapped | residual heat map with changed regions).
"""

import argparse
import csv
import json
import time
from dataclasses import dataclass
from functools import lru_cache

import cv2
import numpy as np
import torch

from common import DATA_ROOT, WORK_DIR, ground_truth, image_list, load_rgb

import kornia.feature as KF  # noqa: E402  (after common: weights download under CACHE_DIR)

WORK_SIDE = 1024  # feature extraction resolution (long side)
CANVAS_SIDE = 512  # comparison resolution (long side of the reference)
NUM_FEATURES = 2048
RANSAC_PX = 3.0  # reprojection threshold at WORK_SIDE
INTERIOR_MARGIN = 0.08  # edge band (fraction of the canvas's short side) excluded from "interior" changes
MOVE_PX = 2.0  # optical flow (canvas px) above which a changed pixel may count as moved
FB_PX = 1.0  # max forward-backward flow inconsistency (canvas px) for a real displacement

DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")

# --pairs -> (output prefix, labels in it)
PAIR_SETS = {"labeled": ("verify", ("duplicate", "similar_folder")), "cross_groups": ("verify_cross", ("cross_group",)),
             "knn": ("verify_knn", ("duplicate", "cross_group", "similar_folder", "unlabeled"))}
# The right decision per label; labels without one (similar_folder) aren't scored.
EXPECTED = {"duplicate": "match", "cross_group": "unrelated"}


@dataclass
class Params:
    min_inliers: int  # fewer: don't even try to align
    abs_threshold: float  # residual (RGB distance, 0..sqrt(3)) that always counts as changed
    noise_k: float  # ... or this many robust sigmas above the pair's own noise level
    max_noise: float  # match if the median residual after alignment is at most this
    strong_inliers: int  # ... or if at least this many inliers
    strong_ratio: float  # ... make up at least this fraction of all matches
    max_moved: float = 0.0  # unrelated if more than this share of the overlap moved (0 = off)
    max_interior_change: float = 0.0  # unrelated if more than this share of the interior changed (0 = off)


def decide(m: dict, params: Params) -> str:
    """match / unrelated from a pair's measurements (see the module docstring)."""
    if m.get("decision") == "error":
        return "error"
    if m.get("noise") is None:  # never aligned
        return "unrelated"
    inliers = int(m["inliers"])
    # A different moment of the same scene: the background agrees, but things moved or the subject changed.
    if params.max_moved and float(m.get("moved_frac") or 0) > params.max_moved:
        return "unrelated"
    if params.max_interior_change and float(m.get("interior_changed_frac") or 0) > params.max_interior_change:
        return "unrelated"
    if float(m["noise"]) <= params.max_noise:
        return "match"
    if params.strong_inliers and inliers >= params.strong_inliers \
            and inliers >= params.strong_ratio * max(1, int(m["matches"])):
        return "match"
    return "unrelated"


# ---------- features ----------

class Features:
    def __init__(self) -> None:
        self.disk = KF.DISK.from_pretrained("depth", device=DEVICE)
        self.matcher = KF.LightGlueMatcher("disk").eval().to(DEVICE)

    @torch.inference_mode()
    def extract(self, img: np.ndarray) -> tuple[torch.Tensor, torch.Tensor]:
        t = torch.from_numpy(np.array(img)).to(DEVICE).permute(2, 0, 1)[None].float() / 255.0
        f = self.disk(t, n=NUM_FEATURES, pad_if_not_divisible=True)[0]
        return f.keypoints, f.descriptors

    @torch.inference_mode()
    def match(self, fa, fb, hw_a, hw_b) -> tuple[np.ndarray, np.ndarray]:
        (ka, da), (kb, db) = fa, fb
        if len(ka) < 8 or len(kb) < 8:
            return np.zeros((0, 2), np.float32), np.zeros((0, 2), np.float32)
        lafs_a = KF.laf_from_center_scale_ori(ka[None], torch.ones(1, len(ka), 1, 1, device=DEVICE))
        lafs_b = KF.laf_from_center_scale_ori(kb[None], torch.ones(1, len(kb), 1, 1, device=DEVICE))
        _, idx = self.matcher(da, db, lafs_a, lafs_b, hw1=hw_a, hw2=hw_b)
        idx = idx.cpu().numpy()
        return ka[idx[:, 0]].cpu().numpy(), kb[idx[:, 1]].cpu().numpy()


@dataclass
class Prepared:
    img: np.ndarray  # uint8 RGB at WORK_SIDE (never upscaled)
    flipped: np.ndarray  # the same, mirrored
    scale: float  # working px per original px
    feats: tuple  # DISK (keypoints, descriptors) of img
    feats_flipped: tuple


def prepare(features: Features, rel: str) -> Prepared:
    im = load_rgb(DATA_ROOT / rel)
    scale = min(1.0, WORK_SIDE / max(im.size))
    if scale < 1.0:
        im = im.resize((max(1, round(im.width * scale)), max(1, round(im.height * scale))), resample=4)  # LANCZOS
    img = np.asarray(im)
    flipped = np.ascontiguousarray(img[:, ::-1])
    return Prepared(img, flipped, scale, features.extract(img), features.extract(flipped))


# ---------- geometry ----------

def coverage(h: np.ndarray, src_hw: tuple[int, int], dst_hw: tuple[int, int]) -> float:
    """Fraction of the destination frame covered by the source frame mapped through h."""
    q = 4  # at quarter resolution
    s = np.diag([1 / q, 1 / q, 1.0])
    src = np.ones((max(1, src_hw[0] // q), max(1, src_hw[1] // q)), np.uint8)
    out = cv2.warpPerspective(src, s @ h @ np.linalg.inv(s), (max(1, dst_hw[1] // q), max(1, dst_hw[0] // q)),
                              flags=cv2.INTER_NEAREST)
    return float(out.mean())


def sane(h: np.ndarray | None) -> bool:
    """Rejects degenerate fits: collapsed, exploded or strongly perspective-warped frames."""
    if h is None or not np.all(np.isfinite(h)):
        return False
    det = np.linalg.det(h[:2, :2])
    return 1e-3 < abs(det) < 1e3 and np.abs(h[2, :2]).max() < 2e-3


def fit_homography(pts_dst: np.ndarray, pts_src: np.ndarray) -> tuple[np.ndarray | None, int]:
    """Homography mapping src -> dst points, and its inlier count."""
    if len(pts_src) < 8:
        return None, 0
    h, mask = cv2.findHomography(pts_src, pts_dst, cv2.USAC_MAGSAC, RANSAC_PX, maxIters=10000, confidence=0.9999)
    if not sane(h):
        return None, 0
    return h, int(mask.sum())


# ---------- photometry ----------

def blur(img: np.ndarray, sigma: float) -> np.ndarray:
    return cv2.GaussianBlur(img, (0, 0), sigma) if sigma > 0.05 else img


def quad_features(x: np.ndarray) -> np.ndarray:
    r, g, b = x[..., 0], x[..., 1], x[..., 2]
    return np.stack([np.ones_like(r), r, g, b, r * r, g * g, b * b, r * g, r * b, g * b], axis=-1)


def fit_color(src: np.ndarray, dst: np.ndarray, rng: np.random.Generator, samples: int = 40000) -> np.ndarray:
    """Quadratic RGB mapping src -> dst (N x 3 each), robust to a minority of edited pixels."""
    if len(src) > samples:
        pick = rng.choice(len(src), samples, replace=False)
        src, dst = src[pick], dst[pick]
    phi = quad_features(src)
    w = np.ones(len(src), np.float32)
    coef = np.zeros((phi.shape[1], 3))
    for _ in range(8):
        sw = np.sqrt(w)[:, None]
        coef, *_ = np.linalg.lstsq(phi * sw, dst * sw, rcond=None)
        r = np.linalg.norm(phi @ coef - dst, axis=1)
        sigma = 1.4826 * np.median(r) + 1e-3
        w = np.clip(1 - (r / (4.685 * sigma)) ** 2, 0, None) ** 2  # Tukey biweight
    return coef


def motion(ref: np.ndarray, other: np.ndarray, region: np.ndarray) -> np.ndarray:
    """Pixels of `region` whose difference is explained by local movement, on aligned, color-mapped
    images (float RGB, same canvas).

    A different moment of the same scene moves things (limbs, hair, cloth); an edit or a
    recompression changes appearance in place. Dense optical flow tells them apart where the
    residual can't: a pixel counts as moved if the flow there is larger than MOVE_PX, consistent
    forward and backward (within FB_PX: a real displacement, not a fit to noise), and warping by it
    explains most of the difference (residual drops below half).
    """
    g_ref = cv2.cvtColor((ref * 255).astype(np.uint8), cv2.COLOR_RGB2GRAY)
    g_other = cv2.cvtColor((other * 255).astype(np.uint8), cv2.COLOR_RGB2GRAY)
    dis = cv2.DISOpticalFlow_create(cv2.DISOPTICAL_FLOW_PRESET_MEDIUM)
    fwd = dis.calc(g_ref, g_other, None)  # ref(x) ~ other(x + fwd(x))
    bwd = dis.calc(g_other, g_ref, None)
    h, w = g_ref.shape
    gx, gy = np.meshgrid(np.arange(w, dtype=np.float32), np.arange(h, dtype=np.float32))
    map_x, map_y = gx + fwd[..., 0], gy + fwd[..., 1]
    warped = cv2.remap(other, map_x, map_y, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
    bwd_at = cv2.remap(bwd, map_x, map_y, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
    consistent = np.linalg.norm(fwd + bwd_at, axis=2) < FB_PX
    res_before = blur(np.linalg.norm(ref - other, axis=2), 2.0)
    res_after = blur(np.linalg.norm(ref - warped, axis=2), 2.0)
    moved = region & consistent & (np.linalg.norm(fwd, axis=2) > MOVE_PX) & (res_after < 0.5 * res_before)
    return cv2.morphologyEx(moved.astype(np.uint8), cv2.MORPH_OPEN,
                            cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))) > 0


def tolerant_diff(ref: np.ndarray, other: np.ndarray, radius: int = 2) -> np.ndarray:
    """Per-pixel RGB distance to the best-matching pixel within +-radius: ignores sub-pixel misalignment."""
    best = np.full(ref.shape[:2], np.inf, np.float32)
    for dy in range(-radius, radius + 1):
        for dx in range(-radius, radius + 1):
            shifted = np.roll(other, (dy, dx), axis=(0, 1))
            best = np.minimum(best, np.linalg.norm(ref - shifted, axis=2))
    return best


# ---------- one pair ----------

def verify_pair(features: Features, pa: Prepared, pb: Prepared, params: Params, rng: np.random.Generator):
    hw_a, hw_b = pa.img.shape[:2], pb.img.shape[:2]
    # Try B as is and mirrored; keep whichever aligns with more inliers.
    best = None
    for flipped in (False, True):
        fb = pb.feats_flipped if flipped else pb.feats
        pts_a, pts_b = features.match(pa.feats, fb, hw_a, hw_b)
        h, inliers = fit_homography(pts_a, pts_b)  # B -> A
        if best is None or inliers > best[2]:
            best = (flipped, h, inliers, len(pts_a))
    flipped, h_ba, inliers, matches = best
    out = {"inliers": inliers, "matches": matches, "flipped": flipped}
    if h_ba is None or inliers < params.min_inliers:
        return {**out, "decision": "unrelated"}, None

    img_b = pb.flipped if flipped else pb.img
    cover_a = coverage(h_ba, hw_b, hw_a)  # how much of A shows up in B
    cover_b = coverage(np.linalg.inv(h_ba), hw_a, hw_b)  # how much of B shows up in A
    # Reference = the more-contained image (the crop), so the overlap fills the canvas.
    if cover_a >= cover_b:
        ref, other, ref_p, other_p, h = pa.img, img_b, pa, pb, h_ba
        out["reference"] = "a"
    else:
        ref, other, ref_p, other_p, h = img_b, pa.img, pb, pa, np.linalg.inv(h_ba)
        out["reference"] = "b"
    out["cover_a"], out["cover_b"] = cover_a, cover_b

    # Comparison canvas: the reference at CANVAS_SIDE.
    c = min(1.0, CANVAS_SIDE / max(ref.shape[:2]))
    cw, ch = max(1, round(ref.shape[1] * c)), max(1, round(ref.shape[0] * c))
    h_canvas = np.diag([c, c, 1.0]) @ h
    ref_c = cv2.resize(ref, (cw, ch), interpolation=cv2.INTER_AREA).astype(np.float32) / 255.0
    s_other = float(np.sqrt(abs(np.linalg.det(h_canvas[:2, :2]))))  # canvas px per other working px
    other_src = blur(other.astype(np.float32) / 255.0, 0.5 * np.sqrt(max(0.0, 1 / s_other**2 - 1)))
    other_c = cv2.warpPerspective(other_src, h_canvas, (cw, ch), flags=cv2.INTER_LINEAR)
    valid = cv2.warpPerspective(np.ones(other.shape[:2], np.uint8), h_canvas, (cw, ch), flags=cv2.INTER_NEAREST)
    valid = cv2.erode(valid, np.ones((7, 7), np.uint8)) > 0
    overlap = float(valid.mean())
    out["overlap"] = overlap
    if valid.sum() < 1000:
        return {**out, "decision": "unrelated"}, None

    # Same effective resolution: a low-res original upsampled onto the canvas is blurrier.
    e_ref = 0.5 * max(1.0, c / ref_p.scale)
    e_other = 0.5 * max(1.0, s_other / other_p.scale)
    e = max(e_ref, e_other)
    ref_c = blur(ref_c, 1.0 + np.sqrt(max(0.0, e**2 - e_ref**2)))
    other_c = blur(other_c, 1.0 + np.sqrt(max(0.0, e**2 - e_other**2)))

    coef = fit_color(other_c[valid], ref_c[valid], rng)
    mapped = np.clip(quad_features(other_c) @ coef, 0, 1).astype(np.float32)

    residual = blur(tolerant_diff(ref_c, mapped), 1.5)
    rv = residual[valid]
    noise = float(np.median(rv))
    mad = float(1.4826 * np.median(np.abs(rv - noise)))
    threshold = max(params.abs_threshold, noise + params.noise_k * mad)
    changed = ((residual > threshold) & valid).astype(np.uint8)
    changed = cv2.morphologyEx(changed, cv2.MORPH_OPEN, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5)))
    n, _, stats, _ = cv2.connectedComponentsWithStats(changed, connectivity=8)
    areas = stats[1:, cv2.CC_STAT_AREA] if n > 1 else np.zeros(1)
    # Changes away from the frame edges: overlays, status bars and border remnants sit at the edges;
    # a different moment of the same shoot (pose, wind) changes the interior.
    margin = max(1, round(INTERIOR_MARGIN * min(cw, ch)))
    interior = cv2.erode(valid.astype(np.uint8), np.ones((2 * margin + 1, 2 * margin + 1), np.uint8)) > 0
    in_changed = changed.astype(bool) & interior
    k, _, in_stats, _ = cv2.connectedComponentsWithStats(in_changed.astype(np.uint8), connectivity=8)
    in_areas = in_stats[1:, cv2.CC_STAT_AREA] if k > 1 else np.zeros(0)
    # Of what changed (dilated a little: a moved edge leaves change on both sides), how much moved?
    near_change = cv2.dilate(changed, np.ones((9, 9), np.uint8)).astype(bool) & valid
    moved = motion(ref_c, mapped, near_change)
    out.update({
        "noise": noise,
        "res_p99": float(np.quantile(rv, 0.99)),
        "changed_frac": float(changed.sum() / valid.sum()),
        "blob_frac": float(areas.max() / valid.sum()),
        "interior_changed_frac": float(in_changed.sum() / max(1, interior.sum())),
        # Distinct changed regions inside, ignoring specks below 0.05% of the overlap.
        "interior_regions": int((in_areas >= 0.0005 * valid.sum()).sum()),
        # Share of the overlap that moved, and share of the change that motion explains.
        "moved_frac": float(moved.sum() / valid.sum()),
        "moved_share": float(moved.sum() / max(1, near_change.sum())),
    })
    out["decision"] = decide(out, params)

    # Diagnostic panels: reference with the other frame outlined | other aligned + mapped | residual.
    corners = np.array([[0, 0], [other.shape[1], 0], [other.shape[1], other.shape[0]], [0, other.shape[0]]], np.float32)
    frame = cv2.perspectiveTransform(corners[None], h_canvas)[0].astype(np.int32)
    p1 = (ref_c * 255).astype(np.uint8).copy()
    cv2.polylines(p1, [frame], True, (0, 220, 255), 2)
    p2 = (np.where(valid[..., None], mapped, 0.5) * 255).astype(np.uint8)
    gray = cv2.cvtColor(p1, cv2.COLOR_RGB2GRAY)
    heat = cv2.applyColorMap(np.clip(residual / (3 * threshold) * 255, 0, 255).astype(np.uint8), cv2.COLORMAP_INFERNO)
    p3 = cv2.addWeighted(cv2.cvtColor(gray, cv2.COLOR_GRAY2BGR), 0.35, heat, 0.65, 0)
    p3[~valid] = (60, 60, 60)
    contours, _ = cv2.findContours(changed, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    cv2.drawContours(p3, contours, -1, (255, 255, 0), 2)  # BGR: cyan
    moved_contours, _ = cv2.findContours(moved.astype(np.uint8), cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    cv2.drawContours(p3, moved_contours, -1, (80, 255, 80), 2)  # BGR: green = moved
    gap = np.full((ch, 8, 3), 255, np.uint8)
    diag = np.hstack([cv2.cvtColor(p1, cv2.COLOR_RGB2BGR), gap, cv2.cvtColor(p2, cv2.COLOR_RGB2BGR), gap, p3])
    return out, diag


# ---------- driver ----------

def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pairs", choices=tuple(PAIR_SETS), default="labeled")
    parser.add_argument("--model", default="sscd_fp16", help="embedding used for similarities and candidate selection")
    parser.add_argument("--floor", type=float, default=0.4,
                        help="min similarity for an unlabeled or negative pair to be checked")
    parser.add_argument("--k", type=int, default=10, help="neighbors per image for --pairs knn")
    parser.add_argument("--limit", type=int, default=0, help="check only the first N pairs of each label (0 = all)")
    parser.add_argument("--min-inliers", type=int, default=20)
    parser.add_argument("--abs-threshold", type=float, default=0.08)
    parser.add_argument("--noise-k", type=float, default=6.0)
    parser.add_argument("--max-noise", type=float, default=0.06)
    # Off by default since 2026-10-29: the only labeled duplicate that needed it (dups2/090) turned out
    # to be two different frames of one shoot, which must not match.
    parser.add_argument("--strong-inliers", type=int, default=0, help="0 = off")
    parser.add_argument("--strong-ratio", type=float, default=0.5)
    # Off until tuned on hand marks (viewer: pair_marks.json); try them with --rescore.
    parser.add_argument("--max-moved", type=float, default=0.0, help="0 = off")
    parser.add_argument("--max-interior-change", type=float, default=0.0, help="0 = off")
    parser.add_argument("--rescore", action="store_true",
                        help="re-apply the decision thresholds to the existing CSV (no image work)")
    args = parser.parse_args()
    params = Params(args.min_inliers, args.abs_threshold, args.noise_k, args.max_noise, args.strong_inliers,
                    args.strong_ratio, args.max_moved, args.max_interior_change)
    prefix, labels = PAIR_SETS[args.pairs]

    if args.rescore:
        with open(WORK_DIR / f"{prefix}_{args.model}.csv", newline="", encoding="utf-8") as f:
            results = [{k: v for k, v in r.items() if v != ""} for r in csv.DictReader(f)]
        for row in results:
            row["decision"] = decide(row, params)
            if row["label"] in EXPECTED:
                row["correct"] = row["decision"] == EXPECTED[row["label"]]
        previous = json.loads((WORK_DIR / f"{prefix}_{args.model}.json").read_text(encoding="utf-8"))
        write_outputs(prefix, args, params, labels, results, previous["seconds"])
        return

    files = image_list()
    gt = ground_truth(files)
    emb = np.load(WORK_DIR / f"emb_{args.model}.npy").astype(np.float32)
    ok = np.load(WORK_DIR / f"ok_{args.model}.npy")

    pairs = []
    if args.pairs == "labeled":
        positives = [(a, b) for a, b in gt.positives if ok[a] and ok[b]]
        pool = np.array([i for i in gt.negative_pool if ok[i]])
        rows, cols = np.nonzero(np.triu(emb[pool] @ emb[pool].T >= args.floor, k=1))
        similar = [(int(pool[r]), int(pool[c])) for r, c in zip(rows, cols)]
        if args.limit:
            positives, similar = positives[:args.limit], similar[:args.limit]
        # "similar_folder": no ground truth under the current rule (variants count as matches,
        # and that folder mixes variants with genuinely different pictures).
        pairs = [("duplicate", a, b) for a, b in positives] + [("similar_folder", a, b) for a, b in similar]
    elif args.pairs == "knn":
        from evaluate import knn, unique_pairs

        n = len(files)
        sims, idx = knn(torch.from_numpy(emb).to(DEVICE), args.k)
        keys, key_sims = unique_pairs(sims, idx, n)
        keys = keys[(key_sims >= args.floor) & ok[keys // n] & ok[keys % n]]
        positive = {min(a, b) * n + max(a, b) for a, b in gt.positives}
        group_of = {i: g for g, members in enumerate(gt.groups) for i in members}
        negative_pool = set(gt.negative_pool)

        def label_of(key: int, a: int, b: int) -> str:
            if key in positive:
                return "duplicate"
            if a in group_of and b in group_of and group_of[a] != group_of[b]:
                return "cross_group"
            if a in negative_pool and b in negative_pool:
                return "similar_folder"
            return "unlabeled"

        # Sorted by key, so consecutive pairs share image a and hit the feature cache.
        pairs = [(label_of(int(k), *divmod(int(k), n)), *divmod(int(k), n)) for k in np.sort(keys)]
        if args.limit:
            pairs = pairs[:args.limit]
    else:
        members = np.array([i for g in gt.groups for i in g if ok[i]])
        group = np.array([gi for gi, g in enumerate(gt.groups) for i in g if ok[i]])
        sims = emb[members] @ emb[members].T
        rows, cols = np.nonzero(np.triu((sims >= args.floor) & (group[:, None] != group[None, :]), k=1))
        order = np.argsort(-sims[rows, cols])  # most similar (hardest) first
        pairs = [("cross_group", int(members[rows[i]]), int(members[cols[i]])) for i in order]
        if args.limit:
            pairs = pairs[:args.limit]
    print(f"{len(pairs)} pairs: " + ", ".join(f"{sum(p[0] == lab for p in pairs)} {lab}" for lab in labels)
          + f" (floor {args.floor})", flush=True)

    features = Features()
    prepared = lru_cache(maxsize=256)(lambda i: prepare(features, files[i]))
    diag_dir = WORK_DIR / f"{prefix}_{args.model}"
    diag_dir.mkdir(exist_ok=True)
    rng = np.random.default_rng(0)

    results = []
    start = time.time()
    for pid, (label, a, b) in enumerate(pairs):
        row = {"id": pid, "label": label, "similarity": float(emb[a] @ emb[b]), "path_a": files[a], "path_b": files[b]}
        try:
            out, diag = verify_pair(features, prepared(a), prepared(b), params, rng)
        except Exception as e:  # one bad file must not stop the run
            out, diag = {"decision": "error", "error": f"{type(e).__name__}: {e}"}, None
        row.update(out)
        if diag is not None:
            cv2.imwrite(str(diag_dir / f"{pid}.jpg"), diag, [cv2.IMWRITE_JPEG_QUALITY, 85])
        if label in EXPECTED:
            row["correct"] = row["decision"] == EXPECTED[label]
        results.append(row)
        if (pid + 1) % 100 == 0:
            print(f"  {pid + 1}/{len(pairs)}  {(pid + 1) / (time.time() - start):.1f} pairs/s", flush=True)
    write_outputs(prefix, args, params, labels, results, round(time.time() - start, 1))


def write_outputs(prefix: str, args, params: Params, labels: tuple, results: list[dict], seconds: float) -> None:
    columns = ["id", "label", "similarity", "decision", "correct", "inliers", "matches", "flipped", "reference",
               "cover_a", "cover_b", "overlap", "noise", "res_p99", "changed_frac", "blob_frac",
               "interior_changed_frac", "interior_regions", "moved_frac", "moved_share", "error",
               "path_a", "path_b"]
    with open(WORK_DIR / f"{prefix}_{args.model}.csv", "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, columns, extrasaction="ignore")
        w.writeheader()
        for r in results:
            w.writerow({k: (f"{v:.4f}" if isinstance(v, float) else v) for k, v in r.items()})

    decisions = ["match", "unrelated", "error"]
    confusion = {lab: {d: sum(r["label"] == lab and r["decision"] == d for r in results) for d in decisions}
                 for lab in labels}
    summary = {"pairs_set": args.pairs, "model": args.model, "floor": args.floor, "params": vars(params),
               "pairs": len(results), "seconds": seconds, "confusion": confusion}
    (WORK_DIR / f"{prefix}_{args.model}.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")

    print(f"\n{len(results)} pairs ({seconds} s of image work)")
    print("| label | " + " | ".join(decisions) + " |")
    print("|---|" + "---|" * len(decisions))
    for lab, counts in confusion.items():
        print(f"| {lab} | " + " | ".join(str(counts[d]) for d in decisions) + " |")
    for lab in labels:
        aligned = [r for r in results if r["label"] == lab and "blob_frac" in r]
        if aligned:
            q = lambda k: " / ".join(f"{v:.4f}" for v in np.quantile([float(r[k]) for r in aligned], [0.1, 0.5, 0.9, 0.99]))
            print(f"{lab} (aligned: {len(aligned)}): blob_frac p10/50/90/99 {q('blob_frac')}; noise {q('noise')}")


if __name__ == "__main__":
    main()
