# Models and accelerators

Measured results for the models: their speed, their retrieval quality, and the accelerators they run on. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [TEST_SET.md](TEST_SET.md). The PDQ tier and the shortcut and grouping are in [VERIFIER.md](VERIFIER.md).

## Model speed (RTX 2060, GPU only, synthetic input, batch 64)

| Model                        | img/s   | GPU hours for 70M |
|------------------------------|---------|-------------------|
| SSCD fp32 320px              | 241     | 81                |
| **SSCD fp16 320px**          | **410** | **47**            |
| SSCD fp16 288px              | 499     | 39                |
| SSCD fp16 256px              | 653     | 30                |
| SSCD large (ResNeXt101) fp32 | 105     | 185               |
| DINOv2-S fp16 224px          | 1020    | 19                |
| DINOv2-B fp16 224px          | 356     | 55                |
| CLIP B/32 fp16 224px         | 1498    | 13                |

## Model quality

### True negatives and new candidates (subset, 2026-10-29)

`eval/evaluate.py` now also counts **cross-group pairs** (915,279 pairs of images from two different
`dups2` groups, i.e. different pictures) above each threshold, the false-positive measure that
replaces `similiar_but_not_dups` (mostly matches now, see VERIFIER.md, "Superseded the same day").

| Model         | R@1 / R@10    | Thr @95%       | Loose dups2 with partner | Cross-group flagged @90% | @95%  |
|---------------|---------------|----------------|--------------------------|--------------------------|-------|
| sscd (fp32)   | 0.964 / 1.000 | 0.879          | 92.4%                    | 0                        | 0     |
| **sscd_fp16** | 0.964 / 1.000 | 0.879          | 92.4%                    | **0**                    | **0** |
| sscd_288_fp16 | 0.964 / 1.000 | 0.874          | 92.5%                    | 0                        | 0     |
| sscd_large    | 0.962 / 1.000 | 0.859          | 91.3%                    | 0                        | 2     |
| dinov2_s      | 0.962 / 1.000 | 0.945          | 92.8%                    | 4                        | 4     |
| dinov2_b      | 0.963 / 1.000 | 0.943          | 93.1%                    | 0                        | 4     |
| clip_b32      | 0.959 / 1.000 | 0.929          | 89.5%                    | 7                        | 17    |
| pdq           | 0.924 / 0.957 | 0.234 (d ≤ 98) | 97.9%                    | 1                        | 345   |

- **SSCD flags no different-picture pair at any operating point up to 95% recall**; DINOv2 4,
  CLIP 17. The cross-group set is easy at threshold level (most of it is far apart); the hard
  cases are the few near pairs that k-NN hands the verifier (see the verifier sections).
- **SSCD-large (ResNeXt101, 1024-d): no gain.** Same R@10 = 1.000, slightly fewer loose files
  paired (91.3% vs 92.4%), 2 cross-group false positives at 95%, and 58 img/s end to end here
  (GPU-only 105 img/s, vs 410 for SSCD fp16). Dropped.

### Model comparison (subset, after label fixes, 2026-10-28)

All six models at the same recall on labeled pairs (higher "loose with partner" is better;
lower "flagged" and "hard neg" are better):

| Model         | Thr @95% | Loose dups2 with partner | Flagged unlabeled | Hard neg flagged (of 68,265) |
|---------------|----------|--------------------------|-------------------|------------------------------|
| sscd (fp32)   | 0.879    | 92.4%                    | 2,653             | 41                           |
| sscd_fp16     | 0.879    | 92.4%                    | 2,653             | 42                           |
| sscd_288_fp16 | 0.874    | 92.5%                    | 2,657             | 41                           |
| dinov2_s      | 0.945    | 92.8%                    | 2,742             | 78                           |
| dinov2_b      | 0.943    | 93.1%                    | 2,806             | 74                           |
| clip_b32      | 0.929    | 89.5%                    | 2,861             | 78                           |

The same pattern holds at 80% and 90% recall (e.g. hard negatives at 90%: SSCD 26,
DINOv2 51–52, CLIP 54).

- **SSCD fp32 = fp16 = 288px in quality.** Precision and size don't matter here, so the
  faster variants are free.
- **SSCD has the best precision:** about half the hard-negative false positives of DINOv2 or
  CLIP at every recall level.
- **DINOv2-B has slightly better recall** on loose dups2 files, but with roughly 2× the false
  positives. It is better used as a second signal in the verifier than as the primary
  descriptor.
- **CLIP is worst on both counts**, as expected from a semantic model.
- **Retrieval rank doesn't separate the models at subset size:** R@1 ≈ 0.96 and R@10 = 1.000
  for all of them. Differences show only in threshold behavior. Expect rank to separate them
  on the full 190k set.

### SSCD fp16 detail (subset, after label fixes, 2026-10-28)

- **Labeled pairs:** R@1 **0.964**, R@10 **1.000**. Every labeled partner is in the top 10.
- **Operating points:**

| Target recall | Threshold | Loose dups2 with partner | Flagged unlabeled pairs | Hard negatives flagged |
|---------------|-----------|--------------------------|-------------------------|------------------------|
| 0.90          | 0.909     | 88.3%                    | 2,496                   | 26 / 68,265 (0.04%)    |
| 0.95          | 0.879     | 92.4%                    | 2,653                   | 42 / 68,265 (0.06%)    |

Flagged unlabeled pairs include real but unlabeled duplicates. Precision is unknown until
`review_*.csv` is labeled.

- **Score separation (pre-fix run, same subset):** hard negatives p99 ≈ 0.30; positives
  p10 ≈ 0.89.
- **What it misses (viewer, scores 0.48–0.78):** strong color grading (e.g. yellow vs
  orange shirt), slight re-crops/zooms of the same photo, AI-image variants with small
  differences.

#### Why some duplicates score low and some non-duplicates score high (2026-10-28)

Looked at the extremes of SSCD fp16 on the subset (`misses_*.csv`, `hard_negatives_*.csv`):

- **Highest non-duplicates (0.94–0.99, CLIP up to 0.996):** the *same* picture with a
  **local content edit**: FaceApp face retouch, AI regenerations or inpainting that change a
  hand, face or detail. The rest of the image is pixel-identical.
- **Lowest duplicates (0.48–0.78):** the same photo under a **global edit**: heavy color
  grading (yellow shirt and grass → orange shirt and dark grass, 0.48), re-crop or zoom plus
  grading (0.57–0.70).

A global embedding measures *how much of the image changed*. A local edit changes little, so it
scores high. A strong grade changes every pixel, so it scores low. This library's definition is
the opposite: global edits = duplicate, local content edits = not a duplicate. Copy-detection
models like SSCD are trained to *ignore* local edits (overlays, pasted content), so this is not
a calibration problem. **No threshold, normalization or better embedding model fixes it.** It
needs a stage that aligns the two images and looks at *where* they differ (see
`../plans/PLAN.md` and the verifier in [VERIFIER.md](VERIFIER.md)).

### Synthetic edits, all models (subset, 2026-10-29)

300 random images, each edit applied, then the edited copy is searched in the 9.3k-image subset.
Each cell is **R@10 · share above the model's 95%-recall threshold**. R@1 is lower for every model
and edit (0.75–0.93, even for JPEG q30) because exact duplicates of a sampled image already exist
in the index and tie or win rank 1, so R@10 is the retrieval measure here.

| Edit              | sscd_fp16       | sscd_large      | dinov2_s    | dinov2_b    | clip_b32    | pdq           |
|-------------------|-----------------|-----------------|-------------|-------------|-------------|---------------|
| JPEG q30          | 1.00 · 1.00     | 1.00 · 1.00     | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.99 | 1.00 · 1.00   |
| Downscale 25%     | 1.00 · 1.00     | 1.00 · 1.00     | 1.00 · 1.00 | 1.00 · 0.99 | 1.00 · 0.76 | 1.00 · 1.00   |
| Grayscale         | 1.00 · 0.97     | 1.00 · 0.96     | 1.00 · 0.60 | 1.00 · 0.81 | 0.99 · 0.02 | 1.00 · 1.00   |
| Color shift       | 1.00 · 1.00     | 1.00 · 1.00     | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.99 | 1.00 · 1.00   |
| Text overlay      | 1.00 · 0.00     | 1.00 · 0.00     | 1.00 · 0.80 | 1.00 · 0.73 | 0.97 · 0.00 | 1.00 · 1.00   |
| Letterbox 10%     | 1.00 · 0.00     | 1.00 · 0.09     | 1.00 · 0.55 | 1.00 · 0.66 | 1.00 · 0.60 | 0.11 · 0.06   |
| Mirror            | 1.00 · 1.00     | 1.00 · 1.00     | 1.00 · 0.65 | 1.00 · 0.98 | 1.00 · 0.99 | 0.00 · 0.00 ¹ |
| Rotate 90°        | 1.00 · 0.00     | 1.00 · 0.01     | 0.95 · 0.00 | 0.95 · 0.00 | 0.99 · 0.01 | 0.00 · 0.00 ¹ |
| Crop 80%          | 1.00 · 0.77     | 1.00 · 0.98     | 1.00 · 0.70 | 1.00 · 0.83 | 1.00 · 0.90 | 0.33 · 0.26   |
| Crop 60%          | 1.00 · 0.00     | 1.00 · 0.09     | 1.00 · 0.30 | 0.99 · 0.38 | 0.99 · 0.49 | 0.02 · 0.01   |
| Crop 40%          | 1.00 · 0.00     | 1.00 · 0.00     | 1.00 · 0.00 | 0.95 · 0.04 | 0.97 · 0.08 | 0.00 · 0.00   |
| Crop 25%          | **1.00** · 0.00 | **1.00** · 0.00 | 0.95 · 0.00 | 0.80 · 0.00 | 0.85 · 0.00 | 0.00 · 0.00   |
| Crop 50% + mirror | 1.00 · 0.00     | 1.00 · 0.00     | 1.00 · 0.00 | 0.99 · 0.10 | 0.99 · 0.20 | 0.00 · 0.00   |

¹ PDQ queried with all 8 rotations/mirrors (`compute_dihedral`): mirror and rotate 90° → R@10 1.00,
d p50 0, within d ≤ 31 99.7%. The other rows don't change.

- **Only SSCD keeps every edited copy in the top 10, down to a 25% crop.** DINOv2 and CLIP fall
  to 0.80–0.95 on heavy crops and rotation. Rank is what the pipeline uses (k-NN + verifier), so
  this is the deciding result: SSCD stays the candidate model.
- **No global threshold survives heavy crops, overlays or letterboxing** for SSCD: the
  95%-recall threshold (0.879) is set by labeled pairs, which are mostly light edits. Confirms
  design conclusion 2 (rank + verifier, not a lower threshold) and 3 (trim borders first).
- **SSCD-large vs SSCD:** same retrieval; slightly higher similarity on crops (crop 80% above the
  threshold 0.98 vs 0.77). Not enough to justify about 4× the cost (185 vs 47 GPU hours for 70M, in the speed table above).

## Accelerators and a second model

### TensorRT and ONNX Runtime for SSCD (`eval/accel.py`, 2026-10-30)

Same weights exported to ONNX (`accel.SSCDExport`: BatchNorm folded, fp16-safe GeM; see "Bugs found"),
RTX 2060, batch 64, 320 px, GPU only. Measured first on the CUDA 12 stack (driver 576.57), then again after
the driver update on CUDA 13 (driver 621.01: torch 2.15 cu132, onnxruntime-gpu 1.31.1, TensorRT 11.4 / 10.16
cu13):

| Runtime                                               | img/s, CUDA 12 → 13 | GPU hours for 70M (CUDA 13) | Cosine vs torch fp32 (9.3k subset): min / p1 |
|-------------------------------------------------------|---------------------|-----------------------------|----------------------------------------------|
| torch fp32                                            | 241 → 231           | 84                          | –                                            |
| torch fp16 autocast                                   | 410 → 459           | 42                          | 0.9998 / 0.9999                              |
| ONNX Runtime CUDA provider, fp32                      | 201 → 232           | 84                          | 1.0000                                       |
| ONNX Runtime CUDA provider, fp16                      | 310 → 446           | 44                          | 0.9998 (128 images)                          |
| ONNX Runtime TensorRT provider (TensorRT 10), fp16    | 1,382 → 1,420       | 14                          | 0.9997 (128 images)                          |
| **TensorRT 11, fp16**                                 | **1,390 → 1,438**   | **14**                      | 0.9996 / 0.9999                              |
| TensorRT 11, int8 (ModelOpt PTQ, entropy calibration) | 2,801 → 2,893       | 7                           | 0.7334 / 0.9293                              |

- **TensorRT fp16 is 3.4× torch fp16, with the same embeddings.** On the subset its retrieval is
  identical to `sscd_fp16`: R@1 0.964, R@10 1.0, the same thresholds (±0.0004) and flagged pairs (±5),
  0 cross-group pairs at ≤ 95% recall.
- **ONNX Runtime's TensorRT provider is as fast as TensorRT itself**, so the Rust side can use the `ort`
  crate with the TensorRT provider (engine cache on disk) and needs no TensorRT bindings of its own.
  ONNX Runtime's own CUDA kernels are slower than torch.
- **int8 is not worth it.** Calibration variants (128–256 images; cosine vs fp32 min / p1, img/s):

  | int8 variant                              | Cosine min / p1 | img/s |
  |-------------------------------------------|-----------------|-------|
  | entropy calibration, everything quantized | 0.90 / 0.93     | 2,959 |
  | max calibration                           | 0.80 / 0.95     | 2,874 |
  | entropy, first conv + layer4 kept fp16    | 0.91 / 0.97     | 2,193 |
  | max, first conv + layer4 kept fp16        | 0.94 / 0.96     | 2,217 |

  Subset retrieval with the first variant is nearly unchanged (R@1 0.962 vs 0.964, same cross-group
  counts), but single images move as far as cosine 0.73, and at fp16 speed the GPU is no longer the
  bottleneck: the 70M pass is CPU-bound (decode ~550 img/s, thumbnails), so int8 would save nothing.
  (ONNX Runtime's TensorRT provider also rejects the int8 Q/DQ graph's input profile; not pursued.)
- **Consequence:** the GPU has ~2/3 of the ingest pass free. A second model in the first pass (PLAN §9
  decision 2, e.g. CLIP/SigLIP for keywords) costs little GPU time if it also runs on TensorRT.
- Setup notes: ONNX Runtime's TensorRT provider is built against TensorRT 10, while the `tensorrt`
  Python package is 11. TensorRT 11 only builds strongly typed networks: the precision comes from the
  ONNX graph, with no fp16/int8 builder flags. ONNX Runtime's TensorRT 10 has no strongly typed mode, so
  it gets the fp16 graph *and* `trt_fp16_enable` (without the flag it runs the fp16 graph in fp32, 336
  img/s). The torch upgrade (2.11 cu128 → 2.15 cu132, then cu134, which measures the same) reproduces the stored SSCD fp16 embeddings at
  cosine ≥ 0.99991; TensorRT fp16 rebuilt on CUDA 13 gives the same subset retrieval (R@1 0.963 vs 0.964,
  same cross-group counts). The verifier (kornia DISK + LightGlue) runs unchanged.
- **fp16 subnormal bug (found with the CUDA 13 stack):** GeM's eps (1e-6) is subnormal in fp16; TensorRT
  10.16 flushes it to zero, and all-zero channels then give 0/0 = NaN in every output. The rescaled GeM
  now clamps its per-channel max at fp16's smallest normal number (2^-14), which leaves the result
  unchanged.

### A second model: CLIP / SigLIP 2 image encoders (`eval/bench_semantic.py`, 2026-10-30)

For PLAN §9 decision 2 (keywords and semantic search in the first pass). Image towers from `open_clip`, exported to
ONNX in fp16 and built as TensorRT 11 engines like SSCD's. Speed is GPU-only (batch 64). "Cosine" compares
TensorRT with torch fp32 on 128 real images. Quality is the published zero-shot ImageNet-1k top-1: there are no
keyword labels here, so it ranks the models, not their fit to this library.

| Model (open_clip)        | Input  | Image tower | ImageNet zero-shot | torch fp16 img/s | TensorRT fp16 img/s | GPU hours for 70M | Cosine min |
|--------------------------|--------|-------------|--------------------|------------------|---------------------|-------------------|------------|
| CLIP ViT-B/32 (LAION-2B) | 224 px | 88 M        | 66.6%              | 1,506            | 2,171               | 9                 | 0.9996     |
| SigLIP 2 B/32            | 256 px | 95 M        | 74.0%              | 1,214            | 2,198               | 9                 | 0.9994     |
| **SigLIP 2 B/16**        | 224 px | 93 M        | **78.2%**          | 448              | **741**             | **26**            | 0.9999     |
| SigLIP 2 L/16            | 256 px | 316 M       | 82.5%              | 109              | 178                 | 109               | 0.9997     |
| SigLIP 2 So400m/14       | 224 px | 428 M       | 83.2%              | 78               | 122                 | 159               | 0.9994     |

ImageNet numbers: SigLIP 2 paper (arXiv 2502.14786, Table 1); CLIP B/32 from open_clip's results table.

- **TensorRT fp16 is safe for all of them** (cosine ≥ 0.9994), at 1.5–1.8× torch fp16.
- **Cost in the first pass:** SSCD on TensorRT is ~14 GPU hours for 70M, and the pass is CPU-bound (decoding ~1.5
  days). SigLIP 2 B/16 adds 26 GPU hours, so the GPU stays within the decode time (~40 h of ~36 h; about even).
  B/32 is nearly free, and L/16 makes the GPU the limit (~109 GPU hours). PLAN's "roughly double GPU time" holds for B/16,
  but the wall time barely changes because decoding dominates.
- **Suggested:** SigLIP 2 B/16 in the first pass (12 points above CLIP B/32, within the decode budget). Given
  "quality over speed", L/16 (+4.3 points, ~109 GPU hours) is the alternative. Both read the same decoded image, resized
  to their own input.

#### NaFlex: native aspect ratio (2026-10-30)

The fixed-resolution SigLIP 2 models **squash** every image to a square (open_clip's `resize_mode='squash'`, as in
training). This library is portrait: height/width median 1.5, 79% taller than 5:4, 2.4% wider than 4:3 (all 188k
images). NaFlex keeps the aspect ratio: the image is resized to a grid of 16 px patches, at most N of them, and the
sequence is padded and masked. A 2:3 picture becomes 13 × 19 patches (208 × 304 px) at N = 256. The learned 16 × 16
position grid is resized to each image's grid. One model serves every N, and the embeddings share one space.

- **Checkpoints:** B/16 and So400m/16 only (`google/siglip2-{base,so400m}-patch16-naflex`), Hugging Face
  `transformers` only (not open_clip). `transformers` 5.22 is now in the eval venv.
- **Quality** (paper Table 7, at an equal 256 tokens): B/16 NaFlex is slightly *below* fixed on natural images
  (ImageNet 78.5 vs 79.1, COCO text→image 51.1 vs 53.2) and above it on text-heavy images (HierText 7.4 vs 6.1,
  SciCap 19.7 vs 17.1, Screen2Words 14.8 vs 12.9). So400m NaFlex is on par (ImageNet 83.5 vs 83.4, COCO 55.1 vs
  55.4, HierText 9.2 vs 7.9). These benchmarks are mostly near-square photos, so they show little of the 1.5×
  squash this library would get.
- **TensorRT works** with the position grid resized outside the engine. A batch of 256 images had only 16–19
  distinct grids, so the resized grids can be cached per shape. The engine takes (patches, positions, mask) at a
  fixed length. It matches the HF model to 3e-6 in fp32. Measured on the
  RTX 2060 (batch 64):

| Model                | Tokens | torch fp16 img/s | TensorRT fp16 img/s | GPU hours for 70M | TensorRT vs fp32 cosine min |
|----------------------|--------|------------------|---------------------|-------------------|-----------------------------|
| B/16 fixed 224       | 196    | 448              | 741                 | 26                | 0.9999                      |
| B/16 fixed 256       | 256    | 280              | 411                 | 47                | –                           |
| **B/16 NaFlex**      | ≤ 256  | 278              | 383                 | 51                | 0.9995                      |
| So400m/14 fixed 224  | 256    | 65–78            | 122                 | 159               | 0.9994                      |
| **So400m/16 NaFlex** | ≤ 256  | 66               | 126                 | 154               | 0.997                       |

- **Conclusion:** at equal tokens NaFlex costs the same (So400m) or 7% more (B/16, the mask). Its gain is no
  distortion of portrait images and better text-in-image retrieval. On the published natural-image numbers it is
  equal or slightly worse. Which matters more for this library can only be measured here: a small set of text
  queries with hand-checked results (incl. text-in-image queries), comparing B/16 fixed 224, B/16 NaFlex 256 and
  So400m NaFlex 256.

#### Why So400m loses precision on TensorRT: fp16 accumulation (2026-10-31)

- **Not one layer.** An engine with every encoder layer's output as an extra output (32 images, valid tokens only)
  has ~5× torch fp16's relative error from layer 0 on, and 1.9× already after the patch-embedding matmul. The error
  grows the same way in every layer, so it isn't one bad layer or op. The extra outputs don't change it (worst
  final cosine 0.9931 vs 0.9924 untapped).
- **The kernels.** The engine inspector names them. Myelin compiles the QKV, out-projection and fc2 matmuls (and
  the patch embedding) to `turing_h1688gemm_*` and `sm75_xmma_gemm_f16f16_f16f16_f16_*`, which accumulate in
  **fp16**. Only fc1 + GELU gets `turing_fp16_s1688gemm_*_gelu` (fp32 accumulation). cuBLAS (torch, ONNX Runtime)
  accumulates fp16 matmuls in fp32. On GeForce Turing, fp16 accumulation runs at twice the rate, and TensorRT
  picks kernels by time.
- **One Linear alone, at So400m's shapes** (16,384 tokens; error vs float64):

| Shape         | cuBLAS fp16 | TensorRT fp16   | TensorRT, output cast to fp32 | TensorRT, inputs cast to fp32 |
|---------------|-------------|-----------------|-------------------------------|-------------------------------|
| QKV 1152→1152 | 2.1e-4      | 2.3e-3 (1.0 ms) | 2.3e-3 (same kernel)          | 2.1e-4 (7.9 ms, `sgemm`)      |
| fc1 1152→4304 | 2.1e-4      | 2.3e-3 (5.0 ms) | 2.3e-3                        | 2.1e-4 (28.9 ms)              |
| fc2 4304→1152 | 2.1e-4      | 4.1e-3 (4.3 ms) | 4.1e-3                        | 2.1e-4 (27.7 ms)              |

- **No switch.** TensorRT 11 has no accumulation setting: it is strongly typed, and its builder flags only cover
  TF32 and strict NaNs. Writing the matmul in fp32 gets the precision back, but only through plain fp32 `sgemm`
  (no tensor cores), ~6–7× slower per matmul.
- **The cost of fp32 accumulation**, each alone on the GPU (batch 64): TensorRT fp16 126 img/s (154 hours for 70M),
  **torch pure fp16 with SDPA 87 img/s (223 hours)**, ONNX Runtime CUDA fp16 61 img/s (319 hours). The earlier
  "1 img/s" for ONNX Runtime was a GPU-memory spill: the fp32 reference model was on the 6 GB GPU too.
- SSCD and DISK on TensorRT keep their measured quality (cosine ≥ 0.9994, same retrieval and decisions), so for
  those models the fp16 accumulation is harmless.
- **ONNX Runtime's transformer optimizer** (`optimize_model`, model types bert, vit and clip, then fp16 with fp32
  I/O; 512 random library images vs torch fp32, alone on the GPU, batch 64): every type fuses the 28 GELUs
  (`FastGelu`) and nothing else; **none fuses attention**, whose pattern in our NaFlex export (the padding mask) it
  doesn't recognize. The GELU fusion alone: **63 → 84 img/s** (231 hours for 70M, level with torch fp16), with
  the same fidelity (nearest neighbour as fp32 0.979 vs 0.977; top-5 overlap 0.986 vs 0.988; worst cosine
  0.9956). Fused attention would need the export to write attention in the optimizer's pattern, or ONNX Runtime's
  `MultiHeadAttention` op directly.

## Bug: SSCD breaks in fp16

- **Meta's SSCD TorchScript breaks in fp16:** `.half()` collapses every image to the same
  vector (backbone relative error ~1.0). Fixed by rebuilding SSCD as a plain module
  (`models.SSCD`) with the weights remapped from ClassyVision to torchvision names.
  Verified against the TorchScript: fp32 max |diff| 1.3e-7; fp16 autocast cosine ≥ 0.999985.
  GeM pooling runs in fp32 (x³ overflows fp16). This module is also the ONNX export source.
  - **Real cause (found 2026-10-30): not TorchScript.** SSCD's BatchNorm running variances reach
    3.5e7, beyond fp16's maximum (65,504). Any pure-fp16 cast turns them into inf and zeroes those
    channels; the rebuilt module breaks the same way under `.half()` (cosine 0.04 vs fp32). Autocast
    keeps BatchNorm parameters in fp32, which is why it worked. The export (`accel.SSCDExport`)
    folds BatchNorm into the convolutions in fp32 first and computes GeM rescaled by the per-channel
    max: a pure fp16 graph then matches fp32 at cosine ≥ 0.99995.

## Bug: R@k undercounted groups of 3+

- **R@k metric undercounted groups of 3+:** another member of the same group ranking ahead of
  the scored partner counted as a miss (R@1 read 0.935 for every model). Fixed so only
  images outside the group count as ranked ahead.
