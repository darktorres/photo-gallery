# Tier 2 candidate retrieval bake-off: remaining work (exact vs tuned ANN, real embeddings, 50k scale)

## Purpose and bounds

The first ANN bake-off (`eval/bench_ann.py`, 41eaa1c) is invalid (see the last section). The redo runs on real SSCD
embeddings, with tuned indexes and a validated harness. It settles PLAN §9 decision 5: exact GPU search or an
approximate index for Tier 2.

Decisions that bound it: **no 70M targets or extrapolation, no reads from E:** (2026-11-01); **much smaller sizes,
not ~35 h** (2026-11-02). Quality over speed, but not days of runtime.

`<data root>` below is `PG_ROOT`, which is `D:\PhotoGallery` on this machine. It holds the datasets, the work folder
and the caches.

## State (checked 2026-11-14)

### Done

- Embeddings of the 187,806 test images and 18,000 planted edits, model `sscd_trt_fp16_trim_draft` (`embed.py --trim
  --draft`), in `<data root>\eval_ann`: `emb_sscd_trt_fp16_trim_draft.npy` holds 205,806 rows, `files.txt` lists them,
  `planted.json` lists the 18,000 edits.
- The harness is committed. `eval/ann/` (data, exact, methods, cpu_worker, metrics, charact, run, report),
  `eval/plant.py`, the `--draft` changes in `common.py` / `imgload.py` / `embed.py`, and the `verify.py --list` option
  came in `addc480` (2026-11-07). `0fc1b95` (2026-11-11) moved the CPU worker into the container. `4bc7e46` (2026-11-09)
  moved the eval's folders under `PG_ROOT`. None of these files has uncommitted changes.

### Gone

- The ANN cache. `D:\PhotoGallery\cache` (the default, since `PG_CACHE_DIR` is unset) has no `ann` folder. The user
  reports none on E:. The earlier version of this plan listed what it held: validation gates, characterization, exact
  ground truths for 25k and 50k, a partial tune run (7 of 64 configurations), the 206k results and the 5M files. None
  of it is on D:. Every stage runs again from the start.

**Scale** (`eval/ann/data.py`)

- `MAIN = 50_000`: 50k test rows plus ~4.8k planted copies (18 edits, about 266 per edit).
- Trend sizes 12.5k and 25k; `VALIDATE = 25_000`. 100k and the full base are not used.
- Each configuration's search is capped at **600 s** (`methods.MAX_SEARCH_S`). A sweep also stops once fewer than 0.01%
  of rows change their top 10 (`cpu_worker.py`).

## Steps

The run starts at `validate`, because `tune` refuses to run until `cache/ann/validate.json` passes.

1. **Validate, characterize, build the exact ground truth.** From `eval/`:
   `PG_DATA_ROOT=<data root>\datasets PG_WORK_DIR=<data root>\eval_ann python -m ann.run validate charact exact`.
   The gates are the SIFT1M check (it downloads the corpus into `cache\ann\sift`), the 25k ceiling, and the GPU join
   against float64. The earlier run's validation exceptions cannot be reproduced; these gates decide now. Record each
   gate's result in step 6.
2. **Decide the USearch grid** (see the grid section), then edit `grid()` in `eval/ann/run.py` if it changes.
3. **Run tune** in the background, from `eval/`, with the same environment: `python -m ann.run tune`. Log to
   `<data root>\experiments_<date>.log`. It is resumable by configuration name. Record the first USearch sweep's time
   and the total.
4. **Run `pca finalists verify report`** after tune. Check each stage's first lines before waiting out the rest.
   - Finalists: the cheapest setting per family at 0.99, 0.995 and 0.999 on the tuning half, rebuilt 3 times (shuffled
     insertion for graphs, new seeds for k-means), and its trend at 12.5k and 25k.
   - Verify: `verify.py --pairs listed --list knn --model sscd_trt_fp16_trim_draft --rotations 45`, on ~250 pairs per
     kind and band stratum plus 400 misses per family. This is what `ann.run verify` runs.
5. **Read `cache/ann/report.md`** and sanity-check it: the exact ceiling, planted copies per edit (about 266 pairs per
   edit, so the intervals are wide), dups2 recall, and true-duplicate recall with Clopper-Pearson bounds.
6. **Write the results section** here: validation gates and any failures, characterization, Pareto tables, finalists,
   trend, planted copies per edit, true-duplicate recall. Say that GPU IVF timings are inflated by its per-list Python
   loop (`methods.gpu_ivf`), and that the CPU numbers come from the Linux wheels in the container, since the Windows
   wheels of FAISS and USearch have no SIMD kernels (see the last section). Update PLAN §9 decision 5 and the M0 line
   from "being redone" to the result.
7. **Cleanup.** Add the plant, `ann.run` and `--draft` lines to the Run block of `eval/README.md` (not there yet). Delete
   `eval/bench_ann.py` (tracked; superseded by `eval/ann/`).
8. **Commit** the results and docs once step 6 is done, with explicit paths only. The harness is already committed. No
   push.

## The open choice: the USearch grid

The 27 USearch configurations (M 16/32/64 × ef_add 128/256/512 × f32/f16/i8) are the only open cost lever. The full
grid is 64 configurations: 1 exact, 9 GPU IVF, 27 USearch, 4 FAISS HNSW, 19 FAISS IVF and 4 NN-descent.

| Option | Total run time |
|---|---|
| Keep the 64-configuration grid | ~2–7 h (guess) |
| **Recommended:** USearch M 16/32 × ef_add 128/256 × f32/i8 (8 configurations), plus f16 and M 64 as single extra points at the best settings | ~1.5–2 h (guess) |

The totals are guesses, not derived from the cap. The cap bounds each sweep at 10 minutes. The 27 USearch sweeps take
at most 4.5 h, and the trimmed 8 plus up to 2 extra points take at most ~1.7 h. The other 36 configurations take at most
~6 h in either option. Both rows assume most sweeps stop early.

The recommendation is the trimmed grid, and its justification is not measured. M and ef_add change the recall each
search reaches, not only memory and build time. The trim is therefore safe only if the cheapest 0.999 setting does not
need M 64 or ef_add 512. f16 sits between f32 and i8, so the family's frontier is probably covered too. Check this
against the first USearch results before dropping the rest. The other families stay as designed.

## Method

- **Ground truth:** exact fp32 GPU join (`eval/ann/exact.py`), top 64 per row. Hubs, rows whose 64th neighbour is still
  above the floor, get an uncapped radius query.
- **Required pair:** in the rule set (k = 10, cosine ≥ 0.4, `data.py`) and beating the row's 11th neighbour by 1e-6.
- **Scoring:** every method's candidates are rescored exactly before the rule (`metrics.rescore`). Settings are chosen on
  one half of the pairs and reported on the other. The half is a fixed hash of the pair key (`data.pair_half`).
- **CPU indexes** run in the `pg-eval` container (`/opt/eval`, Linux wheels with SIMD, at the pins in
  `eval/requirements.txt`: `faiss-cpu==1.15.1`, `usearch==2.26.6`). `eval/ann/methods.run_cpu` starts
  `eval/ann/cpu_worker.py` through `tools/linux.py ann-worker`, with `OPENBLAS_NUM_THREADS=1` and
  `OMP_WAIT_POLICY=PASSIVE`.
- **GPU methods** run in `<data root>\venv-eval`.
- CPU indexes ran in WSL (`/root/annenv`) until 2026-11-11, then moved to the container.

## Constraints

- Python only. No cargo builds are run for this work.
- The viewer for `eval_ann` needs `PG_DATA_ROOT=<data root>\datasets` (the folder that holds `pinterest`). Its run list
  comes from the parent of `PG_WORK_DIR`. Reload it after the runs.

## Verification

- `ann.run tune` writes `cache/ann/results_tune.jsonl`, with names tagged `50k|...`. The first lines should read exact
  1.0 and GPU IVF nlist 256 r1 at 1.0, the values of the earlier interrupted run. Its output is not kept, so these are
  expected values, not a check.
- `report.md` shows the exact ceiling at 1.0 and every planted source-to-copy pair present in the exact join.
- The viewer's k-NN tab shows the verifier samples and misses (`verify_knn_sscd_trt_fp16_trim_draft.csv`).
- Lint: `ruff check` and `ruff format --check` pass on the harness files (checked 2026-11-14). The repo-wide
  `ruff check .` reports 9 errors and one format diff, all outside the harness: `eval/jpeg-decoders/restart.py` and
  `win_build.py`, and `eval/png-decoders/make_inputs.py`, `make_real16.py` and `make_variants.py`. `just lint` runs
  the repo's own checks.

## First attempt: invalid (5M synthetic vectors, 2026-10-31)

The first bake-off (`eval/bench_ann.py` at 41eaa1c) ran on 5M vectors drawn from **one Gaussian**, fitted to the
subset's SSCD mean and covariance, with 10,000 planted partners. **Its index results are not valid.** A single Gaussian
has no cluster structure, so HNSW and IVF had nothing to navigate by, and each index ran at one untuned setting. It
measured the generator, not the indexes. The figures it reported (background neighbours at ~0.29; USearch HNSW int8
finding at most 26% of planted partners; FAISS IVF 77%) are as reported. Their output is not kept in the repo or on D:,
so they cannot be re-checked.

These parts of that run do not depend on its data:

- **Exact search speed:** an fp16 GEMM with fp32 accumulation plus top-10 runs at **10.4 G pairs/s** on the RTX 2060.
  70M × 70M is ~131 GPU hours (~5.5 days; half that if only one triangle is computed). 10,000 new images against 70M
  take ~70 s. Streaming the base from disk is not included. This is arithmetic from the measured rate.
- **USearch int8 storage** of 512-d unit vectors moves cosines by 0.003 on average (p99 0.009). As reported.
- **Windows wheels have no SIMD kernels** (found 2026-11-01, re-checked 2026-11-14 in `venv-eval`). FAISS 1.15.1
  reports empty `get_compile_options()` and has no `swigfaiss_avx2`. USearch 2.26.6 reports `serial` for f32, f16 and
  i8. So the first run's USearch build speed (1,320 vectors/s at 5M, as reported) and all its CPU index timings are
  scalar-code numbers. The Linux wheels of the same versions have SIMD: the container reports `OPTIMIZE DD AVX2` for
  FAISS and `haswell` for USearch, for all three dtypes. For production in Rust on Windows, the SIMD build has to be
  checked the same way.
