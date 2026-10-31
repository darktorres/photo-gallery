# Profiling log

Measuring CPU cost by **instructions executed**, not time. Both tools instrument every instruction, so the counts are
exact and do not depend on clock speed or machine load. Intel SDE runs the real Windows binary. Callgrind, through
the gungraun benches, runs a Linux build under WSL2 and adds call graphs, cache simulation and run-to-run diffs.
Newest entries at the top of each section. Every number states the binary, the command and the data it was
measured on.

---

## Tool: Intel SDE (2026-10-31)

**Why SDE:** we wanted exact instruction counts on Windows, from instrumentation rather than sampling. The options
considered:

| Tool | Gives | Verdict |
|---|---|---|
| **Intel SDE** (Pin-based) | Exact instructions per thread and per function, with call counts | **Chosen**: no build changes; reads our PDBs |
| DynamoRIO (`inscount`, `drcachesim`) | Totals; cache simulation (the cachegrind equivalent) | Keep in mind for cache questions |
| ETW PMC counting (`xperf -pmc InstructionRetired`) | Hardware instruction counts per thread, no slowdown | No per-function view; blocked when Hyper-V/VBS is on |
| `-C instrument-coverage` | Exact execution counts per code region | A proxy for loop counts, not instructions |
| WSL2 + callgrind / gungraun | The real thing | Measures the Linux build (allocator, I/O, TurboJPEG), not ours |

**Install:** SDE 10.13.1 (2026-07-28), `sde-external-10.13.1-2026-07-28-win.tar.xz` from
<https://www.intel.com/content/www/us/en/download/684897/intel-software-development-emulator.html>, SHA256
`74E626ED…F335F651`, which matches Intel's published hash. Extracted to `%LOCALAPPDATA%\Programs\sde` (not in the repo).

**Use:**

```sh
sde.exe -icount -- prog.exe args                # one total per thread, written to sde-icount.txt in the cwd
sde.exe -mix -omix out.txt -- prog.exe args     # per-function and per-block counts; see "FUNCTION TOTALS"
```

- **Symbols:** the release profile's `debug = "line-tables-only"` is enough for function names, including SQLite's,
  which is compiled into the exe.
- **Wrong names in DLLs without PDBs:** they get the nearest export's name, e.g. `av_parser_close` in `avcodec-6.dll`.
  Treat those rows as "somewhere in that DLL".
- **Self counts only:** each function's count excludes its callees. There is no call graph and no kcachegrind view.
- **Threads:** counts are per thread. For pipelines, add the threads up; the split between them varies from run to
  run.
- **Whole run:** the totals include startup, opening the catalog and exit. To measure one region, SDE takes markers
  compiled into the code (`-start_ssc_mark` / `-stop_ssc_mark`). We don't have these yet (see *Open*).
- **Slowdown:** about 10–100×, roughly valgrind's.

## Tool: WSL2 + callgrind / gungraun (2026-10-31)

For repeatable instruction counts per function (regression benches), with callgrind's call graph and its inclusive
costs, which SDE lacks. The catch: it measures the **Linux** build of `pg-core`. That build uses glibc's allocator and
FFmpeg 8 (Ubuntu's `libavcodec` 62.11; the Windows app ships FFmpeg with `avcodec-63`). libjpeg-turbo is built from
the same vendored source, but with GCC instead of MSVC.

**Setup** (a separate WSL2 distro, `Ubuntu-26.04` (26.04.1 LTS), kernel 6.18, 12 CPUs; used as root):

- Installed with `wsl --install -d Ubuntu-26.04 --no-launch` and first started with `-u root`, which skips creating
  a user. It is the default distro. `Ubuntu-24.04` was tried first, but it only has FFmpeg 6.1 and valgrind 3.22;
  it was unregistered on 2026-10-31.
- `tools/profiling/wsl-setup.sh` (idempotent) installs the rest: valgrind 3.26, the build tools, FFmpeg 8's
  headers, rustup (Rust 1.99.1, with clippy and rustfmt), and the `gungraun-runner` of the exact version in
  `Cargo.lock`.

**The bench** is `crates/core/benches/ir.rs` (gungraun 0.20, a Linux-only dev-dependency of `pg-core`). On other
systems the target is an empty `main`, so `cargo clippy --all-targets` still builds it on Windows. It measures
`analyze::analyze` and `thumbs::make(height 256)` on one file from the test set,
`dups2/amateur-alice-von-v-mbDfqe.jpg.mozjpeg.jpg` (126 KB, 1280×1017 baseline JPEG). The file is read from D:
through `/mnt/d` and stays out of the repo, since it's one of the library's photos. `PG_BENCH_JPEG` at build time
picks another file. Reading it and creating the decoder and encoder happen in gungraun's `setup`, outside the
measured function.

```powershell
wsl -d Ubuntu-26.04 -u root -- bash tools/profiling/wsl-setup.sh   # once, and after a gungraun update
wsl -d Ubuntu-26.04 -u root -- bash tools/profiling/wsl-bench.sh   # 3 s once built; compared with the previous run
```

`wsl-bench.sh` builds the **committed HEAD** in a clone on WSL's own disk (`~/photo-gallery`, reset on every run).
That's faster than `/mnt/c`, and the Windows tree's uncommitted work stays out. Arguments after it go to gungraun
(e.g. `--save-baseline=before`, then `--baseline=before`). Read the results with `cgtab.py`, in WSL:

```sh
O=~/photo-gallery/target/gungraun/pg-core/ir/per_file/thumb_jpeg.h256/callgrind.thumb_jpeg.h256.out
C=/mnt/c/Users/Torres/Documents/GitHub/photo-gallery/tools/profiling/cgtab.py
$C table $O                              # self, inclusive, calls per function
$C table $O --sort incl --match 'pg_core::'
$C table $O --by crate                   # self cost per crate, following inlined code to its source
$C calls $O 'precompute_coefficients'    # callers (<) and callees (>), with call counts
$C diff $O.old $O                        # per function: what the last change did
```

**What the Linux build needed** (found one failure at a time):

1. `libssl-dev`: `ureq`'s `native-tls` is OpenSSL on Linux.
2. A way around `.cargo/config.toml`'s `[env]`, which sets Windows paths for every build. `FFMPEG_DIR = D:/…` made
   `crates/core/build.rs` panic looking for the DLLs, and `LIBCLANG_PATH = C:/…/LLVM/bin` kept bindgen from finding
   libclang. Cargo has no per-target `[env]` and can't unset an entry, but the real environment wins over it. So
   `wsl-bench.sh` sets `LIBCLANG_PATH` to the newest `/usr/lib/llvm-*/lib`, and sets `FFMPEG_DIR` to a prefix that
   `wsl-setup.sh` makes (`~/.local/share/pg-bench/ffmpeg`, with `include` and `lib` linked to Ubuntu's multiarch
   directories). `FFMPEG_DIR=/usr` doesn't work, because the headers are in `/usr/include/x86_64-linux-gnu`.
3. `build.rs` copies the FFmpeg DLLs on Windows targets only.
4. `.gitattributes` keeps `*.sh` at LF: they run under WSL straight from the Windows checkout, where
   `core.autocrlf` would add `\r`.

**Noise between builds:** the code is deterministic, but code generation is not stable across unrelated changes. From
41eaa1c to e9d93b6 both benchmarks rose about 0.05% (29 k instructions), with no change to the code they run.
`cgtab.py diff` puts all of it in EXIF parsing: `kamadak-exif` reading through a `Chain` reader instead of a plain
`Cursor`. e9d93b6 added about 1,000 lines to `thumbs.rs`, which changes how rustc splits `pg-core` into codegen
units and so what gets inlined where. Treat changes under ~0.1% from unrelated commits as layout, not cost.

**`cgtab.py`** reads the raw callgrind format instead of scraping `callgrind_annotate`, whose columns shift with
`--show` and which splits each function into one row per source file. That row split made `analyze` look like 78%
of its own benchmark. The script:

- Handles compressed names (`(id) name` the first time, then `(id)`, with separate id spaces for fn/cfn, ob/cob and
  the file keys). Not handling them is what made a direct parse fail elsewhere.
- Gets exact call counts and per-edge inclusive costs from `calls=` lines.
- Groups by function (`--by fn`), crate, source file or object. Crate and file attribution follow `fi=`/`fe=`, so
  inlined code is counted where it came from. Inlined std code (`core_arch` SIMD intrinsics, iterators) goes to the
  crate of the function it was inlined into; without that, `fast_image_resize`'s AVX2 kernels show up as "std".
- `diff` merges by function name, because the object name carries a cargo build hash, and takes `--rename OLD=NEW`
  for renamed functions. Code without a symbol (named by its address, which moves between builds) becomes one row
  per object. `--tsv` prints every row.

Checked against gungraun and `callgrind_annotate` on both benchmarks: the totals match, the inclusive counts of
`box_1d`, `tj3Decompress8`, `tj3Compress8` and `precompute_coefficients` and the self count of `__sin_fma` match,
and a diff of two identical runs is empty. Like `callgrind_annotate`, it leaves out direct recursion from inclusive
cost, but indirect recursion (A → B → A) is still counted twice.

---

## Measurements

### JPEG decoders: libjpeg-turbo against five CPU decoders and nvJPEG (`eval/jpeg-decoders`, 2026-10-31)

**Verdict: keep libjpeg-turbo.**

- **Fewest instructions:** with its default or fast options, it uses the fewest instructions of any CPU decoder on
  every input except two, both FFmpeg's: the progressive file, and the phone photo at 1/4, by 1.5%.
- **Fastest scaled decode:** its DCT-scaled decode, which `analyze` uses, is the fastest decode on every file
  except one progressive file, where FFmpeg is faster.
- **The GPU frees the CPU only with GPU Huffman decoding:** nvJPEG as torchvision runs it (CPU Huffman) keeps one
  CPU core busy for as long as it decodes. Its `GPU_HYBRID` backend, in batches over 50, costs the CPU 0.02–0.6 ms
  per image and decodes 2–4× faster ("nvJPEG with Huffman decoding on the GPU" below).

Any change of decoder would also change the decoded pixels, and with them every stored `pixel_hash`, which
would mean a re-import.

**Inputs:** the main kinds in the library.

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

**Decoders.** All decode to packed RGB8 on one thread unless noted. "Scaled" is the DCT scale `analyze` asks
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

**Instructions per decode** (M; callgrind, Linux build). Most come from gungraun. libvips and jpeg-decoder do
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

**Wall time** (median ms; WSL, i7-8700, machine at 6–18% load). A run during a busy spell (100%: a VMware VM
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

**Output against the app's decode** (`wsl.sh compare`; mean |difference| per channel value, the largest, PSNR):

- **libvips:** identical at full size (libjpeg-turbo 2.1.5 decodes the same as 3.x). Its shrink-on-load rounds sizes
  down: 422×633 against 422×634.
- **zune-jpeg and jpeg-decoder:** within 4 levels at full size (50–73 dB).
- **turbo fast, FFmpeg and nvJPEG:** drift where there is subsampled chroma, because none of them upsample it
  smoothly. On the 4:2:0 phone photo all three are about 29 dB from the app's decode (mean 6.3–6.5, max 56–57). On
  4:4:4, nvJPEG matches (69 dB).
- **Scaled decodes:** they differ more between decoders (each does its reduced IDCT its own way). At 1/4 on the
  phone file, jpeg-decoder and FFmpeg are 28 dB from libjpeg-turbo.

**Where the differences come from** (`cgtab.py` on the gungraun outputs):

- **FFmpeg's decoder costs about what libjpeg-turbo does.** On `pin-base444-1mp`, libavcodec is 31.8 M against
  libjpeg-turbo's 31.0 M in total. The rest, 53 M (62%), is swscale's YUV 4:4:4 → RGB conversion.
- **FFmpeg wins on progressive files** because libjpeg-turbo spends half of a progressive decode in
  `decode_mcu_AC_refine`, 20.7 M of 42.3 M.
- **zune-jpeg loses on high-bitrate files in its Huffman decoder.** `decode_mcu_block` is 75% of the phone photo.
- **The fast libjpeg-turbo options save 3–11% of a full decode on subsampled files and nothing at the scales
  `analyze` uses.** Scaled IDCTs have no fast variant, and 4:4:4 has nothing to upsample. They cost 29 dB on 4:2:0.

**nvJPEG** (Windows, RTX 2060, machine at 10–17% load). The table gives ms per image, with the decoded pixels
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

**nvJPEG with Huffman decoding on the GPU** (`nvjpeg.py`, 2026-10-31; RTX 2060, nvJPEG 13.2.3, the DLL torchvision
ships, machine at ~25% load from a VMware VM). torchvision's `decode_jpeg` has no backend option, so `nvjpeg.py`
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
- **Throughput, not latency:** the GPU at about 820–880 small files/s (to host, batch 64) is likely less than all 12
  CPU threads decode at 1/2 (one thread: 200–270/s; not measured across threads). It adds decode capacity and frees the CPU for PDQ, ThumbHash and the resizes, but its GPU time competes
  with SSCD, which keeps this GPU busy at 410 img/s (FINDINGS "I/O").

**To rerun:**

- **CPU decoders**, from PowerShell in the repo: `wsl -d Ubuntu-26.04 -u root -- bash eval/jpeg-decoders/wsl.sh
  inputs check compare time bench marginal`. This needs `apt-get install libvips-dev` in the distro. The inputs
  are library photos, so they're copied to `~/jpeg-inputs` in WSL and never into the repo.
- **GPU:** `D:\PhotoGallery\venv-eval\Scripts\python.exe -I eval\jpeg-decoders\gpu.py FILE...` (torchvision), and
  `... eval\jpeg-decoders\nvjpeg.py [--interpolate] [--batches 1,64] FILE...` (the two nvJPEG backends).

### `analyze` and `thumbs::make` on one JPEG (gungraun, Linux build of HEAD 41eaa1c, the bench's 126 KB sample, 2026-10-31)

Two runs gave exactly the same counts (63,494,636 and 50,648,725 instructions, gungraun: "No change"), so a change of
even 0.1% shows. gungraun's estimated cycles, from callgrind's cache simulation, are 86.4 M and 68.7 M.

**`analyze::analyze` (the import's work per file): 63.5 M instructions**

| Stage (inclusive, `cgtab.py`) | Instructions | Share | Calls |
|---|---:|---:|---:|
| `analyze` itself | 63.5 M | 100% | 1 |
| JPEG decode (`Decoder::decode`; `tj3Decompress8` 21.2 M, Huffman `decode_mcu` alone 11.4 M) | 21.6 M | 34% | 1 |
| Resizes (`resize::resize`: `fit_within` for PDQ, and the ThumbHash input) | 19.3 M | 30% | 2 |
| ↳ Lanczos weights (`precompute_coefficients`, mostly `sin`) | 3.3 M | 5% | |
| PDQ, 8 dihedral hashes (`hash_rgb_dihedral`) | 16.6 M | 26% | 1 |
| ↳ `pdq::box_1d` | 13.6 M | 21% | 1,838 |
| ThumbHash (with its resize) | 10.0 M | 16% | 1 |
| `pixel_hash` (xxh3) | 0.77 M | 1.2% | 1 |
| `file_hash` (BLAKE3 of 126 KB) | 0.33 M | 0.5% | 1 |
| Metadata (rating, date taken) | 0.05 M | 0.1% | |

The ThumbHash row includes its own resize, which is also in the resize row.

**By crate**, self cost following inlined code to its source (`cgtab.py table --by crate`):

| Crate | `analyze` | `thumbs::make` |
|---|---:|---:|
| libjpeg-turbo | 32.9% | 60.3% |
| fast_image_resize | 23.9% | 30.5% |
| pg_core (PDQ, mostly) | 25.5% | 0.0% |
| thumbhash | 7.3% | — |
| libm (`sin`, the Lanczos weights) | 4.0% | 4.5% |
| libc (memset/memcpy, malloc) | 2.7% | 3.2% |
| std | 1.8% | 1.4% |
| xxhash_rust, blake3 | 1.7% | — |

**`thumbs::make`, height 256: 50.6 M instructions**

| Stage (inclusive) | Instructions | Share |
|---|---:|---:|
| JPEG decode (`tj3Decompress8`) | 28.5 M | 56% |
| Resize to 256 px high (Lanczos3) | 19.4 M | 38% |
| ↳ Lanczos weights (`precompute_coefficients`, mostly `sin`) | 3.2 M | 6.4% |
| JPEG encode (`tj3Compress8`, q85 4:2:0) | 2.4 M | 4.8% |

**Leads** (not acted on):

- **The thumbnail decode is at full scale.** `DECODE_MARGIN = 4` asks for ≥ 1024 px high, and the image is 1017
  high, so no DCT scaling is applied. That costs 28.5 M here against `analyze`'s 21.2 M. The margin is a quality decision
  (FINDINGS "Thumbnails"); this measures its price for images just under 4× the thumbnail height.
- **`pdq::box_1d` is 21% of `analyze`, and the cache misses are concentrated there.** It has 65% of the
  benchmark's L1 read misses and 84% of its L1 write misses. `dct16` makes 2 passes, and each runs it over every
  row and then over every column (`stride = width`). That gives 1,838 calls (2 × 919 rows plus columns of the PDQ input) at
  7.4 k instructions each. The column calls, with their strided access, are the likely source of the misses. A
  transpose, or processing columns in blocks, is the usual fix.
- **Lanczos weights are recomputed for every resize.** `precompute_coefficients` runs twice per resize (horizontal,
  then vertical), 1.6 M each. Of that, 2.8 M per resize is 14,286 `lanczos_filter` evaluations at about 190
  instructions each, mostly libm's `sin`. That's about 6% of a thumbnail. A cache keyed by (source size, target size)
  would only help when sizes repeat. This is glibc's `sin`; MSVC's CRT may cost differently.

### `gallery status` (release `gallery.exe` of 2026-10-31 15:41, pinterest catalog, 2026-10-31)

Catalog: 189,518 files, all analyzed, 5,216 PDQ pairs, no thumbnails. 18 s under SDE.

**1.05 G instructions on the main thread** (`sde -mix`):

| Function | Instructions | Share | Calls |
|---|---:|---:|---:|
| `sqlite3VdbeExec` | 796 M | 75.7% | 88 |
| `btreeParseCellPtr` | 71 M | 6.8% | 1.53 M |
| `sqlite3VdbeSerialGet` | 40 M | 3.9% | 2.30 M |
| `sqlite3BtreeTableMoveto` | 27 M | 2.5% | 579 k |
| `getCellInfo` | 25 M | 2.4% | 2.86 M |
| `sqlite3BtreeNext` | 9.2 M | 0.9% | 617 k |
| `countStep` | 6.9 M | 0.7% | 384,252 |
| `RtlAllocateHeap` (ntdll) | 5.3 M | 0.5% | 18 k |

- About 96% of the instructions are SQLite running the 8 `count(*)` queries in `Catalog::status`
  (`crates/core/src/catalog.rs`). Startup and the rest of the CLI are a few percent.
- **The counts are exact:** `countStep` ran 384,252 times = 189,518 files + 189,518 analyzed + 5,216 PDQ pairs, the
  rows those queries match. The `missing = 1`, `failed` and `file_writes` counts match no rows but still scan.
- The 579 k `sqlite3BtreeTableMoveto` calls are rowid lookups. The two `analysis ⋈ files` joins explain about 379 k
  (one lookup per `analysis` row each). The other ~200 k are probably the `pending` query; not checked yet.
- Not a problem today (`status` is a diagnostic command). If it ever matters: one pass with `sum(missing = 0)`-style
  aggregates instead of separate scans, or an index on `files(missing)`.

---

## Open

- **What to measure next:** the hot paths of the app, i.e. `thumbs` on a small folder (decode, resize, encode) and
  `import`. Use a separate `--catalog-dir`, never the real catalog.
- **Region markers:** a small inline-asm `ssc_mark(id)` helper (`mov ebx, id` followed by the SSC marker bytes `64 67 90`), so
  `-start_ssc_mark` / `-stop_ssc_mark` can count one stage without startup.
- **Repeatability under SDE:** check that two runs of the same command give the same count (single-threaded paths
  should be identical). Under callgrind/gungraun they are (see above).
- **More bench inputs:** a PNG, a WebP, a large (12 MP) JPEG, and one that decodes with DCT scaling.
- **Regression thresholds:** gungraun can fail a run over a limit (`--callgrind-limits`). Given the ~0.05% layout
  noise between unrelated commits, 1% would be a starting point.
