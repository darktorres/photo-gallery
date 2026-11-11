# Open items

Open items moved from the former `FINDINGS.md` log. Some may overlap with the open decisions in [PLAN.md](PLAN.md) §9; check before acting on one.


- Hand marks on the k-NN candidates (viewer), then tune or learn the verifier decision on them.
- Verifier speed, next levers (DISK is on TensorRT, 6.7 pairs/s): LightGlue on TensorRT or fewer keypoints (768 px
  was tried and dropped, ~9% faster); gate the rotation retries (e.g. by SSCD similarity).
- Verifier robustness: the median-residual measure swings for ~0.3% of pairs when a few keypoints change
  (TensorRT vs torch flips); look at those with the hand marks.
- Second model (So400m NaFlex): TensorRT fp16 reaches cosine 0.997 vs fp32, while torch fp16 and ONNX Runtime CUDA
  on the same fp16 graph reach 0.9993 (fp32 LayerNorm doesn't help). Cause found 2026-10-31: TensorRT's matmuls
  accumulate in fp16 ("Why So400m loses precision on TensorRT"); choose a runtime. It changes
  neighbours: on 512 random images (2026-10-31, quiet machine) the nearest neighbour matches fp32 for 0.939 with
  TensorRT vs 0.984 with torch fp16 (top-5 overlap 0.961 vs 0.990; 0.949 vs 0.996 on 512 subset images). The
  flipped ones are near-ties (fp32 top-1/top-2 gap median 0.0013 vs 0.011 overall), but the loss is ~4x torch's.
  Speed confirmed at 126 img/s with the engine alone; with the fp32 model
  also on the 6 GB GPU, Windows spills to shared memory and it drops to ~1 img/s. ONNX Runtime with the
  optimizer's GELU fusion reaches 84 img/s at torch fp16's fidelity; attention isn't fused (2026-10-31). A small
  text-query test on this library would check the semantic quality and the 256-patch budget.
- Hand marks also decide the remaining grouping errors: different frames (670+671) vs local warps (429), and
  template portraits with different faces (the 12-image group).
- (Deferred) duplicate vs variant: learned decision on per-region features + face-focused
  comparison (see "Verifier prototype, first run").
- Hand-label a sample of `review_*.csv` to estimate real precision.
- Thumbnail cache format and cap (size is runtime-adjustable, decided 2026-10-30).
- Candidate retrieval, exact vs approximate (PLAN §9 decision 5): the redo runs on the test set's real
  embeddings plus planted edits, with tuned indexes (the first attempt was invalid, "Nearest-neighbour search,
  first attempt").
- Semantic search's index (So400m, 1152-d): FAISS IVF on real vectors once they exist.
- Full-set runs (on hold).
- Video player: Qt Multimedia plays videos now, with hardware decoding turned off on a GPU without AV1 decoding
  (a stopgap, "Videos and animations"). Choose between our own engine on our FFmpeg 9 and libmpv with its D3D11
  render API ([VIDEO_PLAYER.md](VIDEO_PLAYER.md)).
- Yandex: whether the browser shows all 18 other sizes or about 5 is not measured; only one photo was tested.
