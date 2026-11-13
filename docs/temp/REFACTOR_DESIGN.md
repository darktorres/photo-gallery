# Refactor design: the record, the deviations, and what is open

Status (2026-11-12, HEAD `898b709`): this file is the design record, rewritten from the code. It replaces the 2026-11-06 version, whose Context described the tree as of 2026-11-02. The status source is the ledger in `docs/temp/REFACTOR_PLAN.md` ("Order of work"); where this file and the ledger disagree on status, the ledger wins and the disagreement is listed under "Ledger corrections" below. Commit hashes cited in the ledger are only reachable by object id: history was squashed into one commit, `88e780b` ("the full history squashed into one commit", 2026-11-06), and `5eb81bc` and `c552170` are not ancestors of `main`.

## 1. Where the refactor stands

Done, per the ledger: steps 0 to 11 (scaffolding, spike, the cycle cuts, the platform, model, catalog, thumbnail cache, render, library and CLI, `pg-client`, `pg-app`), the QML split and typed bridge (S-12.2, S-12.3, S-12.5 to S-12.22 except the open items below), the typed-name model (S-17.1 to S-17.7), the text seam (S-19.1 to S-19.3), and the crate splits (S-20.1 to S-20.3).

Open, per the ledger:

| ID | Status | What is left |
|---|---|---|
| S-12 (step 12) | partly fixed | The per-component facades, and the `CurrentItem` spike (ledger, step 12). S-12.1 is partly fixed (the grid's per-row role lists). |
| S-13 | open | S-13.5 (final measurement): the `round_done` startup stage and the rebuild's first-start cost are not measured. S-13.1 has a wrong crate count (see below). |
| S-14 | partly fixed | Verification. `just test` and `just qmltest` run; the screenshot matrix is in place. Coverage gaps are listed in the S-14 row. |
| S-15 | open | CI exists (`.github/workflows/ci.yml`, `windows` and `linux` jobs). `603c5e0` passed; `15ca4e1` failed on 2026-11-08 on a Windows test (`cropped_copies_cut_the_borders_and_leave_the_originals`) and on the Linux `cargo doc` broken links. |
| S-16 | partly fixed | Performance. The callgrind baseline holds 11 results (`docs/perf/baseline.json`); LTO and inlining by measurement (S-16.3) and the checkpoints are the open parts. |
| S-21 | open | The REVIEW rows still listed as open: R-66, R-75, R-76, R-77, R-78, R-80, R-83, R-101, R-112, R-113, R-127. R-75, R-83 and R-101 are "partly fixed" in `REVIEW.md`. Each gets a measured before and after. |

The web client's stack is undecided. `docs/temp/WEB_CLIENT.md` holds the research and the open checks (including the Qt-for-WebAssembly screen check, S-12.4 S14, not run). Nothing in the desktop refactor depends on that decision.

## 2. Corrected baseline

These facts were checked against the tree at HEAD `898b709` on 2026-11-12. They replace the Context of the old file.

- **Crates.** `members = ["crates/*"]` (`Cargo.toml:3`) gives 34 crates. The ledger's S-13.1 row says 24, so it is stale.
- **LTO.** No release LTO is configured. The only explicit `lto = false` is in `[profile.dhat]` (`Cargo.toml:116`), the heap profiler's build.
- **Desktop dependencies.** `crates/desktop/Cargo.toml` depends on seven pg crates at runtime: client, log, model, platform, trace, app, render. It also has `pg-testkit` as a dev-dependency. That is eight in all, not eleven. The ledger's S-13.1 claim that the desktop depends only on Qt and `pg-app` is false.
- **Desktop tests.** The desktop sets `doctest = false` only, with no `test = false`. Its tests run in `just test` (`Justfile`).
- **QML.** `crates/desktop/qml/Main.qml` is 758 lines. It holds no `Timer` and no controller policy. The policy is in Rust: `pg_client::controller` has `deadlines`, `input`, `listing`, `panels`, `replay`, `size` and `start` (`crates/client/src/controller/`). `Shell` is `crates/desktop/src/shell.rs`, a QObject created by id in `Main.qml`.
- **The bridge.** Filter and facet queries are not JSON across the bridge. The bridge uses typed invokables (`Shell::save_view(name)`, `apply_view(name) -> bool`, `delete_view`). JSON is parsed only from the command line, in Rust.
- **`#[qenum]`.** `MediaKind` is the only one (`crates/desktop/src/gallery_bridge.rs:30`).
- **Lossy names.** Fixed. Names are `pg_model::name::Name` (exact bytes; WTF-8 on Windows, raw bytes on Unix). The scanner keeps a lone surrogate (`a_listed_name_is_kept_exactly`, `crates/scan/src/lib.rs`). The catalog stores folder paths and file names as BLOBs (`crates/catalog/src/schema.rs`), and `meta.path_rules` is checked on open.
- **The CLI and native paths.** The import, thumbnail, border and crop readers and the lister take `pg_platform::native::NativePath`, not `Vec<u16>`.
- **Reference keys.** `crates/model/tests/reference_keys.tsv` is committed (2034 rows). Whether its rows were reviewed is not recorded.
- **Screenshots.** `just shots` runs `tools/compare_shots.py` over `tools/shots/` with recorded references. The file `tools/compare-shots.ps1` in the old file does not exist.
- **WSL.** Retired (commit `62b4540`). There is no `just wsl-check` recipe, and `tools/wsl-check.sh` and `tools/profiling/wsl-bench.sh` are gone. Linux checks run in the `pg-dev` container (`just check-linux`) and in CI's `linux` job. Ledger rows S-01.3 and S-17.8 describe WSL and are historical.
- **Fuzzing.** `fuzz/` is outside the workspace and has seven targets (containers, metadata, edit, model keys and query JSON, codec transform).

## 3. The design as built

The decisions from the old file, each with its status.

1. **One controller in `pg-client`, built from sub-machines.** Built. `pg_client::controller` has seven sub-modules (see above). Each is pure, takes `now_ms` as an input, and returns commands. Time is an input, not a `std::time` call. The per-crate `clippy.toml` in `crates/model` and `crates/client` bans `Instant`, `SystemTime`, `thread::spawn` and `fs::*`.
2. **Deadlines.** Built. The core holds one deadline per id (`deadlines.rs`: `ApplyQuery` 300 ms, `Size` 300 ms, `SaveSession` 1 s, `Layout` 50 ms). The desktop drives them with one single-shot `QTimer` in `crates/desktop/cpp/deadline.cpp`.
3. **Data stays with its owner.** Built. `Grid` is a pure `pg-client` struct (`grid.rs`, `layout.rs`), and `GalleryModel` keeps it (`crates/desktop/src/gallery/state.rs`). `Tree` lives in `crates/desktop/src/folders.rs` as a `thread_local!`, not as a field of a model struct. No shared core cell was built.
4. **Where the instance lives.** Built as `Shell` on the GUI thread. `pg-app` keeps persistence only, and there is no `pg-app` to `pg-client` edge. Cross-thread results enter through `Shell::post`.
5. **Listing.** Built as `pg_model::view::Listing` (`folder`, `recursive`, `sort`, `query`), renamed from `Place` in S-12.9. The name `Place` no longer exists in the code.
6. **Commands.** Built in two kinds: model commands run in Rust on the data owner; presentation commands are typed `Shell` signals routed by `Connections` handlers.
7. **Typed boundaries.** Built for facets (`pg_client::facets`, the facet index at the boundary), sort (`pg_client::sort`, `sort_choice`; index 0 is ascending by name, the others are descending), viewer codes (`i32` with a test per code), and URLs (`viewedFullUrl` on `GalleryModel`).
8. **Prefs.** Built. `pg_app::store::JsonStore<T>` (load, `.bad` quarantine, temp-and-rename, `serde(default)`). `gallery.json` is the only prefs file; `desktop.json` is no longer read.
9. **Paths.** Built. See the baseline. The planned `Ext::of_name_utf16` is replaced by `Ext::of_name_bytes`, which returns `None` for non-UTF-8 names.
10. **Facades.** Partly built. A facade exists where it holds logic or owns an object QML calls by id. `ViewsMenu` is a separate file. The remaining facade work is in S-12 (step 12, open).
11. **Localisation.** Built. Fluent (`fluent-bundle` 0.16, `unic-langid`), with `crates/client/locales/en/*.ftl` and the `Words` facade in QML. `en` is the only shipped locale; `--locale=pseudo` runs the pseudo-locale.
12. **Crate splits.** Built (S-20). The pass crates are split out of `pg-library`. The ledger records one deviation in S-20.3.
13. **Verification.** Built with gaps. `just lint` (includes the wasm32 check of `pg-model` and `pg-client`, `cargo deny`, machete, ruff, markdownlint, `lint-qml`), `just test`, `just qmltest`, `just shots`, `just perf`, `just fuzz-smoke`. CI is in place but red at `15ca4e1` (S-15).

## 4. Deviations from the old plan

- The controller's test layout is `crates/client/tests/` (facets, sort and the start traces), not a single `crates/client/tests/controller/` directory.
- `Place` was the name in the session and views code until S-12.9 and is `Listing` now.
- The session's last file is its own `session` key in the desktop's settings, not a field of `Place`.
- `pg_catalog::query` does not exist. `from_json` and `to_json` are in `pg-query` (`crates/query/src/lib.rs:320,326`).
- The web client's verdict is not recorded in this file. See `WEB_CLIENT.md`.

## 5. Ledger corrections (made in `REFACTOR_PLAN.md` on 2026-11-12)

- S-13.1 says "the 24 crates". There are 34 (`members = ["crates/*"]`).
- S-13.1's dependency claim (desktop depends only on Qt and `pg-app`) is false: seven runtime pg crates plus `pg-testkit` for tests.
- S-17.5 is "fixed" in its row but "partly done" in the "Now" paragraph. One of them is wrong; the code has `NativePath` and the scanner test, so "fixed" looks right.
- S-01.3 and S-17.8 describe `just wsl-check` and `tools/wsl-check.sh`, which no longer exist. Mark them as superseded by the Linux container and CI.
- S-12.13 had no row in the ledger. It now has one, "merged into S-12.7b", and its original scope is flagged there as open: `StartGate` takes a `FileId`, not a `Listing`.
- The REVIEW rows the old file listed as open (R-79, R-81, R-96, R-119, R-120) are not in `REVIEW.md`, and the ledger records them as closed in S-12.12, S-12.19 and S-12.21. No action beyond keeping the ledger as the status source.

## 6. Reuse (verified names)

- `pg_model::query` (`Query`, `Sort`, `SortKey`): `crates/model/src/query.rs`.
- `pg_model::view::{Listing, SavedView}`: `crates/model/src/view.rs`.
- `pg_query` (`from_json`, `to_json`): `crates/query/src/lib.rs`.
- `pg_client::{keys, input, grid, layout, facets, sort, tree, controller}`: `crates/client/src/`.
- `Grid::{plan, arrive, release_held, selection_after, placement}` and `layout::neighbor`: `crates/client/src/grid.rs`, `layout.rs`.
- `Tree::{find, reveal}`: `crates/client/src/tree.rs`. (`row_of_path` does not exist.)
- `pg_app::store::JsonStore<T>`: `crates/app/src/store.rs`.
- `pg_app::yandex::{Source, replay}`: the Yandex source seam and the offline replay, `crates/app/src/yandex/`.
- `pg_client::viewport`: `crates/client/src/viewport.rs`. (The desktop's `ViewportRust` is `crates/desktop/src/viewport/state.rs`.)
- `pg_testkit`: `crates/testkit/src/lib.rs`.
- `PG_CATALOG_DIR` and `PG_PREFS`: `crates/app/src/config.rs`, `crates/cli/src/args.rs`.
- `catalog::rebuild`: `crates/catalog/src/rebuild.rs` (`pub(crate)`; there is no CLI command).
- `tools/profiling/synthetic_library.py`, the screenshot harness in `tools/shots/`, `tools/perf_check.py`, `tools/lint_qml.py`, `tools/qmltest.py`.

Not in the code, so not reusable: `command_line::value`, `Tree::row_of_path`, `tools/wsl-check.sh`, `tools/profiling/wsl-bench.sh`, and the key-check pattern cited at `catalog/src/lib.rs:1637-1643` (that file is 54 lines).

## 7. Verification

The state in sections 1 to 6 was checked on 2026-11-12 by reading the code and the ledger, not by running the builds. Four independent reviews (QML and `Main.qml`, Rust names and paths, build and CI, and design, spikes and ledger) each checked their scope against HEAD `898b709`, and the line counts and the squash were spot-checked by hand. Spike results S8 to S13 cannot be re-run from the repository: their commits are unreachable and the spike branch is gone. They are recorded in the ledger.

## 8. Risks (current)

1. **CI is red at `15ca4e1`** (S-15). A Windows test on crop borders and Linux `cargo doc` links. Until a green run, the S-15 claim "CI passes" is not true.
2. **Facades and the `CurrentItem` spike** (S-12, open). The last part of the QML split.
3. **Measurement** (S-13.5, S-16). The LTO and inlining decisions and the `round_done` stage are unmeasured.
4. **Reviews** (S-21). Eleven REVIEW rows remain, each needing a measured before and after.
5. **Web stack.** Undecided. The desktop plan does not depend on it, and the code keeps `pg-model` and `pg-client` wasm32-clean.
