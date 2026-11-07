# Findings log

Measured results and decisions from M0 (see `PLAN.md`). Newest entries at the top of each
section. Every number states where it was measured; "subset" = the quick-test subset
(`eval/make_subset.py`: all labeled files + 3,000 random others, ~9.3k images).

Raw outputs live in `D:\PhotoGallery\eval_smoke` (subset) and `D:\PhotoGallery\eval` (full set,
not run yet). Browse them with `eval/viewer.py`.

---

## Environment

| Item | Value |
|---|---|
| CPU / RAM | i7-8700 (6C/12T, AVX2), 64 GB |
| GPU | RTX 2060, 6 GB, sm_75 |
| Library | E: 22 TB Exos HDD (21.4 TB used); C: has ~60 GB free after the 2026-10-28 cleanup (keep everything off it) |
| Fast storage | D: 2 TB NVMe |
| Eval Python | `D:\PhotoGallery\venv-eval` (torch 2.15 + CUDA 13.4 since 2026-10-30, NVIDIA driver 621.01); global Python's torch is CPU-only |
| Port 8765 | Used by a local test server outside the repo; the viewer takes the next free port (8766) |

## Test set (2026-10-28)

a copy of the Pinterest folder, copied to `D:\PhotoGallery\datasets\pinterest`.

- 189,528 files, 30 GB, average 170 KB (p50 126 KB, p99 548 KB).
- 98.9% images (187,614 `.jpg`), plus ~1.7k videos (excluded for now).
- **98.4% of files are `.mozjpeg.jpg` re-encodes**, so byte-identical matching finds few
  duplicates. The decoded-pixel hash and the perceptual tiers matter more.

### Labels

- `dups2/<NNN>/`: matched duplicate groups. After fixes: **678 groups, 1,363 files, 702 pairs**.
- `dups2/*` (loose): **4,574 files**, each with a duplicate among the other loose files, unsorted.
- `similiar_but_not_dups/`: **370 files** that other tools wrongly flagged; ~68k non-duplicate pairs.

**Label fixes (2026-10-28, applied on both D: and E:, verified identical):** 7 folders each
held two unrelated duplicate pairs. The second pair was moved to a new folder:
075→672, 076→673, 127→674, 174→675, 175→676, 266→677, 281→678.

**2026-10-29:** 090 → 679 (two pairs of different frames of one shoot; E: done, D: after the k-NN
run; `files.txt` paths patched in place so embeddings stay row-aligned). Now 679 groups and
698 labeled pairs.

**Still open:** 9 single-file folders (230, 348, 416, 417, 423, 530, 538, 540, 628). Their
partner is not among the loose `dups2` files (best match ≤ 0.40), so it may be in `Photos/`
or deleted. Finding it needs a full-set run.

## I/O

- **HDD read order (`eval/bench_hdd.py`, 2026-10-29): read in MFT order, or keep ≥ 8 reads in
  flight. Either gives ~3× path order and reaches the disk's sequential speed.** Unbuffered reads
  (no Windows cache), each order on its own block of 7,500 consecutive files of the test folder on E:
  (`--dense 7500`):

  | Order | files/s | MB/s | 70M files, I/O only |
  |---|---|---|---|
  | path (sorted walk), 1 read at a time | 557 | 64 | 1.5 days |
  | **MFT record number** | **1,699** | **228** | **0.5 days** |
  | physical cluster (LCN) | 1,657 | 234 | 0.5 days |
  | path, 8 reads in flight (drive NCQ reorders) | 1,603 | 235 | 0.5 days |
  | physical cluster, 8 in flight | 1,399 | 252 | 0.6 days |
  | random | 188 | 28 | 4.3 days |

  - Even though path, MFT and cluster order almost coincide in this folder (rank correlation
    0.996–0.999), path order one read at a time loses 2/3 of the speed. Small local reorderings
    each cost a disk rotation.
  - **Caveat: this folder was copied in one go, so it is nearly ideal** (1.2% of files fragmented).
    Older parts of the 70M library are likely more scattered, which favors MFT or cluster order
    even more. The Rust scanner should read the MFT (or USN journal), sort by record number, and
    keep a deep read queue.
  - Sparse sample (every 25th file, `--stride 25`): every order runs at ~140 files/s (random 125),
    and 8 in flight gives ~245. When files are far apart, each read pays a seek whatever the order;
    only queue depth helps.
  - At ~1,600 files/s the disk is no longer the bottleneck for a 70M pass; decoding is.
- **Read buffers (`pipeline::Options::budget`, 2026-11-01): the pass's file buffers must cover a file per
  read in flight and per worker, since a worker holds its file while it decodes it.** `gallery import`,
  8 readers, 12 workers, on the first 1,000 files (6.3 GB, 24 MP Sony A6000 JPEGs, 6 MB average) of
  a camera folder on another drive, in on-disk order, purged from the file cache before each run (an unbuffered
  open drops a file's cached pages), three runs each in rotating order, medians:

  | Buffers | MB/s |
  |---|---|
  | no pool (HEAD 2e1b6ca: a channel of 32 files, a `Vec` per file) | 255 |
  | 64 MB | 221 |
  | 128 MB | 250 |
  | 240 MB (12 MiB × (8 + 12), the default) | 268 (HEAD in the same rounds: 270) |
  | 256 MB | 268 |
  | 512 MB | 266 |

  - The disk, not the CPU, sets the pace: ~40 files/s. Below a file per slot, readers wait for workers to
    hand buffers back.
  - Without a pool, the channel bounded files, not bytes: 52 camera files in flight is 300–600 MB, and a
    folder of 100 MB panoramas held 5 GB.
  - One pass per block of files read once (no purge) was useless: the same build read 175–248 MB/s on
    different blocks of the folder, more than the budgets differ.
  - **Again with the pool that replaces a too-small idle buffer** (145ec5b; the runs above were of a pool that
    only reused a buffer already large enough, and otherwise made a new one while the budget had room), at
    1d3685b, the same files and method, 2026-11-02. Medians: no pool (2e1b6ca) 278 MB/s, 64 MB 222,
    128 MB 267, 240 MB 277, 256 MB 278, 512 MB 277. The knee is where it was, so the default stays at 240 MB. The same
    build without a pool read 278 MB/s here and 255 MB/s in the first session, so only rounds of one session
    compare.
  - That pool still made a buffer for nearly every file: only bytes bound the files in flight, so readers read
    ahead until the budget was full (HEAP_PROFILE.md "The read pool still makes a buffer per file").
  - **Again with the files in flight bounded by count too** (`Options::slots`; the pool then holds at most that
    many buffers, reused and grown in place, and the readers' channel has that many places), 2026-11-02. The
    first 400 of the same files (2.6 GB), the same method, three rounds of slots × budget, medians in MB/s:

    | Slots \ budget | 128 MB | 240 MB | 512 MB |
    |---|---:|---:|---:|
    | 20 (readers + workers) | 249 | 271 | 272 |
    | 32 (readers + 2 × workers, the default) | 261 | 273 | 273 |
    | 56 (readers + 4 × workers) | 265 | 274 | 273 |
    | no bound by count | 261 | 273 | 273 |

    The runs of a configuration were within 0.4 s of each other (9.6–10.1 s, but for 20 at 128 MB). The bound by
    count costs nothing from 32 slots on, and 240 MB is still the knee, so both defaults are those. A first try
    on the 1,000 files was dropped: its third round ran 30% slower in every configuration, from some other load
    on the machine.
- **Paths over 260 characters.** std makes a long path verbatim (`\\?\`) itself, and the scanner's `CreateFileW` is
  handed the verbatim form (`crates/platform/src/native.rs`, `\\?\UNC\` for a share). FFmpeg reads its files through
  std (`crates/video/src/ffi.rs`), so it gets the long form too. ExifTool gets the plain path, as `\\?\` isn't a path
  it reads. Probed (ExifTool 13.60, a 375-character path): from the command line, a write and a read reach such a
  file with the plain path, and with `WindowsLongPath=1` too; through the stay-open argument file, with the plain path.
  The option isn't turned on.
- **ExifTool's argument file and odd names.** Probed in WSL (Ubuntu-26.04, ExifTool 13.50), with absolute paths, as a
  rating write (`-charset filename=utf8`, `-XMP:Rating=4 -o out in`) through `-@`. The argument file carries a name
  exactly, except one with a line break (split at it) or one ending in a space, tab, vertical tab, form feed or CR
  (trimmed: "Can't create PNG files from scratch"). A leading space or tab, a space, tab or CR inside a name, `#`, `-`,
  a quote, `[CSTR]` and raw bytes all come through. The process-argument route carries a name's bytes exactly, and
  `crates/exiftool/src/lib.rs` (`needs_argv`) sends the names the argument file can't carry through it. But a name
  ending in whitespace still can't be written by either route: ExifTool takes the file's type from its extension
  (`png `), refuses it ("Can't create PNG files from scratch"), so the write fails with an error and leaves no copy.
  Its argument file lines are absolute paths: a relative name starting with `#` is a comment and one starting with `-`
  is an option. Windows can't hold any of these names; its surrogate names were probed separately (above).
  A `#[CSTR]` line (ExifTool's C-escape form, `\n`, `\r`, `\t`, `\"`, `\\`) carries the same names except those with
  `$` or `@`, which it rewrites (probed on both: WSL 13.50 and Windows 13.60). So it isn't a route for every name, and
  the code doesn't use it: a plain line carries `$` and `@` exactly, and the argument route carries the rest on Unix.
  On Windows the argument route can't carry a lone surrogate (Windows' argv is lossy), so the plain line is its route.
- **HDD read, path order (robocopy E:→D:): 70 MB/s, ~410 files/s** (consistent with the 557 files/s
  above, as robocopy also writes).
- **The harness embeds at only 80–117 img/s against 410 img/s of GPU capacity**, so decoding
  (PIL, Python DataLoader) is the bottleneck. The Rust decode path matters more than the model.

### Decode benchmark (`eval/bench_decode.py`, subset, 2026-10-29)

1,000 random JPEGs (median 0.96 MP, 120 KB), each turned into SSCD's 320×320 input, files already in
RAM, machine otherwise idle. Cosine = SSCD fp16 embedding vs the same image through `pil_full`.

| Pipeline | img/s, 1 process | img/s, 12 processes | Cosine vs reference: min / p1 / p50 |
|---|---|---|---|
| PIL full decode + bilinear (current harness, reference) | 65 | 263 | 1 / 1 / 1 |
| **PIL `draft()`: libjpeg-turbo DCT-scaled decode + bilinear** | **123** | **550** | 0.973 / 0.992 / 0.999 |
| libvips `thumbnail` (shrink-on-load + Lanczos3) | 78 | 293 | 0.920 / 0.972 / 0.993 |
| nvJPEG batched on the GPU + antialiased GPU resize | 267 | – (one process) | 0.991 / 0.995 / 0.999 |

- **DCT-scaled decode is the winner: 2.1× full decode on the CPU, nearly the same embedding.** In
  Rust: libjpeg-turbo's scaling factors (`turbojpeg` crate), the same mechanism.
- **nvJPEG** gives the most faithful fast output but only 267 img/s. The 2060 has no hardware JPEG
  engine, so Huffman decoding still runs on the CPU, and it competes with SSCD for the GPU.
- **libvips is the slowest fast option here and moves embeddings the most** (min cosine 0.92):
  Lanczos3 sharpens differently from the bilinear antialias the model was evaluated with. It may
  speed up with `VIPS_CONCURRENCY=1` per process (12 processes each starting vips threads
  oversubscribe the CPU). Not pursued, since `draft()` already wins.
- **The rule that matters: index and query images must go through the same decode + resize.** A
  0.97–0.99 cosine shift is harmless when it applies to every image, and harmful when only one side
  of a pair gets it.
- At 550 img/s, decoding 70M images takes ~1.5 days on the CPU. The disk (~1,600 files/s in MFT
  order) and SSCD fp16 (410 img/s) are now within the same range, so the pass is GPU-bound at
  ~2 days.

### Thumbnails (`eval/bench_thumbs.py`, 300 random images, 2026-10-30)

Grid thumbnails for a justified layout: height = the row height in device pixels, width from the
aspect ratio, never enlarged. Quality is SSIMULACRA2 (libjxl's tool; ~90 visually lossless, 80
very high, 70 high, 50 medium). Times are one thread, files in RAM, machine in use.

**Resize** (from the original file to the exact thumbnail size; score vs a float Lanczos3 resize of the
full decode, 384 px; zone plate = worst-case aliasing test):

| Pipeline | Score mean / p10 | ms | Zone plate |
|---|---|---|---|
| Full decode + Lanczos3 | 96.6 / 95.9 | 19.0 | 94.6 |
| **DCT-scaled decode keeping ≥ 4× the target + Lanczos3** | **96.6 / 95.9** | 18.5 | 73.9 |
| DCT-scaled decode keeping ≥ 2× + Lanczos3 | 95.7 / 94.7 | 16.4 | 58.8 |
| libvips `thumbnail` (shrink-on-load + Lanczos3) | 95.5 / 94.4 | 13.2 | 58.7 |
| DCT-scaled decode keeping ≥ 1× + Lanczos3 | 87.5 / 75.5 | 7.7 | 36.7 |
| Full decode + antialiased bilinear | 86.0 / 82.3 | 13.0 | 43.9 |

- **Lanczos3 from at least 4× the target is the thumbnail path**: the full-decode quality, and on large
  originals (camera photos) still the 1/2–1/4 DCT-scaled decode. On this sample (median 800×1200)
  there is little to scale, hence the same time. libvips (= 2× margin) is softer on some images
  (p10 at 256 px: 87.7 vs 96.5).
- The embedder's input (DCT-scaled to just ≥ 320 px, "Decode benchmark") is fine for SSCD but not
  for thumbnails (p10 75). Bilinear is too soft. The ingest decodes once and makes both outputs.

**Encode** at the same quality, 384 px (mean bytes per thumbnail; × 70M):

| Format | Score 70 | Score 80 | Score 85 | Encode ms | Decode ms |
|---|---|---|---|---|---|
| JPEG (libjpeg-turbo, 4:2:0) | 13.8 KB (964 GB) | 21.9 KB (1.53 TB) | 29.6 KB (2.07 TB) | 0.9 | 0.6 |
| JPEG (jpegli, 4:2:0, sequential) | 12.2 KB (853 GB) | 19.9 KB (1.39 TB) | – | 1.4 | 0.7 |
| JPEG (jpegli defaults: 4:4:4, progressive) | 12.8 KB | 19.7 KB | 26.4 KB | 3.4 | 1.7 |
| JPEG (mozjpeg) | 12.2 KB | 19.9 KB | 28.5 KB | 12.7 | 0.8 |
| WebP (method 4) | 10.7 KB (747 GB) | 16.9 KB (1.19 TB) | 23.4 KB (1.64 TB) | 20 | 2.7 |
| JPEG XL (effort 7) | 10.8 KB | 16.4 KB | 21.7 KB | 75 | 5.0 |
| AVIF (speed 6) | 8.2 KB (571 GB) | 13.6 KB (949 GB) | 19.0 KB (1.33 TB) | 93 | 3.3 |

Other row heights at score 80: 256 px: JPEG 815 GB, WebP 611 GB, AVIF 512 GB; 512 px: JPEG 2.37 TB,
WebP 1.88 TB, AVIF 1.47 TB.

- **Disk is the binding constraint.** D: has ~690 GB free. PLAN's ~350 GB (5 KB at ~200 px) only holds
  around 256 px at score 60–70. At 384 px only AVIF at about score 70 fits.
- **AVIF is the smallest at every quality** (score 80: −38% vs JPEG, −20% vs WebP), but at speed 6 it
  encodes in ~0.1 s per 384 px thumbnail (~7 days of all 12 threads for 70M) and decodes 5× slower than
  JPEG (3 ms; a screen of 60 thumbnails is ~15 ms on 12 threads, fine).
- **AVIF encoder speed** (100 of the images, 384 px; bytes at score 70 / 80, encode ms):

  | AVIF speed | Score 70 | Score 80 | Encode ms | 70M on 12 threads |
  |---|---|---|---|---|
  | 6 (default) | ~7.6 KB | 12.9 KB | 110 | ~7.4 days |
  | **8** | 8.5 KB | 13.7 KB | 40 | ~2.7 days |
  | 9 (10 is identical) | 9.8 KB | 15.7 KB | 9 | ~0.6 days |
  | WebP method 4, for reference | 10.2 KB | 16.2 KB | 21 | ~1.4 days |

  Speed 8 keeps most of AVIF's advantage (−15% vs WebP) at 2× WebP's encode time; speed 9 is
  faster than WebP and still a little smaller.
- JPEG XL loses to AVIF on size and to WebP on speed. Progressive JPEG is ~2× slower to decode for
  0–3% fewer bytes. jpegli 4:2:0 sequential is the best JPEG (−9%, decodes like any JPEG); mozjpeg
  matches its size at 10× the encode time.

**Thumbnails as a source for later models** (SSCD fp16 on the thumbnail vs on the original, cosine):

| Height | Uncompressed thumbnail: min / p1 / p50 | As JPEG q85 (score ~82): min / p1 / p50 |
|---|---|---|
| 256 px | 0.730 / 0.872 / 0.984 | 0.661 / 0.790 / 0.931 |
| 384 px | 0.844 / 0.981 / 0.998 | 0.800 / 0.897 / 0.978 |
| 512 px | 0.912 / 0.997 / 0.999 | 0.889 / 0.948 / 0.992 |

- Compression moves the embedding more than the size does, and tall images (a 256 px-high
  infographic is ~120 px wide) lose the most. A re-analysis source would need ≥ 512 px at near-lossless
  quality, i.e. several TB. **So thumbnails are sized for display only** (PLAN §9 decision 1), and any
  model wanted later should run in the ingest pass (decision 2), which reads the originals anyway.

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

## Nearest-neighbour search (`eval/bench_ann.py`, 5M synthetic vectors, 2026-10-31)

No full-library embeddings exist yet, so the index bake-off (PLAN M0) ran on 5M synthetic vectors: a Gaussian with
the subset's SSCD mean and covariance, L2-normalized (random-pair cosine p50/p99 0.076/0.236 vs real 0.071/0.265),
plus 10,000 queries, each with one planted partner in the index at a cosine uniform in 0.3–0.99. The measure that
matters is **partners found**: the share of partners with cosine ≥ 0.4 that come back in the top 10 (the k-NN
candidate rule). Recall@10 against exact search counts unrelated background neighbours, which here are near-ties
(the Gaussian has no cluster structure), so it means little.

| Search | Partners found (≥ 0.4) | 0.4–0.5 | 0.8–1.0 | Queries/s | Build | RAM (5M) |
|---|---|---|---|---|---|---|
| **Exact, GPU** | **0.997** | 0.985 | 1.000 | (below) | none | base on disk |
| USearch HNSW int8 (M 16, ef_add 128), ef 64 | 0.131 | 0.024 | 0.245 | 4,223 | 3,788 s (1,320/s) | 5.05 GB |
| ... ef 256 | 0.260 | 0.059 | 0.473 | 1,301 | | |
| FAISS IVF8192,PQ64, nprobe 64 | 0.579 | 0.184 | 0.924 | 840 | 484 s | 0.35 GB |
| ... nprobe 256 | 0.762 | 0.398 | 0.987 | 240 | | |
| ... nprobe 256 + exact rerank of 100 | 0.767 | 0.427 | 0.987 | 222 | | |
| FAISS IVF8192,SQ8, nprobe 256 | 0.769 | 0.441 | 0.987 | 21 | 238 s | 2.44 GB |

- **Exact search on the GPU is fast enough for 70M all-pairs.** An fp16 GEMM (fp32 accumulation) plus top-10
  runs at **10.4 G pairs/s** on the 2060, so 70M × 70M is **~131 GPU hours (~5.5 days)**, in the range already
  accepted for the embedding passes. It reaches the ceiling the planted pairs allow (0.997; even exact search
  misses 32% of partners at 0.3–0.4, where unrelated images score higher). New images later cost little: 10,000
  new against 70M is ~70 s.
- **The approximate indexes lose many partners on this data.** HNSW finds at most 26%, even only 47% of partners
  at cosine 0.8–1.0. IVF finds 99% of those at nprobe 256 but only 40–44% at 0.4–0.5, the heavy crops. USearch's
  int8 storage is not the cause: it moves cosines by 0.003 on average (p99 0.009).
- **The data is a worst case for them.** A Gaussian has no clusters: a query's background neighbours are all at
  ~0.29, so a graph walk gets no gradient toward the partner. Real embeddings cluster (the subset's nearest-
  neighbour cosine p50 is 0.89, with duplicates), so real recall will be higher, by an amount only the full set's
  embeddings can show. Exact search doesn't depend on it.
- **USearch at 70M** would need ~71 GB of RAM in int8 and a build of 15+ hours, slowing as the graph grows.
- FAISS warned that 300,000 training points are slightly fewer than it wants for 8,192 lists (319,488); minor.

## Bugs found

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
- **R@k metric undercounted groups of 3+:** another member of the same group ranking ahead of
  the scored partner counted as a miss (R@1 read 0.935 for every model). Fixed so only
  images outside the group count as ranked ahead.
- **Viewer port collision:** Python's `HTTPServer` sets `SO_REUSEADDR`, which on Windows lets
  it bind a port another server already uses and lose requests silently. The viewer now
  binds with `SO_EXCLUSIVEADDRUSE` and falls back to the next free port.

## Quality: true negatives and new candidates (subset, 2026-10-29)

`evaluate.py` now also counts **cross-group pairs** (915,279 pairs of images from two different
`dups2` groups, i.e. different pictures) above each threshold, the false-positive measure that
replaces `similiar_but_not_dups` (mostly matches now, see Decisions).

| Model | R@1 / R@10 | Thr @95% | Loose dups2 with partner | Cross-group flagged @90% | @95% |
|---|---|---|---|---|---|
| sscd (fp32) | 0.964 / 1.000 | 0.879 | 92.4% | 0 | 0 |
| **sscd_fp16** | 0.964 / 1.000 | 0.879 | 92.4% | **0** | **0** |
| sscd_288_fp16 | 0.964 / 1.000 | 0.874 | 92.5% | 0 | 0 |
| sscd_large | 0.962 / 1.000 | 0.859 | 91.3% | 0 | 2 |
| dinov2_s | 0.962 / 1.000 | 0.945 | 92.8% | 4 | 4 |
| dinov2_b | 0.963 / 1.000 | 0.943 | 93.1% | 0 | 4 |
| clip_b32 | 0.959 / 1.000 | 0.929 | 89.5% | 7 | 17 |
| pdq | 0.924 / 0.957 | 0.234 (d ≤ 98) | 97.9% | 1 | 345 |

- **SSCD flags no different-picture pair at any operating point up to 95% recall**; DINOv2 4,
  CLIP 17. The cross-group set is easy at threshold level (most of it is far apart); the hard
  cases are the few near pairs that k-NN hands the verifier (see the verifier sections).
- **SSCD-large (ResNeXt101, 1024-d): no gain.** Same R@10 = 1.000, slightly fewer loose files
  paired (91.3% vs 92.4%), 2 cross-group false positives at 95%, and 58 img/s end to end here
  (GPU-only 105 img/s, vs 410 for SSCD fp16). Dropped.

### PDQ (`eval/pdq.py`, Meta's 256-bit perceptual hash)

| Hamming distance ≤ | 0 | 8 | 16 | 22 | **31** (Meta's default) | 50 |
|---|---|---|---|---|---|---|
| Labeled pairs found | 32.6% | 65.7% | 74.8% | 80.1% | **82.8%** | 86.8% |

- **Precise but narrow.** At d ≤ 22 (80% recall) it flags 0 cross-group pairs, and at d ≤ 78 only 1.
  But it can't reach 95% recall (that needs d ≤ 98, which flags 345 cross-group pairs and 18.8k
  unlabeled pairs).
- **Breaks on the Pinterest edits:** crop 80% → R@10 0.33, 60% → 0.02, 10% letterbox → 0.11
  (table above). Mirrors and 90° rotations are found only with the 8-variant dihedral query.
- Hashing: 100 img/s with 6 processes in Python (decode-bound; the hash itself is cheap).
- **Role:** a cheap exact/near-exact tier (re-encodes, resizes, grading). ~80% of the labeled pairs
  are this kind of copy, so PDQ (with dihedral queries) could settle those before the embedding
  stage. It can't replace SSCD, and isn't needed for SSCD to work: an optional fast path.

## Quality: model comparison (subset, after label fixes, 2026-10-28)

All six models at the same recall on labeled pairs (higher "loose with partner" is better;
lower "flagged" and "hard neg" are better):

| Model | Thr @95% | Loose dups2 with partner | Flagged unlabeled | Hard neg flagged (of 68,265) |
|---|---|---|---|---|
| sscd (fp32) | 0.879 | 92.4% | 2,653 | 41 |
| sscd_fp16 | 0.879 | 92.4% | 2,653 | 42 |
| sscd_288_fp16 | 0.874 | 92.5% | 2,657 | 41 |
| dinov2_s | 0.945 | 92.8% | 2,742 | 78 |
| dinov2_b | 0.943 | 93.1% | 2,806 | 74 |
| clip_b32 | 0.929 | 89.5% | 2,861 | 78 |

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

## Quality: SSCD fp16 detail (subset, after label fixes)

- **Labeled pairs:** R@1 **0.964**, R@10 **1.000**. Every labeled partner is in the top 10.
- **Operating points:**

| Target recall | Threshold | Loose dups2 with partner | Flagged unlabeled pairs | Hard negatives flagged |
|---|---|---|---|---|
| 0.90 | 0.909 | 88.3% | 2,496 | 26 / 68,265 (0.04%) |
| 0.95 | 0.879 | 92.4% | 2,653 | 42 / 68,265 (0.06%) |

Flagged unlabeled pairs include real but unlabeled duplicates. Precision is unknown until
`review_*.csv` is labeled.

- **Score separation (pre-fix run, same subset):** hard negatives p99 ≈ 0.30; positives
  p10 ≈ 0.89.
- **What it misses (viewer, scores 0.48–0.78):** strong color grading (e.g. yellow vs
  orange shirt), slight re-crops/zooms of the same photo, AI-image variants with small
  differences.

### Why some duplicates score low and some non-duplicates score high (2026-10-28)

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
`PLAN.md` / the verifier decision below).

### Verifier prototype, first run (subset, 2026-10-28)

`eval/verify.py`:

1. DISK + LightGlue matching, also against the mirrored image.
2. MAGSAC homography.
3. Warp onto a 512 px canvas.
4. Robust quadratic color fit.
5. Analyze the residual: a changed region ≥ 0.3% of the overlap, or a median residual > 0.03, means
   variant.

Ran on all 702 labeled duplicate pairs + the 345 `similiar_but_not_dups` pairs with SSCD ≥ 0.4,
at 1.3 pairs/s:

| Label | → duplicate | → variant | → unrelated |
|---|---|---|---|
| duplicate (702) | 666 (94.9%) | 36 | 0 |
| not duplicate (345) | 40 | 305 | 0 |

- ~~Every non-duplicate above 0.4 aligns geometrically (0 unrelated)~~: wrong, the "≥ 20 inliers"
  alignment test also passes unrelated pictures (see "match vs unrelated" below). With the
  corrected rule, 253 of these 345 match (variants) and 92 don't.
- **Not yet better than SSCD alone** at the same recall (SSCD at 95% recall: 42 non-duplicates
  through). The errors fall into three groups:
  - **Small face edits pass as duplicates** (e.g. eyes opened/closed, smile changed, AI face
    retouch). The changed regions are ~0.01–0.05% of the image, smaller than the spurious
    regions on true duplicates. Re-running at full resolution doesn't fix it: the change is
    found, but a size threshold can't separate it.
  - **Overlays make duplicates look like variants:** a TikTok watermark added to one copy.
  - **dups2/090 are probably different frames of one shoot** (the aligned residual shows a
    doubled silhouette), not one photo re-edited: a labeling question. (Confirmed and
    split 2026-10-29.)
- (Superseded the same day: duplicate and variant are now one "match" class; under that rule
  all 1,047 pairs are matches and 0 unrelated.) Deferred next step: replace the hand-set
  thresholds with a classifier trained on these labels (per-region
  features: size, peak residual, shape, distance to the border, text-likeness, inside a face?),
  plus a face-focused high-resolution comparison, since face edits are the dominant variant type.

### Verifier: match vs unrelated, with a negative set (subset, 2026-10-28)

**Negative set:** pairs from two different `dups2/<NNN>` groups. Only 250 of ~915k such pairs
reach SSCD 0.4 and 4 reach 0.8. Used every pair ≥ 0.3: 4,628 pairs, 57 min.

- **Inlier count alone fails badly.** With "≥ 20 inliers = match", **2,720 of 4,628 cross-group
  pairs matched (59%)**. DISK + LightGlue + MAGSAC finds a homography with 20–120 inliers between
  unrelated pictures (similar poses, backgrounds, AI style).
- **The residual after alignment separates them:**

| Measure (p1 / p10 / p50 / p90 / p99) | Labeled duplicates | Similar folder | Cross-group |
|---|---|---|---|
| Inliers | 424 / 1151 / 1455 / 1735 / 2047 | 32 / 61 / 224 / 1172 / 1504 | 0 / 0 / 25 / 60 / 118 |
| Inlier ratio | .66 / .89 / .96 / .99 / 1.0 | .15 / .23 / .45 / .92 / .98 | 0 / 0 / .22 / .33 / .50 |
| Median residual (aligned only) | .001 / .003 / .006 / .009 / .025 | .005 / .010 / .046 / .111 / .263 | .092 / .187 / .255 / .343 / .427 |

- **New rule:** match if median residual ≤ 0.06, **or** ≥ 150 inliers making up ≥ 50% of the
  matches (the second clause catches extreme grades the color model can't undo, e.g. dups2/090).
  ~~e.g. dups2/090~~ **Correction 2026-10-29:** dups2/090 was not a color grade but two pairs of
  different frames of one shoot (residual 0.13, see "Same-shoot frames" below). The second clause
  existed only for it and is now off by default.
  - Labeled duplicates: **702 / 702 matched**.
  - Cross-group: **5 / 4,628 matched**. Four of them are dups2/670 vs 671: two frames of the same
    beach shoot (identical background, different pose). The fifth (125 vs 673) is the same
    person, a different photo, on a plain background.
  - Similar folder: 253 matched, 92 unrelated. The unrelated ones are mostly AI regenerations of
    one composition with a different face or hair color, plus different photos of the same person;
    a few are close regenerations that could count as variants.
- ~~Open labeling question~~ resolved 2026-10-29, see below.
- `verify.py --rescore` re-applies thresholds to stored measurements in seconds (no image work).

### Same-shoot frames are different pictures (2026-10-29)

**Decision (2026-10-29):** different frames of one shoot (same place and outfit, different pose or moment)
are *different pictures*, not matches. dups2/090 held two such pairs and was split (090 + 679).

- **The strong-inlier clause is dropped** (`--strong-inliers 0` by default). With 090 split, no
  labeled duplicate needs it (all have median residual ≤ 0.051); it matched only 090's cross-frame
  pairs (residual 0.13), 21 similar-folder pairs (residual 0.06–0.13) and 125 vs 673.
- **Still matched: 670 vs 671**, same beach, different pose. The median residual is low (0.019)
  because the background dominates. The changed area does not separate it by itself: 9% changed vs
  up to 14% on real duplicates. But **where** differs:
  - On duplicates, large changed areas sit **at the frame edges**: phone status bars, UI icons,
    frame lines (dups2/426 with 14% changed is a screenshot with a status bar).
  - On 670/671 the changes are **inside**: the body, the hat strings, palm leaves in the wind.
- `verify.py` now measures `interior_changed_frac` (changed share of the overlap, minus an 8% edge
  band) and `interior_regions` (separate changed regions inside). Diagnostic until the reruns show
  whether a threshold separates same-shoot frames from local-edit variants (a face retouch also
  changes the interior, but in one compact region).

**Reruns after the split (subset, 2026-10-29), strong clause off:**

| Set | Match | Unrelated |
|---|---|---|
| Labeled duplicates | **698 / 698** | 0 |
| Cross-group (4,632 pairs ≥ 0.3) | 5 (670/671 ×4, 125/673) | 4,627 |
| Similar folder | 231 | 114 |
| k-NN candidates: duplicate / cross-group / similar / unlabeled | 698 / 5 / 224 / **2,973** | 0 / 200 / 98 / **4,146** |

(k-NN = each image's 10 nearest SSCD neighbors ≥ 0.4: 8,344 pairs, 80 min. Before the split, with
the strong clause: unlabeled 2,985 / 4,134.)

**Interior changes don't separate same-shoot frames on their own:**

- 670/671 have 25–32 changed regions inside; labeled duplicates at most 13. "≥ 15 regions or > 15%
  interior change" flips 0 duplicates, 52 similar-folder and 58 unlabeled k-NN matches to unrelated.
- Looking at 20 of the 58 flipped k-NN pairs: **~11 correct** (front vs back of a cosplay, other
  poses, other outfit in the same room: matched only because the background dominates the median
  residual), **~5 wrong** (heavily compressed copies whose blocking makes many small regions; a
  FaceApp beautify, a hair recolor, a face swap: local edits, which count as matches), ~4 ambiguous.
- So **the median-residual rule also matches different poses** in front of the same background,
  beyond 670/671. That's a real precision problem on candidates, not just a labeling detail.
- **Better signal: motion.** A different moment *moves* things (limbs, hair, cloth: coherent
  displacement after alignment); an edit or recompression changes appearance *in place* (no
  displacement). `verify.py` now runs dense optical flow (OpenCV DIS, both directions) between the
  aligned, color-mapped images. A changed pixel counts as **moved** if the flow there is > 2 px,
  forward-backward consistent (< 1 px) and warping by it at least halves the residual. Output:
  `moved_frac` (share of the overlap), `moved_share` (share of the change that motion explains),
  green outlines in the diagnostic.

  First test, 63 pairs with a by-eye truth:

  | Pairs | moved_frac |
  |---|---|
  | 30 random labeled duplicates | **0.000** (all) |
  | 8 duplicates with large edge changes (status bars, frames) | 0.000–0.001, except 448 (0.075) |
  | Recompressed copies, recolor, beautify, face swap (same picture) | 0.000–0.008 |
  | 670/671 (same beach, other pose) | **0.027–0.035** |
  | Other frames of a shoot (2543, 6917, 7007, 224) | 0.017–0.196 |
  | Different outfit/person/pose in the same place (6210, 6317, 6489, 7388, 7640, 5426) | 0.000–0.012 |

  - **Motion catches small pose changes** (670/671 and similar), which nothing else did, and is
    zero on every plain duplicate.
  - **It misses big changes**: when the whole subject differs, flow can't explain it, so nothing
    "moves". Those have a large interior change instead (0.17–0.30), so the two measures complement
    each other: roughly "unrelated if moved > ~1.2% or interior change > ~15%" gets 16 of 17
    different-picture pairs here.
  - **Gray zone:** 448 is a body-reshape (liquify) edit, a real local geometric change, so it moves
    like a pose change; a hair recolor (5152) changes 18% of the interior. Whether such edits are
    "the same picture" is a definition question, and 63 pairs are too few to set thresholds.
- **Rerun of all three sets with motion (subset, 2026-10-29; decisions unchanged, rules off).** Among
  pairs the verifier matches:

  | Set / label | Matched | moved > 0 | moved p99 / max | interior change p99 / max |
  |---|---|---|---|---|
  | Labeled duplicates | 698 | 1.3% | 0.000 / 0.074 (448) | 0.041 / 0.132 |
  | Cross-group (different pictures) | 5 | 100% | all 0.027–0.037 | 0.196 / 0.200 |
  | Similar folder | 231 | 81% | 0.061 / 0.129 | 0.208 / 0.393 |
  | k-NN unlabeled | 2,973 | 6.8% | 0.013 / 0.195 | 0.135 / 0.363 |

  Candidate rules, pairs flipped from match to unrelated (k-NN set; cross-group = known errors,
  duplicate = known losses, the rest unknown until marked):

  | Rule | Cross-group (of 5) | Duplicates (of 698) | Similar folder | Unlabeled |
  |---|---|---|---|---|
  | moved > 0.012 only | **5** | 1 (448) | 34 | 37 |
  | interior > 0.15 only | 1 | 0 | 9 | 21 |
  | **moved > 0.012 or interior > 0.15** | **5** | 1 (448) | 39 | 56 |
  | moved > 0.005 or interior > 0.15 | 5 | 1 | 78 | 109 |
  | moved > 0.02 or interior > 0.20 | 4 | 1 | 24 | 32 |

  - **Motion alone removes every known false match** (all 5, including 125 vs 673, a different photo
    of the same person) at the cost of one labeled duplicate, 448, the liquify edit.
  - Plain duplicates almost never move (98.7% exactly 0). The similar folder moves a lot (81%), as it
    holds AI regenerations and other shots, whose truth is unknown.
  - Whether the 56 unlabeled pairs the combined rule flips are right is the open question: the
    hand marks answer it.
- **Next: hand marks.** The viewer's verifier tabs now have "Same picture (S)" / "Different picture
  (D)" marks per pair (`pair_marks.json` in the run folder, keyed by the image pair, kept across
  reruns), filters for unmarked / disagreements, and the Summary shows how often the verifier agrees
  with the marks. `--max-moved` and `--max-interior-change` (off by default) are then tuned on the
  marks with `--rescore`.

### Verifier on the synthetic edits (`eval/verify_synth.py`, subset, 2026-10-29)

The same 300 images and edits as the synthetic retrieval test below; each original verified against each edited
copy (all should match). 3,900 pairs, 39 min.

| Edit | Matched | Why the rest failed |
|---|---|---|
| JPEG q30, color shift, text overlay, letterbox 10%, mirror | 300 / 300 each | |
| Crop 80% / 60% / 40%, crop 50% + mirror | 300 / 300 each | |
| Crop 25% | 297 | 3 never aligned (0 inliers) |
| **Downscale to 25%** | **260** | **40 never aligned:** the copy is tiny (originals p50 862 px → ~215 px); every original ≥ 923 px long side worked |
| **Grayscale** | **210** | **90 aligned perfectly (p50 1,720 inliers) but noise 0.078 > 0.06:** the color model maps the copy onto the original, and gray can't be mapped back to color |
| **Rotate 90°** | **28** | No rotation attempts (only mirror); the 28 are near-symmetric images |

- **Crops, overlays and letterboxing are solved:** the stage SSCD's threshold failed on (a 25% crop scores ~0.54) the
  verifier accepts 99–100% of the time, so k-NN rank + verifier works as designed.
- **Fixes needed** (the first two done, see "Verifier fixes and speed"):
  - **Color fit in both directions** (copy → original and original → copy, keep the better). Also explains the
    real k-NN miss 5172, a heavy grade (white wall → yellow) at PDQ distance 12.
  - **Scale normalization for very different sizes:** match a tiny copy against the original at a comparable
    size, not 1024 px vs ~215 px. Also explains the real miss 6041 (236 px copy of a 1340 px original).
  - **Rotations:** better left to PDQ's 8-orientation query (exact rotated copies, 99.7% within d ≤ 31) than to
    4× the local-feature work; revisit if the full set shows rotated *edited* copies.
- **The candidate motion rule rarely hurts real copies:** 19 of ~3,500 matched edited copies would flip (6
  downscale and 6 rotation by motion, 4 grayscale and 2 text overlays by interior change, 1 crop).

### Which candidate pairs need the verifier (k-NN set, subset, 2026-10-29)

8,344 k-NN pairs over 9,307 images = **0.9 pairs per image**; 28% of images have none (p90 4, max 48).

| PDQ distance | Pairs | Verifier match | | SSCD cosine | Pairs | Verifier match |
|---|---|---|---|---|---|---|
| ≤ 8 | 27.7% | 100% | | 0.40–0.50 | **48.5%** | **1.2%** |
| 9–22 | 4.7% | 99.5% | | 0.50–0.70 | 6.9% | 26% |
| 23–31 | 1.7% | 98.6% | | 0.70–0.88 | 4.9% | 96% |
| 32–60 | 3.3% | 95.3% | | 0.88–0.95 | 7.1% | 99.8% |
| > 60 | 62.5% | 15.2% | | ≥ 0.95 | 32.7% | 100% |

- **Shortcut: accept PDQ ≤ 22 or SSCD ≥ 0.95 without verifying.** 35% of pairs. The verifier (with the motion
  rule) disagreed on 5 of 2,896, and by eye **the shortcut was right every time**: a heavy grade and a 236 px copy
  the verifier can't handle yet, a FaceApp edit and two AI variants the motion rule flagged. After the shortcut,
  only 31% of images need local features at all.
- **The low band is the cost sink:** SSCD 0.40–0.50 is half the pairs but 1.2% matches; it can't be dropped
  (heavy crops live there). Rejecting on inlier count before the dense stage doesn't help: < 40 inliers removes
  28% of pairs but loses verifier matches and edited copies, and pairs under 20 already skip it.
- **Cost (`profile_verify`, 200 pairs, RTX 2060):**

  | Stage | Cost |
  |---|---|
  | DISK features, image + mirrored | 2 × 109 ms **per image** |
  | LightGlue, as-is + mirrored | 2 × 42 ms per pair |
  | Robust color fit | 81 ms per aligned pair |
  | Misalignment-tolerant residual (25 shifted copies, NumPy) | 102 ms per aligned pair |
  | Optical flow (motion) | 36 ms per aligned pair |
  | MAGSAC | 2 × 3 ms |

  ≈ 240 ms per image + 330 ms per pair. Biggest levers: extract mirrored features only when the upright match
  fails (most pairs aren't mirrored), compute the residual and color fit on the GPU (both are simple array math),
  run the CPU part in parallel workers, and try faster local features (SuperPoint / ALIKED + LightGlue, lower
  resolution) against this same synthetic + labeled + cross-group test.
- Rough scale at the subset's rate (0.9 pairs per image, likely an overestimate, as the subset is enriched with
  labeled duplicates): 63M pairs, 41M after the shortcut. At today's speed that is over a year; with the levers
  above, on the order of weeks. The full set will give the real candidate rate.

### Verifier fixes and speed (subset, 2026-10-29)

Changes in `eval/verify.py`, all four pair sets rerun:

- **Tiny copies:** images under 512 px are upscaled to 512 px before feature extraction. On the 40
  failed 25% downscales: 0/40 → 40/40 (upscaling to the original's working side and downscaling the
  original also gave 40/40, but each cost one 25% crop).
- **Color fit in both directions:** the other image is mapped onto the reference as before; the reverse
  (reference onto the other) is used only when it fits **at least 2× better** (`REVERSE_COLOR_GAIN`).
  Simply keeping the better direction doubled the false matches (cross-group 5 → 10, k-NN cross-group
  5 → 8, all at noise 0.051–0.059): the extra freedom flattens the residual of unrelated pictures too.
  Measured fit ratios: those false matches ≤ 1.56×; grayscale copies ≥ 5.1×; real heavy grades 6–8×; a
  washed-out real copy 2.7×.
- **Mirror only when needed:** the mirrored image is aligned only if the as-is alignment has < 200 inliers
  (45% of k-NN pairs skip it), and retried when a skipped pair then doesn't match: a mirrored copy of a
  near-symmetric picture can align as-is with ~280 inliers and fail.
- **Speed:** color fit by weighted normal equations, the tolerant residual on the GPU (240 → 8 ms), and
  the comparison (color fit, residual, flow) on 4 CPU threads while the main thread extracts and matches
  the next pair. Per image 239 → 127 ms, per pair 334 → 265 ms serial. More workers don't help: the GPU
  main thread is the limit (DISK 61% of it, LightGlue 29%).

| Set | Baseline | Now (DISK) | ALIKED |
|---|---|---|---|
| Synthetic: downscale 25% | 260 / 300 | **300** | 300 |
| Synthetic: grayscale | 210 / 300 | **300** | 300 |
| Synthetic: crop 25% | 297 / 300 | 297 | 300 |
| Synthetic: rotate 90° | 28 / 300 | 28 | **196** (9 of DISK's 28 lost) |
| Synthetic: other 9 edits | 300 each | 300 each | 300 each |
| Labeled duplicates | 698 / 698 | 698 | 698 |
| Cross-group false matches | 5 / 4,632 | **5** | **9** |
| k-NN: cross-group false matches | 5 / 205 | 5 | 7 |
| k-NN: unlabeled matches | 2,973 / 7,119 | 2,979 (+6, none lost) | 3,005 (+32, −6 vs DISK) |
| Throughput (pairs/s; synth / labeled / cross / k-NN) | 1.7 / – / – / 1.5 | 4.9 / 3.4 / 5.0 / 3.7 | 6.5 / 5.4 / 3.0* / 4.9 |

\* the GPU was likely shared during that run; ALIKED extraction alone is 42 ms vs DISK's 116 ms per image
(LightGlue ~30 ms per pair for both; DISK under fp16 autocast is 6× slower, ALIKED fp16 only 8% faster).

- **Both real k-NN misses are fixed:** 5172 (heavy grade) and 6041 (236 px copy) now match, so the shortcut
  disagreements above were the verifier's limits, not the shortcut's.
- **`similiar_but_not_dups`:** +3 matches with DISK, two of them only by noise moving 0.0603 → 0.0599 at the
  0.06 threshold.
- **DISK stays the default.** ALIKED is faster and handles 90° rotations, but finds fewer inliers, so
  borderline pairs move across the noise threshold in both directions and false matches rise (5 → 9).
  Rotation is cheaper to get from PDQ's 8-orientation query or by trying the rotations only when the first
  alignment is weak (like the mirror). `--features aliked` stays available.

### Verifier: TensorRT, rotations, SIFT, resolution (subset, 2026-10-30)

**DISK on TensorRT.** Only DISK's U-Net runs as a TensorRT 11 fp16 engine (`accel.DiskUnetTRT`, dynamic sides
up to 1024 px); kornia's keypoint selection and LightGlue stay in PyTorch. Per image 35 ms vs 113 ms, 98% the same
keypoints, descriptor cosine 0.9999 (a TensorRT fp32 engine gives no speedup, 108 ms). `--disk-runtime trt` is now
the default. All four sets rerun:

| Set | torch DISK | TensorRT DISK |
|---|---|---|
| Labeled duplicates | 698 / 698 | 698 / 698 |
| Synthetic (13 edits × 300) | 3,625 match | 3,626 (3 rot90 pairs flip, ~20 inliers) |
| Cross-group false matches | 5 / 4,632 | 7 |
| k-NN: decisions changed | – | 30 / 8,344 (20 → match, 10 → unrelated) |
| k-NN: cross-group false matches | 5 / 205 | 7 |

- **The flips show a fragile measurement, not TensorRT drift.** Rerunning the two new cross-group matches (group
  125 vs 673, one shoot) in torch reproduces torch's numbers exactly, so the difference comes from the features.
  For some pairs the median residual swings far more than the threshold margin when a few keypoints change:
  k-NN 978 (similar-folder) 0.272 → 0.033 and 4157 0.014 → 0.076, at equal inlier counts. About 0.3% of pairs
  sit there. That belongs to the decision rule (hand marks), not to the runtime.

**Throughput** (labeled set, 1,043 pairs, quiet machine; the four-set runs above overlapped the M1 session's CPU
load and read 4.3–4.7 pairs/s):

| Configuration | Time | Pairs/s | Decisions |
|---|---|---|---|
| torch DISK (before) | 303 s | 3.4 | – |
| torch DISK, images decoded ahead (`--loaders 4`) | 266 s | 3.9 | identical |
| TensorRT DISK | 177 s | 5.9 | 9 similar-folder pairs flip (net +6 matches) |
| TensorRT DISK, decoded ahead | 175–179 s | 5.9 | identical to TensorRT |
| ... and 8 comparison workers (new default) | 155 s | **6.7** | identical |
| ... at 768 px, 2,048 keypoints (4 workers) | 163 s | 6.4 | duplicates 698; similar-folder 232 (237) |
| ... at 768 px, 1,024 keypoints (4 workers) | 141 s | 7.4 | duplicates 698; similar-folder 226 (237) |

- **Where the time goes now** (py-spy, TensorRT + decoding ahead, 175 s): the main thread is busy 133 s,
  LightGlue 68 s and DISK incl. keypoint selection 51 s. The comparison workers use 275 thread-s, decoding 41.
  Decoding ahead only helped the slower torch path. **LightGlue is the next lever**: on TensorRT (LightGlue-ONNX
  exists), or fewer keypoints.
- **Lower resolution** keeps all labeled duplicates and every synthetic edit within 1–3 images at 768 px (crop 25%:
  298 at 768/2,048 and 295 at 768/1,024 vs 297), but similar-folder matches drop (237 → 226). That is unscored,
  so the cross-group set has to be run at 768 px before choosing it.
- **768 px / 2,048 keypoints, cross-group and k-NN sets (2026-10-31): not adopted, 1024 px stays.** Cross-group
  (4,632 pairs, floor 0.3): 8 false matches vs 7; the new one (groups 162 vs 572) has SSCD similarity 0.339, below
  the 0.4 k-NN floor, the rest are the known one-shoot pairs (670/671, 125/673). k-NN set (8,344 pairs): 52 decisions
  flip (0.6%); duplicates 698 / 698, similar-folder 222 matches (224), unlabeled 3,000 (2,989). The gain is only ~9%
  on a quiet machine (labeled 169 s vs 185 s), because LightGlue and the comparison, not DISK, dominate.

**Rotations.** New synthetic edits (`edits.py`, appended so the older edits keep their seeds): a 3–15° tilt cropped to
the picture, and 30°, 45°, 135° and random 0–360° on an enlarged black canvas. 300 images each:

| Verifier | tilt + crop | 30° | 45° | 135° | any | 90° | Time (1,800 pairs) |
|---|---|---|---|---|---|---|---|
| DISK | 300 | 300 | 283 | 0 | 87 | 29 | 551 s |
| DISK, retry at 90° steps (`--rotations 90`) | 300 | 300 | 298 | 299 | 300 | 300 | (slowed by a concurrent GPU test) |
| DISK, retry at 45° steps (`--rotations 45`) | 300 | 300 | **300** | **300** | **300** | **300** | 1,123 s |
| SIFT (`--features sift`) | 300 | 300 | **300** | **300** | **300** | **300** | 511 s |

- **Retry:** a pair that doesn't match is matched again against B rotated by each step. The rotated copy's
  matched points are mapped back into B, so the homography, color fit and comparison use the images as they are.
  A retry only replaces the result if it matches.
- **No false matches from retries:** the cross-group set with 45° retries has the same 7 matches as without, and no
  rotated attempt matched any of the 4,625 others. 90° steps try a subset of those angles, so they can't add any
  either. **Cost:** every non-matching pair pays 7 extra extractions + matchings (cross-group 3,532 s vs 984 s).
  Retries should therefore be gated, e.g. only for candidates whose SSCD similarity is high for an unaligned pair.
- **SSCD finds rotated copies:** R@10 1.000 for every angle (cosine p10 0.50–0.55, above the k-NN floor 0.4), so they
  reach the verifier as candidates. Rotation is covered end to end with DISK + retries.
- **SIFT** (kornia, RootSIFT, oriented; LightGlue's SIFT weights) as the only features: synthetic 5,400 / 5,400 (all
  18 edits, incl. crop 25%), labeled 697 / 698, cross-group 9 false matches (5 with DISK; 3 new ones are weak, 37–73
  inliers). It misses `dups2/533` (14 inliers; DISK 326) and is slower on real images (labeled 402 s vs DISK's
  177 s, no TensorRT). **DISK stays the default**; SIFT is a possible fallback, but the 45°-step retry is cleaner (no
  new false matches).

### Shortcut and grouping (`eval/group.py`, k-NN set, subset, 2026-10-29)

**Shortcut** (PDQ ≤ 22 with both hashes reliable, or SSCD ≥ 0.95): 2,896 of the 8,344 k-NN pairs (35%), and
the fixed verifier matched **every one** of them, so skipping it changes nothing. `verify.py --shortcut` now
does that in pipeline mode (decision "match", `shortcut` = pdq / sscd, no image work).

**Grouping.** Each candidate pair becomes a signed edge: +1 for the shortcut, the verifier's decision weighted
by its noise margin when it compared the pair, and nothing when it couldn't align it (two crops of one picture
that don't overlap say nothing about each other). 7,300 of 8,344 pairs carry evidence: 3,908 match, 3,392
"aligned but different".

| Method | Groups (2+) | Largest | Labeled pairs precision / recall | Folders merged | Folders split | Loose files grouped |
|---|---|---|---|---|---|---|
| Connected components (union-find) | 2,869 | 17, 17, 7 | 0.9887 / 1.0 | 125+673, 670+671 | 0 | 4,458 / 4,574 |
| Min-cut split of conflicts (tried, dropped) | 2,871 | 17, 17, 7 | 0.9943 / 0.9986 | 670+671 | 1 | 4,457 |
| **Multicut (GAEC)** | 2,874 | 13, 12, 7 | **0.9943 / 1.0** | 670+671 | 0 | 4,458 |
| Multicut + candidate motion rule | 2,847 | 12, 9, 6 | **1.0 / 0.9986** | none | 1 (429) | 4,434 |

- **Connected components chain:** the two 17-image groups were a mirror-selfie series (one room, different
  outfits and poses) and AI portraits of different faces on one template. One false edge or a dense run of
  borderline ones is enough.
- **Min-cut on conflicts** (cut a group between two images the verifier aligned and called different, when at
  most one match edge joins them) fixed 125+673 but split folder 125: with equal edge weights any bridge is a
  minimum cut, often the wrong one. Replaced by:
- **Multicut** (correlation clustering, greedy additive edge contraction, Keuper et al. 2015): merge the two
  clusters with the largest positive summed weight until none is left, so a match is outvoted by the conflicts
  it would bring in. Fixes 125+673 without splitting anything, shrinks both chains.
- **What's left is the verifier's decision, not the grouping:**
  - 670+671 (different frames of one beach shoot) match at noise 0.02 with `moved_frac` ≈ 0.03; the motion rule
    separates them, but also splits folder 429, an AI image whose copy has a slight body warp (`moved_frac` 0.07).
    Frames vs. local warps is exactly what the hand marks have to decide.
  - The AI-portrait group (12) matches at noise 0.03–0.05 with low motion and interior change: same pose,
    framing and background, different face. Median noise can't see a different face; a face-region or
    learned decision could (see "duplicate vs variant" in Pending).
- Cost: grouping 8.3k pairs takes about a second (pure Python).

### Synthetic edits, all models (subset, 2026-10-29)

300 random images, each edit applied, then the edited copy is searched in the 9.3k-image subset.
Each cell is **R@10 · share above the model's 95%-recall threshold**. R@1 is lower for every model
and edit (0.75–0.93, even for JPEG q30) because exact duplicates of a sampled image already exist
in the index and tie or win rank 1, so R@10 is the retrieval measure here.

| Edit | sscd_fp16 | sscd_large | dinov2_s | dinov2_b | clip_b32 | pdq |
|---|---|---|---|---|---|---|
| JPEG q30 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.99 | 1.00 · 1.00 |
| Downscale 25% | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.99 | 1.00 · 0.76 | 1.00 · 1.00 |
| Grayscale | 1.00 · 0.97 | 1.00 · 0.96 | 1.00 · 0.60 | 1.00 · 0.81 | 0.99 · 0.02 | 1.00 · 1.00 |
| Color shift | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.99 | 1.00 · 1.00 |
| Text overlay | 1.00 · 0.00 | 1.00 · 0.00 | 1.00 · 0.80 | 1.00 · 0.73 | 0.97 · 0.00 | 1.00 · 1.00 |
| Letterbox 10% | 1.00 · 0.00 | 1.00 · 0.09 | 1.00 · 0.55 | 1.00 · 0.66 | 1.00 · 0.60 | 0.11 · 0.06 |
| Mirror | 1.00 · 1.00 | 1.00 · 1.00 | 1.00 · 0.65 | 1.00 · 0.98 | 1.00 · 0.99 | 0.00 · 0.00 ¹ |
| Rotate 90° | 1.00 · 0.00 | 1.00 · 0.01 | 0.95 · 0.00 | 0.95 · 0.00 | 0.99 · 0.01 | 0.00 · 0.00 ¹ |
| Crop 80% | 1.00 · 0.77 | 1.00 · 0.98 | 1.00 · 0.70 | 1.00 · 0.83 | 1.00 · 0.90 | 0.33 · 0.26 |
| Crop 60% | 1.00 · 0.00 | 1.00 · 0.09 | 1.00 · 0.30 | 0.99 · 0.38 | 0.99 · 0.49 | 0.02 · 0.01 |
| Crop 40% | 1.00 · 0.00 | 1.00 · 0.00 | 1.00 · 0.00 | 0.95 · 0.04 | 0.97 · 0.08 | 0.00 · 0.00 |
| Crop 25% | **1.00** · 0.00 | **1.00** · 0.00 | 0.95 · 0.00 | 0.80 · 0.00 | 0.85 · 0.00 | 0.00 · 0.00 |
| Crop 50% + mirror | 1.00 · 0.00 | 1.00 · 0.00 | 1.00 · 0.00 | 0.99 · 0.10 | 0.99 · 0.20 | 0.00 · 0.00 |

¹ PDQ queried with all 8 rotations/mirrors (`compute_dihedral`): mirror and rotate 90° → R@10 1.00,
d p50 0, within d ≤ 31 99.7%. The other rows don't change.

- **Only SSCD keeps every edited copy in the top 10, down to a 25% crop.** DINOv2 and CLIP fall
  to 0.80–0.95 on heavy crops and rotation. Rank is what the pipeline uses (k-NN + verifier), so
  this is the deciding result: SSCD stays the candidate model.
- **No global threshold survives heavy crops, overlays or letterboxing** for SSCD: the
  95%-recall threshold (0.879) is set by labeled pairs, which are mostly light edits. Confirms
  design conclusion 2 (rank + verifier, not a lower threshold) and 3 (trim borders first).
- **SSCD-large vs SSCD:** same retrieval; slightly higher similarity on crops (crop 80% above the
  threshold 0.98 vs 0.77). Not enough to justify 7× the cost (below).

### Synthetic edits, SSCD fp16 (preliminary, superseded by the table above: 100 images, 4.7k-image index)

| Edit | R@1 | Cos p50 | Verdict |
|---|---|---|---|
| JPEG q30 / downscale 25% / grayscale / color shift | 0.96–1.00 | 0.92–0.99 | handled |
| **Mirror (hflip)** | 0.96 | 0.95 | **handled** |
| Crop 80% / 60% | 0.96 / 0.93 | 0.90 / 0.81 | handled / mostly |
| Crop 40% / 25% | 0.90 / 0.89 | 0.67 / 0.54 | **below threshold, but still ranked #1–10** |
| Text overlay | 0.96 | 0.75 | borderline |
| Letterbox 10% | 0.93 | 0.75 | borderline |
| Rotate 90° | 0.94 | 0.73 | not handled at the threshold |

## Border cleaning (Python prototype, 2026-10-28)

The Python prototype, since ported to the gallery and removed (in git history): "Border detector in the gallery"
below. Finds solid (incl. JPEG-noisy colored) and blurred-padding borders and crops them; dry run by
default, writes cleaned copies only with `--apply`, never touches originals. Default output is a
lossless `jpegtran` crop (left/top snapped to the 8/16 px JPEG block).

Sample of 1,500 random test-set images (79 img/s, 4 workers):

- **35 with borders (2.3%)**, all correct on visual review: black letterbox bars, thin
  (5–20 px) dark frames around AI art, white side padding, a purple frame.
- **Rejected look-alikes:** plain studio walls, skies, and white backgrounds. Rules that made the
  difference:
  - **Real padding is digitally flat:** median deviation 0–3 and line color drift ≤ 5; walls
    have 5–7 and 9–17.
  - **Blurred bars have a straight seam (≥ 0.7) and come in equal pairs:** a one-sided "blurred
    bar" is always window light, curtains or a horizon.
- **Detection only on stored pixels**, so EXIF orientation stays valid after cropping.
- **For the NDD pipeline:** the same detector should run before embedding (design conclusion 3).

### Trimming before embedding (`embed.py --trim`, `synth.py --trim`, subset, 2026-10-30)

The detector runs on each decoded image and the crop is embedded (`imgload.TrimmedSource`). It trims 223 of the
subset's 9,306 images (2.4%), and it also runs on the synthetic edited copies.

| SSCD fp16 | Untrimmed | Trimmed |
|---|---|---|
| Letterbox 10%: R@1 / cosine p50 | 0.807 / 0.740 | **0.930 / 0.998** |
| Other 17 synthetic edits: R@1 | – | same within ±1 image (R@10 1.000 everywhere) |
| Labeled pairs: R@1 / R@10 | 0.964 / 1.000 | 0.962 / 1.000 |
| Flagged pairs at 80 / 90 / 95 / 98% recall | 2,086 / 2,485 / 2,643 / 2,835 | 2,085 / 2,465 / 2,627 / 2,839 |
| Cross-group pairs flagged at 98% recall | 4 / 915,283 | 4 |

- **A letterboxed copy becomes the same picture to SSCD** (cosine 0.74 → 0.998), and nothing else moves: the labeled
  set barely has borders, so it's neutral there. Trimming stays in the design (design conclusion 3); cost is the
  detector's CPU time (~80 img/s per thread on full-size images, less on the DCT-scaled decode).
- Measured with the Python prototype. `--trim` now runs the gallery's detector (`gallery borders --stdin`) on the
  same loaded images, and gives the same box on all 900 checked (below), so the table stands.

## Border detector in the gallery (Rust, `crates/analysis/src/borders.rs`, 2026-11-01)

A port of the Python prototype, in integer arithmetic where the prototype used float32. It runs in a pass of its own
after the import, on the stored pixels decoded at full size, and its results are in the catalog (usage in
`crates/README.md`).

- **Parity with the prototype on all 187,806 Pinterest images:** the same **4,539 borders with the same cut on
  every file** (31,077 vs 31,084 files with rejected candidates only).
  - All remaining differences are the prototype's float32 rounding at exact ties of the seam's detail test: 7
    statuses and about 1,600 printed straightness values (±0.01).
  - Rerunning the differing files in Python with exact arithmetic makes every one of them match the port.
- **Not on the import's decode.** The import decodes JPEGs DCT-scaled (≥ 512 × 320), and the detector is
  calibrated on full-size pixels (3 px minimum, 5–20 px frames, per-row seam tests), so it can't run there. A gate
  on the scaled decode was tried: decode at full size only when a side's first line is solid, or two opposite
  sides look blurred.
  - It lost no border, but sent **24%** of all files to a second full decode (44% before requiring the blurred
    pair). Only 3,648 of those 45,349 had a border.
  - Downscaling averages away JPEG noise, so plain backdrops, skies and white product shots look solid at
    reduced scale.
  - So detection became a background pass after the import, on full-size decodes (decision): exact on
    every file, and the import stays as fast as before. The cost is a second read of each still image.
- **Speed:** on `dups2`, the pass took 16.4 s for 5,934 files (362 files/s, 12 workers), the import 20.9 s.
- **Eval:** `imgload.trim_borders` pipes each loaded image to `gallery borders --stdin`. On 900 images (300 with
  borders, 300 with rejected candidates, 300 random) it gives the prototype's box on all 900 (307 trimmed).

## M1 import (Rust, `crates/`, 2026-10-30)

Measured on `dups2` (5,937 files, copy on D:), with GPU benchmarks running alongside, so
the speeds are lower bounds. The full 190k-image run was stopped before it finished.

- **Speed:** 341 files/s on 12 workers (14 ms per image on one thread: decode 8.7, PDQ 2.3, Lanczos to
  512 px 1.7, rest < 0.2). The pipeline is CPU-bound, as M0 predicted.
- **PDQ matches M0.** The Rust port (dihedral variants derived from the DCT, as Meta's hasher does) is
  bit-exact with the `pdqhash` crate. Against `eval/pdq.py`'s hashes: 74% identical, 98% within 4 bits, max
  14 (the DCT-scaled decode differs slightly from PIL's full decode). Labeled-pair recall at d ≤ 0 / 8 / 16 /
  22 / 31: **30.8 / 66.0 / 75.4 / 80.4 / 83.2%** (M0: 32.6 / 65.7 / 74.8 / 80.1 / 82.8%). No cross-group pair
  at d ≤ 31.
- **Tiers 0a/0b:** 0 byte-identical groups and 234 pixel-identical groups (468 files), as expected for
  `.mozjpeg.jpg` re-encodes (see "Test set").
- **Resumability:** an import killed after 1,500 files and rerun ends with the same 2,784 pairs as an
  uninterrupted run. Renamed and moved files keep their analysis (file ID identity); a deleted file is marked
  missing; a touched file is re-analyzed alone.
- **Thumbnail task:** 415 files/s at normal priority. Windows' background mode (`PROCESS_MODE_BACKGROUND_BEGIN`)
  dropped that to 64 files/s, because it also throttles I/O on an idle disk, so the task uses idle *CPU*
  priority instead (330 files/s with other load on the machine).

## Videos and animations (M2, 2026-10-30)

Our FFmpeg was BtbN's `lgpl-shared` 9.0 build (`D:\PhotoGallery\tools\ffmpeg-9.0`), shared with Qt 6.12's player.
From 2026-11-02 it is BtbN's `gpl-shared` build of the same 9.0.2 commit, the only FFmpeg on this machine's tools path:
the app and the video test use one copy, and the test's x264 clips need the GPL encoders.

### Measured here

- **Dataset videos** (`pinterest`, 1,712 files copied to `D:\PhotoGallery\scratch_videos_all`): 837 H.264,
  748 VP9 and 127 VP8. There is no AV1, HEVC or HDR among them, and 45 are larger than 1080p. Import: 1,712
  files with 0 failed, in 20 s (84 files/s, about 200 MB/s). Thumbnails (poster frames at 256 px): 1,712 with
  0 failed, in 19.5 s. 63 H.264 files carry a creation date.
- **What the dataset videos test** (header-only ffprobe of all 1,712, one at a time; 1,689 answered, and a
  re-run of a failed one worked, so the 23 failures were the run's):
  - All 8-bit 4:2:0; profiles: VP9 profile 0, H.264 Constrained Baseline (581), High (232) and Main (23).
  - 3 are HLG with BT.2020 primaries (`tumblr_*`), 8-bit. No PQ, no 10-bit, no Dolby Vision.
  - No display-matrix rotation; 1,409 are stored portrait.
  - 1,057 have no audio. The other 632 are AAC, all but 7 stereo, mostly 44.1 kHz.
  - One has a variable frame rate; two start more than 0.1 s late (an edit list or a delay).
  - Median length 6.4 s, longest 9 min.
  - So a player test bed also needs generated or sourced HEVC, AV1, 10-bit, PQ, Dolby Vision, rotated and
    mirrored, and surround files.
- **A display matrix's mirror comes after its rotation** (`av_display_matrix_flip` negates the first column;
  Qt's player reads it as "mirrored horizontally after rotation"). EXIF mirrors before rotating, so a mirrored
  matrix's turn is reversed. The importer had it the other way round, so every mirror came out wrong (a
  horizontal flip stored as a vertical one). Fixed 2026-10-31; the eight matrices ffmpeg writes are a unit test.
- **Qt's player can't play AV1 on this GPU** (RTX 2060, Turing, which has no AV1 decoder):
  - Qt picks FFmpeg's built-in `av1` decoder with d3d11va. That decoder only decodes on the GPU. Qt checks
    only that a D3D11 device can be created, not that the GPU decodes the codec. FFmpeg then fails its
    hardware setup on every frame, and the decoder has no software path.
  - Qt falls back to software only when the decoder fails to open (`qffmpegcodeccontext.cpp`,
    `qffmpeghwaccel.cpp`, v6.12.0 and the dev branch alike), so the result is 0 frames and no error shown.
  - The only switch is `QT_FFMPEG_DECODING_HW_DEVICE_TYPES`. It's private API, global for every codec, and
    read once per process.
  - Our stopgap (`crates/desktop/src/gpu_device.rs`): ask D3D11 for the AV1 profile at startup and, without
    it, turn hardware decoding off. Then AV1 plays through libdav1d (61 frames in 2.4 s), but every codec
    decodes on the CPU on this machine.
  - Rust's `std::env::set_var` doesn't reach Qt here. Qt reads the C runtime's copy of the environment,
    which `SetEnvironmentVariableW` doesn't update, so the variable is set with `qputenv`.
- **FFmpeg can't loop animated WebP:** its `webp_anim` demuxer can't seek back to the start.
  `ffmpeg -stream_loop 2` gives 10 frames instead of 30, while GIF and APNG give 30. Through Qt's player, an
  animated WebP plays once, then restarts tens of thousands of times a second.
  - Qt's `AnimatedImage` has no APNG, and WebP only with the Qt Image Formats add-on, which isn't installed.
  - So animations play through our own player (`crates/desktop/src/animation.rs`: the `image` crate's decoders,
    frames pushed into the VideoOutput's sink). Verified for WebP (70 ms frames, loops), GIF and APNG (200 ms
    frames, loop), and pause.
- **libwebp_anim merges identical frames:** 10 identical frames became one still VP8L image. Tests check the
  total duration, not frame counts.
- **Offscreen Qt (software scene graph) draws no video frames.** Playback can only be checked through the
  sink's frame count and the player's position.

### Video player options (web research, 2026-10-30; player not chosen yet)

- **Qt Multimedia:**
  - The same failure shows on other hardware (Qt Forum, "Qt Multimedia 6.7.2 with ffmpeg cannot decode h264
    on Pi5", topic 159939; a reply blames missing checks that the chosen hardware decoder can work).
  - Qt's GStreamer backend page calls Qt Multimedia "not a general purpose streaming framework". That backend
    is for embedded Linux, not Windows.
- **libmpv:**
  - Its manual says that when hardware decoding isn't possible, "mpv will fall back on software decoding";
    it goes down the list of hardware decoders, then software, for each file.
  - It has `loop-file`, `frame-step` and HDR tone mapping.
  - Qt Quick integrations (KDE MpvQt, mpv-examples, 2026 guides) render through OpenGL, so they force Qt
    Quick off Direct3D (`QQuickWindow::setGraphicsApi(OpenGL)`).
  - mpv PR #17764, "libmpv: add D3D11 render API backend", is merged (milestone 0.42.0). mpv renders into an
    application-owned `ID3D11Texture2D` on the application's device. The latest release is still 0.41.0
    (2025-12-21).
  - Daily LGPL builds of libmpv from mpv's git exist (zhongfly/mpv-winbuild, `mpv-dev-lgpl-x86_64-…`; the
    2026-10-29 build). They link their own FFmpeg statically.
- **GStreamer:**
  - It registers its D3D11 decoders (`d3d11av1dec`, …) only when the GPU supports them, so falling back per
    codec is built in (GStreamer `d3d11` plugin documentation).
  - `qml6d3d11sink` (gst-plugins-bad) shows D3D11 frames in a Qt 6 QQuickItem without copies.
  - Costs: a third media stack and a large runtime. The Qt plugin is built against a Qt version (not checked
    for 6.12).
- **QtAVPlayer** (github.com/valbok/QtAVPlayer: MIT, 454 stars, active October 2026):
  - FFmpeg decoding, frames into `QVideoSink` (D3D11 frames without copies are claimed), sound through
    `QAudioSink`, accurate seeking, frame stepping.
  - It claims an automatic fallback to software decoding. Not verified for the AV1 case above.
- **Rust:**
  - No mature player library. A July 2026 survey of Rust media crates lists decoding toolkits only, and
    ffmpeg-next is in "maintenance-only mode".
  - GUI crates wrap GStreamer (`iced_video_player`) or libmpv (`egui_sharkplayer`, through `libmpv2`), or
    hand-roll FFmpeg playback (`egui-video`).
- **Options (not decided):** third-party wrappers (QtAVPlayer, MpvQt, `libmpv2`) are out: the integration
  stays ours. The two candidates are our own engine on our FFmpeg 9, or libmpv on a pinned master commit
  with the D3D11 render API. Both are written up, with spikes, in [VIDEO_PLAYER.md](VIDEO_PLAYER.md).
  Animations stay on our own decoders either way.

## Reverse image search: Yandex (ImageRater port, 2026-10-31)

ImageRater (`ImageRater/shared/static/frontend.js`, `reverseImageSearch`, lines 984–1026) uploads the photo to
`yandex.com/images-apphost/image-download` and opens `yandex.com/images/search?cbir_id=…` in a new tab. Only
`yandex.com` is used there; `.ru` hosts appear only in the browser's analytics traffic. The upload returns
`cbir_id` and `sizes.orig.path`. Measured here with a standalone Rust probe (not committed) and
a Playwright browser session on the same test image (a generated 64×64 PNG).

### What the results page contains

- **Two lists.** Sites (`cbir_page=sites`, paged with `&p=N`) and Similar (`cbir_page=similar`, one page).
- **Data source.** The results are an HTML-escaped state blob in the page (`originalImage&quot;:{&quot;url&quot;:…`).
  The infinite scroll renders from this blob with no further requests. The DOM stops at about 150 items, so it
  is not a complete source.
- **Sites record fields.** `title`, `description`, `url` (source page), `domain`, `thumb` (URL, width, height)
  and `originalImage.url`, the full-size file to download.
- **Similar items.** 55 items, `pos` 0–54. Each item's `img_url=` parameter is the full-size URL.
- **Overlap.** 43 of the 55 Similar images also appear in the Sites set (browser data).
- **Browser totals.** Sites pages 0–5 hold 492 unique originals in all (pages 4 and 6–10 come back empty; pages
  beyond 10 were not checked).

### Why a plain HTTP client gets fewer results

Measured with the same URLs and the same test image:

| Client | Sites p0 | Sites p1 | Similar |
|---|---|---|---|
| Browser (Chrome, Playwright) | 160 records | 155 | 55 items |
| Probe, no cookies | 118 records (79 unique sources) | 0 | 40 items |
| Probe, homepage visited first (`yandex.com/`, cookie jar) | 118 | not measured | 40 |
| Probe, `yp` cookie with screen metrics only | **160** | 0 | **55** |
| Probe, `yp` + `spravka` | not measured | **155** | not measured |

- **Not the cause:** HTTP/1.1 vs HTTP/2 (curl gives the same 118), compression, and browser-like request headers.
- **`yp` cookie.** The browser's JavaScript writes screen metrics into `yp`, in the shape `<ts>.szm.1:WxH:WxH`.
  A fresh client gets `<ts>.ygu.1` from `yandex.com/` and nothing from `yandex.com/images/`. With `yp` in szm
  shape and no other cookie, page 0 and Similar match the browser.
- **`spravka` cookie.** Sites pages 1 and up are empty without it. It appears to be Yandex's anti-robot token;
  the browser had it, probably from the captcha it passed earlier in the session. This was not confirmed.
  The probe does not send it.
- **Captcha.** Yandex returned `showcaptcha` after a few dozen requests from the automated browser. It was
  cleared by hand. Nothing in this section gets past it.

### Crawlers script: `crawlers/yandex-reverse/search.py` (2026-10-31)

The crawlers monorepo already has a Yandex client. Its `perform_yandex_search` gets the same two groups, and
the Sites list above was the wrong target for group 1:

- **Group 1 is "other sizes".** The script reads `initialState.cbirOtherSizes` from the `ImagesApp-*`
  `data-state` attribute on the results page. Those are the same image at other resolutions, which is what
  group 1 means. The Sites list (`cbir_page=sites`) is a different section.
- **Group 2 is the similar JSON API.** `images/search?format=json&request=…&cbir_page=similar&p=N` (the request
  body names the `serpList/fetch` ajax block). Each item's `img_url` query parameter is the full-size URL.
- **Measured on a public-domain photo** (Fronalpstock, Wikimedia Commons, 14.6 MB JPEG), same settings for both
  runs (`SIMILAR_PAGES_TO_DOWNLOAD = 5`):

| Run | Other sizes | Similar |
|---|---|---|
| As written (no cookies) | **18** (largest 10109×4542) | 40, page 1 fails (`Unexpected API response structure … 'blocks'`) |
| With `yp` cookie (szm) | **18** | **55**, page 1 fails the same way |

- **Group 1 needs no cookie.** The other-sizes block is in the initial HTML, and the count matched with and
  without `yp`.
- **Group 2 needs the `yp` cookie** for the full 55 on page 0.
- **Page 1 of Similar is a captcha, with or without `yp`** (rechecked 2026-10-31). Yandex answers it with HTTP
  200 and JSON `{"type": "captcha", "captcha": {…}}`, which links to `showcaptcha`. The script never reports it:
  it reads that as an unexpected structure, stops paging and keeps page 0. So the script gets one page of
  similar images, whatever `SIMILAR_PAGES_TO_DOWNLOAD` says (20 in the script).
- **The app's first version failed on it.** Its client reported the page-1 captcha as an error of the whole
  search, so the other sizes and page 0 were dropped and the panel said "captcha". It now does what the script
  does: a failure on a page after the first ends the list.
- **Not measured:** whether the browser shows all 18 other sizes or about 5 (the user's "5 or so shown").
- The script's known failures (`files_not_working.md`): upload 400s, SSL EOF and remote disconnects.
  `tasks.md`: CDN 403s that load in a browser, and no captcha validation for HTML responses.

### Decision and the port (2026-10-31)

- **The `yp` szm cookie is sent** (user decision): it is a cookie the site's own JavaScript sets, and no
  challenge is passed. `spravka` is never sent, so the Sites list is not fetched (its pages 1 and up need it,
  and copying it from a passed-challenge session would get around the bot check).
- **The port** (`pg_yandex`, the panel in `crates/desktop`, usage in PLAN §8 M2): plain HTTP to `yandex.com`
  like the script, group 1 from the results page's `cbirOtherSizes`, group 2 from the similar JSON API with
  `yp`, up to 5 pages. Like the script, a failure on a page after the first (in practice the page-1 captcha)
  ends the list and keeps what came before. A failure on page 0 of the similar images shows the other sizes with
  a note; one on the upload or the other sizes is the search's error. A captcha is reported, never retried.

## Startup (2026-10-31)

Measured on a copy of the real catalog (189,518 files, 690 folders; session "All folders", recursive, by name) and
its thumbnail cache (189,518 thumbnails at 220 px, 2 GB), release build, timestamps logged from process start
(temporary traces). "Cold" is the first run after copying the files; "warm" the next one.

### Before

| Step | Cold | Warm | On the GUI thread? |
|---|---|---|---|
| Process start → `main` (DLLs) | 0.3 s | 0.03 s | — |
| D3D11 AV1 probe (`video_decoding`) | 0.33 s | 0.32 s | yes, before anything |
| `state::init`: `SELECT sum(bytes) FROM thumbs` | **10.7 s** | 0.46 s | yes, **before any window** |
| `engine.load` until `onCompleted` | 3.0 s | 2.1 s | yes |
| — of which Qt Multimedia's hardware probe | 1.36 s | 1.36 s | yes |
| `onCompleted`: the grid's list (`list_files`, 189k rows) and layout | 1.0 s | 0.75 s | yes |
| **First frame** | **15.4 s** | **3.7 s** | |
| The job's `pending()`, holding the cache mutex the thumbnails need | 1.7 s | 1.6 s | no, but it blocks them |

  Note 2026-11-06: the names in this section have since changed. `state::init` is not in the tree, and the cache file is
  `thumbs.sqlite` (`crates/thumbcache/src/lib.rs:130`), not `thumbs-v3.sqlite`. The measurements stand as recorded.

- **The cache's layout made every metadata scan read the JPEGs' pages.** `thumbs` was one rowid table with each
  ~10.5 KB JPEG inline as its last column, so each row's start (with `bytes`, `mtime`, `last_used`) sat on its own
  leaf page among the blobs' overflow pages. `sum(bytes)` read 522 MB in 127k page reads. Measured read-only with
  `GetProcessIoCounters`.
- **`pending()` read 2 GB with nothing pending.** It looks each catalog file up in `thumbs`, in the catalog's
  on-disk order: random order for `thumbs`. With SQLite's default 2 MB page cache every lookup re-read its index
  and table pages: 498k reads (2.6 per file), 6.2 s even with the file in Windows' cache. It ran under the mutex
  the on-screen thumbnails lock, after the import pass at startup and at every row-height change.
- **Qt Multimedia probes every hardware device type for 1.4 s** (CUDA 0.11 s, QSV 0.85 s, Vulkan 0.3 s;
  `qt.multimedia.ffmpeg.hwaccel`, "Device types checked"), on the GUI thread when the hidden viewer's player is
  made. It does so for `QT_FFMPEG_ENCODING_HW_DEVICE_TYPES` too: unset, the list is probed (`qffmpeghwaccel.cpp`,
  `deviceTypes(const char*)`), and the app never encodes through Qt. Our `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` was
  set only on a GPU without AV1 decoding, so on one with it the decoders were probed as well.
- **SQLite details found on the way** (Python 3.12's SQLite 3.49, in memory): after an upsert that updates,
  `last_insert_rowid()` is the id of the last row *inserted*, another file's, so keying the JPEGs by it would have
  put one file's thumbnail under another; `RETURNING id` is right on both paths. A deferred transaction that
  reads, then writes after another connection committed, fails at once with `SQLITE_BUSY_SNAPSHOT` (517), whatever
  the busy timeout: two processes writing the old cache (the app and `gallery thumbs`) could fail at random.
  `INSERT OR REPLACE` fires delete triggers only with `recursive_triggers` on. On Windows a SQLite file open
  elsewhere can't be deleted (WinError 32: SQLite opens without `FILE_SHARE_DELETE`).
- **Two more, found reviewing the change.** `PRAGMA incremental_vacuum(N)` frees one page per step (it returns a
  row per page), and rusqlite's `execute_batch` steps each statement once: through it the old cache's "vacuum"
  freed one page a call. It has to be stepped to the end. `sqlite3_interrupt` only stops the statements running
  at that moment: one arriving between two statements of a read is lost (and `in_scope` dropped the failed rows
  of an interrupted folder scan, so the read went on). A flag checked by a progress handler, cleared only when the
  next request starts, cancels the whole read.

### After

The same catalog copy, with the new cache filled (`gallery thumbs --height 220`, 183,612 thumbnails, 2.00 GB, 492 s
at idle priority). "First" is the first run after a new build, whose pages and the catalog's weren't in
Windows' cache (the fill had just read 2 GB of images); "warm" the next one.

| Step | First | Warm |
|---|---|---|
| Process start → `main` | 0.07 s | 0.02 s |
| `state::init` (the totals are one row) | 0.01 s | 0.01 s |
| Qt starts, the D3D11 probe (on a thread) joined | 0.38 s | 0.33 s |
| `engine.load` (no Qt probe: both lists set) | 1.33 s | 0.70 s |
| **First frame**, "Loading…" | **1.80 s** | **1.07 s** |
| The grid's list, read off the GUI thread, shown | 4.5 s | 2.2 s |
| `gallery thumbs` with nothing pending: open, heal, `pending()`, all of it | 1.0 s | |

- **The window paints 3.5× sooner warm, and no longer waits for the disk.** Nothing before the first frame
  reads the cache or the files; the catalog's only read there is opening it.
- **`pending()` with 189k files to do takes 0.5 s** (it was 1.6 s with nothing to do, and blocked the
  thumbnails); the first run of the new build made the empty `thumbs-v3.sqlite` and deleted the old 2 GB file.
- **What's left in `engine.load` is Qt's.** Creating the viewer's `MediaPlayer`, `AudioOutput` and
  `VideoOutput` (which loads the FFmpeg plugin) made no measurable difference in Qt's `qml` tool: 0.61–0.82 s from
  process start to the first frame with them, 0.64–0.80 s without, over three runs each. So they aren't deferred.
- **The cache's totals held** through a `gallery thumbs --cap-gb 1.9 --height 100` run killed after 20 s: 1.897 GB
  in 187,068 entries, equal to the entries' sum, no JPEG without its entry; the 11,000 new 100 px thumbnails had
  evicted 220 px ones, the least recently used first.

## Decisions

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
  3 days brute force for 70M on the 2060, streaming from the NVMe; measured 2026-10-31: ~5.5 days,
  "Nearest-neighbour search"), multiple embeddings per
  image, and a learned geometric verifier are all in scope. Resumability and
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

## Design conclusions so far

1. **Mirrors:** SSCD is flip-tolerant, so there is no need to store a flipped embedding
   (resolves PLAN §9 decision 3, pending confirmation on the full set).
2. **Heavy crops:** a single global threshold fails, but rank still works. Use k-NN rank plus a
   verifier for the ~0.4–0.88 band (Pinterest-style), not a lower global threshold.
3. **Borders:** trim borders before embedding, including blurred padding bars (a common
   Pinterest/TikTok edit seen in `dups2/075`). Measured 2026-10-30: fixes letterboxed copies, neutral elsewhere
   ("Trimming before embedding").
4. **Rotation:** SSCD still ranks copies at any rotation in the top 10 (R@10 1.00), but far below
   threshold, so rank + verifier covers it: the verifier retries a pair that doesn't match with the other
   image rotated in 45° (or 90°) steps, with no new false matches ("Verifier: TensorRT, rotations, SIFT,
   resolution"). Gate the retries to keep their cost down. Apply EXIF orientation everywhere; PDQ's dihedral
   query covers the exact-copy tier.
5. **Throughput:** read in MFT order with a deep queue (~1,600 files/s), decode with DCT scaling
   (~550 img/s on 12 threads), embed with SSCD fp16 on TensorRT (1,390 img/s; 410 in torch). The CPU is
   now the limit: decoding ~1.5 days for 70M, plus thumbnail encoding. SSCD fp16 is the model; 288px is
   the same quality (R@10, cross-group 0) if speed is ever needed. SSCD-large, DINOv2 and CLIP are
   dropped as candidate models. int8 is not used (see "TensorRT and ONNX Runtime").
6. **PDQ** is an optional cheap tier for near-exact copies (~80% of labeled pairs at d ≤ 22 with
   0 false positives), never a replacement for SSCD.
7. **Thumbnails:** Lanczos3 from a DCT-scaled decode kept ≥ 4× the target; display-only (not a
   re-analysis source). **Decision 2026-10-30:** not made during the import; a background task makes
   them afterwards at a size adjustable at runtime, into a size-capped cache (PLAN §6). The size/format
   numbers in "Thumbnails" now size that cache rather than a fixed 70M store.
8. **k-NN candidates by exact search on the GPU** (recommended, not decided; PLAN §9 decision 5), not an
   approximate index: ~5.5 days for 70M all-pairs on the
   2060 at the recall ceiling, ~70 s per 10,000 new images afterwards. On synthetic 5M data the best approximate
   index (FAISS IVF, nprobe 256) loses ~23% of the partners at ≥ 0.4 and over half of the heavy crops; real data
   will do better, but exact search makes that question moot ("Nearest-neighbour search"). An index is still
   needed for interactive semantic search, where one text query must scan 70M So400m vectors.

## Phase 0 baseline (2026-11-02, before the refactor)

The numbers the refactor's before-and-after comparison uses (plan step 0). The raw files are in
`D:\PhotoGallery\baseline_2026-11-02`, and the gungraun logs are in WSL under `~/photo-gallery/target/gungraun`. The
files don't record the commit they were measured at, so read them as a reference for that day.

### Instruction counts (gungraun, callgrind `Ir`, WSL, 16:10)

| Bench | Ir |
|---|---|
| `catalog::tree.all` | 3,275,602 |
| `catalog::list.folder_flat` | 735,865 |
| `catalog::list.folder_recursive` | 33,928,736 |
| `catalog::list.all_name` | 32,015,927 |
| `catalog::list.all_taken` | 46,406,218 |
| `catalog::list.all_unrated` | 32,241,343 |
| `catalog::facets.all` | 93,581,775 |
| `catalog::pending_import.empty` | 115,827 |
| `catalog::pending_thumbs.at_height` | 29,447,015 |
| `per_file::analyze_jpeg.sample` | 60,603,752 |
| `per_file::thumb_jpeg.h256` | 52,045,318 |

The `pg-core` folder holds the 2026-10-31 runs of the two per-file benches, under the crate's old name:
`analyze_jpeg.sample` 63,523,272 and `thumb_jpeg.h256` 50,677,681.

### Startup stages (`D:\PhotoGallery\baseline_2026-11-02\startup_runs.json` and `synthetic_startup_runs.json`)

Milliseconds from process start, median of 5 runs (each stage's min and max are in the JSON).

| Stage | Real library (189,518 files) | Synthetic library (4,000 files) |
|---|---|---|
| `main` | 27.2 | 21.5 |
| `init_done` | 60.2 | 27.0 |
| `qml_load` | 82.9 | 45.7 |
| `qml_loaded` | 363.1 | 230.6 |
| `tree_shown` | 454.6 | 322.8 |
| `list_shown` | 1001.4 | 346.9 |
| `first_frame` | 478.6 | 349.7 |
| `ready` | 1144.6 | 473.7 |
| `round_done` | 3730.6 | 3303.9 |

### Import and thumbnails (`import.log` and `synthetic_build.log`)

- Pinterest sample (`D:\PhotoGallery\datasets\pinterest`, 189,518 files in 693 folders): the scan took 6.3 s (about
  30,000 files/s). The import ran at 432–445 files/s (58–63 MB/s) over its first 6,554 files; the log ends there, with
  "6500 analyzed", so the run was stopped.
- Synthetic library (`D:\PhotoGallery\synthetic\library`, 4,000 files): the scan took 0.2 s. The import took 9.6 s
  (417 files/s, 148–158 MB/s, 1,590 PDQ pairs, 0 failed). The 256 px thumbnails took 11.4 s (351 files/s, 124–132 MB/s).

### Test-hook screenshots

The phase 0 hook screenshots are in `D:\PhotoGallery\baseline_2026-11-02\shots`: `start_0` to `start_5`, `library`,
`filters`, the `hooks_*` set (keys, zoom, similar, crops) and the `symmetric_*` set.

## Refactor S-02.3: deploy, icon and live reload (2026-11-03)

The S-02 verify items that the repo did not record.

- **`just deploy`** (release build, then `windeployqt`) at 713cd7e: exit 0 (`deploy5.log`). Two warnings, neither an
  error: no `dxcompiler.dll` or `dxil.dll` (needed only for Direct3D 12), and `VCINSTALLDIR` is not set (no Visual
  Studio installation directory). The same two warnings are in the earlier log `deploy4.log` (13:05).
- **Icon:** the icon resource of `target/release/gallery.exe` (its `winres` build step) shows the app icon at 32 × 32,
  both in the extraction of the 05:24 build (`exe_icon.png`) and of the 13:05 release build (`exe_icon_recheck.png`).
- **Live reload:** on the debug build, with the scratch catalog and prefs. The control run, with no edit, showed
  "No folders in the library yet." (`live_control.png`). The test run changed the text of `Main.qml` 9 s after start
  to "LIVE RELOAD CHECK.", and the window showed the new text (`live_changed.png`). The script reverts `Main.qml` with
  `git checkout` on exit; `Main.qml` is clean.
- **`lint-qml`** runs in `just lint`, and passed at 713cd7e: "QML: no warnings; all 735 bindings and functions
  compiled to C++ (7 files)".
- **Screenshots:** the grid, viewer and filters screenshots match the pre-split build (R-14, 0ac379e). That check was
  recorded at 0ac379e and was not run again today.

## Refactor S-16.1: the counts against S-00, measured again (2026-11-04)

The Phase 0 counts (2026-11-02) do not reproduce on this machine now, so the comparison uses S-00 re-measured today.
The S-00 commit `49d8f3b` was built in a clone in WSL (`~/pg-s00`) and run with the same bench data (`bench-data`, the
layout `v1-orderkey-synthetic`, copied from the same `synthetic_catalog`: schema and row counts match) and the same
toolchain (rustc 1.99.1, valgrind 3.26.0). HEAD's counts are `docs/perf/baseline.json` (a22fbad).

| Bench | S-00, Nov 2 | S-00, today | HEAD (baseline.json) | HEAD vs S-00 today |
|---|---|---|---|---|
| `per_file::analyze_jpeg.sample` | 60,603,752 | 60,603,835 | 60,604,034 | +0.0% |
| `per_file::thumb_jpeg.h256` | 52,045,318 | 52,045,531 | 52,045,748 | +0.0% |
| `catalog::list.all_name` | 32,015,927 | 32,988,949 | 32,349,515 | −1.9% |
| `catalog::list.all_taken` | 46,406,218 | 47,414,681 | 46,738,460 | −1.4% |
| `catalog::list.all_unrated` | 32,241,343 | 33,214,459 | 32,574,673 | −1.9% |
| `catalog::list.folder_recursive` | 33,928,736 | 34,902,511 | 34,261,837 | −1.8% |
| `catalog::list.folder_flat` | 735,865 | 746,553 | 730,480 | −2.2% |
| `catalog::tree.all` | 3,275,602 | 3,221,409 | 3,137,101 | −2.6% |
| `catalog::facets.all` | 93,581,775 | 105,208,622 | 104,873,332 | −0.3% |
| `catalog::pending_import.empty` | 115,827 | 115,859 | 115,391 | −0.4% |
| `catalog::pending_thumbs.at_height` | 29,447,015 | 46,288,886 | 46,328,133 | +0.1% |

- **The Nov 2 numbers are not the S-00 code's numbers on this setup.** Today's S-00 build is 12% above its Nov 2
  `facets.all` and 57% above its Nov 2 `pending_thumbs.at_height`; the cause of that gap is not established (the
  toolchain or the WSL setup changed since Nov 2; the bench data is the same synthetic catalog). The same-day S-00
  re-measure is the reference from here on.
- **Against that reference, five of eleven are within ±1%:** the two per-file benches, `facets.all`,
  `pending_import.empty` and `pending_thumbs.at_height` (within 0.4%). The other six, the five `list` benches and
  `tree.all`, are 1.4–2.6% *lower* at HEAD. That is fewer instructions, not a regression, but it is outside the ±1% band and the cause is not measured.
  The bench bodies changed for those (the typed `RatingFilter::Unrated`, `Catalog` in place of `&Connection`, the
  `Entry` and `FolderTree` types from `pg_model`), which is the likely source. Left as is; `just perf` gates HEAD
  against `baseline.json`, so a further drop would fail it as a change to re-baseline on purpose.
- **Not in this entry:** the Pinterest import throughput and the startup stages (S-13.5) are long timed runs against
  `D:\PhotoGallery\baseline_2026-11-02`; both are still open.

## Refactor S-13.5: import and startup against S-00, measured again (2026-11-04)

Same machine, same day. S-00 (`49d8f3b`) was exported with `git archive` to `D:\PhotoGallery\s00-src` and built in
release (`D:\PhotoGallery\s00-target`, 7 min). Its Qt and FFmpeg DLLs were copied from HEAD's deployed folder, since the
S-00 build had not been deployed (without them it exits with `STATUS_DLL_NOT_FOUND`). HEAD is the working tree's
release build (`target\release`, rebuilt 21:51); its uncommitted changes are tests, the CI file and an assert that only
runs when a tool is missing. The harness was a local script, not in the repo; the copies are under
`D:\PhotoGallery\s13`.

**Import throughput** (Pinterest folder, 189,519 files; `scan`, then `import` from an empty catalog, stopped after about
6,800 files; two rounds each, the rate at the last progress line):

| Build | Round 1 | Round 2 |
|---|---|---|
| S-00 | 460 files/s (6,899 files) | 447 files/s (6,767 files) |
| HEAD | 450 files/s (6,764 files) | 447 files/s (6,710 files) |

Within run-to-run noise (about 2%). No failures in any run.

**Startup stages** (`PG_TRACE=1`, offscreen, ms since process start, median of five runs; the Pinterest catalog copied
and read into the OS cache before each start):

| Stage | S-00 (copy of the S-00 catalog) | HEAD (catalog already migrated, see below) |
|---|---|---|
| `main` | 29 | 24 |
| `init_done` | 183 | 40 |
| `qml_loaded` | 296 | 262 |
| `tree_shown` | 339 | 329 |
| `list_shown` | 773 | 704 |
| `first_frame` | 360 | 347 |
| `ready` | 869 | 807 |

`round_done` did not print within 15 s of `ready` on either build, so it is not compared. Two S-00 runs had a
`list_shown` 5–7 s after `tree_shown` that no HEAD run had; the cause is not known.

- **The first start of HEAD on a catalog written by S-00 takes about 12 s more.** `init_done` was 11.5–12.7 s on five
  fresh copies of that catalog, and `ready` 12.4–13.8 s. HEAD migrates the catalog in place on the first open: the
  definitions of `files`, `analysis`, `roots` and four of their triggers change, eight stamp triggers are added (on
  `ratings`, `file_writes` and `border_marks`), and the change counter `seq` goes from 379,037 to 900,261.
  A second start on the migrated copy is steady: `init_done` 40 ms, `ready` 807 ms (median of five).
  The project's rule is "no schema bumps before release" and catalogs are rebuilt, so this in-place migration is a
  choice to make, not a regression in the stages. Not changed here.

**Not run:** the S-00 Nov 2 `round_done` (3.4 s from process start) did not reproduce, so the background round is not
compared (see the S-16.1 entry above for the same-day reference).

## Refactor S-16.3: LTO and codegen units, by instruction count (2026-11-04)

The callgrind benches at HEAD (`341e82f`, WSL, `tools/profiling/wsl-bench.sh`'s steps), under two release-profile
variants set by Cargo's environment (`CARGO_PROFILE_RELEASE_LTO`, `CARGO_PROFILE_RELEASE_CODEGEN_UNITS`), so the tree's
`Cargo.toml` is unchanged. The default run reproduces `docs/perf/baseline.json` exactly.

| Bench | default (= baseline) | `lto = "thin"` | `codegen-units = 1` |
|---|---|---|---|
| `per_file::analyze_jpeg.sample` | 60,604,034 | −0.01% | −0.48% |
| `per_file::thumb_jpeg.h256` | 52,045,748 | −0.00% | −0.73% |
| `list.all_name` | 32,349,515 | −0.00% | −0.44% |
| `list.all_taken` | 46,738,460 | −0.00% | −0.24% |
| `list.all_unrated` | 32,574,673 | −0.00% | −0.43% |
| `list.folder_recursive` | 34,261,837 | −0.00% | −0.41% |
| `list.folder_flat` | 730,480 | −0.28% | −0.20% |
| `tree.all` | 3,137,101 | −4.05% | +1.29% |
| `facets.all` | 104,873,332 | −0.00% | −0.00% |
| `pending_import.empty` | 115,391 | −0.16% | −0.02% |
| `pending_thumbs.at_height` | 46,328,133 | −0.00% | −0.00% |

- **Not adopted.** Neither variant moves the hot paths by more than about 0.7%, and the 4% on `tree.all` is about
  127,000 instructions, once per start. The benches are dominated by SQLite, libjpeg-turbo and FFmpeg, which are C code
  that Rust LTO does not reach; the cross-crate calls are not where these counts go. The longer build (thin LTO adds a
  link pass over 24 crates) buys nothing measurable here.
- **Not measured:** wall-clock time and the startup stages for either variant (both would need a Windows release build
  each), and the build time with each. The `#[inline]` audit of cross-crate calls is not done: the counts do not point at
  them.
- **Limit:** instruction counts, not time. A variant that reduced cache misses without changing the instruction count
  would not show here.

## Pending

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
- Exact all-pairs k-NN (design conclusion 8): write it streaming the fp16 base from the NVMe, with
  checkpoints. Semantic search's index (So400m, 1152-d): FAISS IVF on real vectors once they exist.
- Full-set runs (on hold).
- Video player: Qt Multimedia plays videos now, with hardware decoding turned off on a GPU without AV1 decoding
  (a stopgap, "Videos and animations"). Choose between our own engine on our FFmpeg 9 and libmpv with its D3D11
  render API ([VIDEO_PLAYER.md](VIDEO_PLAYER.md)).
- Yandex: whether the browser shows all 18 other sizes or about 5 is not measured; only one photo was tested.
