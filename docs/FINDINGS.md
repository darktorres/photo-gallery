# Findings log

Measured results and decisions from M0 (see `PLAN.md`). Newest entries at the top of each
section. Every number states where it was measured; "subset" = the quick-test subset
(`eval/make_subset.py`: all labeled files + 3,000 random others, ~9.3k images).

Raw outputs live in `D:\PhotoGallery\eval_smoke` (subset) and `D:\PhotoGallery\eval` (full set,
not run yet). Browse them with `eval/viewer.py`.

---

## Environment

| Item | Value |
|---|---|
| CPU / RAM | i7-8700 (6C/12T, AVX2), 64 GB |
| GPU | RTX 2060, 6 GB, sm_75 |
| Library | E: 22 TB Exos HDD (21.4 TB used); C: has ~60 GB free after the 2026-10-28 cleanup (keep everything off it) |
| Fast storage | D: 2 TB NVMe |
| Eval Python | `D:\PhotoGallery\venv-eval` (torch 2.11 + CUDA 12.8); global Python's torch is CPU-only |
| Port 8765 | Used by the user's `servidor_teste.py`; the viewer takes the next free port (8766) |

## Test set (2026-10-28)

`E:\Wow!™\_Pinterest_torresdark\pinterest`, copied to `D:\PhotoGallery\datasets\pinterest`.

- 189,528 files, 30 GB, average 170 KB (p50 126 KB, p99 548 KB).
- 98.9% images (187,614 `.jpg`), plus ~1.7k videos (excluded for now).
- **98.4% of files are `.mozjpeg.jpg` re-encodes**, so byte-identical matching finds few
  duplicates. The decoded-pixel hash and the perceptual tiers matter more.

### Labels

- `dups2/<NNN>/`: matched duplicate groups. After fixes: **678 groups, 1,363 files, 702 pairs**.
- `dups2/*` (loose): **4,574 files**, each with a duplicate among the other loose files, unsorted.
- `similiar_but_not_dups/`: **370 files** that other tools wrongly flagged; ~68k non-duplicate pairs.

**Label fixes (2026-10-28, applied on both D: and E:, verified identical):** 7 folders each
held two unrelated duplicate pairs. The second pair was moved to a new folder:
075→672, 076→673, 127→674, 174→675, 175→676, 266→677, 281→678.

**2026-10-29:** 090 → 679 (two pairs of different frames of one shoot; E: done, D: after the k-NN
run; `files.txt` paths patched in place so embeddings stay row-aligned). Now 679 groups and
698 labeled pairs.

**Still open:** 9 single-file folders (230, 348, 416, 417, 423, 530, 538, 540, 628). Their
partner is not among the loose `dups2` files (best match ≤ 0.40), so it may be in `Photos/`
or deleted. Finding it needs a full-set run.

## I/O

- **HDD read order (`eval/bench_hdd.py`, 2026-10-29): read in MFT order, or keep ≥ 8 reads in
  flight. Either gives ~3× path order and reaches the disk's sequential speed.** Unbuffered reads
  (no Windows cache), each order on its own block of 7,500 consecutive files of the test folder on E:
  (`--dense 7500`):

  | Order | files/s | MB/s | 70M files, I/O only |
  |---|---|---|---|
  | path (sorted walk), 1 read at a time | 557 | 64 | 1.5 days |
  | **MFT record number** | **1,699** | **228** | **0.5 days** |
  | physical cluster (LCN) | 1,657 | 234 | 0.5 days |
  | path, 8 reads in flight (drive NCQ reorders) | 1,603 | 235 | 0.5 days |
  | physical cluster, 8 in flight | 1,399 | 252 | 0.6 days |
  | random | 188 | 28 | 4.3 days |

  - Even though path, MFT and cluster order almost coincide in this folder (rank correlation
    0.996–0.999), path order one read at a time loses 2/3 of the speed. Small local reorderings
    each cost a disk rotation.
  - **Caveat: this folder was copied in one go, so it is nearly ideal** (1.2% of files fragmented).
    Older parts of the 70M library are likely more scattered, which favors MFT or cluster order
    even more. The Rust scanner should read the MFT (or USN journal), sort by record number, and
    keep a deep read queue.
  - Sparse sample (every 25th file, `--stride 25`): every order runs at ~140 files/s (random 125),
    and 8 in flight gives ~245. When files are far apart, each read pays a seek whatever the order;
    only queue depth helps.
  - At ~1,600 files/s the disk is no longer the bottleneck for a 70M pass; decoding is.
- **HDD read, path order (robocopy E:→D:): 70 MB/s, ~410 files/s** (consistent with the 557 files/s
  above, as robocopy also writes).
- **The harness embeds at only 80–117 img/s against 410 img/s of GPU capacity**, so decoding
  (PIL, Python DataLoader) is the bottleneck. The Rust decode path matters more than the model.

### Decode benchmark (`eval/bench_decode.py`, subset, 2026-10-29)

1,000 random JPEGs (median 0.96 MP, 120 KB), each turned into SSCD's 320×320 input, files already in
RAM, machine otherwise idle. Cosine = SSCD fp16 embedding vs the same image through `pil_full`.

| Pipeline | img/s, 1 process | img/s, 12 processes | Cosine vs reference: min / p1 / p50 |
|---|---|---|---|
| PIL full decode + bilinear (current harness, reference) | 65 | 263 | 1 / 1 / 1 |
| **PIL `draft()`: libjpeg-turbo DCT-scaled decode + bilinear** | **123** | **550** | 0.973 / 0.992 / 0.999 |
| libvips `thumbnail` (shrink-on-load + Lanczos3) | 78 | 293 | 0.920 / 0.972 / 0.993 |
| nvJPEG batched on the GPU + antialiased GPU resize | 267 | – (one process) | 0.991 / 0.995 / 0.999 |

- **DCT-scaled decode is the winner: 2.1× full decode on the CPU, nearly the same embedding.** In
  Rust: libjpeg-turbo's scaling factors (`turbojpeg` crate), the same mechanism.
- **nvJPEG** gives the most faithful fast output but only 267 img/s. The 2060 has no hardware JPEG
  engine, so Huffman decoding still runs on the CPU, and it competes with SSCD for the GPU.
- **libvips is the slowest fast option here and moves embeddings the most** (min cosine 0.92):
  Lanczos3 sharpens differently from the bilinear antialias the model was evaluated with. It may
  speed up with `VIPS_CONCURRENCY=1` per process (12 processes each starting vips threads
  oversubscribe the CPU). Not pursued, since `draft()` already wins.
- **The rule that matters: index and query images must go through the same decode + resize.** A
  0.97–0.99 cosine shift is harmless when it applies to every image, and harmful when only one side
  of a pair gets it.
- At 550 img/s, decoding 70M images takes ~1.5 days on the CPU. The disk (~1,600 files/s in MFT
  order) and SSCD fp16 (410 img/s) are now within the same range, so the pass is GPU-bound at
  ~2 days.

## Model speed (RTX 2060, GPU only, synthetic input, batch 64)

| Model | img/s | GPU days for 70M |
|---|---|---|
| SSCD fp32 320px | 241 | 3.4 |
| **SSCD fp16 320px** | **410** | **2.0** |
| SSCD fp16 288px | 499 | 1.6 |
| SSCD fp16 256px | 653 | 1.2 |
| SSCD large (ResNeXt101) fp32 | 105 | 7.7 |
| DINOv2-S fp16 224px | 1020 | 0.8 |
| DINOv2-B fp16 224px | 356 | 2.3 |
| CLIP B/32 fp16 224px | 1498 | 0.5 |

TensorRT and int8 are not tried yet.

## Bugs found

- **Meta's SSCD TorchScript breaks in fp16:** `.half()` collapses every image to the same
  vector (backbone relative error ~1.0). Fixed by rebuilding SSCD as a plain module
  (`models.SSCD`) with the weights remapped from ClassyVision to torchvision names.
  Verified against the TorchScript: fp32 max |diff| 1.3e-7; fp16 autocast cosine ≥ 0.999985.
  GeM pooling runs in fp32 (x³ overflows fp16). This module is also the ONNX export source.
- **R@k metric undercounted groups of 3+:** another member of the same group ranking ahead of
  the scored partner counted as a miss (R@1 read 0.935 for every model). Fixed so only
  images outside the group count as ranked ahead.
- **Viewer port collision:** Python's `HTTPServer` sets `SO_REUSEADDR`, which on Windows lets
  it bind a port another server already uses and lose requests silently. The viewer now
  binds with `SO_EXCLUSIVEADDRUSE` and falls back to the next free port.

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

### Verifier prototype, first run (subset, 2026-10-28)

`eval/verify.py`:

1. DISK + LightGlue matching, also against the mirrored image.
2. MAGSAC homography.
3. Warp onto a 512 px canvas.
4. Robust quadratic color fit.
5. Analyze the residual: a changed region ≥ 0.3% of the overlap, or a median residual > 0.03, means
   variant.

Ran on all 702 labeled duplicate pairs + the 345 `similiar_but_not_dups` pairs with SSCD ≥ 0.4,
at 1.3 pairs/s:

| Label | → duplicate | → variant | → unrelated |
|---|---|---|---|
| duplicate (702) | 666 (94.9%) | 36 | 0 |
| not duplicate (345) | 40 | 305 | 0 |

- ~~Every non-duplicate above 0.4 aligns geometrically (0 unrelated)~~: wrong, the "≥ 20 inliers"
  alignment test also passes unrelated pictures (see "match vs unrelated" below). With the
  corrected rule, 253 of these 345 match (variants) and 92 don't.
- **Not yet better than SSCD alone** at the same recall (SSCD at 95% recall: 42 non-duplicates
  through). The errors fall into three groups:
  - **Small face edits pass as duplicates** (e.g. eyes opened/closed, smile changed, AI face
    retouch). The changed regions are ~0.01–0.05% of the image, smaller than the spurious
    regions on true duplicates. Re-running at full resolution doesn't fix it: the change is
    found, but a size threshold can't separate it.
  - **Overlays make duplicates look like variants:** a TikTok watermark added to one copy.
  - **dups2/090 are probably different frames of one shoot** (the aligned residual shows a
    doubled silhouette), not one photo re-edited: a labeling question for the user. (Confirmed and
    split 2026-10-29.)
- (Superseded the same day: duplicate and variant are now one "match" class; under that rule
  all 1,047 pairs are matches and 0 unrelated.) Deferred next step: replace the hand-set
  thresholds with a classifier trained on these labels (per-region
  features: size, peak residual, shape, distance to the border, text-likeness, inside a face?),
  plus a face-focused high-resolution comparison, since face edits are the dominant variant type.

### Verifier: match vs unrelated, with a negative set (subset, 2026-10-28)

**Negative set:** pairs from two different `dups2/<NNN>` groups. Only 250 of ~915k such pairs
reach SSCD 0.4 and 4 reach 0.8. Used every pair ≥ 0.3: 4,628 pairs, 57 min.

- **Inlier count alone fails badly.** With "≥ 20 inliers = match", **2,720 of 4,628 cross-group
  pairs matched (59%)**. DISK + LightGlue + MAGSAC finds a homography with 20–120 inliers between
  unrelated pictures (similar poses, backgrounds, AI style).
- **The residual after alignment separates them:**

| Measure (p1 / p10 / p50 / p90 / p99) | Labeled duplicates | Similar folder | Cross-group |
|---|---|---|---|
| Inliers | 424 / 1151 / 1455 / 1735 / 2047 | 32 / 61 / 224 / 1172 / 1504 | 0 / 0 / 25 / 60 / 118 |
| Inlier ratio | .66 / .89 / .96 / .99 / 1.0 | .15 / .23 / .45 / .92 / .98 | 0 / 0 / .22 / .33 / .50 |
| Median residual (aligned only) | .001 / .003 / .006 / .009 / .025 | .005 / .010 / .046 / .111 / .263 | .092 / .187 / .255 / .343 / .427 |

- **New rule:** match if median residual ≤ 0.06, **or** ≥ 150 inliers making up ≥ 50% of the
  matches (the second clause catches extreme grades the color model can't undo, e.g. dups2/090).
  ~~e.g. dups2/090~~ **Correction 2026-10-29:** dups2/090 was not a color grade but two pairs of
  different frames of one shoot (residual 0.13, see "Same-shoot frames" below). The second clause
  existed only for it and is now off by default.
  - Labeled duplicates: **702 / 702 matched**.
  - Cross-group: **5 / 4,628 matched**. Four of them are dups2/670 vs 671: two frames of the same
    beach shoot (identical background, different pose). The fifth (125 vs 673) is the same
    person, a different photo, on a plain background.
  - Similar folder: 253 matched, 92 unrelated. The unrelated ones are mostly AI regenerations of
    one composition with a different face or hair color, plus different photos of the same person;
    a few are close regenerations that could count as variants.
- ~~Open labeling question~~ resolved 2026-10-29, see below.
- `verify.py --rescore` re-applies thresholds to stored measurements in seconds (no image work).

### Same-shoot frames are different pictures (2026-10-29)

**User decision:** different frames of one shoot (same place and outfit, different pose or moment)
are *different pictures*, not matches. dups2/090 held two such pairs and was split (090 + 679).

- **The strong-inlier clause is dropped** (`--strong-inliers 0` by default). With 090 split, no
  labeled duplicate needs it (all have median residual ≤ 0.051); it matched only 090's cross-frame
  pairs (residual 0.13), 21 similar-folder pairs (residual 0.06–0.13) and 125 vs 673.
- **Still matched: 670 vs 671**, same beach, different pose. The median residual is low (0.019)
  because the background dominates. The changed area does not separate it by itself: 9% changed vs
  up to 14% on real duplicates. But **where** differs:
  - On duplicates, large changed areas sit **at the frame edges**: phone status bars, UI icons,
    frame lines (dups2/426 with 14% changed is a screenshot with a status bar).
  - On 670/671 the changes are **inside**: the body, the hat strings, palm leaves in the wind.
- `verify.py` now measures `interior_changed_frac` (changed share of the overlap, minus an 8% edge
  band) and `interior_regions` (separate changed regions inside). Diagnostic until the reruns show
  whether a threshold separates same-shoot frames from local-edit variants (a face retouch also
  changes the interior, but in one compact region).

**Reruns after the split (subset, 2026-10-29), strong clause off:**

| Set | Match | Unrelated |
|---|---|---|
| Labeled duplicates | **698 / 698** | 0 |
| Cross-group (4,632 pairs ≥ 0.3) | 5 (670/671 ×4, 125/673) | 4,627 |
| Similar folder | 231 | 114 |
| k-NN candidates: duplicate / cross-group / similar / unlabeled | 698 / 5 / 224 / **2,973** | 0 / 200 / 98 / **4,146** |

(k-NN = each image's 10 nearest SSCD neighbors ≥ 0.4: 8,344 pairs, 80 min. Before the split, with
the strong clause: unlabeled 2,985 / 4,134.)

**Interior changes don't separate same-shoot frames on their own:**

- 670/671 have 25–32 changed regions inside; labeled duplicates at most 13. "≥ 15 regions or > 15%
  interior change" flips 0 duplicates, 52 similar-folder and 58 unlabeled k-NN matches to unrelated.
- Looking at 20 of the 58 flipped k-NN pairs: **~11 correct** (front vs back of a cosplay, other
  poses, other outfit in the same room: matched only because the background dominates the median
  residual), **~5 wrong** (heavily compressed copies whose blocking makes many small regions; a
  FaceApp beautify, a hair recolor, a face swap: local edits, which count as matches), ~4 ambiguous.
- So **the median-residual rule also matches different poses** in front of the same background,
  beyond 670/671. That's a real precision problem on candidates, not just a labeling detail.
- **Better signal: motion.** A different moment *moves* things (limbs, hair, cloth: coherent
  displacement after alignment); an edit or recompression changes appearance *in place* (no
  displacement). `verify.py` now runs dense optical flow (OpenCV DIS, both directions) between the
  aligned, color-mapped images. A changed pixel counts as **moved** if the flow there is > 2 px,
  forward-backward consistent (< 1 px) and warping by it at least halves the residual. Output:
  `moved_frac` (share of the overlap), `moved_share` (share of the change that motion explains),
  green outlines in the diagnostic.

  First test, 63 pairs with a by-eye truth:

  | Pairs | moved_frac |
  |---|---|
  | 30 random labeled duplicates | **0.000** (all) |
  | 8 duplicates with large edge changes (status bars, frames) | 0.000–0.001, except 448 (0.075) |
  | Recompressed copies, recolor, beautify, face swap (same picture) | 0.000–0.008 |
  | 670/671 (same beach, other pose) | **0.027–0.035** |
  | Other frames of a shoot (2543, 6917, 7007, 224) | 0.017–0.196 |
  | Different outfit/person/pose in the same place (6210, 6317, 6489, 7388, 7640, 5426) | 0.000–0.012 |

  - **Motion catches small pose changes** (670/671 and similar), which nothing else did, and is
    zero on every plain duplicate.
  - **It misses big changes**: when the whole subject differs, flow can't explain it, so nothing
    "moves". Those have a large interior change instead (0.17–0.30), so the two measures complement
    each other: roughly "unrelated if moved > ~1.2% or interior change > ~15%" gets 16 of 17
    different-picture pairs here.
  - **Gray zone:** 448 is a body-reshape (liquify) edit, a real local geometric change, so it moves
    like a pose change; a hair recolor (5152) changes 18% of the interior. Whether such edits are
    "the same picture" is a definition question, and 63 pairs are too few to set thresholds.
- **Next: hand marks.** The viewer's verifier tabs now have "Same picture (S)" / "Different picture
  (D)" marks per pair (`pair_marks.json` in the run folder, keyed by the image pair, kept across
  reruns), filters for unmarked / disagreements, and the Summary shows how often the verifier agrees
  with the marks. `--max-moved` and `--max-interior-change` (off by default) are then tuned on the
  marks with `--rescore`.

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

## Border cleaning (`tools/borders.py`, 2026-10-28)

Finds solid (incl. JPEG-noisy colored) and blurred-padding borders and crops them; dry run by
default, writes cleaned copies only with `--apply`, never touches originals. Default output is a
lossless `jpegtran` crop (left/top snapped to the 8/16 px JPEG block).

Sample of 1,500 random test-set images (79 img/s, 4 workers):

- **35 with borders (2.3%)**, all correct on visual review: black letterbox bars, thin
  (5–20 px) dark frames around AI art, white side padding, a purple frame.
- **Rejected look-alikes:** plain studio walls, skies, and white backgrounds. Rules that made the
  difference:
  - **Real padding is digitally flat:** median deviation 0–3 and line color drift ≤ 5; walls
    have 5–7 and 9–17.
  - **Blurred bars have a straight seam (≥ 0.7) and come in equal pairs:** a one-sided "blurred
    bar" is always window light, curtains or a horizon.
- **Detection only on stored pixels**, so EXIF orientation stays valid after cropping.
- **For the NDD pipeline:** the same detector should run before embedding (design conclusion 3).

## Decisions

- **2026-10-28 (revised the same day): no duplicate / variant distinction for now.** Local-edit
  variants (retouch, inpainting, AI regeneration) and duplicates are one relation, "match". The
  verifier decides match vs unrelated; its residual analysis stays as a diagnostic, for when the
  distinction is revisited. (Briefly decided earlier: variants as a separate third relation.)
  - Consequence: **`similiar_but_not_dups` is no longer a negative set.** Most pairs in it above
    SSCD 0.4 are variants = matches (253/345 with the corrected verifier rule). The "hard negatives
    flagged" numbers in the model comparison above therefore count matches, not errors, and a
    new source of true negatives is needed.

- **2026-10-28: quality over speed.** Days or weeks of processing are acceptable. Heavier
  models (SSCD-large, fp32), exact rather than approximate k-NN (very rough estimate: about
  3 days brute force for 70M on the 2060, streaming from the NVMe), multiple embeddings per
  image, and a learned geometric verifier are all in scope. Resumability and the user's
  review time (precision) remain the real constraints.
- **2026-10-28: stay on Windows.** CUDA, nvJPEG, PyTorch, TensorRT and ONNX Runtime all work.
  Linux-only tools (FAISS-GPU, cuVS, DALI) aren't needed so far. If one ever is, run only that
  batch step in WSL2 on embeddings, not on images: reading NTFS drives through `/mnt` is slow.
- **2026-10-28: add a geometric verification stage**, local feature matching + RANSAC (e.g.
  SuperPoint/LightGlue, LoFTR), for pairs in the uncertain band only. It checks heavy crops,
  overlays and false positives, and reports which image contains the other. Open risk: AI
  variants with redrawn details may have few matchable points.
- **2026-10-28: benchmark decoders** (PIL full, PIL `draft()`, libvips, nvJPEG) for speed and
  embedding drift. Done 2026-10-29, see "Decode benchmark": DCT-scaled decode wins.

## Design conclusions so far

1. **Mirrors:** SSCD is flip-tolerant, so there is no need to store a flipped embedding
   (resolves PLAN §9 decision 3, pending confirmation on the full set).
2. **Heavy crops:** a single global threshold fails, but rank still works. Use k-NN rank plus a
   verifier for the ~0.4–0.88 band (Pinterest-style), not a lower global threshold.
3. **Borders:** trim borders before embedding, including blurred padding bars (a common
   Pinterest/TikTok edit seen in `dups2/075`).
4. **Rotation:** SSCD still ranks a 90°-rotated copy in the top 10 (R@10 1.00), but far below
   threshold, so rank + verifier covers it (the verifier would need to try rotations as it tries
   mirrors). Apply EXIF orientation everywhere; PDQ's dihedral query covers the exact-copy tier.
5. **Throughput:** read in MFT order with a deep queue (~1,600 files/s), decode with DCT scaling
   (~550 img/s on 12 threads), embed with SSCD fp16 (410 img/s). The GPU is the limit: about 2 days
   for 70M. SSCD fp16 is the model; 288px is the same quality (R@10, cross-group 0) if speed is
   ever needed. SSCD-large, DINOv2 and CLIP are dropped as candidate models.
6. **PDQ** is an optional cheap tier for near-exact copies (~80% of labeled pairs at d ≤ 22 with
   0 false positives), never a replacement for SSCD.

## Pending

- Verifier on real k-NN candidates (`verify.py --pairs knn`, running): precision needs the
  unlabeled pairs judged by eye.
- (Deferred) duplicate vs variant: learned decision on per-region features + face-focused
  comparison (see "Verifier prototype, first run").
- Hand-label a sample of `review_*.csv` to estimate real precision.
- TensorRT / int8 for SSCD (only if the GPU-bound ~2 days ever matters).
- IVF vs exact k-NN (needs the full set).
- Full-set runs (on hold until the user says go).
