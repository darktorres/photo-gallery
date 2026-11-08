# Code ledger

Lists the code files in the repository, with the vendored crates left out.
Recorded 2026-11-06 at commit `2b35a09`; line counts are for that commit, except `eval/test_viewer.py` (added in 262004b) and `eval/jpeg-decoders/env.py` (added with decision 5) and `crates/render/src/transcode.rs` (added later, with the C-10 change)
and counted there.

## Scope

- In: the tracked files in Rust (`.rs`), QML, C++ (`.cpp`, `.h`), Python, shell, PowerShell and batch, `eval/viewer.html`,
  `Justfile` and `CMakeLists.txt`.
- Out: everything under `vendor/`.
- Out: manifests and configuration (`Cargo.toml`, `*.lock`, `clippy.toml`, `deny.toml`), Fluent locale files, patches,
  documentation, test data and images.

## Totals

| Language | Files | Lines |
|---|---:|---:|
| Rust | 210 | 41894 |
| Python | 37 | 7039 |
| QML | 38 | 4157 |
| C++ | 25 | 2314 |
| PowerShell | 9 | 1094 |
| HTML | 1 | 861 |
| Shell | 14 | 652 |
| Batch | 2 | 88 |
| Just | 1 | 87 |
| CMake | 2 | 74 |
| Total | 339 | 58260 |

## Sections

| Section | Files | Lines |
|---|---:|---:|
| `crates/analysis` | 5 | 1698 |
| `crates/app` | 16 | 2168 |
| `crates/bench` | 3 | 208 |
| `crates/catalog` | 21 | 6495 |
| `crates/cli` | 3 | 860 |
| `crates/client` | 33 | 4984 |
| `crates/codec` | 8 | 1936 |
| `crates/container` | 2 | 376 |
| `crates/dedup` | 4 | 662 |
| `crates/desktop` | 82 | 10363 |
| `crates/edit` | 4 | 957 |
| `crates/exiftool` | 1 | 236 |
| `crates/gallery` | 2 | 98 |
| `crates/import` | 1 | 130 |
| `crates/jobs` | 8 | 1356 |
| `crates/library` | 10 | 1671 |
| `crates/metadata` | 1 | 1220 |
| `crates/model` | 27 | 3952 |
| `crates/platform` | 9 | 1342 |
| `crates/query` | 1 | 621 |
| `crates/render` | 5 | 578 |
| `crates/scan` | 1 | 566 |
| `crates/schema` | 1 | 345 |
| `crates/testkit` | 2 | 198 |
| `crates/thumbcache` | 1 | 1762 |
| `crates/thumbs` | 1 | 163 |
| `crates/trace` | 1 | 87 |
| `crates/video` | 2 | 368 |
| `crates/writeback` | 1 | 375 |
| `crates/yandex` | 1 | 619 |
| `docs/probes` | 9 | 663 |
| `eval` | 47 | 8462 |
| `fuzz` | 7 | 96 |
| `tools` | 19 | 2610 |
| Repository root | 1 | 87 |

## crates/analysis

| File | Language | Lines |
|---|---|---:|
| `crates/analysis/examples/stage_times.rs` | Rust | 65 |
| `crates/analysis/src/analyze.rs` | Rust | 247 |
| `crates/analysis/src/borders.rs` | Rust | 975 |
| `crates/analysis/src/lib.rs` | Rust | 10 |
| `crates/analysis/src/pdq.rs` | Rust | 401 |

## crates/app

| File | Language | Lines |
|---|---|---:|
| `crates/app/src/app.rs` | Rust | 187 |
| `crates/app/src/config.rs` | Rust | 34 |
| `crates/app/src/edit.rs` | Rust | 23 |
| `crates/app/src/error.rs` | Rust | 31 |
| `crates/app/src/images.rs` | Rust | 338 |
| `crates/app/src/lib.rs` | Rust | 32 |
| `crates/app/src/library.rs` | Rust | 212 |
| `crates/app/src/pool.rs` | Rust | 40 |
| `crates/app/src/prefs.rs` | Rust | 73 |
| `crates/app/src/reader.rs` | Rust | 235 |
| `crates/app/src/reexport.rs` | Rust | 13 |
| `crates/app/src/store.rs` | Rust | 155 |
| `crates/app/src/views.rs` | Rust | 67 |
| `crates/app/src/yandex.rs` | Rust | 456 |
| `crates/app/src/yandex/replay.rs` | Rust | 199 |
| `crates/app/tests/listing_laws.rs` | Rust | 73 |

## crates/bench

| File | Language | Lines |
|---|---|---:|
| `crates/bench/benches/ir.rs` | Rust | 12 |
| `crates/bench/benches/ir/catalog.rs` | Rust | 136 |
| `crates/bench/benches/ir/linux.rs` | Rust | 60 |

## crates/catalog

| File | Language | Lines |
|---|---|---:|
| `crates/catalog/src/browse.rs` | Rust | 9 |
| `crates/catalog/src/browse/fixtures.rs` | Rust | 103 |
| `crates/catalog/src/browse/read.rs` | Rust | 39 |
| `crates/catalog/src/browse/read/changes.rs` | Rust | 64 |
| `crates/catalog/src/browse/read/details.rs` | Rust | 93 |
| `crates/catalog/src/browse/read/facets.rs` | Rust | 96 |
| `crates/catalog/src/browse/read/grid.rs` | Rust | 148 |
| `crates/catalog/src/browse/read/state.rs` | Rust | 94 |
| `crates/catalog/src/browse/read/tests.rs` | Rust | 726 |
| `crates/catalog/src/browse/read/tree.rs` | Rust | 41 |
| `crates/catalog/src/browse/read/views.rs` | Rust | 33 |
| `crates/catalog/src/browse/writes.rs` | Rust | 80 |
| `crates/catalog/src/dups.rs` | Rust | 184 |
| `crates/catalog/src/error.rs` | Rust | 39 |
| `crates/catalog/src/fixture.rs` | Rust | 78 |
| `crates/catalog/src/fixture/raw.rs` | Rust | 268 |
| `crates/catalog/src/fixture/state.rs` | Rust | 183 |
| `crates/catalog/src/lib.rs` | Rust | 2684 |
| `crates/catalog/src/pair.rs` | Rust | 507 |
| `crates/catalog/src/schema.rs` | Rust | 598 |
| `crates/catalog/src/writes.rs` | Rust | 428 |

## crates/cli

| File | Language | Lines |
|---|---|---:|
| `crates/cli/src/borders.rs` | Rust | 277 |
| `crates/cli/src/dups.rs` | Rust | 37 |
| `crates/cli/src/lib.rs` | Rust | 546 |

## crates/client

| File | Language | Lines |
|---|---|---:|
| `crates/client/src/controller.rs` | Rust | 373 |
| `crates/client/src/controller/deadlines.rs` | Rust | 69 |
| `crates/client/src/controller/input.rs` | Rust | 95 |
| `crates/client/src/controller/listing.rs` | Rust | 53 |
| `crates/client/src/controller/panels.rs` | Rust | 82 |
| `crates/client/src/controller/prefs.rs` | Rust | 105 |
| `crates/client/src/controller/replay.rs` | Rust | 50 |
| `crates/client/src/controller/start.rs` | Rust | 83 |
| `crates/client/src/facets.rs` | Rust | 37 |
| `crates/client/src/grid.rs` | Rust | 663 |
| `crates/client/src/input.rs` | Rust | 163 |
| `crates/client/src/keys.rs` | Rust | 324 |
| `crates/client/src/layout.rs` | Rust | 297 |
| `crates/client/src/lib.rs` | Rust | 15 |
| `crates/client/src/picks.rs` | Rust | 155 |
| `crates/client/src/sort.rs` | Rust | 27 |
| `crates/client/src/text.rs` | Rust | 961 |
| `crates/client/src/tree.rs` | Rust | 247 |
| `crates/client/src/viewport.rs` | Rust | 325 |
| `crates/client/tests/controller.rs` | Rust | 15 |
| `crates/client/tests/controller/deadlines.rs` | Rust | 57 |
| `crates/client/tests/controller/input.rs` | Rust | 83 |
| `crates/client/tests/controller/listing.rs` | Rust | 87 |
| `crates/client/tests/controller/panels.rs` | Rust | 33 |
| `crates/client/tests/controller/prefs.rs` | Rust | 31 |
| `crates/client/tests/controller/start.rs` | Rust | 138 |
| `crates/client/tests/controller_laws.rs` | Rust | 142 |
| `crates/client/tests/facets.rs` | Rust | 13 |
| `crates/client/tests/grid_laws.rs` | Rust | 77 |
| `crates/client/tests/laws.rs` | Rust | 80 |
| `crates/client/tests/replay.rs` | Rust | 51 |
| `crates/client/tests/sort.rs` | Rust | 21 |
| `crates/client/tests/tree_laws.rs` | Rust | 32 |

## crates/codec

| File | Language | Lines |
|---|---|---:|
| `crates/codec/src/animation.rs` | Rust | 89 |
| `crates/codec/src/codecs.rs` | Rust | 1292 |
| `crates/codec/src/decode.rs` | Rust | 357 |
| `crates/codec/src/encode.rs` | Rust | 21 |
| `crates/codec/src/fixtures.rs` | Rust | 15 |
| `crates/codec/src/lib.rs` | Rust | 16 |
| `crates/codec/src/pixels.rs` | Rust | 19 |
| `crates/codec/src/resize.rs` | Rust | 127 |

## crates/container

| File | Language | Lines |
|---|---|---:|
| `crates/container/src/lib.rs` | Rust | 344 |
| `crates/container/tests/search_laws.rs` | Rust | 32 |

## crates/dedup

| File | Language | Lines |
|---|---|---:|
| `crates/dedup/src/lib.rs` | Rust | 14 |
| `crates/dedup/src/pairer.rs` | Rust | 48 |
| `crates/dedup/src/pdq_index.rs` | Rust | 541 |
| `crates/dedup/src/union_find.rs` | Rust | 59 |

## crates/desktop

| File | Language | Lines |
|---|---|---:|
| `crates/desktop/build.rs` | Rust | 80 |
| `crates/desktop/cpp/app_icon.cpp` | C++ | 16 |
| `crates/desktop/cpp/app_icon.h` | C++ | 9 |
| `crates/desktop/cpp/deadline.cpp` | C++ | 46 |
| `crates/desktop/cpp/deadline.h` | C++ | 16 |
| `crates/desktop/cpp/frame_sink.cpp` | C++ | 40 |
| `crates/desktop/cpp/frame_sink.h` | C++ | 43 |
| `crates/desktop/cpp/gpu_device.cpp` | C++ | 152 |
| `crates/desktop/cpp/gpu_device.h` | C++ | 35 |
| `crates/desktop/cpp/gui_app.cpp` | C++ | 66 |
| `crates/desktop/cpp/gui_app.h` | C++ | 15 |
| `crates/desktop/cpp/live_reload.cpp` | C++ | 111 |
| `crates/desktop/cpp/live_reload.h` | C++ | 19 |
| `crates/desktop/cpp/providers.cpp` | C++ | 155 |
| `crates/desktop/cpp/providers.h` | C++ | 17 |
| `crates/desktop/cpp/screenshot.cpp` | C++ | 176 |
| `crates/desktop/cpp/screenshot.h` | C++ | 13 |
| `crates/desktop/qml/BorderOverlay.qml` | QML | 81 |
| `crates/desktop/qml/FilterBar.qml` | QML | 292 |
| `crates/desktop/qml/FolderTree.qml` | QML | 57 |
| `crates/desktop/qml/GalleryGrid.qml` | QML | 153 |
| `crates/desktop/qml/GridCell.qml` | QML | 145 |
| `crates/desktop/qml/GridRow.qml` | QML | 137 |
| `crates/desktop/qml/Hud.qml` | QML | 108 |
| `crates/desktop/qml/ImageLayers.qml` | QML | 110 |
| `crates/desktop/qml/LibraryPanel.qml` | QML | 91 |
| `crates/desktop/qml/Loupe.qml` | QML | 55 |
| `crates/desktop/qml/Main.qml` | QML | 556 |
| `crates/desktop/qml/MediaView.qml` | QML | 122 |
| `crates/desktop/qml/ModalOverlay.qml` | QML | 28 |
| `crates/desktop/qml/RatingBar.qml` | QML | 94 |
| `crates/desktop/qml/RatingPopup.qml` | QML | 27 |
| `crates/desktop/qml/Ratings.qml` | QML | 16 |
| `crates/desktop/qml/ResultTile.qml` | QML | 70 |
| `crates/desktop/qml/SaveViewDialog.qml` | QML | 55 |
| `crates/desktop/qml/StatusBar.qml` | QML | 82 |
| `crates/desktop/qml/TestHooks.qml` | QML | 122 |
| `crates/desktop/qml/Theme.qml` | QML | 26 |
| `crates/desktop/qml/TopBar.qml` | QML | 133 |
| `crates/desktop/qml/VideoTransport.qml` | QML | 91 |
| `crates/desktop/qml/Viewer.qml` | QML | 505 |
| `crates/desktop/qml/ViewsMenu.qml` | QML | 64 |
| `crates/desktop/qml/WheelListView.qml` | QML | 96 |
| `crates/desktop/qml/YandexPanel.qml` | QML | 242 |
| `crates/desktop/qml/YandexPreview.qml` | QML | 186 |
| `crates/desktop/src/animation.rs` | Rust | 139 |
| `crates/desktop/src/app.rs` | Rust | 86 |
| `crates/desktop/src/app_icon.rs` | Rust | 13 |
| `crates/desktop/src/command_line.rs` | Rust | 175 |
| `crates/desktop/src/controller.rs` | Rust | 110 |
| `crates/desktop/src/deadline.rs` | Rust | 64 |
| `crates/desktop/src/facet_choices.rs` | Rust | 203 |
| `crates/desktop/src/filters.rs` | Rust | 316 |
| `crates/desktop/src/folders.rs` | Rust | 268 |
| `crates/desktop/src/gallery.rs` | Rust | 1037 |
| `crates/desktop/src/gpu_device.rs` | Rust | 62 |
| `crates/desktop/src/gui_app.rs` | Rust | 29 |
| `crates/desktop/src/keymap.rs` | Rust | 248 |
| `crates/desktop/src/lib.rs` | Rust | 132 |
| `crates/desktop/src/library.rs` | Rust | 348 |
| `crates/desktop/src/live_reload.rs` | Rust | 37 |
| `crates/desktop/src/panic_log.rs` | Rust | 68 |
| `crates/desktop/src/providers.rs` | Rust | 151 |
| `crates/desktop/src/screenshot.rs` | Rust | 38 |
| `crates/desktop/src/settings.rs` | Rust | 108 |
| `crates/desktop/src/shell.rs` | Rust | 644 |
| `crates/desktop/src/trace.rs` | Rust | 49 |
| `crates/desktop/src/viewport.rs` | Rust | 247 |
| `crates/desktop/src/words.rs` | Rust | 115 |
| `crates/desktop/src/yandex.rs` | Rust | 448 |
| `crates/desktop/tests/qml/stubs/pg/gallery/AnimationPlayer.qml` | QML | 9 |
| `crates/desktop/tests/qml/stubs/pg/gallery/GalleryModel.qml` | QML | 29 |
| `crates/desktop/tests/qml/stubs/pg/gallery/KeyMap.qml` | QML | 16 |
| `crates/desktop/tests/qml/stubs/pg/gallery/Settings.qml` | QML | 6 |
| `crates/desktop/tests/qml/stubs/pg/gallery/Shell.qml` | QML | 11 |
| `crates/desktop/tests/qml/stubs/pg/gallery/Viewport.qml` | QML | 24 |
| `crates/desktop/tests/qml/stubs/pg/gallery/Words.qml` | QML | 22 |
| `crates/desktop/tests/qml/tst_gridrow.qml` | QML | 111 |
| `crates/desktop/tests/qml/tst_viewer.qml` | QML | 110 |
| `crates/desktop/tests/qml/tst_wheellistview.qml` | QML | 75 |
| `crates/desktop/tests/qml_host/CMakeLists.txt` | CMake | 13 |
| `crates/desktop/tests/qml_host/main.cpp` | C++ | 49 |

## crates/edit

| File | Language | Lines |
|---|---|---:|
| `crates/edit/src/crop.rs` | Rust | 162 |
| `crates/edit/src/lib.rs` | Rust | 45 |
| `crates/edit/src/rewrite.rs` | Rust | 490 |
| `crates/edit/src/rotate.rs` | Rust | 260 |

## crates/exiftool

| File | Language | Lines |
|---|---|---:|
| `crates/exiftool/src/lib.rs` | Rust | 236 |

## crates/gallery

| File | Language | Lines |
|---|---|---:|
| `crates/gallery/build.rs` | Rust | 12 |
| `crates/gallery/src/main.rs` | Rust | 86 |

## crates/import

| File | Language | Lines |
|---|---|---:|
| `crates/import/src/lib.rs` | Rust | 130 |

## crates/jobs

| File | Language | Lines |
|---|---|---:|
| `crates/jobs/src/control.rs` | Rust | 273 |
| `crates/jobs/src/events.rs` | Rust | 200 |
| `crates/jobs/src/failure.rs` | Rust | 20 |
| `crates/jobs/src/lib.rs` | Rust | 11 |
| `crates/jobs/src/panics.rs` | Rust | 35 |
| `crates/jobs/src/pipeline.rs` | Rust | 524 |
| `crates/jobs/src/schedule.rs` | Rust | 89 |
| `crates/jobs/src/service.rs` | Rust | 204 |

## crates/library

| File | Language | Lines |
|---|---|---:|
| `crates/library/src/borders.rs` | Rust | 76 |
| `crates/library/src/crop.rs` | Rust | 178 |
| `crates/library/src/dups.rs` | Rust | 60 |
| `crates/library/src/lib.rs` | Rust | 24 |
| `crates/library/src/runner.rs` | Rust | 516 |
| `crates/library/tests/borders.rs` | Rust | 53 |
| `crates/library/tests/crop.rs` | Rust | 162 |
| `crates/library/tests/crop_roots.rs` | Rust | 68 |
| `crates/library/tests/video.rs` | Rust | 206 |
| `crates/library/tests/writeback.rs` | Rust | 328 |

## crates/metadata

| File | Language | Lines |
|---|---|---:|
| `crates/metadata/src/lib.rs` | Rust | 1220 |

## crates/model

| File | Language | Lines |
|---|---|---:|
| `crates/model/src/analysis.rs` | Rust | 143 |
| `crates/model/src/date.rs` | Rust | 154 |
| `crates/model/src/details.rs` | Rust | 15 |
| `crates/model/src/detection.rs` | Rust | 207 |
| `crates/model/src/format.rs` | Rust | 66 |
| `crates/model/src/grid.rs` | Rust | 133 |
| `crates/model/src/ids.rs` | Rust | 74 |
| `crates/model/src/image.rs` | Rust | 217 |
| `crates/model/src/lib.rs` | Rust | 29 |
| `crates/model/src/media.rs` | Rust | 109 |
| `crates/model/src/name.rs` | Rust | 316 |
| `crates/model/src/num.rs` | Rust | 128 |
| `crates/model/src/orientation.rs` | Rust | 241 |
| `crates/model/src/path.rs` | Rust | 295 |
| `crates/model/src/pdq.rs` | Rust | 46 |
| `crates/model/src/query.rs` | Rust | 533 |
| `crates/model/src/rating.rs` | Rust | 22 |
| `crates/model/src/rules.rs` | Rust | 327 |
| `crates/model/src/state.rs` | Rust | 41 |
| `crates/model/src/tree.rs` | Rust | 360 |
| `crates/model/src/turns.rs` | Rust | 31 |
| `crates/model/src/view.rs` | Rust | 28 |
| `crates/model/src/write.rs` | Rust | 24 |
| `crates/model/tests/laws.rs` | Rust | 132 |
| `crates/model/tests/name_laws.rs` | Rust | 77 |
| `crates/model/tests/path_laws.rs` | Rust | 60 |
| `crates/model/tests/rules_reference.rs` | Rust | 144 |

## crates/platform

| File | Language | Lines |
|---|---|---:|
| `crates/platform/src/clock.rs` | Rust | 92 |
| `crates/platform/src/dir.rs` | Rust | 749 |
| `crates/platform/src/file.rs` | Rust | 70 |
| `crates/platform/src/lib.rs` | Rust | 14 |
| `crates/platform/src/message.rs` | Rust | 16 |
| `crates/platform/src/names.rs` | Rust | 130 |
| `crates/platform/src/native.rs` | Rust | 189 |
| `crates/platform/src/priority.rs` | Rust | 28 |
| `crates/platform/src/process.rs` | Rust | 54 |

## crates/query

| File | Language | Lines |
|---|---|---:|
| `crates/query/src/lib.rs` | Rust | 621 |

## crates/render

| File | Language | Lines |
|---|---|---:|
| `crates/render/src/display.rs` | Rust | 70 |
| `crates/render/src/lib.rs` | Rust | 11 |
| `crates/render/src/maker.rs` | Rust | 276 |
| `crates/render/src/player.rs` | Rust | 169 |
| `crates/render/src/transcode.rs` | Rust | 52 |

## crates/scan

| File | Language | Lines |
|---|---|---:|
| `crates/scan/src/lib.rs` | Rust | 566 |

## crates/schema

| File | Language | Lines |
|---|---|---:|
| `crates/schema/src/lib.rs` | Rust | 345 |

## crates/testkit

| File | Language | Lines |
|---|---|---:|
| `crates/testkit/src/events.rs` | Rust | 41 |
| `crates/testkit/src/lib.rs` | Rust | 157 |

## crates/thumbcache

| File | Language | Lines |
|---|---|---:|
| `crates/thumbcache/src/lib.rs` | Rust | 1762 |

## crates/thumbs

| File | Language | Lines |
|---|---|---:|
| `crates/thumbs/src/lib.rs` | Rust | 163 |

## crates/trace

| File | Language | Lines |
|---|---|---:|
| `crates/trace/src/lib.rs` | Rust | 87 |

## crates/video

| File | Language | Lines |
|---|---|---:|
| `crates/video/build.rs` | Rust | 62 |
| `crates/video/src/lib.rs` | Rust | 306 |

## crates/writeback

| File | Language | Lines |
|---|---|---:|
| `crates/writeback/src/lib.rs` | Rust | 375 |

## crates/yandex

| File | Language | Lines |
|---|---|---:|
| `crates/yandex/src/lib.rs` | Rust | 619 |

## docs/probes

| File | Language | Lines |
|---|---|---:|
| `docs/probes/v1_symlink.ps1` | PowerShell | 76 |
| `docs/probes/v1b_junction.ps1` | PowerShell | 68 |
| `docs/probes/v2_volumes.ps1` | PowerShell | 85 |
| `docs/probes/v34_wsl.sh` | Shell | 19 |
| `docs/probes/v4_drvfs_superblock.sh` | Shell | 21 |
| `docs/probes/win_probe.ps1` | PowerShell | 168 |
| `docs/probes/win_probe2.ps1` | PowerShell | 120 |
| `docs/probes/wsl_probe.sh` | Shell | 81 |
| `docs/probes/wsl_probe2.sh` | Shell | 25 |

## eval

| File | Language | Lines |
|---|---|---:|
| `eval/accel.py` | Python | 344 |
| `eval/bench_ann.py` | Python | 360 |
| `eval/bench_decode.py` | Python | 223 |
| `eval/bench_hdd.py` | Python | 237 |
| `eval/bench_models.py` | Python | 48 |
| `eval/bench_semantic.py` | Python | 158 |
| `eval/bench_thumbs.py` | Python | 413 |
| `eval/check_groups.py` | Python | 95 |
| `eval/common.py` | Python | 95 |
| `eval/edits.py` | Python | 138 |
| `eval/embed.py` | Python | 50 |
| `eval/evaluate.py` | Python | 222 |
| `eval/group.py` | Python | 218 |
| `eval/imgload.py` | Python | 129 |
| `eval/jpeg-decoders/benches/ir.rs` | Rust | 84 |
| `eval/jpeg-decoders/debesheedas.py` | Python | 81 |
| `eval/jpeg-decoders/encoders.py` | Python | 225 |
| `eval/jpeg-decoders/env.py` | Python | 25 |
| `eval/jpeg-decoders/gpu.py` | Python | 117 |
| `eval/jpeg-decoders/gpu_encoders.py` | Python | 87 |
| `eval/jpeg-decoders/harness-wsl.sh` | Shell | 52 |
| `eval/jpeg-decoders/harness/CMakeLists.txt` | CMake | 61 |
| `eval/jpeg-decoders/harness/dec_gpu.cpp` | C++ | 283 |
| `eval/jpeg-decoders/harness/dec_jpegli.cpp` | C++ | 61 |
| `eval/jpeg-decoders/harness/dec_parajpeg.cpp` | C++ | 42 |
| `eval/jpeg-decoders/harness/dec_turbo.cpp` | C++ | 52 |
| `eval/jpeg-decoders/harness/dec_wuffs.cpp` | C++ | 61 |
| `eval/jpeg-decoders/harness/decoder.h` | C++ | 45 |
| `eval/jpeg-decoders/harness/harness.cpp` | C++ | 277 |
| `eval/jpeg-decoders/nvimgcodec_bench.py` | Python | 99 |
| `eval/jpeg-decoders/nvjpeg.py` | Python | 328 |
| `eval/jpeg-decoders/restart.sh` | Shell | 62 |
| `eval/jpeg-decoders/src/lib.rs` | Rust | 386 |
| `eval/jpeg-decoders/src/main.rs` | Rust | 134 |
| `eval/jpeg-decoders/table.py` | Python | 53 |
| `eval/jpeg-decoders/win-build.cmd` | Batch | 74 |
| `eval/jpeg-decoders/win-env.cmd` | Batch | 14 |
| `eval/jpeg-decoders/wsl-build.sh` | Shell | 93 |
| `eval/jpeg-decoders/wsl.sh` | Shell | 69 |
| `eval/make_subset.py` | Python | 36 |
| `eval/models.py` | Python | 234 |
| `eval/pdq.py` | Python | 162 |
| `eval/synth.py` | Python | 112 |
| `eval/test_viewer.py` | Python | 117 |
| `eval/verify.py` | Python | 903 |
| `eval/verify_synth.py` | Python | 108 |
| `eval/viewer.html` | HTML | 861 |
| `eval/viewer.py` | Python | 359 |

## fuzz

| File | Language | Lines |
|---|---|---:|
| `fuzz/fuzz_targets/codec_transform.rs` | Rust | 18 |
| `fuzz/fuzz_targets/container_chunks.rs` | Rust | 20 |
| `fuzz/fuzz_targets/container_jpeg.rs` | Rust | 16 |
| `fuzz/fuzz_targets/edit_rotate.rs` | Rust | 11 |
| `fuzz/fuzz_targets/metadata_read.rs` | Rust | 11 |
| `fuzz/fuzz_targets/model_keys.rs` | Rust | 12 |
| `fuzz/fuzz_targets/model_query_json.rs` | Rust | 8 |

## tools

| File | Language | Lines |
|---|---|---:|
| `tools/compare-shots.ps1` | PowerShell | 417 |
| `tools/fuzz-smoke.sh` | Shell | 31 |
| `tools/lint-qml.ps1` | PowerShell | 78 |
| `tools/make_icon.py` | Python | 145 |
| `tools/perf-check.sh` | Shell | 20 |
| `tools/perf-compare.py` | Python | 88 |
| `tools/profiling/bench_data.py` | Python | 81 |
| `tools/profiling/callgraph/build.sh` | Shell | 33 |
| `tools/profiling/callgraph/callgraph.cpp` | C++ | 515 |
| `tools/profiling/callgraph/profile.ps1` | PowerShell | 32 |
| `tools/profiling/callgraph/symbolize.py` | Python | 312 |
| `tools/profiling/cgtab.py` | Python | 364 |
| `tools/profiling/make_bench_jpeg.py` | Python | 63 |
| `tools/profiling/synthetic_library.py` | Python | 123 |
| `tools/profiling/wsl-bench.sh` | Shell | 63 |
| `tools/profiling/wsl-setup.sh` | Shell | 54 |
| `tools/qmltest.ps1` | PowerShell | 50 |
| `tools/shots/fixture.py` | Python | 112 |
| `tools/wsl-check.sh` | Shell | 29 |

## Repository root

| File | Language | Lines |
|---|---|---:|
| `Justfile` | Just | 87 |
