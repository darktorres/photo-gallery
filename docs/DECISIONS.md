# Decisions

Dated decisions, in the order they were made. Each one rests on a measurement in the topic files; the links are in the bullets.


- **2026-10-28 (revised the same day): no duplicate / variant distinction for now.** Local-edit
  variants (retouch, inpainting, AI regeneration) and duplicates are one relation, "match". The
  verifier decides match vs unrelated; its residual analysis stays as a diagnostic, for when the
  distinction is revisited. (Briefly decided earlier: variants as a separate third relation.)
  - Consequence: **`similiar_but_not_dups` is no longer a negative set.** Most pairs in it above
    SSCD 0.4 are variants = matches (253/345 with the corrected verifier rule). The "hard negatives
    flagged" numbers in the model comparison above therefore count matches, not errors, and a
    new source of true negatives is needed.

- **2026-10-28: quality over speed.** Days or weeks of processing are acceptable. Heavier
  models (SSCD-large, fp32), exact rather than approximate k-NN (very rough estimate: about
  3 days brute force for 70M on the 2060, streaming from the NVMe; measured 2026-10-31: ~5.5 days of GEMM,
  "Nearest-neighbour search, first attempt"), multiple embeddings per
  image, and a learned geometric verifier are all in scope. Resumability and the user's
  review time (precision) remain the real constraints.
- **2026-10-28: stay on Windows.** CUDA, nvJPEG, PyTorch, TensorRT and ONNX Runtime all work.
  Linux-only tools (FAISS-GPU, cuVS, DALI) aren't needed so far. If one ever is, run only that
  batch step in WSL2 on embeddings, not on images: reading NTFS drives through `/mnt` is slow.
- **2026-10-28: add a geometric verification stage**, local feature matching + RANSAC (e.g.
  SuperPoint/LightGlue, LoFTR), for pairs in the uncertain band only. It checks heavy crops,
  overlays and false positives, and reports which image contains the other. Open risk: AI
  variants with redrawn details may have few matchable points.
- **2026-10-28: benchmark decoders** (PIL full, PIL `draft()`, libvips, nvJPEG) for speed and
  embedding drift. Done 2026-10-29, see "Decode benchmark": DCT-scaled decode wins.
- **2026-10-30: second model = SigLIP 2 So400m/16 NaFlex** (256 patches, TensorRT fp16) for keywords and semantic
  search, alongside SSCD (PLAN §9 decision 2). Chosen over B/16 for quality (ImageNet zero-shot 83.5 vs 78.5 at
  256 patches), over the fixed So400m for the native aspect ratio at the same cost. ~154 GPU hours for 70M.
- **2026-10-31: the Yandex client sends the `yp` szm cookie and never `spravka`** ("Reverse image search: Yandex").
