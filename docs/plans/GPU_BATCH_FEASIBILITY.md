# GPU batch decode: feasibility of the four decoder options

Status: evaluation, 2026-11-17. Nothing is implemented. Reads with the batched encoder plan
(`GPU_BATCHED_ENCODER_PLAN.md`) and the decoder notes in `JPEG_DECODERS.md`.

## Decisions this rests on

- The `pixel_hash` change is accepted, with a full re-hash of the library. Decoded pixels may differ from
  libjpeg-turbo's, so nothing needs to be bit-exact against it.
- The GPU is used for import only. The viewer decodes one image at a time with parajpeg's CPU threaded decode
  (`parajpeg-t12`).
- DCT scaling is removed from the decode path, so GPU decoders' lack of scaling is not a cost.
- The licence can change if needed.
- Effort is not a constraint. The bottleneck is correctness and throughput at millions of files.

"The four options" is read as the four decoders in the batching table: nvJPEG, jpeggpu, GPUJPEG, parajpeg CUDA.

## Summary

| Option | Batch API | Feasibility | Progressive | Output | Licence | Verdict |
|---|---|---|---|---|---|---|
| nvJPEG (`nvjpegDecodeBatched`, `GPU_HYBRID`) | Yes | High | CPU Huffman | RGB, device or host | NVIDIA EULA, redistributable component | **Primary for baseline files** |
| jpeggpu (our fork) | No; we would add one | High with effort | Rejected | YCbCr planes | GPL-3.0 (we can change it) | **Fallback and fully owned path** |
| GPUJPEG | No; N decoders on N streams | Medium-low | Rejected | RGB 4:4:4 | BSD-2-Clause | Not for this dataset |
| parajpeg CUDA | No; a rewrite | Low | Yes, CPU entropy | RGB | MIT | Not worth it |

## 1. nvJPEG, batched

**Mechanism.** `nvjpegDecodeBatched` takes a batch of images. Its GPU Huffman decoding is used only for batches
over 50 baseline interleaved JPEGs (`nvjpeg.h:167–168`). That threshold is a closed-library rule. No API or flag
changes it (see the nvJPEG investigation). At millions of files, batches of 64–128 are easy to form, so the
threshold costs little on the import path.

**Measured** (`JPEG_DECODERS.md`, RTX 2060): GPU_HYBRID at batch 64 costs 0.02–0.08 ms of CPU per Pinterest file
and 1.1–2.9 ms wall. At batch 128, 0.85 ms wall for a 1 MP file. The 24 MP file doesn't fit a batch over 50 in the
6 GB card's memory.

**Pixels.** nvJPEG's default chroma upsampling differs from libjpeg-turbo's (about 29 dB on 4:2:0). Its
`NVJPEG_FLAGS_UPSAMPLING_WITH_INTERPOLATION` flag brings it to about 64 dB on the 4:2:0 test file. Under decision 1
that difference is acceptable, but the flag is the better choice for thumbnails.

**Limits.**

- Progressive files decode Huffman on the CPU. Lossless conversion to baseline, which we already have, avoids it.
- Batch memory bounds the largest images per batch. Camera originals need smaller batches or CPU fallback.
- The decoded output can stay in device memory (the notes say the harness copies it to pinned host). Device output
  for the batched path is not yet measured.

**Licence.** The CUDA EULA lists nvJPEG among its redistributable components (`EULA.txt:786–795`). The exact
redistribution clause needs checking before we ship the DLL.

**Effort.** Days. An FFI binding plus a batch queue in the import. The binding exists in `eval/jpeg-decoders/nvjpeg.py`
as ctypes and would be re-expressed in Rust or C++.

**Verdict.** Primary for baseline files. It is the fastest measured path, it is already validated, and its licence
is clean.

## 2. jpeggpu, with a batch API we add

**Mechanism.** The library decodes one image per handle, and each launch shape is per image. The reports sort the
options into two levels:

- A wrapper of N handles on N streams, one sync. About 1–2 days, low risk. It runs concurrently but doesn't reduce
  launches.
- Fused batch kernels: one launch set for N images, with per-image offset tables, per-image grid sizes, and an image
  index per block. Estimated 2–4 weeks of agent-driven work, plus GPU validation. Risk points are the sync kernels,
  DC prediction across segments, tmp-buffer sizing, and a 16 MiB scan-data limit per scan (`decode_sync.cu:722–727`).

**Measured** (harness, Pinterest sample): 1.1–2.1 ms per image of latency, 0.3–1.6 ms of CPU per image. The
harness numbers are planes-only, so they are not comparable with RGB decoders until the colour stage is added.

**Pixels.** Planes differ from libjpeg-turbo's by up to one level. With our colour stage, RGB differs by mean 0.03
and max 3. Accepted under decision 1.

**Colour.** Output is YCbCr planes, so we need the colour stage. `harness/colour.cu` already does libjpeg-turbo's
fancy upsampling and YCbCr→RGB. It supports only 4:2:0, 4:2:2 and 4:4:4 and fails silently on other sampling. It
needs extending for batches and for validation of sampling factors.

**Limits.**

- Progressive files are rejected. They need the lossless conversion path or a CPU fallback.
- Each scan's entropy data is capped at 16 MiB.
- Experimental upstream, validated only by MSE against nvJPEG.

**Licence.** GPL-3.0. Under decision 3 we can change the app's licence. The other options don't need that.

**Verdict.** The fallback and the fully owned path. It keeps the whole decode in our code, which matters if nvJPEG's
limits or licence bite. It also works without the 50-image rule.

## 3. GPUJPEG

**Mechanism.** No batch API. One decoder per call, with one stream per decoder (`gpujpeg_decoder.c:149`). Batching
means N decoders on N streams, one per thread. The wrapper currently uses the default stream.

**Limits.**

- GPU Huffman needs restart markers, and the decoder only uses the GPU path with 32 or more segments
  (`gpujpeg_decoder.c:268`). Unmarked files always use CPU Huffman. Adding markers is a file rewrite (lossless
  `jpegtran -restart`), which is a bulk change to the library.
- Progressive is rejected (`gpujpeg_reader.c:1437–1438`).
- `HUFFMAN_GPU_CONST_TABLES` copies to `__constant__` memory on every decode, which races across streams.
- Chroma upsampling is weak on subsampled files (29.5–50 dB in our measurements). That is poor for thumbnails.

**Licence.** BSD-2-Clause.

**Verdict.** Not for this dataset. Markers would have to be added to the whole library, and the chroma quality is
lower than the alternatives'.

## 4. parajpeg CUDA

**Mechanism.** No batch API. Each call creates its own stream (`gpu_decoder.cu:544`), allocates new buffers
(`:567`), and synchronizes several times (`:607, 668, 691, 709, 725`). Batching needs stream and buffer reuse plus a
multi-image path.

**Measured.** About 5–8 ms per Pinterest file, which is slower than the CPU decoder. Our docs (`docs/gpu.md`) say the
CUDA backend has never run on hardware, which conflicts with the timings in `JPEG_DECODERS.md`. That conflict needs
resolving before any decision rests on it.

**Licence.** MIT.

**Verdict.** Not worth it. Its CUDA path is slower than its CPU path for our file sizes, and it would need the most
rework. parajpeg's CPU threaded decode stays the viewer's decoder under decision 2, but with one decode at a time:
its thread pool has a single job slot and isn't safe for concurrent callers.

## Cross-cutting parts of the pipeline

- **Resize (MKS2021).** A custom CUDA kernel is feasible. Under decision 1 it need not be bit-exact with
  fast_image_resize. The upstream u8 path uses integer weights, 32-bit accumulation, and rounding. The app pins a
  fork (`darktorres/fast_image_resize` at `0ea7089`), whose arithmetic I have not checked.
- **PDQ and ThumbHash.** Both can run as batched kernels. PDQ's float ordering needs care to be reproducible, but
  exact reproduction is not required. ThumbHash works on 100 px images, so its cost is small per image.
- **Encode.** nvJPEG's encoder is single-image, and no GPU encoder in our set is batched. We will write one (see
  `GPU_BATCHED_ENCODER_PLAN.md`).

## Recommended pipeline

Per batch, keep all data on the device between stages:

1. Decode with nvJPEG, batched, RGB to device memory. Progressive files take the lossless baseline conversion first,
   or the CPU path.
2. Resize with the batched MKS2021 kernel.
3. PDQ and ThumbHash kernels on the resized images.
4. Encode thumbnails with our batched baseline encoder.
5. Copy only the outputs back: hashes, ThumbHash bytes and thumbnail JPEGs.

Why nvJPEG first: it is the fastest measured path, its licence is clean, and it already has a validated batch
mechanism. Why keep jpeggpu in reserve: if nvJPEG's batch or licence limits bite, a fused jpeggpu batch path is
the fallback, and the colour stage is the only extra piece it needs.

## Open verifications

- nvJPEG batched decode into device memory: not yet measured.
- The fork's MKS2021 arithmetic against upstream.
- The exact nvJPEG redistribution clause in the EULA.
- The parajpeg CUDA "never run on hardware" conflict.
- Whether the viewer's parajpeg calls are serialized in the app (the serializing shim lives in the fork, commit
  `8e2c53a`, and isn't in this checkout).
