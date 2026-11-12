# Verifier

Measured results moved here from the former `FINDINGS.md` log. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [TEST_SET.md](TEST_SET.md); the old titles are listed in [FINDINGS.md](FINDINGS.md).

## Verifier after the resampling fix (262004b, re-pick, 2026-11-08, subset)

The verifier fix (PIL LANCZOS resampling, and the blur scaled by `ref_scale` / `other_scale`) is in commit 262004b. Old
and new figures from the same work dir: the pre-fix tree is 22ae0ff (`262004b^`), the post-fix tree is HEAD. The runs
are in `D:\PhotoGallery\eval\archive-prefix-22ae0ff\` and `archive-postfix-262004b-HEAD\`. The pre-fix labeled run
reproduces the recorded labeled baseline exactly. Sets: labeled (1043 pairs); cross-groups (4637 pairs, floor 0.3);
k-NN (8450 pairs on a 9,307-image subset, built by selecting rows of the full-set embeddings by path).

| Set | Pre-fix | Post-fix |
|---|---|---|
| Labeled | duplicates 698/698; similar 237 match / 108 unrelated; duplicate noise p99 0.0171 | duplicates 698/698; similar 236 / 109; duplicate p99 0.0171 |
| Cross-groups | 7 matched; aligned noise p1 0.0858 | 8 matched; aligned p1 0.0852 |
| k-NN | duplicates 698/698; cross 7 of 207 matched; similar 225 / 97; unlabeled 2981 / 4242 | duplicates 698/698; cross 7 of 207; similar 225 / 97; unlabeled 2978 / 4245 |

The criterion window (duplicate p99 to cross-group p1) is [0.0171, 0.0858] before the fix and [0.0171, 0.0852] after.
The fix barely moves it.

Sweep of `--max-noise` on the post-fix data (`verify.py --rescore`, all three sets):

| max-noise | duplicates unrelated (of 698) | cross matched (of 4637) | k-NN cross matched (of 207) | labeled similar match / unrelated | k-NN unlabeled match / unrelated |
|---|---|---|---|---|---|
| 0.02 | 6 | 3 | 3 | 96 / 249 | 2880 / 4343 |
| 0.03 | 2 | 4 | 4 | 124 / 221 | 2914 / 4309 |
| 0.04 | 1 | 4 | 4 | 156 / 189 | 2936 / 4287 |
| 0.05 | 1 | 7 | 6 | 197 / 148 | 2958 / 4265 |
| **0.06** | **0** | **8** | **7** | **236 / 109** | **2978 / 4245** |
| 0.07 | 0 | 12 | 9 | 264 / 81 | 2998 / 4225 |
| 0.08 | 0 | 21 | 11 | 283 / 62 | 3024 / 4199 |
| 0.085 | 0 | 26 | 13 | 297 / 48 | 3043 / 4180 |

Decision: `--max-noise` stays at 0.06 (the default in `eval/verify.py`). It is inside the post-fix window, keeps every
duplicate, and gives 8 cross-group matches. The midpoint of the window (about 0.051) would be 0.05, which loses one
duplicate (697 of 698) for one fewer cross-group match.

Not run: the full 187,808-image k-NN set (hours per run); `verify_synth.py` after the fix.

## Earlier verifier runs

### Verifier figures below predate two fixes (C-44, C-45); the re-pick of `--max-noise` is pending

`eval/verify.py` had two defects, both now fixed in the code:

- **C-44:** the working images were resized with `resample=4`, commented as Lanczos. In Pillow 4 is `BOX`
  (Lanczos is 1), so every working image was box-filtered. It now uses `Image.Resampling.LANCZOS`.
- **C-45:** the effective-resolution blur divided by the scales (`ref_scale`, `other_scale`) where it should multiply:
  they are working px per original px, so canvas px per original px is `c * ref_scale`. It now multiplies.

**Every verifier figure in the sections below was measured with the box filter and the old blur, and has not been
re-measured:** the residual percentiles and the 0.06 rule, the reruns after the split, the rerun of all three sets with
motion, the synthetic-edit table (including the grayscale noise 0.078), and "Verifier fixes and speed" (including the
0.0603 → 0.0599 pair). They are kept as the record of what was run, not as the verifier's current behaviour.

**No pre-fix baseline was taken for the three pair sets (`labeled`, `cross_groups`, `knn`).** The recorded labeled
numbers (1043 pairs; duplicates 698/698; similar_folder 237 match, 108 unrelated; median-residual percentiles
p10/50/90/99: duplicates 0.0032/0.0056/0.0093/0.0171, similar_folder 0.0101/0.0436/0.0999/0.2347) are the only
reference, and they have no `cross_groups` or `knn` counterpart. So no before-and-after comparison exists for the fix.

**`--max-noise` is still 0.06, the value picked on the box-filtered data.** The labeled set has no negatives
(`PAIR_SETS['labeled']` is duplicates and similar_folder, and `EXPECTED` covers only duplicate and cross_group), so it
cannot choose the threshold. The re-pick has not been done. It needs a `verify.py --rescore` sweep on the
`cross_groups` data, with the criterion used above (cross-group p1 of the median residual against duplicate p99:
0.092 against 0.025 in the old data), and a verifier run with the fixed code to produce that data. Until then, treat 0.06
as unvalidated for the fixed code.

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
    doubled silhouette), not one photo re-edited: a labeling question. (Confirmed and
    split 2026-10-29.)
- (Superseded the same day: duplicate and variant are now one "match" class; under that rule
  all 1,047 pairs are matches and 0 unrelated.) Deferred next step: replace the hand-set
  thresholds with a classifier trained on these labels (per-region
  features: size, peak residual, shape, distance to the border, text-likeness, inside a face?),
  plus a face-focused high-resolution comparison, since face edits are the dominant variant type.

### Verifier: match vs unrelated, with a negative set (subset, 2026-10-28)

> **Predates the verifier fix (commit 262004b, 2026-11-07).** These figures are from the verifier before its PIL
> resampling and its scale-corrected blur. The post-fix re-run is in "Verifier after the resampling fix" at the end
> of this file.

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

> **Predates the verifier fix (commit 262004b, 2026-11-07).** The verifier figures in this section (match and motion
> tables) were measured with BOX resampling and the uncorrected blur. Not rerun against the fixed verifier.

**Decision (2026-10-29):** different frames of one shoot (same place and outfit, different pose or moment)
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
- **Rerun of all three sets with motion (subset, 2026-10-29; decisions unchanged, rules off).** Among
  pairs the verifier matches:

  | Set / label | Matched | moved > 0 | moved p99 / max | interior change p99 / max |
  |---|---|---|---|---|
  | Labeled duplicates | 698 | 1.3% | 0.000 / 0.074 (448) | 0.041 / 0.132 |
  | Cross-group (different pictures) | 5 | 100% | all 0.027–0.037 | 0.196 / 0.200 |
  | Similar folder | 231 | 81% | 0.061 / 0.129 | 0.208 / 0.393 |
  | k-NN unlabeled | 2,973 | 6.8% | 0.013 / 0.195 | 0.135 / 0.363 |

  Candidate rules, pairs flipped from match to unrelated (k-NN set; cross-group = known errors,
  duplicate = known losses, the rest unknown until marked):

  | Rule | Cross-group (of 5) | Duplicates (of 698) | Similar folder | Unlabeled |
  |---|---|---|---|---|
  | moved > 0.012 only | **5** | 1 (448) | 34 | 37 |
  | interior > 0.15 only | 1 | 0 | 9 | 21 |
  | **moved > 0.012 or interior > 0.15** | **5** | 1 (448) | 39 | 56 |
  | moved > 0.005 or interior > 0.15 | 5 | 1 | 78 | 109 |
  | moved > 0.02 or interior > 0.20 | 4 | 1 | 24 | 32 |

  - **Motion alone removes every known false match** (all 5, including 125 vs 673, a different photo
    of the same person) at the cost of one labeled duplicate, 448, the liquify edit.
  - Plain duplicates almost never move (98.7% exactly 0). The similar folder moves a lot (81%), as it
    holds AI regenerations and other shots, whose truth is unknown.
  - Whether the 56 unlabeled pairs the combined rule flips are right is the open question: the
    hand marks answer it.
- **Next: hand marks.** The viewer's verifier tabs now have "Same picture (S)" / "Different picture
  (D)" marks per pair (`pair_marks.json` in the run folder, keyed by the image pair, kept across
  reruns), filters for unmarked / disagreements, and the Summary shows how often the verifier agrees
  with the marks. `--max-moved` and `--max-interior-change` (off by default) are then tuned on the
  marks with `--rescore`.

### Verifier on the synthetic edits (`eval/verify_synth.py`, subset, 2026-10-29)

> **Predates the verifier fix (commit 262004b, 2026-11-07).** Measured with the BOX-resampling verifier. `verify_synth.py`
> was not rerun after the fix, so these figures are unverified against it.

The same 300 images and edits as the synthetic retrieval test below; each original verified against each edited
copy (all should match). 3,900 pairs, 39 min.

| Edit | Matched | Why the rest failed |
|---|---|---|
| JPEG q30, color shift, text overlay, letterbox 10%, mirror | 300 / 300 each | |
| Crop 80% / 60% / 40%, crop 50% + mirror | 300 / 300 each | |
| Crop 25% | 297 | 3 never aligned (0 inliers) |
| **Downscale to 25%** | **260** | **40 never aligned:** the copy is tiny (originals p50 862 px → ~215 px); every original ≥ 923 px long side worked |
| **Grayscale** | **210** | **90 aligned perfectly (p50 1,720 inliers) but noise 0.078 > 0.06:** the color model maps the copy onto the original, and gray can't be mapped back to color |
| **Rotate 90°** | **28** | No rotation attempts (only mirror); the 28 are near-symmetric images |

- **Crops, overlays and letterboxing are solved:** the stage SSCD's threshold failed on (a 25% crop scores ~0.54) the
  verifier accepts 99–100% of the time, so k-NN rank + verifier works as designed.
- **Fixes needed** (the first two done, see "Verifier fixes and speed"):
  - **Color fit in both directions** (copy → original and original → copy, keep the better). Also explains the
    real k-NN miss 5172, a heavy grade (white wall → yellow) at PDQ distance 12.
  - **Scale normalization for very different sizes:** match a tiny copy against the original at a comparable
    size, not 1024 px vs ~215 px. Also explains the real miss 6041 (236 px copy of a 1340 px original).
  - **Rotations:** better left to PDQ's 8-orientation query (exact rotated copies, 99.7% within d ≤ 31) than to
    4× the local-feature work; revisit if the full set shows rotated *edited* copies.
- **The candidate motion rule rarely hurts real copies:** 19 of ~3,500 matched edited copies would flip (6
  downscale and 6 rotation by motion, 4 grayscale and 2 text overlays by interior change, 1 crop).

### Which candidate pairs need the verifier (k-NN set, subset, 2026-10-29)

> **Predates the verifier fix (commit 262004b, 2026-11-07).** The verifier counts here (the match and unrelated numbers
> in the lines below) are from before the fix. The post-fix k-NN figures, on a 9,307-image subset, are in "Verifier
> after the resampling fix" at the end of this file.

8,344 k-NN pairs over 9,307 images = **0.9 pairs per image**; 28% of images have none (p90 4, max 48).

| PDQ distance | Pairs | Verifier match | | SSCD cosine | Pairs | Verifier match |
|---|---|---|---|---|---|---|
| ≤ 8 | 27.7% | 100% | | 0.40–0.50 | **48.5%** | **1.2%** |
| 9–22 | 4.7% | 99.5% | | 0.50–0.70 | 6.9% | 26% |
| 23–31 | 1.7% | 98.6% | | 0.70–0.88 | 4.9% | 96% |
| 32–60 | 3.3% | 95.3% | | 0.88–0.95 | 7.1% | 99.8% |
| > 60 | 62.5% | 15.2% | | ≥ 0.95 | 32.7% | 100% |

- **Shortcut: accept PDQ ≤ 22 or SSCD ≥ 0.95 without verifying.** 35% of pairs. The verifier (with the motion
  rule) disagreed on 5 of 2,896, and by eye **the shortcut was right every time**: a heavy grade and a 236 px copy
  the verifier can't handle yet, a FaceApp edit and two AI variants the motion rule flagged. After the shortcut,
  only 31% of images need local features at all.
- **The low band is the cost sink:** SSCD 0.40–0.50 is half the pairs but 1.2% matches; it can't be dropped
  (heavy crops live there). Rejecting on inlier count before the dense stage doesn't help: < 40 inliers removes
  28% of pairs but loses verifier matches and edited copies, and pairs under 20 already skip it.
- **Cost (`profile_verify`, 200 pairs, RTX 2060):**

  | Stage | Cost |
  |---|---|
  | DISK features, image + mirrored | 2 × 109 ms **per image** |
  | LightGlue, as-is + mirrored | 2 × 42 ms per pair |
  | Robust color fit | 81 ms per aligned pair |
  | Misalignment-tolerant residual (25 shifted copies, NumPy) | 102 ms per aligned pair |
  | Optical flow (motion) | 36 ms per aligned pair |
  | MAGSAC | 2 × 3 ms |

  ≈ 240 ms per image + 330 ms per pair. Biggest levers: extract mirrored features only when the upright match
  fails (most pairs aren't mirrored), compute the residual and color fit on the GPU (both are simple array math),
  run the CPU part in parallel workers, and try faster local features (SuperPoint / ALIKED + LightGlue, lower
  resolution) against this same synthetic + labeled + cross-group test.
- Rough scale at the subset's rate (0.9 pairs per image, likely an overestimate, as the subset is enriched with
  labeled duplicates): 63M pairs, 41M after the shortcut. At today's speed that is over a year; with the levers
  above, on the order of weeks. The full set will give the real candidate rate.

### Verifier fixes and speed (subset, 2026-10-29)

> **Predates the verifier fix (commit 262004b, 2026-11-07).** This section describes the fixes before 262004b, which
> the fix itself corrects (the resampling and the blur scale). Its speed figures are not affected by the fix.

Changes in `eval/verify.py`, all four pair sets rerun:

- **Tiny copies:** images under 512 px are upscaled to 512 px before feature extraction. On the 40
  failed 25% downscales: 0/40 → 40/40 (upscaling to the original's working side and downscaling the
  original also gave 40/40, but each cost one 25% crop).
- **Color fit in both directions:** the other image is mapped onto the reference as before; the reverse
  (reference onto the other) is used only when it fits **at least 2× better** (`REVERSE_COLOR_GAIN`).
  Simply keeping the better direction doubled the false matches (cross-group 5 → 10, k-NN cross-group
  5 → 8, all at noise 0.051–0.059): the extra freedom flattens the residual of unrelated pictures too.
  Measured fit ratios: those false matches ≤ 1.56×; grayscale copies ≥ 5.1×; real heavy grades 6–8×; a
  washed-out real copy 2.7×.
- **Mirror only when needed:** the mirrored image is aligned only if the as-is alignment has < 200 inliers
  (45% of k-NN pairs skip it), and retried when a skipped pair then doesn't match: a mirrored copy of a
  near-symmetric picture can align as-is with ~280 inliers and fail.
- **Speed:** color fit by weighted normal equations, the tolerant residual on the GPU (240 → 8 ms), and
  the comparison (color fit, residual, flow) on 4 CPU threads while the main thread extracts and matches
  the next pair. Per image 239 → 127 ms, per pair 334 → 265 ms serial. More workers don't help: the GPU
  main thread is the limit (DISK 61% of it, LightGlue 29%).

| Set | Baseline | Now (DISK) | ALIKED |
|---|---|---|---|
| Synthetic: downscale 25% | 260 / 300 | **300** | 300 |
| Synthetic: grayscale | 210 / 300 | **300** | 300 |
| Synthetic: crop 25% | 297 / 300 | 297 | 300 |
| Synthetic: rotate 90° | 28 / 300 | 28 | **196** (9 of DISK's 28 lost) |
| Synthetic: other 9 edits | 300 each | 300 each | 300 each |
| Labeled duplicates | 698 / 698 | 698 | 698 |
| Cross-group false matches | 5 / 4,632 | **5** | **9** |
| k-NN: cross-group false matches | 5 / 205 | 5 | 7 |
| k-NN: unlabeled matches | 2,973 / 7,119 | 2,979 (+6, none lost) | 3,005 (+32, −6 vs DISK) |
| Throughput (pairs/s; synth / labeled / cross / k-NN) | 1.7 / – / – / 1.5 | 4.9 / 3.4 / 5.0 / 3.7 | 6.5 / 5.4 / 3.0* / 4.9 |

\* the GPU was likely shared during that run; ALIKED extraction alone is 42 ms vs DISK's 116 ms per image
(LightGlue ~30 ms per pair for both; DISK under fp16 autocast is 6× slower, ALIKED fp16 only 8% faster).

- **Both real k-NN misses are fixed:** 5172 (heavy grade) and 6041 (236 px copy) now match, so the shortcut
  disagreements above were the verifier's limits, not the shortcut's.
- **`similiar_but_not_dups`:** +3 matches with DISK, two of them only by noise moving 0.0603 → 0.0599 at the
  0.06 threshold.
- **DISK stays the default.** ALIKED is faster and handles 90° rotations, but finds fewer inliers, so
  borderline pairs move across the noise threshold in both directions and false matches rise (5 → 9).
  Rotation is cheaper to get from PDQ's 8-orientation query or by trying the rotations only when the first
  alignment is weak (like the mirror). `--features aliked` stays available.

### Verifier: TensorRT, rotations, SIFT, resolution (subset, 2026-10-30)

> **Predates the verifier fix (commit 262004b, 2026-11-07).** Measured with BOX resampling. Not rerun against the fixed
> verifier; the figures stand as the record of the pre-fix run.

**DISK on TensorRT.** Only DISK's U-Net runs as a TensorRT 11 fp16 engine (`accel.DiskUnetTRT`, dynamic sides
up to 1024 px); kornia's keypoint selection and LightGlue stay in PyTorch. Per image 35 ms vs 113 ms, 98% the same
keypoints, descriptor cosine 0.9999 (a TensorRT fp32 engine gives no speedup, 108 ms). `--disk-runtime trt` is now
the default. All four sets rerun:

| Set | torch DISK | TensorRT DISK |
|---|---|---|
| Labeled duplicates | 698 / 698 | 698 / 698 |
| Synthetic (13 edits × 300) | 3,625 match | 3,626 (3 rot90 pairs flip, ~20 inliers) |
| Cross-group false matches | 5 / 4,632 | 7 |
| k-NN: decisions changed | – | 30 / 8,344 (20 → match, 10 → unrelated) |
| k-NN: cross-group false matches | 5 / 205 | 7 |

- **The flips show a fragile measurement, not TensorRT drift.** Rerunning the two new cross-group matches (group
  125 vs 673, one shoot) in torch reproduces torch's numbers exactly, so the difference comes from the features.
  For some pairs the median residual swings far more than the threshold margin when a few keypoints change:
  k-NN 978 (similar-folder) 0.272 → 0.033 and 4157 0.014 → 0.076, at equal inlier counts. About 0.3% of pairs
  sit there. That belongs to the decision rule (hand marks), not to the runtime.

**Throughput** (labeled set, 1,043 pairs, quiet machine; the four-set runs above overlapped the M1 session's CPU
load and read 4.3–4.7 pairs/s):

| Configuration | Time | Pairs/s | Decisions |
|---|---|---|---|
| torch DISK (before) | 303 s | 3.4 | – |
| torch DISK, images decoded ahead (`--loaders 4`) | 266 s | 3.9 | identical |
| TensorRT DISK | 177 s | 5.9 | 9 similar-folder pairs flip (net +6 matches) |
| TensorRT DISK, decoded ahead | 175–179 s | 5.9 | identical to TensorRT |
| ... and 8 comparison workers (new default) | 155 s | **6.7** | identical |
| ... at 768 px, 2,048 keypoints (4 workers) | 163 s | 6.4 | duplicates 698; similar-folder 232 (237) |
| ... at 768 px, 1,024 keypoints (4 workers) | 141 s | 7.4 | duplicates 698; similar-folder 226 (237) |

- **Where the time goes now** (py-spy, TensorRT + decoding ahead, 175 s): the main thread is busy 133 s,
  LightGlue 68 s and DISK incl. keypoint selection 51 s. The comparison workers use 275 thread-s, decoding 41.
  Decoding ahead only helped the slower torch path. **LightGlue is the next lever**: on TensorRT (LightGlue-ONNX
  exists), or fewer keypoints.
- **Lower resolution** keeps all labeled duplicates and every synthetic edit within 1–3 images at 768 px (crop 25%:
  298 at 768/2,048 and 295 at 768/1,024 vs 297), but similar-folder matches drop (237 → 226). That is unscored,
  so the cross-group set has to be run at 768 px before choosing it.
- **768 px / 2,048 keypoints, cross-group and k-NN sets (2026-10-31): not adopted, 1024 px stays.** Cross-group
  (4,632 pairs, floor 0.3): 8 false matches vs 7; the new one (groups 162 vs 572) has SSCD similarity 0.339, below
  the 0.4 k-NN floor, the rest are the known one-shoot pairs (670/671, 125/673). k-NN set (8,344 pairs): 52 decisions
  flip (0.6%); duplicates 698 / 698, similar-folder 222 matches (224), unlabeled 3,000 (2,989). The gain is only ~9%
  on a quiet machine (labeled 169 s vs 185 s), because LightGlue and the comparison, not DISK, dominate.

**Rotations.** New synthetic edits (`edits.py`, appended so the older edits keep their seeds): a 3–15° tilt cropped to
the picture, and 30°, 45°, 135° and random 0–360° on an enlarged black canvas. 300 images each:

| Verifier | tilt + crop | 30° | 45° | 135° | any | 90° | Time (1,800 pairs) |
|---|---|---|---|---|---|---|---|
| DISK | 300 | 300 | 283 | 0 | 87 | 29 | 551 s |
| DISK, retry at 90° steps (`--rotations 90`) | 300 | 300 | 298 | 299 | 300 | 300 | (slowed by a concurrent GPU test) |
| DISK, retry at 45° steps (`--rotations 45`) | 300 | 300 | **300** | **300** | **300** | **300** | 1,123 s |
| SIFT (`--features sift`) | 300 | 300 | **300** | **300** | **300** | **300** | 511 s |

- **Retry:** a pair that doesn't match is matched again against B rotated by each step. The rotated copy's
  matched points are mapped back into B, so the homography, color fit and comparison use the images as they are.
  A retry only replaces the result if it matches.
- **No false matches from retries:** the cross-group set with 45° retries has the same 7 matches as without, and no
  rotated attempt matched any of the 4,625 others. 90° steps try a subset of those angles, so they can't add any
  either. **Cost:** every non-matching pair pays 7 extra extractions + matchings (cross-group 3,532 s vs 984 s).
  Retries should therefore be gated, e.g. only for candidates whose SSCD similarity is high for an unaligned pair.
- **SSCD finds rotated copies:** R@10 1.000 for every angle (cosine p10 0.50–0.55, above the k-NN floor 0.4), so they
  reach the verifier as candidates. Rotation is covered end to end with DISK + retries.
- **SIFT** (kornia, RootSIFT, oriented; LightGlue's SIFT weights) as the only features: synthetic 5,400 / 5,400 (all
  18 edits, incl. crop 25%), labeled 697 / 698, cross-group 9 false matches (5 with DISK; 3 new ones are weak, 37–73
  inliers). It misses `dups2/533` (14 inliers; DISK 326) and is slower on real images (labeled 402 s vs DISK's
  177 s, no TensorRT). **DISK stays the default**; SIFT is a possible fallback, but the 45°-step retry is cleaner (no
  new false matches).
