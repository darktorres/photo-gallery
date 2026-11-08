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
  1. Match local features (DISK + LightGlue) between A and B; also between A and mirrored B
     unless the first alignment is already strong. Images work at a long side between
     MIN_WORK_SIDE and WORK_SIDE: DISK finds almost nothing usable on a ~200 px image. With
     --rotations, a pair that doesn't match is retried against B rotated in those steps
     (DISK isn't rotation-invariant), its matches mapped back into B's own frame.
  2. Fit a homography with MAGSAC. Undoes crops, zooms and mirrors.
  3. Warp the less-contained image onto the more-contained one (the "reference"), on a
     comparison canvas, with both blurred to the same effective resolution.
  4. Fit one global color mapping (quadratic in RGB, Tukey-reweighted so a locally edited
     region does not bias it), other -> reference; reference -> other instead only if that fits
     REVERSE_COLOR_GAIN times better: grayscale can't be mapped back to color, but color maps onto
     grayscale. Undoes grading, grayscale, brightness and contrast.
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

--shortcut (pipeline mode, off for evaluation) accepts pairs with PDQ distance <= 22 or SSCD cosine >= 0.95
without image work (group.py's shortcut(); about a third of k-NN pairs, all of which the verifier matched).

--rescore re-applies the decision thresholds to an existing CSV without touching images.

Writes <prefix>_<model>.csv / .json and one diagnostic image per pair in <prefix>_<model>/
(reference | other, aligned | residual heat map with changed regions; the two images in the colors
they were compared in).

Images are decoded and resized ahead on --loaders CPU threads (Prefetcher), steps 1-2 run on the GPU in the
main thread, steps 3-5 on --workers CPU threads (PairRunner).
"""

import argparse
import csv
import json
import time
import zlib
from collections import deque
from concurrent.futures import Future, ThreadPoolExecutor
from dataclasses import dataclass
from functools import lru_cache, partial

import cv2
import numpy as np
import torch
from PIL import Image

from common import DATA_ROOT, WORK_DIR, ground_truth, image_list, load_rgb
from group import SHORTCUT_PDQ, SHORTCUT_SIM, pdq_distances, shortcut

# isort: split
import kornia as K  # after common: weights download under CACHE_DIR
import kornia.feature as KF

WORK_SIDE = 1024  # feature extraction resolution (long side); larger images are downscaled
MIN_WORK_SIDE = 512  # ... smaller ones upscaled to this (25%-downscaled copies: 0/40 aligned as is, 40/40 at 512)
# Don't try the mirrored image if the as-is alignment has this many inliers (45% of k-NN pairs), unless the
# pair then doesn't match: a mirrored symmetric picture can align as is with ~300 inliers (2026-10-29).
MIRROR_SKIP_INLIERS = 200
CANVAS_SIDE = 512  # comparison resolution (long side of the reference)
NUM_FEATURES = 2048
RANSAC_PX = 3.0  # reprojection threshold at WORK_SIDE
INTERIOR_MARGIN = 0.08  # edge band (fraction of the canvas's short side) excluded from "interior" changes
# Map the reference's colors onto the other image (instead of the reverse) only if that fits this many times
# better: grayscale / washed-out copies 2.7-35x; everything else <= 1.56x, where taking the better of two noisy
# fits only made different pictures look closer (5 new false matches on the cross-group set, 2026-10-29).
REVERSE_COLOR_GAIN = 2.0
MOVE_PX = 2.0  # optical flow (canvas px) above which a changed pixel may count as moved
FB_PX = 1.0  # max forward-backward flow inconsistency (canvas px) for a real displacement

DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")

# --pairs -> (output prefix, labels in it)
PAIR_SETS = {
    "labeled": ("verify", ("duplicate", "similar_folder")),
    "cross_groups": ("verify_cross", ("cross_group",)),
    "knn": ("verify_knn", ("duplicate", "cross_group", "similar_folder", "unlabeled")),
    # Pairs listed by another script in WORK_DIR/pairs_<--list>.csv (columns a, b, label: row indices into
    # files.txt), written to verify_<--list>_<model>.csv; e.g. the ANN bake-off's samples (eval/ann).
    "listed": ("verify", ()),
}
# The right decision per label; labels without one (similar_folder) aren't scored. Planted copies (plant.py) are
# edits of their source, so they must match.
EXPECTED = {"duplicate": "match", "cross_group": "unrelated", "planted": "match"}


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
    if m.get("shortcut"):  # accepted without image work (--shortcut)
        return "match"
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
    if params.strong_inliers and inliers >= params.strong_inliers and inliers >= params.strong_ratio * max(1, int(m["matches"])):
        return "match"
    return "unrelated"


# ---------- features ----------

FEATURES = ("disk", "aliked", "sift")  # local features (each with its own LightGlue weights)
ROTATIONS = {"none": (), "90": (90, 180, 270), "45": (45, 90, 135, 180, 225, 270, 315)}


DISK_RUNTIMES = ("trt", "torch")


class Features:
    def __init__(self, kind: str = "disk", disk_runtime: str = "trt", rotations: tuple[int, ...] = ()) -> None:
        self.kind = kind
        self.rotations = rotations  # tried for a pair that doesn't match as is (retry_variants)
        if kind == "disk":
            self.net = KF.DISK.from_pretrained("depth", device=DEVICE)
            if disk_runtime == "trt":  # the U-Net on TensorRT fp16 (accel.py); kornia's keypoint selection stays
                import accel

                self.net.unet = accel.DiskUnetTRT("fp16")
        elif kind == "sift":  # rotation-invariant; LightGlue's SIFT weights expect OpenCV's descriptor layout
            self.net = KF.SIFTFeature(
                num_features=NUM_FEATURES, upright=False, rootsift=True, device=DEVICE, descriptor_layout="opencv"
            )
        else:  # top-k mode: the NUM_FEATURES strongest, like DISK
            self.net = KF.ALIKED.from_pretrained(
                "aliked-n16", max_num_keypoints=NUM_FEATURES, detection_threshold=0, device=DEVICE
            )
        self.matcher = KF.LightGlueMatcher(kind).eval().to(DEVICE)

    @torch.inference_mode()
    def extract(self, img: np.ndarray) -> tuple:
        """(keypoints, descriptors) or, for SIFT, (keypoints, descriptors, local affine frames)."""
        t = torch.from_numpy(np.array(img)).to(DEVICE).permute(2, 0, 1)[None].float() / 255.0
        if self.kind == "sift":
            lafs, _, descs = self.net(K.color.rgb_to_grayscale(t))
            return KF.get_laf_center(lafs)[0], descs[0], lafs
        f = self.net(t, n=NUM_FEATURES, pad_if_not_divisible=True)[0] if self.kind == "disk" else self.net(t)[0]
        return f.keypoints, f.descriptors

    @torch.inference_mode()
    def match(self, fa, fb, hw_a, hw_b) -> tuple[np.ndarray, np.ndarray]:
        (ka, da, *frames_a), (kb, db, *frames_b) = fa, fb
        if len(ka) < 8 or len(kb) < 8:
            return np.zeros((0, 2), np.float32), np.zeros((0, 2), np.float32)
        ones = lambda k: torch.ones(1, len(k), 1, 1, device=DEVICE)  # noqa: E731
        lafs_a = frames_a[0] if frames_a else KF.laf_from_center_scale_ori(ka[None], ones(ka))
        lafs_b = frames_b[0] if frames_b else KF.laf_from_center_scale_ori(kb[None], ones(kb))
        _, idx = self.matcher(da, db, lafs_a, lafs_b, hw1=hw_a, hw2=hw_b)
        idx = idx.cpu().numpy()
        return ka[idx[:, 0]].cpu().numpy(), kb[idx[:, 1]].cpu().numpy()


@dataclass
class Prepared:
    img: np.ndarray  # uint8 RGB, long side between MIN_WORK_SIDE and WORK_SIDE
    scale: float  # working px per original px (> 1 for upscaled small images)
    feats: tuple  # DISK (keypoints, descriptors) of img
    _mirror: tuple | None = None  # (mirrored img, its features), extracted on first use

    def mirrored(self, features: Features) -> tuple[np.ndarray, tuple]:
        if self._mirror is None:
            flipped = np.ascontiguousarray(self.img[:, ::-1])
            self._mirror = (flipped, features.extract(flipped))
        return self._mirror


def rotated_features(features: Features, img: np.ndarray, angle: float) -> tuple[tuple[int, int], tuple, np.ndarray]:
    """Features of `img` rotated `angle` degrees counter-clockwise on an enlarged canvas (fitted to WORK_SIDE):
    (that image's height and width, its features, the 2x3 affine map from its pixels back to img's)."""
    h, w = img.shape[:2]
    m = cv2.getRotationMatrix2D(((w - 1) / 2, (h - 1) / 2), angle, 1.0)
    cos, sin = abs(m[0, 0]), abs(m[0, 1])
    fit = min(1.0, WORK_SIDE / max(w * cos + h * sin, h * cos + w * sin))
    nw, nh = max(1, round((w * cos + h * sin) * fit)), max(1, round((h * cos + w * sin) * fit))
    m = cv2.getRotationMatrix2D(((w - 1) / 2, (h - 1) / 2), angle, fit)
    m[0, 2] += (nw - 1) / 2 - (w - 1) / 2
    m[1, 2] += (nh - 1) / 2 - (h - 1) / 2
    out = cv2.warpAffine(img, m, (nw, nh), flags=cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT, borderValue=(0, 0, 0))
    return (nh, nw), features.extract(out), cv2.invertAffineTransform(m)


def work_image(im) -> tuple[np.ndarray, float]:
    """`im`: an upright RGB PIL image -> (uint8 RGB array at the working resolution, its scale). CPU only."""
    side = max(im.size)
    scale = min(WORK_SIDE, max(MIN_WORK_SIDE, side)) / side
    if scale != 1.0:
        im = im.resize((max(1, round(im.width * scale)), max(1, round(im.height * scale))), resample=Image.Resampling.LANCZOS)
    return np.asarray(im), scale


def load_work_image(rel: str) -> tuple[np.ndarray, float]:
    return work_image(load_rgb(DATA_ROOT / rel))


def prepared(features: Features, work: tuple[np.ndarray, float]) -> Prepared:
    """The GPU part: features of a working-resolution image."""
    img, scale = work
    return Prepared(img, scale, features.extract(img))


def prepare(features: Features, rel: str) -> Prepared:
    return prepared(features, load_work_image(rel))


def prepare_image(features: Features, im) -> Prepared:
    """`im`: an upright RGB PIL image."""
    return prepared(features, work_image(im))


class Prefetcher:
    """Runs `load` (the CPU part of preparing images: decode, edit, resize) on its own threads for `keys`, in the
    order they will be asked for and up to `ahead` of them in advance, so the main thread only feeds the GPU.
    A key that isn't pending (asked for again after the caller's cache dropped it) loads in the caller."""

    def __init__(self, load, keys, threads: int, ahead: int = 32) -> None:
        self.load, self.keys, self.ahead = load, iter(keys), ahead
        self.pool = ThreadPoolExecutor(max(1, threads), thread_name_prefix="load")
        self.pending: dict = {}
        self._fill()

    def _fill(self) -> None:
        while len(self.pending) < self.ahead and (key := next(self.keys, None)) is not None:
            self.pending.setdefault(key, self.pool.submit(self.load, key))

    def get(self, key):
        fut = self.pending.pop(key, None)
        self._fill()
        return fut.result() if fut is not None else self.load(key)

    def close(self) -> None:
        self.pool.shutdown(cancel_futures=True)


# ---------- geometry ----------


def coverage(h: np.ndarray, src_hw: tuple[int, int], dst_hw: tuple[int, int]) -> float:
    """Fraction of the destination frame covered by the source frame mapped through h."""
    q = 4  # at quarter resolution
    s = np.diag([1 / q, 1 / q, 1.0])
    src = np.ones((max(1, src_hw[0] // q), max(1, src_hw[1] // q)), np.uint8)
    out = cv2.warpPerspective(
        src, s @ h @ np.linalg.inv(s), (max(1, dst_hw[1] // q), max(1, dst_hw[0] // q)), flags=cv2.INTER_NEAREST
    )
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


def fit_color(src: np.ndarray, dst: np.ndarray) -> tuple[np.ndarray, float]:
    """Quadratic RGB mapping src -> dst (N x 3 each), robust to a minority of edited pixels.

    Returns the coefficients and the fit's relative error: median residual over dst's color spread,
    so the two directions of a pair compare fairly (a washed-out dst doesn't win just by being flat).
    """
    phi = quad_features(src.astype(np.float64))
    dst = dst.astype(np.float64)
    w = np.ones(len(src))
    coef = np.zeros((phi.shape[1], 3))
    r = np.zeros(len(src))
    for _ in range(8):
        # Weighted least squares via the 10x10 normal equations; lstsq there also copes with a
        # rank-deficient src (grayscale: r = g = b).
        pw = phi * w[:, None]
        coef = np.linalg.lstsq(pw.T @ phi, pw.T @ dst, rcond=None)[0]
        r = np.linalg.norm(phi @ coef - dst, axis=1)
        sigma = 1.4826 * np.median(r) + 1e-3
        w = np.clip(1 - (r / (4.685 * sigma)) ** 2, 0, None) ** 2  # Tukey biweight
    spread = float(np.sqrt(dst.var(axis=0).sum())) + 1e-3
    return coef, float(np.median(r)) / spread


def apply_color(img: np.ndarray, coef: np.ndarray) -> np.ndarray:
    return np.clip(quad_features(img) @ coef.astype(np.float32), 0, 1).astype(np.float32)


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
    return cv2.morphologyEx(moved.astype(np.uint8), cv2.MORPH_OPEN, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))) > 0


@torch.inference_mode()
def tolerant_diff(ref: np.ndarray, other: np.ndarray, radius: int = 2) -> np.ndarray:
    """Per-pixel RGB distance to the best-matching pixel within +-radius: ignores sub-pixel misalignment."""
    r, o = torch.from_numpy(ref).to(DEVICE), torch.from_numpy(other).to(DEVICE)
    best = torch.full(r.shape[:2], float("inf"), device=DEVICE)
    for dy in range(-radius, radius + 1):
        for dx in range(-radius, radius + 1):
            shifted = torch.roll(o, (dy, dx), dims=(0, 1))
            best = torch.minimum(best, torch.linalg.vector_norm(r - shifted, dim=2))
    return best.cpu().numpy()


# ---------- one pair ----------


@dataclass
class Aligned:
    """Result of the GPU half: the alignment, or the reason there is none (out["decision"] set)."""

    out: dict
    ref: np.ndarray | None = None  # the more-contained image (working resolution)
    other: np.ndarray | None = None
    ref_scale: float = 1.0  # working px per original px
    other_scale: float = 1.0
    h: np.ndarray | None = None  # other -> ref
    mirror_skipped: bool = False  # the mirrored image wasn't tried (strong as-is alignment)


def align(features: Features, pa: Prepared, pb: Prepared, params: Params, mirror: bool = False) -> Aligned:
    """Matches A with B as is, then (unless that alignment is already strong and `mirror` is off) with
    B mirrored, and keeps whichever aligns with more inliers."""
    hw_a, hw_b = pa.img.shape[:2], pb.img.shape[:2]
    pts_a, pts_b = features.match(pa.feats, pb.feats, hw_a, hw_b)
    h, inliers = fit_homography(pts_a, pts_b)  # B -> A
    best = (False, h, inliers, len(pts_a), pb.img)
    skip = not mirror and inliers >= MIRROR_SKIP_INLIERS
    if not skip:
        img_m, feats_m = pb.mirrored(features)
        pts_a, pts_b = features.match(pa.feats, feats_m, hw_a, hw_b)
        h, inliers = fit_homography(pts_a, pts_b)
        if inliers > best[2]:
            best = (True, h, inliers, len(pts_a), img_m)
    flipped, h_ba, inliers, matches, img_b = best
    return _aligned({"inliers": inliers, "matches": matches, "flipped": flipped}, pa, img_b, pb.scale, h_ba, params, skip)


def _aligned(
    out: dict, pa: Prepared, img_b: np.ndarray, scale_b: float, h_ba: np.ndarray | None, params: Params, skip: bool = False
) -> Aligned:
    """Picks the reference (the more-contained image) for an alignment B -> A, or rejects a weak one."""
    hw_a, hw_b = pa.img.shape[:2], img_b.shape[:2]
    if h_ba is None or out["inliers"] < params.min_inliers:
        return Aligned({**out, "decision": "unrelated"})

    cover_a = coverage(h_ba, hw_b, hw_a)  # how much of A shows up in B
    cover_b = coverage(np.linalg.inv(h_ba), hw_a, hw_b)  # how much of B shows up in A
    out["cover_a"], out["cover_b"] = cover_a, cover_b
    # Reference = the more-contained image (the crop), so the overlap fills the canvas.
    if cover_a >= cover_b:
        return Aligned({**out, "reference": "a"}, pa.img, img_b, pa.scale, scale_b, h_ba, skip)
    return Aligned({**out, "reference": "b"}, img_b, pa.img, scale_b, pa.scale, np.linalg.inv(h_ba), skip)


def align_rotated(features: Features, pa: Prepared, pb: Prepared, params: Params, beat: int) -> Aligned | None:
    """Matches A with B rotated by each of features.rotations; the best rotation's matches are mapped back
    into B's own frame, so the alignment and the comparison use B as it is. None unless some rotation aligns
    with more than `beat` inliers."""
    best = None
    for angle in features.rotations:
        hw_r, feats_r, back = rotated_features(features, pb.img, angle)
        pts_a, pts_r = features.match(pa.feats, feats_r, pa.img.shape[:2], hw_r)
        pts_b = (pts_r @ back[:, :2].T + back[:, 2]).astype(np.float32) if len(pts_r) else pts_r
        h, inliers = fit_homography(pts_a, pts_b)
        if h is not None and inliers > beat and (best is None or inliers > best[1]):
            best, beat = (h, inliers, len(pts_a), angle), inliers
    if best is None:
        return None
    h, inliers, matches, angle = best
    return _aligned(
        {"inliers": inliers, "matches": matches, "flipped": False, "rotation": angle}, pa, pb.img, pb.scale, h, params
    )


def retry_variants(
    features: Features, pa: Prepared, pb: Prepared, al: Aligned, result: tuple, params: Params, rng: np.random.Generator
) -> tuple:
    """`result` of compare(al). If the pair doesn't match: tries the mirrored image if it was skipped (a mirrored
    symmetric picture also aligns as is), then B rotated (features.rotations); a retry replaces the result
    only by aligning better (mirror) or by matching (rotation)."""
    if result[0].get("decision") == "match":
        return result
    if al.mirror_skipped:
        again = align(features, pa, pb, params, mirror=True)
        if again.out["flipped"]:
            result = compare(again, params, rng)
            if result[0].get("decision") == "match":
                return result
    if features.rotations:
        rot = align_rotated(features, pa, pb, params, beat=int(result[0].get("inliers", 0)))
        if rot is not None and (rotated := compare(rot, params, rng))[0].get("decision") == "match":
            return rotated
    return result


def verify_pair(features: Features, pa: Prepared, pb: Prepared, params: Params, rng: np.random.Generator):
    """Both halves in the calling thread: (measurements and decision, diagnostic image or None)."""
    al = align(features, pa, pb, params)
    return retry_variants(features, pa, pb, al, compare(al, params, rng), params, rng)


def compare(al: Aligned, params: Params, rng: np.random.Generator, samples: int = 40000):
    """The CPU half: warp onto the comparison canvas, undo color edits, measure what is left."""
    out, ref, other = al.out, al.ref, al.other
    if al.h is None or ref is None or other is None:
        return out, None
    # Comparison canvas: the reference at CANVAS_SIDE.
    c = min(1.0, CANVAS_SIDE / max(ref.shape[:2]))
    cw, ch = max(1, round(ref.shape[1] * c)), max(1, round(ref.shape[0] * c))
    h_canvas = np.diag([c, c, 1.0]) @ al.h
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

    # Same effective resolution: a low-res original upsampled onto the canvas is blurrier. Both factors are canvas px
    # per ORIGINAL px (the scales are working px per original px, so the working image's canvas factor is multiplied by them).
    e_ref = 0.5 * max(1.0, c * al.ref_scale)
    e_other = 0.5 * max(1.0, s_other * al.other_scale)
    e = max(e_ref, e_other)
    ref_c = blur(ref_c, 1.0 + np.sqrt(max(0.0, e**2 - e_ref**2)))
    other_c = blur(other_c, 1.0 + np.sqrt(max(0.0, e**2 - e_other**2)))

    # Color mapping both ways on the same pixels; compare in the reference's colors unless the other image
    # lost color information the mapping can't restore (REVERSE_COLOR_GAIN).
    ref_v, other_v = ref_c[valid], other_c[valid]
    if len(ref_v) > samples:
        pick = rng.choice(len(ref_v), samples, replace=False)
        ref_v, other_v = ref_v[pick], other_v[pick]
    to_ref, err_to_ref = fit_color(other_v, ref_v)
    to_other, err_to_other = fit_color(ref_v, other_v)
    if err_to_ref <= REVERSE_COLOR_GAIN * err_to_other:
        ref_cmp, other_cmp = ref_c, apply_color(other_c, to_ref)
        out["color_to"] = "reference"
    else:
        ref_cmp, other_cmp = apply_color(ref_c, to_other), other_c
        out["color_to"] = "other"

    residual = blur(tolerant_diff(ref_cmp, other_cmp), 1.5)
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
    moved = motion(ref_cmp, other_cmp, near_change)
    out.update(
        {
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
        }
    )
    out["decision"] = decide(out, params)

    # Diagnostic panels, in the compared colors: reference with the other frame outlined | other aligned | residual.
    corners = np.array([[0, 0], [other.shape[1], 0], [other.shape[1], other.shape[0]], [0, other.shape[0]]], np.float32)
    frame = cv2.perspectiveTransform(corners[None], h_canvas)[0].astype(np.int32)
    p1 = (ref_cmp * 255).astype(np.uint8).copy()
    cv2.polylines(p1, [frame], True, (0, 220, 255), 2)
    p2 = (np.where(valid[..., None], other_cmp, 0.5) * 255).astype(np.uint8)
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


class PairRunner:
    """Verifies pairs with the GPU half (align) in the calling thread and the CPU half (compare) on
    worker threads, so the GPU extracts and matches the next pairs while earlier ones are compared.
    Finished pairs go to `done(row, out, diag)` in submission order."""

    def __init__(self, features: Features, params: Params, workers: int, done) -> None:
        self.features, self.params, self.done = features, params, done
        self.pool = ThreadPoolExecutor(max(1, workers))
        self.pending: deque[tuple[dict, Future, tuple | None]] = deque()
        self.max_pending = 2 * max(1, workers)

    def submit(self, row: dict, get_a, get_b) -> None:
        """`get_a`, `get_b`: return the two Prepared images; a failure there is the pair's error."""
        retry = None
        try:
            pa, pb = get_a(), get_b()
            al = align(self.features, pa, pb, self.params)
        except Exception as e:  # noqa: BLE001 (one bad file must not stop the run)
            fut: Future = Future()
            fut.set_exception(e)
        else:
            # Seeded per pair: results don't depend on thread timing or on where a resumed run started.
            seed = zlib.crc32(str(row["id"]).encode())
            fut = self.pool.submit(compare, al, self.params, np.random.default_rng(seed))
            retry = (pa, pb, al, seed)
        self.pending.append((row, fut, retry))
        while len(self.pending) > self.max_pending:
            self._finish_one()

    def _finish_one(self) -> None:
        row, fut, retry = self.pending.popleft()
        try:
            out, diag = fut.result()
            if retry is not None:  # in this thread: the mirrored / rotated attempts need the GPU
                pa, pb, al, seed = retry
                out, diag = retry_variants(self.features, pa, pb, al, (out, diag), self.params, np.random.default_rng(seed))
        except Exception as e:  # noqa: BLE001 (the failure is recorded as this pair's result)
            out, diag = {"decision": "error", "error": f"{type(e).__name__}: {e}"}, None
        self.done(row, out, diag)

    def close(self) -> None:
        while self.pending:
            self._finish_one()
        self.pool.shutdown()


def add_decision_args(parser: argparse.ArgumentParser) -> None:
    """The verifier's thresholds as command-line options, and --workers (shared with verify_synth.py)."""
    parser.add_argument("--workers", type=int, default=8, help="CPU threads for the dense comparison")
    parser.add_argument("--loaders", type=int, default=4, help="CPU threads decoding and resizing images ahead")
    parser.add_argument("--features", choices=FEATURES, default="disk", help="local features for the alignment")
    parser.add_argument(
        "--disk-runtime",
        choices=DISK_RUNTIMES,
        default="trt",
        help="DISK's U-Net on TensorRT fp16 (3x faster, same features) or torch fp32",
    )
    parser.add_argument(
        "--rotations",
        choices=tuple(ROTATIONS),
        default="none",
        help="for a pair that doesn't match as is, also try the other image rotated in these steps",
    )
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


def params_from(args: argparse.Namespace) -> Params:
    return Params(
        args.min_inliers,
        args.abs_threshold,
        args.noise_k,
        args.max_noise,
        args.strong_inliers,
        args.strong_ratio,
        args.max_moved,
        args.max_interior_change,
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pairs", choices=tuple(PAIR_SETS), default="labeled")
    parser.add_argument("--model", default="sscd_fp16", help="embedding used for similarities and candidate selection")
    parser.add_argument("--floor", type=float, default=0.4, help="min similarity for an unlabeled or negative pair to be checked")
    parser.add_argument("--k", type=int, default=10, help="neighbors per image for --pairs knn")
    parser.add_argument("--limit", type=int, default=0, help="check only the first N pairs of each label (0 = all)")
    parser.add_argument("--list", default="", help="for --pairs listed: reads WORK_DIR/pairs_<LIST>.csv")
    parser.add_argument(
        "--shortcut", action="store_true", help="accept PDQ <= 22 or SSCD >= 0.95 without image work (pipeline mode)"
    )
    add_decision_args(parser)
    parser.add_argument(
        "--rescore", action="store_true", help="re-apply the decision thresholds to the existing CSV (no image work)"
    )
    args = parser.parse_args()
    params = params_from(args)
    prefix, labels = PAIR_SETS[args.pairs]
    listed: list[tuple[str, int, int]] = []
    if args.pairs == "listed":
        if not args.list:
            parser.error("--pairs listed needs --list")
        prefix = f"verify_{args.list}"
        with open(WORK_DIR / f"pairs_{args.list}.csv", newline="", encoding="utf-8") as f:
            listed = [(r["label"], int(r["a"]), int(r["b"])) for r in csv.DictReader(f)]
        labels = tuple(sorted({label for label, _, _ in listed}))

    if args.rescore:
        with (WORK_DIR / f"{prefix}_{args.model}.csv").open(newline="", encoding="utf-8") as f:
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
        similar = [(int(pool[r]), int(pool[c])) for r, c in zip(rows, cols, strict=True)]
        if args.limit:
            positives, similar = positives[: args.limit], similar[: args.limit]
        # The similar_folder pairs have no ground truth under the current rule (variants count as matches,
        # and that folder mixes variants with genuinely different pictures).
        pairs = [("duplicate", a, b) for a, b in positives] + [("similar_folder", a, b) for a, b in similar]
    elif args.pairs == "listed":
        pairs = [(label, a, b) for label, a, b in listed if ok[a] and ok[b]]
        if args.limit:
            pairs = pairs[: args.limit]
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
            pairs = pairs[: args.limit]
    else:
        members = np.array([i for g in gt.groups for i in g if ok[i]])
        group = np.array([gi for gi, g in enumerate(gt.groups) for i in g if ok[i]])
        sims = emb[members] @ emb[members].T
        rows, cols = np.nonzero(np.triu((sims >= args.floor) & (group[:, None] != group[None, :]), k=1))
        order = np.argsort(-sims[rows, cols])  # most similar (hardest) first
        pairs = [("cross_group", int(members[rows[i]]), int(members[cols[i]])) for i in order]
        if args.limit:
            pairs = pairs[: args.limit]
    print(
        f"{len(pairs)} pairs: "
        + ", ".join(f"{sum(p[0] == lab for p in pairs)} {lab}" for lab in labels)
        + f" (floor {args.floor})",
        flush=True,
    )

    features = Features(args.features, args.disk_runtime, ROTATIONS[args.rotations])
    diag_dir = WORK_DIR / f"{prefix}_{args.model}"
    diag_dir.mkdir(exist_ok=True)

    results: list = [None] * len(pairs)
    checkpoint = Checkpoint(prefix, args.model)
    start, verified = time.time(), 0

    def done(row: dict, out: dict, diag) -> None:
        nonlocal verified
        row.update(out)
        if diag is not None:
            cv2.imwrite(str(diag_dir / f"{row['id']}.jpg"), diag, [cv2.IMWRITE_JPEG_QUALITY, 85])
        if row["label"] in EXPECTED:
            row["correct"] = row["decision"] == EXPECTED[row["label"]]
        results[row["id"]] = row
        checkpoint.add(row)
        verified += 1
        if verified % 100 == 0:
            print(f"  {verified}/{len(pairs)}  {verified / (time.time() - start):.1f} pairs/s", flush=True)

    pdq = pdq_distances(files) if args.shortcut else None
    todo = []  # (row, a, b) of the pairs that need image work
    for pid, (label, a, b) in enumerate(pairs):
        if (row := checkpoint.done(pid)) is not None:
            results[pid] = row
            continue
        row = {"id": pid, "label": label, "similarity": float(emb[a] @ emb[b]), "path_a": files[a], "path_b": files[b]}
        if pdq and (why := shortcut(pdq(files[a], files[b]), row["similarity"], SHORTCUT_PDQ, SHORTCUT_SIM)):
            done(row, {"decision": "match", "shortcut": why}, None)
            continue
        todo.append((row, a, b))

    # Each image's first use, in order: what the prefetcher decodes ahead of the GPU.
    first_use = dict.fromkeys(i for _, a, b in todo for i in (a, b))
    prefetch = Prefetcher(lambda i: load_work_image(files[i]), first_use, args.loaders)
    cached = lru_cache(maxsize=256)(lambda i: prepared(features, prefetch.get(i)))
    runner = PairRunner(features, params, args.workers, done)
    for row, a, b in todo:
        runner.submit(row, partial(cached, a), partial(cached, b))
    runner.close()
    prefetch.close()
    write_outputs(prefix, args, params, labels, results, round(time.time() - start, 1))
    checkpoint.finish()


COLUMNS = [
    "id",
    "label",
    "similarity",
    "decision",
    "correct",
    "inliers",
    "matches",
    "flipped",
    "rotation",
    "reference",
    "cover_a",
    "cover_b",
    "overlap",
    "color_to",
    "noise",
    "res_p99",
    "changed_frac",
    "blob_frac",
    "interior_changed_frac",
    "interior_regions",
    "moved_frac",
    "moved_share",
    "shortcut",
    "error",
    "path_a",
    "path_b",
]


def csv_row(r: dict) -> dict:
    return {k: (f"{v:.4f}" if isinstance(v, float) else v) for k, v in r.items()}


class Checkpoint:
    """Appends every finished pair to <prefix>_<model>.partial.csv, so an interrupted run resumes where it
    stopped instead of starting over. Pair ids must be stable across runs (same inputs and options);
    delete the file to start fresh. Removed once the run completes."""

    def __init__(self, prefix: str, model: str) -> None:
        self.path = WORK_DIR / f"{prefix}_{model}.partial.csv"
        self.rows: dict[str, dict] = {}
        if self.path.exists():
            with self.path.open(newline="", encoding="utf-8") as f:
                self.rows = {r["id"]: {k: v for k, v in r.items() if v != ""} for r in csv.DictReader(f)}
            print(f"resuming: {len(self.rows)} pairs already done ({self.path.name})", flush=True)
        self._file = self.path.open("a", newline="", encoding="utf-8")  # kept open for the run
        self._writer = csv.DictWriter(self._file, COLUMNS, extrasaction="ignore")
        if not self.rows:
            self._writer.writeheader()

    def done(self, pid) -> dict | None:
        return self.rows.get(str(pid))

    def add(self, row: dict) -> None:
        self._writer.writerow(csv_row(row))
        self._file.flush()

    def finish(self) -> None:
        self._file.close()
        self.path.unlink()


def write_outputs(prefix: str, args, params: Params, labels: tuple, results: list[dict], seconds: float) -> None:
    with (WORK_DIR / f"{prefix}_{args.model}.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, COLUMNS, extrasaction="ignore")
        w.writeheader()
        for r in results:
            w.writerow(csv_row(r))

    decisions = ["match", "unrelated", "error"]
    confusion = {lab: {d: sum(r["label"] == lab and r["decision"] == d for r in results) for d in decisions} for lab in labels}
    summary = {
        "pairs_set": args.pairs,
        "model": args.model,
        "floor": args.floor,
        "features": getattr(args, "features", "disk"),
        "params": vars(params),
        "pairs": len(results),
        "seconds": seconds,
        "confusion": confusion,
    }
    (WORK_DIR / f"{prefix}_{args.model}.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")

    print(f"\n{len(results)} pairs ({seconds} s of image work)")
    print("| label | " + " | ".join(decisions) + " |")
    print("|---|" + "---|" * len(decisions))
    for lab, counts in confusion.items():
        print(f"| {lab} | " + " | ".join(str(counts[d]) for d in decisions) + " |")

    def quantiles(rows: list[dict], key: str) -> str:
        return " / ".join(f"{v:.4f}" for v in np.quantile([float(r[key]) for r in rows], [0.1, 0.5, 0.9, 0.99]))

    for lab in labels:
        aligned = [r for r in results if r["label"] == lab and "blob_frac" in r]
        if aligned:
            print(
                f"{lab} (aligned: {len(aligned)}): blob_frac p10/50/90/99 {quantiles(aligned, 'blob_frac')}; "
                f"noise {quantiles(aligned, 'noise')}"
            )


if __name__ == "__main__":
    main()
