# Profiling ledger: what is still open

What the profiling reference ([PROFILING.md](../reference/PROFILING.md)) does not yet answer, and the leads measured
but not acted on. It was checked on 2026-11-14 by reading the code at HEAD `51a9657`. Nothing was built or run for it.
Items marked *read* were found by reading the code and have not been reproduced.

A row stays while any part of its proper fix is missing. Rows whose work is done are removed.

Leads dropped in this pass, because the code has moved on: the ASCII fast path in `path_collation` (the list is read in
`files_order` now, and `path_collation` no longer exists), and caching the Lanczos weights (the thumbnail path no longer
uses Lanczos3; see P-03).

## Summary

| Section | Rows | Notes |
|---|---:|---|
| Pending | 6 | Measurements not made, or checks the reference asks for |
| Leads | 4 | Measured ideas that have not been acted on |
| Decisions | 2 | Need a person: a quality trade-off and a gate threshold |
| Total | 12 | |

Kind legend: `pending` is a measurement or a check not made; `lead` is an idea from a measurement that is not acted on;
`decision` needs a choice between cost and something else.

## Pending

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| P-01 | Reference, "Open" | pending | `crates/render/src/maker.rs`, `crates/codec/src/resize.rs`, `crates/analysis/src/analyze.rs` | The app's hot paths are not measured: the thumbnail pass (decode, resize, encode) on a small folder, and the import. Every number in the reference is one bench file or the start. | Measure both on a scratch catalog (`--catalog-dir`, never the real one): dhat for heap, callgrind or SDE for instructions. Record the results in the reference. | **Open.** |
| P-03 | Reference, `analyze` and `thumbs::make` | pending | `crates/codec/src/resize.rs` | The resize rows are from Lanczos3 through `fast_image_resize`. The thumbnail path now resizes in linear light with Magic Kernel Sharp 2021 (`a70d174`), and that resize has no measurement. | Measure it with callgrind on the bench's sample, adding a bench if none covers it. Replace the Lanczos rows in the reference with the result. | **Open.** |
| P-08 | Reference, "Open" | pending | `tools/profiling/callgraph`, `crates/bench` | Repeatability under SDE is not checked. Single-threaded paths should give identical counts; callgrind already does. | Run the same SDE command twice on a single-threaded path and compare the totals. Record it in the reference. | **Open.** |
| P-09 | Reference, "Open" | pending | `crates/trace/src/lib.rs`, `crates/library/src/runner.rs:109`, `crates/desktop/src/library.rs:158` | Stages mark the start and the job's rounds (`round_done`). A scan and a thumbnail pass have none. | Add stages for them the same way, as `pg_stage_*` functions in `crates/trace/src/lib.rs`, so SDE and the Pin call graph can stop at them. | **Open.** |
| P-10 | Reference, "Open" | pending | `crates/bench/benches/ir/` | The only bench input is `sample.jpg` (1280×1017 baseline). Missing: a PNG, a WebP, a large (12 MP) JPEG, and a JPEG that decodes with DCT scaling. | Add them to the bench data, with a budget where a change is expected to move them (`docs/perf/budgets.json`). `make_bench_jpeg.py` makes the JPEG; the others need a source. | **Open.** |
| P-12 | Reference, "Library scan, wall clock" | pending | `tools/profiling/scan_timing.py` (successor of `scan-timing.sh`) | The 2026-11-07 wall-clock numbers came from the shell script. The reference records no run of the Python one, so it is not known to reproduce them. | Run `scan_timing.py` on a library of the same shape, on Windows and on Linux, and record its three timings with the commit, the drive and the cache state. | **Open.** |

## Leads

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| P-02 | Reference, `analyze` (2026-10-31) | lead | `crates/analysis/src/pdq.rs` (`box_1d`, `dct16`) | `pdq::box_1d` is 21% of `analyze` (13.6 M of 63.5 M, 1,838 calls), with 65% of the L1 read misses and 84% of the write misses. The column calls, strided by `width`, are the likely cause. Measured at 41eaa1c. | Re-measure at HEAD first (`just perf`, then `cgtab.py calls`): the PDQ buffers have changed since. If the column calls still dominate, transpose or process columns in blocks. `NIGHTLY_AND_BUILD_FLAGS_PLAN.md` (E2, box_1d) depends on this row. | **Open.** Re-measure first. |
| P-05 | Reference, the start (2026-10-31) | lead | `crates/desktop/src/gpu_device.rs`, `crates/desktop/cpp/gpu_device.cpp` | A Direct3D 11 device costs about 0.9 G instructions here, most of it NVIDIA's loader checking its signature. The app now makes one device per start, shared with Qt Quick. Nothing is kept across starts. | Lead, not measured: cache the probe's result per adapter and driver version, and measure whether a start with the cache skips the device. | **Open.** |
| P-06 | Reference, the start (2026-10-31) | lead | Qt's window creation (`WindowCreationData::create` → `GpuDescription::detect` → `Direct3DCreate9`) | Qt creates a Direct3D 9 object while creating the window: 0.55 G on the main thread, with three signature scans. Why Qt does it is not known. | Find out why from Qt's source (`C:\Qt\6.12.0\Src`), and whether a setting avoids it. | **Not investigated.** |
| P-07 | Reference, `gallery status` (2026-10-31) | lead | `crates/catalog/src/lookup.rs` (`Catalog::status`) | The eight `count(*)` queries are 96% of the `status` command's 1.05 G instructions. It is a diagnostic command, so this is not a problem today. | If it ever matters: one pass with aggregates (`sum(missing = 0)`-style), or an index on `files(missing)`. | **Not acted on.** |

## Decisions

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| P-04 | Reference, `thumbs::make` (2026-10-31) | decision | `crates/render/src/display.rs:24` (`DECODE_MARGIN`) | The thumbnail decode runs at full scale for images under four times the thumbnail height: 28.5 M of the 50.6 M in `thumbs::make` on the 1017-px sample, against 21.2 M for `analyze`'s decode. The margin is a quality choice, in `THUMBNAILS.md`. | Nothing to do unless the quality choice changes. A smaller margin needs the quality table in `THUMBNAILS.md` re-run with the current resampler first (P-03). | **Waiting on a quality-against-cost decision.** |
| P-11 | Reference, callgrind (2026-11-11) | decision | `justfile` (`perf`), `tools/profiling/bench.py` | gungraun can fail a run over a limit (`--callgrind-limits`), and the repo does not use it. Layout noise between unrelated commits is about 0.05%, so 1% was proposed as a start. | Decide whether `just perf` gets a limit, or the budgets in `docs/perf/budgets.json` and the ±1% tolerance are enough. | **Waiting on a decision.** |
