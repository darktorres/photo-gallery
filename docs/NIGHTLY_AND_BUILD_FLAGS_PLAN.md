# Plan: evaluate nightly features, build flags and stable performance levers

## Context

The question is which Rust nightly features the workspace could use, and every one is evaluated properly rather than
spot-checked. The survey covered the whole Unstable Book of `nightly-2026-09-22` (about 430 library
features, 250 compiler flags, 260 language features, cross-checked against the live index). Each entry was read and
searched for in `crates/` and `vendor/`, or excluded under a named rule (other targets, compiler or std internals,
async). The per-entry outcome is in the appendix so nothing is re-argued.

What the survey found:

1. **No unstable language or library feature pays off alone.** About a dozen have a real site (section 5); each is a
   cleanup or a small robustness gain. Only one could move performance (N14, png SIMD), and it needs a measurement.
2. **Nightly's real value here is tooling**, usable without touching product code: fuzzing, ASan, move attribution,
   type-size audits, optimization remarks, layout randomization, compile-time profiling.
3. **The largest likely wins are stable and untried:** `-C target-cpu=x86-64-v3`, release LTO / `codegen-units = 1`,
   PGO, a bit-exact loop interchange in the PDQ DCT, and the dev profile's debuginfo (`target/` was 48 GiB).
4. **Every performance number in the repo is stale.** `docs/PROFILING.md` is measured at 41eaa1c; since then commit
   81ad925 fixed the strided column pass in PDQ `box_1d` (so the "box_1d is 21% of analyze, cache-miss-bound" lead is
   gone) and further commits changed allocation and copying. Phase 1 re-measures before anything is judged.

Nothing has been built or measured yet; `target/` was cleaned (48 GiB freed). Product code is untouched.

Status (2026-11-06): the pin that Phase 6 decides on is taken: `rust-toolchain.toml` pins `nightly-2026-11-06` (commit
edf7a39). The survey below is at `nightly-2026-09-22`. Whether the phases below were run is not checked here.

## Decisions (confirmed)

| # | Question | Decision |
|---|---|---|
| D1 | Does the binary only run on this machine class (i7-8700, AVX2)? | **Yes.** `-C target-cpu=x86-64-v3` may be adopted if it measures well. |
| D2 | Is a nightly pin acceptable for the workspace (`rust-toolchain.toml`) or only for opt-in runs? | **A pin is acceptable.** `#![feature]`, `cargo-features` and `-Z` flags in the product build are allowed. |

What a pin costs, to weigh per adoption (not a blocker): build, clippy, `cargo deny` and the container benches all move to
the pinned nightly; its date is bumped by hand and features can break or be renamed on a bump (`allocator_api` became
`allocator_ext`); crates using `#![feature]` no longer build on stable. So: try everything first in a scratch target
dir (`-Zcrate-attr=feature(x)` enables a feature without editing source), and prefer a stable equivalent when it gives
the same result.

## How the work runs

- **Benchmarks:** `just perf` (`tools/profiling/bench.sh`) exports the *committed* HEAD into the container and runs gungraun (callgrind) on
  `crates/bench/benches/ir.rs`: `analyze_jpeg`, `thumb_jpeg` (`ir/linux.rs`) and the catalog benches (`ir/catalog.rs`).
  Results go to `/bench/target/gungraun/` in the container and are read with `tools/profiling/cgtab.py`. Counts are
  deterministic, so one run per variant is enough. Uncommitted work is never benched, so:
  - flag and profile trials go in through the environment (`RUSTFLAGS`, `CARGO_PROFILE_BENCH_LTO`,
    `CARGO_PROFILE_BENCH_CODEGEN_UNITS`), passed into the container with `-e` (`tools/linux.py`, to verify on first use);
  - code trials (E8) are committed on a short-lived local branch `eval/<id>`, benched, then merged or deleted.
- **Nightly in the container:** the image has the pinned nightly only; Phase 0 installs `nightly-2026-09-22` in the container too (needed for N5, N10, N17).
- **Scratch target dirs** outside the repo (`CARGO_TARGET_DIR=<scratch>\cargo-targets\<name>` on Windows,
  `/bench/target-<name>` in the container) for every nightly or flag experiment, so the main `target/` is never rebuilt by them.
- **No Windows build, test or timing while other edits are in progress in the tree**; check `git status` before each
  Windows step.
- **Adoption threshold:** a performance change is adopted when estimated cycles
  on the affected bench fall by at least 3% with the tests green and, for PDQ and thumbnails, byte-identical output;
  otherwise it is closed with the numbers recorded. Robustness findings (crashes, UB) are fixed regardless of size.
- **Measure at the end of the steps, not per step**, except where a number decides the next step (E8 depends on E0).
- Commit each verified step separately, with the measured counts in the message; no unrelated changes in a commit; push
  is a separate step. Results also go in `docs/PROFILING.md` (dated subsection: commit, tool, counts; "Open" list updated).

## 1. Phases (the order of work)

**Phase 0: checks that need no build of the tree.**

- 0a. Plain stable compile (`rustc +stable`, no `#![feature]`) of tiny files in a scratch directory outside the repo using `core::hint::cold_path`,
  `<[T]>::array_windows`, `ptr.is_aligned()`, `split_first_chunk`/`first_chunk`. Settles which survey entries are already
  stable (the first probe was invalid: stable rejects `#![feature]` outright).
- 0b. `rustc +stable -O --emit asm` of a copy of `Hash::distance`'s body (`crates/model/src/pdq.rs:27`), at baseline and
  with `-C target-cpu=x86-64-v3`: is it a software popcount today? Decides how much of E1 is about POPCNT.
- 0c. `rustup component list --toolchain nightly-2026-09-22` for what is installed (rust-docs only so far); install
  the nightly in the container (`rustup toolchain install nightly-2026-09-22 --profile minimal`, in `just linux-shell`) and the components later phases need.

**Phase 1: baseline at HEAD.**

- 1a. Add the missing bench inputs, one commit each: PDQ index nearest-neighbour search (`crates/dedup/src/pdq_index.rs`,
  `best_matches` 239 and `query` 261, has no bench) and a PNG decode through `analyze`. Both in
  `crates/bench/benches/ir/linux.rs` beside the existing ones
  (not added yet: the file still has only `analyze_jpeg` and `thumb_jpeg`).
- 1b. `just linux-bench --save-baseline=head`. Read with `cgtab.py`.
  Record counts and the new top self-cost lines in `PROFILING.md`; replace the stale 41eaa1c leads (E2, box_1d).

**Phase 2: stable levers, one batched pass in the container** (each against `--baseline=head`): E1, then E5, then E8 if the
baseline shows `dct_64_to_16` is visible, then E6 (needs E5's final profile). E3 and E4 are measurements/decisions
that need no bench. Adopt winners by the threshold above; each adoption is its own commit and `just lint` + the crate
tests must pass.

**Phase 3: nightly measurement runs** (no product-code change; scratch dirs): N10 and N17 in the container on the bench build,
N12 and N15 as `check`/`rustc` dumps, N14 (needs the PNG bench from 1a), N19, N1, N16.

**Phase 4: robustness tooling:** N5 fuzzing in the container (longest-running; start early and let it run while Phase 3 is read),
N13 coverage from the same corpus/tests, N6 ASan on Windows after a link-and-run proof on a trivial crate.

**Phase 5: compile time and footprint** (Windows, needs a quiet tree): T1 timings first, then E7, then N3, N8, N9,
N11, N2 only as T1 shows they could matter.

**Phase 6: the pin decision.** With every number in hand: take a `rust-toolchain.toml` pin only if the wins justify
moving the whole build (candidates: N14, N2, any `-Z` option that measured well, the section 5 cleanups taken as a
group). Otherwise stay on stable and keep nightly for opt-in tooling. If taken: the pin and the changes it enables are
separate commits, `just lint` and the full tests run on the pinned toolchain, and the bench script runs on the same pin (it builds `/head` with its `rust-toolchain.toml`).

**Phase 7: close out.** `PROFILING.md` final numbers, delete the scratch target dirs and branches, and report each item's result in the tracker.

## 2. Stable levers (D2-independent)

| ID | Evaluate | Evidence | How | Gate |
|---|---|---|---|---|
| E0 | Baseline at HEAD and stable-feature check | See Context 4; Phase 0 and 1 | Phase 0a, 1b | Everything below compares to it |
| E1 | `-C target-cpu=x86-64-v3` | `.cargo/config.toml` sets no `target-cpu`, so the x86-64 baseline. `Hash::distance` (`crates/model/src/pdq.rs:27`) and the Hamming scan (`crates/dedup/src/pdq_index.rs:323`) use `count_ones`; POPCNT is not in baseline x86-64 (asm unverified, Phase 0b). Also AVX2 autovectorization of our own loops; the JPEG, resize and zlib kernels already pick their own SIMD paths | `RUSTFLAGS="-C target-cpu=x86-64-v3"` into the container with `-e`; compare `analyze_jpeg`, `thumb`, the new PDQ-search and PNG benches | Threshold; D1 allows adopting. A flag in `.cargo/config.toml` `[build]` also applies to the Windows build |
| E2 | PDQ after 81ad925 | Premise gone (Context 4) | Re-read the Phase 1 profile; update the `PROFILING.md` lead | Close if `pdq` no longer dominates cache misses |
| E3 | Lanczos weight cache (`precompute_coefficients` in the `fast_image_resize` fork, a git pin) | 3.2 M of 50.6 M instructions per thumbnail (6.4%) at 41eaa1c, mostly libm `sin` | Count (source, target) size repeats in a real `gallery thumbs` pass on a scratch catalog; prototype a cache only if repeats are common | High repeat rate and the threshold |
| E4 | `DECODE_MARGIN = 4` (full-scale decode vs DCT scaling) | 28.5 M vs 21.2 M instructions for images just under 4x | A quality comparison (FINDINGS "Thumbnails"), not a speed one | A decision for the maintainer; the price is recorded |
| E5 | Release `lto = "fat"`/`"thin"` and `codegen-units = 1`; nightly variants `-Zcross-crate-inline-threshold=always`, `-Zinline-mir` | `[profile.release]` sets only `debug = "line-tables-only"`; no cross-crate inlining between the pg-* crates, the vendored decoders and std; release is non-incremental so inference is on | `CARGO_PROFILE_BENCH_LTO` / `_CODEGEN_UNITS` into the container with `-e`, also build time; nightly variants only if the stable ones leave a gap | Threshold and acceptable build time |
| E6 | PGO (`-Cprofile-generate` / `-Cprofile-use`) | Stable; needs `llvm-profdata` (`llvm-tools` component) | Instrument a thumbnail and import run on a scratch catalog, rebuild, compare | After E5; threshold and a workable profile-refresh process |
| E7 | `target/` footprint (48 GiB, 78,053 files) | `Cargo.toml` leaves dev `debug` unset (full) and builds dependencies at `opt-level = 3` (inferred cause, unmeasured) | `[profile.dev.package."*"] debug = "line-tables-only"` (or `false`), `split-debuginfo`; compare `target/` size after one full dev build; then N11 | Size down without hurting debugging |
| E8 | Bit-exact loop interchange in `dct_64_to_16` (`crates/analysis/src/pdq.rs:218-242`) | The k-sums are in-order f32 and `input[k][j]` is read down a column. Looping i, k, j keeps each output's operation sequence (Rust never fuses `a*b+c`), so hashes stay bit-identical, and lets LLVM vectorize across j; stage 2 needs the DCT matrix transposed once (same values). Tests assert exact bits against the reference crate (`matches_reference_crate`, `pdq.rs:361`) | Only if the Phase 1 profile shows it visible. Branch `eval/e8`; N17 remarks show whether the current loop vectorizes | Bit-identical hashes and the threshold |

## 3. Nightly tooling and options (opt-in runs; adoptable under D2 if they win)

| ID | Evaluate | Evidence | How | Gate |
|---|---|---|---|---|
| N5 | `cargo-fuzz` (libFuzzer) on parsers of untrusted bytes | No fuzzing existed when this was written. Superseded: a top-level `fuzz/` crate has seven targets (`fuzz/fuzz_targets/`: codec_transform, container_chunks, container_jpeg, edit_rotate, metadata_read, model_keys, model_query_json) and `just fuzz-smoke` runs them (`tools/fuzz-smoke.sh`); they are not the per-crate directories planned here. Targets: `Meta::read` (`crates/metadata/src/lib.rs:101`), `orientation` (148), `patch_orientation` (635; rewrites the user's files through write-back), `animation::frames`, the codec wrappers (`crates/codec/src/codecs.rs`). `pg-metadata` and `pg-codec` build on Linux | `fuzz/` targets (in `crates/metadata/` and `crates/codec/`) seeded from the test samples; a fixed time per target in the container; check `patch_orientation` output re-parses with the orientation asked | Any crash, hang or OOM: fix with a regression test |
| N13 | Branch coverage: `-Cinstrument-coverage -Zcoverage-options=branch` (`mcdc` is rejected on this nightly) | Needs `llvm-tools` | Tests plus N5's corpus, scratch target dir; read uncovered parser branches | Informational; add tests |
| N6 | `-Zsanitizer=address` over the tests, aimed at the C boundaries (turbojpeg, ffmpeg-next, `crates/platform/src/dir.rs`, `crates/video/src/lib.rs`) | The MSVC target supports only `address`. Needs the ASan runtime DLL on `PATH`. Link-and-run not yet tried; C libraries stay uninstrumented | A trivial crate first, then `pg-codec` / `pg-library` tests with `--target x86_64-pc-windows-msvc` | Fix what it finds |
| N1 | `-Zub-checks=yes` on a release test run, and `cargo careful` | Dev and test already check UB (debug-assertions); this adds release / `dhat` coverage only. Failures abort. The only `from_raw_parts` in `crates/` is `platform/src/dir.rs:429` (sound: the API-filled buffer); more unsafe in `video`, `platform/src/process.rs`, `crates/desktop`, `vendor/` | One run with `RUSTFLAGS="-Zub-checks=yes"`, own target dir | Informational, low priority |
| N10 | `-Zannotate-moves[=N]` | Debug info only: big moves appear as `core::profiling::compiler_move::<T, SIZE>` frames above `memcpy`. libc memcpy / memset / malloc together were 2.7% / 3.2% with no Rust attribution. Whether callgrind plus `cgtab.py` shows the frames is unverified | container bench build with the flag; counts must equal the baseline (self-check); list memcpy callers by annotated type | Informational: avoidable large moves, or all in C libraries |
| N12 | `-Zprint-type-sizes` of hot structs | Recent commits moved data inline (ArrayVec of 12, inline ThumbHash) | `cargo +nightly rustc -p pg-analysis -- -Zprint-type-sizes` (and `pg-model`), scratch dir; list types over a threshold | Fix only clearly wasteful padding |
| N15 | `-Zmove-size-limit=N` (`large_assignments` lint) | `pdq.rs:74`, `:94` and `:218` return `[f32; 256]` by value (in the current file); `crates/model/src/grid.rs:41` pins `Entry` at 56 bytes | `RUSTFLAGS=-Zmove-size-limit=512 cargo +nightly check`, scratch dir | Informational; pairs with N10 |
| N16 | `-Zrandomize-layout -Zlayout-seed=N` over the tests | Catches reliance on unspecified `repr(Rust)` layout (the UTF-16 slice at `platform/src/dir.rs:429`, FFI wrappers, vendored transmutes); cxx shared structs are `repr(C)`; std is not randomized without build-std | Several seeds, scratch dir | Green across seeds |
| N17 | `-Cremark=all -Zremark-dir=<dir>` | Shows whether `dct_64_to_16`, `box_1d` and the resize kernels vectorize; output format unverified | Release build of `pg-analysis` and `pg-codec` in WSL, grep remarks at the hot lines | Informs E1 / E8 |
| N14 | `portable_simd` in the `png` fork | The one site that could move speed. `png` fork `Cargo.toml:41` (`unstable = ["crc32fast/nightly"]`), `png` fork `src/lib.rs:69` (`cfg_attr(feature = "unstable", feature(portable_simd))`), Paeth in `filter/paeth.rs`; upstream PR #632 (cited at `paeth.rs:262`): +23% on Epyc 7B13, +10% on Zen 3 for 3 / 4-byte pixels. Never measured here, and `png = "0.18"` is built without it | The 1a PNG bench, stable vs `--features png/unstable` on the nightly | Threshold and PNG decode being hot at all: then enable under the pin, or port the loop to stable `core::arch` (the `fast_image_resize` fork already does AVX2 that way) |
| N18 | `loop_hints` (`#[rustc_unroll]` via `-Zcrate-attr=feature(loop_hints)`) on `pdq.rs:218-242` | In-order f32 sums do not vectorize by unrolling; tracking #156874, the spelling may change (#159429) | Only if E8 is rejected | Closed if E8 lands |
| N19 | Lints `strict_provenance_lints` (`crates/container/src/lib.rs:16`, the only pointer-to-integer cast found) and `non_exhaustive_omitted_patterns_lint` (`crates/codec/src/codecs.rs:458-461`, a `_ =>` over `image::ColorType`; whether that enum is `non_exhaustive` is unchecked) | One site each | `-Zcrate-attr=feature(...)` on a nightly `cargo check` | Fix the sites on stable (`.addr()`, explicit variants) |

## 4. Compile time and rebuild (gated on T1)

| ID | Evaluate | Evidence | How | Gate |
|---|---|---|---|---|
| T1 | Dev build time breakdown | Incremental is on for dev by default, off for release; nothing is timed | Quiet tree: full dev build, then a one-line edit in `pg-model`, with `cargo build --timings`; split compile, link of `gallery.exe` (Qt, FFmpeg) and build scripts; repeat for `pg-desktop` | Gates this section |
| N3 | `--jobs-frontend=N` (with `-Zunstable-options`; `-Zthreads` is superseded) and `-Zshare-generics=yes` | Both accepted by the nightly. Front-end time matters for `check` / clippy on `pg-desktop` (cxx-qt bridge); `opt-level = 3` dependencies may duplicate generics | Re-time T1 one flag at a time; `target/` size and bench counts for share-generics | Clear time or size win; note the CPU cost |
| N8 | `-Zself-profile`, `-Ztime-passes`, `-Zllvm-time-trace`, `-Zdump-mono-stats`, `-Zmacro-stats` | `--timings` splits by crate only | Only if T1 shows compile time is the problem: slowest crate first | Diagnostic for N2 / N3 |
| N9 | `-Zbuild-std` | `rust-src` not installed. Lets std use `x86-64-v3` and gives sanitizers and ub-checks std coverage | Only if E1 pays or for N6 | Clear win only |
| N11 | Cargo options: `-Zembed-metadata=no`, `-Zbuild-analysis` with `cargo report rebuilds`, `-Zfeature-unification` (`CARGO_RESOLVER_FEATURE_UNIFICATION=workspace`), `-Zchecksum-freshness` (`CARGO_BUILD_FINGERPRINT=content`) | The tree is exported into the container for benches and edited on Windows; mtime-only freshness can rebuild the `vendor/*` path crates for nothing. Duplicate dependency artifacts across `-p` and `--workspace` builds are unchecked. Status: `ci.yml` now sets `CARGO_UNSTABLE_CHECKSUM_FRESHNESS` and `CARGO_BUILD_FINGERPRINT=content` | After E7: `cargo report rebuilds`, `target/` size, a no-op rebuild after a copy | Adopt what removes rebuilds or disk |
| N2 | Cranelift backend for our crates (`codegen-backend` profile key) | Component `rustc-codegen-cranelift-x86_64-pc-windows-msvc` available, not installed. cg_clif has no Windows unwinding and partial `std::arch`; panic=abort is its default. Whether we rely on unwinding is unverified (`crates/desktop/src/panic_log.rs` installs a hook). The pin makes `cargo-features` possible | After T1, only if codegen is a large share; run the suite under it | WATCH: a clear win and green tests |

## 5. Unstable features with a site in our code (candidates under the pin, Phase 6)

None is worth a pin alone; the question is whether they are worth it as a group. All need `#![feature]`.

| Feature | Site | Gain and stable alternative |
|---|---|---|
| `nonpoison_mutex`, `nonpoison_condvar`, `sync_nonpoison` (#134645) | **57** lock / wait unwrap sites in 10 files when counted, before the crate split (now spread over `crates/jobs` `pipeline.rs` `control.rs` `events.rs` `service.rs`, `crates/library` `runner.rs` `thumbs.rs`, `crates/thumbcache`, `crates/app`, `crates/desktop` `animation.rs` `settings.rs`; recount before relying on the number) | Removes the unwraps and the panic cascade after a poisoned lock. Stable today: an extension trait using `unwrap_or_else(PoisonError::into_inner)` (no site in `crates/` uses it now) gives the same semantics everywhere |
| `lock_value_accessors` (#133407) | sites counted before the split (`control.rs`, `writeback.rs`, `settings.rs`, `state.rs`); not re-located | With nonpoison only; a two-line helper does it on stable |
| `mpmc_channel` (#126840) | `crates/jobs/src/pipeline.rs:19,95,96`, the only crossbeam use | Drops a dependency; no speed change expected |
| `oneshot_channel` (#143674) | Seven `mpsc::channel` reply channels, `crates/thumbcache/src/lib.rs:457-563` | A small saving per cache batch |
| `iter_array_chunks` (#100450) | `src/array_chunks.rs` in the `fast_image_resize` fork, a 14-unsafe-line polyfill | Deletes the polyfill; same counts |
| `read_array`, `read_le` (#148848, #156984) | No site now: `crates/cli/src/borders.rs:154` reads with `read_exact` and stable `u32::from_le_bytes`; no `read_le` or `read_array` use found | Nothing to change |
| `windows_by_handle` (#63010) | `crates/platform/src/dir.rs:365-373` (the `volume` method, `FileIdInfo`) | One unsafe call fewer; the handle still comes from raw `CreateFileW` |
| `windows_process_extensions_raw_attribute` (#114854) | `crates/exiftool/src/lib.rs:46-53` (via `crates/platform/src/process.rs:22-48`, `die_with_parent`) | Starts the ExifTool child inside the kill-on-close job, closing the window before assignment. A correctness gain, the most meaningful of this table |
| `panic_can_unwind` (#92988) | `crates/desktop/src/panic_log.rs:17-29` | The panic message could say the app will close |
| `casefold` (#157000) | `crates/model/src/path.rs:117` (`is_under` allocates two lowercased strings) | Not a drop-in: Unicode folding differs from NTFS's upcase table |
| `default_field_values` (#132162), `try_blocks` | `crates/cli/src/borders.rs` and `crates/desktop/src/settings.rs` (lines not re-located; `examples/decodediff.rs` no longer exists) | Readability only |
| `loop_match`, `const_continue` (#132306, stalled) | The only in-function state loop is `src/decompress.rs:971-973` in the `fdeflate` fork, not the needed shape | None |
| `integer_casts` family (#157388, #154650, #154330) | About 546 `as <int>` casts in `crates/` | Watch: the API is new |
| `borrowed_buf` / `read_buf` | `crates/analysis/src/analyze.rs:46-50` (`file_hash_of`) | No site left: it streams the file with BLAKE3's `update_reader`, no zero-filled buffer |
| `float_mul_add_relaxed` (#151770), `float_algebraic` | `pdq.rs:201-224` | Unusable: PDQ is bit-exact with Meta's reference |

## 6. Closed with no site

Library: 176 (a-g) and 235 (h-z) entries examined, 133 bulk-excluded. Flags: 177 examined, 72 excluded. Language:
208 examined, 60 excluded. Notable closures (full lists in the appendix):

- `allocator_ext` (formerly `allocator_api`): the "malloc about 3%" figure combined memset, memcpy and malloc
  (`RtlAllocateHeap` alone was 0.5% in the `status` profile); buffers are reused and the decoders use C malloc.
- `btree_cursors`, `btree_merge`, `btree_set_entry`: no BTree range or bound queries in `crates/` or the forks.
- `hint_prefetch`: the strided access it would paper over is already fixed (E2). `likely_unlikely`: `cold_path` is
  stable (Phase 0a) and no branch problem is measured.
- `target_feature` / `effective_target_features`: the `fast_image_resize` fork already uses stable `#[target_feature]` with
  `core::arch`; this CPU has no AVX-512, APX, AVX10 or AMX.
- `explicit_tail_calls`, `instrument_fn` and `-Zinstrument-xray` (XRay is unsupported on this target; trace stages are
  plain functions), `static_align`, `fn_align`, `f16` (tiff uses `half`), `gen_blocks`, `specialization`,
  `generic_const_exprs`: no site.
- `-Zhint-mostly-unused` (the mostly-unused crates are extern declarations, nothing to defer), `-Ztune-cpu` (stable
  `-Ctarget-cpu` covers it), `-Zthreads` (superseded by `--jobs-frontend`), `-Zlocation-detail`, `-Zfunction-sections`,
  `-Zfmt-debug`, `-Zretpoline`, `-Zcf-protection`: no benefit here.
- `pointer_is_aligned_to`: stable `ptr.is_aligned()` would cover the record walk at `crates/platform/src/dir.rs:139-143` (the old `scan.rs:116` check is gone).

## Critical files

- Edited by adoptions: `Cargo.toml` (profiles), `.cargo/config.toml` (`target-cpu`), `crates/analysis/src/pdq.rs` (E8),
  `docs/PROFILING.md` (results), and with the pin `rust-toolchain.toml`, `tools/profiling/bench.sh`, section 5 sites.
- New: bench inputs in `crates/bench/benches/ir/linux.rs`; `fuzz/` directories under `crates/metadata/` and `crates/codec/` (N5; built as the top-level `fuzz/` crate instead).
- Reused as is: `tools/profiling/bench.sh`, `cgtab.py`, `bench_data.py`, `crates/bench/benches/ir.rs`, the
  `justfile` recipes (`lint`: fmt, clippy `-D warnings`, `cargo deny`, `cargo machete`, ruff, markdownlint).

## Verification

- Every adopted change: `just lint` and the crate tests pass; PDQ hashes and thumbnails stay byte-identical where the
  change is meant to preserve behavior (E1, E3, E5, E6, E8).
- Every result is reproducible from `PROFILING.md`: tool, flags, commit, counts. Two gungraun runs of one variant agree
  ("No change"). A variant that cannot be told from noise is closed, not adopted.
- Phase 0 results (stable features, popcount asm) are reviewed before Phase 2 relies on them.
- Rejected items keep their reason in this file and in the appendix so they are not re-litigated.

## Review provenance and caveats

- The survey (library a-g, library h-z, compiler flags, language features) read each entry's page or tracking issue
  and searched the code with targeted greps; not every file was read. The nightly's local book and
  the live index differ (renames such as `allocator_api` -> `allocator_ext`); both were used.
- Unverified claims carried above: popcount codegen at baseline (E1), `cold_path`/`array_windows` being stable (E0),
  `-Zannotate-moves` visibility in callgrind (N10), ASan link-and-run on this machine (N6), the project's reliance on
  unwinding (N2), `image::ColorType` being non_exhaustive (N19), whether `target/` size is mostly debuginfo (E7).
- 16 entries could not be classified (library: numfmt, temporary_niche_types, sized_type_properties,
  partial_ord_chaining_methods, legacy_receiver_trait, inplace_iteration, slice_concat_ext, slice_index_methods,
  prelude_next, trusted_fused, trusted_random_access, windows_net, ip_multicast_reserved, ipv6_hop_limit,
  io_error_uncategorized; flag: metrics_dir). All are internal or networking/stub pages with no site found.

## Appendix: closed entries by reason (all examined; none has a site in `crates/` or `vendor/` unless noted)

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
hash_map_macro and hash_set_entry (std hash containers are banned), thin_box, unique_rc_arc, arc_is_unique,
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
(stable `std::path::absolute` already used at `crates/cli/src/borders.rs:50`), path_trailing_sep, str_as_str, str_copy_from_str,
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
sliceindex_wrappers, sort_floats (stable `select_nth_unstable_by` already used at `crates/analysis/src/borders.rs:381`), split_array,
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
write_long_types_to_disk. Open candidates among flags are in tables B and C: ub_checks, annotate_moves, print_type_sizes,
move_size_limit, randomize_layout, remark_dir, coverage_options, sanitizer (address), self_profile, time_passes,
llvm_time_trace, dump_mono_stats, macro_stats, share_generics, threads/jobs-frontend, codegen_backend,
cross_crate_inline_threshold, inline_mir/mir_opt_level, embed_metadata, incremental_info, codegen_source_order (WATCH),
stack_protector (WATCH).
