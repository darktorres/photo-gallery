# M0 model bake-off

One-time evaluation (see `docs/PLAN.md` §8, M0; results go in `docs/FINDINGS.md`) to pick the near-duplicate descriptor, its
precision/resolution, and thresholds, measured on the ~190k-image Pinterest test set.
Python is used only here; everything that runs at library scale is Rust.

## Setup

- Data: `D:\PhotoGallery\datasets\pinterest` (copy of `E:\Wow!™\_Pinterest_torresdark\pinterest`)
- Outputs: `D:\PhotoGallery\eval` (embeddings, results, CSVs)
- Model and pip caches: `D:\PhotoGallery\cache` (C: is nearly full)
- Environment: `D:\PhotoGallery\venv-eval` (CUDA PyTorch; the global Python has CPU-only torch)

All paths can be overridden with `PG_DATA_ROOT`, `PG_WORK_DIR`, `PG_CACHE_DIR`.

## Ground truth

- **Positives:** every pair inside `dups2/<NNN>/` (matched duplicate groups).
- **Unpaired positives:** loose files directly in `dups2/`; each has its duplicate among the
  other loose files, not yet sorted into a subfolder.
- **Hard negatives:** every pair inside `similiar_but_not_dups/` (images other duplicate tools
  flagged as duplicates, but which aren't). **Outdated since 2026-10-28:** variants now count
  as matches, and that folder is mostly variants (every pair above SSCD 0.4 aligns). Its
  "hard negatives flagged" numbers count matches, not errors.
- **Negatives (current):** pairs from two different `dups2/<NNN>` groups (different pictures).
  Used by `verify.py --pairs cross_groups` and reported by `evaluate.py` as "cross-group pairs flagged".

## Run

```powershell
$py = 'D:\PhotoGallery\venv-eval\Scripts\python.exe'
& $py bench_models.py                 # pure GPU throughput per model
& $py embed.py --model sscd           # embed all images -> emb_sscd.npy
& $py evaluate.py --model sscd        # recall, thresholds, flagged pairs, CSVs
& $py synth.py --model sscd           # robustness to crops, flips, overlays...
& $py check_groups.py                 # audit dups2/<NNN> folders for unrelated pictures
& $py verify.py --model sscd_fp16     # second stage: align pairs, classify match / unrelated
& $py verify.py --pairs cross_groups --floor 0.3   # ... on the negative set (pairs across dups2 groups)
& $py verify.py --pairs knn           # ... on real k-NN candidates (each image's 10 nearest above 0.4)
& $py pdq.py                          # PDQ hashes -> emb_pdq.npy; then evaluate.py --model pdq
& $py pdq.py --synth                  # PDQ robustness to the same synthetic edits
& $py bench_decode.py                 # JPEG decode pipelines: speed and embedding drift (idle machine)
& $py bench_hdd.py                    # HDD read order on E: (path / MFT / physical), unbuffered, read-only
& $py viewer.py                       # browse all results visually (local web page)
```

Models: `sscd`, `sscd_fp16`, `sscd_288_fp16`, `sscd_ts`, `sscd_large`, `dinov2_s`, `dinov2_b`,
`clip_b32` (see `models.py`), and `pdq` (hash stored as ±1/16 vectors: cosine = 1 − Hamming/128,
so every tool here works on it unchanged; Meta's match threshold d ≤ 31 is cosine ≥ 0.758).
Synthetic edits live in `edits.py` (torch-free, shared by `synth.py` and `pdq.py`).

### Quick-test subset

Everything labeled plus 3,000 random others (~9.3k images), for fast iteration:

```powershell
& $py make_subset.py                  # writes D:\PhotoGallery\eval_smoke\files.txt
$env:PG_WORK_DIR = 'D:\PhotoGallery\eval_smoke'   # then run the scripts above
```

Rerun `make_subset.py` after editing the labeled folders: `files.txt` is a cached snapshot.

### Verifier

`verify.py` checks every labeled duplicate pair plus every `similiar_but_not_dups` pair whose
embedding similarity clears `--floor` (the non-duplicates a candidate stage would hand it). Per
pair it matches DISK + LightGlue features (also against the mirrored image), fits a MAGSAC
homography, warps one image onto the other, fits a robust global color mapping, and measures
what is left:

- **match:** after alignment and color mapping the pixels agree (median residual ≤ `--max-noise`).
  Duplicates and variants are not told apart for now. Different frames of one shoot are *not*
  matches (user decision 2026-10-29); `--strong-inliers` (match on an overwhelming alignment alone)
  is therefore off by default, since it matched exactly those.
- **unrelated:** otherwise. An inlier count alone is not enough: unrelated pictures often get a
  20–120-inlier homography.

The changed-region analysis (overall, and `interior_*` away from the frame edges, where a different
frame of the same shoot shows up) and the motion measure (`moved_frac`: changed pixels explained by
consistent optical flow, i.e. things that moved, as in another frame) are drawn in the diagnostic image
(cyan = changed, green = moved). They affect the decision only with `--max-moved` /
`--max-interior-change`, which are off until tuned on hand marks.

**Hand marks:** in the viewer's verifier tabs, mark pairs "same picture" (incl. edits) or "different
picture" (buttons, or open a pair and press S / D to mark and move on, ← / → to browse). Marks go to
`pair_marks.json` in the run folder, keyed by the image pair, so they survive reruns; the Summary tab
shows how often the verifier agrees with them.
`--rescore` re-applies the thresholds to the stored measurements without redoing image work. Needs
`kornia` and `opencv-python-headless`; DISK and LightGlue weights download to the cache dir.

### Viewer

`viewer.py` serves a local page (localhost only; first free port from 8765) that shows every
run folder under `D:\PhotoGallery` side by side: a model comparison, and each pair list as
thumbnails (click a pair for full size). Thumbnails are cached in `D:\PhotoGallery\cache\viewer_thumbs`.

## Outputs (in the work dir)

| File | Contents |
|---|---|
| `files.txt` | Canonical image list; row i of every embedding file is line i |
| `emb_<model>.npy`, `ok_<model>.npy` | L2-normalized fp16 embeddings; load-success mask |
| `results_<model>.json` | Recall@k and operating points (threshold for 80/90/95/98% recall) |
| `misses_<model>.csv` | Labeled duplicate pairs the model scores lowest |
| `hard_negatives_<model>.csv` | Non-duplicate pairs the model scores highest |
| `review_<model>.csv` | Unlabeled pairs above the 95%-recall threshold, for hand-labeling precision |
| `unpaired_pairs_<model>.csv` | Best match of each loose `dups2` file (`mutual` = each other's best) |
| `synth_<model>.json` | Retrieval of originals after synthetic edits |
| `verify_<model>.csv`, `.json` | Verifier decision and measurements per pair; confusion table |
| `verify_<model>/<id>.jpg` | Verifier diagnostic per pair: reference · aligned + color-mapped · residual |
| `verify_cross_<model>.csv`, `.json`, `/` | The same for the cross-group negative set |
| `verify_knn_<model>.csv`, `.json`, `/` | The same for real k-NN candidate pairs (labeled where known) |
| `quality_pdq.npy` | PDQ's own hash quality score per image (0–100) |
| `synth_pdq.json` | PDQ synthetic robustness; `dihedral` = best of the 8 rotations/mirrors |
| `bench_decode.json` | Decode pipelines: img/s and SSCD cosine vs the PIL reference |
| `bench_hdd.json` | Read order on the HDD: files/s and MB/s per order, layout correlations |
