# Replace the command line with a headless driver

Part A of one plan that was split in phase 0. The combined text is in commit `a8efee20`. The other parts are `STORAGE_AND_CLEANUPS_PLAN.md` (Part B) and `ADAPTIVE_READERS_PLAN.md` (Part C).

> **Naming (added when it shipped).** This plan calls the driver `pg-dev`, `crates/dev` and `pg_dev`. It shipped as `pg-driver`, `crates/driver` and `pg_driver`, with the type `Headless`. The Linux check container keeps the name `pg-dev` (`docker/`, `tools/linux.py`).

**Global phase order:** A1, A2 (this document); B3, B4 (`STORAGE_AND_CLEANUPS_PLAN.md`); C5 to C10 (`ADAPTIVE_READERS_PLAN.md`). Phase numbers are global, so references between the documents stay valid.

## Context

The `gallery` command line (`crates/cli`, 1,897 lines, 14 tests) is a second implementation of the pass layer. The pass crates (`pg_import`, `pg_thumbs`, `pg_borders`, `pg_scan`) hold the work once, but the CLI re-implements which passes run, in what order, with what options, over which volumes, and what is done with each pass's stats. Reading both sides found 21 differences. Among them:

- The CLI runs import, hash, pair and borders at `Normal` priority; the app uses `BelowNormal`.
- Each CLI drive group gets its own full memory budget, while the app splits one governed budget.
- `hash` and `borders` run over the whole library in the CLI, but per lane in the app.
- A scan failure ends the CLI command, while the app posts a notice and carries on.
- The CLI never runs `checkpoint_passive`.
- Removal and discard logic is written twice.

The user does not use the CLI. Its real uses are developer tooling:

- `tools/compare_shots.py:333` builds test catalogs with it.
- `tools/profiling/bench_data.py` and `scan_timing.py` run it.
- `docs/reference/IO.md`, `PROFILING.md`, `HEAP_PROFILE.md` and `START_PROFILE.md` describe measurements taken with it.

The replacement is a headless driver over the app's own service. There is then one implementation of the passes.

## Decisions (made by the user)

- **No back-compat.** There are no existing users, so removed flags, settings and fields are removed outright. There are no aliases, deprecation periods or migrations of old preference files or catalogs. This applies to all three parts.
- **Remove the CLI.** Its developer uses move to `pg-dev`, a non-shipped binary over a driver type `Headless`.
- **Delete the eval commands:** `borders` with paths, `--csv`, `--stdin` and `--import-review`, and `dups`. Nothing in the repo uses them or plans to use them. The checks are listed under "Removal".
- **No public tuning flags.** `--readers`, `--workers`, `--buffer-mb` and `--slots` go with the CLI. Counts and memory come from the governor and the controller.
- **Developer pins are typed, not hidden.** `RunnerConfig.pin_readers` and `pin_workers` (both `Option<NonZeroUsize>`) are set by `Headless` and by `pg-dev run --pin-readers N --pin-workers N`. The library reads no environment variable. A pinned run never writes a learned profile.

## Design

### The driver

- **Naming.** `pg_app::Session` already exists (`crates/app/src/reader.rs:213`), so the driver's type is `pg_dev::Headless`. This document says "the driver" for it.
- **Crate.** `crates/dev` is a library, `pg_dev`, plus the `pg-dev` binary. Its tests live in `crates/dev/tests/`. It is not in `pg_app`: a driver used only by tools and tests does not belong in a product crate. The `gallery` test binary cannot link Qt, so the driver can't live in `gallery` or `desktop` either. Add `crates/dev` to `tools/crate-layers.txt` after `app`, `jobs-testkit` and `testkit`.
- **Where its tests go.** All driver tests live in `crates/dev/tests/`, and benches use the driver too. They must not live in `crates/app/tests/`: `dev` depends on `app`, and `tools/boundaries.py` rule 2 forbids a cycle among normal, dev and build edges. An app test that used the driver would fail `just lint`. App tests use `pg_testkit` and `LibraryService` directly.
- **What it does.** It starts `pg_app::LibraryService` on a catalog folder. It steers the service with `add_folder`, `remove_folder`, `discard_removal`, `rescan`, `set_features`, `set_thumb_height` and `status`. It waits until quiescent, with a timeout, and returns typed results: counts, rounds, per-lane times, failures, and once Part C lands, the tuner's decisions. It holds no pass logic. It starts, steers and reads the app's service, which uses `pg_library::Runner`.

### What `pg_app` needs (checked)

- `Library.runner` is private. `Library::start` is `pub(crate)` and takes positional arguments (`crates/app/src/library.rs:104`).
- `App::open(Config)` is public and Qt-free (`crates/app/src/app.rs:80`).
- Add a typed `RunnerConfig` (below), carried by `Config`. Add `LibraryService` pass-throughs: `quiescent`, `set_passes` and `run_until_quiescent`.
- **The writer.** `Library::start` always starts the `Writer` (`library.rs:112`). A headless run on a real catalog would then write queued ratings and turns into the user's photo files. The CLI never did that outside `gallery write`. `RunnerConfig.writer` defaults to `false` for `pg-dev`, and only `pg-dev write` starts one.

### `RunnerConfig` (new)

`Runner::start` takes six positional arguments today (`crates/library/src/runner.rs:43-56`). Replace them with a typed config holding:

- `pin_readers: Option<NonZeroUsize>`
- `pin_workers: Option<NonZeroUsize>`. `docs/reference/HEAP_PROFILE.md:15,17` uses `--workers 1` for deterministic allocation counts, so the pin needs a home.
- `passes: PassMask`
- `writer: bool`

Until Part C lands, `pin_readers` sets `Options.readers` directly, which is still a plain count. From phase C9 it selects `Readers::Fixed(n)` instead.

### Pass filter

`lane.rs:107-115` is a fixed list of seven passes, and `schedule::round` needs no change. The mask is a live cell on `Shared`, beside `HeightCell` and `FeatureCell`. It is set at start and can change between runs. Lanes read it at the start of each round.

`Headless::run(&[Pass::Thumbs, Pass::Placeholders])` then runs those passes in the real round and skips the rest. The driver never calls the pass functions directly. Tests use the filter too.

`--height` is one `HeightCell`. Several heights in one `pg-dev run` are therefore sequential runs, with an idle wait between them.

### `Runner::quiescent()`

`Runner::idle()` cannot serve `until_idle`. It is `all(!lane.inner.is_active())` (`runner.rs:185-187`), and `is_active` is only the round flag (`inner.rs:69`). A lane waiting for its busy-retry timer (`lane.rs:43`) reports idle while a busy scan is still owed. Pending writes and unfinished removals are not covered either.

Add `Runner::quiescent()`. It is true when:

- no lane is in a round;
- no lane has queued or busy scans (`Inner::scans`, `Inner::busy`);
- no writes are left (`left_to_write == 0`);
- no removal markers remain.

Each condition gets a test. `idle()` stays for its current caller.

### `pg-dev`

Commands:

- `run --home H [--create] [--add F...] [--height N]... [--passes ...] [--pin-readers N] [--pin-workers N] [--timeout S] [--perf FILE]`. As built (`pg-driver`): a run always waits until the job is quiet, so the plan's `--until-idle` is not a flag. `--catalog-dir` is `--home`, and a missing home is refused unless `--create` is given.
- `status`
- `remove [--discard-pending]`
- `write`
- `perf FILE`

Exit codes are clear, and the JSON summary goes to stdout. `PG_PERF_LOG` is opened and summarized as the CLI does today (`crates/cli/src/lib.rs:107-139`). The app already opens it at `crates/desktop/src/trace.rs:47`.

Runs default to an isolated home (`Layout::in_home` under a work folder), so a sweep or a test never writes the real profile or catalog.

### What each CLI feature becomes

- `scan --wait`: not needed. A busy scan is requeued by its lane and retried every `BUSY_RETRY` (5 s). `quiescent()` covers the wait.
- `write`: `Headless::write_pending()`. It starts the writer, wakes it, and waits until `left_to_write == 0`, as `wait_for_writes` does now.
- `remove` and `--discard-pending`: the `LibraryService` methods the app already uses. The CLI's own copies (`cli/src/lib.rs:433-491`) are deleted.
- `status`: the typed `Headless::status()`.
- `scan`, `import`, `hash`, `pair`, `thumbs` and `borders`: `pg-dev run` with `--passes`.
- The eval commands: deleted, see Decisions.

### Behaviour the replacement must keep

`compare_shots.py` runs `scan; import; thumbs 120; thumbs 220; borders` in that order. It becomes the same sequence of filtered runs:

- `pg-dev run --passes scan,import`
- `pg-dev run --passes thumbs --height 120`
- `pg-dev run --passes thumbs --height 220`
- `pg-dev run --passes borders`

It does not become one full round, so the catalogs match what it built before. The check is a catalog diff: row counts per table, before and after.

## Tests: where the CLI's 14 go

- **Deleted with the CLI,** because they test the command line itself: `the_command_line_is_told_from_the_app_without_building_clap`, `the_catalog_flag_is_catalog_dir`, and `gallery_scan_prints_its_notes`. The last one's notices are tested where they are made (`scans.rs` `scan_notices`). Both tests in `borders.rs` go with the eval commands.
- **Become driver tests** in `crates/dev/tests/`:
  - `a_refused_folder_between_two_others_does_not_stop_the_scan`
  - `gallery_remove_takes_the_folders_kept_notices`
  - `a_missing_catalog_folder_is_refused_with_the_create_hint_unless_asked`, as `Headless::open` with and without create
  - `gallery_write_returns_when_the_writer_ends` and `the_write_command_closes_its_cache_and_returns`, as `write_pending`, keeping the `Writer::start_with_fault` panic case
- **Deleted with `add_folder_finishing`.** Its only production caller was the CLI (`cli/src/lib.rs:329`). The app calls `folders::add_folder` (`app/src/library.rs:388`). The tests are `gallery_scan_finishes_a_removal_over_the_folder_first`, `..._typed_with_dot_dot`, `..._typed_by_its_short_name`, and the two in `library/tests/writeback.rs:1171,1201`. Before deleting them, confirm that the app's `add_folder` refuses a folder under a pending removal, and keep or add a test of that.
- **`two_roots_scan_import_and_thumbs_agree_with_and_without_serial`** becomes a test that two roots on two lanes give the same catalog as the same roots in one lane. It uses a pass filter on one lane, or `crates/library/tests/lanes.rs`.

## Removal list (phase A2)

**Deleted:**

- `crates/cli`, all of it.
- The CLI branch of `crates/gallery/src/main.rs`: `wants_cli`, `attach_to_parent` and the no-window line.
- `pg_wording::cli` and `cli.ftl`. Check what `words.rs` still needs first.
- Library code described as the CLI's:
  - `lanes::{Group, groups, command_groups, run_each}` (`lanes.rs:73-180`). `plan`, `assign` and `probe_place` stay, because the coordinator uses them.
  - `folders::{add_folder_finishing, Added, Finished, AddFinishError}` (`folders.rs:86`).
- `pg_platform::console` and its `Win32_System_Console` feature (`crates/platform/Cargo.toml:59`).
- The gallery crate's `desktop` feature. `pg-desktop` becomes non-optional. `tools/check_feature_coverage.py` currently covers the feature only through the default features. Remove the `pg-wording` dependency if cargo-machete flags it.
- The `pg-cli` entries in the workspace and in `tools/crate-layers.txt`.
- The `--again` flags. A new `IMPORT_VERSION` or `PAIRING_VERSION` already re-analyses or re-pairs everything (`lifecycle.rs:159, 506`). `Catalog::mark_pending` and `pairs_again` stay, because they have callers beyond the CLI. See Part B.

**Scripts and recipes:**

- `tools/linux.py:430-440` (`gallery()` builds `pg-gallery --no-default-features`, which is the CLI-only binary) and `justfile:227-230` (`linux-gallery`).
- The `run` and `dev` recipes (`justfile:69-75`, documented as `just run scan D:\Photos`) and `crates/README.md:278,313`.
- `tools/test_ci_files.py:461`, `ANYHOW_PERMANENT = {"cli", "desktop"}`. Drop `cli`, and add `dev` if it uses anyhow.
- `tools/profiling/scan_timing.py:110` (a bare `scan` of existing roots) and `bench_data.py:20-22` (`--create`, which `pg-dev` takes).
- `tools/compare_shots.py:187,333`. It builds each side's catalog with that side's own binary. Change it to `-p pg-dev` per side. `--record <sha>` builds an older commit, which has no `pg-dev`, so references recorded before this change cannot be re-recorded.
- `tools/profiling/callgraph/profile.py` and `callgraph.cpp`: check what they run. `lint_qml.py`, `linux.py`, `make_icon.py` and `qmltest.py` matched only the program's name. Check them too.
- `crates/borders/src/lib.rs:3`: the module doc points at `pg_cli::borders`.

**Docs rewritten to `pg-dev`, or removed:**

- `crates/README.md:352-354`: the how-to lines for `dups`, `borders --csv` and `--import-review`.
- `docs/reference/IO.md`, `PROFILING.md`, `HEAP_PROFILE.md` (the recipe at `:17` uses `--readers 1 --workers 1`, which is rewritten), `START_PROFILE.md`, `STORAGE.md`, and any other plan or reference that names `gallery import`, `scan` or `thumbs`.
- `docs/plans/MULTI_DRIVE_LANES_PLAN.md`, lines 14, 297, 299, 337 and 358.
- `docs/plans/DECODE_ONCE_PLAN.md:1125`: the planned `import --no-borders` and `--no-dups` flags are moot.
- `docs/plans/BOUNDED_MEMORY_PLAN.md`: its memory-bound work on these commands is moot. Strike table rows 73-74, section 4.8 (line 1275), the `dups` waits (lines 863, 1368 and 1819), and test rows 1947-1948 and 1966. Note that the removal also removes the 450–850 GB worst case recorded there. If that plan is still meant to be built, say so and this deletion waits.
- `docs/plans/SECURITY_FLOW_REVIEW.md:704, 718, 732`: these findings are triggered by `gallery borders <paths> --csv`. The removal closes them, and their status is updated.

## Phases

1. **Headless foundations.** Add `RunnerConfig` in place of `Runner::start`'s positional arguments and `Library::start`'s. Add the pass filter, `Runner::quiescent()` (one test per condition), the no-writer start and the `LibraryService` pass-throughs. Add `Headless` and `pg-dev` (`run`, `status`, `remove`, `write`, `perf`), with tests in `crates/dev/tests/`. The CLI still exists and is untouched.
2. **Migrate, then delete the CLI.** Update the scripts, recipes and docs listed above first. Then make the deletions. Nothing in the repo runs the CLI afterwards. This phase waits until the `compare_shots` catalog diff passes.

## Verification

- `cargo test -p pg-dev --locked` (Qt-free): driver tests, `quiescent()` tests and pass-filter tests.
- `cargo test -p pg-library --locked` for the runner and `lanes`.
- `just check`, `just lint`, `just modgraph` (no cycle from `crates/dev`), `cargo doc -D warnings`, `cargo machete`, and `tools/test_ci_files.py`.
- Catalog diff: `compare_shots.py` builds from `pg-dev` runs produce the same per-table row counts as the old `gallery` sequence. Record those counts once, from the old binary, before phase A2.
- `pg-driver run` on a scratch library returns a JSON summary once the job is quiet (no `--until-idle`: a run always waits). `write` on a copy leaves the original files unchanged.

## Risks and unverified

- Whether `Runner::quiescent()` covers every lane state that matters. A test per condition is the check.
- Which tools in `tools/` run the CLI beyond `compare_shots.py` and the profiling scripts. The search found `callgraph`, `lint_qml.py`, `linux.py`, `make_icon.py` and `qmltest.py`, which must be checked.
- That `Config` and `App::open` are Qt-free in practice. This is taken from `app.rs:80` and has not been built.
- What `words.rs` still needs from `cli.ftl`.

## Critical files

- `crates/dev/` (new): `pg_dev` (`Headless`) and `pg-dev`.
- `crates/library/src/runner.rs`, `lane.rs`, `lanes.rs`, `folders.rs`.
- `crates/app/src/library.rs` and `app.rs` (`Config`).
- `crates/gallery/src/main.rs`; `crates/cli/` (removed); `tools/compare_shots.py`; `tools/profiling/*.py`; `tools/crate-layers.txt`; `tools/test_ci_files.py`; `justfile`.
