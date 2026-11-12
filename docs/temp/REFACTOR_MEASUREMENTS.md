# Refactor measurements

Measured results moved here from the former `FINDINGS.md` log. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [TEST_SET.md](TEST_SET.md); the old titles are listed in [FINDINGS.md](FINDINGS.md).

## Phase 0 baseline (2026-11-02, before the refactor)

The numbers the refactor's before-and-after comparison uses (plan step 0). The raw files are in
`D:\PhotoGallery\baseline_2026-11-02`, and the gungraun logs are in WSL under `~/photo-gallery/target/gungraun`. The
files don't record the commit they were measured at, so read them as a reference for that day.

### Instruction counts (gungraun, callgrind `Ir`, WSL, 16:10)

| Bench | Ir |
|---|---|
| `catalog::tree.all` | 3,275,602 |
| `catalog::list.folder_flat` | 735,865 |
| `catalog::list.folder_recursive` | 33,928,736 |
| `catalog::list.all_name` | 32,015,927 |
| `catalog::list.all_taken` | 46,406,218 |
| `catalog::list.all_unrated` | 32,241,343 |
| `catalog::facets.all` | 93,581,775 |
| `catalog::pending_import.empty` | 115,827 |
| `catalog::pending_thumbs.at_height` | 29,447,015 |
| `per_file::analyze_jpeg.sample` | 60,603,752 |
| `per_file::thumb_jpeg.h256` | 52,045,318 |

The `pg-core` folder holds the 2026-10-31 runs of the two per-file benches, under the crate's old name:
`analyze_jpeg.sample` 63,523,272 and `thumb_jpeg.h256` 50,677,681.

### Startup stages (`D:\PhotoGallery\baseline_2026-11-02\startup_runs.json` and `synthetic_startup_runs.json`)

Milliseconds from process start, median of 5 runs (each stage's min and max are in the JSON).

| Stage | Real library (189,518 files) | Synthetic library (4,000 files) |
|---|---|---|
| `main` | 27.2 | 21.5 |
| `init_done` | 60.2 | 27.0 |
| `qml_load` | 82.9 | 45.7 |
| `qml_loaded` | 363.1 | 230.6 |
| `tree_shown` | 454.6 | 322.8 |
| `list_shown` | 1001.4 | 346.9 |
| `first_frame` | 478.6 | 349.7 |
| `ready` | 1144.6 | 473.7 |
| `round_done` | 3730.6 | 3303.9 |

### Import and thumbnails (`import.log` and `synthetic_build.log`)

- Pinterest sample (`D:\PhotoGallery\datasets\pinterest`, 189,518 files in 693 folders): the scan took 6.3 s (about
  30,000 files/s). The import ran at 432–445 files/s (58–63 MB/s) over its first 6,554 files; the log ends there, with
  "6500 analyzed", so the run was stopped.
- Synthetic library (`D:\PhotoGallery\synthetic\library`, 4,000 files): the scan took 0.2 s. The import took 9.6 s
  (417 files/s, 148–158 MB/s, 1,590 PDQ pairs, 0 failed). The 256 px thumbnails took 11.4 s (351 files/s, 124–132 MB/s).

### Test-hook screenshots

The phase 0 hook screenshots are in `D:\PhotoGallery\baseline_2026-11-02\shots`: `start_0` to `start_5`, `library`,
`filters`, the `hooks_*` set (keys, zoom, similar, crops) and the `symmetric_*` set.

## Refactor S-02.3: deploy, icon and live reload (2026-11-03)

The S-02 verify items that the repo did not record.

- **`just deploy`** (release build, then `windeployqt`) at 713cd7e: exit 0 (`deploy5.log`). Two warnings, neither an
  error: no `dxcompiler.dll` or `dxil.dll` (needed only for Direct3D 12), and `VCINSTALLDIR` is not set (no Visual
  Studio installation directory). The same two warnings are in the earlier log `deploy4.log` (13:05).
- **Icon:** the icon resource of `target/release/gallery.exe` (its `winres` build step) shows the app icon at 32 × 32,
  both in the extraction of the 05:24 build (`exe_icon.png`) and of the 13:05 release build (`exe_icon_recheck.png`).
- **Live reload:** on the debug build, with the scratch catalog and prefs. The control run, with no edit, showed
  "No folders in the library yet." (`live_control.png`). The test run changed the text of `Main.qml` 9 s after start
  to "LIVE RELOAD CHECK.", and the window showed the new text (`live_changed.png`). The script reverts `Main.qml` with
  `git checkout` on exit; `Main.qml` is clean.
- **`lint-qml`** runs in `just lint`, and passed at 713cd7e: "QML: no warnings; all 735 bindings and functions
  compiled to C++ (7 files)".
- **Screenshots:** the grid, viewer and filters screenshots match the pre-split build (R-14, 0ac379e). That check was
  recorded at 0ac379e and was not run again today.

## Refactor S-16.1: the counts against S-00, measured again (2026-11-04)

The Phase 0 counts (2026-11-02) do not reproduce on this machine now, so the comparison uses S-00 re-measured today.
The S-00 commit `49d8f3b` was built in a clone in WSL (`~/pg-s00`) and run with the same bench data (`bench-data`, the
layout `v1-orderkey-synthetic`, copied from the same `synthetic_catalog`: schema and row counts match) and the same
toolchain (rustc 1.99.1, valgrind 3.26.0). HEAD's counts are `docs/perf/baseline.json` (a22fbad).

| Bench | S-00, Nov 2 | S-00, today | HEAD (baseline.json) | HEAD vs S-00 today |
|---|---|---|---|---|
| `per_file::analyze_jpeg.sample` | 60,603,752 | 60,603,835 | 60,604,034 | +0.0% |
| `per_file::thumb_jpeg.h256` | 52,045,318 | 52,045,531 | 52,045,748 | +0.0% |
| `catalog::list.all_name` | 32,015,927 | 32,988,949 | 32,349,515 | −1.9% |
| `catalog::list.all_taken` | 46,406,218 | 47,414,681 | 46,738,460 | −1.4% |
| `catalog::list.all_unrated` | 32,241,343 | 33,214,459 | 32,574,673 | −1.9% |
| `catalog::list.folder_recursive` | 33,928,736 | 34,902,511 | 34,261,837 | −1.8% |
| `catalog::list.folder_flat` | 735,865 | 746,553 | 730,480 | −2.2% |
| `catalog::tree.all` | 3,275,602 | 3,221,409 | 3,137,101 | −2.6% |
| `catalog::facets.all` | 93,581,775 | 105,208,622 | 104,873,332 | −0.3% |
| `catalog::pending_import.empty` | 115,827 | 115,859 | 115,391 | −0.4% |
| `catalog::pending_thumbs.at_height` | 29,447,015 | 46,288,886 | 46,328,133 | +0.1% |

- **The Nov 2 numbers are not the S-00 code's numbers on this setup.** Today's S-00 build is 12% above its Nov 2
  `facets.all` and 57% above its Nov 2 `pending_thumbs.at_height`; the cause of that gap is not established (the
  toolchain or the WSL setup changed since Nov 2; the bench data is the same synthetic catalog). The same-day S-00
  re-measure is the reference from here on.
- **Against that reference, five of eleven are within ±1%:** the two per-file benches, `facets.all`,
  `pending_import.empty` and `pending_thumbs.at_height` (within 0.4%). The other six, the five `list` benches and
  `tree.all`, are 1.4–2.6% *lower* at HEAD. That is fewer instructions, not a regression, but it is outside the ±1% band and the cause is not measured.
  The bench bodies changed for those (the typed `RatingFilter::Unrated`, `Catalog` in place of `&Connection`, the
  `Entry` and `FolderTree` types from `pg_model`), which is the likely source. Left as is; `just perf` gates HEAD
  against `baseline.json`, so a further drop would fail it as a change to re-baseline on purpose.
- **Not in this entry:** the Pinterest import throughput and the startup stages (S-13.5) are long timed runs against
  `D:\PhotoGallery\baseline_2026-11-02`; both are still open.

## Refactor S-13.5: import and startup against S-00, measured again (2026-11-04)

Same machine, same day. S-00 (`49d8f3b`) was exported with `git archive` to `D:\PhotoGallery\s00-src` and built in
release (`D:\PhotoGallery\s00-target`, 7 min). Its Qt and FFmpeg DLLs were copied from HEAD's deployed folder, since the
S-00 build had not been deployed (without them it exits with `STATUS_DLL_NOT_FOUND`). HEAD is the working tree's
release build (`target\release`, rebuilt 21:51); its uncommitted changes are tests, the CI file and an assert that only
runs when a tool is missing. The harness was a local script, not in the repo; the copies are under
`D:\PhotoGallery\s13`.

**Import throughput** (Pinterest folder, 189,519 files; `scan`, then `import` from an empty catalog, stopped after about
6,800 files; two rounds each, the rate at the last progress line):

| Build | Round 1 | Round 2 |
|---|---|---|
| S-00 | 460 files/s (6,899 files) | 447 files/s (6,767 files) |
| HEAD | 450 files/s (6,764 files) | 447 files/s (6,710 files) |

Within run-to-run noise (about 2%). No failures in any run.

**Startup stages** (`PG_TRACE=1`, offscreen, ms since process start, median of five runs; the Pinterest catalog copied
and read into the OS cache before each start):

| Stage | S-00 (copy of the S-00 catalog) | HEAD (catalog already migrated, see below) |
|---|---|---|
| `main` | 29 | 24 |
| `init_done` | 183 | 40 |
| `qml_loaded` | 296 | 262 |
| `tree_shown` | 339 | 329 |
| `list_shown` | 773 | 704 |
| `first_frame` | 360 | 347 |
| `ready` | 869 | 807 |

`round_done` did not print within 15 s of `ready` on either build, so it is not compared. Two S-00 runs had a
`list_shown` 5–7 s after `tree_shown` that no HEAD run had; the cause is not known.

- **The first start of HEAD on a catalog written by S-00 takes about 12 s more.** `init_done` was 11.5–12.7 s on five
  fresh copies of that catalog, and `ready` 12.4–13.8 s. HEAD migrates the catalog in place on the first open: the
  definitions of `files`, `analysis`, `roots` and four of their triggers change, eight stamp triggers are added (on
  `ratings`, `file_writes` and `border_marks`), and the change counter `seq` goes from 379,037 to 900,261.
  A second start on the migrated copy is steady: `init_done` 40 ms, `ready` 807 ms (median of five).
  The project's rule is "no schema bumps before release" and catalogs are rebuilt, so this in-place migration is a
  choice to make, not a regression in the stages. Not changed here.

**Not run:** the S-00 Nov 2 `round_done` (3.4 s from process start) did not reproduce, so the background round is not
compared (see the S-16.1 entry above for the same-day reference).

## Refactor S-16.3: LTO and codegen units, by instruction count (2026-11-04)

The callgrind benches at HEAD (`341e82f`, WSL, `tools/profiling/wsl-bench.sh`'s steps), under two release-profile
variants set by Cargo's environment (`CARGO_PROFILE_RELEASE_LTO`, `CARGO_PROFILE_RELEASE_CODEGEN_UNITS`), so the tree's
`Cargo.toml` is unchanged. The default run reproduces `docs/perf/baseline.json` exactly.

| Bench | default (= baseline) | `lto = "thin"` | `codegen-units = 1` |
|---|---|---|---|
| `per_file::analyze_jpeg.sample` | 60,604,034 | −0.01% | −0.48% |
| `per_file::thumb_jpeg.h256` | 52,045,748 | −0.00% | −0.73% |
| `list.all_name` | 32,349,515 | −0.00% | −0.44% |
| `list.all_taken` | 46,738,460 | −0.00% | −0.24% |
| `list.all_unrated` | 32,574,673 | −0.00% | −0.43% |
| `list.folder_recursive` | 34,261,837 | −0.00% | −0.41% |
| `list.folder_flat` | 730,480 | −0.28% | −0.20% |
| `tree.all` | 3,137,101 | −4.05% | +1.29% |
| `facets.all` | 104,873,332 | −0.00% | −0.00% |
| `pending_import.empty` | 115,391 | −0.16% | −0.02% |
| `pending_thumbs.at_height` | 46,328,133 | −0.00% | −0.00% |

- **Not adopted.** Neither variant moves the hot paths by more than about 0.7%, and the 4% on `tree.all` is about
  127,000 instructions, once per start. The benches are dominated by SQLite, libjpeg-turbo and FFmpeg, which are C code
  that Rust LTO does not reach; the cross-crate calls are not where these counts go. The longer build (thin LTO adds a
  link pass over 24 crates) buys nothing measurable here.
- **Not measured:** wall-clock time and the startup stages for either variant (both would need a Windows release build
  each), and the build time with each. The `#[inline]` audit of cross-crate calls is not done: the counts do not point at
  them.
- **Limit:** instruction counts, not time. A variant that reduced cache misses without changing the instruction count
  would not show here.
