# Code review: every file in the code ledger

A file-by-file review of the code files in `docs/CODE_LEDGER.md`, in ledger order, reading each file in full. Each
finding has a settled fix (see "Fixes") and a status (see "Status").

- **Code reviewed:** the working tree as each file was read, not one commit. The review began at `48f01b0`, and HEAD
  was `92cbaeb` at the end (see "Coverage").
- **Limits:** findings come from reading the code. Nothing was built, run or profiled, except where a fix says a probe
  was run. The fixes were checked against the code by a review workflow (a validator per group of findings, three
  adversarial challenges per group, and four cross-package reviews). Its corrections are already in this document.
- **Not repeated here:** what `docs/REVIEW.md` already has open (`R-nn`).
- **IDs:** `C-nn`, separate from the `R-nn` rows, never reused.
- **Severity:** `high`, `medium` and `low` by impact.
- **Status:** `Fixed` (in a commit, named in the row), `Partly fixed` (part is committed, the rest is planned),
  `Planned` (the fix below is settled, not yet written). Verification is recorded separately, under "Verification".

## Coverage

All 334 files in the ledger were read in full, in ledger order.

- **Not reviewed:** `crates/schema/` and `crates/catalog/src/schema.rs`. They appeared during the review and aren't in
  the ledger. They were only looked up to recheck rows.
- **Changed after being read:** `crates/README.md`, `crates/app/src/app.rs`, `crates/bench/benches/ir/catalog.rs`,
  `crates/catalog/src/error.rs`, `crates/catalog/src/lib.rs`, `crates/cli/src/lib.rs`, `crates/desktop/src/app.rs` and
  `crates/thumbcache/src/lib.rs`. Rows citing them were rechecked against the current text: C-3 is fixed, part of C-7
  went with it, and the line numbers were updated. The new code in those files wasn't reviewed.
- **Checked, no finding:** `Subsamp::mcu_width` on an unknown subsampling (an 8-pixel grid, and a misaligned crop is
  refused), `big_enough` against the decoder's `MinSize::Fit`, and the 10 s wait in `App::consistent` (it ends in
  `Error::Rewriting`). The fuzz targets call the current APIs.
- **Stale in `docs/REVIEW.md`:** R-93, R-95, R-97 and R-99 are already fixed in the code, and have since been removed from
  `docs/REVIEW.md`. R-94 is still listed there: its `groups!` has the `IS NOT NULL`, but `current!` and the query's
  duplicate rule were not rechecked.
- **Corrected by the fix review:** C-22 and C-23 were already fixed at HEAD when their fixes were settled. C-20's
  `goTo` and `Hud.filePath` no longer exist. Four findings described things that were not as stated: C-6's raw string
  is at `tests.rs:319`, C-30's `place_of` doc is right, C-34's `Change` is already above its tests, and C-46's
  conclusion does not follow from the saved runs. Each is corrected in its row and fix below. Line numbers are as of
  the review; `crates/catalog/src/lib.rs` and `crates/thumbcache/src/lib.rs` have uncommitted changes, so their numbers
  may have moved.

## Findings

| ID | Severity | Where | Finding | Status |
|---|---|---|---|---|
| C-1 | low | `crates/app/src/error.rs:20`, `crates/app/src/images.rs:300-307, 190, 196` | `Error::Original` is never constructed. Its doc says a caller can branch on it, but `decode` puts the same text into an anyhow error, which becomes `Error::Failed`, and a video's frame failure has no such text at all. | Fixed (cb123b9: `error::original` builds the variant; `decode` makes it outside `with_maker` for the read, the decode and a video's frame, and the thumbnail's poster frame and read use it too; a test opens a file whose original is gone) |
| C-2 | medium | `crates/catalog/src/writes.rs:77-105`, `crates/catalog/src/lib.rs:1443-1460`, `crates/catalog/src/browse/read/details.rs:40` | A rating made before the file's analysis exists is not queued: `format` is NULL, so `can_hold` is false. The file never gets it, with write-back on and the details panel saying it will (`details.rs:40` treats an unknown format as able to hold one). Analysis-failed rows are not handled either. | Fixed (cb123b9: `set_rating` queues by `writes::rating_in_file_possible` (an image whose format can hold one or is not known yet; not a failed analysis), which the details panel shares; the writer marks a rating done when the file turns out unable to hold one) |
| C-3 | low | `crates/catalog/src/schema.rs:105` | `record_rules` ran after the baseline's own transaction, so a crash between them left a catalog with no `path_rules` row. The rules are now inserted in that transaction. | Fixed (cb123b9) |
| C-4 | low | `crates/catalog/src/writes.rs:191` | `recent_errors` says "the latest errors" but has no `ORDER BY`, so the `LIMIT` keeps arbitrary rows. `failed` stamps `queued`, so the order is available. | Fixed (cb123b9: `recent_errors` orders by `queued DESC, file_id DESC, kind DESC`; `error` is set only by `failed`, which stamps `queued`) |
| C-5 | low | `crates/catalog/src/pair.rs:186-191` | `unpaired_still` treats a statement error as "not unpaired": a failed read silently skips the row's tiers, and the pass commits as if it was done. | Fixed (cb123b9: `unpaired_still` returns `Result<bool>`, a missing row is false; the pairing loops propagate a failed read) |
| C-6 | low | `crates/catalog/src/browse/read/tests.rs:319` | `r"E:\i\u{307}"` is a raw string, so the `\u{307}` is the literal text, not a combining dot. The case meant to check that `İ` and `i̇` lowercase together isn't tested. | Fixed (cb123b9: the case is the ordinary string `"E:\\i\u{307}"`, so the combining dot is tested) |
| C-7 | low | `crates/app/src/library.rs:166-167`, `crates/catalog/src/pair.rs:407` | Docs drift. `remove_folder` says the files stay until the next scan, but `remove_root` marks them missing at once. The test comment says the ambiguous row "stays unpaired", but the clean pass clears it. | Fixed (cb123b9: `remove_folder` says the files are marked missing at once; the test comment says the ambiguous row is counted, then cleared as a new file) |
| C-8 | low | `crates/catalog/src/browse/read/changes.rs:12`, `crates/catalog/src/lib.rs:1458`, `crates/catalog/src/schema.rs` (triggers) | `changes_since` has no production caller yet (its planned consumer, the Bus `Lagged` resync, is not implemented). `store_borders` writes `cut_*` without stamping `files.changed`, since the `analysis_changed` trigger doesn't list those columns. The `changes.rs` doc says every user-visible change stamps its file, and `Entry` shows the cut. | Fixed (cb123b9: migration 2, the first, adds `analysis_cut_stamp`, `analysis_removed_stamp` and the `border_sides` insert and delete stamps, which set `files.changed` and not `files.seq`; the baseline is frozen; `changes_since` is kept and its doc says it has no caller yet) |
| C-9 | low | `crates/client/src/controller.rs:336-340` | `flush` drops `Timer::ApplyQuery`, and that timer's fire is also what saves the session. A query edited within 300 ms of closing isn't saved, despite the "no save is lost" doc. | Fixed (cb123b9: `flush` turns a waiting `ApplyQuery` or `SaveSession` into one `SaveSession` fire, which saves the last file too, and never shows a list; three tests in `controller_laws.rs`) |
| C-10 | low | `crates/app/src/yandex.rs:203-209` | `is_still_image` leaves out tif and tiff, so a TIFF gets the "not still" message, and the upload takes no check of the bytes. | Fixed (cb123b9: `read_photo` sniffs the first 16 bytes and refuses anything but a still image before reading the rest; a TIFF is sent as a JPEG through the new `pg_render::transcode::to_jpeg`, and one that won't decode is `SearchError::Undecodable`) |
| C-11 | low | `crates/app/src/yandex.rs:296-300, 359, 427` | A failed write leaves a partial file at the photo's name. `Search::start` and `save` use `thread::spawn`, which panics if the spawn fails and leaves `busy` set. | Fixed (cb123b9: `write_new` writes a synced `.<name>.pg-save` beside the target, claims the name with `create_new` and renames over the claim, removing both on a failure; `start` and `save` spawn through `spawn_worker` (`thread::Builder`) and return the spawn error, which clears `busy` and which the desktop logs and shows as `yandex-not-started`) |
| C-12 | low | `crates/catalog/src/browse/read/grid.rs:60-61` | One unknown extension in the catalog fails the whole grid list. | Fixed (cb123b9: `Catalog::prepare` marks missing the present rows whose extension this build does not hold, so every read leaves them out; queued writes on them are dropped as for any missing file) |
| C-13 | low | `crates/app/tests/listing_laws.rs:30-39` | `key()` leaves out `SortKey::Borders`, so that sort isn't covered by the laws. | Fixed (cb123b9: each case saves and reads back the listing for every `SortKey::iter()` key, both ways; `strum` is a pg-app dev-dependency) |
| C-14 | low | `crates/app/src/app.rs:189`, `crates/client/src/controller.rs:58`, `crates/app/src/edit.rs:2`, `crates/client/tests/replay.rs:2`, `crates/catalog/src/lib.rs:1507`, `crates/cli/src/lib.rs:151-152` | Comment slips. `app.rs` says a thread ends once the cache is gone, but the cache is leaked; `cli/src/lib.rs` says the same. `controller.rs:58` has a misplaced doc comment. `edit.rs:2` says "server's preferences". `replay.rs:2` refers to itself. `catalog/src/lib.rs` says "as Windows spells it". | Fixed (cb123b9: the comments reworded, the cli's to say its thread ends when a dropped cache takes the bus and runs to the end for the leaked one; `Panel`, `Size` and `Listing` have their own docs. `replay.rs:2` names `session.jsonl`, not itself: nothing to change) |
| C-15 | medium | `crates/desktop/qml/Viewer.qml:349-350, 377`, `crates/desktop/src/viewport.rs` | The fitted image isn't rendered again when the window is resized while the viewer is open. `fit_px` depends only on the window, but `fitSource` is a plain invokable, so the binding depends only on `viewer.visible` and `viewer.key`. The image keeps its old pixels and size until the next step, despite the "a fitted view follows it" comment at `Viewer.qml:205`. | Fixed (cb123b9: `Viewport` has `fitWidth`/`fitHeight` (NOTIFY) and `fitSource(key, w, h)`; the viewer renders the fit at a size it settles on 120 ms after a resize (at once on opening or another item), and follows a pixel-ratio change; a `tst_viewer.qml` case) |
| C-16 | low | `tools/compare-shots.ps1:87`, `crates/desktop/src/command_line.rs:69`, `crates/desktop/qml/TestHooks.qml:17, 25`, `crates/README.md:348` | `parse` keeps only `--name=value` arguments (its test drops `--flag`), so a bare `--borders` does nothing. The shot scenario `borders` passes exactly that, so it doesn't show the borders. The README's flag list has `--borders=1`, but its prose says `--borders`. | Fixed (cb123b9, 262004b: the `borders` shot scenario, `TestHooks.qml`'s list and prose and the README's prose say `--borders=1`; pending: the shot references are not recorded again yet, see Verification) |
| C-17 | low | `crates/desktop/cpp/screenshot.cpp:135-140` | The poll returns while the window isn't `ready`, before it reads `timed_out`. A window that never gets ready polls forever, though the header says `max_ms` is the most the hook waits and that such a window is saved anyway. | Fixed (cb123b9: the poll reads the timeout first; a window never ready is saved at the timeout, and with no window the log is written with no image and the app exits 1, a status `run()` now returns and `main` exits with) |
| C-18 | low | `crates/desktop/src/folders.rs:182`, `crates/desktop/src/library.rs:283-284, 312, 329`, `crates/desktop/src/yandex.rs:141, 187, 191-192, 222-223` | User-visible words bypass `pg_client::text`: "All folders", "Background work paused", "Paused · …", "… is already in the library", and the Yandex save and start lines. `ui.ftl` already has `ui-all-folders`. The `--locale=pseudo` scenario doesn't cover these. | Fixed (cb123b9: the library, Yandex, start and crash-box lines are `Message` variants with `text.ftl` entries and exact-string tests (plural 1/n); `App::add_folder` returns `Error::FolderRefused`, and `YandexError` and `Notice::RoundFailed`/`CropFailed` are worded by variant; "All folders" was done in 4b; compare-shots `$Pseudo` gains `pseudo-library`) |
| C-19 | low | `crates/desktop/src/library.rs:204-209` | When `App::library` fails, the reason only goes to `gallery.log`. The window looks idle, and nothing is ever scanned or imported. | Fixed (cb123b9: a failed `App::library` shows `text::library_unavailable` as a warning in the panel, and a folder list that cannot be read shows its own `text::folders_unreadable`; the `folders.rs` ROLE_NAME arms hold no English) |
| C-20 | medium | `crates/desktop/qml/ImageLayers.qml:31`, `crates/desktop/qml/Viewer.qml:115-123, 480`, `crates/desktop/qml/GridCell.qml:40`, `crates/desktop/src/gallery.rs:196-199, 287, 434, 845` | Dead code. `goTo` and `Hud.filePath` are already gone. The `fullImage` alias has no user. `Viewer`'s `fitted`, `fitZoom`, `minZoom`, `dw`, `dh` and `pixels` are read by nothing. `GridCell.key` is read by nothing, so `ROLE_KEYS` and `shownKeys` are built per cell for no reader. `ratingInFile` has no caller, and most of `GalleryModel`'s other invokables are only called from Rust. | Fixed (cb123b9: `fullImage`, the Viewer aliases, Viewport's `fitted`/`fitZoom`/`minZoom`, `GridCell.key`/`itemKey`, `GridRow` `keys`/`shownKeys`, `ROLE_KEYS` and `ratingInFile` are deleted; the Rust-only model methods left the bridge; REVIEW R-77 updated) |
| C-21 | low | `crates/desktop/src/facet_choices.rs:116`, `crates/desktop/src/command_line.rs:1-2`, `crates/desktop/src/screenshot.rs:5-6`, `crates/desktop/qml/YandexPanel.qml:95-96`, `crates/codec/src/codecs.rs:402`, `crates/README.md:348` | Comment drift. `facet`'s default is 0, which is `Types`, not "out of range". The hooks are in `TestHooks.qml`, not `Main.qml`. The wheel steps the viewer, it doesn't zoom it. The tiff decoder refuses an empty image (the guard is the tiff fork's, not `image`'s), so the "empty image" case can't happen. The README says `--wait` is 4 s and counts from the window showing, but it is 10 s from the call. | Fixed (cb123b9: the comments and the README say what the code does; the `folders.rs` items needed nothing) |
| C-22 | low | `crates/desktop/src/yandex.rs:232-244` (`file_url`) | `file_url` hand-escaped a path, and a UNC path came out as `file://///server/…`. | Fixed (7e2d444, 014865e: paths are typed `PathBuf` end to end, and `file_url` is gone) |
| C-23 | medium | `crates/model/src/path.rs`, `crates/model/src/rules.rs:218-250`, `crates/catalog/src/browse/read.rs`, `crates/catalog/src/lib.rs:528` | The scan stored folder keys from `PathRules`, while the grid's scope and the tree used a function that lowercased the whole component. A folder ending in Σ after a letter (`ΟΔΥΣΣΕΥΣ`) got a key range that matched none of its files, so the grid came up empty. | Fixed (278dfe8 deleted the string key functions; every key is now a `PathRules` key. cb123b9 added a scan-to-grid regression test under both rules in `crates/scan`, and fixed the two stale comments) |
| C-24 | medium | `crates/edit/src/rewrite.rs:384, 398, 419`, `crates/writeback/src/lib.rs:121-137`, `crates/catalog/src/writes.rs:131-133`, `crates/cli/src/lib.rs:464-470` | `weezl` 0.2.1 (the version `pg-edit` uses) panics in its decoder and encoder for a minimum code size above 12. The byte comes straight from the file and `gif_blocks` doesn't check it, so turning or cropping a damaged GIF panics. On the writer thread nothing catches it: the thread ends, no failure is recorded, and the job stays the oldest in the queue. Every later start dies on the same job, and `gallery write` loops forever. (The finding's "below 2" for the encoder is wrong: weezl 0.2.1 accepts 0 and 1.) | Fixed (cb123b9: `rewrite::gif` refuses a code size outside 1..=11 and decodes into a buffer held to the frame; the writer's `drain` guards each job, and `gallery write` stops when `Writer::is_running` is false) |
| C-25 | low | `crates/metadata/src/lib.rs:401-410, 626-653`, `crates/edit/src/rotate.rs:43-59` | The orientation reader and patcher accept different tags. `Exif::read` takes a BYTE, a LONG or a SHORT array, and the last of a repeated tag. `patch_orientation` takes only the first entry, and only a single SHORT. `rotate` turns the pixels by the orientation it read, then `unwrap_or(out)` keeps the old tag when the patch returns `None`. So a JPEG with a BYTE or LONG orientation of 6 ends up turned twice. | Fixed (cb123b9: the reader and the patcher take the same IFD0 orientation entries (BYTE, SHORT, LONG, LONG8 in BigTIFF), the patcher sets each at its own width where its value is, and `rotate` fails if the turned file's tag still reads other than 1) |
| C-26 | low | `crates/edit/src/rewrite.rs:60-77`, `crates/model/src/orientation.rs:131-142` | `Op::frame` passes `left + w - 1` to `shown_pixel`. That underflows for a 0-size frame, and `shown` underflows for a frame past the logical screen, which real GIFs have. There is no `overflow-checks` setting, so a debug build panics, and a release build wraps and places the frame by the wrong corner. | Fixed (cb123b9: `Op::frame` clips a turned frame to the screen once, and a frame with no pixels or off the screen keeps nothing; `shown_pixel` asserts its precondition in debug builds) |
| C-27 | medium | `crates/jobs/src/panics.rs`, `crates/jobs/src/pipeline.rs`, `crates/desktop/src/panic_log.rs:25, 32`, `crates/library/src/runner.rs:313-322` | `CAUGHT` is set only on the job thread, so a panic in a pipeline worker or reader counts as uncaught. The hook shows a blocking box on that worker, then `thread::scope` re-panics on the job thread as `RoundPanicked`. Nothing catches per file, so the file stays pending and does this every round. | Fixed (cb123b9: `panics::catch` nests, and the pipeline guards `read`, `init` and `process` per file, so a panic is that file's `Panicked` error; a thumbnail's panic doesn't rewind the checkpoint). The test seam is Decision 10. |
| C-28 | low | `crates/import/src/lib.rs:88-127`, `crates/library/src/borders.rs:53-72`, `crates/library/src/crop.rs:89-129`, `crates/scan/src/lib.rs:234-340`, `crates/thumbs/src/lib.rs:67-156` | `?` on `pipeline::run` or a catalog call skips `ctl.finish()`, and the runner never finishes the phase after a failed or panicked round. The phase stays non-idle, so `busy` stays true on an idle job. | Fixed (cb123b9: `Control::begin` returns a `#[must_use]` `PhaseGuard` whose drop finishes the phase, on `?` and on unwind) |
| C-29 | low | `crates/jobs/src/pipeline.rs:107`, `crates/jobs/src/control.rs:145-156` | Readers block in `ctl.checkpoint()` while paused and don't see the pipeline's own `stop`. A sink error during a pause holds `thread::scope` until the user resumes. | Fixed (cb123b9: readers wait in `Control::checkpoint_until(&stop)`, and the pass's end sets its stop and calls `Control::wake`, both under the pause lock) |
| C-30 | low | `crates/library/src/crop.rs:162-168` | A failed `fs::write`, `keep_time` or `rename` leaves the `.pg-crop` temporary behind. (The `place_of` doc was checked: a Unix anchor is always empty, so the doc is right.) | Fixed (cb123b9: `crop::publish` arms a `pg_platform::file::TempFile` guard before the write and persists it only after the rename, so any failure removes the temporary) |
| C-31 | low | `crates/edit/src/crop.rs:9`, `crates/library/src/borders.rs:31-47`, `crates/exiftool/src/lib.rs:191-214` | Three small slips. "TIFF losslessly" isn't true for a CMYK TIFF (it goes through `image` to RGB) or a BigTIFF (it comes out classic). `stats.files` counts files whose read failed and stored nothing, so `imported()` fires anyway. `pump` uses `thread::spawn`, which panics if the spawn fails. | Fixed (cb123b9: 8- and 16-bit CMYK and RGB TIFFs are cropped in their own samples and kind by the tiff encoder, and the crop doc lists what other TIFFs lose; `borders::Stats::stored` counts what went into the catalog, used by the runner and the cli line; `pump` uses `thread::Builder` and `start` kills the child on a failure) |
| C-32 | low | `crates/metadata/src/lib.rs:550-561` | `MAX_INFLATED` is per packet, and `assembled` holds every packet of the file. The comment says the import won't grow to gigabytes, but a small PNG with many compressed `iTXt` chunks still can, at 16 MiB each. | Fixed (cb123b9: each compressed `iTXt` is inflated within what the file's earlier packets left of `MAX_INFLATED`; with none left it is absent) |
| C-33 | low | `crates/model/src/rules.rs:167-169` | The recursive scope's upper bound is the folder's key plus `0xFF`, and the comment covers only a name key. On Posix a subfolder whose raw name starts with byte `0xFF` has a folder key at or past the bound, so it is left out of its parent's recursive listing. | Fixed (cb123b9: `prefix_upper_bound` in `rules.rs`, `lo[..-1] + [0x02]` for a folder key; unit tests and a Posix law with a `0xFF` subfolder) |
| C-34 | low | `crates/model/src/ids.rs:22-24`, `crates/yandex/src/lib.rs:12, 26, 602`, `crates/model/src/rules.rs:443`, `crates/library/tests/borders.rs:14` | Small cleanups. `FileIdentity`'s doc says the device is truncated to 32 bits, and later that it is whole. `crawlers/yandex-reverse/search.py` isn't in this repository. `trim_seps` sits after a test module. `borders.rs:14` has a redundant `#[cfg(windows)]`. (`path::is_under` is already gone, and `Change` in `jobs/src/events.rs` is already above its tests; `PathRules::is_under` is live and stays.) | Fixed (cb123b9: the `FileIdentity` doc says the Unix volume is the whole device number; the yandex docs and test name say the crawlers monorepo is outside this repository; `trim_seps` is above the tests. `library/tests/borders.rs` had already lost its `cfg`, and `Change` needed nothing) |
| C-35 | medium | `crates/writeback/src/lib.rs:229-235, 295-298`, `crates/catalog/src/writes.rs:93-94, 118, 207, 240` | Finishing a temp ignores the value it carries. `process` passes `job.value` to `done`, but a rating or turn made after `record_temp` has already changed `job.value`, and `temp_value` is the value in the file. Say a locked file fails its copy at rating 3 and the user then sets 5. The retry copies 3 in, records that the file holds 5, and deletes the row, so 5 is never written. Turns lose the later turn the same way. | Fixed (cb123b9: a finishing copy is done with `Job { value: temp_value, .. }`, so a later rating or turn stays queued) |
| C-36 | medium | `crates/writeback/src/lib.rs:207-212, 360-369`, `crates/catalog/src/writes.rs:276-291` | `give_up` deletes the temp even when the copy failed part way. `copy_into` overwrites the original in place, so after a failed `write_all` (the disk filling up, a read error) the temp is the only whole copy of the image. Five tries take about 30 s, then it is deleted and the original stays damaged. Also, a temp is recorded before the copy starts, so "recorded" cannot tell an untouched original from a half-written one. | Fixed (cb123b9: temps are per kind, and recorded only once the original is open for the copy; at the cap the temp and its record are kept, the error says the file may be incomplete (`WriteError::CopyIncomplete`), and `retry_copies` tries it again at each start; `abandon_temp` is gone) |
| C-37 | low | `crates/writeback/src/lib.rs:229-232, 368` | `copy_into` deletes the temp before `done` commits. A crash between the two leaves `temp` set with no file. Each retry fails on `reading`, and after five the job is abandoned: the write is in the file, but the queue shows it as failing. | Fixed (cb123b9: the temp is deleted only after `done` commits, best-effort; a recorded temp that is missing completes the write) |
| C-38 | low | `crates/writeback/src/lib.rs:155-169`, `crates/client/src/text.rs:739-745`, `crates/client/locales/en/text.ftl:76-79` | The status can be `queued = 0` with an error: `drop_gone` empties the queue and then sets `error`, which stays until the next success. `text.rs` then shows "0 changes not yet in the files: A queued rating was dropped…", which is also English that bypasses `pg_client::text`. | Fixed (cb123b9: a drop is counted in `WriteStatus::dropped` and `last_dropped`, clears the error when `error_file` names it, and the desktop notes it once (`text::write_dropped`); `(0, Some(e))` reads as `write-queue-failed`) |
| C-39 | low | `crates/writeback/src/lib.rs:150, 242-248, 295-301` | Two small gaps. The settle wait is a plain `sleep`, so `stop` waits up to 2 s for it. The file is read before `before` is taken and isn't compared again before `copy_into`, so another program's save in that window is overwritten without notice. | Fixed (cb123b9: the settle wait is `waker.wait`, so a stop ends it; `before` is taken before the read and compared on the handle `pg_platform::file::open_for_copy` opens, denying other writers during the copy) |
| C-40 | low | `crates/scan/src/lib.rs:272, 327`, `crates/catalog/src/pair.rs:160` | Every reparse folder goes into `unreadable`, so `clean` is false on every scan of a root that holds a junction or a symlinked folder. Tier 2, replacement and the clearing of `unpaired` then never run for that root, and the app doesn't say so. | Fixed (cb123b9: `pair` takes `Coverage { complete, skipped }`; on a complete scan tier 2, replacement and clearing run outside the skipped folders, on the fresh rows this scan saw; `docs/S17_7_PLAN.md` decisions 4 and 9 updated). Deviation: `docs/S17_7_PLAN.md` says to extend `a_reparse_folder_is_not_descended_and_its_rows_are_kept` (`crates/scan/src/lib.rs:538-547`); the change went into the new test `a_complete_scan_with_a_reparse_folder_still_pairs_the_rest_of_the_root` (`crates/scan/src/lib.rs:550`) instead. No code change needed. |
| C-41 | low | `crates/platform/src/native.rs:1-3, 24-26, 119-127` | Only paths of 240 units or more get the verbatim form, so a short path goes through Win32 normalization, which removes a trailing `.` from a path part. NTFS can hold such a name (a folder copied from Linux or macOS, such as `Acme Inc.`). The scan lists it, then opens `Acme Inc`, so that folder fails or names another one. (Not run.) | Fixed (cb123b9: `NativePath::set` gives every plain absolute path the verbatim form, `set_plain` serves the volume calls, `native::verbatim` serves std; writeback and crop refuse a folder ending in a dot or a space with `AlteredFolder` / `WriteError::FolderRefused`, since ExifTool takes plain paths) |
| C-42 | low | `crates/thumbcache/src/lib.rs:375-396, 633-648, 685`, `crates/app/src/app.rs:84-89` | A damaged cache is never rebuilt. `judge` returns an error for a corrupt file and `Foreign` for a garbled header, so the app runs with `ThumbCache::disabled()` on every start, and the only sign is a "not stored" note. The file holds derived data only. | Fixed (cb123b9: `judge` maps `NotADatabase` and `DatabaseCorrupt` to `Found::Damaged`; `open_writer` moves the file aside to `.bad` with `pg_schema::move_aside` (the catalog's, moved there) and retries once, all under its lock, with `heal_if_due` inside it; `Event::StartedOver` carries a typed `StartOverReason`, the old file's path and a technical `detail` for logs; the command line and the desktop word it through `pg_client::text::Message::ThumbCacheStartedOver` (one `text.ftl` entry per reason, `thumb-cache-*`); body damage is found at the daily check) |
| C-43 | low | `crates/yandex/src/lib.rs:162, 256-270` | `claimed_format` refuses types that real servers send for real images: `image/jpg`, `image/pjpeg`, `image/x-png`, `binary/octet-stream`. Its own comment says the bytes have the last word, but they are never reached. The 30 s timeout also covers the whole request, so a large upload or download on a slow link fails, up to the 100 MB limit. | Fixed (cb123b9: `plausibly_image` allows the image types real servers send and the untyped ones, and refuses the rest before the body; the URL fallback is gone. The upload and the download get per-request phase, per-read/write and body timeouts; the agent's 30 s stays for the search calls) |
| C-44 | low | `eval/verify.py:226` | `resample=4 # LANCZOS` is `BOX` in Pillow: Lanczos is 1 (4 is OpenCV's `INTER_LANCZOS4`). Every working image is box-filtered, and a small one upscaled to `MIN_WORK_SIDE` comes out blocky. The verifier's recorded figures were measured that way. | Fixed (262004b: `verify.py` imports PIL and resamples with `Image.Resampling.LANCZOS`; the pre-fix verifier figures predate it, see docs/FINDINGS.md) |
| C-45 | low | `eval/verify.py:393, 502-503` | The effective-resolution blur divides by the scale where it should multiply. `ref_scale` is working px per original px, so canvas px per original px is `c * ref_scale`, not `c / ref_scale`. For a 200 px copy against a 4000 px original, the sharp image gets the larger `e` and the blurry copy gets the extra blur, the opposite of the comment. The noise thresholds were tuned on this. | Fixed (262004b: the effective-resolution blur multiplies by `ref_scale` / `other_scale`; the pre-fix figures predate it) |
| C-46 | low | `eval/jpeg-decoders/nvimgcodec_bench.py:36-45`, `eval/jpeg-decoders/gpu.py:39-49`, `docs/JPEG_DECODERS.md:46, 179-200, 223-226, 481-486` | Neither script sets `cudaDeviceScheduleBlockingSync`, though `nvjpeg.py:90-98` and the doc's lines 223-226 say spinning bills the GPU's time to the CPU. The doc concludes from `nvimgcodec_bench.py`'s CPU time, equal to wall time, that nvImageCodec "never used nvJPEG's GPU Huffman path". The saved runs don't support that conclusion (the blocking-sync run still shows CPU about 2x wall at batch 64). | Fixed (262004b, with the doc in cb123b9: each script carries its own `blocking_sync()` through `nvcuda.dll`, with return codes checked; docs/JPEG_DECODERS.md rewritten, not re-measured) |
| C-47 | low | `eval/jpeg-decoders/gpu.py:55` | `gpu.py` requests the ceiling size, `ceil(w/d)`, so Pillow's draft can pick a larger scale than `1/d` for a side that isn't a multiple of `d`. `nvjpeg.py:224` requests the floor, with a comment explaining why. | Fixed (262004b, with the doc in cb123b9: `gpu.py` asserts the ceiling size after `draft` (RuntimeError otherwise); docs/JPEG_DECODERS.md footnote rewritten, not re-run) |
| C-48 | low | `eval/jpeg-decoders/debesheedas.py:22`, `eval/jpeg-decoders/gpu_encoders.py:28`, `eval/jpeg-decoders/nvimgcodec_bench.py:31` | `read_ppm` uses `split(maxsplit=4)`, which skips whitespace before the last field. A picture whose first bytes are 9-13 or 32 (a dark first pixel) loses them, so `reshape` raises. | Fixed (working tree: each script reads PPM with PIL, np.ascontiguousarray(np.asarray(Image.open(path))); no shared reader module, which Decision 5 permits) |
| C-49 | low | `eval/jpeg-decoders/encoders.py:51-57, 119-121` | `png_of` runs on the thread pool and checks `exists()` before it writes. The four guetzli jobs of a file start together, so one can see a half-written PNG and fail, which stops `encode`. | Fixed (262004b: `png_of` writes a temp and renames it; PNGs are made serially, only for inputs guetzli will get) |
| C-50 | low | `eval/jpeg-decoders/benches/ir.rs:3, 31`, `eval/jpeg-decoders/src/lib.rs:115-142` | The header says setup isn't measured, but `Decoders::new` leaves FFmpeg's MJPEG decoders unopened, so `open_mjpeg` runs inside the one measured decode. `run` also takes `Decoders` by value, so its drop is counted too. | Fixed (262004b: FFmpeg MJPEG decoders opened eagerly in `Decoders::new`; `run` returns the decoders so their drop is outside the measured call; callgrind not run here) |
| C-51 | low | `eval/bench_ann.py:79`, `eval/accel.py:9`, `eval/bench_hdd.py:73, 165-167`, `eval/pdq.py:4`, `eval/verify.py:675`, `eval/jpeg-decoders/encoders.py:126`, `eval/jpeg-decoders/win-build.cmd:21`, `eval/jpeg-decoders/harness/harness.cpp:264`, `tools/perf-compare.py:6, 57`, `tools/shots/fixture.py:4-6`, `tools/profiling/wsl-setup.sh:50` | Comment drift, plus one code defect. `bench_hdd.py` says the blocks are "disjoint", but with `--dense` they overlap whenever dense > step (about `len < 45,000` at the default), so the code needs fixing, not just the comment. The other comments: the ANN cosines are uniform in 0.3-0.99, not "drawn from the real duplicate pairs"; PDQ's sample equals `synth.py`'s only if the two `ok` masks match; the speed header says Q75 but guetzli's column is Q84; `:clone` says shallow but clones whole; `perf-compare.py` says `callgrind.out` for `callgrind.*.out`; `fixture.py`'s docstring leaves out `settled`; `wsl-setup.sh` says `/mnt/c`, but the repo is on D:. | Fixed (262004b: `bench_hdd.py` refuses a `--dense` that would overlap, and the comments are reworded; `pdq.py` docstring corrected; `accel.py` names the TensorRT plan with its `_trt<N>` suffix and `harness.cpp`'s usage line lists `dump`, both found by the review ledger, L-04) |
| C-52 | low | `eval/bench_hdd.py:73`, `eval/bench_ann.py:263`, `eval/jpeg-decoders/debesheedas.py:36`, `eval/jpeg-decoders/harness/harness.cpp:126, 216`, `eval/jpeg-decoders/restart.sh:30-34`, `eval/verify_synth.py:93-99` | Small failures in eval tools. `open_file` prefixes `\\?\`, so a relative `--root` fails. `rerank` fails with fewer than K candidates. `debesheedas.py` runs with `cwd` set to its temp folder, so relative `FILE` and `--exe` paths don't resolve. The harness reads `args[1]` of a last flag with no value. `restart.sh`'s `ir` doesn't check valgrind, so a failed run prints `0.0` (`wsl.sh`'s prints `ERR`). In `verify_synth.py` three calls run outside `PairRunner`'s `try`, so one bad file stops the run. | Fixed (262004b: root resolved once; `rerank` pads with -1; harness argument checks; `restart.sh ir()` returns ERR; `verify_synth.py` caches per image; debesheedas uses absolute paths) |
| C-53 | medium | `eval/viewer.py:172, 204-214, 283-286, 306`, `eval/viewer.html:489-502, 717-727` | `set_marks` holds `marks_lock`, but `GET /api/marks` reads the file without it. On Windows `os.replace` fails while a reader has the target open. `do_POST` doesn't catch `PermissionError`, so the connection drops, `fetch` rejects, and `toggleMark` shows no alert: the mark looks unsaved with no message. `postMarks` has the same unhandled rejection. | Fixed (262004b: `viewer.py` reads marks unlocked under `marks_lock` in `set_marks`; `OSError` answered with 500; `viewer.html` `postJson` helper for both POSTs; `eval/test_viewer.py` and `just test-eval`) |
| C-54 | low | `tools/profiling/wsl-bench.sh:52-66`, `tools/perf-check.sh:14` | The archive is named by commit only, so a filtered run overwrites that commit's full archive with its subset. The header calls it "one archive per benched commit", and `PG_BENCH_SKIP_CATALOG=1` is such a run. | Fixed (262004b, with the doc in cb123b9: `wsl-bench.sh` skips the archive only for runs with a bench-name filter; docs/PROFILING.md updated) |
| C-55 | low | `tools/compare-shots.ps1:111-113, 121, 134, 208, 239, 426`, `tools/compare-shots.ps1:48` | Small slips. `.Trim()` on a failed `git rev-parse` throws first, so the "no commit" messages never show. `Build-Gallery` assigns to the automatic `$args`. `-Record -Only x` replaces the reference with that subset, and every other scenario then reads MISSING. The failure count includes the pseudo-locale, but the total doesn't. (The `$proc.ExitCode` claim did not reproduce in a probe, so it is not changed.) | Fixed (262004b: `$LASTEXITCODE` checks; `-Record`/`-Only` refused together; `$args` renamed `$cargoArgs`; total is `$report.Count`; `$null = $proc.Handle` dropped, no defect reproduced) |
| C-56 | medium | `crates/edit/src/rewrite.rs:230`, `crates/codec/src/codecs.rs:73-84, 483-488` | Turning or cropping a PNG allocates what its header declares, with no cap. The vendored `png` limit counts one row, and `output_buffer_size` only stops at `isize::MAX`. A damaged height makes `vec![0u8; n]` fail, which aborts the process: no panic hook, no error, no failure recorded. The job stays the oldest in the queue, so every start ends the app. The decode path caps at 512 MiB. (From reading; not run.) | Fixed (cb123b9: `pg_codec::max_decode_bytes` is the one cap; the PNG, WebP and GIF rewrites check their buffers (a GIF's screen and frame too) against it and allocate with `try_reserve_exact`; BMP is bounded by its file size) |
| C-57 | low | `crates/desktop/qml/MediaView.qml:34, 57, 69, 90`, `crates/desktop/src/gallery.rs:790-796`, `crates/model/src/grid.rs:74` | A video opened before its import has `fw` and `fh` 0, because the size comes from the catalog while the kind comes from the extension. `playScale` is then 0, so the `VideoOutput` is hidden while the player starts: sound and the transport with no picture until the import reaches the file. | Fixed (cb123b9: the video's box is shown for any video, at the view's size until the size is known, and `PreserveAspectFit` fits the frame; `tst_media_view.qml` cases) |
| C-58 | low | `crates/model/src/tree.rs:89-90, 110-117`, `crates/desktop/src/folders.rs:150` | Files directly in a drive root are in no tree node, so `total()` leaves them out. "All folders" shows a lower count than the grid it opens lists, and those files have no row of their own to select. | Fixed (cb123b9: `FolderTree::root_files` counts them and `total()` includes them; the tree shows a "Drive roots" row (folder `""` without subfolders, `OpenRoots`) when there are some, and "All" opens with subfolders on (`OpenAll`)) |

## Decisions

These were settled while the fixes were checked. Each one changes what gets built, so each is recorded here.

**Status:** the fixes are committed: cb123b9 holds most of them, and 262004b holds the eval and tools fixes (C-44 to
C-55). Each row names its commit. Four fixes have edits in the working tree that are not committed yet (C-15, C-26,
C-30 and C-43), so their rows name the commit that first holds the fix. The pending checks are under "Verification"
and "Not run". nvImageCodec is kept for now, by decision of the user; see the eval notes in `JPEG_DECODERS.md`.

1. **C-36 temp names are per kind** (`.rating` and `.turns`), not a per-file queue serialization. This keeps the queue contract and fixes the shared-temp collision at its source. *Confirmed by the user.*
2. **C-2(d) is dropped.** The finding's premise was false: `known` None means "the file held no rating when last synced", and the existing three-way sync test adopts an external change. The queued-rating guard covers the finding's case. *Confirmed by the user.*
3. **C-39: a sharing violation on an existing folder is a counted attempt.** The job waits at the cap for the next change. The alternative, uncounted waiting, can loop forever on a file another program holds open. *Confirmed by the user.*
4. **C-41: writeback and crop refuse folders whose names end in a dot or space**, with a typed error. ExifTool cannot take `\\?\` paths, so ExifTool-bound jobs get the same refusal. This is a limit with a refusal, not a silent write to a sibling photo. *Confirmed by the user.*
5. **Python isolation: the eval scripts will drop `python -I`, and print their package versions instead.** `-I` was added without a recorded reason. Its only effect that matters here is to keep the script's folder off `sys.path`, which breaks sibling imports. The eval venv already keeps out the user site-packages and any system packages, and `PYTHONPATH` is unset, so the flag guards against nothing this setup uses. What actually ties a number to its environment is the package versions that produced it. Each script runs from its venv (`venv-eval` for torch and nvJPEG, the nvImageCodec venv for `nvimgcodec_bench.py`) and prints the versions of numpy, Pillow, torch and nvImageCodec at the top of its output. Shared helper modules are allowed, since the folder is on `sys.path`. Decision 5 supersedes the earlier "no shared Python modules" choice. *Confirmed by the user.*
6. **C-46: the nvImageCodec and nvJPEG pins are settled.** `eval/jpeg-decoders/requirements.txt` pins `nvidia-nvimgcodec-cu13` at 0.10.0.9, the version the numbers were measured with, and `nvidia-nvjpeg` at 13.3.2.43, the latest release, so both share one env with torch. The same file also moves `nvidia-libnvcomp-cu13` from 5.3.0.16 to 6.0.0.12, a major version bump made in commit 314a99c (outside this review's range; the plan left the pins to the user). The user kept the current pins, libnvcomp 6.0.0.12 included. The GPU figures in `JPEG_DECODERS.md` were measured on nvJPEG 13.2.3 and need a rerun on 13.3.2.43. *Confirmed by the user.*
   Still open: the `nvjpeg64_13.dll` note under C-46 (an unpinned `nvidia-nvimgcodec` replaced the DLL that `venv-eval` shares). The pin is in place, but no check of the installed DLL is recorded here, so the note stays open.
7. **C-55: the `$proc.ExitCode` change is dropped**, because the claim did not reproduce in a probe. *Confirmed by the user.*
8. **C-50** opens the four FFmpeg MJPEG decoders eagerly in `Decoders::new`, and the bench returns the input so its drop falls outside the measured call. *Confirmed by the user.*
9. **C-12: unheld extensions are marked missing at open**, rather than skipped at each reader. Their queued writes are then dropped by the existing `file_gone` path; see Behaviour changes. *Confirmed by the user.*
10. **C-27 test seam: the fault seam is gated on `any(test, feature = "fault")`, not `cfg(test)`.** The plan asked for a `#[cfg(test)]` seam that makes `process` panic for one `FileId`. `cfg(test)` is set only for the crate under test, so a test in `pg-cli` or `pg-library` could not reach it. The seam is gated as `any(test, feature = "fault")` (`crates/writeback/src/lib.rs:130`, and the other gates in that file), and `Writer::start_with_fault` (`:175`) is its entry point. The `fault` feature is defined in `crates/writeback/Cargo.toml` and enabled by the dev-dependencies of `pg-cli` (`crates/cli/Cargo.toml:31`) and `pg-library` (`crates/library/Cargo.toml:38`). This is a deviation from the plan, recorded here; it is not confirmed by the user.

## Conventions for the fixes

- **Panic-freedom:** the workspace's clippy lints deny `unwrap`, `expect`, indexing, `panic` and the cast lints in non-test code. Propagate `Result`.
- **User-visible text:** every user-visible Rust string is a `pg_client::text::Message` variant with an entry in
  `crates/client/locales/en/text.ftl`. `text.rs` tests check ids against entries. `ui.ftl` ids must be asked for by QML
  (`words.rs` test), so Rust-only strings go in `text.ftl`. Crates below the client (writeback, jobs) cannot build a
  `Message`; they return typed errors that the client renders. `text::render` needs no Qt, so the desktop can use it.
- **QML properties from Rust** are declared FINAL, and NOTIFY when QML binds to them (`just lint-qml`).
- **Layering:** Win32 calls live in `pg-platform` (`crates/README.md:11-12`).
- **Schema:** `MIGRATIONS` holds migration 2 (`TO_2_CHANGE_STAMPS`, `schema.rs:25`: the C-8 triggers), and the first migration freezes `BASELINE` (comment at `schema.rs:15-22`).
  `Catalog::prepare` compares an existing catalog with baseline + the migrations up to its version. So **every schema
  change in this review goes into ONE migration (catalog version 2), never into `BASELINE`.** Comment-only edits to
  `BASELINE` are safe. Migration 2's only content is C-8's triggers. The places that say "no migrations" are updated:
  `schema.rs`'s doc comment, `crates/README.md:~186`, `docs/S17_7_PLAN.md`, and the comment at `analyze.rs:27`.
  `docs/DATABASE.md` has no such statement. `CARRIED` lists BASELINE tables only, so it needs no change.
- **Working order:** implement everything, then build and test once at the end. Commit on `main`; do not create a
  branch.

## Fixes

Grouped by work package, in dependency order. Each finding's fix is final; where the review changed a fix, the
corrected version is what is written.

### 1. Panic and phase infrastructure (C-24, C-27, C-28, C-29)

Everything else depends on this package.

- **C-27.** In `jobs/src/panics.rs`, `catch` saves and restores the previous `CAUGHT` flag, and a new `guard(f) ->
  anyhow::Result<T>` is built **on `catch`**. A raw `catch_unwind` would leave `CAUGHT` false, so the panic hook would
  show the blocking box. `Panicked(String)` has a payload-only `Display`; the words a user sees ("a panic in this file")
  come from a `Message` variant.
  In `jobs/src/pipeline.rs`:
  - Wrap `init` and `process` in `guard`. After a panic, rebuild the worker's state. If the rebuild (or `init`) fails,
    send `(usize::MAX, Err)` like the setup failure, so the pass fails instead of skipping items.
  - Wrap `read` too. On a panic, or a read that returns false, still call `pool.pass(i)` and send `Err` on `raw_tx`.
    Without `pool.pass(i)` the in-order chain (`Pool::take`) never advances and the pass hangs.
  - Count the results after the sink loop, and return an error if fewer than the indices sent arrive. A worker that
    exits without a message would otherwise let the pass return `Ok` with items skipped.
  - Per-file effect: import → `store_failure` (persisted); borders → "no borders" (stored); crop → a failed item, not
    requeued.
  - **Thumbs:** a `Panicked` error does not set `first_failed`. That error rewinds the checkpoint
    (`thumbcache/src/lib.rs:543-547`), so a deterministic panic reran and wrote a backtrace every pass. The panicking
    file is counted as failed and kept out of the rewind; it is retried only when it changes.
- **C-29.** `Control::wake(&self)` locks `self.paused` (`unwrap_or_else(PoisonError::into_inner)`) and calls
  `resumed.notify_all()`, the same pattern as `request_stop` (`control.rs:105-109`). `checkpoint_until(&self, stop:
  &AtomicBool) -> bool` checks `ctl.stop_requested()` and `stop.load()` **while holding the paused lock**, then waits.
  `StopOnDrop::drop` stores the local flag, then calls `ctl.wake()`. Without the lock a stop can be lost and the scope
  hangs when a sink error happens during a pause.
- **C-28.** `PhaseGuard` is `#[must_use]`. `begin()` returns it; its `Drop` calls a private `finish`. Every call site
  becomes `let _phase = ctl.begin(..);`: `import/src/lib.rs`, `library/src/borders.rs`, `library/src/crop.rs`,
  `scan/src/lib.rs`, `thumbs/src/lib.rs`, `cli/src/borders.rs`, and the tests. In `thumbs/src/lib.rs`, bind the guard in
  a block that ends after the last flush, so `Finished` still fires before `cache.checked` and `cache.shrink` and the
  desktop's busy flag behaves as it does today. No extra finish in `runner.rs::round_caught`: the guard's `Drop` runs on
  unwind as well as on `?`.
- **C-24.** In `edit/src/rewrite.rs::gif`, `ensure!((1..=11).contains(&min_code), "damaged GIF: …")` runs after
  `min_code` is read (`:384`) and before the weezl decoder (`:398`) and encoder (`:419`) are built. The range is the one
  the app's own GIF decoder accepts: the `gif` fork's `LzwReader::check_code_size` refuses `> 11 || < 1`, so the
  rewriter never rewrites a GIF the viewer can't show. A probe confirmed that weezl 0.2.1 panics only above 12. Replace
  the unbounded `decode` with `decode_bytes` into a buffer capped at `w*h`, require `LzwStatus::Done` (the existing
  `decode()` requires the end marker), and handle `w*h == 0`. The frame's `w` and `h` are u16, so without the cap the
  buffer could reach about 4 GiB from a few KB of file.
- **C-24 (writer).** The writer's drain guard is per job, not per thread: `writeback/src/lib.rs::drain` wraps `process`
  in `guard`, and a panic takes the normal failure path (`writes::failed`, drop exiftool, `give_up` at the cap). A panic
  **outside** `process` (in `next_ready`, `remaining` or `attemptable`) is not caught: the writer thread ends, and
  `Service::is_running()` (a new accessor, since `Shared` is private to `pg_jobs::service`) reports it. The `gallery write`
  loop in `cli/src/lib.rs` (`Command::Write`, `:464-470`) breaks with an error when the writer is no longer running. A
  guard that kept the thread alive would let the loop spin, because `left_to_write` still counts the row.
- **Tests:** a pipeline whose item 3 panics completes with that item as `Err` (read and process variants), under a test
  timeout so a regression fails instead of hanging; a sink error during a pause returns promptly with a second reader
  waiting; a dropped `PhaseGuard` leaves the phase Idle and `Finished`; bad-code-size GIFs (the image descriptor's byte of
  `pg_testkit::gif_rgb` set to 0, 12 and 13, since the testkit writes a full-screen frame) → `Err`, as a unit test in
  `edit` (not a fuzz seed, because `fuzz/corpus` is gitignored); a `#[cfg(test)]` seam that makes `process` panic for one
  `FileId`, showing that the writer survives it; and a drain-level panic that makes `gallery write` return.

### 2. Writeback (C-2, C-35, C-36, C-37, C-38, C-39)

- **C-35.** Finish a temp with the value it carries: `Job { value: temp_value, ..job.clone() }`, so `done` leaves a later
  rating or turn queued. Tests: in `catalog/src/writes.rs`, a rotation with `record_temp` at 3, a re-queue, then `done`
  with the temp value leaves the remainder queued with attempts 0; a rating 3→5 after the copy. In
  `library/tests/writeback.rs`, `queue_finished_copy` then `set_rating(5)` ends with the file at 5.
- **C-36 and C-37: the copy.** Today `record_temp` runs before the copy, so "temp recorded" can mean an untouched original
  (the open failed) or a half-written one. Move it, so a recorded temp always means "the copy may have started". A fresh
  job's `process`:
  1. `before = stat(path)`, taken **before** reading. Read the file, make the temp (per kind, below), and compute the
     metadata-only hash from the temp's bytes.
  2. `pg_platform::file::open_for_copy(path)`, a new function beside `open_read` in `platform/src/file.rs`. On Windows it
     calls `CreateFileW` with `GENERIC_WRITE`, `FILE_SHARE_READ` and `OPEN_EXISTING`; on Unix, an `OpenOptions` write.
     **Deviation (L-12), as built:** the Windows open requests `GENERIC_WRITE | FILE_READ_ATTRIBUTES`, not
     `GENERIC_WRITE` alone. The extra right is what lets `File::metadata` read the size and write time from the handle
     (`crates/platform/src/file.rs:49-50`). The share mode and disposition are as stated, and the Unix open is unchanged.
     Size and mtime are read from **that handle** (`last_write_time`) and compared with `before`, through a pure
     function over `(before, now)` so it can be tested. A mismatch deletes the temp and fails with "… changed while its
     change was being prepared"; that counts as an attempt, and the retry re-reads the file. An open failure deletes the
     temp too, so nothing is recorded and the retry starts from the file.
  3. `sync_all` the temp, before it is recorded: a complete temp must be durable before the original is overwritten
     from it.
  4. Only now `writes::record_temp`.
  5. `write_into` the open handle (`write_all`, `set_len`, `sync_all`). Drop the handle before any later `stat`.
  6. `done`.
  7. Delete the temp after `done` commits, best-effort (`let _ =`). A failure here must not turn a successful write into
     a counted failure. A leftover temp with no record is cleared by the next write of that file.

  The result: a recorded temp always means "the copy may have started". No `copying` column is added, and migration 2
  holds only C-8's triggers.
  - **Temp names are per kind.** `temp_of` takes the kind and uses `with_affixes` (`.rating.pg-write` and
    `.turns.pg-write`). The schema allows a rating row and a turns row for one file (`PRIMARY KEY (file_id, kind)`), and
    both used one temp path: a fresh turns job ran `remove_file` on the rating's complete temp, and C-37's rule would then
    mark the rating done though the original was never copied in. Rows store the actual temp bytes, and `next_ready`
    parses them, so rows from before the fix keep their names and need no migration.
  - **`give_up` at the cap keeps the temp and its record.** The file keeps reading as being written (`FileState::writing`,
    so the import skips it, since it may be part-written). The error says the original may be incomplete and names the
    temp, through a `Message` variant. `abandon_temp` is deleted. A later change to the file is taken first as a finishing
    copy and repairs it.
  - **Reset rules.** `queue_rotation` and `set_rating`'s write-back branch reset `attempts`; `set_rating`'s else branch
    (the one that only updates `value`) does not. It also resets `attempts` for a row with a temp, so a later change
    retries a stuck copy as the docs say. `retry_copies` resets `attempts` of rows with a temp on the writer's first
    catalog open in `run`, so each start tries a stuck copy again.
  - **Finishing copies** (record set) open with the same deny-write mode and do not compare, because the original may
    already be partly ours. They copy the temp in, call `done` with `before = (-1,-1)`, then remove the temp.
  - **A missing recorded temp** means an earlier build removed it, or another program did before the copy began. The
    second case cannot be detected, and is listed under Behaviour changes. The handling: skip the copy, stat the original,
    and call `done` with `before = (-1,-1)`.
  - **Offline classification.** Writeback currently treats a fresh job's `PermissionDenied` as offline and uncounted, so a
    read-only original retries forever. The rule becomes `PermissionDenied | NotFound => !folder_present(..)`, with no
    `job.temp.is_none()` guard. A read-only original on an existing folder is therefore counted and reaches the cap.
  - **Docs** that state the old order are reworded: the writeback module header, `record_temp`'s and `writes.rs`'s module
    docs, `not_being_written!` (`catalog/src/lib.rs:1048`; its doc already says "before the file is rewritten in place",
    so only the temp description changes), and the `file_writes` comment in `BASELINE` (`schema.rs:433-437`).
  - **Tests:** rewrite `a_finishing_copy_that_keeps_failing_is_given_up` (the temp and its record are kept at the cap,
    the error names the temp, the file still reads as writing; after the lock is dropped and the writer restarted, the
    copy completes); a fresh job whose open fails leaves no temp and no record; a file changed between read and copy is
    retried, not overwritten; a missing recorded temp completes; a restarted writer resets a stuck copy's attempts; a
    read-only original is counted and reaches the cap; a rating stuck at the cap plus `queue_rotation` on the same file
    leaves the rating's temp and record intact and the rating unwritten.
- **C-38.** `WriteStatus` (`writeback/src/lib.rs:57`, derive at 56) gets `dropped: usize`, `last_dropped: Option<WriteKind>` and
  `error_file: Option<FileId>`, which is set with `error` in the failure arm. The drop arm bumps the counters and
  **clears `error` when `error_file` is the dropped job's**. `desktop/src/library.rs::refresh_writes` keeps a
  `dropped_seen` counter and, when `status.dropped` grows, shows `text::write_dropped(kind, n)` once with `show(…, true)`.
  New `Message::WriteDropped{kind,count}` (with a `write-dropped` entry in `text.ftl`). The `(0, Some(e))` case becomes
  `write-queue-failed`. A drop does not light `write_failed`. Tests: the drop test,
  `a_write_for_a_file_that_is_gone_is_dropped_and_a_deleted_one_is_counted` in `library/tests/writeback.rs`, asserts `dropped == 1` and that the error is cleared when it named the dropped job.
- **C-39.** The settle wait becomes `waker.wait(Some(dur))`, which returns on `Woke::Quit`. `before` is taken before the
  read (see C-36). The compare and the deny-write open are the C-36 copy above. Behaviour change: other programs can't
  write the photo during the in-place copy, so their save fails or retries (see Behaviour changes).
- **C-2.** The `AnalysisWriter`'s write-back flag is not added: write-back is an app preference, unknown to the import
  job, and ratings made with write-back off are catalog-only by design.
  - (a) `catalog/src/writes.rs::set_rating` reads `files.kind` and `analysis.format` (with `error IS NULL`) in one query,
    and queues the rating when `rating_in_file_possible(kind, format)` holds. The helper is `kind == Kind::Image.key() &&
    format.and_then(Format::from_name).is_none_or(pg_metadata::can_hold_rating)`: kind first, so videos are excluded.
    `details.rs:40` uses the same helper, and the set_rating doc at `writes.rs:63-65` is rewritten. Analysis-failed rows
    are not queued.
  - (b) For `WriteKind::Rating` only, when the catalog format is None and the sniffed format is None, or the format can't
    hold a rating, `writeback::process` calls `writes::done(.., WriteEffect::Nothing)` and returns `Ok` **before** the
    `.context("unknown format")?` at `lib.rs:247-252`. Turns keep the error. Without this, a rating on a non-image `.jpg`
    fails every attempt and leaves a permanent "unknown format" warning.
  - (c) The writeback comment at `lib.rs:220-221` (video ratings "never queued") is corrected to the kind-first rule.
  - (d) Dropped; see Decisions.
  - Tests: a row queued with no analysis; a BMP rating dropped as done; an analysis-failed non-image `.jpg` rating leaves
    the queue with no error; a video rating is never queued; rate-then-import keeps the catalog rating.

### 3. Edit and metadata (C-25, C-26, C-30, C-31, C-32, C-56)

- **C-25.** One helper in `metadata/src/lib.rs` yields `(data range, entry)` for each IFD0 orientation entry of a kind
  `Tiff::uint` reads: BYTE, SHORT and LONG, and LONG8 in BigTIFF. The data start is factored out of `Tiff::value`
  (`lib.rs` ~342-357), which follows the offset for out-of-line values. Do not use `value_at` (`lib.rs:336-338`), which
  would overwrite a pointer field. `Exif::read` takes the last readable entry over the same set, so the reader and the
  patcher agree. The patch writes each entry at its own width, in the file's byte order.
  `edit/src/rotate.rs::rewritten` becomes fallible (`-> Result<Outcome>`; its closure is typed `-> Outcome` at `:43-46`),
  with `?` on the three calls. After the patch it checks the end state:
  `ensure!(pg_metadata::orientation(&out, format) == Orientation::UPRIGHT, …)`. A `None` because the rewrite left no
  tag is correct (nothing to reset). Any tag that still reads non-1 is an error, not a double turn.
  Tests: a BYTE, LONG and repeated-tag patch round trip (metadata ~l.845); a JPEG with a BYTE or LONG orientation of 6
  rotates once; an out-of-line LONG orientation patches correctly.
- **C-26.** Clip once, in `Op::frame` (`rewrite.rs:60-77`), not also in `rewrite::gif`. For both arms compute `x0 = left`,
  `x1 = min(left + w, sw)`, `y0 = top`, `y1 = min(top + h, sh)`, and return `None` when `x0 >= x1 || y0 >= y1`. For Orient,
  the placement is the componentwise min of `shown_pixel(x0, y0, sw, sh)` and `shown_pixel(x1 - 1, y1 - 1, sw, sh)`, the
  kept rectangle is `[x0 - left, y0 - top, x1 - x0, y1 - y0]`, and the new screen is `self.size(sw, sh)`. Crop keeps its
  own rectangle. `shown` is generic over `T` (`orientation.rs:131-133`), so it is not changed to return an `Option`; a
  `debug_assert` states its precondition. Clipping, not growing the screen, is right: `codec/src/codecs.rs::gif_image`
  places the frame into the logical screen as far as the image goes, so the turned file holds the pixels the user turned.
  Tests: a frame with `left > sw`; `w == 0`; a frame that sticks out past the screen.
- **C-56.** One cap for every decoded buffer: `pub fn max_decode_bytes() -> u64` in `crates/codec` (beside `reserve`,
  `codecs.rs:483-488`), equal to `Limits::default().max_alloc`, used by `codecs.rs:84` and by the rewrite paths:
  - PNG (`rewrite.rs:230`): check `reader.output_buffer_size()` against the cap before allocating, then allocate with
    `Vec::try_reserve_exact`. The PNG `Limits` count row buffers only, so `new_with_limits` has no effect here.
  - WebP (`rewrite.rs` ~302): the same check.
  - GIF: the logical screen (`sw * sh * 4`, as the app reserves) and each frame (`w * h`) before the weezl buffer, with
    `try_reserve_exact`.
  - BMP is not changed: its rewrite only proceeds when the stored bytes equal `stride * h`, so it is bounded by the file size.
  Tests: a PNG with a huge IHDR height → `Err`; a GIF with a huge logical screen → `Err`. Both are unit tests in
  `rotate.rs`/`rewrite.rs`, not fuzz seeds.
- **C-31.** Three parts.
  - `library/src/borders.rs` Stats gets `pub stored: usize`, incremented after `store_borders` succeeds. `files` and
    `failed` keep their meaning. The consumer is `cli/src/lib.rs:423-427`, which prints `borders looked for in {files}
    files ({failed} failed)`; it and `runner.rs:470` use `stored` where they mean "stored". `library/tests/borders.rs:40-47`
    asserts `stored`.
  - CMYK TIFF: keep the CMYK samples. Decode the raw CMYK with the `tiff` decoder (as `codec/src/codecs.rs` does for
    `Stored::Cmyk8/16`), crop the rows, and write CMYK8/CMYK16 with the `tiff` crate's encoder (`TiffEncoder::new` for
    classic, `new_big` for BigTIFF). No silent colour conversion. If that is judged too large, refuse a CMYK TIFF with a
    typed error on the pattern of `edit/src/crop.rs:46-52`, instead.
  - The `edit/src/crop.rs:9` doc lists what is not kept: 1-bit grey becomes 8-bit (`image`'s tiff.rs:61, 405-413), and
    palette and YCbCr TIFFs are refused (`tiff.rs:75-76, 193`).
  - `exiftool::pump` (`exiftool/src/lib.rs:191-214`) uses `thread::Builder` and returns `Result<Receiver<String>>`, and
    callers propagate. A `?` after the spawn kills and waits for the child (`run()` does that at `:74-77`). Update the pump
    test call at `:332`.
- **C-30.** `library/src/crop.rs` gets a plain-path `publish(temp, out, bytes, source) -> Result<()>` doing `fs::write`,
  `keep_time` and `rename`. A `TempFile` drop guard, in `crates/platform/src/file.rs`, is armed right after the temp path
  is computed, before `fs::write`. `persist()` disarms it only after `rename` succeeds. So a failed or partial write is
  covered too. Tests: unit tests on `publish` (a failed rename, and a failed write, each leave no temp), so they run on
  every platform; `library/tests/crop.rs` stays as the end-to-end check. The temp name is deterministic (`<out>.pg-crop`),
  so a crash's leftover is replaced by the next save, and no directory sweep is done.
- **C-32.** Pass the remaining budget, `MAX_INFLATED - assembled.len()`, into `inflate_zlib` for PNG `iTXt`. Once it is
  exhausted, further chunks are absent. Test: many compressed `iTXt` chunks keep `assembled.len() <= MAX_INFLATED`.

### 4. Catalog, model, scan and platform (C-4, C-5, C-6, C-8, C-12, C-23, C-33, C-34, C-40, C-41, C-58)

- **C-4.** `recent_errors` gets `ORDER BY queued DESC, file_id DESC, kind DESC`, so the shown set is fully deterministic.
  Its doc says `error` is set only by `failed()`, which stamps `queued`. Test: three error rows with one tie at the same
  second; the order and the limit are asserted.
- **C-5.** `unpaired_still(&self, tx) -> Result<bool>`, via `prepare_cached(..)?.query_row(..).optional()?`, returning
  `Ok(unpaired == Some(1))`, so a missing row is false, not an error. The lazy `.filter()` chains at `pair.rs:100` and
  `:135`, and the count at `:154`, become `for` loops with `?`. The check stays inside the loop body, so it still runs
  after earlier rebinds.
- **C-6.** Change `r"E:\i\u{307}"` at `browse/read/tests.rs:319` to `"E:\\i\u{307}"`, the form its neighbour uses. Keep
  `check_tree` unchanged. A failure then is a real tree or key bug, to be fixed in the tree code.
- **C-8.** Migration 2's triggers, built with the `next_seq!`/`seq!` macros as the existing triggers are
  (`schema.rs` ~546-572):
  - `analysis_cut_stamp`: AFTER UPDATE OF `cut_left, cut_top, cut_right, cut_bottom` ON analysis, when any of them
    differ. It runs `next_seq!` and sets `files.changed = seq!()` for `new.file_id`, and does not touch `files.seq`
    (thumbs regenerate on that).
  - `border_sides` AFTER INSERT and AFTER DELETE, mirroring `border_marks_stamp_insert` and `_delete`, so `store_borders`
    is seen.
  - AFTER DELETE ON analysis, mirroring the existing `analysis_removed` pattern, if that pattern doesn't already cover it.
  `changes_since` is kept: S-08.8 is done (`REFACTOR_PLAN.md:756`), and its consumer is not implemented, which the doc says.
  The finding's `lib.rs:1329` cite is stale; `store_borders` is at `lib.rs:1458`. Tests: `store_borders` shows up in
  `changes_since` with `files.seq` unchanged; a cut change and a border delete stamp the file. Schema tests: baseline plus
  migration equals the expected schema for version 2; a version-1 catalog migrates forward without a rebuild and keeps
  its rows.
- **C-12.** Mark, don't skip. In `Catalog::prepare` (`lib.rs:590-630`), after `check_rules` and before
  `register_functions`, inside `write_transaction`: `SELECT DISTINCT ext FROM files WHERE missing = 0` (constant SQL);
  for each ext where `pg_model::media::Ext::of` returns None, `UPDATE files SET missing = 1 WHERE ext = ?1 AND missing = 0`
  (bound value). The scan keeps only held extensions (`scan/src/lib.rs:104-110`), so those rows never come back. Nothing
  else changes (`entry_from_row`, `entry()`, `list_grid`, the tree, facets and Status), so every count agrees by
  construction. `entry()` flattens the `query_row(..).optional()` result (`grid.rs:78-84`) as it does now. Test: a row with
  ext `xyz` set through SQL is missing after prepare; the grid list succeeds; the tree, Status and facets all exclude it.
  The consequence is listed under Behaviour changes.
- **C-23.** The key functions are already gone (278dfe8): `browse/read.rs::scope` takes the catalog's `PathRules`, the
  tree (`tree.rs:39, 53, 96, 113`) and `views.rs:33` do too. So no code deletion and no new `rules.rs` API. Remaining:
  - A **scan → `list_grid` regression test** in `crates/scan/src/lib.rs`'s tests, with a fake listing. Folders named `ΑΣ`
    and `ΟΔΥΣΣΕΥΣ` with one file each, plus a child folder with one file. `list_grid(folder, false)` returns the folder's
    own file, and `list_grid(ancestor, true)` returns all of them, under both `PathRules::Windows` and `PathRules::Posix`.
  - Fix the stale comments at `catalog/src/lib.rs:528` and `schema.rs:296`, which name `path::` functions that are gone.
  - Mark `docs/S17_REMAINDERS_PLAN.md:9` fixed.
- **C-33.** `rules.rs` gets a `prefix_upper_bound` helper for the recursive scope. A folder's key ends in `0x01` for a
  non-empty folder, so the bound replaces that last byte with `0x02` (`lo.last_mut()`). An empty folder key stays
  unbounded (`None`). Rewrite the comment at `rules.rs:237-238` to state the invariant: "a folder key ends in 0x01, so
  `lo[..-1] + [0x02]` is exact". Unit tests for empty, `[0x01]` and `[0xFF, 0x01]`. A Posix property in
  `model/tests/path_laws.rs` with a child whose name starts with `0xFF`.
- **C-34.** `ids.rs:22-24` says the device is the whole `u64`, consistent with `ids.rs:31-33` and `dir.rs:469`, and drops
  the 32-bit sentence. `yandex/src/lib.rs:12`, `:26` and the test name at `:602` say "the crawlers monorepo's search.py,
  which is outside this repository". `trim_seps` (`rules.rs:443`) moves above the first `#[cfg(test)]` at `:300`.
  `library/tests/borders.rs:14`: if the redundant `#[cfg(windows)]` is still there, it is removed. Do **not** touch
  `PathRules::is_under` (`rules.rs:250`), which is live.
- **C-40.** `pair` takes `Coverage { complete: bool, skipped: &[RulePath] }`. `skipped` is the unreadable folders plus the
  reparse folders not followed, and `scan/src/lib.rs` passes the full chained list to both `pair` and `mark_missing`
  (`~l.336`).
  - In `Catalog::pair`, the skipped folder ids are computed once, with a helper shared with `mark_missing`'s `under()`
    closure (`catalog/src/lib.rs:734-741`, lifted into a shared helper).
  - The fresh rows (`fresh_rows`, `pair.rs:202-215`) are filtered by **`last_scan = scan`**. A row seen this scan was
    upserted in a folder that was listed (`lib.rs:1174-1190, 1253-1260`), so it is in a readable folder. Rows in skipped
    folders are also dropped. Without this, a stale fresh row can take over an unseen live row in a skipped folder through
    `replaced_by` (`pair.rs:128-136`).
  - Tier-2 candidates (`pair.rs:91-100`) exclude skipped folders. The clearing UPDATE (`pair.rs:155-165`) adds
    `folder_id NOT IN (skipped)`.
  - Tiers 2, replacement and clearing run when `complete`. A stopped scan does tier 1 only, as before.
  - Behaviour change: the tiers now run on a complete scan with unreadable folders. `docs/S17_7_PLAN.md` decisions 4
    (`:63-70`) and 9 (`:102-104`) state the old rule and are updated.
  - Update the `pair.rs` doc (`:51`) and the test helper `pair` (`pair.rs:356`). Extend
    `a_reparse_folder_is_not_descended_and_its_rows_are_kept`, and add a test that an unreadable folder's rows are neither
    paired nor cleared.
- **C-41.** The verbatim rule becomes the platform's one rule (`platform/src/native.rs`).
  - `NativePath::set` applies the verbatim form (`\\?\`, or `\\?\UNC\` for a UNC path) to absolute paths with no `/` and
    no `.`/`..` segments. The rule is computed over the **joined** units: `set_units` (`:62-73`) and `split` (`:101-108`)
    see the whole path, not the first part. Relative, drive-less and already-prefixed paths stay plain. `SHORT` (`:26`)
    and its `len` parameter are deleted.
  - The volume calls keep their inputs: `volume_root` (`dir.rs:171-172`) and `drive_type` (`dir.rs:184-185`) use a plain
    setter (`NativePath::set_plain`, the old short-path rule), so Win32 gets what it gets today.
  - `pg_platform::native::verbatim(&Path) -> PathBuf` uses the same rule. It is applied to writeback's `stat`,
    `std::fs::read` (`lib.rs:247`), `copy_into` (`lib.rs:365-368`), the temp I/O (`lib.rs:291`, `temp_of`), and crop's
    `stat`, `write` and `rename` (`crop.rs:153-175`).
  - Writeback and crop refuse, with a typed error, any path whose Windows folder segment ends in a dot or a space. ExifTool
    can't take `\\?\`, so ExifTool-bound jobs get the same refusal (Decisions, 4).
  - Tests: the short-path test is flipped; relative, `/` and a real `Acme Inc.` trailing-dot folder test; a writeback test
    that a rating for `Acme Inc./a.jpg` is refused and does not touch `Acme Inc/a.jpg`.
  - The native.rs header and the summary say that ExifTool still normalizes the paths it is given, and that the refusal
    covers it.
- **C-58.** Recursion is a global toggle, so the Drive-roots row is a shortcut for the pair `(folder "", recursive false)`.
  - `model/src/tree.rs`: `FolderTree.root_files` counts the files of an empty-parts path in `TreeBuilder::add` (the early
    return at `:110-112`). `total()` includes it, and the doc at `:107-108` is updated.
  - `client/src/tree.rs`: `Visible` becomes `enum { All, DriveRoots, Folder(usize) }`, deriving `Ord` in that order,
    because `binary_search` at `:97` relies on it. `row_of(folder, recursive)` (the `row_of_path` at `:93-97`, used by
    `folders.rs`'s `spelling_of` and `reveal_listing`, and by `tests/tree_laws.rs:38-40`) maps `("", true)` to All,
    `("", false)` to DriveRoots when `root_files > 0` and to All otherwise. So the start gate (`start.rs:13-14, 51-55`)
    keeps its contract. DriveRoots is shown at row 1 only when `root_files > 0`. `plan_toggle` (`tree.rs:102-110`) returns
    None for non-Folder rows, and `FolderModel::toggle` ignores them.
  - `desktop/src/shell.rs` **dispatches by row kind**: `open_folder_row` (`:437-438`) sends All → `OpenAll`, DriveRoots
    → a new `listing::Event::OpenRoots` (folder `""`, recursive false, one OpenList), and Folder → `OpenFolder`. Today
    `folders::merged_at` maps every non-folder row to `""` with recursion unchanged, which would show every file. The
    `spelling_of` call at `shell.rs:312-313` and `reveal_listing` get the recursive flag.
  - Labels: the row's text is `ui-drive-roots`, added to `ui.ftl` and asked for by QML (`words.rs` test). The hard-coded
    "All folders" in QML is replaced by the existing `ui-all-folders`. Clicking All (`OpenAll`) always sets `recursive = true`,
    not only when the Drive-roots row exists. So `("", false)` never shows as an empty All row when the drive roots hold
    no files. The code keeps this choice; the plan's earlier wording (only with the row) is superseded. Top-bar and
    save-dialog label for the empty folder: "Drive roots" only when subfolders are off and the drive roots hold files
    (`Tree::shown_as`); otherwise "All folders".
  - Qt side: the new row needs a role in `desktop/src/folders.rs`, and `Main.qml`, `FolderTree.qml` and the qmltest stubs
    change.
  - Tests: `model/src/tree.rs` (drive-root-only and mixed); `client/tests/tree_laws.rs`; the `check_tree` oracle in the
    catalog tests; a `shell` test that clicking Drive roots dispatches `OpenRoots`, not `OpenFolder("")` with recursion on.

### 5. App, client, thumbcache and Yandex (C-1, C-7, C-9, C-10, C-11, C-13, C-14, C-42, C-43)

- **C-1.** Add `pub fn original(e: impl Display) -> Error { Error::Original(format!("{e:#}")) }` in `app/src/error.rs`,
  beside `failed` (`:29-31`), documented as "the original can't be read (offline, or not decodable)". In
  `images.rs::decode` (~`:300-307`), build the variant **outside** `with_maker`, because `with_maker` maps every closure
  error through `failed` (`app.rs:161-163`) and would flatten it back. The `read_file` error (io) and the `decode_file`
  error (anyhow) are each mapped with `original`. A video's `pg_video::frame` failure is mapped with `original` too. The
  same helper covers the thumbnail's poster frame (`images.rs:190`) and read (`:196`), which are the same condition. Drop
  the hand-written "original unavailable:" prefix; thiserror adds it. Test: a catalog file with a missing path →
  `Err(Error::Original(_))`.
- **C-7 and C-14.** Reword the comments: `app/src/library.rs:166-167` says the files are marked missing at once and come
  back when the folder is added again (matching `catalog/src/lib.rs:802-804`); `catalog/src/pair.rs:407` says the row is
  not paired and the clean pass clears its unpaired mark; `app/src/app.rs:189`, `cli/src/lib.rs:151-152` say the thread
  runs until the process ends, since the cache is leaked; `catalog/src/lib.rs:1507` drops "as Windows spells it"; and
  `app/src/edit.rs:2` says "the app's preferences". `replay.rs:2`: the self-reference the finding describes was not found,
  so its wording is located before editing, or the item is dropped. `controller.rs:58`: the "A size request…" doc moves to
  `Panel`, `Size` and `Listing`, each with its own doc.
- **C-9.** `Controller::flush` (`controller.rs:336-340`) drains the deadlines. `ApplyQuery` or `SaveSession` sets `save`,
  and `Size` is kept. If `save`, it inserts one `Timer::SaveSession`, and it calls `self.fire(list)`. The `SaveSession` arm
  of `fire` (`:347-349`) already emits `SaveSession` and `SaveLastFile`, so a hand-written bare `SaveSession` would drop
  `SaveLastFile` on shutdown. `ApplyQuery` must never reach `listing_now` (`OpenList`). Tests in
  `client/tests/controller_laws.rs`: a pending `SaveSession` gives `SaveLastFile`; a pending `ApplyQuery` gives one
  `SaveSession` and no `OpenList`; both pending gives one `SaveSession` and one `SaveLastFile`.
- **C-10.** `app/src/yandex.rs` stops deciding by extension. `read_photo(path)` opens the file once, reads 16 bytes, and
  `Format::sniff`s them. `None` → `SearchError::NotStill`, so a video is refused after 16 bytes, not read in full. Jpeg,
  Png, Webp, Gif and Bmp are uploaded as they are. That list is the set the search has always taken (`is_still_image`'s
  list, from the crawler it was ported from), not something `pg_yandex::sniffed` checks; `sniffed` checks downloads.
  **Tiff** is transcoded by `pg_render::transcode::to_jpeg(bytes) -> Result<Vec<u8>>`:
  `let mut decoder = Decoder::new()?; let decoded = display::decode(&mut decoder, bytes, MinSize::Full, Turns::NONE)?;`,
  then a turbojpeg `Compressor` at quality 95 with `Subsamp::None` (the `maker.rs:191` pattern). The new module doesn't
  call `Format::sniff`, `pg_metadata::orientation` or `decode_into`; `display.rs:1-4` keeps the EXIF read in one place.
  `pg_codec`'s `encode::cropped` (`encode.rs:11-22`) already encodes TIFF and BMP through the `image` crate, and pg_codec
  has no JPEG encoder (turbojpeg is in `pg_render::maker`). `SearchError::Undecodable(String)` maps to
  `text::yandex_unreadable_photo`; the match in `desktop/src/yandex.rs:196-202` gets the new arm; the `yandex-not-still`
  text lists TIFF.
  Tests: PNG bytes unchanged; video-like bytes → NotStill; a TIFF → JPEG of the same size, verified by decoding through
  `pg_codec` (pg-codec is a normal dependency of pg-app, not a dev-dependency with feature `fixtures`; `deny.toml:57` limits turbojpeg to pg-codec, pg-edit and pg-render,
  so the pg-app test can't use it directly); a pg-render unit test for `to_jpeg`. The live Yandex path is not run.
- **C-11.** `save_one` writes the bytes to a dot-temp beside the target (`.<name>.pg-save`, the repo's convention:
  `.pg-write` at `writeback/src/lib.rs:355-361`, `.pg-crop` at `crop.rs:164`) and syncs it. It claims the final name with
  `create_new` in a " (2)", " (3)" suffix loop, replacing `unique_path`'s check-then-write. It then renames the temp over
  the claimed placeholder. A failure at any step removes both. The 0-byte placeholder exists only for the instant between
  claim and rename, not for the whole download, and a crash in that instant leaves it (documented). Search and save share
  one `spawn_worker` helper using `thread::Builder`. `start` and `save` return `Result<bool, io::Error>` from `spawn`; the
  desktop logs it with `panic_log::note` and shows a `Message` variant, not `Status::Stopped`, whose text says the details
  are in gallery.log (`desktop/src/yandex.rs:141, 233`), which a spawn error never writes. Tests: a failing write leaves no
  partial; two saves of the same name get distinct files; the seam for a failing write is a directory at the temp path.
- **C-13.** `listing_laws.rs` keeps its proptest, and the key generator iterates `SortKey::iter()`, so every key is
  exercised. `SortKey` derives `EnumIter`; `strum.workspace = true` goes into pg-app's `[dev-dependencies]`, and the test
  uses `strum::IntoEnumIterator`.
- **C-42.** Damage is detected **inside `open_writer`, under the lock it already holds** (`pg_schema::Lock`, `lib.rs:690-692`),
  not by a second lock taken after `judge`, which would not be atomic. `judge` maps `rusqlite::ErrorCode::DatabaseCorrupt`
  and `NotADatabase` to `Found::Damaged`. `open_writer` moves the file aside and retries once on a fresh file, all under
  the same lock. `heal_if_due` runs in that locked section too, so heal-time corruption takes the same path. Heal runs at
  most once a day (`lib.rs:240`, `:1100-1106`), so a damaged body is found up to a day late. The move-aside is the
  catalog's `move_aside` (`catalog/src/lib.rs:319-341`), which never overwrites and removes sidecars first; it is reused
  through `pg_schema`, not reimplemented. Non-cache errors (busy, permissions, disk) stay errors, and `Foreign` stays for
  valid non-cache databases. The event is `Event::StartedOver{why}`, with a `Message` variant rendered in both
  `cli/src/lib.rs:168-174` and `desktop/src/app.rs:76-82`. Tests: a garbage body and a truncated real cache both open, with
  `.bad` holding the old bytes and the event published; a valid foreign database is still refused (the tests at
  `~:1474/:1484` are updated); the heal test resets `meta.healed` (as `lib.rs:1576-1580` does), or it never reaches heal.
- **C-43.** In `yandex/src/lib.rs`, `claimed_format` becomes `plausibly_image(content_type)`, an **allowlist**: image
  types (`image/*`, including `jpg`, `pjpeg` and `x-png`), plus empty, `application/octet-stream` and
  `binary/octet-stream`. Anything else is refused before the body is read, so `video/mp4`, `application/pdf` and
  `image/svg+xml` are not downloaded up to the cap. The URL extension fallback is dropped; `sniffed` decides after
  download. Timeouts: ureq 3.5.1 does have a per-read timeout (`timeout_per_read`), plus `timeout_send_body`,
  `timeout_recv_response` and `timeout_recv_body`. The download uses per-request configuration (`request.rs:368`): connect,
  response, per-read as the idle bound, and recv_body as the total cap. The agent-wide `timeout_global` stays as it is,
  since the small JSON search calls use it too (`lib.rs:172-219`). Tests at `~:611-619`: the image types, text/html, json.

### 6. Desktop, QML and C++ (C-15, C-16, C-17, C-18, C-19, C-20, C-21, C-57)

- **C-15.** `Viewport` (`desktop/src/viewport.rs`) declares `fit_width` and `fit_height` as
  `#[qproperty(i64, fit_width, cxx_name = "fitWidth", READ, WRITE, NOTIFY, FINAL)]` (and `fit_height`), matching
  `fit_px` (`viewport.rs:103`). NOTIFY is needed: every qproperty there has it (`viewport.rs:16-28`), and without it the QML
  bindings and the settle timer never fire. WRITE is needed for `set_fit_width` in `sync()` (`viewport.rs:202-214`).
  In QML: the existing `onVisibleChanged` at `Viewer.qml:81` is extended, since a second handler on the same object is an
  error; `onKeyChanged` is added; the fit `Image` source at `Viewer.qml:349` (guarded by `viewer.visible`) and the Repeater
  source at `:377` bind to `fitSource(key, fitW, fitH)`; `onDprChanged: pushWindow()` is added at `:68`; the comment at
  `:205` is fixed. A ~120 ms `Timer` settles the sizes, and `ImageLayers.fit`'s `retainWhileLoading` keeps the old pixels
  until the new ones are ready. The stub `crates/desktop/tests/qml/stubs/pg/gallery/Viewport.qml` gains `fitWidth`,
  `fitHeight` (NOTIFY) and `fitSource(key, w, h)`, which it has neither of today. The fit `Image` in
  `ImageLayers.qml:59-70` gets an `objectName`, for a `tst_viewer.qml` resize case.
- **C-16.** Change the `borders` entry in `tools/compare-shots.ps1:87` to `--borders=1`, and change `TestHooks.qml:17`
  (the list; `:16` is `--add-folder`) and `:25` (the prose "--borders are shown from the start by Main"), and
  `crates/README.md:348` (its prose; the flag list at `:346` already says `--borders=1`). `parse` stays as it is.
  **Recipe:** commit the fix first, because references are per commit (`tools/shots/reference/<sha10>/`). Run
  `just shots -Expect borders` once for the transition. Then `just shots -Record <fix-commit>`. `-Expect` only declares a
  scenario as expected to differ; it does not re-record. Compare later with `-Base <fix-commit>` (`compare-shots.ps1:236`).
- **C-17.** `screenshot.cpp` computes `timed_out` first. With no window and a timeout, it saves an empty PNG through the
  existing `screenshot_taken` path (which writes the settle log and the stderr line; no second writer) and calls
  `QCoreApplication::exit(1)` only there. Not ready and timed out: skip the ready and hook bookkeeping and save, reporting
  "timed out … not ready" **without** `state->waited.elapsed()`, which starts only when ready is first seen
  (`screenshot.cpp:133, 158, 177-179`). Update the header comments at `screenshot.cpp:6-7` and `screenshot.h:8-9`, which
  describe only the not-settled case. The exit code must reach the OS: `desktop/src/lib.rs:113-115` discards
  `app.exec()`'s status and `run()` returns `()` (`:39`), and `crates/gallery/src/main.rs` returns after `pg_desktop::run`.
  So `run()` returns the `exec()` status, and `main` propagates it as an `ExitCode`. Checked by hand with `--wait=1`, and
  the exit code by `echo $?`.
- **C-18 and C-19.** The "All folders" literal (`folders.rs:182`, the ROLE_NAME arm; the `:147` line is a doc comment)
  uses the existing `text::ui('ui-all-folders')`, which QML already asks for. The "Background work paused" literals are
  `library.rs:283-284`; `notice-more` is `library.rs:312`; "already in the library" is `:329`. The failing `start()` branch
  is `library.rs:204-209` (its note at `:207`), and it also shows `text::library_unavailable(..)` with `show(…, true)`.
  `load_roots` (`:189-197`) fails for a different reason, that the folder list can't be read, so it gets its own message.
  The yandex literals are at `yandex.rs:141` (INTERNAL_STOP), `:187`, `:191-192` and `:222-223`; `:134` is a use line,
  and `:180` and `:184-185` already call `text::render`. Each new `Message` variant has a `text.ftl` entry and an exact-string
  test, including plural 1/n. A pseudo-locale library scenario is added to compare-shots `$Pseudo`.
  Also routed through `Message` variants: `crates/app/src/library.rs:157` (`"{} can't be added: {refused}"`, shown via
  `desktop/src/library.rs:333`); the `YandexError` display strings (`crates/yandex/src/lib.rs:44-57`, shown via
  `desktop/src/yandex.rs:181`); the `Notice::RoundFailed` and `CropFailed` text (`desktop/src/library.rs:174`, and the
  `:346` and `:352` calls that pass lower-layer text to `show`); `panic_log.rs:28-33`; and `desktop/src/lib.rs:68`
  ("Can't open the catalog").
- **C-20.** `goTo` and `Hud.filePath` are already gone. Delete: the `ImageLayers` `fullImage` alias; Viewer's aliases
  `fitted`, `fitZoom`, `minZoom`, `dw`, `dh` and `pixels` (`Viewer.qml:115-123`), and Viewport's Rust `fitted`, `fit_zoom`
  and `min_zoom` properties (the stub keeps its own private fit zoom); `GridCell.key` and `itemKey`; `GridRow`'s `keys` and
  `shownKeys`, and `GridRow.qml:19`'s `required property keys`; `ROLE_KEYS` with its `data()` arm (`gallery.rs:434`) and its
  comments (`gallery.rs:272-273`, `GridCell.qml:12`, `GridRow.qml:17-18`); `tst_gridrow.qml`'s `keys:`; and `rating_in_file`
  (`gallery.rs:845`) with its bridge entry (`:196`). `CELL_ROLES` goes from `[(i32, &str); 12]` to `[(i32, &str); 11]` at
  the same time (`gallery.rs:287`), or it won't compile. The methods only Rust calls (`aspect`, `rating`, `badge`,
  `border_marked`, `info`, `neighbor`, `rotate`, `set_rating`, `set_border_marked`, `refresh_turned`) are removed from the
  cxx-qt bridge declaration and stay as plain `impl qobject::GalleryModel` methods. The bridge keeps `registerModel`,
  `relayout`, `fileId`, `indexOf`, `rowOf` and `saveCroppedCopiesPicked` (`gallery.rs:224-225`, used at `Main.qml:157`).
  `docs/REVIEW.md` R-77 is updated.
- **C-21.** Reword the listed comments: `command_line.rs:1` → "listed in TestHooks.qml"; `screenshot.rs:6` → "with the QML's
  own hooks (TestHooks.qml)"; `crates/README.md:348` → "`--wait` (10 s by default) counts from the call";
  `facet_choices.rs:116` → "facet is the list's index in Facet; 0 (the default) is Types"; `YandexPanel.qml:95-96` → "the
  viewer would step to the next picture (its wheel), so it is not reached"; `codecs.rs:402` → "the decoder refuses a zero
  width or height (tiff decoder `image.rs:325`), so an image has at least one plane here". The `folders.rs` restore and
  missing-quote items need nothing: no `restore` remains, and the quote is there.
- **C-57.** `MediaView.qml`'s wrapper gets `visible: !media.still` and an `objectName`. The gate is `MediaView.qml:69`.
  The size is written explicitly: `width: playScale > 0 ? fw*playScale : media.width`, and the same for height. `playScale`
  is 0 whenever `fw` is 0 (`MediaView.qml:34`), so the fallback has to read `media.width`. `VideoOutput`'s
  `PreserveAspectFit` fits the frame. `tst_media_view.qml` already instantiates `MediaView` (its `mediaC` component), and
  `MediaView.qml:7` imports QtMultimedia, so the regression is testable: a video with `fw` 0 is visible at media size, and a
  still stays hidden. Check the real frame by hand.

### 7. Eval and tools (C-44 to C-55)

- **C-44 and C-45**, in `eval/verify.py`. Add `from PIL import Image` to the import block (`:58-77`); `Image` appears only
  in docstrings today, so `Image.Resampling.LANCZOS` at `:226` would otherwise be a `NameError`. Multiply by
  `ref_scale` and `other_scale` at `:502-503`. (`:393` is only the `Aligned.ref_scale` field.)
  - **Baseline first.** Take pre-fix runs of all three pair sets: `labeled`, `cross_groups` and `knn`. The labeled baseline
    recorded so far (1043 pairs; duplicates 698/698; similar_folder 237 match and 108 unrelated; noise percentiles
    p10/50/90/99: duplicates 0.0032/0.0056/0.0093/0.0171, similar_folder 0.0101/0.0436/0.0999/0.2347) has no counterpart for
    the other two sets. Archive the pre-fix `verify_sscd_fp16` outputs before rerunning, since `write_outputs`
    (`verify.py:863-882`) overwrites them, and delete any leftover `.partial.csv`, which `Checkpoint` would resume
    (`verify.py:835-862`).
  - **`--max-noise`.** The labeled set has no negatives (`PAIR_SETS['labeled']` is duplicate and similar_folder,
    `verify.py:97-99`; `EXPECTED` covers only duplicate and cross_group, `:104`), so it can't choose the threshold. The
    default (0.06, `verify.py:646`) is re-picked with the FINDINGS criterion (cross-group p1 against duplicate p99,
    `FINDINGS.md:619-641`) on the `cross_groups` data, using `verify.py --rescore` for the sweep. Old and new figures go in
    `docs/FINDINGS.md`. The verifier figures at `FINDINGS.md:655-661, 736, 747, 805-806` were measured with BOX; each is
    either rerun or marked as predating the fix.
- **C-46.** Each of `gpu.py` and `nvimgcodec_bench.py` carries its own `blocking_sync()`, implemented with `ctypes` through
  `nvcuda.dll` (`cuInit`, then `cuDevicePrimaryCtxSetFlags(0, 0x04)`), checking the return codes. It needs no torch, which
  the nvImageCodec venv doesn't have. It is called before the first CUDA call, and works in both venvs. `nvjpeg.py` gets the
  same helper only if its numbers need it.
  - **Numbers.** The rewrite quotes them separately: torch's spin share is 30.9 to 19.9 ms of CPU at batch 1
    (`gpu_spin.txt` against `gpu_blocking.txt`), about 7 to 11 ms; nvImageCodec's is 16.8 to 13.3 ms. The "about 3 ms"
    figure belongs to nvImageCodec only. The version cited is nvImageCodec 0.10.0.9 (Decisions, 6).
  - **Conclusion.** Qualified, not dropped: `nvimgcodec_blocking.txt` still shows CPU about 2x wall at batch 64, and
    `nvjpeg.py`'s GPU_HYBRID (CPU 0.02 ms, `JPEG_DECODERS.md:233`) against HYBRID (3.2 ms) is a discriminator.
  - **Doc rewrite:** `docs/JPEG_DECODERS.md` at `:46`, `:58`, `:179-200` (the table, the CPU-equals-wall bullet at `:197`,
    and footnote `:189-190`), `:223-226` and `:481-486`.
  - **GPU_ONLY.** "Fails for every image" and "HW_GPU_ONLY is unsupported on Turing" have no saved output
    (`gpu_only_probe.py` and `gpu_only_kinds.py` have none, and `backends(nv)` (`nvimgcodec_bench.py:26`, built in `main()` after `blocking_sync()`) has no
    HW_GPU_ONLY). Run the probe and save its output before quoting it, or state the claim as unverified.
  - `nvimgcodec_bench.py` already records per-backend failures in the decode path (`:77-82`, `:90-94`). Only the `Decoder`
    construction (`:65-67`) is unguarded: make it report an explicit message, and check `out is None` ("returned no image")
    per backend.
  - Installing nvidia-nvimgcodec without a pin replaced the shared `nvjpeg64_13.dll` in `venv-eval`, because its `[all]`
    extra pulls in an unpinned `nvidia-nvjpeg`. The requirements file now pins `nvidia-nvjpeg` at 13.3.2.43, so both packages
    stay in one env.
- **C-47.** `gpu.py:55` requests the **floor** size, `(im.width // d, im.height // d)`, as `nvjpeg.py:224` does. Pillow's draft
  picks its scale from the request, so the floor request keeps the `1/d` scale. The decoded size is then the ceiling,
  `ceil(W/d)`: 3840x2151 at `d=4` gives 960x538. After `im.draft()`, assert `im.size == (-(-w // d), -(-h // d))`. The
  `nvimgcodec_bench.py` part is dropped: it has no draft and no `d` (`:74-96`), and its shape check is `diff()` at `:49-50`.
  Rerun `gpu.py`, and rewrite `docs/JPEG_DECODERS.md:185-190` (the "Pillow full / scaled" column and footnote 1), which
  the bug explains.
- **C-48.** `read_ppm` is replaced in `debesheedas.py`, `gpu_encoders.py` and `nvimgcodec_bench.py`, which may share one helper module (Decisions, 5). Each replacement is
  `np.ascontiguousarray(np.asarray(Image.open(path)))`; Pillow is a dependency of all three venvs. The contract is the same:
  uint8, shape `(h, w, 3)`. A shared module would need a ledger row.
- **C-49.** `png_of` writes to a temp and renames it, and saves with `format='PNG'`. PNGs are made in `prepare()`, serially,
  and only for inputs guetzli will get (`w*h/1e6 <= GUETZLI_MAX_MP`, `encoders.py:42`), so 24 MP PNGs aren't written for nothing.
- **C-50.** `Decoders::new` opens the four FFmpeg MJPEG decoders eagerly (`src/lib.rs:115-120`), and the lazy path
  (`:136-142`) goes. The bench's `run` returns `(Rgb, Vec<u8>, Decoders)`, so the input's free falls outside the measured
  call (today `run` drops the input inside the measurement, `ir.rs:31`). The `bench!` macro (`ir.rs:35-43`) fixes
  `setup = load` and `-> Rgb`, so it changes to the new return type. The FFmpeg rows in `docs/JPEG_DECODERS.md` (about
  `:44`, `:109-150`, `:171-173`) become stale and are rewritten. The benches run under callgrind, which isn't available
  here; compile-check only. The header comment is fixed.
- **C-51.** `bench_hdd.py:165-167`: `step = len(files)//6` and the slices `files[k*step : k*step+dense]` overlap when
  `dense > step`. Fix it in `bench_hdd` main: error out when `args.dense * len(orders) > len(files)` (the disjoint promise
  can't hold), and use `step = args.dense`, or `len//len(orders)`, so blocks never overlap. Keep the docstring. The
  comment fixes are the ones listed in the Findings table; `pdq.py:4` becomes "same sample as synth.py when both ok masks
  match".
- **C-52.** `bench_hdd.py` `open_file`: resolve `--root` once in `main` (`Path.resolve`) before the walk, and drop the
  abspath from `open_file`. `bench_ann.py` `rerank`: fill `out[i]` with -1, and write the top-K valid ids into the first
  slots. `harness.cpp`: check `args.size()` before reading `args[1]` in both flag loops, and `args.empty()` before
  `args.at(0)` in `throughput_mode`, returning the usage code 2. `restart.sh`'s `ir()` mirrors `wsl.sh`'s `ERR`.
  `debesheedas.py`: absolute paths for `FILE` and `--exe`. `verify_synth.py`: move the three calls outside `PairRunner`'s
  `try` (`prefetch.get(i)` at `:93`, `prepared(features, work)` at `:94`, `copies[t_index]` at `:99`) into per-pair
  `get_a` and `get_b` lambdas, with an `lru_cache` per image, as `verify.py:790` does. `Prefetcher.get` pops its key
  (`verify.py:264-267`), so a second call would reload.
- **C-53.** `viewer.py`:
  - Split `_read_marks` (unlocked) from `load_marks`, which takes `marks_lock`. **`set_marks` (`:204-205`) calls the
    unlocked `_read_marks` inside its with-block**, because `marks_lock` is a non-reentrant `threading.Lock` (`:172`), so
    taking the lock in `load_marks` alone would deadlock the first POST. `benchmarks()` (`:91`) uses `load_marks`.
  - No replace retry: no external reader of `pair_marks.json` has been identified (the viewer's only reader is its own
    GET), and a retry under the lock would stall GET.
  - `do_GET`'s `except` (`:283-286`) gets an `OSError` clause, so GET `/api/marks` doesn't drop the connection.
    `do_POST` gets an `OSError` → 500 clause after the connection-error clause (`ConnectionAbortedError`,
    `BrokenPipeError`, and `ConnectionResetError` from `rfile.read` at `:299`, silenced as `do_GET` silences it at `:285`).
  - `viewer.html`: one helper wraps `fetch` in try/catch and checks `res.ok`. It is used by `toggleMark` (`:489-502`) and
    by `postMarks` (`:717-727`).
  - Test: `eval/test_viewer.py` uses stdlib `unittest` (`eval/requirements.txt` is a frozen freeze, so pytest would be a new
    dependency). Importing `viewer` needs `PG_ROOT`, `PG_WORK_DIR` and `PG_CACHE_DIR` set (`common.py:35-50`), so the test
    sets them first. Concurrent `load_marks` and `set_marks` threads, and a POST that hits `PermissionError` returning 500
    with the message. The concurrency reproduction fails only on Windows (POSIX `os.replace` ignores open readers); the
    test says so.
  - Runner: no Justfile recipe or CI step runs Python tests (`Justfile:65-67` is cargo only; `ci.yml:158` runs `just test`).
    Add a recipe, `python -m unittest discover -s eval -p 'test_*.py'`, and have it run.
- **C-54.** `wsl-bench.sh` skips the archive when a **non-option argument** (a bench-name filter) is given, not when any
  argument is given, so a full run with `--save-baseline=before` is still archived (`PROFILING.md:154-158`). After the cargo
  bench line (`:52`), set `filtered=1` for any argument that doesn't start with `-`. Wrap the archive block (`:54-66`) in
  `if [ $filtered -eq 0 ]`, and echo "profiles: not archived (filtered run)" otherwise. Rewrite the header (`:6-8`), the
  block at `:54-57` (which says a bench-name run archives those benches), and `docs/PROFILING.md:156-157`. `perf-check.sh`
  needs no change.
- **C-55.** `tools/compare-shots.ps1`:
  - Check `$LASTEXITCODE` before `.Trim()` at `:121` (`Update-BaseWorktree`), `:134` (the `$top` rev-parse, which has no
    check at all) and `:239` (`Resolve-Reference`).
  - Rename `$args` to `$cargoArgs` at `:111-113`.
  - Add `if ($Record -and $Only) { throw '-Record and -Only do not go together' }` after `:48`, and do the same for `-Expect`
    if that is refused too.
  - At `:426`, the total is `$report.Count`, which covers the pseudo-locale rows.
  - The `$proc.Handle` change is dropped (Decisions, 7).

### 8. Documentation

- `docs/CODE_REVIEW.md`: this file.
- `docs/REVIEW.md` R-77 (C-20).
- `docs/CODE_LEDGER.md`: a row for `eval/test_viewer.py`, and for `crates/render/src/transcode.rs` if the ledger lists
  Rust files. No row for `ppm.py` or a shared blocking-sync module, since neither exists.
- `docs/FINDINGS.md`: the C-44 and C-45 reruns, and the BOX-era figures.
- `docs/JPEG_DECODERS.md`: C-46, C-47 and C-50 (`:44`, `:46`, `:58`, `:109-150`, `:171-173`, `:179-200`, `:223-226`,
  `:481-486`).
- `docs/S17_7_PLAN.md`: decisions 4 and 9 (C-40, `:63-70` and `:102-104`), and the "no migrations" statement.
- `docs/S17_REMAINDERS_PLAN.md:9`: fixed (C-23).
- `docs/PROFILING.md`: C-54 (`:156-157`).
- `crates/README.md`: the "no migrations" statement (~`:186`), `--borders=1` (`:348`), and `--wait` (`:348`).
- `docs/DATABASE.md`: no change.

## Verification

Run once, after all edits.

- `cargo fmt --check`; `cargo clippy --workspace --all-targets` (the panic-freedom lints); `cargo test --workspace`. Notably:
  pg-jobs, including the timeout-guarded hang tests; pg-catalog, including the schema check (BASELINE plus migrations against
  expected) and the v1-to-v2 migration test; pg-writeback and pg-library; pg-client text, controller and tree laws; pg-model
  path and rules laws; pg-edit; pg-metadata; pg-thumbcache; pg-yandex; pg-render; pg-app; pg-platform's `open_for_copy` and
  `verbatim`; the desktop words test. Also `cargo check -p pg-client --target wasm32-unknown-unknown`.
- `just lint` (ruff, markdownlint); `just lint-qml`; `just qmltest`.
- `just shots`, by the C-16 recipe: commit first, `-Expect borders` once, then `-Record <fix-commit>`.
- Python: `ruff check eval tools`; the new Python test recipe (C-53); each script run from its venv, checking that the
  version header prints and the output matches the reader's contract (C-48).
- Python runs that can be done here: `verify.py` before and after the fix, for all three pair sets (C-44, C-45); GPU
  benchmarks with the blocking-sync helper and the ceiling assert (C-46, C-47) in both venvs.
- Not runnable here, to be reported plainly: callgrind (C-50); the live Yandex path (C-10, C-43); Qt GUI behaviour
  without a built `gallery.exe` (C-15, C-17, C-57), beyond the tests and manual checks named above.

**Verification status.** Nothing in this list has a recorded result. No command was run for this record, and no build,
test or Python run was made for it. Verified so far: the fix review checked each row's claims against the code by
reading (see "Limits"), and the probes that rows name (C-24's weezl probe, C-55's `$proc.ExitCode` probe) were run
during the review. No saved output for them is named in the repository, so they are not evidence in this record.

- **Pending, the list above:** run it once in the pg-dev container, after the last edit and once the uncommitted
  edits in the working tree (see "Status") are committed. Run `just check-linux` (`docker/check.py`, which replaced `docker/check.sh`), and the wasm32 check that CI runs
  (`.github/workflows/ci.yml:290`; the ledger cites `:286`). Record each command's result here.
- **Pending, C-16 shot references:** the shot references are not recorded again. No commit so far changes
  `tools/shots/reference/` (its last change is in 856ce1b). The recipe is: commit first, `just shots -Expect borders`
  once, then `just shots -Record <fix-commit>`. It needs the built desktop app, so it is not run here, and the new
  reference directory is committed after it is recorded. Until then the C-16 row stays pending.

## Behaviour changes and limits

Each is listed so the change is visible, not hidden in a fix.

- **C-39:** other programs can't write a photo during the in-place copy; their save fails or retries. A sharing violation on
  an existing folder is a counted attempt, so a job waits at the cap for the next change.
- **C-12:** rows with an extension this build doesn't hold are marked missing at open, not shown. Queued ratings and turns
  on them are then dropped by the existing `file_gone` path.
- **C-41:** writeback and crop refuse folders whose names end in a dot or a space, and ExifTool-bound jobs too. Other
  std and ExifTool opens still go through Win32 normalization.
- **C-2:** stuck ratings made before the fix are kept in the catalog but not written to their files retroactively. The
  "known is None" rule was dropped.
- **C-36:** no `copying` column. A copy that fails at the cap keeps its temp and record: the file reads as being written,
  and each start or next change retries it. Temp names are per kind; rows from before the fix keep their names.
  Residual: rows written by the previous build carry the old "record before copy" meaning, and their finishing copies
  don't compare. Storing the before-stat would need a column, which was declined.
- **C-37 residual:** a recorded temp removed by another program before the copy began makes the copy look finished. A
  crash between `done` and the temp's removal leaves a `.pg-write` beside the photo, which the next write of that file clears.
- **C-24:** GIF min code sizes 1 to 11 are accepted, not 12.
- **C-27:** a panic in a thumbnail is no longer retried on every pass; the file is retried only when it changes. A read
  panic completes the pass as a per-file error.
- **C-40:** tiers 2, replacement and clearing run on a complete scan with unreadable folders.
- **C-42:** a non-database file at the cache name is moved aside (the `x` test in thumbcache flips). Body damage is found at
  the daily heal, not on open.
- **C-43:** non-image types are refused before the body is read. The timeouts are per request; the agent-wide timeout is
  unchanged.
- **C-58:** a Drive-roots row is shown when there are root files. All includes the drive roots' own files: its count is `total()`, the folders' files plus the files directly in the drive roots. A click on Drive roots shows
  only the root files.
- **C-10:** TIFF is transcoded to JPEG; videos are refused after a 16-byte sniff; Yandex may refuse the transcode.
- **C-11:** spawn errors are logged and shown as their own message; the saved file is written to a dot-temp first.
- **C-17:** the exit code now reaches the OS.
- **C-38:** a dropped write no longer lights `write_failed`; the drop is shown once, with its count.
- **C-53:** `do_GET` and `do_POST` answer 500 on `OSError`, and `postMarks` gets the same error handling as `toggleMark`.
- **C-31:** `Stats` gains `stored`, and the cli line uses it. CMYK TIFF crops keep CMYK.
- **C-56:** GIF screens and frames, and PNG and WebP, are capped. BMP is not (it is bounded by file size).
- **C-54:** a full run with `--save-baseline` is still archived; only filtered runs are not.
- **C-46:** the nvImageCodec pin in `eval/jpeg-decoders/requirements.txt` is now 0.10.0.9, the measured version. The nvJPEG pin is 13.3.2.43, so the GPU figures need a rerun. The same file moves `nvidia-libnvcomp-cu13` from 5.3.0.16 to 6.0.0.12, a major version bump (commit 314a99c); the user kept it (Decision 6).
- **C-51 and C-44:** each script carries its own helper. The one shared Python module is `eval/jpeg-decoders/env.py`, the package-version header that the jpeg-decoders scripts import (Decision 5).
- **Decision 5:** the eval scripts run without `python -I`, and print their package versions. Shared helper modules are allowed, so `env.py` is shared.
- **Not run:** the callgrind benches (C-50), the live Yandex path (C-10, C-43), and the Qt GUI checks that need a built
  `gallery.exe` (C-15, C-17, C-57).
