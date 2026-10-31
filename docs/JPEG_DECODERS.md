# JPEG decoders

Which JPEG decoder the app should use: libjpeg-turbo, as `pg-core` calls it, against five CPU decoders, nvJPEG's
backends and NVDEC on seven library files, plus restart markers, lossless baseline conversion, and the decoders
researched but not measured (`eval/jpeg-decoders`, 2026-10-31). Instruction counts use the callgrind setup in
[PROFILING.md](PROFILING.md).

## Verdict: keep libjpeg-turbo

- **Fewest instructions:** with its default or fast options, it uses the fewest instructions of any CPU decoder on
  every input except two, both FFmpeg's: the progressive file, and the phone photo at 1/4, by 1.5%.
- **Fastest scaled decode:** its DCT-scaled decode, which `analyze` uses, is the fastest decode on every file
  except one progressive file, where FFmpeg is faster.
- **The GPU frees the CPU only with GPU Huffman decoding:** nvJPEG as torchvision runs it (CPU Huffman) keeps one
  CPU core busy for as long as it decodes. Its `GPU_HYBRID` backend, in batches over 50, costs the CPU 0.02–0.6 ms
  per image and decodes 2–4× faster ("nvJPEG with Huffman decoding on the GPU" below).

Changing to a decoder whose pixels differ (every one measured here except libvips) would also change every
stored `pixel_hash`, which would mean a re-import.

## All decoders at a glance

Speeds are against libjpeg-turbo on the seven inputs: wall time on one thread, the
full-size table below, unless noted. "Claimed" figures come from the project's own benchmarks on other hardware.

| Decoder | Runs on | Measured here | DCT scaling | Pixels against the app's decode | Speed | Verdict |
|---|---|---|---|---|---|---|
| **libjpeg-turbo 3.1** (vendored) | CPU | yes | 1/2–1/8 | the reference | fastest scaled decode; fewest instructions | **keep** |
| libjpeg-turbo, fast options | CPU | yes | yes, no fast variant | 29 dB on 4:2:0 | 3–11% fewer instructions full size, none scaled | no |
| Pillow 13 (libjpeg-turbo) | CPU | yes (Windows) | `draft()`, rounds down | same library | within 10% of the WSL times | reference only |
| zune-jpeg 0.5.15 | CPU (Rust) | yes | **no** | within 4 levels (50–73 dB) | 3–35% slower full size | no |
| jpeg-decoder 0.3.2 | CPU (Rust), worker threads | yes | 1/2–1/8 | within 4 levels (50–73 dB) | 1.3–3.0× slower, despite extra threads | no |
| FFmpeg 8 `mjpeg` | CPU | yes | `lowres` 1/2–1/8 | 29 dB on 4:2:0 | 1.3× faster on progressive; 1.2–3.2× slower otherwise | no |
| libvips 8.18 | CPU (libjpeg-turbo) | yes | shrink-on-load, rounds down | identical at full size | 1.2–2.0× slower; its threads don't speed up a decode | no |
| nvJPEG `HYBRID` (torchvision's) | GPU + CPU Huffman | yes | no | 69 dB on 4:4:4, 29.5–59 dB subsampled; 60–64 dB with `--interpolate` | batch 64: 1.8–33 ms; CPU time ≈ wall time | no |
| nvJPEG `GPU_HYBRID`, batches over 50 | GPU | yes | no | same as `HYBRID` | 1.1–2.9 ms Pinterest, 13–15 ms camera, CPU 0.02–0.6 ms; not progressive or restart-marker files; 24 MP batches don't fit 6 GB | candidate for the thumbnail job |
| nvJPEG `GPU_HYBRID`, one image at a time | GPU, partly CPU | yes | no | same as `HYBRID` | camera files: CPU 12–32 ms, against 28–63 ms with CPU Huffman; no gain on small files | camera files only |
| nvJPEG `HARDWARE` | GPU JPEG engine | no: A100/A30, Hopper, Blackwell | 1/2–1/8 | — | — | not on the RTX 2060 |
| NVDEC (`mjpeg_cuvid`) | GPU video engine | yes (2 files) | no | — | 4:2:0 only; no faster than FFmpeg's CPU decoder, same CPU time | no |
| Intel Quick Sync (`mjpeg_qsv`) | iGPU | no: UHD 630 disabled in the BIOS | — | — | — | only if the iGPU is enabled |
| parajpeg 0.1.0 | CPU threads (AVX2/NEON); CUDA unrun | no | **no** | byte-identical (claimed) | claimed, Apple M3, 8 threads: 2.8–3.1× without markers, 4.4–5.4× with; 0.94× with one image per core | test, for full-size viewer decodes |
| jpeggpu | GPU (CUDA), GPU Huffman without markers | no | no | MSE ≈ 0.2 against nvJPEG (its README's example) | claimed, RTX 2070: 6 MP in 1.26 ms, against nvJPEG's 6.89 | test next |
| GPUJPEG (CESNET) | GPU (CUDA) | no | no | — | GPU Huffman only with restart markers; CPU Huffman without | only with restart markers |
| Fastvideo SDK | GPU (CUDA), commercial | no | — | — | claimed 19–75× faster with restart markers; CPU Huffman without | only with restart markers |
| nvImageCodec | wrapper: nvJPEG + libjpeg-turbo | no | as its backends | as its backends | as nvJPEG | the integration layer for GPU decoding |
| Wuffs `std/jpeg` | CPU, memory-safe | no | — | — | no published numbers | maybe; easy to add to the harness |
| epeg | wrapper: libjpeg DCT scaling | no | yes | — | the technique we already use | the YUV-output idea, for PDQ |
| jpegli, jpegn (Go), course projects | — | no | — | — | jpegli's gains are in encoding; jpegn is Go-only; Fulim13/jpeg uses a custom format; debesheedas needs 4:4:4 input | no |

Details for the measured rows are in the tables below, and for the rest in "Other decoders, researched but not
measured".

## Inputs

The main kinds in the library.

- **Pinterest set:** a census of 1,000 files found 81% baseline 4:4:4, 17% progressive 4:2:0 and 1% baseline
  4:2:0 (p50 0.96 MP, p90 2.8 MP).
- **Camera originals:** from E:\Cameras.

| Input | Kind | Size |
|---|---|---|
| `pin-base444-1mp` | baseline 4:4:4 | 844×1267, 92 KB |
| `pin-base444-3mp` | baseline 4:4:4 | 1500×1910, 182 KB |
| `pin-prog420-1mp` | progressive 4:2:0 | 736×1104, 64 KB |
| `pin-base420-1mp` (the bench sample) | baseline 4:2:0 | 1280×1017, 126 KB |
| `phone-base420-4mp` (Motorola, 2016) | baseline 4:2:0 | 2592×1456, 3.1 MB |
| `sony-base422-7mp` (Sony compact) | baseline 4:2:2 | 3648×2056, 3.6 MB |
| `a6000-base422-24mp` (Sony A6000) | baseline 4:2:2 | 6000×4000, 5.6 MB |

## Decoders measured

All decode to packed RGB8 on one thread unless noted. "Scaled" is the DCT scale `analyze` asks
for (long side ≥ 512, short side ≥ 320): 1/2 for the Pinterest files, 1/4 for the phone and the 7 MP Sony, 1/8
for the A6000.

- **libjpeg-turbo:** the vendored 3.x, called as `pg-core` does: accurate IDCT, smooth upsampling.
- **libjpeg-turbo fast:** the same, with `TJPARAM_FASTDCT` and `TJPARAM_FASTUPSAMPLE`.
- **zune-jpeg 0.5.15:** the `image` crate's decoder since 0.25. It has no DCT scaling.
- **jpeg-decoder 0.3.2:** the `image` crate's decoder before 0.25. Even without rayon it decodes in worker
  threads, so its wall times got help from extra threads.
- **FFmpeg 8 `mjpeg`:** `lowres` for scaling, then swscale to RGB, as `video.rs` converts.
- **libvips 8.18.0:** `jpegload_buffer` with shrink-on-load, sequential, `concurrency` 1, operation cache off,
  plus a copy out of its buffer. It decodes with Ubuntu's libjpeg-turbo 2.1.5, in a worker thread.
- **nvJPEG:** through torchvision 0.29 (CUDA 13.4) on Windows (`gpu.py`), with Pillow 13 and pyvips 8.18.7 as
  CPU references in the same process.

## Instructions per decode

Millions of instructions; callgrind, Linux build. Most come from gungraun. libvips and jpeg-decoder do
their work in threads gungraun doesn't count, so their figures come from whole runs instead (`wsl.sh marginal`:
3 decodes minus 1, halved, all threads). That method gives the same numbers as gungraun for libjpeg-turbo, to
within 0.5 M.

| Full size | turbo | turbo fast | zune | jpeg-decoder | FFmpeg | libvips |
|---|---:|---:|---:|---:|---:|---:|
| pin-base444-1mp | **31.0** | 31.2 | 32.9 | 54.4 | 86.4 | 45.0 |
| pin-base444-3mp | **77.5** | 77.6 | 80.3 | 140.4 | 224.0 | 105.6 |
| pin-prog420-1mp | 42.3 | 40.2 | 56.2 | 80.9 | **28.6** | 54.7 |
| pin-base420-1mp | 28.5 | **25.4** | 35.6 | 58.1 | 124.4 | 43.7 |
| phone-base420-4mp | 246.8 | **238.4** | 333.9 | 361.8 | 253.9 | 294.9 |
| sony-base422-7mp | 355.1 | **341.4** | 437.6 | 637.2 | 401.2 | 431.2 |
| a6000-base422-24mp | 835.0 | **789.5** | 1,017.6 | 1,804.5 | 997.0 | 1,038.3 |

| Scaled | turbo | turbo fast | jpeg-decoder | FFmpeg | libvips |
|---|---:|---:|---:|---:|---:|
| pin-base444-1mp (1/2) | **24.6** | **24.6** | 51.5 | 65.0 | 32.1 |
| pin-base444-3mp (1/2) | **60.6** | **60.6** | 129.1 | 166.8 | 70.7 |
| pin-prog420-1mp (1/2) | 37.6 | 37.6 | 70.3 | **31.6** | 45.2 |
| pin-base420-1mp (1/2) | **21.2** | 21.3 | 41.9 | 63.1 | 29.1 |
| phone-base420-4mp (1/4) | 216.1 | 216.1 | 277.6 | **212.9** | 235.5 |
| sony-base422-7mp (1/4) | 285.3 | **284.1** | 357.6 | 289.1 | 307.1 |
| a6000-base422-24mp (1/8) | 378.4 | **377.9** | 761.5 | 582.8 | 396.1 |

## Wall time

Median ms; WSL, i7-8700, machine at 6–18% load. A run during a busy spell (100%: a VMware VM
plus the app) was 2–3× slower everywhere and was discarded; a repeat when quiet matched the first quiet run.

| Full size | turbo | turbo fast | zune | jpeg-decoder | FFmpeg | libvips |
|---|---:|---:|---:|---:|---:|---:|
| pin-base444-1mp | 3.36 | **3.07** | 3.52 | 10.05 | 10.26 | 6.58 |
| pin-base444-3mp | 8.37 | **7.58** | 8.61 | 18.89 | 26.91 | 13.25 |
| pin-prog420-1mp | 4.17 | 3.97 | 5.64 | 8.80 | **3.11** | 6.76 |
| pin-base420-1mp | 3.24 | **2.88** | 4.13 | 7.77 | 9.68 | 6.12 |
| phone-base420-4mp | 28.11 | **27.04** | 37.70 | 40.29 | 42.21 | 32.50 |
| sony-base422-7mp | 39.72 | **37.58** | 49.46 | 58.54 | 48.18 | 46.91 |
| a6000-base422-24mp | 124.25 | **117.89** | 140.43 | 165.31 | 173.21 | 176.41 |

| Scaled | turbo | turbo fast | jpeg-decoder | FFmpeg | libvips |
|---|---:|---:|---:|---:|---:|
| pin-base444-1mp (1/2) | **2.36** | 2.37 | 9.48 | 5.41 | 4.15 |
| pin-base444-3mp (1/2) | **5.64** | 5.68 | 16.49 | 13.56 | 8.18 |
| pin-prog420-1mp (1/2) | 3.53 | 3.52 | 7.41 | **3.13** | 5.07 |
| pin-base420-1mp (1/2) | 2.30 | **2.25** | 5.73 | 5.49 | 3.86 |
| phone-base420-4mp (1/4) | **23.95** | 24.08 | 35.01 | 28.56 | 24.94 |
| sony-base422-7mp (1/4) | 30.27 | **30.25** | 44.93 | 37.11 | 31.52 |
| a6000-base422-24mp (1/8) | **49.43** | 49.45 | 94.54 | 77.20 | 52.20 |

## Output against the app's decode

From `wsl.sh compare`: mean |difference| per channel value, the largest, and PSNR.

- **libvips:** identical at full size (libjpeg-turbo 2.1.5 decodes the same as 3.x). Its shrink-on-load rounds sizes
  down: 422×633 against 422×634.
- **zune-jpeg and jpeg-decoder:** within 4 levels at full size (50–73 dB).
- **turbo fast, FFmpeg and nvJPEG:** drift where there is subsampled chroma, because none of them upsample it
  smoothly. On the 4:2:0 phone photo all three are about 29 dB from the app's decode (mean 6.3–6.5, max 56–57). On
  4:4:4, nvJPEG matches (69 dB).
- **Scaled decodes:** they differ more between decoders (each does its reduced IDCT its own way). At 1/4 on the
  phone file, jpeg-decoder and FFmpeg are 28 dB from libjpeg-turbo.

## Where the differences come from

From `cgtab.py` on the gungraun outputs.

- **FFmpeg's decoder costs about what libjpeg-turbo does.** On `pin-base444-1mp`, libavcodec is 31.8 M against
  libjpeg-turbo's 31.0 M in total. The rest, 53 M (62%), is swscale's YUV 4:4:4 → RGB conversion.
- **FFmpeg wins on progressive files** because libjpeg-turbo spends half of a progressive decode in
  `decode_mcu_AC_refine`, 20.7 M of 42.3 M.
- **zune-jpeg loses on high-bitrate files in its Huffman decoder.** `decode_mcu_block` is 75% of the phone photo.
- **The fast libjpeg-turbo options save 3–11% of a full decode on subsampled files and nothing at the scales
  `analyze` uses.** Scaled IDCTs have no fast variant, and 4:4:4 has nothing to upsample. They cost 29 dB on 4:2:0.

## nvJPEG through torchvision (Huffman decoding on the CPU)

Windows, RTX 2060, machine at 10–17% load. The table gives ms per image, with the decoded pixels
left on the GPU and, in brackets, copied back to host memory. Batches are N copies of the file in one
`decode_jpeg` call. The 24 MP file stops at batch 32: 72 MB per decoded image, against the GPU's 6 GB.

| Input | Pillow full / scaled | nvJPEG batch 1 | batch 8 | batch 32 | batch 64 |
|---|---:|---:|---:|---:|---:|
| pin-base444-1mp | 4.44 / 4.46¹ | 2.65 (3.02) | 1.87 (2.33) | 1.79 (2.30) | 1.78 (2.25) |
| pin-base444-3mp | 10.33 / 6.46 | 6.14 (6.98) | 4.27 (5.45) | 4.12 (5.26) | 4.04 (5.28) |
| pin-prog420-1mp | 6.04 / 4.67 | 3.53 (3.46) | 2.31 (2.86) | 2.25 (2.72) | 2.25 (2.72) |
| pin-base420-1mp | 4.39 / 4.43¹ | 3.49 (3.64) | 1.97 (2.53) | 1.92 (2.47) | 1.90 (2.44) |
| phone-base420-4mp | 28.58 / 21.83 | 33.62 (38.98) | 27.11 (31.29) | 26.60 (28.45) | 26.23 (27.99) |
| sony-base422-7mp | 44.07 / 28.00 | 48.43 (41.65) | 30.24 (34.30) | 29.46 (32.16) | 29.18 (32.60) |
| a6000-base422-24mp | 111.14 / 47.24 | 85.53 (100.62) | 62.34 (74.46) | 61.93 (72.98) | — |

¹ Pillow's `draft()` rounds sizes down and so turns down 1/2 for odd heights: these two rows are full decodes.

- **The CPU is still the limit.** The process's CPU time per image equals the wall time at every batch size.
  Turing has no hardware JPEG decoder, so nvJPEG's hybrid backend does the Huffman decoding on one CPU thread and
  leaves the GPU the IDCT and color conversion.
- **What batching does:** it removes the per-call latency, from 2.7–3.5 ms down to 1.8–2.3 ms per small Pinterest
  file, and then levels off: about 450–560 images/s on one core.
- **Against the CPU decode `analyze` actually uses:**
  - **Small files:** against libjpeg-turbo's 1/2 decode (2.3–5.6 ms in WSL), nvJPEG with batching is 0.9–1.3×
    as fast once the pixels are copied back to host memory, which PDQ and the resizes need. Left on the GPU,
    it's 1.2–1.6×.
  - **Camera photos:** it's no faster or slower (26–62 ms against 24–49 ms), since the CPU decode skips most of
    the IDCT and nvJPEG can't scale.
- **Throughput:** libjpeg-turbo spreads over 12 threads, while nvJPEG here is bounded by its one Huffman thread.
  The GPU is also what SSCD needs (FINDINGS "I/O": the full pass is GPU-bound at 410 img/s).
- **Windows against WSL:** Pillow on Windows (libjpeg-turbo) is within 10% of the WSL libjpeg-turbo times on
  camera files (111 against 124 ms, 47 against 49 ms), so the two tables compare.

## nvJPEG with Huffman decoding on the GPU

`nvjpeg.py`, 2026-10-31; RTX 2060, nvJPEG 13.2.3, the DLL torchvision
ships, machine at ~25% load from a VMware VM. torchvision's `decode_jpeg` has no backend option, so `nvjpeg.py`
calls nvJPEG through ctypes and compares its two backends for this GPU:

- **`HYBRID`:** Huffman decoding on the CPU, the rest on the GPU. This is what torchvision measured above.
- **`GPU_HYBRID`:** Huffman decoding on the GPU too. The batched API (`nvjpegDecodeBatched`) only uses it for
  batches over 50 baseline interleaved files; the decoupled API (one image at a time) uses it per image.

The CUDA context waits by sleeping (`cudaDeviceScheduleBlockingSync`), so CPU time counts work. CUDA's default
spins while it waits, which bills the GPU's time to the CPU: in the first run, every row showed CPU time equal to
wall time, including `GPU_HYBRID` at batch 64, which with blocking sync costs 0.03 ms. The torchvision rows above
were measured with spinning; `HYBRID` measured here still costs the CPU 0.8–1.0× its wall time, so their
conclusion stands.

Per image, decoded pixels copied to pinned host memory. Batches are N copies of one file.

| Input | Pillow 1/d (scaled) | `HYBRID` batch 64 | `GPU_HYBRID` batch 64 | `GPU_HYBRID` batch 128 | `GPU_HYBRID` decoupled |
|---|---:|---:|---:|---:|---:|
| pin-base444-1mp | 4.96 ms | 3.79 ms, CPU 3.21 | **1.22 ms, CPU 0.02** | 0.85 ms, CPU 0.04 | 3.89 ms, CPU 2.96 |
| pin-base444-3mp | 6.51 ms | 7.73 ms, CPU 6.64 | **2.85 ms, CPU 0.08** | 2.15 ms, CPU 0.06 | 4.85 ms, CPU 3.09 |
| pin-prog420-1mp | 5.52 ms | 3.25 ms, CPU 2.58 | 3.32 ms, CPU 2.62 | 3.80 ms, CPU 3.56 | 4.19 ms, CPU 3.61 |
| pin-base420-1mp | 4.13 ms | 2.67 ms, CPU 2.09 | **1.13 ms, CPU 0.06** | 0.85 ms, CPU 0.08 | 3.84 ms, CPU 3.12 |
| phone-base420-4mp | 22.54 ms | 25.62 ms, CPU 24.07 | **12.65 ms, CPU 0.54** | 8.12 ms, CPU 0.56 | 17.72 ms, CPU 11.81 |
| sony-base422-7mp | 62.24 ms¹ | 63.05 ms, CPU 45.70 | **15.18 ms, CPU 0.44** | — | 19.44 ms, CPU 15.02 |
| a6000-base422-24mp | 49.09 ms | — | — | — | 42.34 ms, CPU 31.71 |

¹ Noisy: 29–31 ms in the two later runs.

- **Batches over 50 move the whole decode off the CPU:** 0.02–0.08 ms of CPU per Pinterest file and about 0.5 ms
  per camera file, against 4–7 ms and 24–50 ms of CPU for Pillow's scaled decode. Batch 50 and below (1, 8, 32), it runs
  exactly like `HYBRID`.
- **It's also the fastest decode here:** 1.1–2.9 ms per Pinterest file, and 13–15 ms per camera file. These are
  full-size decodes against Pillow's scaled ones; nvJPEG can only scale in its hardware backend.
- **Exceptions:**
  - **Progressive files** (17% of the Pinterest census) always decode Huffman on the CPU.
  - **The 24 MP file doesn't fit a batch over 50:** 51 × 72 MB of output is over what the 6 GB card has free.
    The decoupled API is the only GPU route for it: 42 ms to host, against Pillow's 140 ms full and 49 ms at 1/8,
    but still 32 ms of CPU (Pillow 1/8: 50 ms), so GPU Huffman does only part of the work.
  - **The decoupled API doesn't help small files:** its CPU time stays near `HYBRID`'s. It helps camera files
    (CPU 12–32 ms against `HYBRID`'s 28–63 ms).
- **Same pixels as `HYBRID`** on all seven files (max difference 0).
- **Smooth chroma upsampling is a flag:** `NVJPEG_FLAGS_UPSAMPLING_WITH_INTERPOLATION` (`--interpolate`) brings
  nvJPEG within 3 levels of Pillow on 4:2:0 and 4:2:2: 64 dB on pin-base420-1mp (from 50), 61 dB on the phone
  photo (from 29.5), 63 dB on the Sony (from 59). It costs nothing measurable. It's still not bit-exact, so
  `pixel_hash` values would change.
- **Throughput, not latency:** the GPU at about 820–880 small files/s (to host, batch 64) is likely less than all
  12 CPU threads decode at 1/2 (one thread: 200–270/s; not measured across threads). It adds decode capacity and
  frees the CPU for PDQ, ThumbHash and the resizes, but its GPU time competes with SSCD, which keeps this GPU busy
  at 410 img/s (FINDINGS "I/O").

## Restart markers and lossless baseline conversion

`restart.sh`, 2026-10-31, WSL. Every fast GPU decoder
outside nvJPEG relies on restart markers to split the Huffman stream (GPUJPEG, the Fastvideo SDK, parajpeg's
fastest path). None of the seven inputs has any, and `jpegtran` can add them, or turn progressive into baseline,
without re-encoding. Variants made with jpegtran 2.1.5 `-copy all -optimize`:

- **baseline:** no markers;
- **every 10 MCUs** (`-restart 10B`, the Fastvideo encoder's interval);
- **every MCU row** (`-restart 1`).

Decoded pixels are identical to the original's in every variant, at full size and 1/2 (`djpeg` output compared
byte for byte). Instructions are the vendored libjpeg-turbo's, per decode.

| Input | baseline: size | M instr. full / scaled | row markers: size | M instr. full / scaled |
|---|---:|---:|---:|---:|
| pin-base444-1mp | 0.0% | 31.1 / 25.1 | +0.4% | 32.7 / 26.7 (+5%, +6%) |
| pin-base444-3mp | 0.0% | 77.9 / 60.8 | +0.4% | 81.4 / 64.2 (+4%, +6%) |
| pin-prog420-1mp | −0.1% | **16.3 / 11.8** (from 42.4 / 37.9) | +0.2% | 17.4 / 12.9 (+7%, +9%) |
| pin-base420-1mp | 0.0% | 28.7 / 21.7 | +0.2% | 30.9 / 23.9 (+8%, +10%) |
| phone-base420-4mp | −14.0% | 243.0 / 212.1 (from 247.5 / 216.6) | −14.0% | 288.1 / 257.2 (+19%, +21%) |
| sony-base422-7mp | −23.3%¹ | 356.3 / 285.4 | −23.3%¹ | 405.5 / 334.6 (+14%, +17%) |
| a6000-base422-24mp | −9.1%¹ | 832.7 / 376.2 | −9.1%¹ | 923.1 / 455.0 (+11%, +21%) |

The marker percentages compare with the baseline column. ¹ Mostly not compression: jpegtran drops what follows the
first image's end marker, which is where the cameras put their MPF preview (1616×1080 on the A6000, 1920×1080 on
the Sony; the 160×120 EXIF thumbnail survives). Without it, the saving is about 3% on the A6000 and under 1% on
the Sony. The phone's 14% is real Huffman optimization; its 512×288 preview sits inside EXIF and is kept.

- **Progressive → baseline is a clear win:** the progressive file takes 2.6× fewer instructions full size and 3.2×
  fewer scaled, decodes in 1.99 ms instead of 6.46 (1.35 instead of 3.76 at 1/2), with identical pixels and
  0.1% smaller. It also qualifies for nvJPEG's GPU Huffman: batch 64 in 0.70 ms per image with 0.02 ms of CPU,
  against 3.3 ms and 2.6 ms of CPU as progressive. These are the originals ImageReduce keeps when its re-encode
  comes out bigger, so the fix belongs there.
- **Restart markers cost the current paths:** libjpeg-turbo takes 4–21% more instructions with a marker per MCU
  row, more on the high-bitrate camera files; wall times grew up to 8% full size and 13% scaled. And nvJPEG's batched `GPU_HYBRID` decodes
  Huffman on the CPU again for files with markers (phone at batch 64: 32 ms of CPU, against 0.5 ms without);
  its decoupled API is unaffected. A marker every 10 MCUs costs slightly more instructions, and 1.7–10.7% more size on
  the Pinterest files. So markers only pay off with a decoder that uses them (GPUJPEG, Fastvideo, parajpeg), and
  none is adopted.
- **Don't run `jpegtran -optimize` on camera originals:** it deletes their large embedded preview.

## Other decoders, researched but not measured

As of 2026-10-31.

- **parajpeg** ([anniigr/parajpeg](https://github.com/anniigr/parajpeg), version 0.1.0): splits one image across
  threads without restart markers, by speculative self-synchronising Huffman decoding. Output byte-identical to
  libjpeg-turbo's accurate IDCT. On an Apple M3 with 8 threads: 2.8–3.1× without markers, 4.4–5.4× with,
  2.2–3.0× progressive; one image per core is 0.94× libjpeg-turbo's throughput. No DCT scaling, and the x86
  AVX2 path has no published numbers. Fits full-size decodes for the viewer, not `analyze` or thumbnails.
- **jpeggpu** ([nolmoonen/jpeggpu](https://github.com/nolmoonen/jpeggpu), active since 2023): CUDA, GPU Huffman
  for one image without markers (Weißenberger and Schmidt, 2021). On an RTX 2070: 1.26 ms against nvJPEG's
  6.89 ms on 6 MP. Baseline only, no scaling, experimental.
- **GPUJPEG** ([CESNET/GPUJPEG](https://github.com/CESNET/GPUJPEG)): CUDA encoder and decoder. GPU Huffman only
  with restart markers; without them, Huffman decoding runs on the CPU and its FAQ calls decoding slow.
- **Fastvideo SDK** (commercial): the vendor's benchmark puts it 19–75× faster with markers than without, and
  without markers it decodes Huffman on the CPU.
- **nvImageCodec** ([NVIDIA/nvImageCodec](https://github.com/NVIDIA/nvImageCodec)): nvJPEG and libjpeg-turbo
  behind one API, with mixed-size batches and fallback. The layer to use if the app ever decodes on the GPU.
- **NVDEC** (FFmpeg `mjpeg_cuvid`, measured once): the 2060's video engine decodes 4:2:0 JPEG only. It rejects
  4:4:4 and 4:2:2, and on 4:2:0 it was no faster than FFmpeg's CPU decoder, with the same CPU time.
- **Not useful here:** Wuffs (no published JPEG numbers), jpegli (its work is on encoding), epeg (libjpeg's DCT
  scaling, which we use), jpegn (Go), and course projects (Fulim13/jpeg writes a custom format;
  debesheedas/GPU-JPEG-Decoder needs 4:4:4 input).

## Next

- **Progressive originals → baseline in ImageReduce:** the one change measured to pay off with nothing lost
  (2.6× fewer instructions, identical pixels). Existing `*.mozjpeg.jpg` files that are still progressive need a
  one-off pass.
- **jpeggpu on the RTX 2060:** the only decoder found that decodes Huffman on the GPU for a single image without
  restart markers, which nvJPEG does poorly.
- **parajpeg's AVX2 build on the i7-8700:** full-size decodes for the viewer. Its output is claimed byte-identical
  to libjpeg-turbo's, so it would leave `pixel_hash` alone.
- **Wuffs in the harness:** cheap to add; it has no published JPEG numbers.
- **GPU decoding for the background thumbnail job**, if wanted: nvImageCodec, batches over 50, `--interpolate`,
  with the GPU shared with SSCD.

## To rerun

- **CPU decoders**, from PowerShell in the repo: `wsl -d Ubuntu-26.04 -u root -- bash eval/jpeg-decoders/wsl.sh
  inputs check compare time bench marginal`. This needs `apt-get install libvips-dev` in the distro. The inputs
  are library photos, so they're copied to `~/jpeg-inputs` in WSL and never into the repo.
- **GPU:** `D:\PhotoGallery\venv-eval\Scripts\python.exe -I eval\jpeg-decoders\gpu.py FILE...` (torchvision), and
  `... eval\jpeg-decoders\nvjpeg.py [--interpolate] [--batches 1,64] FILE...` (the two nvJPEG backends).
- **Restart markers and baseline conversion:** `wsl -d Ubuntu-26.04 -u root -- bash eval/jpeg-decoders/restart.sh`
  (needs `apt-get install libjpeg-turbo-progs`). It keeps the variants in `~/jpeg-restart` for GPU runs.
