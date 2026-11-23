# Plan: nightly features, build flags and stable performance levers

## Status

Status as of 2026-11-14.

- The toolchain pin that Phase 6 decides on is already taken: `rust-toolchain.toml` pins `nightly-2026-11-06` (commit
  edf7a39, "Pin the toolchain to nightly-2026-11-06 and fix the lints it adds"). Builds, CI and the container all use it.
- The survey is at `nightly-2026-09-22`.
- Nothing for this plan has been built or benchmarked. `docs/reference/PROFILING.md` has measurements from 2026-11-07 to
  2026-11-11 for other work, but no HEAD baseline (Phase 1b) is recorded.
- `target/` was cleaned on 2026-11-06 (48 GiB freed) and has been rebuilt since (15,208 files on 2026-11-14).
- Product code is untouched by this plan.
- This version was checked against the tree on 2026-11-14. See Review provenance for what was checked and what remains
  open.

## Context

The question is which Rust nightly features the workspace could use, and every one is evaluated rather than spot-checked.
The survey covered the whole Unstable Book of `nightly-2026-09-22`: 530 library pages, 248 compiler-flag pages and 258
language-feature pages. The 2026-11-06 book has 538, 251 and 264. Each entry was read and searched for in `crates/` and
the pinned forks, or excluded under a named rule (other targets, compiler or std internals, async). The appendix holds
the per-entry outcome so nothing is re-argued. The per-letter and total counts recorded during the survey do not
reconcile with either book (see section 6), so the lists are the record.

What the survey found:

1. **No unstable language or library feature pays off alone.** About a dozen have a real site (section 5). Each is a
   cleanup or a small robustness gain. Only N14 (png SIMD) could move performance, and it needs a measurement.
2. **Nightly's real value here is tooling**, usable without touching product code: fuzzing, ASan, move attribution,
   type-size audits, optimization remarks, layout randomization and compile-time profiling.
3. **The largest likely wins are stable and untried:** `-C target-cpu=x86-64-v3`, release LTO with
   `codegen-units = 1`, PGO, and the dev profile's debuginfo (`target/` was 48 GiB). The loop interchange in the PDQ DCT
   (E8) is already in the tree; only its measurement remains.
4. **The performance numbers in `docs/reference/PROFILING.md` are stale.** They were measured at 41eaa1c. Since then, commit
   81ad925 fixed the strided column pass in PDQ `box_1d`, so the "box_1d is 21% of analyze, cache-miss-bound" lead is
   gone, and later commits changed allocation and copying. Phase 1 re-measures before anything is judged.

## Decisions (confirmed)

| #  | Question                                                                                       | Decision                                                                                                                                |
|----|------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------|
| D1 | Does the binary only run on this machine class (i7-8700, AVX2)?                                | **Yes.** `-C target-cpu=x86-64-v3` may be adopted if it measures well.                                                                  |
| D2 | Is a nightly pin acceptable for the workspace (`rust-toolchain.toml`) or only for opt-in runs? | **A pin is acceptable.** `#![feature]`, `cargo-features` and `-Z` flags in the product build are allowed. The pin is in place (Status). |

What a pin costs, to weigh per adoption (not a blocker): build, clippy, `cargo deny` and the container benches all move to
the pinned nightly. Its date is bumped by hand, and features can break or be renamed on a bump (`allocator_api` became
`allocator_ext` between the two books). Crates using `#![feature]` no longer build on stable. So: try everything first in
a scratch target dir (`-Zcrate-attr=feature(x)` enables a feature without editing source), and prefer a stable equivalent
when it gives the same result.

## How the work runs

- **Benchmarks.** `just perf` runs `tools/linux.py perf`. It exports the *committed* HEAD into the container, runs the
  callgrind benches of `crates/bench/benches/ir.rs` (`analyze_jpeg`, `thumb_jpeg`, `viewer_still` for JPEG and PNG, and
  `viewer_poster` in `ir/linux.rs`, plus the catalog benches in `ir/catalog.rs`), and gates the counts at ±1% against
  `docs/perf/baseline.json` (`tools/perf_check.py`). `just linux-bench` runs `tools/profiling/bench.py` without the gate.
  Results go to `/bench/target/gungraun/` in the container and are read with `tools/profiling/cgtab.py`. Counts are
  deterministic, so one run per variant is enough.
- **Uncommitted work is never benched.** Flag and profile trials go in through the environment (`RUSTFLAGS`,
  `CARGO_PROFILE_BENCH_LTO`, `CARGO_PROFILE_BENCH_CODEGEN_UNITS`), passed into the container with `-e`. `tools/linux.py`
  does not forward them yet: `bench` forwards only `PG_HEAD_SHA`, and `perf` forwards `PG_BENCH_SKIP_CATALOG` and
  `PG_HEAD_SHA`. Add the names to those lists before the first trial.
- **Code trials** are committed on a short-lived local branch `eval/<id>`, benched, then merged or deleted. E8 is already
  in the tree and needs no trial branch; measure it against the baseline.
- **Nightly in the container.** The image installs the toolchain named in `rust-toolchain.toml`, so it has the pinned
  nightly only. Phase 0 adds `nightly-2026-09-22` as well, for the N5, N10 and N17 runs.
- **Scratch target dirs** sit outside the repo (`CARGO_TARGET_DIR=<scratch>\cargo-targets\<name>` on Windows,
  `/bench/target-<name>` in the container) for every nightly or flag experiment, so the main `target/` is never rebuilt by
  them.
- **No Windows build, test or timing while other edits are in progress in the tree.** Check `git status` before each
  Windows step.
- **Adoption threshold.** A performance change is adopted when the estimated cycles on the affected bench fall by at
  least 3%, with the tests green and, for PDQ and thumbnails, byte-identical output. Otherwise it is closed with the
  numbers recorded. Robustness findings (crashes, UB) are fixed regardless of size.
- **Measure at the end of the steps, not per step**, except where a number decides the next step.
- **Commits.** Commit each verified step separately, with the measured counts in the message. No unrelated changes in a
  commit. Push is a separate step. Results also go in `docs/reference/PROFILING.md` as a dated subsection (commit, tool,
  counts), and its "Open" list is updated.

## 1. Phases (the order of work)

**Phase 0: checks that need no build of the tree.**

- 0a. Plain stable compile (`rustc +stable`, no `#![feature]`) of tiny files in a scratch directory outside the repo, using
  `core::hint::cold_path`, `<[T]>::array_windows`, `ptr.is_aligned()` and `split_first_chunk`/`first_chunk`. The stable
  std source already settles which of these are stable: `cold_path` since 1.95.0, `array_windows` since 1.94.0,
  `is_aligned` since 1.79.0, and `first_chunk` and `split_first_chunk` since 1.77.0. The compile still confirms that the
  code builds. (The first probe was invalid: stable rejects `#![feature]` outright.)
- 0b. `rustc +stable -O --emit asm` of a copy of `Hash::distance`'s body (`crates/model/src/pdq.rs:27`), at baseline and
  with `-C target-cpu=x86-64-v3`: is it a software popcount today? This decides how much of E1 is about POPCNT.
- 0c. `rustup component list --toolchain nightly-2026-09-22` for what is installed. On 2026-11-14 that toolchain has
  rustc, cargo, rust-std, rustfmt, clippy and rust-docs. Then install the nightly in the container
  (`rustup toolchain install nightly-2026-09-22 --profile minimal`, in `just linux-shell`) with the components the later
  phases need.

**Phase 1: baseline at HEAD.**

- 1a. Add the missing bench inputs, one commit each, in `crates/bench/benches/ir/linux.rs` beside the existing ones:
  - a PDQ index nearest-neighbour bench (the `pdq_index.rs` this named is removed; `OPEN_ITEMS.md` records it). Not
    yet added;
  - a PNG decode through `analyze`. Not yet added. `viewer_still` already has a PNG case, through the viewer decoder
    rather than `analyze`.
- 1b. `just linux-bench --save-baseline=head`, read with `cgtab.py`. Record the counts and the new top self-cost lines in
  `PROFILING.md`, and replace the stale 41eaa1c leads (E2, box_1d).

**Phase 2: stable levers, one batched pass in the container.** Each is compared against `--baseline=head`: E1, then E5,
then E8 (already in the tree; measure it), then E6, which needs E5's final profile. E3 and E4 are measurements or
decisions that need no bench. Adopt winners by the threshold above. Each adoption is its own commit, and `just lint` and
the crate tests must pass.

**Phase 3: nightly measurement runs** (no product-code change; scratch dirs): N10 and N17 in the container on the bench
build; N12 and N15 as `check` or `rustc` dumps; N14 (uses the PNG bench, `viewer_still`); N19, N1 and N16.

**Phase 4: robustness tooling.** N5 fuzzing in the container (the longest-running item: start early and let it run while
Phase 3 is read); N13 coverage from the same corpus and tests; N6 ASan on Windows, after a link-and-run proof on a trivial
crate.

**Phase 5: compile time and footprint** (Windows, needs a quiet tree). T1 timings first, then E7, then N3, N8, N9 and N11;
N2 only as T1 shows it could matter.

**Phase 6: the pin decision.** The pin was taken in edf7a39 before any measurement, so this phase now decides whether it
stays. With every number in hand, keep the pin only if the wins justify moving the whole build. Candidates: N14, N2, any
`-Z` option that measured well, and the section 5 cleanups taken as a group. Otherwise return to stable and keep nightly
for opt-in tooling. If the pin is kept, the pin and the changes it enables are separate commits. `just lint` and the full
tests run on the pinned toolchain, and the bench runs on the same pin (`tools/profiling/bench.py` builds `/head` with its
`rust-toolchain.toml`).

**Phase 7: close out.** `PROFILING.md` final numbers, delete the scratch target dirs and branches, and report each item's
result in the tracker.

## 2. Stable levers (no pin needed)

| ID | Evaluate                                                                                                                            | Evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | How                                                                                                                                                                                         | Gate                                                                                                          |
|----|-------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------|
| E0 | Baseline at HEAD and stable-feature check                                                                                           | See Phases 0a and 1                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Phase 0a, 1b                                                                                                                                                                                | Everything below compares to it                                                                               |
| E1 | `-C target-cpu=x86-64-v3`                                                                                                           | There is no `.cargo/config.toml`, so no `target-cpu` is set and the x86-64 baseline applies. `Hash::distance` (`crates/model/src/pdq.rs:27`) and the Hamming scan (the removed `pdq_index.rs:323`; the join is `crates/dedup/src/join.rs`) use `count_ones`; POPCNT is not in the x86-64 baseline (asm unverified, Phase 0b). Also AVX2 autovectorization of our own loops. The resize kernels in the `fast_image_resize` fork pick their SIMD paths at run time (286 `target_feature` or detection hits); `crates/` has no detection of its own, and JPEG and zlib SIMD sits in dependencies | `RUSTFLAGS="-C target-cpu=x86-64-v3"` into the container with `-e`; compare `analyze_jpeg`, `thumb`, the PDQ-search bench (to add) and the PNG bench                                        | Threshold; D1 allows adopting. A flag in `.cargo/config.toml` `[build]` would also apply to the Windows build |
| E2 | PDQ after 81ad925                                                                                                                   | Premise gone (Context 4)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Re-read the Phase 1 profile; update the `PROFILING.md` lead                                                                                                                                 | Close if `pdq` no longer dominates cache misses                                                               |
| E3 | Lanczos weight cache (`precompute_coefficients` in the `fast_image_resize` fork, a git pin)                                         | 3.2 M of 50.6 M instructions per thumbnail (6.4%) at 41eaa1c, mostly libm `sin` (`PROFILING.md`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Count (source, target) size repeats in a real `gallery thumbs` pass on a scratch catalog; prototype a cache only if repeats are common                                                      | High repeat rate and the threshold                                                                            |
| E4 | `DECODE_MARGIN = 4` (`crates/render/src/display.rs:24`): full-scale decode vs DCT scaling                                           | 28.5 M vs 21.2 M instructions for images just under 4x (`PROFILING.md`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | A quality comparison (`THUMBNAILS.md`, "Thumbnails"), not a speed one                                                                                                                       | A decision for the maintainer; the price is recorded                                                          |
| E5 | Release `lto = "fat"` or `"thin"` and `codegen-units = 1`; nightly variants `-Zcross-crate-inline-threshold=always`, `-Zinline-mir` | `[profile.release]` sets only `debug = "line-tables-only"`. No cross-crate inlining between the pg-* crates, the forked decoders and std. Release is non-incremental by default                                                                                                                                                                                                                                                                                                                                                                                                               | `CARGO_PROFILE_BENCH_LTO` / `_CODEGEN_UNITS` into the container with `-e` (not forwarded yet, see How the work runs); also build time. Nightly variants only if the stable ones leave a gap | Threshold and acceptable build time                                                                           |
| E6 | PGO (`-Cprofile-generate` / `-Cprofile-use`)                                                                                        | Stable; needs `llvm-profdata` (`llvm-tools` component, not checked)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Instrument a thumbnail and import run on a scratch catalog, rebuild, compare                                                                                                                | After E5; threshold and a workable profile-refresh process                                                    |
| E7 | `target/` footprint (48 GiB and 78,053 files before the clean; 15,208 files on 2026-11-14; size not measured)                       | `Cargo.toml` leaves dev `debug` unset (full) and builds dependencies at `opt-level = 3` (`[profile.dev.package."*"]`). The cause is inferred, not measured                                                                                                                                                                                                                                                                                                                                                                                                                                    | `[profile.dev.package."*"] debug = "line-tables-only"` (or `false`), `split-debuginfo`; compare `target/` size after one full dev build; then N11                                           | Size down without hurting debugging                                                                           |
| E8 | Bit-exact loop interchange in `dct_64_to_16` (`crates/analysis/src/pdq.rs:217-236`), **already applied**                            | At 41eaa1c the same function was the i-j-k loop (`crates/core/src/pdq.rs:176-198`). The current code runs i, k, j, with each output's additions in the same order. Rust does not fuse `a*b+c`, so hashes should stay bit-identical, and the loop lets LLVM vectorize across j. The new loop first appears in commit 88e780b (the squash), so its landing commit cannot be dated further. Stage 2 reads `d[j][k]` directly, with no transposed copy. Tests assert exact bits against the reference crate (`matches_reference_crate`, `pdq.rs:360`)                                             | Measure against the baseline. N17 remarks show whether the loop vectorizes                                                                                                                  | Bit-identical hashes and the threshold                                                                        |

## 3. Nightly tooling and options (opt-in runs; adoptable under D2 if they win)

| ID  | Evaluate                                                                                                                                                                                                    | Evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | How                                                                                                                                                                     | Gate                                                                                                                                                                     |
|-----|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| N5  | `cargo-fuzz` (libFuzzer) on parsers of untrusted bytes                                                                                                                                                      | Superseded: a top-level `fuzz/` crate has seven targets (`fuzz/fuzz_targets/`: codec_transform, container_chunks, container_jpeg, edit_rotate, metadata_read, model_keys, model_query_json), and `just fuzz-smoke` runs them through `tools/fuzz_smoke.py`. They are not the per-crate directories planned earlier. Targets: `Meta::read` (`crates/metadata/src/lib.rs:55`), `orientation` and `patch_orientation` (`crates/metadata/src/orientation.rs:14` and `:60`; the latter rewrites the user's files through write-back), `animation::frames` (`crates/codec/src/animation.rs:29`), and the codec wrappers (`crates/codec/src/codecs.rs`). `pg-metadata` and `pg-codec` exist as packages | The `fuzz/` targets, seeded from the test samples; a fixed time per target in the container; check that `patch_orientation` output re-parses with the orientation asked | Any crash, hang or OOM: fix with a regression test                                                                                                                       |
| N13 | Branch coverage: `-Cinstrument-coverage -Zcoverage-options=branch` (`mcdc` is rejected on this nightly; not in the book text, unverified)                                                                   | The `coverage-options` page documents branch coverage. Needs `llvm-tools`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Tests plus N5's corpus, scratch target dir; read uncovered parser branches                                                                                              | Informational; add tests                                                                                                                                                 |
| N6  | `-Zsanitizer=address` over the tests, aimed at the C boundaries: turbojpeg, the FFmpeg bindings of `crates/video` (its own FFI, not `ffmpeg-next`), `crates/platform/src/dir.rs`, `crates/video/src/lib.rs` | The MSVC target supports only `address` (not in the book text; unverified). Needs the ASan runtime DLL on `PATH`. Link-and-run not yet tried; C libraries stay uninstrumented                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | A trivial crate first, then `pg-codec` and `pg-library` tests with `--target x86_64-pc-windows-msvc`                                                                    | Fix what it finds                                                                                                                                                        |
| N1  | `-Zub-checks=yes` on a release test run, and `cargo careful`                                                                                                                                                | Dev and test already check UB (debug assertions); this adds release and `dhat` coverage only. Failures abort. The `from_raw_parts` calls in `crates/` are `platform/src/dir.rs:392` (sound: the API-filled buffer) and four in `crates/video` (`ffi.rs` three times, `ffi/io.rs` once). More unsafe in `video` (105 source lines containing `unsafe`), `platform` (80, `process.rs` among them) and `crates/desktop` (70), by a grep of the current tree (line counts, not the 472 lint hits in `CLIPPY_LEDGER.md`)                                                                                                                                                                              | One run with `RUSTFLAGS="-Zub-checks=yes"`, own target dir                                                                                                              | Informational, low priority                                                                                                                                              |
| N10 | `-Zannotate-moves[=N]`                                                                                                                                                                                      | Debug info only: big moves appear as `core::profiling::compiler_move::<T, SIZE>` frames above `memcpy`. libc memcpy, memset and malloc together were 2.7% and 3.2% with no Rust attribution (`PROFILING.md`). Whether callgrind plus `cgtab.py` shows the frames is unverified                                                                                                                                                                                                                                                                                                                                                                                                                   | Container bench build with the flag; counts must equal the baseline (self-check); list memcpy callers by annotated type                                                 | Informational: avoidable large moves, or all in C libraries                                                                                                              |
| N12 | `-Zprint-type-sizes` of hot structs                                                                                                                                                                         | Recent commits moved data inline: `Sides` is an `ArrayVec` of `MAX_SIDES` = 12 (`crates/model/src/detection.rs:176`), and ThumbHash is held inline (`crates/model/src/analysis.rs:140`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | `cargo +nightly rustc -p pg-analysis -- -Zprint-type-sizes` (and `pg-model`), scratch dir; list types over a threshold                                                  | Fix only clearly wasteful padding                                                                                                                                        |
| N15 | `-Zmove-size-limit=N` (`large_assignments` lint)                                                                                                                                                            | `pdq.rs:46` (`dct16`), `:73` (`decimate`) and `:217` (`dct_64_to_16`) return `[f32; 256]` by value. `crates/model/src/grid.rs:41` pins `Entry` at 56 bytes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | `RUSTFLAGS=-Zmove-size-limit=512 cargo +nightly check`, scratch dir                                                                                                     | Informational; pairs with N10                                                                                                                                            |
| N16 | `-Zrandomize-layout -Zlayout-seed=N` over the tests                                                                                                                                                         | Catches reliance on unspecified `repr(Rust)` layout: the UTF-16 slice at `platform/src/dir.rs:392` (`Unit` is `u16` on Windows), FFI wrappers, transmutes in the forks. cxx shared structs are `repr(C)` (cxx, not checked here). std is not randomized without build-std                                                                                                                                                                                                                                                                                                                                                                                                                        | Several seeds, scratch dir                                                                                                                                              | Green across seeds                                                                                                                                                       |
| N17 | `-Cremark=all -Zremark-dir=<dir>`                                                                                                                                                                           | Shows whether `dct_64_to_16`, `box_1d` and the resize kernels vectorize. Output format unverified                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Release build of `pg-analysis` and `pg-codec` in the pg-dev container (`just linux-shell`; WSL is retired, commit 62b4540), then grep the remarks at the hot lines      | Informs E1 and E8                                                                                                                                                        |
| N14 | `portable_simd` in the `png` fork                                                                                                                                                                           | The one site that could move speed. In the fork at the pinned rev 2f176ed: the fork's `Cargo.toml` (`unstable = ["crc32fast/nightly"]`), `src/lib.rs:69` (`cfg_attr(feature = "unstable", feature(portable_simd))`), Paeth in `filter/paeth.rs`. Upstream PR #632 (cited at `paeth.rs:263`): +23% on Epyc 7B13, +10% on Zen 3 for 3- and 4-byte pixels. Never measured here. `png = "0.18"` is patched to the fork (the png entry in `[patch.crates-io]`) and built without the feature                                                                                                                                                                                                          | The PNG bench (`viewer_still`, with a case through `analyze` still to add), stable vs `--features png/unstable` on the nightly                                          | Threshold, and PNG decode being hot at all. Then enable under the pin, or port the loop to stable `core::arch` (the `fast_image_resize` fork already does AVX2 that way) |
| N18 | `loop_hints` (`#[rustc_unroll]` via `-Zcrate-attr=feature(loop_hints)`) on `pdq.rs:217-236`                                                                                                                 | In-order f32 sums do not vectorize by unrolling. Tracking #156874; the spelling may change (#159429)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Only if E8 measures worse                                                                                                                                               | Closed if E8 is adopted                                                                                                                                                  |
| N19 | Lints `strict_provenance_lints` and `non_exhaustive_omitted_patterns_lint`                                                                                                                                  | `strict_provenance_lints`: the site the plan cited (`crates/container/src/lib.rs:18`) is already the stable `.addr()`, and the other pointer-to-integer conversions in the tree (`log/src/lib.rs:175`, `platform/src/file.rs:168`, `platform/src/process.rs:125`) use `.addr()` too, so there is nothing to fix. `non_exhaustive_omitted_patterns_lint`: no `_ =>` over `image::ColorType` in `crates/codec/src/codecs.rs` (454 lines). `image::ColorType` is `#[non_exhaustive]` in image 0.25.10 (`src/color.rs:13`), so a wildcard over it would be a site. No site found by grep. The lints need a nightly `cargo check` to find any                                                         | `-Zcrate-attr=feature(...)` on a nightly `cargo check`                                                                                                                  | Fix any site the run finds on stable (explicit variants)                                                                                                                 |

## 4. Compile time and rebuild (gated on T1)

| ID  | Evaluate                                                                                                                                                                                                                    | Evidence                                                                                                                                                                                                                                                                                                                                                                                       | How                                                                                                                                                                                                                          | Gate                                      |
|-----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------|
| T1  | Dev build time breakdown                                                                                                                                                                                                    | Incremental is on for dev by default, off for release. Nothing is timed                                                                                                                                                                                                                                                                                                                        | Quiet tree: full dev build, then a one-line edit in `pg-model`, with `cargo build --timings` (already used by `tools/lint.py`); split compile, link of `gallery.exe` (Qt, FFmpeg) and build scripts; repeat for `pg-desktop` | Gates this section                        |
| N3  | `--jobs-frontend=N` (with `-Zunstable-options`; `-Zthreads` is superseded) and `-Zshare-generics=yes`                                                                                                                       | Both are accepted by the nightly. `jobs-frontend` is documented in the `threads` page. Front-end time matters for `check` and clippy on `pg-desktop` (cxx-qt bridge). `opt-level = 3` dependencies may duplicate generics                                                                                                                                                                      | Re-time T1 one flag at a time; `target/` size and bench counts for share-generics                                                                                                                                            | Clear time or size win; note the CPU cost |
| N8  | `-Zself-profile`, `-Ztime-passes`, `-Zllvm-time-trace`, `-Zdump-mono-stats`, `-Zmacro-stats`                                                                                                                                | `--timings` splits by crate only                                                                                                                                                                                                                                                                                                                                                               | Only if T1 shows compile time is the problem: slowest crate first                                                                                                                                                            | Diagnostic for N2 and N3                  |
| N9  | `-Zbuild-std`                                                                                                                                                                                                               | `rust-src` is not installed for the pinned nightly. It would let std use `x86-64-v3` and give sanitizers and ub-checks std coverage                                                                                                                                                                                                                                                            | Only if E1 pays, or for N6                                                                                                                                                                                                   | Clear win only                            |
| N11 | Cargo options: `-Zembed-metadata=no`, `-Zbuild-analysis` with `cargo report rebuilds`, `-Zfeature-unification` (`CARGO_RESOLVER_FEATURE_UNIFICATION=workspace`), `-Zchecksum-freshness` (`CARGO_BUILD_FINGERPRINT=content`) | The tree is exported into the container for benches and edited on Windows, so mtime-only freshness can rebuild workspace and fork crates for nothing (`vendor/` holds no code now). Duplicate dependency artifacts across `-p` and `--workspace` builds are unchecked. `ci.yml:45-46` already sets `CARGO_UNSTABLE_CHECKSUM_FRESHNESS` and `CARGO_BUILD_FINGERPRINT=content`                   | After E7: `cargo report rebuilds`, `target/` size, a no-op rebuild after a copy                                                                                                                                              | Adopt what removes rebuilds or disk       |
| N2  | Cranelift backend for our crates (`codegen-backend` profile key)                                                                                                                                                            | The Windows MSVC component `rustc-codegen-cranelift-nightly-x86_64-pc-windows-msvc` is in the 11-06 manifest and not installed. cg_clif has no Windows unwinding and partial `std::arch`; panic=abort is its default (both external, not checked here). Whether we rely on unwinding is unverified; `crates/desktop/src/panic_log.rs` installs a hook. The pin makes `cargo-features` possible | After T1, only if codegen is a large share; run the suite under it                                                                                                                                                           | WATCH: a clear win and green tests        |

## 5. Unstable features with a site in our code (candidates under the pin, Phase 6)

None is worth a pin alone; the question is whether they are worth it as a group. All need `#![feature]`.

| Feature                                                            | Site                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Gain and stable alternative                                                                                                                                                                                                                                                |
|--------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `nonpoison_mutex`, `nonpoison_condvar`, `sync_nonpoison` (#134645) | **55** lock or wait unwrap lines in `crates/` (2 in tests), in about 16 files: `crates/app` (`inbox.rs`, `library.rs`, `images.rs`), `crates/catalog` (`inbox.rs`), `crates/render` (`player.rs`), `crates/jobs` (`pipeline.rs`, `control.rs`, `events.rs`), `crates/thumbs`, `crates/trace`, `crates/log`, `crates/exiftool`, `crates/platform` (`process.rs`). The earlier file list had sites in `service.rs`, the library `runner.rs` and `thumbs.rs`, `thumbcache`, and the desktop `animation.rs` and `settings.rs`; those have none now | Removes the unwraps and the panic cascade after a poisoned lock. The stable form is already the usual one: `unwrap_or_else(PoisonError::into_inner)` appears 110 times in 27 files. Applying it to the remaining unwraps gives the same semantics, with no extension trait |
| `lock_value_accessors` (#133407)                                   | `crates/jobs/src/control.rs` and `crates/desktop/src/settings.rs` still exist. `writeback.rs` (library) and `app/src/state.rs` do not; `model/src/state.rs` does. Recount before relying on it                                                                                                                                                                                                                                                                                                                                                 | With nonpoison only; a two-line helper does it on stable                                                                                                                                                                                                                   |
| `mpmc_channel` (#126840)                                           | `crates/jobs/src/pipeline.rs:21,120,121`, the only crossbeam use                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Drops a dependency; no speed change expected                                                                                                                                                                                                                               |
| `oneshot_channel` (#143674)                                        | Two production `mpsc::channel` reply channels in `crates/thumbcache/src/lib.rs` (`:130`, `:298`). Lines 457–563 are tests, so the earlier count of seven was wrong                                                                                                                                                                                                                                                                                                                                                                             | A small saving per cache batch                                                                                                                                                                                                                                             |
| `iter_array_chunks` (#100450)                                      | `src/array_chunks.rs` in the `fast_image_resize` fork, a 14-unsafe-line polyfill                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Deletes the polyfill; same counts                                                                                                                                                                                                                                          |
| `read_array`, `read_le` (#148848, #156984)                         | No site now: `crates/cli/src/borders.rs:160-161` reads with `read_exact` and stable `u32::from_le_bytes`. No `read_le` or `read_array` use found                                                                                                                                                                                                                                                                                                                                                                                               | Nothing to change                                                                                                                                                                                                                                                          |
| `windows_by_handle` (#63010)                                       | `crates/platform/src/dir.rs:302-307` (the `volume` method, `FileIdInfo`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | One unsafe call fewer; the handle still comes from raw `CreateFileW` (`dir.rs:164`, `:283`)                                                                                                                                                                                |
| `windows_process_extensions_raw_attribute` (#114854)               | No exiftool site now (`crates/exiftool/src/lib.rs:46-53` is struct fields). The spawn is `crates/platform/src/process.rs` `spawn_background` (`:20-50`), which calls `cmd.spawn()` and then `die_with_parent` (`:104`), so the window before job assignment is still open                                                                                                                                                                                                                                                                      | Starts the ExifTool child inside the kill-on-close job, closing that window. A correctness gain, the most meaningful item in this table                                                                                                                                    |
| `panic_can_unwind` (#92988)                                        | `crates/desktop/src/panic_log.rs:13-35` (the hook)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | The hook already shows the error box only for uncaught panics (`panic_caught()`). The feature would add only the unwind information                                                                                                                                        |
| `casefold` (#157000)                                               | `crates/model/src/rules.rs:281` (`is_under`; `same_part` at `:300` allocates two lowercased vectors)                                                                                                                                                                                                                                                                                                                                                                                                                                           | Not a drop-in: Unicode folding differs from NTFS's upcase table                                                                                                                                                                                                            |
| `default_field_values` (#132162), `try_blocks`                     | `crates/cli/src/borders.rs` and `crates/desktop/src/settings.rs`: neither has a `Default::default()` update to convert, so no site is located. `examples/decodediff.rs` no longer exists                                                                                                                                                                                                                                                                                                                                                       | Readability only                                                                                                                                                                                                                                                           |
| `loop_match`, `const_continue` (#132306, stalled)                  | The only in-function state loop is `src/decompress.rs:971-973` in the `fdeflate` fork, not the needed shape                                                                                                                                                                                                                                                                                                                                                                                                                                    | None                                                                                                                                                                                                                                                                       |
| `integer_casts` family (#157388, #154650, #154330)                 | None: 0 `as <int>` casts in `crates/` outside `crates/model/src/num.rs` (11 there). The workspace denies `as_conversions` (in `[workspace.lints.clippy]`), so the family is obsolete                                                                                                                                                                                                                                                                                                                                                           | Watch: the API is new                                                                                                                                                                                                                                                      |
| `borrowed_buf` / `read_buf`                                        | `crates/analysis/src/analyze.rs:40-42` (`file_hash_of`, now `FileHash::of_reader`)                                                                                                                                                                                                                                                                                                                                                                                                                                                             | No site left: it streams the file through BLAKE3's `update_reader` (`crates/model/src/analysis.rs:24`), with no zero-filled buffer                                                                                                                                         |
| `float_mul_add_relaxed` (#151770), `float_algebraic`               | The DCT code in `crates/analysis/src/pdq.rs` (about lines 199–236). `float_mul_add_relaxed` is on the 11-06 book page; `float_algebraic` is in neither book, so check the name                                                                                                                                                                                                                                                                                                                                                                 | Unusable: PDQ is bit-exact with Meta's reference                                                                                                                                                                                                                           |

## 6. Closed with no site

The survey's per-letter and total counts do not reconcile with either book. The books have 213 (a–g) and 317 (h–z)
library pages on 09-22, so 530 in all; the survey recorded 176 and 235 examined, plus 133 bulk-excluded. Flags:
the survey recorded 177 examined and 72 excluded, against 248 pages on 09-22 and 251 on 11-06. Language: the survey
recorded 208 examined and 60 excluded, against 258 pages on 09-22 and 264 on 11-06. Treat the counts as unverified. The
appendix lists are the record.

Notable closures (full lists in the appendix):

- `allocator_ext` (formerly `allocator_api`): the "malloc about 3%" figure combined memset, memcpy and malloc
  (`RtlAllocateHeap` alone was 0.5% in the `gallery status` profile, `PROFILING.md:472`). Buffers are reused and the
  decoders use C malloc.
- `btree_cursors`, `btree_merge`, `btree_set_entry`: no BTree range or bound queries in `crates/` (checked; the forks are
  not checked).
- `hint_prefetch`: the strided access it would paper over is already fixed (E2). `likely_unlikely`: `cold_path` is stable
  (Phase 0a), and no branch problem is measured.
- `target_feature` / `effective_target_features`: the `fast_image_resize` fork already uses stable `#[target_feature]` with
  `core::arch`. This CPU has no AVX-512, APX, AVX10 or AMX (external fact, not checked here).
- `explicit_tail_calls`, `instrument_fn` and `-Zinstrument-xray` (XRay is unsupported on this target; trace stages are
  plain functions), `static_align`, `fn_align`, `f16` (tiff uses `half`), `gen_blocks`, `specialization`,
  `generic_const_exprs`: no site.
- `-Zhint-mostly-unused` (the mostly-unused crates are extern declarations, nothing to defer), `-Ztune-cpu` (stable
  `-Ctarget-cpu` covers it), `-Zthreads` (superseded by `--jobs-frontend`), `-Zlocation-detail`, `-Zfunction-sections`,
  `-Zfmt-debug`, `-Zretpoline`, `-Zcf-protection`: no benefit here.
- `pointer_is_aligned_to`: stable `ptr.is_aligned()` (1.79) would cover the record walk in `crates/platform/src/dir.rs`
  (around `:380-392`; `:139-143` is now another function). The old `scan.rs:116` check is gone.

## Critical files

- Edited by adoptions: `Cargo.toml` (profiles); `.cargo/config.toml` (a new file, for `target-cpu`);
  `crates/analysis/src/pdq.rs` (E8 is already applied, so only a measurement or a revert would edit it); `docs/reference/PROFILING.md`
  (results). With the pin: `rust-toolchain.toml`, `tools/profiling/bench.py`, and the section 5 sites.
- New: bench inputs in `crates/bench/benches/ir/linux.rs`. `fuzz/` exists as a top-level crate with seven targets, so no
  per-crate `fuzz/` directories are needed.
- Reused as is: `tools/profiling/bench.py`, `tools/linux.py`, `tools/profiling/cgtab.py`, `tools/profiling/bench_data.py`,
  `crates/bench/benches/ir.rs`, and the `justfile` recipes. `just lint` runs fmt and clippy with `-D warnings` (`cargo deny` is its own recipe, which CI runs),
  `cargo machete`, ruff and markdownlint.

## Verification

- Every adopted change: `just lint` and the crate tests pass. PDQ hashes and thumbnails stay byte-identical where the
  change is meant to preserve behavior (E1, E3, E5, E6, E8).
- Every result is reproducible from `PROFILING.md`: tool, flags, commit and counts. Two gungraun runs of one variant agree
  ("No change"). A variant that cannot be told from noise is closed, not adopted.
- Phase 0 results (stable features, popcount asm) are reviewed before Phase 2 relies on them.
- Rejected items keep their reason in this file and in the appendix, so they are not re-argued.

## Review provenance

On 2026-11-14 this plan was checked against the tree and the local toolchains. Nothing was built, tested or benchmarked.
The sources were grep, git history, the Unstable Book pages in the 09-22 and 11-06 toolchains, the stable std source, and
the fork checkouts in the cargo cache at the pinned revs.

- **Confirmed:** the pin and the profiles; the CI environment variables; the seven fuzz targets; the feature names and
  tracking issues cited (all but `float_algebraic`); the stability of the Phase 0a APIs; the png fork's `unstable`
  feature, its `portable_simd` gate and the PR #632 numbers; `image::ColorType` being `#[non_exhaustive]`; the cranelift
  component in the 11-06 manifest; the unsafe counts; the cast count; and the PROFILING figures for E3, E4 and N10.
- **Corrected in this version:** the bench entry points and the env-var forwarding gap; E8, which is already applied; the
  WSL reference (N17); the FFmpeg binding (N6), which is the crate's own FFI rather than `ffmpeg-next`; the `vendor/`
  references (N1, N11); the cast count; the nonpoison, oneshot and raw-attribute sites; the line numbers throughout.
- **Still open, needs a build or a run:** the popcount asm (Phase 0b); PGO's `llvm-tools` component (E6); the MSVC-only
  `address` sanitizer claim and the `mcdc` rejection (N6, N13), neither of which is in the book text; whether the
  `-Zannotate-moves` frames appear in callgrind (N10); the ASan link-and-run (N6); the cg_clif Windows unwinding claim (N2);
  whether `target/` size is mostly debuginfo (E7); and the 48 GiB figure.
- **External facts, not checked here:** POPCNT in the x86-64 baseline; that Rust never fuses `a*b+c`; cxx's `repr(C)`
  layout of shared structs; the absence of AVX-512 and the other extensions on this CPU.
- **The closed-entry sweep:** 599 closed names were searched for API-name fragments in `crates/` and `tools/`. 96 had
  token hits. The candidates that could be unstable APIs (`wrapping_next_power_of_two`, `maybe_uninit_array_assume_init`,
  `nonzero_from_mut`, `current_thread_id`, `prefetch`, `float_conversions`, `option_reference_flattening`,
  `int_from_ascii`, `core_intrinsics`) are stable APIs, the crate's own functions, or comments. The other hits are common
  words, not checked one by one. The names with no hit are weak evidence of absence, not proof.
- **Sixteen entries** were listed as unclassifiable (library: numfmt, temporary_niche_types, sized_type_properties,
  partial_ord_chaining_methods, legacy_receiver_trait, inplace_iteration, slice_concat_ext, slice_index_methods,
  prelude_next, trusted_fused, trusted_random_access, windows_net, ip_multicast_reserved, ipv6_hop_limit,
  io_error_uncategorized; flag: metrics_dir). Each also appears in the appendix as closed, so the appendix is the record.

## Appendix: closed entries by reason (all examined; none has a site in `crates/` unless noted)

**Library features, const-evaluation only (nothing is const-evaluated):** const_alloc_error, const_array,
const_atomic, const_binary_search, const_bool, const_btree_len, const_carrying_mul_add, const_cell_traits,
const_clone, const_cmp, const_control_flow, const_convert, const_default, const_drop_in_place, const_eval_select,
const_heap (the DCT matrix needs runtime `cos`), const_index, const_iter, const_manually_drop_take,
const_nonnull_with_exposed_provenance, const_nonzero_strict_ops, const_ops, const_option_ops, const_param_ty_trait,
const_path_separators, const_range_bounds, const_raw_ptr_comparison, const_ref_cell, const_result_trait_fn,
const_result_unwrap_unchecked, const_select_unpredictable, const_slice_from_mut_ptr_range,
const_slice_from_ptr_range, const_slice_make_iter, const_split_off_first_last, const_swap_with_slice,
const_try_residual, const_type_name, const_unsigned_bigint_helpers, const_volatile.

**Library, collections and containers unused:** binary_heap_as_mut_slice, binary_heap_drain_sorted,
binary_heap_from_raw_vec, binary_heap_into_iter_sorted, binary_heap_peek_mut_refresh, binary_heap_pop_if, btree_cursors,
btree_merge, btree_set_entry, btreemap_alloc, deque_extend_front, linked_list_cursors, linked_list_remove,
linked_list_retain, vec_deque_extract_if, vec_deque_iter_as_slices, vec_deque_retain_range, vec_as_non_null,
vec_fallible_shrink, vec_into_chunks, vec_peek_mut, vec_push_within_capacity, vec_recycle, vec_split_at_spare,
try_with_capacity (stable `try_reserve`; `crates/` no longer uses it), try_reserve_kind, try_entry, map_try_insert,
hash_map_macro and hash_set_entry (std hash containers are banned by `clippy.toml`), thin_box, unique_rc_arc, arc_is_unique,
arc_raw_get_strong, box_as_non_null, box_into_boxed_slice, box_into_inner, box_take, cow_is_borrowed, cell_get_cloned,
cell_leak, refcell_try_map, lazy_cell_into_inner, once_cell_get_mut, once_cell_new_init, once_cell_try,
once_cell_try_insert, once_lock_new_init, once_lazy_lock_get_unchecked, sync_unsafe_cell.

**Library, memory, pointers, unsafe (no site or no gain):** align_to_uninit_mut, alloc_error_hook, alloc_slice_into_array,
array_into_iter_constructors, array_ptr_get, cast_maybe_uninit, clone_from_ref, clone_to_uninit, coerce_unsized,
covariant_unsafe_cell, deref_pure_trait, downcast_unchecked, forget_unsized, freeze, get_mut_unchecked,
maybe_dangling, maybe_uninit_array_assume_init, maybe_uninit_as_bytes, maybe_uninit_fill, maybe_uninit_from_ref,
maybe_uninit_uninit_array_transpose, mem_copy_fn, mutex_data_ptr, ptr_alignment_type, ptr_as_uninit, ptr_cast_array,
ptr_mask, ptr_metadata, raw_slice_split, rwlock_data_ptr, reentrant_lock_data_ptr, set_ptr_value, slice_from_ptr_range,
slice_ptr_get, slice_swap_unchecked, str_from_raw_parts, transmutability, transmute_neo, transmute_prefix,
pointer_is_aligned_to (stable `is_aligned`), unsafe_pinned, unsafe_unpin, unsize, trivial_clone, atomic_ptr_null,
atomic_volatile, generic_atomic, integer_atomics, abort_immediate, abort_unwind, panic_always_abort,
panic_backtrace_config, panic_update_hook, backtrace_frames, return_address, breakpoint.

**Library, numbers, floats, time:** clamp_bounds, clamp_magnitude, clamp_to, cmp_minmax, cmp_splat, complex_numbers,
convert_float_to_int, core_float_math, disjoint_bitor, exact_bitshifts, exact_div, f32_from_f16, float_bits_const,
float_conversions, float_erf, float_exact_integer_constants, float_gamma, float_masks, float_minimum_maximum,
float_nan_to, more_float_constants, funnel_shifts, int_from_ascii, int_roundings (all `div_ceil` sites unsigned,
stable), nonzero_bitwise, nonzero_from_mut, nonzero_ops, nonzero_strict_ops, signed_bigint_helpers, uint_carryless_mul,
uint_gather_scatter_bits, widening_mul, wrapping_int_impl, wrapping_next_power_of_two, extended_float_error_kind,
duration_constants, duration_constructors, duration_integer_division, duration_millis_float, duration_units,
time_saturating_systemtime, time_systemtime_limits, deadline_api, thread_sleep_until.

**Library, strings, chars, paths, OS strings:** ascii_char, ascii_char_variants, bstr, bstr_to_string, byte_str,
byte_str_to_string, case_ignorable, char_is_xid, char_to_u32, char_unassigned_private_use, concat_bytes, cstr_bytes,
cstr_display, default_ignorable, normalize_lexically, os_str_slice, os_str_split_at, os_string_truncate, path_absolute_method
(stable `std::path::absolute` already used at `crates/cli/src/borders.rs:54`), path_trailing_sep, str_as_str, str_copy_from_str,
str_lines_remainder, str_split_inclusive_remainder, str_split_remainder, str_split_whitespace_remainder,
string_remove_matches, string_replace_in_place, titlecase, u8_split_ascii_whitespace, utf16_extra, pattern,
slice_pattern, fmt_arguments_from_str, formatting_options, debug_closure_helpers, hasher_prefixfree_extras.

**Library, iterators, options, results, slices:** iter_advance_by, iter_collect_into, iter_intersperse,
iter_is_partitioned, iter_macro, iter_map_windows, iter_next_chunk, iter_order_by, iter_partition_in_place,
iterator_try_collect, iterator_try_reduce, extend_one, extend_one_unchecked, exact_size_is_empty, control_flow_into_value,
drain_keep_rest, bound_as_ref, bound_copied, one_sided_range, range_bounds_is_empty, range_into_bounds,
new_range_remainder, next_index, option_array_transpose, option_get_or_try_insert_with, option_into_flat_iter,
option_reduce, option_reference_flattening, option_zip, rev_into_inner, try_find, try_trait_v2, try_trait_v2_residual,
try_trait_v2_yeet, array_try_from_fn, array_try_map, slice_concat_ext, slice_concat_trait, slice_index_methods,
slice_iter_mut_as_mut_slice, slice_partial_sort_unstable, slice_partition_dedup, slice_range, slice_shift,
sliceindex_wrappers, sort_floats (stable `select_nth_unstable_by` already used at `crates/analysis/src/borders/stats.rs:50`), split_array,
split_as_slice, step_trait, trusted_len, trusted_step, trusted_fused, trusted_random_access, partial_ord_chaining_methods,
inplace_iteration, sized_type_properties, tuple_trait, type_ascription, type_info, variant_count, try_as_dyn.

**Library, I/O, fs, process, threads, sync, net:** abort/exit items: exit_status_error, exitcode_exit_method,
command_resolved_envs; io: alloc_io, core_io, can_vector, buf_read_has_data_left, bufreader_peek, cursor_split,
file_buffered, io_const_error, io_error_inprogress, io_error_input_output_error, io_error_more,
io_error_too_many_open_files, io_error_uncategorized, io_slice_as_bytes, io_slice_const, read_buf, read_buf_at,
seek_io_take_position, seek_read_exact_seek_write_all, seek_stream_len, stdio_fd_consts, stdio_makes_pipe, stdio_swap,
write_all_vectored; fs and OS: dirfd, dir_entry_ext2, fs_home_dirs, fs_media_dirs, junction_point, rename_noreplace,
set_permissions_nofollow, windows_change_time, windows_freeze_file_times, windows_permissions_ext,
windows_process_extensions_async_pipes, windows_process_extensions_desktop, windows_process_extensions_force_quotes,
windows_process_extensions_startupinfo, windows_unix_domain_sockets, windows_c, windows_handle, windows_stdio,
windows_net, fd_view; threads and sync: current_thread_id, thread_id_value, thread_os_id, thread_raw,
thread_spawn_hook, exclusive_wrapper, mapped_lock_guards, mpsc_is_disconnected, nonpoison_rwlock, reentrant_lock;
net and misc: addr_parse_ascii, addr_unspecified_from, ip, ip_as_octets, ip_multicast_reserved, ipv6_hop_limit,
gethostname, random, error_generic_member_access, error_iter, error_reporter, error_type_id, c_size_t,
c_variadic_int128, c_variadic_va_arg_safe, cfg_accessible, cfg_eval, codeview_annotation, log_syntax, trace_macros,
numfmt, temporary_niche_types, legacy_receiver_trait, prelude_next, hint_must_use, hint_prefetch, likely_unlikely,
core_intrinsics, ub_checks, autodiff, share_trait, pattern_type_macro, pattern_type_range_trait, phantom_variance_markers,
pin_coerce_unsized_trait, view_type_macro, windows_c.

**Language features, const and generics (no use):** adt_const_params, const_block_items, const_c_variadic,
const_closures, const_destruct, const_for, const_param_ty_unchecked, const_precise_live_drops, const_trait_impl,
const_try, generic_const_exprs, generic_const_items, generic_const_parameter_types, generic_const_args,
min_generic_const_args, macroless_generic_const_args, macroless_const_item_generic_const_args, gca_* (five),
function_arg_const_generics, min_adt_const_params, unsized_const_params, generic_pattern_types, pattern_types.

**Language features, cfg, target features, ABI, asm (nothing needs them; CPU is Coffee Lake, Windows MSVC):**
cfg_contract_checks, cfg_overflow_checks, cfg_relocation_model, cfg_sanitize, cfg_sanitizer_cfi,
cfg_target_compact, cfg_target_has_atomic, cfg_target_has_reliable_f16_f128, cfg_target_has_reliable_f16b,
cfg_target_has_threads, cfg_target_thread_local, cfg_ub_checks, cfg_version, cfi_encoding, apx_target_feature,
avx10_target_feature, clflushopt_target_feature, ermsb_target_feature, fma4_target_feature, lahfsahf_target_feature,
movdir64b_target_feature, movdiri_target_feature, movrs_target_feature, prfchw_target_feature, rtm_target_feature,
x86_amx_intrinsics, x87_target_feature, xop_target_feature, effective_target_features, abi_vectorcall,
abi_x86_interrupt, asm_const_ptr, asm_experimental_reg, asm_goto_with_outputs, asm_unwind, c_variadic_experimental_arch,
naked_functions_rustic_abi, naked_functions_target_feature, link_arg_attribute, link_cfg, link_llvm_intrinsics,
linkage, native_link_modifiers_as_needed, simd_ffi, repr_simd, ffi_const, ffi_pure, extern_types, extern_item_impls,
explicit_extern_abis, export_stable, patchable_function_entry, used_with_arg, rust_cold_cc, rust_preserve_none_cc,
rust_tail_cc, fn_align, static_align, thread_local, global_registration.

**Language features, syntax, types, traits, macros, diagnostics (cosmetic or unused):** alloc_error_handler,
anonymous_lifetime_in_impl_trait, arbitrary_self_types, arbitrary_self_types_pointers, associated_type_defaults,
auto_traits, checked_type_aliases, marked_unchecked_type_aliases, closure_lifetime_binder, closure_track_caller,
coverage_attribute, custom_inner_attributes, custom_test_frameworks, decl_macro, deprecated_suggestion, deref_patterns,
derive_from, diagnostic_on_const, diagnostic_on_move, diagnostic_on_type_error, diagnostic_on_unknown,
diagnostic_on_unmatched_args, diagnostic_opaque, doc_cfg, doc_masked, doc_notable_trait, dropck_eyepatch,
ergonomic_clones, exhaustive_patterns, field_projections, final_associated_functions, fmt_debug, fn_delegation,
fn_static, forced_keywords, freeze_impls, frontmatter, fundamental, gen_blocks, guard_patterns,
half_open_range_patterns_in_slices, impl_restriction, impl_trait_in_assoc_type, impl_trait_in_bindings,
impl_trait_in_fn_trait_return, import_trait_associated_functions, inherent_associated_types, intra_doc_pointers,
macro_attr, macro_derive, macro_guard_matcher, macro_metavar_expr, macro_metavar_expr_concat, marker_trait_attr,
min_specialization, more_qualified_paths, move_expr, multiple_supertrait_upcastable, mut_ref, mut_restriction,
named_fn_trait_parameters, negative_impls, never_patterns, new_range, non_lifetime_binders, offset_of_enum,
offset_of_slice, optimize_attribute, pin_ergonomics, postfix_match, proc_macro_hygiene, reborrow,
ref_pat_eat_one_layer_2024 (+ structural), register_tool, return_type_notation, rustdoc_missing_doc_code_examples,
sanitize, sized_hierarchy, specialization, splat, stmt_expr_attributes, super_let, supertrait_item_shadowing,
trait_alias, transmute_generic_consts, transparent_unions, trivial_bounds, try_blocks_heterogeneous,
type_alias_impl_trait, type_changing_struct_update, unboxed_closures, unnamed_enum_variants, unqualified_local_imports,
unsafe_binders, unsafe_fields, unsized_fn_params, where_clause_attrs, yeet_expr, explicit_tail_calls, instrument_fn,
f16, f16b, f128.

**Compiler flags, closed (examined, no benefit here):** allow_features, allow_partial_mitigations, always_encode_mir,
assert_incr_state, async_panic, binary_dep_depinfo, box_noalias, cache_proc_macros, cf_protection, check_cfg_all_expected,
codegen_emit_retag, codegen_options, contract_checks, crate_attr (used as the enabler above), debug_info_type_line_numbers,
debuginfo_compression, debuginfo_for_profiling, deduplicate_diagnostics, default_visibility, deny_partial_mitigations,
dep_info_omit_d_target, direct_access_external_data, disable_incr_comp_backend_caching, dual_proc_macros, dwarf_version,
dylib_lto, ehcont_guard, embed_source, emit_stack_sizes, export_executable_symbols, extern_options, external_clangrt,
extra_const_ub_checks, fewer_names, flatten_format_args, fmt_debug, force_intrinsic_fallback, function_return,
function_sections, has_thread_local, help, hint_mostly_unused, hint_msrv, human_readable_cgu_names,
ignore_directory_in_diagnostics_source_blocks, implicit_sysroot_deps, incremental_verify_ich, indirect_branch_cs_prefix,
inline_llvm, inline_mir_preserve_debug, instrument_mcount, instrument_xray (unsupported on this target),
large_data_threshold, link_directives, link_native_libraries, link_only, llvm_module_flag, llvm_plugins,
llvm_target_feature, location_detail, ls, macro_backtrace, merge_functions, metrics_dir (unknown), min_function_alignment,
min_recursion_limit, mir_enable_passes, mir_strip_debuginfo, move_size_limit (see N15), next_solver, no_analysis,
no_codegen, no_generate_arange_section, no_link, no_parallel_backend, no_parallel_llvm, no_profiler_runtime,
no_trait_vptr, no_unique_section_names, normalize_docs, on_broken_pipe, panic_abort_tests, panic_in_drop,
patchable_function_entry, plt, polonius, pre_link_arg, pre_link_args, precise_enum_drop_elaboration, print_check_cfg,
print_codegen_stats, print_codegen_stats_json, print_crate_root_lint_levels, print_llvm_passes, print_mono_items,
print_supported_crate_types, proc_macro_backtrace, proc_macro_execution_strategy, profile_closures, profiler_runtime,
remap_cwd_prefix, retpoline, retpoline_external_thunk, sanitizer_cfi_*(all, unsupported on this target),
sanitizer_dataflow_abilist, sanitizer_ignorelist, sanitizer_kcfi_arity, sanitizer_memory_track_origins,
sanitizer_recover, shell_argfiles, split_dwarf_inlining, split_dwarf_kind, split_dwarf_out_dir, split_lto_unit,
src_hash_algorithm, staticlib_* (four), strict_init_checks, teach, temps_dir, terminal_urls, thinlto, tls_model,
trace_macros, translate_remapped_path_to_local_path, trap_unreachable, trim_diagnostic_paths, tune_cpu, unsound_mir_opts,
unstable_options (enabler only), use_ctors_section, use_sync_unwind, verbose_asm, virtual_function_elimination,
write_long_types_to_disk. Open candidates among flags are the N-items of section 3: ub_checks, annotate_moves, print_type_sizes,
move_size_limit, randomize_layout, remark_dir, coverage_options, sanitizer (address), self_profile, time_passes,
llvm_time_trace, dump_mono_stats, macro_stats, share_generics, threads/jobs-frontend, codegen_backend,
cross_crate_inline_threshold, inline_mir/mir_opt_level, embed_metadata, incremental_info, codegen_source_order (WATCH),
stack_protector (WATCH).
