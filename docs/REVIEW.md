# Review ledger: the crate refactor (phases 0–3)

The review of the workspace refactor (phases 0–3): every finding with its status, and the files each pass has read.

- **Code reviewed:** HEAD `40cceb4`. `Cargo.toml` and `tools/profiling/wsl-bench.sh` show as modified in the working
  tree, but their content matches HEAD (stat-only), so they are reviewed as committed.
- **Last updated:** 2026-11-03. Completed findings are removed (see the Summary).
- **What this review did not do:** the fixes are built and tested as they land, but the whole workspace is not yet
  re-reviewed. Findings come from reading the code, except where a row says "reproduced".

## Summary

| Status | Count |
|---|---|
| `open` | 8 |
| `needs decision` | 0 |
| `wontfix` | 0 |

The completed findings (`fixed` and `verified`) are removed from this file. Each was fixed in the commit its row named
before removal: `git log -p docs/REVIEW.md` shows the rows as they were.

Of the 8 findings that are open or need a decision, by severity: high 0, medium 1, low 4, judgement 3, cosmetic 0.
No row needs a decision now: the agreed recommendations are in each row's Next step column. R-66 is open until it is
measured. R-06 and R-22 are open because their QML copies of the swap rule wait for the viewport facade; their Rust
sites are fixed.

## How to read this file

- **IDs** (`R-nn`) never change and are never reused. R-36 and R-61 were never assigned.
- **Status:** `open` (not fixed); `needs decision` (the fix waits on a call the owner has to make); `fixed <sha>`;
  `wontfix <reason>`; `verified` (checked, no defect).
- **Severity:** `high`, `medium`, `low` by impact. `judgement`: the answer depends on intent, not only on the code.
  `cosmetic`: no behaviour changes.
- **Coverage:** each file in scope has a checkbox in the Coverage section. `[x]` means read in full.

## Findings

### High

No open findings in this severity.

### Medium

| ID | Status | Where | Finding | Next step |
|---|---|---|---|---|
| R-06 | open (Rust sites: 53f480c; QML: S-10.4) | `decode.rs:41`, `decode.rs:124`, `rewrite.rs:28`, `analyze.rs:188` (`turns % 2`), `display.rs:37`, `crates/desktop/src/gallery.rs:710` (`turns % 2 == 1`), `crates/desktop/qml/Viewer.qml:404–405` (`playerTurns % 2`) | The quarter-turn swap rule is still written by hand in Rust and QML. Commit `b5235c8` says only the `>= 5` forms were replaced, but its message reads as if all were. The SQL copies are in R-22. | Done for the Rust sites: `orientation::swaps_axes` for an orientation, `orientation::turns_swap` for the catalog's turns, with a test tying them together. The QML copy moves to the viewport facade (S-10.4). |

### Low

| ID | Status | Where | Finding | Next step |
|---|---|---|---|---|
| R-10 | fixed 713cd7e | `PLAN.md`, `catalog.rs` tests, `browse/read.rs` | Phase 3 deviations: the plan text for `pdq_dihedral` is stale; the `Day` type is deferred; catalog tests still call `browse` and `conn`. | PLAN.md checked: its text matches the code (the dihedral hashes are stored per file, in `analysis.pdq_dihedral`), and `Day` is in pg-model (22e7a05). Done (713cd7e, S-08.6): the catalog's own tests no longer touch `conn`. The SQL they need is in the test-only `fixture::raw` (one named helper per statement); the layout tests open the file with `raw::on_disk`, since the file on disk is what they check. The cross-crate tests keep `fixture::connection` (S-08.6, moves with S-09). |
| R-11 | fixed 2122a8f, 5999ff8 | `crates/testkit/src/lib.rs`, `ingest/tests/crop.rs:19`, `ingest/tests/writeback.rs:21`, `catalog.rs:1386`, `ingest/src/pipeline.rs:474,514` | Phase 1's testkit has no fixture, though the plan said it would. Two ingest tests keep their own `scratch()` wrappers. `pipeline.rs` uses `std::env::temp_dir()` with the process id, not testkit's scratch. The catalog fixture is still `pub(crate) mod fixture` in `pg-core`. | Keep the catalog fixture in core behind a `fixture` feature, re-exported by testkit (the plan's design). Move the scratch wrappers and `pipeline.rs`'s `temp_dir` to testkit's scratch now. Done (2122a8f, 5999ff8); `crates/desktop/src/yandex.rs:494`'s `temp_dir` moves with S-11.6. |
| R-20 | fixed 828dbf0 (measured at S-13.5) | `crates/core/src/thumbs.rs` (`pending_in`, ~539) | The `WorkList::with_capacity` hint was dropped, so `pending_in` starts from `WorkList::default()`. Probably negligible, but the plan requires keeping the allocation profile. | Done: `pending_in` sizes the folder table with `WorkList::with_capacity(0, max_folder_id)` (828dbf0). The item count stays 0 on purpose, since counting first scans the view. Measure at the end (S-13.5). |
| R-22 | open (Rust and SQL sites: cd141eb, 22e7a05; QML: S-10.4) | `query.rs:752–753` (confirmed), `writeback.rs:365` (confirmed: `?2 % 2 = 1`), `Viewer.qml:404–405` (confirmed: `playerTurns % 2`) | The swap rule's SQL and QML copies. The border-sort SQL writes `a.orientation >= 5` by hand, the write-back's `done` swaps width and height with `?2 % 2 = 1`, and the viewer's fitted player swaps with `playerTurns % 2`. The sort's axis choice agrees with `browse::read::shown_cut`, because `Info.orientation` already includes the catalog rotation. | Done for the SQL: `swapping_orientations!` generates the list, and a test ties it to the rule. The QML copy goes with the viewport facade (S-10.4). |

### Judgement

| ID | Status | Where | Finding | Next step |
|---|---|---|---|---|
| R-59 | open (part: a8ad458) | `crates/desktop/src/panic_log.rs:17–30`, `crates/desktop/src/loader.rs:88` | Every panic shows a modal message box and appends a backtrace. The loader catches panics in catalog reads (`catch_unwind`), and the hook runs first, so one failing query shows a dialog. The job thread's panics do the same. | No. Only a panic that ends the process shows it. Caught panics (the loader's `catch_unwind`, worker threads) go to the log and the status line. |
| R-63 | open (comments added: e361cba; roles at the QML step) | `crates/desktop/qml/Main.qml:489, 534, 679–691, 814–833`; `FilterBar.qml:64, 119, 148, 222`; `YandexPanel.qml:51, 71–73, 85, 266, 331, 382` | The `revision >= 0` and `count >= 0` guards are true for any loaded model, so they read as no-ops. Their job is to make each binding depend on `revision`, so it re-evaluates after a change, and nothing says so. The cell bindings (`Main.qml:679–691`) repeat the guard per cell. | Replace them with the model roles of the facades (plan step 12): a cell reads its own values, and the role carries the change signal. That is the proper end state, not a workaround. The guards stay only until the roles exist, because the QML step comes later; keep one comment on them that says so. |
| R-66 | open | `crates/desktop/qml/Main.qml:815` (`keysAround(viewer.index, 4)`), `crates/desktop/qml/Viewer.qml:517–525` (neighbour preloads) | The neighbour preloads request `fitSource` images of the neighbours with a 4× margin. Those are near-full decodes on the same two full-image provider threads (`providers.cpp`), so they can delay the shown image. Not measured. | Measure first, as the plan requires. Keep them only if they make the shown image appear sooner. If they stay, size them to the image the viewer shows, not a 4× margin (no measurement justifies the 4×), and cancel them when the index moves. |

### Cosmetic

No open findings in this severity.

## Coverage

Scope: every Rust, QML and C++ source under `crates/`, the build scripts and manifests, `Justfile`, `deny.toml`,
`.cargo/config.toml`, the tools the plan changed, and the docs the plan changed. Out of scope: docs the plan did not
change (design notes, measurement logs) and `eval/` (only its stale references were in scope). `[x]` = read in full.

| Area | Files | Read | Findings |
|---|---|---|---|
| `crates/core/src` | `analysis.rs`, `analyze.rs`, `orientation.rs`, `decode.rs`, `display.rs`, `maker.rs`, `format.rs`, `date.rs`, `detection.rs`, `path.rs`, `paths.rs`, `container.rs`, `browse.rs`, `browse/read.rs`, `browse/writes.rs`, `browse/fixtures.rs` | [x] | R-10 |
| `crates/core/src` (large) | `catalog.rs` (2,180 lines), `thumbs.rs` (1,404), `metadata.rs` (1,167), `codecs.rs` (1,199), `query.rs` (997), `borders.rs` (929), `yandex.rs` (549), `pdq.rs` (404), `pdq_index.rs` (483), `rewrite.rs` (431), `crop.rs` (181), `rotate.rs` (252), `video.rs` (288), `layout.rs` (223), `resize.rs` (94), `media.rs` (113), `animation.rs` (88), `lib.rs` (35) | [x] | R-06, R-20, R-22 (no findings in `crop.rs`, `pdq_index.rs`, `layout.rs`, `resize.rs`, `media.rs`) |
| `crates/core` build and examples | `build.rs` (46), `Cargo.toml`, `examples/stage_times.rs` (57) | [x] | None open (the example's `unwrap`/`expect` on arguments are fine for an example) |
| `crates/ingest/src` | `pipeline.rs` (535), `writeback.rs` (561), `cli.rs` (372), `runner.rs` (324), `scan.rs` (279), `borders.rs` (290), `control.rs` (221), `dups.rs` (143), `thumbs.rs` (144), `import.rs` (124), `crop.rs` (157), `lib.rs` (14), `Cargo.toml` | [x] | R-11 (`import.rs`: the batch's index insert after the store check is correct) |
| `crates/ingest/tests` | `writeback.rs` (191), `video.rs` (206), `crop.rs` (155) | [x] | R-11 |
| `crates/desktop/src` | `gallery.rs`, `filters.rs`, `folders.rs`, `state.rs`, `yandex.rs`, `library.rs`, `providers.rs`, `loader.rs`, `settings.rs`, `view.rs`, `animation.rs`, `trace.rs`, `panic_log.rs`, `command_line.rs`, `live_reload.rs`, `gpu_device.rs`, `screenshot.rs`, `app_icon.rs`, `gui_app.rs`, `lib.rs`, `build.rs`, `Cargo.toml` | [x] | R-59 |
| `crates/desktop/qml` | `Main.qml` (1,085), `Viewer.qml` (752), `YandexPanel.qml` (402), `FilterBar.qml` (274), `BorderOverlay.qml` (82), `WheelListView.qml` (90), `Ratings.qml` | [x] | R-06, R-22, R-63, R-66 |
| `crates/desktop/cpp` | `app_icon`, `frame_sink`, `gpu_device`, `gui_app`, `live_reload`, `providers`, `screenshot` (`.cpp` and `.h`) | [x] | None. Checked: LIFO priority, the `to_qimage` ownership transfer, the link's cancel flag, the GPU device's `qputenv` only when unset, the `SingleShot` disconnect, the debug-only live reload, and the screenshot's `ready` poll. |
| `crates/gallery` | `src/main.rs`, `build.rs`, `Cargo.toml` | [x] | None |
| `crates/testkit`, `crates/bench` | `testkit/src/lib.rs`, `testkit/Cargo.toml` (no dependencies), `bench/benches/ir.rs`, `bench/benches/ir/catalog.rs`, `bench/benches/ir/linux.rs`, `bench/Cargo.toml` | [x] | None open |
| `tools` | `wsl-check.sh`, `synthetic_library.py`, `lint-qml.ps1`, `make_icon.py`, `profiling/bench_data.py`, `profiling/wsl-bench.sh`, `profiling/callgraph/` (`callgraph.cpp`, `build.sh`, `profile.ps1`, `symbolize.py`) | [x] | None open (the other tools: no finding) |
| Workspace | `Justfile`, `deny.toml`, `.cargo/config.toml` (its machine paths are the documented setup), root `Cargo.toml` (members line only) | [x] | None open |
| Docs | `crates/README.md`, `PLAN.md`, `PROFILING.md`, `STORAGE.md`, `VIDEO_PLAYER.md`, `WEB_CLIENT.md`: changed hunks | [x] | R-10. markdownlint passes. |
| Commit `9ee9f7e` | Read in full | [x] | None open |
