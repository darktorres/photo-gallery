# JPEG decoders

Which JPEG decoder the app should use: libjpeg-turbo, as `pg-codec` calls it, against every other decoder found
(eight on the CPU, six on the GPU) on seven library files, plus restart markers and lossless baseline conversion
(`eval/jpeg-decoders`, 2026-10-31). Instruction counts use the callgrind setup in [PROFILING.md](PROFILING.md).
Encoders are in [JPEG_ENCODERS.md](JPEG_ENCODERS.md).

## Verdict: keep libjpeg-turbo

- **Fewest instructions:** with its default or fast options, it uses the fewest instructions of any CPU decoder on
  every input except two, both FFmpeg's: the progressive file, and the phone photo at 1/4, by 1.5%.
- **Fastest scaled decode:** its DCT-scaled decode, which `analyze` uses, is the fastest decode on every file
  except one progressive file, where FFmpeg is faster.
- **The GPU frees the CPU only with GPU Huffman decoding:** nvJPEG as torchvision runs it (CPU Huffman) keeps one
  CPU core busy for as long as it decodes. Its `GPU_HYBRID` backend, in batches over 50, costs the CPU 0.02–0.6 ms
  per image and decodes 2–4× faster ("nvJPEG with Huffman decoding on the GPU" below).
- **For throughput, the CPU wins except against jpeggpu:** libjpeg-turbo on 12 workers decodes more images per
  second than every other GPU path. jpeggpu on 4 workers beats it 1.1–2.9× on camera files with 3–4 ms of CPU per
  image against 43–148 ms ("Throughput" below). Either way, what the GPU buys is CPU time for the rest of the
  import.
- **Two decoders are worth keeping in view** ("Every decoder in one harness" below):
  - **parajpeg** cuts the latency of one full-size decode 2.8–2.9× on camera files with 12 threads, and 3.5–4.7×
    with restart markers, with pixels identical to libjpeg-turbo's. It costs 2.8–4.3× the CPU time (1.6–2.5× with
    markers), so it suits the viewer, not the batch jobs.
  - **jpeggpu** decodes one image on the GPU in 1.1–2.1 ms (Pinterest) and 10.6 ms (24 MP, 8.8× libjpeg-turbo),
    with 0.3–1.6 ms of CPU and no restart markers needed. But it is experimental, rejects progressive files and
    stops at YCbCr planes: upsampling and colour conversion would be ours to write.

Changing to a decoder whose pixels differ (every one measured here except libvips, parajpeg and Wuffs) would also
change every stored `pixel_hash`, which would mean a re-import.

## All decoders at a glance

Speeds are against libjpeg-turbo on the seven inputs: wall time on one thread, the
full-size table below, unless noted. "Claimed" figures come from the project's own benchmarks on other hardware.

| Decoder | Runs on | Measured here | DCT scaling | Pixels against the app's decode | Speed | Verdict |
|---|---|---|---|---|---|---|
| **libjpeg-turbo 3.1** (vendored) | CPU | yes | 1/2–1/8 | the reference | fastest scaled decode; fewest instructions | **keep** |
| libjpeg-turbo, fast options | CPU | yes | yes, no fast variant | 29 dB on 4:2:0 | 3–11% fewer instructions full size, none scaled | no |
| Pillow 13 (libjpeg-turbo) | CPU | yes (Windows) | `draft()`, decoded sizes round up (`ceil(w/d)`) | same library | within 10% of the WSL times | reference only |
| zune-jpeg 0.5.15 | CPU (Rust) | yes | **no** | within 4 levels (50–73 dB) | 3–35% slower full size | no |
| jpeg-decoder 0.3.2 | CPU (Rust), worker threads | yes | 1/2–1/8 | within 4 levels (50–73 dB) | 1.3–3.0× slower, despite extra threads | no |
| FFmpeg 8 `mjpeg` | CPU | yes | `lowres` 1/2–1/8 | 29 dB on 4:2:0 | 1.3× faster on progressive; 1.2–3.2× slower otherwise (counted before the C-50 fix, see the note below the FFmpeg tables) | no |
| libvips 8.18 | CPU (libjpeg-turbo) | yes | shrink-on-load, rounds down | identical at full size | 1.2–2.0× slower; its threads don't speed up a decode | no |
| nvJPEG `HYBRID` (torchvision's) | GPU + CPU Huffman | yes | no | 69 dB on 4:4:4, 29.5–59 dB subsampled; 60–64 dB with `--interpolate` | batch 64: 1.8–33 ms; CPU time ≈ wall time, measured with CUDA's spinning wait (at batch 1 the CPU time fell from 30.9 to 19.9 ms with blocking sync, "nvJPEG through torchvision" below) | no |
| nvJPEG `GPU_HYBRID`, batches over 50 | GPU | yes | no | same as `HYBRID` | 1.1–2.9 ms Pinterest, 13–15 ms camera, CPU 0.02–0.6 ms; not progressive or restart-marker files; 24 MP batches don't fit 6 GB | candidate for the thumbnail job |
| nvJPEG `GPU_HYBRID`, one image at a time | GPU, partly CPU | yes | no | same as `HYBRID` | camera files: CPU 12–32 ms, against 28–63 ms with CPU Huffman; no gain on small files | camera files only |
| nvJPEG `HARDWARE` | GPU JPEG engine | no: A100/A30, Hopper, Blackwell | 1/2–1/8 | — | — | not on the RTX 2060 |
| NVDEC (`mjpeg_cuvid`) | GPU video engine | yes (2 files) | no | — | 4:2:0 only; no faster than FFmpeg's CPU decoder, same CPU time | no |
| Intel Quick Sync (`mjpeg_qsv`) | iGPU | no: UHD 630 disabled in the BIOS | — | — | — | only if the iGPU is enabled |
| parajpeg 0.1.0, CPU | CPU, threads within one image (AVX2) | yes | **no** | **identical** | 1 thread: 1.0–1.9× slower; 12 threads: 2.8–2.9× faster on camera files (3.5–4.7× with markers) at 2.8–4.3× the CPU | for full-size viewer decodes |
| parajpeg 0.1.0, CUDA | GPU | yes | no | **identical** | 5–8 ms on Pinterest files (slower than the CPU); 1.9–2.8× faster on camera files, CPU 4.8–15 ms | no: its CPU backend is better |
| jpeggpu | GPU, GPU Huffman without markers | yes | no | planes within 1 level (63–74 dB) | **1.1–2.1 ms Pinterest, 4.3 ms phone and Sony, 10.6 ms 24 MP; CPU 0.3–1.6 ms**; planes only, no progressive | the GPU candidate, if we write the colour stage |
| GPUJPEG (CESNET) | GPU | yes | no | 57–64 dB on 4:4:4; 29.5–50 dB subsampled; no progressive | without markers: up to 1.9× slower than libjpeg-turbo (CPU Huffman); with markers: 1.8–24.5 ms, CPU 0.2–1.4 ms | only with restart markers, and its chroma is off |
| Wuffs (2026-10-31 snapshot) | CPU, memory-safe | yes | no | **identical** | instructions: equal on 4:4:4, 7–24% more on subsampled, 41% more progressive | the memory-safe option; no scaling |
| jpegli (decoder) | CPU | yes | — | 49–52 dB: its own dequantization, by design | 1.8–2.2× the instructions | no |
| nvImageCodec 0.10.0.9 | wrapper: nvJPEG + a CPU decoder | yes | as its backends | as nvJPEG (60–69 dB) | as nvJPEG's default backend; batches use CPU threads, and their CPU time (still about 2× wall at batch 64 with blocking sync) does not show GPU Huffman in use; CPU backend 2.0–3.5× slower | no: call nvJPEG directly |
| debesheedas/GPU-JPEG-Decoder | GPU, one warp | yes | no | 51 dB on 4:4:4; garbage on 4:2:0 or markers | ~1.3 s per megapixel | no |
| Fastvideo SDK | GPU (CUDA), commercial | no | — | — | claimed 19–75× faster with restart markers; CPU Huffman without | only with restart markers |
| epeg | wrapper: libjpeg DCT scaling | no | yes | — | the technique we already use | the YUV-output idea, for PDQ |
| jpegn (Go), Fulim13/jpeg | — | no | — | — | jpegn is Go-only; Fulim13/jpeg uses a custom format | no |

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

- **libjpeg-turbo:** the vendored 3.x, called as `pg-codec` does: accurate IDCT, smooth upsampling.
- **libjpeg-turbo fast:** the same, with `TJPARAM_FASTDCT` and `TJPARAM_FASTUPSAMPLE`.
- **zune-jpeg 0.5.15:** the `image` crate's decoder since 0.25. It has no DCT scaling.
- **jpeg-decoder 0.3.2:** the `image` crate's decoder before 0.25. Even without rayon it decodes in worker
  threads, so its wall times got help from extra threads.
- **FFmpeg 8 `mjpeg`:** `lowres` for scaling, then swscale to RGB, as `crates/video/src/lib.rs` converts.
- **libvips 8.18.0:** `jpegload_buffer` with shrink-on-load, sequential, `concurrency` 1, operation cache off,
  plus a copy out of its buffer. It decodes with Ubuntu's libjpeg-turbo 2.1.5, in a worker thread.
- **nvJPEG:** through torchvision 0.29 (CUDA 13.4) on Windows (`gpu.py`), with Pillow 13 and pyvips 8.18.7 as
  CPU references in the same process.

## Instructions per decode

Millions of instructions; callgrind, Linux build. Most come from gungraun. libvips and jpeg-decoder do
their work in threads gungraun doesn't count, so their figures come from whole runs instead (`wsl.py marginal`:
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

**The FFmpeg instruction counts above predate the C-50 fix and were not re-measured.** Before it, `Decoders::new` left
FFmpeg's MJPEG decoders unopened, so the one measured decode in the callgrind bench also opened a decoder, and every
column's input was dropped inside the measured call. Now the four decoders (one per `lowres`) open in `Decoders::new` and
`run` returns the input, so neither is counted. The FFmpeg counts are therefore probably too high, by an amount not
measured: callgrind was not run after the change, so the rows are not updated. Whether the wall-time rows below (from
`wsl.sh`) were affected has not been checked.

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

From `tools/profiling/cgtab.py` on the gungraun outputs.

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

| Input | Pillow full / draft² | nvJPEG batch 1 | batch 8 | batch 32 | batch 64 |
|---|---:|---:|---:|---:|---:|
| pin-base444-1mp | 4.44 / 4.46¹ | 2.65 (3.02) | 1.87 (2.33) | 1.79 (2.30) | 1.78 (2.25) |
| pin-base444-3mp | 10.33 / 6.46 | 6.14 (6.98) | 4.27 (5.45) | 4.12 (5.26) | 4.04 (5.28) |
| pin-prog420-1mp | 6.04 / 4.67 | 3.53 (3.46) | 2.31 (2.86) | 2.25 (2.72) | 2.25 (2.72) |
| pin-base420-1mp | 4.39 / 4.43¹ | 3.49 (3.64) | 1.97 (2.53) | 1.92 (2.47) | 1.90 (2.44) |
| phone-base420-4mp | 28.58 / 21.83 | 33.62 (38.98) | 27.11 (31.29) | 26.60 (28.45) | 26.23 (27.99) |
| sony-base422-7mp | 44.07 / 28.00 | 48.43 (41.65) | 30.24 (34.30) | 29.46 (32.16) | 29.18 (32.60) |
| a6000-base422-24mp | 111.14 / 47.24 | 85.53 (100.62) | 62.34 (74.46) | 61.93 (72.98) | — |

¹ These two rows are full decodes: `gpu.py` asked Pillow's `draft()` for the ceiling size, `ceil(w/d)`, and for
these files Pillow then picked no reduction.

² The column was measured before C-47 was fixed. Pillow's `draft()` picks the largest DCT scale that still gives at
least the requested size, and the decoded size is the ceiling (3840×2151 at 1/4 is 960×538, not 960×537), so a
ceiling request can select a larger image than `1/d` for a side that isn't a multiple of `d`. `gpu.py` now requests
the floor size, as `nvjpeg.py` does, and raises if the decoded size is not `ceil(w/d) × ceil(h/d)`. The figures above
were not re-measured, and the other rows were not checked for the same effect, because the old script did not check
the decoded size.

- **The CPU was the limit, measured with CUDA's spinning wait.** The process's CPU time per image equalled the wall
  time at every batch size, but that includes the time the CPU spent spinning for the GPU. With blocking sync
  (`blocking_sync()` in `gpu.py`) the CPU time at batch 1 fell from 30.9 to 19.9 ms, so about 7–11 ms of it was the
  spin. These two runs are `gpu_spin.txt` and `gpu_blocking.txt`; the files are not in the repository, and the
  figures here were not reproduced in this pass. Turing has no hardware JPEG decoder, so nvJPEG's hybrid backend does
  the Huffman decoding on one CPU thread and leaves the GPU the IDCT and color conversion. The table above has not
  been re-measured with blocking sync.
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
were measured with spinning, and `HYBRID` measured here (with blocking sync) still costs the CPU 0.8–1.0× its wall
time. The torchvision conclusion that the CPU is the limit therefore stands, but its CPU figures are overstated by the
spin (30.9 against 19.9 ms at batch 1). Those two runs are not in the repository; `nvjpeg.py` itself was run on
nvJPEG 13.2.3.

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

## Every decoder in one harness

`eval/jpeg-decoders/harness`, 2026-10-31: one C++ program, built on Windows (MSVC, CUDA 13.4) and in WSL
(clang), decodes with every library through one interface, so they're measured alike. Each decoder keeps its
buffers between calls. GPU decoders copy their output to pinned host memory and wait for it, and the CUDA context
sleeps while it waits, so CPU time is real work. Timing is the Rust harness's: warm-up decodes, then samples for at
least 2 s and 5 samples, the median; CPU time is the process's (all threads) per decode. Output is compared with
libjpeg-turbo's (jpeggpu, which returns planes, with libjpeg-turbo's planes).

The inputs are the seven files as they are, the progressive one converted losslessly to baseline, and each with a
restart marker every MCU row (`restart.py`; jpegtran's output is baseline, so the row-marker progressive file is
baseline too).

| Decoder | Version | Built |
|---|---|---|
| libjpeg-turbo | 3.1.0, the vendored copy | Windows and WSL |
| parajpeg | f45081d (2026-10-21), with its CUDA backend | Windows (clang-cl, nvcc) and WSL (CPU) |
| jpeggpu | 3ae7ac0 | Windows |
| GPUJPEG | 37ab481 | Windows |
| nvJPEG | 13.2 (CUDA 13.4 toolkit) | Windows |
| jpegli | 031a007 (2026-06-01) | WSL |
| Wuffs | d28ecd4 (2026-10-31), `wuffs_aux::DecodeImage` to RGBA (its AVX2 path) | WSL |

The first Windows run was discarded: a VMware VM was taking 3.3 cores, which made single-thread decodes 30–50%
slower whenever they shared a core with it. The run below had 0.3–0.6 cores of other load.

### Instructions per decode (CPU decoders)

Millions; callgrind in WSL, all threads, (3 decodes − 1) / 2. libjpeg-turbo's counts are 1–4% below the
gungraun ones above (this harness's C++ build against the Rust one).

| Input | turbo | turbo planes | parajpeg 1 thread | parajpeg 12 threads | jpegli | Wuffs |
|---|---:|---:|---:|---:|---:|---:|
| pin-base444-1mp | 29.8 | 26.0 | 43.0 | 59.6 | 64.1 | **29.6** |
| pin-base444-3mp | 74.3 | 64.5 | 110.1 | 143.3 | 163.3 | **73.7** |
| pin-prog420-1mp | 42.9 | 38.1 | **39.3** | 40.8 | 76.0 | 60.6 |
| pin-prog420-1mp, baseline | **15.8** | 11.0 | 25.9 | 34.6 | 34.5 | 18.5 |
| pin-base420-1mp | **28.0** | 20.9 | 41.8 | 57.2 | 57.5 | 30.0 |
| phone-base420-4mp | 244.6 | 223.5 | **230.9** | 415.5 | 427.7 | 294.0 |
| sony-base422-7mp | **349.8** | 310.7 | 359.4 | 584.3 | 631.4 | 432.8 |
| a6000-base422-24mp | **819.9** | 695.2 | 957.0 | 1,409.9 | 1,763.4 | 1,013.6 |
| *row markers:* | | | | | | |
| pin-base444-1mp | 31.0 | 27.1 | 43.1 | 43.2 | 64.2 | **29.8** |
| phone-base420-4mp | 270.0 | 248.9 | **223.8** | 224.6 | 424.5 | 288.7 |
| a6000-base422-24mp | **876.7** | 752.0 | 956.2 | 958.3 | 1,762.0 | 1,010.2 |

- **Wuffs is libjpeg-turbo's equal on baseline 4:4:4,** 81% of the Pinterest census, and 7–24% behind on
  subsampled files and 41% on progressive. Its pixels are identical. It has no DCT scaling, which `analyze` needs.
- **parajpeg on one thread** costs 1.0–1.6× libjpeg-turbo's instructions, and slightly fewer on the phone photo
  and the progressive file.
- **parajpeg's speculative decoding is the price of its threads:** without markers, 12 threads execute 30–80% more
  instructions than one on baseline files (4% on the progressive one); with a marker per row, none, since each segment decodes independently.
- **jpegli's decoder costs about twice libjpeg-turbo's** (it decodes in floating point).
- **Planes only** (no upsampling, no colour conversion) saves 9–30%. That is the share of the decode that
  jpeggpu leaves out.

### Wall and CPU time per decode on Windows

Median ms, with process CPU ms in brackets. Windows process times tick every 15.6 ms, so CPU figures below about
0.1 ms aren't resolved. nvJPEG uses smooth chroma upsampling (`NVJPEG_FLAGS_UPSAMPLING_WITH_INTERPOLATION`).

| Input | turbo | parajpeg 1 thread | parajpeg 12 threads | parajpeg CUDA | nvJPEG | nvJPEG GPU Huffman | jpeggpu (planes) | GPUJPEG |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| pin-base444-1mp | 3.33 (3.4) | 5.98 (6.5) | 2.86 (11.6) | 5.44 (1.2) | 2.88 (1.9) | 2.44 (1.7) | **1.33 (0.3)** | 3.61 (2.6) |
| pin-base444-3mp | 7.98 (8.3) | 15.19 (15.6) | 5.30 (35.3) | 8.15 (2.3) | 6.69 (4.0) | 4.50 (2.8) | **2.11 (0.4)** | 7.84 (6.2) |
| pin-prog420-1mp | 5.26 (5.5) | 5.49 (5.7) | **3.22 (9.3)** | 5.13 (5.0) | 3.41 (2.2) | 4.30 (2.8) | not supported | not supported |
| pin-prog420-1mp, baseline | 1.73 (1.7) | 3.04 (3.1) | 1.37 (5.7) | 6.12 (1.3) | 1.66 (1.0) | 1.99 (1.5) | **1.12 (0.4)** | 2.24 (1.4) |
| pin-base420-1mp | 3.03 (3.1) | 5.39 (5.6) | 2.05 (10.3) | 5.01 (1.5) | 2.77 (2.1) | 2.37 (1.7) | **1.18 (0.4)** | 3.88 (3.5) |
| phone-base420-4mp | 26.35 (28.2) | 31.79 (32.8) | 9.57 (83.2) | 13.85 (4.8) | 28.76 (26.7) | 16.88 (10.2) | **4.31 (1.0)** | 49.19 (47.3) |
| sony-base422-7mp | 37.06 (37.7) | 49.21 (50.0) | 13.15 (106.9) | 13.16 (5.6) | 34.33 (30.7) | 16.89 (11.4) | **4.32 (0.8)** | 59.34 (56.1) |
| a6000-base422-24mp | 93.20 (93.8) | 136.10 (135.4) | 31.60 (275.5) | 36.80 (14.8) | 77.58 (62.5) | 37.56 (25.8) | **10.56 (1.6)** | 120.55 (106.6) |
| *row markers:* | | | | | | | | |
| pin-base444-1mp | 3.29 (3.4) | 6.18 (6.6) | **1.31 (10.1)** | 4.30 (1.3) | 2.94 (2.2) | 2.42 (1.8) | 1.45 (0.5) | 1.85 (0.3) |
| pin-base444-3mp | 8.03 (8.3) | 15.22 (15.4) | 3.46 (26.9) | 8.08 (2.2) | 6.99 (5.3) | 4.61 (3.0) | **2.24 (0.4)** | 3.30 (0.6) |
| pin-prog420-1mp (baseline) | 1.74 (1.8) | 3.04 (3.1) | **0.62 (5.4)** | 4.59 (1.0) | 1.72 (1.0) | 1.95 (1.4) | 1.18 (0.4) | 2.20 (0.2) |
| pin-base420-1mp | 3.05 (3.1) | 5.31 (5.3) | **1.12 (8.8)** | 7.88 (2.0) | 2.81 (1.9) | 2.46 (1.9) | 1.24 (0.4) | 3.42 (0.3) |
| phone-base420-4mp | 26.01 (27.2) | 30.13 (30.8) | 5.58 (44.6) | 58.95 (3.2) | 27.89 (26.0) | 11.56 (10.9) | **4.41 (0.8)** | 23.57 (0.7) |
| sony-base422-7mp | 40.80 (41.5) | 49.76 (50.0) | 9.38 (83.0) | 38.66 (6.4) | 35.51 (30.4) | 15.41 (11.4) | **4.67 (1.3)** | 16.96 (1.1) |
| a6000-base422-24mp | 89.51 (90.4) | 134.84 (133.3) | 25.25 (228.2) | 55.02 (11.7) | 78.40 (66.3) | 36.53 (24.0) | **10.41 (1.4)** | 24.50 (1.4) |

- **jpeggpu is the fastest single-image decode on every file without markers:** 1.5–8.8× faster than
  libjpeg-turbo (6.1–8.8× on camera files), with 2–23% of its CPU time. With markers, parajpeg's 12 threads beat
  it on the 1 MP files. Its output stops at YCbCr planes on the GPU; the upsampling and colour conversion it skips
  are a small kernel (nvJPEG does them in its GPU time). It rejects progressive files.
- **parajpeg's threads pay off on camera files:** 2.8–2.9× faster without markers, 3.5–4.7× with. On Pinterest
  files they gain 1.2–1.6× without markers and 2.3–2.8× with. They take 2.8–4.3× the CPU time without markers.
  One thread is 1.0–1.9× slower than libjpeg-turbo.
- **parajpeg's CUDA backend loses to its CPU backend:** 4–8 ms on the Pinterest files, which libjpeg-turbo decodes
  in 1.7–8 ms, and slower with markers (59 ms on the phone photo).
- **nvJPEG one image at a time:** with Huffman on the CPU it runs at 0.65–1.1× libjpeg-turbo's wall time, with
  40–95% of its CPU time. Its decoupled `GPU_HYBRID` API brings the camera files to 40–64% of libjpeg-turbo's time
  (17–38 ms, 10–26 ms of CPU) and gains little on small ones.
- **GPUJPEG needs markers:** without them its CPU Huffman decoder is up to 1.9× slower than libjpeg-turbo; with them
  it decodes on the GPU with 0.2–1.4 ms of CPU, but at 2.4–5.3× jpeggpu's wall time on camera files.
- **Restart markers don't slow libjpeg-turbo's wall time** measurably here (−4% to +10%), against 4–21% more
  instructions.

### Wall and CPU time per decode in WSL (CPU decoders)

The same harness, clang build, about 1 core of other load. Median ms (CPU ms); the files as they are.

| Input | turbo | parajpeg 1 thread | parajpeg 12 threads | jpegli | Wuffs |
|---|---:|---:|---:|---:|---:|
| pin-base444-1mp | **3.36 (3.6)** | 6.48 (6.7) | 3.23 (13.6) | 7.60 (8.4) | 4.00 (4.3) |
| pin-base444-3mp | 10.48 (10.9) | 18.64 (19.3) | **5.75 (36.6)** | 23.04 (24.9) | 10.37 (11.1) |
| pin-prog420-1mp | 5.04 (5.7) | 5.26 (5.5) | **3.41 (8.0)** | 8.22 (8.5) | 6.72 (7.0) |
| pin-prog420-1mp, baseline | **1.78 (2.0)** | 3.64 (3.8) | 1.83 (7.2) | 4.98 (5.5) | 2.13 (2.3) |
| pin-base420-1mp | 3.38 (3.5) | 5.45 (5.8) | **2.21 (12.1)** | 6.20 (6.3) | 3.46 (3.6) |
| phone-base420-4mp | 26.97 (27.7) | 29.79 (30.2) | **9.38 (91.7)** | 39.77 (40.2) | 35.09 (35.7) |
| sony-base422-7mp | 38.85 (39.0) | 45.32 (45.9) | **16.84 (142.5)** | 55.65 (56.1) | 50.59 (52.4) |
| a6000-base422-24mp | 99.76 (113.1) | 158.03 (162.1) | **37.73 (368.9)** | 161.84 (161.0) | 114.71 (116.6) |

On 12 workers (images/s, CPU ms per image):

| Input | turbo | parajpeg 1 thread each | jpegli | Wuffs |
|---|---:|---:|---:|---:|
| pin-base444-1mp | **1,731 (6.7)** | 970 (12.2) | 823 (14.3) | 1,088 (10.8) |
| pin-base420-1mp | **1,966 (5.6)** | 894 (13.2) | 804 (14.4) | 1,348 (8.7) |
| phone-base420-4mp | **223 (51.0)** | 194 (56.7) | 114 (92.5) | 177 (66.1) |
| a6000-base422-24mp | **64 (182)** | 48 (240) | 41 (284) | 46 (247) |

- **Wuffs runs at 1.0–1.3× libjpeg-turbo's time on one thread,** in line with its instruction counts, but at
  0.57–0.79× its throughput on 12 workers, probably because it writes 4 bytes per pixel against 3 and the workers
  share the memory bus.
- **jpegli decodes at 1.4–2.3× libjpeg-turbo's time,** at 0.4–0.6× its throughput.
- **parajpeg in WSL is close to the Windows build:** 2.3–2.9× faster than libjpeg-turbo on camera files with 12
  threads (2.8–2.9× on Windows).

### Throughput

Images per second, with CPU ms per image in brackets. Every worker has its own decoder and decodes the same file
over and over; the GPU rows use 4 workers (4 streams). The last column is `nvjpeg.py`'s best batched figure
(`GPU_HYBRID`, to host). That run had about 2.8 cores of other load, from Everything indexing the encoder outputs.

| Input | turbo ×12 | parajpeg ×12 (1 thread each) | nvJPEG ×4 | nvJPEG GPU Huffman ×4 | parajpeg CUDA ×4 | jpeggpu ×4 | GPUJPEG ×4 (row markers) | nvJPEG batched |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| pin-base444-1mp | 1,826 (5.7) | 897 (10.5) | 878 (2.4) | 547 (3.1) | 191 (2.5) | **665–2,117 (0.6–1.1)** | 474 (0.8) | 1,084 (0.06), batch 128 |
| pin-base444-3mp | 677 (14.1) | 405 (26.2) | 349 (5.6) | 283 (5.3) | 125 (4.1) | **430–796 (0.9–1.0)** | 277 (0.9) | 481 (0.00), batch 128 |
| pin-prog420-1mp | **1,066 (9.5)** | 818 (11.0) | 987 (3.1) | 281 (5.9) | 209 (9.3) | — | 407 (0.7)¹ | 346 (2.8), CPU Huffman |
| pin-base420-1mp | 2,120 (5.1) | 1,077 (9.4) | 1,120 (2.3) | 573 (3.3) | 184 (3.5) | **452–2,351 (0.8–1.1)** | 266 (0.6) | 814 (0.04), batch 64 |
| phone-base420-4mp | 252 (43.1) | 187 (51.4) | 112 (32.4) | 151 (16.5) | 61 (9.0) | **273 (2.7)** | 40 (1.1) | 110 (0.29), batch 128 |
| sony-base422-7mp | 171 (62.7) | 120 (79.4) | 49 (47.8) | 124 (18.9) | 52 (9.2) | **282–499 (1.5–2.8)** | 58 (1.2) | 65 (0.34), batch 64 |
| a6000-base422-24mp | 71 (148) | 46 (220) | 20 (94.2) | 47 (39.8) | 21 (25.3) | **140 (3.9)** | 40 (1.0) | 23 (30.8), decoupled³ |

¹ The row-marker file is baseline. ³ Batches over 50 of 24 MP don't fit the 6 GB card. jpeggpu was rerun after
a harness fix (its first run stopped at the progressive file); its ranges are the spread of 2–4 repeats, which varied
up to 3× on the small files, probably with the GPU's clock state. Its 8-worker figures were no better than 4
(2,094–2,436/s on the 1 MP files, 450–547/s on the Sony).

- **jpeggpu on 4 workers out-decodes libjpeg-turbo on 12 on the camera files:** 1.1× on the phone photo, 1.6–2.9× on
  the Sony, 2.0× on the A6000, with 2.7–3.9 ms of CPU per image against 43–148 ms. On the small files it lands
  anywhere from 0.2× to 1.2× libjpeg-turbo's rate, depending on the repeat. Every other GPU path is slower than
  libjpeg-turbo on 12 workers.
- **The GPU's case is CPU time, not speed:** jpeggpu costs about 0.9 ms of CPU per image, GPUJPEG with markers
  about 1 ms, and batched `GPU_HYBRID` 0.0–0.3 ms, against 5–148 ms for libjpeg-turbo. The thumbnail job also
  runs PDQ, ThumbHash and the resizes on the CPU, and SSCD keeps the GPU busy.
- **parajpeg on one thread per image has about half libjpeg-turbo's throughput.** Use it only for single decodes
  that are waited on.

### Output against libjpeg-turbo

Mean |difference| / largest / PSNR, on the same inputs.

- **Identical:** parajpeg (CPU and CUDA) and Wuffs on every file.
- **nvJPEG with interpolation:** max 3; 69 dB on 4:4:4, 60.6–65.1 dB subsampled.
- **jpeggpu (planes against libjpeg-turbo's planes):** max 1; 63–74 dB.
- **GPUJPEG:** 57.5–63.8 dB on 4:4:4 and 4:2:2, but 29.5 dB on the phone's 4:2:0 (mean 6.3, max 57) and 46–50 dB
  on the Pinterest 4:2:0 files: no smooth chroma upsampling.
- **jpegli:** 49–52 dB, max 5–36, also on 4:4:4. Its decoder biases dequantized coefficients towards zero and
  works in floating point (`idct.cc`), a deliberate change of reconstruction, not an error.
- **debesheedas:** 51 dB (mean 0.5, max 3) on 4:4:4 without markers; 6–7 dB, unusable, with markers or 4:2:0.

### nvImageCodec and debesheedas

- **nvImageCodec 0.10.0.9** (`nvimgcodec_bench.py`; this run had 2.8–4.1 cores of other load, so read it as rough;
  0.10.0.9 is the version the numbers were measured with, and `eval/jpeg-decoders/requirements.txt` pins it):
  - **One image per call** costs about what nvJPEG does with its default backend: 3.4–7.6 ms on the Pinterest
    files, 20–53 ms on camera files. The CPU time equalled the wall time in the spinning run; with blocking sync
    (`blocking_sync()` in `nvimgcodec_bench.py`) the CPU time at batch 1 fell from 16.8 to 13.3 ms, so the spin
    explains about 3 ms of it, not all of it. These figures (`nvimgcodec_blocking.txt` and its spinning counterpart) are
    not in the repository and were not reproduced in this pass.
  - **Batches** spread the Huffman decoding over CPU threads: batch 64 runs at 1.6–3.5 ms per Pinterest file and
    5–9 ms per camera file, but costs 4–8 ms and 19–25 ms of CPU per image, which is libjpeg-turbo's own CPU cost.
    The blocking-sync run still shows CPU time about 2× the wall time at batch 64, which fits CPU threads doing the
    Huffman decoding. That is an inference from CPU time, not a trace of which backend ran. Against it,
    `GPU_HYBRID` in `nvjpeg.py` (CPU 0.02 ms) and `HYBRID` (3.2 ms) at batch 64 show what a GPU Huffman path looks
    like, and nvImageCodec's batches do not look like that; the earlier wording that it "never used" the GPU path
    was stronger than the saved runs support.
  - The **CPU backend is 2.0–3.5× slower** than libjpeg-turbo on one image (11.4 ms on `pin-base444-1mp`, 234 ms on
    the A6000). An earlier probe reported that `GPU_ONLY` returned no image on this card, but that result is
    unverified: the probe scripts (`gpu_only_probe.py`, `gpu_only_kinds.py`) saved no output and are not in the
    repository, and `backends()` in `nvimgcodec_bench.py` has no `HW_GPU_ONLY`, so the claim that `HW_GPU_ONLY` is
    unsupported on Turing is unverified too. The script now reports a backend that cannot be created, or that returns
    no image, with an explicit message.
  - So: an integration layer with nothing to gain here over calling nvJPEG directly.
- **debesheedas/GPU-JPEG-Decoder** (`debesheedas.py`) is a course project that decodes in a single warp: 1.3 s
  for 1.1 MP and 3.6 s for 2.9 MP past its 0.2 s start-up (a whole process per decode, including writing its
  text output).

## Other decoders, researched but not measured

As of 2026-10-31. First the links and claims for the decoders measured above:

- **parajpeg** ([anniigr/parajpeg](https://github.com/anniigr/parajpeg), version 0.1.0): splits one image across
  threads without restart markers, by speculative self-synchronising Huffman decoding. It claims 2.8–3.1× on an
  Apple M3 with 8 threads; the i7-8700 with 12 measured 2.8–2.9× on camera files.
- **jpeggpu** ([nolmoonen/jpeggpu](https://github.com/nolmoonen/jpeggpu)): GPU Huffman for one image without
  markers (Weißenberger and Schmidt, 2021). It claims 1.26 ms against nvJPEG's 6.89 ms on 6 MP on an RTX 2070;
  measured here, 4.3 ms against nvJPEG's 29–34 ms on 4–7.5 MP.
- **GPUJPEG** ([CESNET/GPUJPEG](https://github.com/CESNET/GPUJPEG)), **nvImageCodec**
  ([NVIDIA/nvImageCodec](https://github.com/NVIDIA/nvImageCodec)), **Wuffs**
  ([google/wuffs](https://github.com/google/wuffs)), **jpegli** ([google/jpegli](https://github.com/google/jpegli)),
  **debesheedas/GPU-JPEG-Decoder** ([GitHub](https://github.com/debesheedas/GPU-JPEG-Decoder)).

Not measured:

- **Fastvideo SDK** (commercial, needs a licence): the vendor's benchmark puts it 19–75× faster with markers than
  without, and without markers it decodes Huffman on the CPU.
- **NVDEC** (FFmpeg `mjpeg_cuvid`, measured once): the 2060's video engine decodes 4:2:0 JPEG only. It rejects
  4:4:4 and 4:2:2, and on 4:2:0 it was no faster than FFmpeg's CPU decoder, with the same CPU time.
- **nvJPEG `HARDWARE`** (A100/A30, Hopper, Blackwell) and **Intel Quick Sync** (the UHD 630 is disabled in the
  BIOS): not available on this machine.
- **Not useful here:** epeg (libjpeg's DCT scaling, which we use), jpegn (Go), Fulim13/jpeg (writes a custom
  format, can't read JFIF).
- **The Python libraries in Iglovikov, "Need for Speed: A Comprehensive Benchmark of JPEG Decoders in Python"**
  ([arXiv 2501.13131](https://arxiv.org/html/2501.13131v1), 2025) are wrappers around decoders measured above:
  OpenCV `imdecode` (its bundled libjpeg-turbo; `IMREAD_REDUCED_*` is the same DCT scaling), TensorFlow
  `decode_jpeg` (bundled libjpeg-turbo), torchvision's CPU `decode_jpeg` (libjpeg-turbo), jpeg4py (ctypes over the
  system libjpeg-turbo), Pillow-SIMD (SIMD in resize and filters, not in the decoder), imageio and scikit-image
  (both read through Pillow), and kornia-rs (`kornia-io`: zune-jpeg by default, the `turbojpeg` crate as a feature).
  The paper's gaps between them are binding overhead and disk I/O (it times cold-cache file reads). It says Pillow
  uses the original libjpeg; its wheels ship libjpeg-turbo.

## Next

- **Progressive originals → baseline in ImageReduce:** the one change measured to pay off with nothing lost
  (2.6× fewer instructions, identical pixels). Existing `*.mozjpeg.jpg` files that are still progressive need a
  one-off pass.
- **jpeggpu's missing colour stage:** a kernel for upsampling and YCbCr → RGB, checked against libjpeg-turbo's
  output, and its throughput measured again with it (its small-file runs varied up to 3×). Then it can be judged
  as a GPU path for the thumbnail job: it already beats libjpeg-turbo's 12 workers on camera files, with a few ms
  of CPU per image. Progressive files would still need the CPU (or a lossless conversion to baseline first).
- **parajpeg for the viewer's full-size decodes:** byte-identical to libjpeg-turbo, so `pixel_hash` stays; 2.8–2.9×
  lower latency on camera files with 12 threads.

## To rerun

- **CPU decoders (Rust harness)**, on Linux (natively or in the container; there is no WSL step any more): `python3
  eval/jpeg-decoders/wsl.py inputs check compare time bench marginal`. This needs `apt-get install libvips-dev`. The inputs
  are library photos, so they're copied to `~/jpeg-inputs` and never into the repo.
- **GPU through Python:** `D:\PhotoGallery\venv-eval\Scripts\python.exe eval\jpeg-decoders\gpu.py FILE...`
  (torchvision), and `... eval\jpeg-decoders\nvjpeg.py [--interpolate] [--batches 1,64] FILE...` (the two nvJPEG
  backends).
- **Restart markers and baseline conversion:** `python3 eval/jpeg-decoders/restart.py`
  (needs `apt-get install libjpeg-turbo-progs`). It keeps the variants in `~/jpeg-restart`.
- **The C++ harness:**
  - Build: `python eval\jpeg-decoders\win_build.py` on Windows (VS Build Tools, CUDA 13.4, CMake, Ninja, NASM, LLVM;
    `win_env.py` sets them up) and `python3 eval/jpeg-decoders/wsl_build.py`. Both
    clone the third-party code at pinned commits into `D:\PhotoGallery\tools\jpeg-eval` or `~/jpeg-eval` and apply
    `eval/jpeg-decoders/patches/`: for jpeggpu, C++20 and binary file reads for MSVC; for parajpeg, its GCC flags
    kept off nvcc and MSVC equivalents of two builtins.
  - Windows: `harness.exe time --secs 2 FILE...` and `harness.exe throughput --workers N DECODER FILE...`, with
    CUDA 13.4's `bin\x64` on PATH; `harness.exe list` names the decoders.
  - WSL: `python3 eval/jpeg-decoders/harness_wsl.py compare ir time throughput`.
  - nvImageCodec, in its own venv (`python -m pip install -r eval/jpeg-decoders/requirements.txt`, which pins
    nvImageCodec 0.10.0.9 and nvJPEG 13.3.2.43; an unpinned `[all]` extra replaced the shared nvJPEG DLL):
    `nvimgcodec_bench.py --ref-dir DIR
    FILE...`, with references from `harness.exe dump turbo FILE DIR\NAME.ppm`. debesheedas: `debesheedas.py --exe
    DECODER.exe --ref-dir DIR FILE...`.
