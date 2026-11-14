# Tier 2 candidate retrieval bake-off: remaining work (exact vs tuned ANN, real embeddings, 50k scale)

## Context

The first ANN bake-off (`eval/bench_ann.py`, 41eaa1c) used one unclustered Gaussian and untuned indexes, so its index
numbers are invalid (the first-attempt section below says so). The redo runs on real SSCD embeddings with tuned indexes and a validated
harness, to settle PLAN §9 decision 5 (exact GPU search vs an approximate index for Tier 2).

Decisions that bound it: **no 70M targets or extrapolation, no reads from E:** (2026-11-01); **much smaller sizes,
not ~35 h** (2026-11-02). Quality over speed, but not days of runtime.

## Status

Done (`<data root>` is the directory outside the repo that holds the datasets and caches; `PG_ROOT` is `D:\PhotoGallery`):

- Embedding of the 187,806 test images + 18,000 planted edits (D0 pipeline, `sscd_trt_fp16_trim_draft`) in
  `<data root>\eval_ann` (205,806 rows, checked 2026-11-14).

Committed (the harness is in git: `addc480`, 2026-11-07; `4bc7e46` and `0fc1b95` moved its CPU worker into the container):

- `eval/ann/` (data, exact, methods, cpu_worker, metrics, charact, run, report), `eval/plant.py`, the `--draft`
  changes in `common.py` / `imgload.py` / `embed.py`, and the `verify.py --list` option. `eval/` and `docs/temp/` are
  clean in `git status` (checked 2026-11-14).

Gone (checked 2026-11-14): the ANN cache. `D:\PhotoGallery\cache` (the default, since `PG_CACHE_DIR` is unset) has no
`ann` folder, and nothing is on E:. It held the validation gates (`validate.json`), the characterization
(`charact.json`), the exact ground truths for 25k / 50k, a partial tune run (7 of 64 configurations), the 206k results
and the 5M files. It is treated as deleted: every stage below runs again from the start.

- Scale change: `MAIN = 50_000` (50k test rows + ~4.8k planted copies), trend sizes 12.5k / 25k, `VALIDATE = 25_000`;
  100k and the full base are gone. Sweep cap is now **600 s of total search per configuration** (plus the convergence
  rule: fewer than 0.01% of rows change their top 10).

## Remaining steps

The harness is committed and the cache is gone, so the run starts at `validate`.

1. **Validate, characterize, and build the exact ground truth.** From `eval/`, with
   `PG_DATA_ROOT=<data root>\datasets PG_WORK_DIR=<data root>\eval_ann`: `python -m ann.run validate charact exact`.
   `tune` refuses to run until `cache/ann/validate.json` passes. The gates are the SIFT1M check (it downloads the
   corpus into the cache), the 25k ceiling, and the GPU-vs-float64 check. Record each gate's result in step 6. The two
   exceptions of the earlier run cannot be checked again, since its cache is gone.
2. **Decide the USearch grid size** (the only open cost lever; see below), then edit `grid()` in `eval/ann/run.py`.
3. **Run the tune stage** in the background, logged to `<data root>\experiments_<date>.log`, with
   `PG_DATA_ROOT=<data root>\datasets PG_WORK_DIR=<data root>\eval_ann` from `eval/`
   (`python -m ann.run tune`). Resumable by configuration name. Record the first USearch sweep's time and the
   measured total.
4. **Run `pca finalists verify report`** after tune. Check each stage's first lines before waiting out the rest.
   Finalists: cheapest setting per family at 0.99 / 0.995 / 0.999 (tune half), 3 rebuilds (shuffled insertion for
   graphs), trend at 12.5k / 25k. Verify: `verify.py --pairs listed --list knn --model sscd_trt_fp16_trim_draft
   --rotations 45` on ~250 pairs per kind × band stratum plus 400 misses per family.
5. **Read `cache/ann/report.md`** and sanity-check it: exact ceiling, planted per edit (~265 pairs per edit, so
   intervals are wide), dups2 recall, true-duplicate recall with Clopper-Pearson bounds.
6. **Docs:** add a results section here (validation gates and any exceptions, characterization, Pareto tables,
   finalists, trend, planted per edit, true-duplicate recall; flag that GPU IVF timings are inflated by its Python
   per-list loop and that Windows pip wheels of FAISS/USearch lack SIMD, so CPU numbers come from the Linux wheels, then
   in WSL and now in the container). Update PLAN §9 decision 5 and the M0 line from "being redone" to the result.
7. **Cleanup:** add the plant / `ann.run` / `--draft` lines to the Run block of `eval/README.md` (not there yet, checked
   2026-11-14). Delete `eval/bench_ann.py` (tracked, superseded by `eval/ann/`).
8. **Commit** (explicit paths only, the files of this work, not unrelated changes): the results and docs once step 6 is
   done. The harness is already committed. No push.

## The one open choice: USearch grid

The 27 USearch configurations (M 16/32/64 × ef_add 128/256/512 × f32/f16/i8) are the whole uncertainty: each sweep
takes up to 10 minutes of search. Estimates (not yet measured for USearch at 50k):

| Option | Total run time |
|---|---|
| Keep the 64-configuration grid | ~2–7 h (guess 3–4 h) |
| **Recommended:** USearch M 16/32 × ef_add 128/256 × f32/i8 (8 configs); f16 and M 64 only as single extra points at the best settings | ~1.5–2 h |

Recommendation: the trimmed grid. M 64 and ef_add 512 only cost memory and build time for a graph this small, and
f16 sits between f32 and i8, so the family's frontier is covered. The other families (GPU IVF, FAISS HNSW / IVF /
NN-descent) stay as designed.

## Design reference (implemented, unchanged)

- Ground truth: exact fp32 GPU join (`eval/ann/exact.py`), top 64 per row, hubs with an uncapped radius query.
  Required pair = in the k=10 / cosine ≥ 0.4 rule set and beating the row's 11th neighbour by 1e-6.
- Every method's candidates are rescored exactly before the rule (`metrics.rescore`). Settings are chosen on a
  random half of the pairs and reported on the other half (splitmix hash of the pair key).
- CPU indexes run in the Linux container (the `pg-eval` image's `/opt/eval`, Linux wheels with SIMD, at the pins of
  `eval/requirements.txt`) through `eval/ann/cpu_worker.py`, which `methods.run_cpu` starts with `tools/linux.py
  ann-worker` (`OPENBLAS_NUM_THREADS=1 OMP_WAIT_POLICY=PASSIVE`); GPU methods run in `<data root>/venv-eval`. They ran
  in WSL (`/root/annenv`) until 2026-11-11.

## Constraints while working

- Python only: no cargo builds are run for this work.
- The viewer for `eval_ann` needs `PG_DATA_ROOT=<data root>\datasets`; reload it after the runs.

## Verification

- `python -m ann.run tune` writes `cache/ann/results_tune.jsonl` with names tagged `50k|...`; the first lines should
  read exact 1.0 and GPU IVF nlist 256 r1 at 1.0 (the values of the interrupted run, whose file is gone).
- `report.md` shows the exact ceiling at 1.0 and every planted source↔copy pair present in the exact join.
- The viewer's k-NN tab shows the verifier samples and misses (`verify_knn_sscd_trt_fp16_trim_draft.csv`).
- `ruff check .` and `ruff format --check .` pass; `just lint` for the repo.

## Nearest-neighbour search, first attempt: invalid (5M synthetic vectors, 2026-10-31)

The first index bake-off (`eval/bench_ann.py` at 41eaa1c) ran on 5M vectors drawn from **one Gaussian** fitted to
the subset's SSCD mean and covariance, with 10,000 planted partners. **Its index results are not valid:** a single
Gaussian has no cluster structure (a query's background neighbours all sit at ~0.29), so HNSW and IVF had nothing to
navigate by, and the indexes ran at one untuned setting each. It measured the generator, not the indexes (USearch
HNSW int8 found at most 26% of planted partners, FAISS IVF 77%). The redo, on real library embeddings with tuned
indexes, is PLAN §9 decision 5.

What stands, because it doesn't depend on the data:

- **Exact search speed:** an fp16 GEMM (fp32 accumulation) plus top-10 runs at **10.4 G pairs/s** on the 2060, so
  70M × 70M is ~131 GPU hours (~5.5 days; half that if only one triangle is computed), and 10,000 new images
  against 70M ~70 s. Streaming the base from disk is not included.
- **USearch's int8 storage** of 512-d unit vectors moves cosines by 0.003 on average (p99 0.009).
- **The Windows pip wheels of both libraries have no SIMD kernels** (found 2026-11-01): FAISS 1.15.1 loads its
  generic build (`get_compile_options()` is empty; there is no `swigfaiss_avx2` on Windows), and USearch 2.26.6 is
  built without SimSIMD (`hardware_acceleration` is `serial` for every type). So the first run's USearch build speed
  (1,320 vectors/s at 5M) and all its CPU index timings are scalar-code numbers. The Linux wheels of the same
  versions have them (FAISS `AVX2`, USearch `haswell`), so the redo runs the CPU indexes in WSL2. For production
  in Rust on Windows, the SIMD build has to be checked the same way.
