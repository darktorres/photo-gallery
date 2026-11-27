# Open items

Open items moved from the former `FINDINGS.md` log. Some may overlap with the open decisions in [PLAN.md](PLAN.md) §9; check before acting on one.

Checked against the code and the eval outputs on 2026-11-12. Items that name 2026-11-14 were rechecked that day. Rechecked again on 2026-11-18: the code and docs had moved, so each item whose status or location changed that day says so ("2026-11-18"). The eval outputs are in `D:\PhotoGallery` (`eval`, `eval_smoke`, `eval_ann`, `cache`).

- Hand marks on the k-NN candidates (viewer), then tune or learn the verifier decision on them. **Open.** The viewer
  already takes marks on verifier pairs (`eval/viewer.py`, one `pair_marks.json` per run), but no run has a marks file yet.
- Verifier speed, next levers. **Open.** DISK runs on TensorRT by default (`eval/verify.py`, `--disk-runtime trt`). The
  6.7 pairs/s figure was measured before the 262004b verifier fix and has not been re-measured. LightGlue still runs in
  PyTorch (kornia); `eval/` has no LightGlue-ONNX or TensorRT path. 768 px was tried and not adopted (../eval/VERIFIER.md,
  "not adopted"). The rotation retries are not gated: `--rotations` defaults to `none`, and when it is on, every
  non-matching pair pays for the retries.
- Verifier robustness: the median-residual measure swings for ~0.3% of pairs when a few keypoints change
  (TensorRT vs torch flips); look at those with the hand marks. **Open.** The 0.3% figure was measured before the fix
  (../eval/VERIFIER.md, the rotations section, which says so). It has not been re-measured since, and there are no hand marks.
- Second model (So400m NaFlex): choose a runtime. **Open.** No So400m code exists in the crates, only in `eval/` and
  the docs. The cause is recorded in ../eval/MODELS.md ("Why So400m loses precision on TensorRT"), and so is the ONNX
  Runtime GELU-fusion result (84 img/s at torch fp16's fidelity). Not recorded in any doc: the neighbour-agreement
  numbers (0.939 vs 0.984; top-5 0.961 vs 0.990; 0.949 vs 0.996 on the subset), the torch fp16 cosine of 0.9993, and
  the claim that fp32 LayerNorm doesn't help. Those need a measurement note or a rerun before they are cited. The
  small text-query test (../eval/MODELS.md, NaFlex section) has not been run. PLAN.md §9 item 2 says TensorRT fp16 was decided on 2026-10-30, but MODELS.md's precision section shows that choice loses precision, so PLAN §9 item 2 needs correcting.
- Hand marks also decide the remaining grouping errors: different frames (670+671) vs local warps (429), and
  template portraits with different faces (the 12-image group). **Open.** 670+671 and 429 are in ../eval/VERIFIER.md's grouping
  section (lines 72–88). The template-portrait case is at ../eval/VERIFIER.md:78; the AI-portrait group (12) is at VERIFIER.md:90, so
  use the group list there before marking it.
- (Deferred) duplicate vs variant: learned decision on per-region features + face-focused comparison (see
  "Verifier prototype, first run"). **Deferred.** That section (../eval/VERIFIER.md:116, its steps at :123) is still the only record.
- Hand-label a sample of `review_*.csv` to estimate real precision. **Open.** `eval/evaluate.py` writes
  `review_<model>.csv` (line 187), but no such file exists in `D:\PhotoGallery\eval` or `eval_smoke`, so no sample has
  been drawn for any model.
- Thumbnail cache format. **Open.** The cache is JPEG at quality 85, 4:2:0
  (`crates/render/src/maker.rs:37`, `JPEG_QUALITY`), in its own SQLite file (`crates/thumbcache`). THUMBNAILS.md
  says disk is the binding constraint and AVIF is ~38% smaller than JPEG at score 80 (but encodes ~100× slower), and
  the reason is in the code comment at `crates/render/src/maker.rs:35-36` (fastest to make and to show). PLAN §9 item 1
  still asks for a decision on the format (2026-11-18: the reference was §9.1, which does not exist; the maker's comment moved from `:18-19`).
- Candidate retrieval, exact vs approximate (PLAN §9 decision 5): the redo runs on the test set's real
  embeddings plus planted edits, with tuned indexes (the first attempt was invalid, "Nearest-neighbour search,
  first attempt"). **Open.** The harness is committed (`eval/ann/`), and the embeddings and planted edits exist in
  `D:\PhotoGallery\eval_ann`. No tune results exist, and `cache/ann` (validate.json, charact.json) is not in
  `D:\PhotoGallery\cache`. TIER2_ANN_BAKEOFF_PLAN.md's status was checked 2026-11-06, when it said the harness was uncommitted. It was rechecked 2026-11-14 and now says the harness is committed (f0e4b0f).
- Semantic search's index (So400m, 1152-d): FAISS IVF on real vectors once they exist. **Open, blocked.** No So400m
  vectors exist, and the crates have no semantic search.
- Full-set runs (on hold). **Unclear.** No doc says "on hold". What exists: full-set SSCD embeddings
  (`D:\PhotoGallery\eval\emb_sscd_fp16.npy`, 187,806 rows of 512 fp16) and a cross-group verify (2026-11-09, `verify_cross_sscd_fp16.json`). That file records 4,637 pairs, the subset's count in VERIFIER.md, and no set size, so it is not shown to be full-set. Not run: the
  full k-NN verify (187,808 images, "hours per run", ../eval/VERIFIER.md; the embeddings have 187,806 rows, so the two counts differ). 2026-11-18: ../eval/TEST_SET.md no longer has the "full set, not run yet" wording. Its line 41 says nine single-file folders "need a full-set run", and the full-set embeddings exist. Confirm
  whether this is still on hold.
- Video player: Qt Multimedia plays videos now, with hardware decoding turned off on a GPU without AV1 decoding
  (a stopgap, "Videos and animations"). Choose between our own engine on our FFmpeg 9 and libmpv with its D3D11
  render API ([VIDEO_PLAYER.md](VIDEO_PLAYER.md)). **Open.** Qt Multimedia is in use (`crates/desktop/build.rs:72`,
  `qml/MediaView.qml`), and on a GPU without AV1 decoding the player decodes on the CPU (`crates/desktop/src/gpu_device.rs`).
  Neither libmpv nor our own engine is in the code, and VIDEO_PLAYER.md says "not decided".
- Yandex: whether the browser shows all 18 other sizes or about 5 is not measured; only one photo was tested.
  **Open.** This needs a live check with the desktop app (`--find-similar`). A search of the Rust code found no cap on the
  other sizes, and REVERSE_SEARCH.md records 18 for the one photo.
- Page cache per catalog connection (formerly the start profile's lead 6). **Open.** The premise changed on 2026-11-18.
  `SETTINGS` (`crates/catalog/src/schema.rs:605-606`) now sets `cache_size = -8192` (8 MB) on each connection, not 256 MB.
  `Catalog::prepare` sets `PREPARE_CACHE_KIB`, 16 MB (`crates/catalog/src/core.rs:35`, `:112`). The grid reader raises its
  own connection to 256 MB (`GRID_CACHE_KIB`, `crates/app/src/reader.rs:39`, set at `:180`), and drops the pages its jobs
  read after each job (`reader.rs:183`). The thumbnail reader sets -131072 (`crates/thumbcache/src/reader.rs:38`) and only
  reads a few rows each. SQLite's memory is outside dhat, so this has not been measured. Measure the resident
  set of the start with the grid reader's 256 MB cache before changing it.
- Tier-1 index offsets per import (from the import profile). **Open; the named code is gone.** 2026-11-18: `PdqIndex`
  and `crates/dedup/src/pdq_index.rs` no longer exist, and `PdqIndex::reindex` is not in the tree. The same shape is in
  `crates/dedup/src/join.rs`: a table of `VALUES + 1` `u32` offsets (`VALUES = 1 << 16`, so 65,537 entries, 4.2 MB) per
  `FewTable` (`:653`, `:698`). Confirm how many such tables an index holds, then measure an import into a copy of the
  pinterest catalog (190k files) before deciding. The rebuild's `(i64, Hash)` list (about 7.6 MB, an estimate) is not
  measured either.
- Resizer buffers are never trimmed (from the import profile). **Open, premise changed (2026-11-18).** The resizer no
  longer keeps its own `src` and `dst` copies: `crates/codec/src/resize.rs:17-19` holds only fast_image_resize's `Resizer`,
  and `run` (`:98-115`) reads the caller's image and writes into the caller's `out`. The buffers that stay full size are
  the caller's: `Analyzer::trim` (`crates/analysis/src/analyze.rs:153-161`) releases the image, the spare, the PDQ buffer,
  the decoder and the detector, but no resize output that is not one of those. Not checked: which buffer each resize call
  writes into, and whether fast_image_resize's own scratch grows with the image. The earlier 290 MB estimate used the old
  `src`/`dst` and no longer applies.
- Windows path copies per file (from the import profile). **Open.** `set_file` (`crates/platform/src/names.rs:39-46`) builds
  the folder's and the name's UTF-16 buffers for every file, then copies them into the reused `NativePath` (`native.rs:54`). Lead: write the
  units straight from the work list's arena.
- Dihedral hashes collected per analysis (from the import profile). **Resolved (2026-11-18).** The analysis stores the four
  kept orientations (`pg_model::pdq::ORIENTATIONS = 4`), not eight, and `pg_model::pdq::to_blob` returns a fixed
  `[u8; BLOB_BYTES]` (128 bytes, `crates/model/src/pdq.rs:124-127`), which the writer binds at `crates/catalog/src/writers.rs:491`.
  No per-analysis `Vec<u8>` remains. Remove this item once the bind has been confirmed.
- Import at scale, not profiled. **Open.** The 305-JPEG sample has no EXIF, no earlier hashes, no large files and no
  non-JPEG decodes, so the import's costs for those are estimates. Profile an import of a copy of the pinterest catalog.
- On-demand thumbnails and viewer reads, not profiled. **Open.** The heap profile covered the scan, the import and the
  borders and thumbnail passes, and the app's start. The app's on-demand thumbnail requests and viewer reads were checked
  by screenshots and the cached bytes only.
- Grid thumbhash shows when scrolling up, never when scrolling down (opened 2026-11-15). **Open, not measured.** Seen while
  fast-scrolling the grid. (2026-11-17: the ThumbHash is now made by the Placeholders pass after the thumbnails, so a file
  shows one only after that pass has made it; the question of the direction is unchanged.) From the code: a cell shows its thumbhash until its thumb is `Ready` and has faded in
  (`crates/desktop/qml/GridCell.qml`); cells and their requests are made 1500 px ahead of the view (`GalleryGrid.qml`,
  `cacheBuffer`); thumb requests run on a LIFO pool whose comment assumes the newest requests are the ones on screen
  (`crates/desktop/cpp/providers.cpp:4-5`). Nothing has shown which of these gives the direction difference. Plan: thumbnail
  all of `D:\PhotoGallery\datasets\planted` first (18,000 images, confirmed 2026-11-18; `gallery scan`, `import`, `thumbs --height 220`, with
  nothing else running), then time a fast scroll down and back up with per-request timestamps. Temporary logs tagged
  `DEBUGLOG` were in those three files; 2026-11-18: no `DEBUGLOG` is in `crates/` now, so the cleanup step applies only if the logs are added again.
- Cross-language LTO with the bundled SQLite (opened 2026-11-15). **Open, not tried.**
  - **Today.** SQLite is built by `libsqlite3-sys`'s `cc` build: MSVC's `cl` on Windows, gcc in the Linux container.
    Either way it sits outside Rust's LTO.
  - **What was measured.** Fat LTO with one codegen unit on the Rust side alone gave −0.7% on the scan
    (`docs/webclient/scan-optimizations-validation.md`, item 21). That was a callgrind run in the container, so with gcc,
    not MSVC.
  - **Cross-language LTO** brings SQLite in: Rust built with `-Clinker-plugin-lto`, SQLite compiled by clang with
    `-flto=thin`, and both optimized together at the link by `lld-link`. The rustc book's "Linker-plugin-based LTO"
    chapter covers it.
  - **The toolchain fits.** `rustc 1.101.0-nightly` uses LLVM 23.1.5, and `C:\Program Files\LLVM\bin` has clang 23.1.5,
    `clang-cl`, `lld-link` and `llvm-lib`. They aren't on PATH.
  - **The compiler is chosen for SQLite alone.**
    - The `cc` variables (`CC_x86_64_pc_windows_msvc`, `CFLAGS_x86_64_pc_windows_msvc`) are global. In `Cargo.lock`,
      `cc` also builds blake3, libwebp-sys, parajpeg, cxx and the cxx-qt build crates, and turbojpeg-sys builds through
      `cmake`, which reads `CC` and `CFLAGS` too.
    - Set globally, they would make build 2 measure every C dependency's compiler change, and `-flto=thin` would turn
      all of them into bitcode.
    - So the fork's `libsqlite3-sys/build.rs` picks the compiler and flags behind a variable or feature only it reads,
      as the scan plan's Phase 2 does for its defines. (2026-11-18: rusqlite comes from a git fork, the rusqlite entry in `[patch.crates-io]`; `vendor/` has no `libsqlite3-sys` to edit.)
  - **The link.** The MSVC linker can't read bitcode, so the LTO builds also need `-Clinker=lld-link`.
  - **What it can gain.** SQLite is one translation unit, so its own code is already optimized as a whole. The gain is at
    the Rust-to-SQLite boundary: inlining the per-row bind, step, column and reset calls into rusqlite.
    - Expected: a few percent at most.
    - The scan plan's Phases 1 and 3 (`docs/plans/SCAN_OPTIMIZATION_PLAN.md`) shrink that boundary further, so measure
      after them.
  - **What to check first:**
    - whether any static library the app links was built with MSVC's `/GL`, which `lld-link` can't read (the Qt static
      build is the one to check);
    - which rustc flag selects fat LTO across the languages;
    - for the container, a clang built on the same LLVM as rustc in `pg-tools`, since GCC objects can't take part.
  - **Plan: four release builds,** each measured on the scan probe's first scan and rescan at 200k (instruction counts in
    the container, wall time on Windows) and on `just perf`:
    1. MSVC, as today;
    2. clang, no LTO;
    3. clang with cross-language ThinLTO;
    4. clang with cross-language fat LTO.

    Build 2 separates the compiler change from LTO: a different compiler alone may make SQLite faster or slower.
  - **The rule for adopting one:** it must beat build 1 on both instruction counts and Windows time, with no loss
    elsewhere in `just perf`. Fat LTO is chosen over ThinLTO only if it also beats ThinLTO on both, since its link is
    serial and much heavier. Record the build-time cost of the winner.

## S-17.7: file identity and placeholders (moved from `docs/temp/S17_7_PLAN.md`, checked 2026-11-14, items marked 2026-11-18 rechecked)

The S-17.7 plan is removed. Its last committed copy is `docs/temp/S17_7_PLAN.md` at `7086cc5`, which also holds decisions 6 and 13 (the refusal policy), not repeated here. Windows probe results (volume serials, file IDs on overwrite, rename and delete) and WSL device numbers were not re-run; the scripts are in `docs/probes/`.

- **Cloud-file placeholders are read by the app (defect).** **Open.** The scanner takes its reparse flag from the listing's `FileAttributes` (`crates/platform/src/dir.rs:410`), and that flag is clear for these cloud files. On Dropbox, 2026-11-14: 15 online-only media files (14 on a recheck the same day; their `GetFileAttributes` value also includes `REPARSE_POINT`, 0x400) (`0x501620`, `Offline` and `RecallOnDataAccess` set) are listed as `reparse = false`, recorded as new rows, and left pending. The import reads every pending row (`pending_import`, `crates/catalog/src/work.rs:439`, called at `crates/import/src/lib.rs:82`). A one-byte read of one of them downloaded it (162,158 bytes, 1.5 s, attributes `0x501620` to `0x420`). The grid lists rows with no analysis (`crates/catalog/src/browse/read/grid.rs` (its `LEFT JOIN analysis` at line 41)), and a thumbnail request reads the original (`crates/app/src/images.rs:214`). I traced the grid path in code; I did not run the Qt app, so that download is not observed. Hydrated Dropbox files (2,396 media files, `0x420`) are local, not placeholders. 2026-11-18: `crates/platform/src` has no `OFFLINE` or `RECALL_ON_*` test, so nothing in the code has changed.
- **Cloud-file placeholder design (proposed).** **Open.** A cloud-file placeholder is a file whose bytes download when read: the file record has `FILE_ATTRIBUTE_OFFLINE` (0x1000), `RECALL_ON_DATA_ACCESS` (0x400000) or `RECALL_ON_OPEN` (0x40000). `REPARSE_POINT` is not the test. The state is read per entry from the file record, without opening the file's data. A cloud-file placeholder gets a row with a `placeholder` flag, new or known. Nothing reads it: the import, the on-demand thumbnail and full image (`images.rs`), the thumbs job, write-back and rotation all skip it. The flag is separate from `pending` (which means "not analyzed yet"), because a placeholder hydrated back with the same size and mtime still needs its import, and `pending` alone cannot say so. When a scan reads the file record without the placeholder bits, the flag clears. `ScanStats.skipped_reparse` should count by the new rule. 2026-11-18: in code, "placeholder" already names the ThumbHash pass (`Placeholders`, `crates/library/src/passes.rs:301`), so the flag needs another name.
- **Checks before building it.** **Open.** The cost of reading the file record per entry on a large library. Whether `GetFileAttributesEx` (or the file ID info) gives the same bits as the file record in every case. Why the listing's `FileAttributes` disagrees with it (not found; the file's `GetFileAttributes` value has `REPARSE_POINT` set, and the listing's does not, per the item above). Whether a reparse folder can be a placeholder (a Dropbox folder was not tested). Tests: the fake listings set `reparse` directly, so they cannot show this defect; the new tests need real attributes.
- **Older-binary end-to-end run.** **Not possible.** The history is squashed at `88e780b`, which already holds S-17.7, so no older-binary catalog exists. The rest of the end-to-end run was done on 2026-11-11 (`docs/reference/PROFILING.md`). (2026-11-18: `88e780b` is in the repo's history; I did not check the squash.)
- **Code and plan disagree on three points.** **Open.** (1) Tier 2 counts candidates only after the seen-volume and skipped-folder filters (`crates/catalog/src/pair.rs:157-197`; the filter is at `:197`); the plan said every row counts (`docs/temp/S17_7_PLAN.md` at `7086cc5`, line 81). (2) Replacement candidates must be unseen by this scan (`pair.rs:221-245`, which also takes live rows this scan did not see); the plan said "displaced or still unpaired" (line 82). (3) `replace()` also deletes `border_sides` and `file_failures` (`pair.rs:555`, `:560`), which the plan's reset list (decision 7, line 105) omits. Decide which is intended, then align the code or the record.
- **Stale comment.** **Open.** `crates/catalog/src/schema.rs:731` says the volume is the 64-bit serial (Windows) or the device number (Unix). Since `94a2530` it is the file system's UUID, with the device number as the fallback (`crates/platform/src/dir.rs:535-598`). (2026-11-18: the comment moved from `:550`.)
- **Not built, by decision** (kept so they are not reopened by accident): a fallback identity for FAT32, exFAT or ReFS (those are refused); repointing a fresh row's children; a repair pass for duplicates in an old catalog (an old catalog is rebuilt); a layout number (`LAYOUT_CHECK`): the schema is compared whole, and later changes are `MIGRATIONS` steps.
- **Accepted risks.** On drvfs, `created` is NULL, so a reused inode keeps the old row and its rating (not observed: 0 of 2000 delete-create cycles). Same-name reuse inherits the rating (decision 4, candidates a and b), since a rename-over and an inode reuse look the same to the scanner. The FAT32 refusal is tested through a fake `RootFacts`, since no FAT32 drive is on CI.

## Pipeline: crops, cuts, the pairing name and the status line (added 2026-11-17)

Found while reading the background job's round (`crates/library/src/runner.rs`). Agreed to do the first three; none is started. The rename and the crop/cut move are not prerequisites of the perf log (decided 2026-11-17), but they change the round. (2026-11-18: crops and cuts have already left the round, see the first two items.)

- **Cuts are not part of the import round.** **Partly resolved (2026-11-18).** There is no `Cuts` pass. A confirmed cut is a request on the crop job's own thread (`Runner::cut_borders`, `crates/library/src/runner.rs:112`, then `Coordinator::request_cut`, `coordinator.rs:290`). The cut still rewrites the original: `pg_crop::cut` (`crates/crop/src/lib.rs:228`) replaces it through `pg_writeback::replace`. Still open: queue each confirmed cut as a `file_writes` entry and let the writer apply it, as rotations do (`Catalog::queue_rotation`, `crates/catalog/src/queue.rs:103`). Check that the writer's queue can carry a cut's data (the cut's JPEG quality and TIFF/WebP handling, and the "no borders" state it stores).
- **Crops are not part of the import round.** **Resolved (2026-11-18).** `Lanes::request_crop` (`crates/library/src/coordinator.rs:307`) queues the request on the crop job, a service thread of its own (`Service::start("crop job", …)`, `coordinator.rs:185`), with its own queue (`registry().crops`). The "before anything else" promise in `Runner::save_crops` (`runner.rs:105`) is kept by holding the lanes while a crop works (`crates/library/src/lane.rs:6`, `:49`). That has not been tested.
- **Two meanings of "pairing".** **Open.** `pair_after_scan` (`crates/catalog/src/pair.rs:86`) rebinds a scanned file to its old row (volumes renumbered, files moved while the app was closed, files replaced). The `Pairing` pass (`crates/library/src/passes.rs:186`, `crates/import/src/pairs.rs`) makes duplicate pairs from perceptual hashes. Both use the word, and the catalog's `Pairing` struct (`pair.rs:46`) and the runner's `Pairing` pass share a name. Plan: rename the identity one (`rebind` or `reidentify`) and its result type and the `ScanStats` field (`stats.pairing`, `crates/scan/src/lib.rs:1166`). Keep "pairing" for the duplicate pass, which the `PAIRING_*` plans use. The perf log's span names for it (`pair.*`) follow the rename.
- **The status line is idle, or says "Scanning", while real work runs.** **Open.** After the scan walk, `pair_after_scan` and `mark_missing` run inside the scan's phase (`Phase::Scan`, `crates/model/src/progress.rs:17`), so the status keeps saying "Scanning" with no count moving. After the phase closes, the runner's post-scan work (`scanned()`, `forget_replaced`, which waits on the thumbnail cache's writer thread, temp files, notices) runs with the status idle. Before a pass's phase begins, its `pending_*` query runs (import, hashes, placeholders, borders), and the duplicate pairing's load and store, and the thumbnails' pending query, run too: the status shows nothing for those. The perf log (`PG_PERF_LOG`) shows each stretch; the fix is a status line that follows them, which is a UI change and is not part of the perf log.

## UI: the inbox button shows a bell icon, not text (added 2026-11-17)

- **Bell icon in place of the inbox's text.** **Open.** The status bar's inbox button (`crates/desktop/qml/StatusBar.qml`, the `Button` at line 162) shows the text from `ui-inbox` (`crates/client/locales/en/ui.ftl:84`): "Inbox", or "Inbox (N)" with the unread count. Plan: a bell icon, with the unread count beside it (or as a badge), in place of the text. The button's accessible name is its text today (`Accessible.name: text`, line 164), so it must move to a string such as `ui-inbox-title` (already "Inbox", `ui.ftl:88`) before the text goes. Open decisions: where the bell comes from. The UI has no icon set: the desktop crate's only images are the app icon (`crates/desktop/resources/app-icon.png`) and the pictures in `crates/desktop/resources/mascots/`, and `crates/` has no SVG. So the bell needs an SVG or a Qt icon added to the resources. It also needs to work in the dark theme and at the status bar's height. The `ui-inbox` string stays for the tooltip.

## UI: progress on the taskbar (added 2026-11-18)

- **Progress bar on the taskbar button.** **Open, not started.** Show the background job's progress on the app's taskbar
  button, using the Windows taskbar progress indicator (`ITaskbarList3::SetProgressValue` / `SetProgressState`). Nothing in
  the code sets it yet (rechecked 2026-11-18): `crates/desktop/cpp/app_icon.cpp` only sets the window icon. Open decisions: which job progress to
  show (the background job's round as a whole, or the current pass), and the states to show (paused, error, indeterminate).

## UI: remember the window's size and position (added 2026-11-22)

- **Window size and position are not kept between launches.** **Open, not started.** The app does not save the main
  window's size, position or maximized state, and does not restore them at start (checked 2026-11-22: the only geometry
  match in `crates/` is Qt's `--qwindowgeometry` option in `crates/desktop/src/command_line.rs`). Open decisions: where
  to store the values (the app's settings, or the catalog), how to handle a saved position that is off screen after a
  monitor change, and whether the state is per monitor setup or global.

## Yandex: the number of similar images is hardcoded (added 2026-11-27)

- **Make the similar-image count configurable.** **Open, not started.** The reverse search always reads the same number
  of similar pages: `DEFAULT_MAX_SIMILAR_PAGES = 5` (`crates/yandex/src/client.rs:25`), passed to `source.similar(...)` at
  `crates/app/src/yandex.rs:324`. No setting or option changes it. The unit is pages, not images: how many images a page
  holds comes from Yandex's answer, so the count the user sees depends on that. Open decisions: whether the setting is a
  page count or an image count (an image count needs the page size, which the code does not read), its default (5 today),
  a maximum (each page is one more request to Yandex), and where it is stored (the app's preferences, or per run only,
  as `--find-similar` would take it).
