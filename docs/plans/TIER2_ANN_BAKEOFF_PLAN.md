# Tier 2 candidate retrieval bake-off: remaining work (exact vs tuned ANN, real embeddings, 50k scale)

## Context

The first ANN bake-off (`eval/bench_ann.py`, 41eaa1c) used one unclustered Gaussian and untuned indexes, so its index
numbers are invalid (FINDINGS says so). The redo runs on real SSCD embeddings with tuned indexes and a validated
harness, to settle PLAN §9 decision 5 (exact GPU search vs an approximate index for Tier 2).

Decisions that bound it: **no 70M targets or extrapolation, no reads from E:** (2026-11-01); **much smaller sizes,
not ~35 h** (2026-11-02). Quality over speed, but not days of runtime.

## Status

Done and committed or cached (`<data root>` is the directory outside the repo that holds the datasets and caches):

- Embedding of the 187,806 test images + 18,000 planted edits (D0 pipeline, `sscd_trt_fp16_trim_draft`) in
  `<data root>\eval_ann`; cached exact ground truths for 25k / 50k; validation gates passed at 25k
  (`cache/ann/validate.json`, two explained exceptions); characterization in `cache/ann/charact.json`.

Done, **not yet committed**:

- `eval/ann/` (data, exact, methods, cpu_worker, metrics, charact, run, report), `eval/plant.py`, the `--draft`
  changes in `common.py` / `imgload.py` / `embed.py`, the `verify.py --list` option, and the FINDINGS / PLAN edits.
  Checked 2026-11-06: none of the harness is in the tree. `eval/ann/` holds only `__pycache__` (its `.pyc` files are
  dated 2026-11-02), `eval/plant.py` is absent, `common.py`, `imgload.py` and `embed.py` have no `--draft`, and
  `verify.py` has no `--list` (its `--pairs` choices are `labeled`, `cross_groups` and `knn`). `eval/bench_ann.py` is
  tracked, and FINDINGS has no edit for this work. Remaining step 1 applies: recover the files before going on.
- Scale change: `MAIN = 50_000` (50k test rows + ~4.8k planted copies), trend sizes 12.5k / 25k, `VALIDATE = 25_000`;
  100k and the full base are gone. Sweep cap is now **600 s of total search per configuration** (plus the convergence
  rule: fewer than 0.01% of rows change their top 10).
- The 50k tune stage had finished 7 of 64 configurations (exact, GPU IVF nlist 256/1024) at 25–30 s each.
  Earlier 206k results are in `cache/ann/results_tune_206k_stopped.jsonl` and are not used.

## Remaining steps

1. **Confirm the harness is in the tree.** The files listed under Status must be present; if they are missing, recover
   them before going on. Commit as soon as step 2 holds, so the work is not lost again.
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
6. **Docs:** add a FINDINGS section (validation exceptions, characterization, Pareto tables, finalists, trend,
   planted per edit, true-duplicate recall; flag that GPU IVF timings are inflated by its Python per-list loop and
   that Windows pip wheels of FAISS/USearch lack SIMD, so CPU numbers come from the WSL Linux wheels). Update PLAN §9
   decision 5 and the M0 line from "being redone" to the result.
7. **Cleanup:** delete `eval/bench_ann.py` and the `cache/ann/5M_*` files (~5 GB); delete the unused 206k caches
   (`base_all.npy`, `exact_all.npz`) after the report is read; add the plant / `ann.run` / `--draft` lines to the
   Run block of `eval/README.md`.
8. **Commit** (explicit paths only: the files of this work, not unrelated changes): first the harness once step
   3 shows it works at 50k, then the results and docs once step 6 is done. No push.

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
- CPU indexes run in WSL (`/root/annenv`, Linux wheels with SIMD) through `eval/ann/cpu_worker.py` with
  `OPENBLAS_NUM_THREADS=1 OMP_WAIT_POLICY=PASSIVE`; GPU methods run in `<data root>/venv-eval`.

## Constraints while working

- Python only: no cargo builds are run for this work.
- The viewer for `eval_ann` needs `PG_DATA_ROOT=<data root>\datasets`; reload it after the runs.

## Verification

- `python -m ann.run tune` writes `cache/ann/results_tune.jsonl` with names tagged `50k|...`; the first lines match
  the interrupted run (exact 1.0; GPU IVF nlist 256 r1 at 1.0).
- `report.md` shows the exact ceiling at 1.0 and every planted source↔copy pair present in the exact join.
- The viewer's k-NN tab shows the verifier samples and misses (`verify_knn_sscd_trt_fp16_trim_draft.csv`).
- `ruff check .` and `ruff format --check .` pass; `just lint` for the repo.
