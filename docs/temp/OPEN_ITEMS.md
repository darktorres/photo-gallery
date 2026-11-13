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
