# M0 model bake-off

One-time evaluation (see `docs/PLAN.md` §8, M0; results go in `docs/FINDINGS.md`) to pick the near-duplicate descriptor, its
precision/resolution, and thresholds, measured on the ~190k-image Pinterest test set.
Python is used only here; everything that runs at library scale is Rust.

## Setup

Paths come from `.env` at the repo root: copy `.env.example` to `.env` and set `PG_ROOT`, the base folder for the
datasets, outputs, caches, venvs and tools (there is no default). `.env.example` lists every variable. The Python
scripts load `.env` themselves; a variable already set in the environment wins over it. PowerShell does not read
`.env`, so the snippets below expect `PG_ROOT` in the shell (`$env:PG_ROOT`), or run them through `just`.

- Data: `$PG_ROOT\datasets\pinterest` (a copy of the source folder)
- Outputs: `$PG_ROOT\eval` (embeddings, results, CSVs)
- Model and pip caches: `$PG_ROOT\cache` (keep it on a disk with room)
- Environment: `$PG_ROOT\venv-eval` (CUDA PyTorch; the global Python has CPU-only torch)

Each of these can be overridden by its own variable: `PG_DATA_ROOT`, `PG_WORK_DIR`, `PG_CACHE_DIR`, `PG_TOOLS_DIR`,
`PG_QUANT_PYTHON`, `PG_TRT10_DIR` (see `.env.example`).
`--trim` runs the gallery's border detector (`crates/analysis/src/borders.rs`) through `gallery borders --stdin`: it
needs `target\release\gallery.exe` built (`just build`), or `PG_GALLERY` pointing at one.

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
$py = "$env:PG_ROOT\venv-eval\Scripts\python.exe"
& $py bench_models.py                 # pure GPU throughput per model
& $py embed.py --model sscd           # embed all images -> emb_sscd.npy
& $py evaluate.py --model sscd        # recall, thresholds, flagged pairs, CSVs
& $py synth.py --model sscd           # robustness to crops, flips, overlays...
& $py embed.py --model sscd_fp16 --trim   # borders cut first (gallery borders --stdin) -> emb_sscd_fp16_trim.npy
& $py synth.py --model sscd_fp16 --trim   # ... and its synthetic robustness (edited copies trimmed too)
& $py check_groups.py                 # audit dups2/<NNN> folders for unrelated pictures
& $py verify.py --model sscd_fp16     # second stage: align pairs, classify match / unrelated
& $py verify.py --pairs cross_groups --floor 0.3   # ... on the negative set (pairs across dups2 groups)
& $py verify.py --pairs knn           # ... on real k-NN candidates (each image's 10 nearest above 0.4)
& $py verify_synth.py                 # ... on synth.py's edited copies (crops, overlays, rotation...): all should match
& $py verify_synth.py --edits rot_45 rot_any --prefix verify_rot --rotations 45   # some edits; 45° retries
& $py verify.py --pairs knn --shortcut   # pipeline mode: PDQ <= 22 or SSCD >= 0.95 accepted without image work
& $py group.py                        # groups from the verified k-NN pairs (multicut), scored on the labels
& $py pdq.py                          # PDQ hashes -> emb_pdq.npy; then evaluate.py --model pdq
& $py pdq.py --synth                  # PDQ robustness to the same synthetic edits
& $py bench_decode.py                 # JPEG decode pipelines: speed and embedding drift (idle machine)
& $py bench_hdd.py                    # HDD read order on the data's drive (path / MFT / physical), unbuffered, read-only
& $py bench_thumbs.py                 # grid thumbnails: resize pipelines, formats x quality (bytes, SSIMULACRA2, ms)
& $py bench_models.py --models sscd_fp16 sscd_trt_fp16 sscd_trt_int8   # TensorRT / ONNX Runtime (accel.py)
& $py bench_semantic.py               # CLIP / SigLIP 2 image encoders: torch fp16 vs TensorRT fp16 (a second model)
& $py viewer.py                       # browse all results visually (local web page)
```

Models: `sscd`, `sscd_fp16`, `sscd_288_fp16`, `sscd_ts`, `sscd_large`, `dinov2_s`, `dinov2_b`,
`clip_b32` (see `models.py`), and `pdq` (hash stored as ±1/16 vectors: cosine = 1 − Hamming/128,
so every tool here works on it unchanged; Meta's match threshold d ≤ 31 is cosine ≥ 0.758).
Synthetic edits live in `edits.py` (torch-free, shared by `synth.py` and `pdq.py`).

The SSCD weights (`sscd_disc_mixup`, `sscd_disc_large`) are Meta's, released under the MIT license in
[facebookresearch/sscd-copy-detection](https://github.com/facebookresearch/sscd-copy-detection). `models.py` checks
each TorchScript file against a pinned SHA-256 before loading it.

The same SSCD weights also run outside PyTorch (`accel.py`): `sscd_trt_fp16` / `sscd_trt_int8` (TensorRT 11
engines), `sscd_ort_cuda_fp32` / `_fp16` (ONNX Runtime's CUDA provider) and `sscd_ort_trt10_fp16` / `_int8`
(ONNX Runtime's TensorRT provider). They plug into every script above like the torch models. The ONNX graphs,
calibration batch and engines are built on first use under `$PG_ROOT\cache\onnx` and `$PG_ROOT\cache\trt` (also the
verifier's DISK U-Net, `disk_fp16`, and `bench_semantic.py`'s encoders); build the
int8 graph with `PG_WORK_DIR` on the subset, so the calibration images come from outside it. Requirements beyond
this venv:

- Only to rebuild the int8 graph (int8 is not used; its built graph and engine stay in the cache):
  `$PG_ROOT\venv-quant` (its python is `PG_QUANT_PYTHON`) with NVIDIA ModelOpt (`python -m venv` it, then
  `pip install "nvidia-modelopt[onnx]"`). It pins older onnx / onnxruntime versions, hence its own venv. Removed after
  the int8 evaluation.
- `$PG_ROOT\tools\tensorrt-10.16.1.11-cu13` (`PG_TRT10_DIR`): TensorRT 10 DLLs (the `tensorrt-cu13-libs` wheel from
  pypi.nvidia.com, unpacked) for ONNX Runtime's TensorRT provider, which is built against TensorRT 10.
- The venv is on CUDA 13 (torch 2.15 cu134, onnxruntime-gpu 1.31.1, `tensorrt-cu13`), which needs NVIDIA
  driver ≥ 580.

`bench_thumbs.py` uses libjxl's `ssimulacra2.exe` and `cjpegli.exe` from `$PG_ROOT\tools\libjxl-0.11.1` (`PG_TOOLS_DIR`)
(the GitHub release zip of libjxl/libjxl; 0.11.1 is the last release that ships cjpegli).

### Quick-test subset

Everything labeled plus 3,000 random others (~9.3k images), for fast iteration:

```powershell
& $py make_subset.py                  # writes $PG_ROOT\eval_smoke\files.txt
$env:PG_WORK_DIR = "$env:PG_ROOT\eval_smoke"   # then run the scripts above
```

Rerun `make_subset.py` after editing the labeled folders: `files.txt` is a cached snapshot.

### Verifier

`verify.py` checks every labeled duplicate pair plus every `similiar_but_not_dups` pair whose
embedding similarity clears `--floor` (the non-duplicates a candidate stage would hand it). Per
pair it matches DISK + LightGlue features (`--features aliked` for ALIKED, `sift` for SIFT; also against the
mirrored image unless the first alignment is already strong), fits a MAGSAC homography, warps one image onto
the other, fits a robust global color mapping (onto the reference, or the reverse when that fits at
least 2× better: grayscale can't be mapped back to color), and measures what is left. Images smaller than 512 px are upscaled
for feature extraction; DISK finds almost nothing on a ~200 px copy. DISK's U-Net runs as a TensorRT fp16
engine (`--disk-runtime torch` for PyTorch fp32; same keypoints to 98%). DISK is not rotation-invariant (past
~45° it fails): `--rotations 90` / `45` retries a pair that doesn't match with the second image rotated in those
steps, matching the rotated copy's features and mapping them back, so everything after uses the images as they
are. Images are decoded and resized ahead on `--loaders` CPU threads (default 4), feature extraction and
matching run on the GPU in the main thread, the comparison on `--workers` CPU threads (default 8).
The decision:

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
picture" (buttons, or open a pair and press S / D to mark and move on, ← / → to browse). An open pair shows
one image at a time in a shared box; Space (or a click on the image) swaps A and B. Marks go to
`pair_marks.json` in the run folder, keyed by the image pair, so they survive reruns; the Summary tab
shows how often the verifier agrees with them.
`--rescore` re-applies the thresholds to the stored measurements without redoing image work. Runs are
resumable: each finished pair is appended to `<prefix>_<model>.partial.csv`, a restarted run skips those, and
the file is removed when the run completes (delete it to start over, e.g. after changing options). Needs
`kornia` and `opencv-python-headless`; DISK and LightGlue weights download to the cache dir.

### Viewer

`viewer.py` serves a local page (localhost only; first free port from 8765) that shows every
run folder under `$PG_ROOT` side by side: a model comparison, and each pair list as
thumbnails (click a pair for full size). Thumbnails are cached in `$PG_ROOT\cache\viewer_thumbs`.

The **Groups** page shows `group.py`'s groups, largest first, with filters for unmarked groups, groups that
merge labeled folders, and groups with aligned-but-different pairs inside. Mark a group by giving each
picture in it its own set (click the letter on a thumbnail to step through the sets in use plus a new one,
Shift+click back; or press the set's letter, or 1–9, in the full view; up to one set per image) and saving: pairs
within a set become "same picture" marks, pairs across sets "different" (in `pair_marks.json`, shared with the
verifier tabs). The full view shows one member at a time in a fixed box: ← / → step through the group, Space
returns to the previously shown image, Enter saves, PgUp / PgDn change group.

## Outputs (in the work dir)

| File | Contents |
|---|---|
| `files.txt` | Canonical image list; row i of every embedding file is line i |
| `emb_<model>.npy`, `ok_<model>.npy` | L2-normalized fp16 embeddings; load-success mask (`<model>_trim`: `--trim`) |
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
| `verify_synth_<model>.csv`, `.json`, `/` | The same for original vs synthetic edited copy (label = the edit; `--prefix` renames) |
| `groups_<model>.json` | `group.py`: edge counts, scores against the labels, and the groups (largest first) |
| `quality_pdq.npy` | PDQ's own hash quality score per image (0–100) |
| `synth_pdq.json` | PDQ synthetic robustness; `dihedral` = best of the 8 rotations/mirrors |
| `bench_decode.json` | Decode pipelines: img/s and SSCD cosine vs the PIL reference |
| `bench_hdd.json` | Read order on the HDD: files/s and MB/s per order, layout correlations |
| `bench_thumbs.json`, `_zoneplate.png` | Thumbnails: resize scores, bytes / score / ms per format and quality, SSCD re-use |
| `bench_semantic.json` | CLIP / SigLIP 2 encoders: params, img/s (torch fp16, TensorRT fp16), cosine vs fp32 |
