# Review ledger: the crate refactor (phases 0–3)

Every finding from the review of the workspace refactor, with its status, and the files each review pass has read.
Opened 2026-11-02 at HEAD `f2c29d1`. The plan is `PLAN.md` §7 and the refactor plan (`since-we-are-going-parsed-flask.md`).

## How to use this file

- A finding has one ID (`R-nn`) that never changes. When it is fixed, its commit goes in the Status column; a
  finding that is decided not to fix says so and why. Nothing is deleted, so the record of what was found stays.
- Status values: `open` (not fixed), `needs decision` (the fix depends on a call the owner has to make),
  `fixed <sha>`, `wontfix <reason>`, `verified` (checked, no defect).
- Coverage: each file in scope has a checkbox. `[x]` means the whole file was read in this review or an earlier
  pass that is recorded in the Coverage section. `[ ]` means not yet read in full.

## Findings

| ID | Status | Severity | Where | Finding | Next step |
|---|---|---|---|---|---|
| R-01 | open | high | `crates/core/src/metadata.rs:583`, `container.rs:216` | A truncated GIF with an XMP trailer panics `Meta::read` (index out of range). Reproduced with a temporary test (since deleted). `gif_xmp` slices `bytes[range.start + 14..range.end]` without checking the length, and `container.rs:216` computes `packet_end + p + 258` without a bound. Pre-existing code, moved in the refactor. Not yet checked: whether one bad file stops the whole import. | Bounds checks in both places; regression test with the truncated file. |
| R-02 | needs decision | medium | `analyze.rs:188`, `analyze.rs:192`, `browse/read.rs:352` | For a video turned by the catalog, the stored width and height include the turn (`analyze.rs:188`), but the stored `orientation` is the display matrix without it (`analyze.rs:192`). `Entry.orientation` is documented as including the turn, so `stored_size` swaps the wrong way for those videos. Only stills use the stored size today, so nothing shows yet. | Decide: read-side fix (no version bump; recommended) or store the composed orientation (needs an `IMPORT_VERSION` bump, which the no-bump rule forbids). |
| R-03 | open | medium | `container.rs:256`, `rewrite.rs:127–141` (old) | `jpeg_is_progressive` stops at a bad segment length before SOF2/SOS; the old walk continued. The lossless rewriter now behaves differently on damaged JPEGs. Well-formed files give the same result. | Decide which behaviour is right, then match it and add a damaged-JPEG test. |
| R-04 | open | medium | `crates/desktop/qml/YandexPanel.qml` (`centre` Timer), `Main.qml` (`yandexPreviewHook`) | Commit `d4c484b` is described as a pure split but also changes two QML behaviours: `Qt.callLater` becomes a zero-interval `Timer`, and the `--yandex-preview` hook is applied at creation instead of afterwards. Neither change is in the commit message, and neither has been run. | Run the app offscreen with `--find-similar` and `--yandex-preview`, compare with the old behaviour, and say so in the commit or revert. |
| R-05 | open | medium | `codecs.rs:24`, `decode.rs:12` | The one remaining module cycle in `pg-core`: `codecs` imports `Rgb` and `release_over` from `decode`, and `decode` imports `Codecs`. Phase 3's goal was an acyclic core. | Move the pixel buffer types (`Rgb`, `release_over`) to a module both use. No data change. |
| R-06 | open | medium | `decode.rs:41`, `decode.rs:124`, `rewrite.rs:28`, `analyze.rs:188` (`turns % 2`), `display.rs:37` | The quarter-turn swap rule is still hand-written in Rust. The commit `b5235c8` says only the `>= 5` forms were replaced, but the message reads as if all were. SQL and QML copies are not checked yet (see R-22). | Replace with `Orientation::swaps_axes` / `Turns` at each site. |
| R-07 | open | low | `decode.rs:231–240` vs `orientation.rs` | The decoder keeps its own inverse per-pixel match table, and a test ties it to `orientation.rs`'s forward map. The orientation algebra has two maps, not one. | Decide whether the decoder can use the shared inverse. |
| R-08 | open | low | `browse/read.rs:20` | `scope` (a folder as a key range) is still in `browse/read.rs`. The plan's `path` module list names it. | Move to `path.rs`. |
| R-09 | open | judgement | `browse/read.rs` (1,503 lines) | Six jobs in one file: the tree, grid entries, details and border sides, facets, file states, views and session. The phase 3 bullet is met literally. | Split per job if you want the plan's per-job read modules now; otherwise defer to the catalog step. |
| R-10 | open | low | `PLAN.md`, `catalog.rs` tests, `browse/read.rs` | Phase 3 deviations: the plan text for `pdq_dihedral` is stale; the `Day` type is deferred; catalog tests still call `browse` and `conn`. | Update the plan text; move the catalog tests to the API. |
| R-11 | open | low | `crates/testkit/src/lib.rs`, `ingest/tests/crop.rs:19`, `ingest/tests/writeback.rs:21`, `catalog.rs:1386` | Phase 1's testkit has no fixture (the plan said "with the fixture and scratch dirs"). The scratch `scratch()` wrappers remain in two ingest tests. The catalog fixture is still `pub(crate) mod fixture` in `pg-core`. | Move the fixture into testkit (or behind a feature), and point the wrappers at testkit. |
| R-12 | open | low | `tools/wsl-check.sh:2`, `:26` | `wsl-check` excludes `pg-gallery` as well as `pg-ingest`, and the reason is recorded nowhere. The header says "every crate but the desktop", which is inaccurate. | Record the reason in the script; fix the header. |
| R-13 | open | low | `docs/FINDINGS.md` (missing) | Phase 0 baselines exist only in scratch (`D:\PhotoGallery\baseline_2026-11-02`, WSL's `target/gungraun`). The final comparison depends on them. | Copy the numbers into `FINDINGS.md`. |
| R-14 | open | medium | desktop screenshots, phase 2 | The phase 2 screenshot check is not like-for-like: `phase2_filters.png` has a selected first cell and a status bar path, and no baseline frame has that state. So "the split changed nothing visible" is unproven. | Re-run the same hook arguments against the pre-split build. |
| R-15 | needs decision | low | `analyze.rs:16–19` | The `IMPORT_VERSION` doc says to bump before release. That contradicts the standing no-bump-pre-release rule. | Rewrite the doc to the rule, or ask the owner which rule stands. |
| R-16 | open | low | `decode.rs:50,56,77,93`, `maker.rs:99`, `paths.rs:7`; private links `borders.rs:14`, `catalog.rs:721,885`, `decode.rs:76` | Broken rustdoc links from the moves: links to `Decoder::decode`/`decode_stored` (deleted), `ThumbCache::put_batch`, and `PathArena` (moved). Several more link to private items. Nothing gates rustdoc. | Fix the links; add `cargo doc` to lint (warnings as errors). |
| R-17 | open | low | `.cargo/config.toml:1`, `docs/PROFILING.md:137`, `crates/README.md:50`, `eval/jpeg-decoders` comments | Stale references: "Qt for the UI crate"; the bench path `crates/core/benches/ir.rs` (gungraun is now in pg-bench); `crates\ui\qml`; comments naming pg-core's `decode.rs`. | Fix each. |
| R-18 | open | cosmetic | `metadata.rs:26–27`; `video.rs` (`Probe.taken`) | No blank line before the consts in `metadata.rs`. `Probe.taken` is a tuple `(i64, bool)` where the analysis uses `Taken`. | Fix the blank line; make `Probe.taken` a `Taken`. |
| R-19 | open | cosmetic | `browse/writes.rs:12–13`, `browse/read.rs:613–614`, `browse/read.rs:863–864`, `browse/fixtures.rs:9–10` | A doc block describing `scope` sits above `set_rating` in `writes.rs`. A doc paragraph is duplicated in `read.rs`. A `use` appears mid-module in `read.rs`'s tests. A blank line is missing in `fixtures.rs`. | Move and dedupe. |
| R-20 | open | low | `crates/core/src/thumbs.rs` (`pending_in`, ~539) | The `WorkList::with_capacity` hint was dropped; `pending_in` starts from `WorkList::default()`. Probably negligible. The plan requires keeping the allocation profile. | Restore the hint; measure at the end. |
| R-21 | verified | — | `crates/core/src/catalog.rs` (`pdq_hashes`), `pdq_index.rs`, `crates/ingest/src/import.rs`, commit `9ee9f7e` | The PDQ change (all eight orientations stored and queried; `pdq_dihedral` in the baseline schema, `LAYOUT_PARTS` 10) was read in full. The `bail!` guard, the query and the insert are correct. Existing catalogs need `import --again`, which the guard reports. | None. |
| R-22 | open | low | `query.rs`, `writeback.rs:365`, `Viewer.qml` | The swap rule's SQL and QML copies. Not checked yet. | Check during the reads in the Coverage section; add to R-06 if found. |

## Coverage

Scope: every Rust, QML and C++ source under `crates/`, the build scripts and manifests, `Justfile`, `deny.toml`,
`.cargo/config.toml`, the tools the plan changed, and the docs the plan changed (read as hunks, with stale-reference
grep). Out of scope: docs the plan did not change (design notes, measurement logs) and `eval/`, except for the
stale references in R-17. Each file gets one checkbox: `[x]` once it is read in full.

### Read in full, earlier passes (this session)

- [x] `crates/core/src/`: `analysis.rs`, `analyze.rs`, `orientation.rs`, `decode.rs`, `display.rs`, `maker.rs`,
  `format.rs`, `date.rs`, `detection.rs`, `path.rs`, `paths.rs`, `container.rs`, `browse.rs`, `browse/read.rs`,
  `browse/writes.rs`, `browse/fixtures.rs`
- [x] `crates/testkit/src/lib.rs`, `crates/bench/benches/ir/catalog.rs`, `crates/bench/benches/ir/linux.rs`
- [x] `crates/gallery/src/main.rs`, `crates/gallery/build.rs`, `crates/gallery/Cargo.toml`
- [x] `crates/desktop/src/lib.rs`, `crates/desktop/build.rs`
- [x] `Justfile`, `deny.toml`, `crates/README.md`, `tools/wsl-check.sh`, `tools/profiling/synthetic_library.py`
- [x] Commit `9ee9f7e` in full (R-21)

### Read as diffs only (must be read in full)

- [ ] `crates/core/src/catalog.rs` (2,180 lines; only the phase-3 changes and the changed functions read)
- [ ] `crates/core/src/thumbs.rs` (1,404)
- [ ] `crates/core/src/metadata.rs` (1,167)
- [ ] `crates/core/src/codecs.rs` (1,199; only its imports changed)
- [ ] `crates/core/src/query.rs` (997)
- [ ] `crates/core/src/rewrite.rs` (431)
- [ ] `crates/core/src/rotate.rs` (252)
- [ ] `crates/core/src/video.rs` (288)
- [ ] `crates/core/src/borders.rs` (929)
- [ ] `crates/core/src/crop.rs` (181)
- [ ] `crates/core/src/pdq.rs` (404)
- [ ] `crates/core/src/pdq_index.rs` (483)
- [ ] `crates/core/src/yandex.rs` (549)
- [ ] `crates/core/src/layout.rs` (223)
- [ ] `crates/core/src/resize.rs` (94)
- [ ] `crates/core/src/media.rs` (113)
- [ ] `crates/core/src/animation.rs` (88)
- [ ] `crates/core/src/lib.rs` (35), `crates/core/build.rs` (46), `crates/core/Cargo.toml`, `crates/core/examples/stage_times.rs` (57)
- [ ] `crates/ingest/src/`: `pipeline.rs` (535), `writeback.rs` (561), `cli.rs` (372), `runner.rs` (324),
  `scan.rs` (279), `borders.rs` (290), `control.rs` (221), `dups.rs` (143), `thumbs.rs` (144), `import.rs` (124),
  `crop.rs` (157), and the crate's `lib.rs`, `main.rs` if present, `Cargo.toml`
- [ ] `crates/ingest/tests/`: `writeback.rs` (191), `video.rs` (206), `crop.rs` (155)
- [ ] `crates/desktop/src/`: `gallery.rs` (959), `filters.rs` (432), `folders.rs` (371), `state.rs` (408),
  `yandex.rs` (479), `library.rs` (267), `providers.rs` (154), `loader.rs` (165), `settings.rs` (180), `view.rs` (184),
  `animation.rs` (226), `trace.rs` (108), `panic_log.rs` (52), `command_line.rs` (141), `live_reload.rs` (37),
  `gpu_device.rs` (64), `screenshot.rs` (31), `app_icon.rs`, `gui_app.rs`, `Cargo.toml`
- [ ] `crates/desktop/qml/`: `Main.qml` (1,085), `Viewer.qml` (752), `YandexPanel.qml` (402), `FilterBar.qml` (274),
  `BorderOverlay.qml` (82), `WheelListView.qml` (90), `Ratings.qml`
- [ ] `crates/desktop/cpp/`: `app_icon.cpp/.h`, `frame_sink.cpp/.h`, `gpu_device.cpp/.h`, `gui_app.cpp/.h`,
  `live_reload.cpp/.h`, `providers.cpp/.h`, `screenshot.cpp/.h`
- [ ] `crates/bench/Cargo.toml`, `crates/bench/benches/ir.rs`
- [ ] `crates/testkit/Cargo.toml`
- [ ] `tools/lint-qml.ps1`, `tools/make_icon.py`, `tools/profiling/bench_data.py`,
  `tools/profiling/wsl-bench.sh`, `tools/profiling/callgraph/` (callgraph.cpp, profile.ps1)
- [ ] `.cargo/config.toml`, `Cargo.toml` (root; its diff is the members line only), `crates/desktop/Cargo.toml`
- [ ] Changed docs, read as hunks (`PLAN.md`, `PROFILING.md`, `STORAGE.md`, `VIDEO_PLAYER.md`, `WEB_CLIENT.md`,
  `crates/README.md`): done as hunks, plus markdownlint, which passes.
