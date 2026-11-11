# Models and accelerators

Measured results moved here from the former `FINDINGS.md` log. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [TEST_SET.md](TEST_SET.md); the old titles are listed in [FINDINGS.md](FINDINGS.md).

## Model speed (RTX 2060, GPU only, synthetic input, batch 64)

| Model | img/s | GPU days for 70M |
|---|---|---|
| SSCD fp32 320px | 241 | 3.4 |
| **SSCD fp16 320px** | **410** | **2.0** |
| SSCD fp16 288px | 499 | 1.6 |
| SSCD fp16 256px | 653 | 1.2 |
| SSCD large (ResNeXt101) fp32 | 105 | 7.7 |
| DINOv2-S fp16 224px | 1020 | 0.8 |
| DINOv2-B fp16 224px | 356 | 2.3 |
| CLIP B/32 fp16 224px | 1498 | 0.5 |

### TensorRT and ONNX Runtime for SSCD (`eval/accel.py`, 2026-10-30)

Same weights exported to ONNX (`accel.SSCDExport`: BatchNorm folded, fp16-safe GeM; see "Bugs found"),
RTX 2060, batch 64, 320 px, GPU only. Measured first on the CUDA 12 stack (driver 576.57), then again after
the driver update on CUDA 13 (driver 621.01: torch 2.15 cu132, onnxruntime-gpu 1.31.1, TensorRT 11.4 / 10.16
cu13):

| Runtime | img/s, CUDA 12 → 13 | GPU days for 70M | Cosine vs torch fp32 (9.3k subset): min / p1 |
|---|---|---|---|
| torch fp32 | 241 → 231 | 3.5 | – |
| torch fp16 autocast | 410 → 459 | 1.8 | 0.9998 / 0.9999 |
| ONNX Runtime CUDA provider, fp32 | 201 → 232 | 3.5 | 1.0000 |
| ONNX Runtime CUDA provider, fp16 | 310 → 446 | 1.8 | 0.9998 (128 images) |
| ONNX Runtime TensorRT provider (TensorRT 10), fp16 | 1,382 → 1,420 | 0.6 | 0.9997 (128 images) |
| **TensorRT 11, fp16** | **1,390 → 1,438** | **0.6** | 0.9996 / 0.9999 |
| TensorRT 11, int8 (ModelOpt PTQ, entropy calibration) | 2,801 → 2,893 | 0.3 | 0.7334 / 0.9293 |

- **TensorRT fp16 is 3.4× torch fp16, with the same embeddings.** On the subset its retrieval is
  identical to `sscd_fp16`: R@1 0.964, R@10 1.0, the same thresholds (±0.0004) and flagged pairs (±5),
  0 cross-group pairs at ≤ 95% recall.
- **ONNX Runtime's TensorRT provider is as fast as TensorRT itself**, so the Rust side can use the `ort`
  crate with the TensorRT provider (engine cache on disk) and needs no TensorRT bindings of its own.
  ONNX Runtime's own CUDA kernels are slower than torch.
- **int8 is not worth it.** Calibration variants (128–256 images; cosine vs fp32 min / p1, img/s):

  | int8 variant | Cosine min / p1 | img/s |
  |---|---|---|
  | entropy calibration, everything quantized | 0.90 / 0.93 | 2,959 |
  | max calibration | 0.80 / 0.95 | 2,874 |
  | entropy, first conv + layer4 kept fp16 | 0.91 / 0.97 | 2,193 |
  | max, first conv + layer4 kept fp16 | 0.94 / 0.96 | 2,217 |

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

| Model (open_clip) | Input | Image tower | ImageNet zero-shot | torch fp16 img/s | TensorRT fp16 img/s | GPU hours for 70M | Cosine min |
|---|---|---|---|---|---|---|---|
| CLIP ViT-B/32 (LAION-2B) | 224 px | 88 M | 66.6% | 1,506 | 2,171 | 9 | 0.9996 |
| SigLIP 2 B/32 | 256 px | 95 M | 74.0% | 1,214 | 2,198 | 9 | 0.9994 |
| **SigLIP 2 B/16** | 224 px | 93 M | **78.2%** | 448 | **741** | **26** | 0.9999 |
| SigLIP 2 L/16 | 256 px | 316 M | 82.5% | 109 | 178 | 109 | 0.9997 |
| SigLIP 2 So400m/14 | 224 px | 428 M | 83.2% | 78 | 122 | 159 | 0.9994 |

ImageNet numbers: SigLIP 2 paper (arXiv 2502.14786, Table 1); CLIP B/32 from open_clip's results table.

- **TensorRT fp16 is safe for all of them** (cosine ≥ 0.9994), at 1.5–1.8× torch fp16.
- **Cost in the first pass:** SSCD on TensorRT is ~14 GPU hours for 70M, and the pass is CPU-bound (decoding ~1.5
  days). SigLIP 2 B/16 adds 26 GPU hours, so the GPU stays within the decode time (~40 h of ~36 h; about even).
  B/32 is nearly free, and L/16 makes the GPU the limit (~5 days). PLAN's "roughly double GPU time" holds for B/16,
  but the wall time barely changes because decoding dominates.
- **Suggested:** SigLIP 2 B/16 in the first pass (12 points above CLIP B/32, within the decode budget). Given
  "quality over speed", L/16 (+4.3 points, ~5 days) is the alternative. Both read the same decoded image, resized
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

| Model | Tokens | torch fp16 img/s | TensorRT fp16 img/s | GPU hours for 70M | TensorRT vs fp32 cosine min |
|---|---|---|---|---|---|
| B/16 fixed 224 | 196 | 448 | 741 | 26 | 0.9999 |
| B/16 fixed 256 | 256 | 280 | 411 | 47 | – |
| **B/16 NaFlex** | ≤ 256 | 278 | 383 | 51 | 0.9995 |
| So400m/14 fixed 224 | 256 | 65–78 | 122 | 159 | 0.9994 |
| **So400m/16 NaFlex** | ≤ 256 | 66 | 126 | 154 | 0.997 |

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

| Shape | cuBLAS fp16 | TensorRT fp16 | TensorRT, output cast to fp32 | TensorRT, inputs cast to fp32 |
|---|---|---|---|---|
| QKV 1152→1152 | 2.1e-4 | 2.3e-3 (1.0 ms) | 2.3e-3 (same kernel) | 2.1e-4 (7.9 ms, `sgemm`) |
| fc1 1152→4304 | 2.1e-4 | 2.3e-3 (5.0 ms) | 2.3e-3 | 2.1e-4 (28.9 ms) |
| fc2 4304→1152 | 2.1e-4 | 4.1e-3 (4.3 ms) | 4.1e-3 | 2.1e-4 (27.7 ms) |

- **No switch.** TensorRT 11 has no accumulation setting: it is strongly typed, and its builder flags only cover
  TF32 and strict NaNs. Writing the matmul in fp32 gets the precision back, but only through plain fp32 `sgemm`
  (no tensor cores), ~6–7× slower per matmul.
- **The cost of fp32 accumulation**, each alone on the GPU (batch 64): TensorRT fp16 126 img/s (6.4 days for 70M),
  **torch pure fp16 with SDPA 87 img/s (9.3 days)**, ONNX Runtime CUDA fp16 61 img/s (13 days). The earlier
  "1 img/s" for ONNX Runtime was a GPU-memory spill: the fp32 reference model was on the 6 GB GPU too.
- SSCD and DISK on TensorRT keep their measured quality (cosine ≥ 0.9994, same retrieval and decisions), so for
  those models the fp16 accumulation is harmless.
- **ONNX Runtime's transformer optimizer** (`optimize_model`, model types bert, vit and clip, then fp16 with fp32
  I/O; 512 random library images vs torch fp32, alone on the GPU, batch 64): every type fuses the 28 GELUs
  (`FastGelu`) and nothing else; **none fuses attention**, whose pattern in our NaFlex export (the padding mask) it
  doesn't recognize. The GELU fusion alone: **63 → 84 img/s** (9.6 days for 70M, level with torch fp16), with
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
