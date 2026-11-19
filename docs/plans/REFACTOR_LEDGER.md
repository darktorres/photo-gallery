# Refactor ledger: what is still open

This ledger lists what the refactor plan asked for and the code does not yet do, or does with a different shape. The
plan itself, `docs/temp/REFACTOR_PLAN.md`, was deleted in the commit that added this ledger; its last version is in git
history. It was checked on 2026-11-14 at HEAD `9f94f8a` by reading the code, one reviewer per part of the plan. Nothing
was built or run. Each row was checked against the working tree, not against the plan's own statuses. Rows whose work
is done are removed. Rows whose premise is gone are removed too.

A row stays while any part of its proper fix is missing. A row with `decide` needs a choice before work starts: either
build what the plan says, or change the plan. The plan's IDs (`S-nn`, `M-nn`, `R-nn`) are kept where a row comes from
the plan. Rows numbered `F-nn` are new, from the check.

## Summary

| Section | Rows | Notes |
|---|---:|---|
| Catalog, thumbnails and jobs | 16 | Most open items are here |
| Errors, dependencies and CI | 5 | The `anyhow` rule and the missing WASM check |
| Platform, codec, metadata, analysis and edit | 12 | Helpers and functions in the wrong crate |
| Tests | 6 | Tests in the wrong crate, and gaps |
| Model | 6 | Keys, names and placement |
| Client: grid and tree | 3 | The grid update protocol |
| Desktop QML | 7 | Controller logic still in QML |
| Order of work (S-12 to S-21) | 5 | Checks and review rows still open |
| Deferred by the plan | 4 | Work the plan puts later |
| Stale tool docs | 1 | A docstring and a row that name old files |
| Total | 65 | |

Kind legend: `missing` is a fix that is absent; `shortcut` is a fix that is present but is not the proper one; `decide`
is a choice between building the plan and changing the plan; `verify` is a row this check could not settle by reading;
`untested` is a behaviour without the test that would catch it; `stale-doc` is a document that says something false;
`deferred` is work the plan puts in a later step.

## Catalog, thumbnails and jobs

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| S-06.1 | decide | `crates/testkit/Cargo.toml`, 15 files that use `pg_catalog::fixture` | The plan says testkit re-exports the catalog fixture. It does not: testkit has no catalog dependency, and tests depend on `pg-catalog` with `features = ["fixture"]` | Keep the direct dependency and correct the row, or add the re-export. Either way the row is marked fixed and is not | decide |
| S-08.1 | verify | `crates/catalog/src/queue.rs`, `abandon_temp` | The row names `abandon_temp`. It has no hit in the catalog crate. The write queue is `queue.rs`, not `writes` | Find where the behaviour went. If nothing covers it, add it; if it is not needed, correct the row | verify |
| S-08.8 | decide | `crates/catalog/src/schema.rs:571` and its triggers | `changes_since` is gone (M-48), but the `files.changed` column and its triggers remain. Nothing reads them, and the schema comment still says "a client's resync" | Drop the column and triggers in a migration, or keep them and correct the comment and the row | decide |
| F-01 | missing | `crates/catalog/src/core.rs:177`, test at `:354` | The `Cancel` token is not built. `cancel_when` still takes `Arc<AtomicBool>` | Add a `Cancel` type and replace `cancel_when` with it | open |
| F-02 | missing | `crates/thumbcache/src/reader.rs:23-30`, `crates/catalog/src/work.rs` | The thumbnail cache ATTACHes the catalog and reads `thumb_clock`, `thumb_folders` and `thumb_sources` itself (`reader.rs:28,30,43,76`). `catalog/src/work.rs` has only `pending_import`, `pending_borders` and `pdq_hashes`. The planned `catalog::read::work` does not exist | Add the `read::work` API with the `thumb_sources` read; thumbcache calls it and does not ATTACH | open |
| F-03 | shortcut | `crates/thumbcache/src/reader.rs:90-93` | Reader connections are a `thread_local!` (`READERS`). The plan asks for a small pool the cache owns | Replace the thread-local with the owned pool | open |
| F-04 | missing | `crates/thumbs/src/lib.rs:132-161` | The thumbnail pass only folds `first_failed`. It does not check `file_state` before keeping a thumbnail. The display path does this in `crates/app/src/images.rs:141` | Check `file_state` before keeping the thumbnail, the same rule the display path uses | open |
| F-05 | untested | `crates/catalog` tests | The `thumb_sources` view is not tested against a TEMP stub `entries` table, as the plan asks | Add the test | open |
| F-06 | shortcut | `crates/exiftool/src/lib.rs:29-31`, callers `crates/writeback/src/lib.rs:521,542` and `crates/crop/src/lib.rs:205` | `pg-exiftool` reads `PG_EXIFTOOL` inside the crate. The plan says the caller gives the program. The row S-05.8 names `exiftool_program` in pg-library, which no longer exists | Take the program as a parameter from the callers, and correct S-05.8 | decide |
| F-07 | missing | `crates/writeback/src/lib.rs:422,432` | `Offline` is a local bool, not a typed error | Make it a typed error variant | open |
| F-08 | missing | `crates/writeback` | `wait_idle()` does not exist | Add it, or remove it from the plan | decide |
| F-09 | verify | `crates/scan/src/lib.rs:374`, `crates/thumbcache/src/writer.rs` (`fn flush`), `crates/app/src/inbox.rs:250` | The plan says three copies of the batch-and-flush sink should become one. Three hand-written sinks remain. `crates/jobs/src/pipeline.rs:240` has a `batched()` that may be the shared one | Check whether the three share `batched()`. If not, route them through it | verify |
| F-10 | decide | `crates/jobs/src/schedule.rs:17-23`, `crates/library/src/runner.rs:296` | The plan's `Scheduler` type with registration is not built. The code has `Pass` and `round(&mut [&mut dyn Pass])` | Build the `Scheduler` type, or correct the plan to the `Pass` design | decide |
| F-11 | decide | `crates/app/src/reader.rs:196`, `crates/app/src/library.rs:159` | The plan says `Session` holds an event subscription. `Session` has a fixed id of 1, and the subscription is `Library::changes()` | Decide which owns the subscription, and correct the plan | decide |
| F-12 | shortcut | `crates/library/src/layout.rs:30-38`, `crates/exiftool/src/lib.rs:31`, `crates/desktop/src/command_line.rs:198`, `crates/desktop/src/controller.rs:57`, `crates/desktop/src/trace.rs:25`, `crates/desktop/src/lib.rs:153`, `crates/gallery/src/main.rs:24` | The plan's `Config` is meant to resolve every `PG_*` variable in one place. They are read in seven places across five crates | Add the `Config` in pg-app, and have every crate take its value from it | open |
| F-13 | shortcut | `crates/library/tests/writeback.rs:353,370,453,805` | The writeback tests compare against a literal `5` for the attempt cap. `MAX_ATTEMPTS` is defined once, at `crates/catalog/src/queue.rs:30` | Use the constant in the tests | open |

## Errors, dependencies and CI

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| F-14 | decide | `crates/*/Cargo.toml` (20 crates list `anyhow`), `Justfile`, `deny.toml` | The plan's rule is that `anyhow` lives only in cli, desktop and gallery. It is in 20 crates, and no lint or deny rule enforces the rule | Relax the rule to what the code does ("no `anyhow` in a library's public API"), or enforce it | decide |
| F-15 | missing | `crates/exiftool/src/lib.rs:21`, `crates/writeback/src/lib.rs:86` | Typed errors are missing: exiftool has no error enum, and writeback's `WriteError` has only `Failed` and `CopyIncomplete` | Add a typed error to each, with `Offline` (see F-07) | open |
| F-16 | missing | `deny.toml` | The planned bans are missing. `windows-sys` is not confined to pg-platform. `cxx` is not confined to pg-desktop; only `cxx-qt` and `cxx-qt-lib` are banned. `image` lists pg-library as a wrapper (`deny.toml:56`), and the plan bans it from codec and testkit. `ffmpeg-next` is not used anywhere, so that ban is moot | Add the two bans, and make the `image` wrapper list match the plan, or correct the plan | open |
| F-17 | missing | `Justfile`, `.github/workflows/ci.yml:280` | There is no WASM check in `just lint` or CI. `ci.yml:280` mentions one in a comment only | Add a `wasm32` check of pg-model and pg-client to `just lint` and to CI | open |
| F-18 | shortcut | `crates/library/Cargo.toml:36`, `crates/library/tests/{borders,crop,crop_folders,crop_roots,crop_tags,writeback}.rs` | `image` is a dev-dependency of pg-library, and six of its test files use it directly. The plan takes test fixtures from testkit | Move the fixtures into `pg-testkit` and drop the direct dependency | open |

## Platform, codec, metadata, analysis and edit

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| S-05.9 | verify | `crates/render/src/display.rs:127`, `crates/render/src/maker.rs:139` | The row says video makes its own verbatim paths. In the code the caller applies them (`crates/import/src/lib.rs:110`). The row's status text, "fixed 2295b5a (part: 90e9b75, 6d5a6f1)", is ambiguous | Check the render callers; correct the status text | verify |
| F-19 | missing | `crates/model/src/path.rs:51`, `crates/model/src/media.rs:101,108` | `WorkList::wide` and `verbatim_wide` do not exist. `Ext::of_name` and `of_name_bytes` replace `of_name_utf16` | Add the platform helper, or remove it from the plan | open |
| F-20 | missing | `crates/platform/src/dir.rs:398`, `crates/scan/src/lib.rs:177` | Reparse is a bool. The plan wants the reparse tag, read from `EaSize` | Read the tag and skip by it | open |
| F-21 | missing | `crates/writeback/src/file.rs:43,62`, `crates/crop/src/lib.rs:224` | `copy_into`, `temp_path` and `set_modified` are not in platform. Their equivalents are `temp_of`, `write_into` and `keep_time` | Move the helpers to pg-platform | open |
| F-22 | shortcut | `crates/app/src/store.rs:68` | Atomic replace for small files is a private `write_atomically` in pg-app | Move it to pg-platform | open |
| F-23 | shortcut | `crates/codec/src/decode.rs:218-249`, `crates/codec/src/fixtures.rs:11` | The plan's `orient` API in the codec is not built under that name. The non-test equivalents (`orient_into`, `orient_pixels`, `transform`, `transform_into`) exist | Align the names, or correct the plan | verify |
| F-24 | missing | `crates/edit/src/crop.rs:65`, `crates/codec` | `reencoded` is in pg-edit. The plan moves it to codec's `encode` | Move it into codec | open |
| F-25 | shortcut | `crates/render/src/maker.rs:160` | The JPEG `encode_image` wrapper is a private `Maker` method. The plan puts it in codec | Move it into codec | open |
| F-26 | missing | `crates/render/src/display.rs:149`, `crates/desktop/src/providers.rs:80` | The plan's `pixels` module, layout parameter and `straight` upload constructor are not built. Only `Rgba::opaque` exists, and the desktop maps the opaque flag to a pixel format | Add the constructor and the pixel format to codec | open |
| F-27 | missing | `crates/metadata/src/container.rs:114` | `extended_packets` is a private function in pg-metadata. The plan puts it in pg-container | Move it to pg-container | open |
| F-28 | decide | `crates/analysis/src/borders.rs` (407 lines) and `crates/analysis/src/borders/` (six files), `crates/borders/src/lib.rs` (139 lines) | The border detector is still in pg-analysis. pg-borders holds only the pass and imports `pg_analysis::analyze`. The as-built text says the detector moved | Move the detector into pg-borders, or correct the as-built text and M-15 | decide |
| F-29 | decide | `crates/metadata/src/` | The plan's `read` and `patch` modules are not there. `Meta::read` is in `lib.rs:55`, `patch_orientation` in `orientation.rs:60`, and `inflate_zlib` and `png_itxt_xmp` in `container.rs` | Accept the layout and correct the plan, or split the modules | decide |

## Tests

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| F-30 | shortcut | `crates/library/tests/video.rs:133,165-198`, `crates/video/tests/` | Decoder-level probe and frame tests are in pg-library. `crates/video/tests/` holds only `sample.mp4` | Move them to `crates/video/tests/` | open |
| F-31 | missing | `crates/codec/tests/` | The plan's codec animation tests do not exist | Add them | open |
| F-32 | shortcut | `crates/video/src/lib.rs:294`, `crates/app/src/store.rs:91`, `crates/app/tests/listing_laws.rs:24`, `crates/dbfile/tests/live.rs:12` | Thin local scratch-directory wrappers remain. The plan moves them to testkit | Use testkit's helper | open |
| F-33 | missing | `crates/testkit/src/lib.rs`; `crates/cli/src/lib.rs:371` | `wait_for_writes` is a private function in cli. The letterbox helper and ffmpeg discovery are not in testkit | Move them into testkit | open |
| F-34 | decide | `crates/desktop/Cargo.toml` | The plan's `[lib] test = false` for pg-desktop is not set; only `doctest = false` is. The desktop's unit tests run | Retire the `test = false` target. Tests should run | decide |
| F-35 | untested | `crates/model/src/pdq.rs:12`, `crates/model/src/detection.rs:7` | The macro-pair tests, which check `group_distance!` and `min_side!` against their constants, do not exist | Add them | open |

## Model

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| F-36 | decide | `crates/model` (no `Session` type); nearest are `crates/model/src/view.rs:12` (`Listing`) and `:30` (`SavedView`) | The plan's `Session` record (folder, recursive, query, sort, last file) is not built | Build it, or record that `Listing` and `SavedView` replace it | decide |
| F-37 | shortcut | `crates/model/src/tree.rs:313` | `FolderTree::anchors(n)` returns a formatted "D: E:" string. The plan puts display text in pg-client | Move the formatting into `pg-client::text` | open |
| F-38 | shortcut | `crates/model/src/query.rs:27,53,79,154`, `crates/model/src/write.rs:37`, `crates/model/src/media.rs:36` | Hand-written `key()` and `from_key()` on `Shape`, `Status`, `BorderFacet`, `SortKey`, `WriteKind` and `media::Kind`. The plan wants strum derives and a test that lists every key | Derive with strum, and add the all-keys test | open |
| F-39 | shortcut | `crates/model/src/image.rs:12-34,39,45` | `ImageKey` is `{file, version}`, not the planned `{file, source, turns}`. It has `Display` but no `FromStr`; `parse` is inherent | Decide the shape. Add `FromStr` or correct the plan | decide |
| F-40 | shortcut | `crates/model/src/path.rs`, `crates/model/src/rules.rs:257,266,281`, `crates/model/src/name.rs:124` | `components`, `path_key` and `order_key_of` are not in the path module. The order keys are in `rules.rs`. `scope` became `scope_keys` | Move them, or correct the plan | decide |
| F-41 | decide | `crates/model/src/grid.rs:41`, `crates/model/src/tree.rs:81`, `crates/query/src/lib.rs:416`, `crates/client/src/image.rs:44`, `crates/client/src/layout.rs:96`, `crates/model/src/query.rs:371,406,419` | Placement and naming differences, to accept or correct: the `Entry` fields (`content`, `shown_cut`, `side_line`, `stored_size`, `shown`) differ; `TreeBuilder` is in model, not `catalog::read::tree`; `ImageRequest` is in pg-client; the layout value is a borrowed `Placement`; the query API is `toggle(Choice)` and `number`/`set_number`, not `toggle(Facet, value)` and `bound(RangeFacet, End)` | Accept each and correct the plan, or move the code | decide |

## Client: grid and tree

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| F-42 | missing | `crates/client/src/grid.rs` | The list snapshot carries no catalog `seq`, and the plan's "changes since the snapshot's seq are patched in" is not built | Add the `seq` to the snapshot, and patch the changes since it in | open |
| F-43 | shortcut | `crates/client/src/grid.rs:122-125,323,345,352,359` | The `edited` and `turned` overlay is still there (`edit()`, `note_turns_written()`, `take_turned()`, `refresh_turned()`). The plan replaces it with the change feed, which covers other clients' edits too | Replace the overlay with the change-feed patch | open |
| F-44 | missing | `crates/client/src/grid.rs:31-39,44-53` | Per-row versions and `GridUpdate::Changed(range, Roles{span, version, height})` are not built. `Update` has `span`, `items` and `height`; `Shape` has `Reset`, `Insert`, `Remove` and `Same`. The rule "a row gets a new span or a new version, never both" cannot be tested | Add per-row versions and the `Changed` roles, and test that rule | open |

## Desktop QML

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| F-45 | missing | `crates/desktop/qml/*.qml` | 106 functions of controller logic remain in QML: Main 25, Viewer 17, InboxPanel 15, FilterBar 11. Examples: `Main.qml:142` `updateFacets`, `:193` `relayout`, `:315` `select` | Move each into a Rust facade, as S-12 says | open |
| F-46 | shortcut | `crates/desktop/qml/Main.qml` (758 lines), `:209` `gridLoaded`, `:232` `treeLoaded` | `Main.qml` is still 758 lines, and start choreography runs through `gridLoaded` and `treeLoaded`. The plan's startup gate is not built | Build the startup gate, then move the choreography into it | open |
| F-47 | shortcut | `crates/desktop/qml/FilterBar.qml:35-37,57` | `FilterBar` instantiates `Filters {}` directly and calls `countFacets()` and `reload()` on it, bypassing a facade | Route the calls through the facade | open |
| F-48 | decide | `crates/desktop/src/keymap.rs`, `viewport.rs`, `words.rs`; `Words` in 10+ components (e.g. `FolderTree.qml:21`, `InboxPanel.qml:21`); `KeyMap` in `GlobalShortcuts.qml:20`, `Hud.qml:46`, `Main.qml:347` | The S-12 facade criterion is undecided. There is no grid, tree, viewer or yandex facade, and the per-component bridges remain | Decide the criterion, then build the facades it names | decide |
| F-49 | decide | `crates/desktop/src/gallery_bridge.rs:30`, `crates/desktop/qml/Viewer.qml:29-69` | The `CurrentItem` spike and `#[qml_singleton]` are not built. `Viewer.qml` has 8 `required property` declarations | Run the spike, or drop it from the plan | decide |
| F-50 | shortcut | `crates/desktop/qml/Theme.qml` and 16 other QML files | `Theme.qml` is a singleton, but hex colours are still hand-typed: `Main.qml` 14, `InboxPanel.qml` 13, and five each in `GalleryGrid` and `LogPanel` (`StatusBar`'s are in `Theme` now, `statusText` and `statusWarning`) | Move the colours into `Theme` | open |
| F-51 | shortcut | `crates/desktop/qml/FilterBar.qml:41-46`, `FolderTree.qml:56-57`, `TopBar.qml:36-40`, `crates/desktop/src/folders.rs:37-39` | Facet kinds and folder kinds are hand-typed integers (`facetTypes: 0` to `5`, `node.kind === 0/1`). The sort names are a QML list aligned by index to `pg_client::sort::KEYS` | Export the Rust enums to QML | open |

## Order of work (S-12 to S-21)

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| S-13.5 | stale-doc | `crates/catalog/src/schema.rs:51-53`, `crates/catalog/src/rebuild.rs:170` | The row says `MIGRATIONS` is empty. It has three steps (`TO_2_CHANGE_STAMPS`, `TO_3_NOTICES`, and more). `carry_rows` is in `rebuild.rs`, not `lib.rs`. The row's 12-second explanation is stale. The `round_done` stage was not compared, and the rebuild's first-start cost was not timed | Time the first start and compare `round_done`; correct the row | open |
| S-14 | untested | `crates/desktop/qml/tst_wheellistview.qml`, the shots matrix | The wheel test calls `wheelBy()` directly, so the `WheelHandler` is not tested. The bordered fixture and the Yandex results panel are not in the shots matrix | Test through the `WheelHandler`; add both to the matrix | open |
| S-15 | verify | CI on `main` and HEAD | The latest completed CI run on `main` is `success` on `cffa7bf`. No run is recorded for the 78 commits after it. Branch protection was not checked | Run CI on HEAD. The repository owner checks branch protection | verify |
| S-16.4 | stale-doc | `docs/perf/baseline.json`, `crates/bench/benches/ir/catalog.rs:40` | The checkpoints are not run. The row says `baseline.json` has 11 results, the layout is v5, and the baseline was measured in WSL. The file has 19 results, the layout is `v7-windows-rules`, and the baseline was re-run in `1f26831` | Run the checkpoints, and correct the row | open |
| S-21 | missing | `docs/plans/REVIEW.md` | Open review rows: R-66, R-76, R-77, R-80, R-112, R-113 and R-127. R-75, R-83 and R-101 are partly fixed. R-78 is closed, and the row still lists it | Fix the open rows, and remove R-78 from S-21 | open |

## Deferred by the plan

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| F-52 | deferred | `crates/platform` (no MFT or USN code); `crates/cli/src/args.rs` (no elevated subcommand) | The MFT reader, the USN journal reader and the elevated `gallery` subcommand are not built | Build them per `docs/temp/NTFS_MFT_SCAN_PLAN.md` | deferred |
| F-53 | deferred | `crates/catalog/src/browse/read/grid.rs:115`, `browse/read/facets.rs:62`, `browse/read/tree.rs:11` | The grid range with a total, and facet and tree counts from trigger counters, are not built. Both still use `GROUP BY` | Build them when the plan's later step comes | deferred |
| F-54 | deferred | `crates/dedup` (`verifier`, `multicut` and `keeper` absent) | The tier-2 dedup work is not built | Build it at tier 2 | deferred |
| F-55 | deferred | `crates/codec/Cargo.toml` | The `heif` and `raw` features are not started | Start them at their step | deferred |

## Stale tool docs

These are comments in tooling that describe the code wrongly. They are not code work.

| ID | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|
| F-58 | stale-doc | `tools/lint_qml.py:8-10`, `S-02.2` | The lint's docstring says it copies `qml/*.qml`, and the old `tools/lint-qml.ps1` name is still cited in the docs. `eval/common.py:70`, `eval/embed.py:19` and `tools/profiling/bench.py:89` still name `crates/core` or `pg-core`, and S-02.2 is marked fixed | Update the docstring, and rename the stale names in the three files | open |

## Not checked

These parts of the plan were not checked by reading. They need a run, a person at the app, or a reading not yet done.

- The latest-wins request boilerplate: the plan names three sites, and only `crates/app/src/reader.rs:7` was found.
- The bodies of `Main.qml`'s 25 functions, and the write-back rule, the library change detection and tree expansion in `crates/client/src/tree.rs`.
- The per-worker scratch (`codec::Codecs`, `MetaScratch`) and the allocation claims, which need the benches.
- The screenshot, pixel and timing claims in S-02.3, S-14, S-16 and the M rows.
- The "Outcome" section (the cargo-deny confinement claim), and whether `cargo doc` passes now.
