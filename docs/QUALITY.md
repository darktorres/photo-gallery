# Quality

Measured results moved here from the former `FINDINGS.md` log. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [TEST_SET.md](TEST_SET.md); the old titles are listed in [FINDINGS.md](FINDINGS.md).

## Quality: true negatives and new candidates (subset, 2026-10-29)

`evaluate.py` now also counts **cross-group pairs** (915,279 pairs of images from two different
`dups2` groups, i.e. different pictures) above each threshold, the false-positive measure that
replaces `similiar_but_not_dups` (mostly matches now, see Decisions).

| Model | R@1 / R@10 | Thr @95% | Loose dups2 with partner | Cross-group flagged @90% | @95% |
|---|---|---|---|---|---|
| sscd (fp32) | 0.964 / 1.000 | 0.879 | 92.4% | 0 | 0 |
| **sscd_fp16** | 0.964 / 1.000 | 0.879 | 92.4% | **0** | **0** |
| sscd_288_fp16 | 0.964 / 1.000 | 0.874 | 92.5% | 0 | 0 |
| sscd_large | 0.962 / 1.000 | 0.859 | 91.3% | 0 | 2 |
| dinov2_s | 0.962 / 1.000 | 0.945 | 92.8% | 4 | 4 |
| dinov2_b | 0.963 / 1.000 | 0.943 | 93.1% | 0 | 4 |
| clip_b32 | 0.959 / 1.000 | 0.929 | 89.5% | 7 | 17 |
| pdq | 0.924 / 0.957 | 0.234 (d ≤ 98) | 97.9% | 1 | 345 |

- **SSCD flags no different-picture pair at any operating point up to 95% recall**; DINOv2 4,
  CLIP 17. The cross-group set is easy at threshold level (most of it is far apart); the hard
  cases are the few near pairs that k-NN hands the verifier (see the verifier sections).
- **SSCD-large (ResNeXt101, 1024-d): no gain.** Same R@10 = 1.000, slightly fewer loose files
  paired (91.3% vs 92.4%), 2 cross-group false positives at 95%, and 58 img/s end to end here
  (GPU-only 105 img/s, vs 410 for SSCD fp16). Dropped.

### PDQ (`eval/pdq.py`, Meta's 256-bit perceptual hash)

| Hamming distance ≤ | 0 | 8 | 16 | 22 | **31** (Meta's default) | 50 |
|---|---|---|---|---|---|---|
| Labeled pairs found | 32.6% | 65.7% | 74.8% | 80.1% | **82.8%** | 86.8% |

- **Precise but narrow.** At d ≤ 22 (80% recall) it flags 0 cross-group pairs, and at d ≤ 78 only 1.
  But it can't reach 95% recall (that needs d ≤ 98, which flags 345 cross-group pairs and 18.8k
  unlabeled pairs).
- **Breaks on the Pinterest edits:** crop 80% → R@10 0.33, 60% → 0.02, 10% letterbox → 0.11
  (table above). Mirrors and 90° rotations are found only with the 8-variant dihedral query.
- Hashing: 100 img/s with 6 processes in Python (decode-bound; the hash itself is cheap).
- **Role:** a cheap exact/near-exact tier (re-encodes, resizes, grading). ~80% of the labeled pairs
  are this kind of copy, so PDQ (with dihedral queries) could settle those before the embedding
  stage. It can't replace SSCD, and isn't needed for SSCD to work: an optional fast path.

## Quality: model comparison (subset, after label fixes, 2026-10-28)

All six models at the same recall on labeled pairs (higher "loose with partner" is better;
lower "flagged" and "hard neg" are better):

| Model | Thr @95% | Loose dups2 with partner | Flagged unlabeled | Hard neg flagged (of 68,265) |
|---|---|---|---|---|
| sscd (fp32) | 0.879 | 92.4% | 2,653 | 41 |
| sscd_fp16 | 0.879 | 92.4% | 2,653 | 42 |
| sscd_288_fp16 | 0.874 | 92.5% | 2,657 | 41 |
| dinov2_s | 0.945 | 92.8% | 2,742 | 78 |
| dinov2_b | 0.943 | 93.1% | 2,806 | 74 |
| clip_b32 | 0.929 | 89.5% | 2,861 | 78 |

The same pattern holds at 80% and 90% recall (e.g. hard negatives at 90%: SSCD 26,
DINOv2 51–52, CLIP 54).

- **SSCD fp32 = fp16 = 288px in quality.** Precision and size don't matter here, so the
  faster variants are free.
- **SSCD has the best precision:** about half the hard-negative false positives of DINOv2 or
  CLIP at every recall level.
- **DINOv2-B has slightly better recall** on loose dups2 files, but with roughly 2× the false
  positives. It is better used as a second signal in the verifier than as the primary
  descriptor.
- **CLIP is worst on both counts**, as expected from a semantic model.
- **Retrieval rank doesn't separate the models at subset size:** R@1 ≈ 0.96 and R@10 = 1.000
  for all of them. Differences show only in threshold behavior. Expect rank to separate them
  on the full 190k set.

## Quality: SSCD fp16 detail (subset, after label fixes)

- **Labeled pairs:** R@1 **0.964**, R@10 **1.000**. Every labeled partner is in the top 10.
- **Operating points:**

| Target recall | Threshold | Loose dups2 with partner | Flagged unlabeled pairs | Hard negatives flagged |
|---|---|---|---|---|
| 0.90 | 0.909 | 88.3% | 2,496 | 26 / 68,265 (0.04%) |
| 0.95 | 0.879 | 92.4% | 2,653 | 42 / 68,265 (0.06%) |

Flagged unlabeled pairs include real but unlabeled duplicates. Precision is unknown until
`review_*.csv` is labeled.

- **Score separation (pre-fix run, same subset):** hard negatives p99 ≈ 0.30; positives
  p10 ≈ 0.89.
- **What it misses (viewer, scores 0.48–0.78):** strong color grading (e.g. yellow vs
  orange shirt), slight re-crops/zooms of the same photo, AI-image variants with small
  differences.

### Why some duplicates score low and some non-duplicates score high (2026-10-28)

Looked at the extremes of SSCD fp16 on the subset (`misses_*.csv`, `hard_negatives_*.csv`):

- **Highest non-duplicates (0.94–0.99, CLIP up to 0.996):** the *same* picture with a
  **local content edit**: FaceApp face retouch, AI regenerations or inpainting that change a
  hand, face or detail. The rest of the image is pixel-identical.
- **Lowest duplicates (0.48–0.78):** the same photo under a **global edit**: heavy color
  grading (yellow shirt and grass → orange shirt and dark grass, 0.48), re-crop or zoom plus
  grading (0.57–0.70).

A global embedding measures *how much of the image changed*. A local edit changes little, so it
scores high. A strong grade changes every pixel, so it scores low. This library's definition is
the opposite: global edits = duplicate, local content edits = not a duplicate. Copy-detection
models like SSCD are trained to *ignore* local edits (overlays, pasted content), so this is not
a calibration problem. **No threshold, normalization or better embedding model fixes it.** It
needs a stage that aligns the two images and looks at *where* they differ (see
`PLAN.md` / the verifier decision below).


### Shortcut and grouping (`eval/group.py`, k-NN set, subset, 2026-10-29)

**Shortcut** (PDQ ≤ 22 with both hashes reliable, or SSCD ≥ 0.95): 2,896 of the 8,344 k-NN pairs (35%), and
the fixed verifier matched **every one** of them, so skipping it changes nothing. `verify.py --shortcut` now
does that in pipeline mode (decision "match", `shortcut` = pdq / sscd, no image work).

**Grouping.** Each candidate pair becomes a signed edge: +1 for the shortcut, the verifier's decision weighted
by its noise margin when it compared the pair, and nothing when it couldn't align it (two crops of one picture
that don't overlap say nothing about each other). 7,300 of 8,344 pairs carry evidence: 3,908 match, 3,392
"aligned but different".

| Method | Groups (2+) | Largest | Labeled pairs precision / recall | Folders merged | Folders split | Loose files grouped |
|---|---|---|---|---|---|---|
| Connected components (union-find) | 2,869 | 17, 17, 7 | 0.9887 / 1.0 | 125+673, 670+671 | 0 | 4,458 / 4,574 |
| Min-cut split of conflicts (tried, dropped) | 2,871 | 17, 17, 7 | 0.9943 / 0.9986 | 670+671 | 1 | 4,457 |
| **Multicut (GAEC)** | 2,874 | 13, 12, 7 | **0.9943 / 1.0** | 670+671 | 0 | 4,458 |
| Multicut + candidate motion rule | 2,847 | 12, 9, 6 | **1.0 / 0.9986** | none | 1 (429) | 4,434 |

- **Connected components chain:** the two 17-image groups were a mirror-selfie series (one room, different
  outfits and poses) and AI portraits of different faces on one template. One false edge or a dense run of
  borderline ones is enough.
- **Min-cut on conflicts** (cut a group between two images the verifier aligned and called different, when at
  most one match edge joins them) fixed 125+673 but split folder 125: with equal edge weights any bridge is a
  minimum cut, often the wrong one. Replaced by:
- **Multicut** (correlation clustering, greedy additive edge contraction, Keuper et al. 2015): merge the two
  clusters with the largest positive summed weight until none is left, so a match is outvoted by the conflicts
  it would bring in. Fixes 125+673 without splitting anything, shrinks both chains.
- **What's left is the verifier's decision, not the grouping:**
  - 670+671 (different frames of one beach shoot) match at noise 0.02 with `moved_frac` ≈ 0.03; the motion rule
    separates them, but also splits folder 429, an AI image whose copy has a slight body warp (`moved_frac` 0.07).
    Frames vs. local warps is exactly what the hand marks have to decide.
  - The AI-portrait group (12) matches at noise 0.03–0.05 with low motion and interior change: same pose,
    framing and background, different face. Median noise can't see a different face; a face-region or
    learned decision could (see "duplicate vs variant" in Pending).
- Cost: grouping 8.3k pairs takes about a second (pure Python).

### Synthetic edits, all models (subset, 2026-10-29)

300 random images, each edit applied, then the edited copy is searched in the 9.3k-image subset.
Each cell is **R@10 · share above the model's 95%-recall threshold**. R@1 is lower for every model
and edit (0.75–0.93, even for JPEG q30) because exact duplicates of a sampled image already exist
in the index and tie or win rank 1, so R@10 is the retrieval measure here.

| Edit | sscd_fp16 | sscd_large | dinov2_s | dinov2_b | clip_b32 | pdq |
|---|---|---|---|---|---|---|
| JPEG q30 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.99 | 1.00 · 1.00 |
| Downscale 25% | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.99 | 1.00 · 0.76 | 1.00 · 1.00 |
| Grayscale | 1.00 · 0.97 | 1.00 · 0.96 | 1.00 · 0.60 | 1.00 · 0.81 | 0.99 · 0.02 | 1.00 · 1.00 |
| Color shift | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.99 | 1.00 · 1.00 |
| Text overlay | 1.00 · 0.00 | 1.00 · 0.00 | 1.00 · 0.80 | 1.00 · 0.73 | 0.97 · 0.00 | 1.00 · 1.00 |
| Letterbox 10% | 1.00 · 0.00 | 1.00 · 0.09 | 1.00 · 0.55 | 1.00 · 0.66 | 1.00 · 0.60 | 0.11 · 0.06 |
| Mirror | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.65 | 1.00 · 0.98 | 1.00 · 0.99 | 0.00 · 0.00 ¹ |
| Rotate 90° | 1.00 · 0.00 | 1.00 · 0.01 | 0.95 · 0.00 | 0.95 · 0.00 | 0.99 · 0.01 | 0.00 · 0.00 ¹ |
| Crop 80% | 1.00 · 0.77 | 1.00 · 0.98 | 1.00 · 0.70 | 1.00 · 0.83 | 1.00 · 0.90 | 0.33 · 0.26 |
| Crop 60% | 1.00 · 0.00 | 1.00 · 0.09 | 1.00 · 0.30 | 0.99 · 0.38 | 0.99 · 0.49 | 0.02 · 0.01 |
| Crop 40% | 1.00 · 0.00 | 1.00 · 0.00 | 1.00 · 0.00 | 0.95 · 0.04 | 0.97 · 0.08 | 0.00 · 0.00 |
| Crop 25% | **1.00** · 0.00 | **1.00** · 0.00 | 0.95 · 0.00 | 0.80 · 0.00 | 0.85 · 0.00 | 0.00 · 0.00 |
| Crop 50% + mirror | 1.00 · 0.00 | 1.00 · 0.00 | 1.00 · 0.00 | 0.99 · 0.10 | 0.99 · 0.20 | 0.00 · 0.00 |

¹ PDQ queried with all 8 rotations/mirrors (`compute_dihedral`): mirror and rotate 90° → R@10 1.00,
d p50 0, within d ≤ 31 99.7%. The other rows don't change.

- **Only SSCD keeps every edited copy in the top 10, down to a 25% crop.** DINOv2 and CLIP fall
  to 0.80–0.95 on heavy crops and rotation. Rank is what the pipeline uses (k-NN + verifier), so
  this is the deciding result: SSCD stays the candidate model.
- **No global threshold survives heavy crops, overlays or letterboxing** for SSCD: the
  95%-recall threshold (0.879) is set by labeled pairs, which are mostly light edits. Confirms
  design conclusion 2 (rank + verifier, not a lower threshold) and 3 (trim borders first).
- **SSCD-large vs SSCD:** same retrieval; slightly higher similarity on crops (crop 80% above the
  threshold 0.98 vs 0.77). Not enough to justify 7× the cost (below).

### Synthetic edits, SSCD fp16 (preliminary, superseded by the table above: 100 images, 4.7k-image index)

| Edit | R@1 | Cos p50 | Verdict |
|---|---|---|---|
| JPEG q30 / downscale 25% / grayscale / color shift | 0.96–1.00 | 0.92–0.99 | handled |
| **Mirror (hflip)** | 0.96 | 0.95 | **handled** |
| Crop 80% / 60% | 0.96 / 0.93 | 0.90 / 0.81 | handled / mostly |
| Crop 40% / 25% | 0.90 / 0.89 | 0.67 / 0.54 | **below threshold, but still ranked #1–10** |
| Text overlay | 0.96 | 0.75 | borderline |
| Letterbox 10% | 0.93 | 0.75 | borderline |
| Rotate 90° | 0.94 | 0.73 | not handled at the threshold |


## Bug: R@k undercounted groups of 3+

- **R@k metric undercounted groups of 3+:** another member of the same group ranking ahead of
  the scored partner counted as a miss (R@1 read 0.935 for every model). Fixed so only
  images outside the group count as ranked ahead.
