# Open items

Open items moved from the former `FINDINGS.md` log. Some may overlap with the open decisions in [PLAN.md](../temp/PLAN.md) §9; check before acting on one.

Checked against the code and the eval outputs on 2026-11-12. Items that name 2026-11-14 were rechecked that day. Each item says what the code and the outputs show. The eval outputs are in `D:\PhotoGallery` (`eval`, `eval_smoke`, `eval_ann`, `cache`).

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
  small text-query test (../eval/MODELS.md, NaFlex section) has not been run. PLAN.md §9.2 says TensorRT fp16 was decided on 2026-10-30, but MODELS.md's precision section shows that choice loses precision, so PLAN §9.2 needs correcting.
- Hand marks also decide the remaining grouping errors: different frames (670+671) vs local warps (429), and
  template portraits with different faces (the 12-image group). **Open.** 670+671 and 429 are in ../eval/VERIFIER.md's grouping
  section (lines 72–88). The template-portrait case is at ../eval/VERIFIER.md:78; the AI-portrait group (12) is at VERIFIER.md:90, so
  use the group list there before marking it.
- (Deferred) duplicate vs variant: learned decision on per-region features + face-focused comparison (see
  "Verifier prototype, first run"). **Deferred.** That section (../eval/VERIFIER.md:123) is still the only record.
- Hand-label a sample of `review_*.csv` to estimate real precision. **Open.** `eval/evaluate.py` writes
  `review_<model>.csv` (line 187), but no such file exists in `D:\PhotoGallery\eval` or `eval_smoke`, so no sample has
  been drawn for any model.
- Thumbnail cache format. **Open.** The cache is JPEG at quality 85, 4:2:0
  (`crates/render/src/maker.rs:20`), in its own SQLite file (`crates/thumbcache`). THUMBNAILS.md
  says disk is the binding constraint and AVIF is ~38% smaller than JPEG at score 80 (but encodes ~100× slower), and
  the reason is in the code comment at `crates/render/src/maker.rs:18-19` (fastest to make and to show). PLAN §9.1 still lists the format as open.
- Candidate retrieval, exact vs approximate (PLAN §9 decision 5): the redo runs on the test set's real
  embeddings plus planted edits, with tuned indexes (the first attempt was invalid, "Nearest-neighbour search,
  first attempt"). **Open.** The harness is committed (`eval/ann/`), and the embeddings and planted edits exist in
  `D:\PhotoGallery\eval_ann`. No tune results exist, and `cache/ann` (validate.json, charact.json) is not in
  `D:\PhotoGallery\cache`. TIER2_ANN_BAKEOFF_PLAN.md's status was checked 2026-11-06, when it said the harness was uncommitted. It was rechecked 2026-11-14 and now says the harness is committed (f0e4b0f).
- Semantic search's index (So400m, 1152-d): FAISS IVF on real vectors once they exist. **Open, blocked.** No So400m
  vectors exist, and the crates have no semantic search.
- Full-set runs (on hold). **Unclear.** No doc says "on hold". What exists: full-set SSCD embeddings
  (`D:\PhotoGallery\eval\emb_sscd_fp16.npy`, 187,806 rows) and a cross-group verify (2026-11-09, `verify_cross_sscd_fp16.json`). That file records 4,637 pairs, the subset's count in VERIFIER.md, and no set size, so it is not shown to be full-set. Not run: the
  full k-NN verify (187,808 images, "hours per run", ../eval/VERIFIER.md). ../eval/TEST_SET.md's "full set, not run yet" is partly stale: the full-set embeddings exist. Confirm
  whether this is still on hold.
- Video player: Qt Multimedia plays videos now, with hardware decoding turned off on a GPU without AV1 decoding
  (a stopgap, "Videos and animations"). Choose between our own engine on our FFmpeg 9 and libmpv with its D3D11
  render API ([VIDEO_PLAYER.md](plans/VIDEO_PLAYER.md)). **Open.** Qt Multimedia is in use (`crates/desktop/build.rs`,
  `qml/MediaView.qml`), and on a GPU without AV1 decoding the player decodes on the CPU (`crates/desktop/src/gpu_device.rs`).
  Neither libmpv nor our own engine is in the code, and VIDEO_PLAYER.md says "not decided".
- Yandex: whether the browser shows all 18 other sizes or about 5 is not measured; only one photo was tested.
  **Open.** This needs a live check with the desktop app (`--find-similar`). A search of the Rust code found no cap on the
  other sizes, and REVERSE_SEARCH.md records 18 for the one photo.
- Page cache per catalog connection (formerly the start profile's lead 6). **Open.** `SETTINGS` sets
  `cache_size = -262144` (256 MB) on every connection (`crates/catalog/src/schema.rs:424-425`), applied in `Catalog::open`
  and `prepare` (`crates/catalog/src/core.rs:101`, `:140`). Only the start's reader releases its pages, after each job
  (`crates/app/src/reader.rs:183`). The thumbnail reader connections set -131072 (`crates/thumbcache/src/reader.rs:27`)
  and only read a few rows each. SQLite's memory is outside dhat, so this has not been measured. Measure the resident
  set of the start with a smaller cache before changing it.
- Tier-1 index offsets per import (from the import profile). **Open.** `PdqIndex::reindex` (`crates/dedup/src/pdq_index.rs:229-240`)
  allocates 16 offset tables of 65,537 `u32`s (4.2 MB) whenever the index has entries, including the rebuild from existing
  hashes. Only the empty index is free now. Measure an import into a copy of the pinterest catalog (190k files) before
  deciding. The rebuild's `(i64, Hash)` list (about 7.6 MB, an estimate) is not measured either.
- Resizer buffers are never trimmed (from the import profile). **Open.** `Analyzer::trim` (`crates/analysis/src/analyze.rs:156-162`)
  frees the image, the spare, the PDQ and ThumbHash buffers, the decoder and the detector, but not the resizer. Its `src` and
  `dst` (`crates/codec/src/resize.rs:22-23`, 12 bytes a pixel) are kept at full size per worker. A 24 MP non-JPEG file could
  leave about 290 MB held per worker (an estimate from the decode size, not measured).
- Windows path copies per file (from the import profile). **Open.** `set_file` (`crates/platform/src/names.rs:27-46`) builds
  the folder's and the name's UTF-16 buffers for every file, then copies them into the reused `NativePath`. Lead: write the
  units straight from the work list's arena.
- Dihedral hashes collected per analysis (from the import profile). **Open.** `crates/catalog/src/writers.rs:333` collects each
  stored analysis's eight dihedral hashes into a 256-byte `Vec<u8>`. Bind the slices instead, as the PDQ hash already is.
- Import at scale, not profiled. **Open.** The 305-JPEG sample has no EXIF, no earlier hashes, no large files and no
  non-JPEG decodes, so the import's costs for those are estimates. Profile an import of a copy of the pinterest catalog.
- On-demand thumbnails and viewer reads, not profiled. **Open.** The heap profile covered the scan, the import and the
  borders and thumbnail passes, and the app's start. The app's on-demand thumbnail requests and viewer reads were checked
  by screenshots and the cached bytes only.
- Grid thumbhash shows when scrolling up, never when scrolling down (opened 2026-11-15). **Open, not measured.** Seen while
  fast-scrolling the grid. From the code: a cell shows its thumbhash until its thumb is `Ready` and has faded in
  (`crates/desktop/qml/GridCell.qml`); cells and their requests are made 1500 px ahead of the view (`GalleryGrid.qml`,
  `cacheBuffer`); thumb requests run on a LIFO pool whose comment assumes the newest requests are the ones on screen
  (`crates/desktop/cpp/providers.cpp:4-5`). Nothing has shown which of these gives the direction difference. Plan: thumbnail
  all of `D:\PhotoGallery\datasets\planted` first (18,000 images; `gallery scan`, `import`, `thumbs --height 220`, with
  nothing else running), then time a fast scroll down and back up with per-request timestamps. Temporary logs tagged
  `DEBUGLOG` are in those three files and must be removed afterwards.

## S-17.7: file identity and placeholders (moved from `docs/temp/S17_7_PLAN.md`, checked 2026-11-14)

The S-17.7 plan is removed. Its last committed copy is `docs/temp/S17_7_PLAN.md` at `7086cc5`, which also holds decisions 6 and 13 (the refusal policy), not repeated here. Windows probe results (volume serials, file IDs on overwrite, rename and delete) and WSL device numbers were not re-run; the scripts are in `docs/probes/`.

- **Placeholders are read by the app (defect).** **Open.** The scanner takes its reparse flag from the listing's `FileAttributes` (`crates/platform/src/dir.rs:398`), and that flag is clear for these cloud files. On Dropbox, 2026-11-14: 15 online-only media files (14 on a recheck the same day; their `GetFileAttributes` value also includes `REPARSE_POINT`, 0x400) (`0x501620`, `Offline` and `RecallOnDataAccess` set) are listed as `reparse = false`, recorded as new rows, and left pending. The import reads every pending row (`pending_import`, called at `crates/import/src/lib.rs:33`). A one-byte read of one of them downloaded it (162,158 bytes, 1.5 s, attributes `0x501620` to `0x420`). The grid lists rows with no analysis (`crates/catalog/src/browse/read/grid.rs` (its `LEFT JOIN analysis` at line 41)), and a thumbnail request reads the original (`crates/app/src/images.rs:214`). I traced the grid path in code; I did not run the Qt app, so that download is not observed. Hydrated Dropbox files (2,396 media files, `0x420`) are local, not placeholders.
- **Placeholder design (proposed).** **Open.** A placeholder is a file whose bytes download when read: the file record has `FILE_ATTRIBUTE_OFFLINE` (0x1000), `RECALL_ON_DATA_ACCESS` (0x400000) or `RECALL_ON_OPEN` (0x40000). `REPARSE_POINT` is not the test. The state is read per entry from the file record, without opening the file's data. A placeholder gets a row with a `placeholder` flag, new or known. Nothing reads it: the import, the on-demand thumbnail and full image (`images.rs`), the thumbs job, write-back and rotation all skip it. The flag is separate from `pending` (which means "not analyzed yet"), because a placeholder hydrated back with the same size and mtime still needs its import, and `pending` alone cannot say so. When a scan reads the file record without the placeholder bits, the flag clears. `ScanStats.skipped_reparse` should count by the new rule.
- **Checks before building it.** **Open.** The cost of reading the file record per entry on a large library. Whether `GetFileAttributesEx` (or the file ID info) gives the same bits as the file record in every case. Why the listing's `FileAttributes` disagrees with it (not found; the file's `GetFileAttributes` value has `REPARSE_POINT` set, and the listing's does not, per the item above). Whether a reparse folder can be a placeholder (a Dropbox folder was not tested). Tests: the fake listings set `reparse` directly, so they cannot show this defect; the new tests need real attributes.
- **Older-binary end-to-end run.** **Not possible.** The history is squashed at `88e780b`, which already holds S-17.7, so no older-binary catalog exists. The rest of the end-to-end run was done on 2026-11-11 (`docs/reference/PROFILING.md`).
- **Code and plan disagree on three points.** **Open.** (1) Tier 2 counts candidates only after the seen-volume and skipped-folder filters (`crates/catalog/src/pair.rs:163-167`); the plan said every row counts. (2) Replacement candidates (a) must be unseen by this scan (`pair.rs:199`); the plan said "still unpaired". (3) `replace()` also deletes `border_sides` and `file_failures` (`pair.rs:428`, `:433`), which the plan's reset list omits. Decide which is intended, then align the code or the record.
- **Stale comment.** **Open.** `crates/catalog/src/schema.rs:550` says the Unix volume is the device number. Since `94a2530` it is the file system's UUID, with the device number as the fallback (`crates/platform/src/dir.rs:535-598`).
- **Not built, by decision** (kept so they are not reopened by accident): a fallback identity for FAT32, exFAT or ReFS (those are refused); repointing a fresh row's children; a repair pass for duplicates in an old catalog (an old catalog is rebuilt); a layout number (`LAYOUT_CHECK`): the schema is compared whole, and later changes are `MIGRATIONS` steps.
- **Accepted risks.** On drvfs, `created` is NULL, so a reused inode keeps the old row and its rating (not observed: 0 of 2000 delete-create cycles). Same-name reuse inherits the rating (decision 4, candidates a and b), since a rename-over and an inode reuse look the same to the scanner. The FAT32 refusal is tested through a fake `RootFacts`, since no FAT32 drive is on CI.
