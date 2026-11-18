# Batched GPU baseline JPEG encoder: plan

Status: plan, 2026-11-17. Companion to `GPU_BATCH_FEASIBILITY.md`, which chose nvJPEG for decode and left encode open.

## Goal

Encode a batch of RGB images, held in device memory, to baseline JPEG on the GPU, with one launch set per stage for
the whole batch. The output is valid JPEG that libjpeg-turbo decodes. Pixels and bytes need not match libjpeg-turbo's
encoder, but quality at a given setting must be comparable.

## Scope

In scope:
- Baseline sequential Huffman (SOF0), 8-bit, three components (YCbCr, JFIF).
- Chroma: 4:2:0 (thumbnails) and 4:4:4 (ImageReduce's setting).
- Quality 1–100 with libjpeg's scaling of the Annex K tables.
- Standard Annex K Huffman tables first.
- Any image size, including sizes that aren't multiples of 8 or 16.

Out of scope for this phase:
- Progressive output, arithmetic coding, restart markers.
- Optimized Huffman tables (phase 2, see below).
- Grayscale, CMYK, ICC and EXIF output (headers are written by the host, minimal APP0 JFIF).

## Interface

Input: one device buffer of interleaved RGB8, plus a descriptor array with, per image, the buffer offset, width,
height and stride. Output: per image, a JPEG byte stream copied to host memory. The batch may mix sizes and sampling.

C API (`encode_gpu.h`): create a context once with the chosen quality, chroma and device; encode a batch; read back
the outputs. The context owns its device buffers and grows them (grow-only), as the decoder wrappers do.

## Pipeline

Each stage processes the whole batch. Data stays on the device until the output copy.

1. **Pad and colour.** For each image, pad to whole MCUs by edge replication (libjpeg's behaviour: 16×16 for 4:2:0,
   8×8 for 4:4:4). Convert RGB to YCbCr (JFIF, full range, BT.601). Downsample chroma by 2×2 averaging for 4:2:0.
   Level-shift by −128.
2. **Forward DCT.** Separable 8×8 float DCT with a precomputed cosine matrix in shared memory. Output float
   coefficients per block.
3. **Quantize.** Divide by the quality table (libjpeg scaling of Annex K: scale = 5000/q for q < 50, else 200−2q;
   entries clamped to 1..255), round half away from zero, store int16 in zigzag order.
4. **Block symbol sizes.** One thread per block: DC difference against the previous block of the same component in
   MCU order (independent, since all quantized DCs are known), its category, and the bit count of its DC code plus AC
   symbols (run-length with ZRL and EOB). Store the bit count per block.
5. **Scan.** Exclusive prefix sum over the batch's blocks in MCU order (per image, blocks are in the order the
   stream requires). Each image's bit region starts at a byte boundary.
6. **Bit writing.** One thread per block writes its codes at its bit offset into a per-image 32-bit word buffer
   (MSB first). Words are cleared first, and writes use atomicOr, which is only contended at word boundaries.
7. **Padding.** Fill each image's final partial byte with 1-bits (JPEG's rule), then mark the entropy length.
8. **Byte stuffing.** Each 0xFF byte in the entropy data is followed by 0x00. Compute output positions by a prefix sum
   over per-byte 0xFF flags, and write the stuffed bytes.
9. **Assemble.** The host writes each image's header (SOI, APP0 JFIF, DQT in zigzag order, SOF0, DHT with the Annex K
   tables, SOS) into the device output buffer before stage 8. Stage 8 writes the stuffed entropy after it, and EOI
   follows. The host copies each image's bytes out.

Every kernel indexes through the descriptor arrays, so one launch covers all images. Block-to-image lookup is a
binary search over the batch's block prefix sums.

## Validation

1. **Entropy stage is exact.** A CPU reference (`cpu_reference.cpp`) encodes the same quantized coefficients that the
   GPU produced, and the GPU's entropy bytes must match it byte for byte. This isolates the Huffman and bit-packing
   code from the float DCT.
2. **Batch-invariant output.** An image encoded alone must give the same bytes as the same image inside a batch with
   other images. This catches offset and padding errors.
3. **Valid JPEG.** Every output decodes with libjpeg-turbo (turbojpeg) without warnings, and the decoded size matches.
4. **Quality against libjpeg-turbo.** Encode a sample of the Pinterest set (decoded with turbojpeg) at qualities 50,
   75 and 90, 4:2:0 and 4:4:4, with turbojpeg's encoder as the reference. Compare the decoded output with the source
   by SSIMULACRA2 and PSNR, and the file size. Expect quality within noise and sizes within a few percent. The
   thresholds are set after the first run, not before.
5. **Edge sizes.** 1×1, 7×9, 8×8, 15×17, 16×16, 1×1000, 1000×1, and sizes just above a multiple of 16.
6. **Throughput.** Images per second for batches of 64 and 128, and time per stage, with the GPU otherwise idle.

## Phase 2: optimized Huffman tables

ImageReduce's encoder uses optimized Huffman tables (`optimize_coding`). The GPU encoder needs a histogram pass over
the symbols, a host-side table build (small), and a second entropy pass. The bit-count and writing stages already
exist, so this is an addition, not a rewrite.

## Risks

- Float DCT differs from libjpeg's integer islow DCT. Expected to be within a level of quantized coefficients, and
  the quality test in validation 4 covers it.
- atomicOr bit writes assume zeroed words. The clear must happen before stage 6 in every batch.
- The DC difference must use the previous block of the same component in MCU order, not in image raster order.
- Padding with 1-bits and the 0xFF stuffing rule are easy to get wrong; validation 1 and 3 catch both.
- Batch memory grows with total pixels. Batch size needs a pixel budget, not only a count.

## Open questions

- Where the code lives: `eval/jpeg-encoder/` as a standalone harness first, then moved into the app once validated.
- Whether thumbnails use 4:2:0 only (the app's current thumbnail encoder is 4:2:0; see `crates/codec/src/encode.rs`).

## Status (2026-11-17)

Implemented as a standalone harness in `eval/jpeg-encoder/` (`encode_gpu.h`, `encode_gpu.cu`, `test_encoder.cpp`,
`build.ps1`, `README.md`). Built natively with nvcc 13.4 and MSVC on an RTX 2060 (sm_75). Results are in the README.

- Entropy exactness vs libjpeg-turbo (jpeg_write_coefficients): 42 of 42 scan streams byte-identical (4:2:0 and 4:4:4, q50/75/100).
- Coefficients vs libjpeg's forward DCT: 100% within ±1; 99.9% exact.
- Valid JPEG (0 warnings), batch invariance (bytes identical across alone / batch / shuffled / tiny budget), edge sizes: all pass.
- Quality vs turbo (300 samples): size ratio within 0.97–1.03 at all six settings. PSNR within 0.5 dB at q50 and q75 (both chroma modes). At q90 the GPU's PSNR is 0.65 dB (4:2:0) and 0.88 dB (4:4:4) above turbo's. Those two settings fail the pre-registered PSNR criterion. Cause not established.
- Throughput (4:2:0 q75, device-resident input): about 2160 images/s at batch 64, about 2120 at batch 128.

Deviation from the plan's padding rule: blocks wholly outside the image grid are coded as libjpeg does for coefficient transcoding (zero AC, DC equal to the previous real block's DC). Without this the entropy stream does not match libjpeg.

Not done: Phase 2 (optimized tables), the 2^32-bit offset path, and integration into the app. Nothing committed.
