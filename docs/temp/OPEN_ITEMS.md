# Open items

Open items moved from the former `FINDINGS.md` log. Some may overlap with the open decisions in [PLAN.md](PLAN.md) §9; check before acting on one.

Checked against the code and the eval outputs on 2026-11-12. Each item says what the code and the outputs show. The eval outputs are in `D:\PhotoGallery` (`eval`, `eval_smoke`, `eval_ann`, `cache`).

- Hand marks on the k-NN candidates (viewer), then tune or learn the verifier decision on them. **Open.** The viewer
  already takes marks on verifier pairs (`eval/viewer.py`, one `pair_marks.json` per run), but no run has a marks file yet.
- Verifier speed, next levers. **Open.** DISK runs on TensorRT by default (`eval/verify.py`, `--disk-runtime trt`). The
  6.7 pairs/s figure was measured before the 262004b verifier fix and has not been re-measured. LightGlue still runs in
  PyTorch (kornia); `eval/` has no LightGlue-ONNX or TensorRT path. 768 px was tried and not adopted (VERIFIER.md,
  "not adopted"). The rotation retries are not gated: `--rotations` defaults to `none`, and when it is on, every
  non-matching pair pays for the retries.
- Verifier robustness: the median-residual measure swings for ~0.3% of pairs when a few keypoints change
  (TensorRT vs torch flips); look at those with the hand marks. **Open.** The 0.3% figure was measured before the fix
  (VERIFIER.md, the rotations section, which says so). It has not been re-measured since, and there are no hand marks.
- Second model (So400m NaFlex): choose a runtime. **Open.** No So400m code exists in the crates, only in `eval/` and
  the docs. The cause is recorded in MODELS.md ("Why So400m loses precision on TensorRT"), and so is the ONNX
  Runtime GELU-fusion result (84 img/s at torch fp16's fidelity). Not recorded in any doc: the neighbour-agreement
  numbers (0.939 vs 0.984; top-5 0.961 vs 0.990; 0.949 vs 0.996 on the subset), the torch fp16 cosine of 0.9993, and
  the claim that fp32 LayerNorm doesn't help. Those need a measurement note or a rerun before they are cited. The
  small text-query test (MODELS.md, NaFlex section) has not been run.
- Hand marks also decide the remaining grouping errors: different frames (670+671) vs local warps (429), and
  template portraits with different faces (the 12-image group). **Open.** 670+671 and 429 are in VERIFIER.md's grouping
  section (lines 72–88). The template-portrait case is at VERIFIER.md:78; the "12-image group" is not named there, so
  find it in the grouping output before marking it.
- (Deferred) duplicate vs variant: learned decision on per-region features + face-focused comparison (see
  "Verifier prototype, first run"). **Deferred.** That section (VERIFIER.md:123) is still the only record.
- Hand-label a sample of `review_*.csv` to estimate real precision. **Open.** `eval/evaluate.py` writes
  `review_<model>.csv` (line 187), but no such file exists in `D:\PhotoGallery\eval` or `eval_smoke`, so no sample has
  been drawn for any model.
- Thumbnail cache format and cap. **Format still open; cap implemented.** The cache is JPEG at quality 85, 4:2:0
  (`crates/render/src/maker.rs:20`), in its own SQLite file (`crates/thumbcache`). The cap defaults to 5% of the volume,
  between 1 and 50 GB (`crates/thumbcache/src/config.rs`), and is set with `--cap-gb` or `PG_THUMB_CAP_GB`. THUMBNAILS.md
  says disk is the binding constraint and AVIF is ~38% smaller than JPEG at score 80 (but encodes ~100× slower), and
  no doc records why JPEG was kept. PLAN §9.1 still lists the format as open.
- Candidate retrieval, exact vs approximate (PLAN §9 decision 5): the redo runs on the test set's real
  embeddings plus planted edits, with tuned indexes (the first attempt was invalid, "Nearest-neighbour search,
  first attempt"). **Open.** The harness is committed (`eval/ann/`), and the embeddings and planted edits exist in
  `D:\PhotoGallery\eval_ann`. No tune results exist, and `cache/ann` (validate.json, charact.json) is not in
  `D:\PhotoGallery\cache`. TIER2_ANN_BAKEOFF_PLAN.md's status (checked 2026-11-06) says the harness is uncommitted, which
  is no longer true.
- Semantic search's index (So400m, 1152-d): FAISS IVF on real vectors once they exist. **Open, blocked.** No So400m
  vectors exist, and the crates have no semantic search.
- Full-set runs (on hold). **Unclear.** No doc says "on hold". What exists: full-set SSCD embeddings
  (`D:\PhotoGallery\eval\emb_sscd_fp16.npy`, 187,806 rows) and a full-set cross-group verify (2026-11-09). Not run: the
  full k-NN verify (187,808 images, "hours per run", VERIFIER.md). TEST_SET.md's "full set, not run yet" is stale. Confirm
  whether this is still on hold.
- Video player: Qt Multimedia plays videos now, with hardware decoding turned off on a GPU without AV1 decoding
  (a stopgap, "Videos and animations"). Choose between our own engine on our FFmpeg 9 and libmpv with its D3D11
  render API ([VIDEO_PLAYER.md](VIDEO_PLAYER.md)). **Open.** Qt Multimedia is in use (`crates/desktop/build.rs`,
  `qml/MediaView.qml`), and on a GPU without AV1 decoding the player decodes on the CPU (`crates/desktop/src/gpu_device.rs`).
  Neither libmpv nor our own engine is in the code, and VIDEO_PLAYER.md says "not decided".
- Yandex: whether the browser shows all 18 other sizes or about 5 is not measured; only one photo was tested.
  **Open.** This needs a live check with the desktop app (`--find-similar`). A search of the Rust code found no cap on the
  other sizes, and REVERSE_SEARCH.md records 18 for the one photo.
- Page cache per catalog connection (formerly the start profile's lead 6). **Open.** `SETTINGS` sets
  `cache_size = -262144` (256 MB) on every connection (`crates/catalog/src/schema.rs:378`), applied in `Catalog::open`
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
