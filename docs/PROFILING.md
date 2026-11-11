# Profiling log

Measuring CPU cost by **instructions executed**, not time. Both tools instrument every instruction, so the counts are
exact and do not depend on clock speed or machine load. Intel SDE runs the real Windows binary. Callgrind, through
the gungraun benches, runs a Linux build in the pg-dev container (docker/) and adds call graphs, cache simulation and run-to-run diffs.
Newest entries at the top of each section. Every number states the binary, the command and the data it was
measured on.

---

## Tool: dhat heap profiles, `--features dhat-heap` (2026-11-01)

Every Rust heap allocation of a `gallery` run, by the backtrace that made it: how many, how big, how long they live,
what was live at the global peak and what is left at exit. `dhat` is a patched dhat-rs 0.3.3 (vendor/README.md
"dhat") that takes whole backtraces in seconds where upstream takes minutes. Qt's, FFmpeg's and libjpeg-turbo's
allocations go to the C runtime's heap, so they are not in it.

```powershell
cargo build --profile dhat --features dhat-heap -p pg-gallery --bin gallery   # target/dhat/gallery.exe; ~9 min clean
$env:PATH = "C:\Qt\6.12.0\msvc2022_64\bin;$env:PATH"   # Qt's DLLs: target/dhat has only FFmpeg's
$env:PG_CATALOG_DIR = 'D:\…\scratch_dhat\catalog'; $env:PG_DHAT_UNTRIMMED = 1
target/dhat/gallery.exe import --workers 1        # writes dhat-heap.json in the current directory
```

- **Backtraces:** cut to 10 frames, plus dhat's and `alloc`'s own at the top, unless `PG_DHAT_UNTRIMMED` is set.
  Untrimmed, each stack starts with dhat's frames (`backtrace::trace`, `dhat::new_backtrace_inner`, `__rust_alloc`),
  as upstream's does, and ends at the thread's start. On Windows the patch's frame-pointer walk doesn't apply
  (Linux and macOS only), so `-C force-frame-pointers=yes` is not needed: it unwinds with `RtlVirtualUnwind`, outside
  the lock.
- **The profile is written when `main` returns.** Runs that end in `process::exit` (a command's error, the app's
  `fatal`) write none. The app writes it when its window is closed.
- **One worker** (`--workers 1`) makes an import's or a thumbnail pass's counts the same from run to run.
- **Reading:** `pps` are the program points: `tb`/`tbk` total bytes and blocks, `gb`/`gbk` at the global peak,
  `eb`/`ebk` at exit, `fs` indices into `ftbl`. The viewer (`dh_view.html`,
  <https://nnethercote.github.io/dh_view/dh_view.html>) opens small ones; parse big ones in a script. dhat counts a
  `realloc` as a new block of the new size, charged to the site that first allocated it. Identical code folding can
  name the wrong type in a frame (`RawVec<Unrelated>::grow_one`): go by the calling frame. Charge an allocation to
  the innermost frame of ours that isn't a container or conversion helper: that is the code that decides to allocate.
- **Checked** on `gallery import` of 305 small JPEGs (`D:\PhotoGallery\borders_sample\_preview`, 5.3 MB), 1.3 s:
  425.6 MB in 167,454 blocks, 33.2 MB at the peak, 1 kB at exit. Trimmed and untrimmed give the same totals.
  Untrimmed, the stacks are up to 55 frames deep and reach the thread's start; the largest site was `pdq::dct16`'s
  zeroed buffer (105.7 MB in 305 blocks, `pdq.rs:117` then; `crates/analysis/src/pdq.rs` now keeps the hasher's
  buffers between images).

## Tool: Pin call graph, `tools/profiling/callgraph` (2026-10-31)

Callgrind's view of the **Windows app**: instructions per function, self and inclusive, with call counts and the call
graph, in callgrind's output format (one file per thread), so `cgtab.py` and KCachegrind read it. It is a Pin tool
built against the Pin 4.3 kit inside SDE, since SDE's `-mix` has self counts only, and callgrind can't run the app.

```powershell
python tools/profiling/callgraph/build.py      # target/pin/callgraph.dll, a few seconds (Windows)
just deploy release pdb                          # once: Qt's PDBs beside its DLLs (Qt's "Debug Information Files")
$env:PG_CATALOG_DIR = 'D:\…\scratch\catalog'; $env:PG_PREFS = 'D:\…\scratch\gallery.json'
python tools/profiling/callgraph/profile.py --out D:\…\profiles\ready   # to pg_stage_ready; --stop/--start take any stage
python tools/profiling/cgtab.py table D:\…\profiles\ready.<pid>-<thread> --sort incl
```

- **Stages:** `crates/trace/src/lib.rs` marks the start's stages with functions of plain C names (`pg_stage_main` …
  `pg_stage_ready`). `-stop STAGE` stops every thread when one enters it, writes the profile and ends the process, so
  nothing after it is counted. SDE's `-control stop:…` didn't do that: threads started later kept counting. `-start
  STAGE` zeroes the counts. With `PG_TRACE=1` the app prints each stage's milliseconds since process start.
- **Threads** are named from `SetThreadDescription` (Rust's and Qt's thread names) in `PREFIX.<pid>.threads`, with
  each thread's total and every image loaded.
- **Symbols:** Pin reads a PDB only from beside its DLL or from where it was built; it ignores `_NT_SYMBOL_PATH` and
  DIA's registry path. Where it has one (our exe, Qt with `just deploy release pdb`) a function is Pin's routine.
  Elsewhere Pin's routines are the exports, each spanning to the next (SDE's wrong names), so the tool names a
  function by the address it was entered at, `ntdll.dll+0x1fa14`. `symbolize.py` (run by `profile.py`) then names
  those with dbghelp from Microsoft's symbol server, cached in `D:\symbols`, turns the positions (instruction
  addresses) into source lines, and demangles Rust's v0 names with `llvm-cxxfilt`. It needs the Windows SDK's x64
  debugging tools. Windows' public PDBs have no lines (`???`), and NVIDIA's driver has no symbols (nearest export).
- **No symbol lookups in the app:** the tool first asked Pin for source lines (`PIN_GetSourceLocation`, a request to
  Pin's `pind` process), while instrumenting and then at the stop. Both times the gallery waited forever on one.
- **Model:** a shadow stack per thread, following call and ret. Frames that end without a ret (longjmp, exception
  unwinding, kernel callbacks) are dropped once the stack pointer is above them. Entering another function at the
  frame's entry stack pointer is a tail call (the frame is replaced, as callgrind does), and so is an indirect jump
  there. That is how Windows' control-flow-guard thunks (`guard_dispatch_icall_nop`, `jmp rax`) reach their target;
  before that rule, a thunk's frame took in everything its DLL called through it. A jump table in a frameless
  function looks the same and shows as a call to `Function+0x…`. Indirect recursion is counted twice in inclusive
  cost, as with callgrind (`QQmlObjectCreator::finalize` shows 119%).
- **Checked** on a small C program (`mid` 1,000 calls, `leaf` 2,010 including 10 tail calls, recursion cut by
  `longjmp`): counts and inclusive costs as expected.
- **Cost:** the start to `ready` takes about 7 minutes under Pin, and `symbolize.py` a few more.

## Tool: Intel SDE (2026-10-31)

**Why SDE:** we wanted exact instruction counts on Windows, from instrumentation rather than sampling. The options
considered:

| Tool | Gives | Verdict |
|---|---|---|
| **Intel SDE** (Pin-based) | Exact instructions per thread and per function, with call counts | **Chosen**: no build changes; reads our PDBs |
| DynamoRIO (`inscount`, `drcachesim`) | Totals; cache simulation (the cachegrind equivalent) | Keep in mind for cache questions |
| ETW PMC counting (`xperf -pmc InstructionRetired`) | Hardware instruction counts per thread, no slowdown | No per-function view; blocked when Hyper-V/VBS is on |
| `-C instrument-coverage` | Exact execution counts per code region | A proxy for loop counts, not instructions |
| callgrind / gungraun, in the pg-dev container | The real thing | Measures the Linux build (allocator, I/O, TurboJPEG), not ours |

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
- **Self counts only:** each function's count excludes its callees. There is no call graph and no kcachegrind view:
  the Pin call graph above has both.
- **Threads:** counts are per thread. For pipelines, add the threads up; the split between them varies from run to
  run.
- **Whole run:** the totals include startup, opening the catalog and exit. `-control stop:enter_func:pg_stage_ready:bcast`
  stops at a startup stage (`crates/trace/src/lib.rs`; Rust's `::` paths can't be named there, hence the C names),
  but threads started after the stop still count.
- **Slowdown:** about 10–100×, roughly valgrind's.

## Tool: callgrind / gungraun (2026-10-31; run in WSL2 then, in the pg-dev container since 2026-11-09)

For repeatable instruction counts per function (regression benches), with callgrind's call graph and its inclusive
costs, which SDE lacks. The catch: it measures the **Linux** build of the per-file code (`pg-analysis`, `pg-render`). That build uses glibc's allocator and
FFmpeg 8 (Ubuntu's `libavcodec` 62.11; the Windows app ships FFmpeg with `avcodec-63`). libjpeg-turbo is built from
the same vendored source, but with GCC instead of MSVC.

**Setup as it was on 2026-10-31** (a separate WSL2 distro, `Ubuntu-26.04` (26.04.1 LTS), kernel 6.18, 12 CPUs; used as root;
superseded: the environment is now the pg-dev image, `docker/Dockerfile`, which installs what is listed here):

- Installed with `wsl --install -d Ubuntu-26.04 --no-launch` and first started with `-u root`, which skips creating
  a user. It is the default distro. `Ubuntu-24.04` was tried first, but it only has FFmpeg 6.1 and valgrind 3.22;
  it was unregistered on 2026-10-31.
- `tools/profiling/wsl-setup.sh` (idempotent; its bench part was removed on 2026-11-09, and the CI part is now `tools/profiling/ci_setup.py`) installed the rest: valgrind 3.26, the build tools, FFmpeg 8's
  headers, rustup (the toolchain that `rust-toolchain.toml` names, with clippy and rustfmt), and the `gungraun-runner` of the exact version in
  `Cargo.lock`.

**The bench** is `crates/bench/benches/ir.rs` (gungraun 0.20, a Linux-only dependency of `pg-bench`). On other
systems the target's `main` only prints where to run them, so `cargo clippy --all-targets` still builds it on Windows. It measures
`Analyzer::analyze` and `Maker::make` (height 256) on one synthetic photo, `crates/bench/benches/ir/sample.jpg`
(126 KB, 1280×1017 baseline JPEG, quality 80, 4:2:0), which `tools/profiling/make_bench_jpeg.py` generates: 1/f noise,
the spectrum of a natural photo but few hard edges. It's committed, so CI runs the same benches. Its numbers are not
comparable with the ones from the real photo that the benches used before (FINDINGS records those). `PG_BENCH_JPEG`
at build time picks another file. Reading it and creating the `Analyzer` and the `Maker` happen in gungraun's `setup`, outside
the measured function. Their buffers start empty, so each run measures a worker's first file. The bench also has
catalog benches (`crates/bench/benches/ir/catalog.rs`: the grid's list, the tree and the background job, on a
synthetic catalog made by `tools/profiling/bench_data.py`); `tools/profiling/bench.py` (`just linux-bench`) takes a bench name to run only those.

```powershell
just linux-build                          # once, and after a Dockerfile or Cargo.lock (gungraun) change
just perf                                 # the committed HEAD's benches, compared with docs/perf/baseline.json
just linux-bench --save-baseline=before   # gungraun's own options, without the gate
```

`just perf` benches the **committed HEAD**: the launcher exports it (`git archive`) into the container, so the Windows tree's
uncommitted work stays out and the build runs on the container's own disk. Arguments of `just linux-bench` go to gungraun
(e.g. `--save-baseline=before`, then `--baseline=before`). A full run's `.out` files are also archived, zstd-compressed, to
`docs/perf/callgrind/<sha>.tar.zst` (the benched HEAD's short sha, committed). A run given a bench name (a filter) is partial and
is not archived; options such as `--save-baseline` do not make a run partial. An option's value goes after `=`:
`--save-baseline before` is refused, because `before` could be a bench name.

**The large dataset** (2026-11-11, for the notifications' fix plan, [NOTIFICATIONS_FIX_PLAN.md](NOTIFICATIONS_FIX_PLAN.md)
F0). Three catalog benches measure what the inbox and the scan do at scale:

- `scan_walk` (`first`, `again`): `pg_scan::scan_with` over a synthetic tree (`pg-scan`'s `bench` feature,
  `pg_scan::synthetic`: 100 folders of 100 files and 4 folders it can't open, so the folder ledger is written) into a
  catalog in memory. No file system under it and no bench data, so it runs wherever the benches do. `again` walks the
  same tree over the catalog that has it, as a rescan does.
- `import_flush`: one import batch (500 files, one in five failing) stored in one transaction, on the large layout, so
  the import ledger's triggers run over its 50,000 failures.
- `inbox_refresh` (`closed`, `open`): `Catalog::ledger_entries` on the large layout, with the inbox closed (each ledger's
  cheap state) and open (the counts too).

The large layout is made like the default one, on the Windows side, under a name ending in `-large`
(`v6-windows-rules-large`, the bench's `LARGE_LAYOUT`): `synthetic_library.py --profile large` (400 folders of 500
files, a quarter of them broken JPEGs that the import fails, small pictures so the library is a few GB), the CLI's scan,
import and thumbnails over it, then `bench_data.py v6-windows-rules-large <catalog folder> <bench-data>`; the commands are
in `bench_data.py`'s docstring. For a `-large` layout `bench_data.py` also seeds the failure ledger to 100,000 rows
(`--ledger-rows`; the import's own rows first, then thumbnails' and borders' rows), when the copy's schema has
`file_failures.hidden`; an older schema is left unseeded, with a note.

**Budgets** (`docs/perf/budgets.json`): a change that moves some counts on purpose is gated once by
`just perf --budgets` instead of `--update`. Each budget names a bench (`scan-walk` matches gungraun's `scan_walk.<id>`;
`-` and `_` are the same) and the range its change must fall in, as fractions of the baseline (`min`, `max`, either
optional; `"max": -0.90` is "must fall by 90% or more"). The benches with no budget keep the ±1% tolerance. A budget no
bench matches fails the gate (a renamed bench isn't passed unseen), a bench that misses its budget fails, and
`--budgets` is refused with `--update`: a miss is not re-baselined. The budgets of the fix plan's F1:

| Bench | Budget | Why |
|---|---|---|
| `scan-walk` | ≤ +10% | batching and the scan lease |
| `import-flush` | ≤ +15% | the analysis triggers that keep the import ledger |
| `inbox-refresh` | ≤ −90% | the ledger's cheap state on the partial index instead of the counts |

What callgrind can't see (how long a write lock is held, an I/O wait) is measured by the scan crate's wall-clock tests
(each write transaction of a 200k-file synthetic walk under a stated bound), not by a bench.

Unpack an archive with
`tar --zstd -xf docs/perf/callgrind/<sha>.tar.zst -C some/dir` (the container has zstd: `docker/apt-packages-bench.txt`). Read the results with `cgtab.py`, in the container (`just linux-shell`):

```sh
O=/bench/target/gungraun/pg-bench/ir/per_file/thumb_jpeg.h256/callgrind.thumb_jpeg.h256.out
C=/work/tools/profiling/cgtab.py
$C table $O                              # self, inclusive, calls per function
$C table $O --sort incl --match 'pg_analysis::'
$C table $O --by crate                   # self cost per crate, following inlined code to its source
$C calls $O 'precompute_coefficients'    # callers (<) and callees (>), with call counts
$C diff $O.old $O                        # per function: what the last change did
```

**What the Linux build needed** (found one failure at a time):

1. `libssl-dev`: `ureq`'s `native-tls` is OpenSSL on Linux.
2. The build environment. `.cargo/config.toml` no longer sets Windows paths for every build (it had an `[env]` entry for
   `FFMPEG_DIR`, which made the Linux build look for Windows DLLs). `crates/video/build.rs` now finds FFmpeg's headers on Linux
   through pkg-config (the development packages in `docker/apt-packages.txt`), and on Windows in `.deps/ffmpeg`. `LIBCLANG_PATH`
   still has to be set on Linux, because libclang's folder depends on the Ubuntu release: the container sets it
   (a link, `/opt/libclang`, that the Dockerfile makes) to the newest `/usr/lib/llvm-*/lib`, and the CI workflow does the same for the runner.
3. `build.rs` copies the FFmpeg DLLs on Windows targets only.
4. `.gitattributes` keeps `*.sh` at LF: they run in the container straight from the Windows checkout, where
   `core.autocrlf` would add `\r`.

**Noise between builds:** the code is deterministic, but code generation is not stable across unrelated changes. From
41eaa1c to e9d93b6 both benchmarks rose about 0.05% (29 k instructions), with no change to the code they run.
`cgtab.py diff` puts all of it in EXIF parsing: `kamadak-exif` reading through a `Chain` reader instead of a plain
`Cursor`. e9d93b6 added about 1,000 lines to `thumbs.rs` (now `crates/thumbcache/src/lib.rs`), which changes how rustc splits `pg-core` (since split into the per-concern crates) into codegen
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

The start after the optimizations, drilled thread by thread: [START_PROFILE.md](START_PROFILE.md) (2026-11-01).

Every heap allocation of a scan, an import and the borders and thumbnail passes, site by site (dhat), before and
after three rounds of fixes: [HEAP_PROFILE.md](HEAP_PROFILE.md) (2026-11-01, 2026-11-02).

Every heap allocation of the app's start on the default catalog (dhat), before and after:
[HEAP_PROFILE_START.md](HEAP_PROFILE_START.md) (2026-11-01, 2026-11-02).

### Wall-clock library scan, WSL (`tools/profiling/scan-timing.sh`, 2026-11-07)

Not gated: wall-clock time on this machine, not instructions, so it is a ledger entry, not a `perf` result.

- **Binary:** `gallery` release build (`cargo build --release -p pg-gallery --bin gallery`, WSL Ubuntu-26.04, Qt 6.12
  for Linux, nightly-2026-11-06), from the tree at `262004b`. The crates are the same as `22ae0ff`: `262004b` changed
  no crate.
- **Library:** `tools/profiling/synthetic_library.py --folders 100 --per-folder 100 --seed 1`: 10,000 files (mostly JPEG,
  some PNG, GIF and WebP), on WSL's ext4 disk (`/dev/sdf`), not tmpfs. Renumbered in place, which the scan pairs
  (the same folder, size, mtime and identity under a new name).
- **Command:** `scan-timing.sh GALLERY LIBRARY WORK [--drop-caches]`, run as root. Each scan is into a fresh catalog;
  `first` scans the library from nothing, `unchanged` rescans it, and `renumbered` rescans after every file's number
  moved to the other range (0 to 999 and back).
- **Machine:** 12 cores, 31 GB RAM, nothing else running. Times are wall clock, one run per line unless noted.

| Caches | Run | first | unchanged | renumbered |
|---|---|---|---|---|
| as they are | 1 | 1.672 s | 0.227 s | 0.478 s |
| as they are | 2 | 1.647 s | 0.485 s | 0.397 s |
| dropped before each scan | 1 | 4.844 s | 0.888 s | 1.283 s |

- **What the renumbered run did:** the catalog has 10,000 file rows after all three scans, and none is missing (checked
  by reading the catalog), so the renamed files were paired with their rows. Its scan reports 10,000 new or changed,
  which counts the renames, and 0 missing.
- **tmpfs under a scanned root** (WSL, the S-17.7 manual check): a tmpfs folder of 20 files inside a library with 20
  plain files (device numbers 178 and 2128). Two scans; 40 rows and 0 missing after each. No row flipped.
- **The cold VM** (the first run after `wsl --shutdown`, same library, same binary): first 1.353 s, unchanged 0.129 s,
  renumbered 0.245 s. The VM's own cache was empty at the restart; this run did not drop the caches.
- **Device change across a restart** (the S-17.7 end-to-end check, in WSL, with the HEAD binary): a 40-file library on
  the Windows drive (drvfs), scanned, one rating set to 4, and imported (40 analysis rows). Across `wsl --shutdown` the
  library's device number changed (201 to 145), and the rescan found all 40 rows under a new identity: same ids and
  names, none missing, the rating and the analysis kept. A second restart changed it again (145 to 74), with the same
  result. Not done here: the plan's first step, a catalog built with the older binary; this check used HEAD's binary
  for both scans.
- **Windows** (the same script, under Git Bash with the Windows Python; `gallery.exe` release build of the working tree,
  whose crates are HEAD's, on NTFS, the 10,000-file library on `D:`): first 1.316 s, unchanged 0.146 s, renumbered
  0.322 s. The renumbered scan paired every row: 10,000 rows, none missing.
- **The `just perf` gate** (see REFACTOR_PLAN, S-16): it failed before it measured until the bench data moved to the `v5` layout, and the baseline was re-measured in `15ca4e1`. It has run in the `pg-dev` container since 2026-11-09, and the baseline was last re-measured at `e7576f0` (MODULARITY_PLAN.md, Validation 4).

### The start's stages before and after the two heap plans (release, `PG_TRACE=1`, offscreen, 2026-11-02)

`gallery-prev.exe` (2e1b6ca, before both plans) against `gallery.exe` (1d3685b), on the default catalog: 189,518 files,
"All folders" by name, file 8838 selected, nothing to do. Each run was `--wait=2 --screenshot=…`, with
`QT_QPA_PLATFORM=offscreen`. The two builds alternated, one warm-up round and then five measured. The medians are in
ms from `pg_stage_main`. The previous build's process start to `main` was bimodal: 19–20 ms in 2 runs, 122–126 ms
in 3. That is before any of our code runs, and the new build's was 18–19 ms in all 5. So the stages are measured from
`main`.

| Stage, from `main` | 2e1b6ca | 1d3685b |
|---|---:|---:|
| `init_done` | 17.3 | 16.9 |
| `qml_load` | 32.6 | 32.1 |
| `qml_loaded` | 205.8 | 206.0 |
| `tree_shown` | 273.3 | 273.6 |
| `first_frame` | 289.6 | 289.2 |
| `list_shown` | 601.1 (587–672) | 559.5 (548–571) |
| `ready` | 601.2 (587–672) | 642.0 (635–668) |
| `round_done` (the background job's first round) | 651.5 (636–722) | 645.3 (638–671) |

- **The list is shown 42 ms sooner.** Reading and laying out 189,518 entries no longer grows the list's `Vec`
  (29.4 MB copied before) or builds an aspects `Vec`. The reset no longer makes the top rows' cells, which the view
  then left (54714b1).
- **`ready` is 41 ms later, but it now marks more.** Since 54714b1 the view is placed on the selected file before
  any cell is made, so that file's rows are made between `list_shown` and `ready` (`gridLoaded`,
  `positionViewAtIndex`). Before, `ready` came with the list, while the top rows' cells were made, and the selected
  file's rows were made after it. The end of the first round, which follows both, is the same (651 → 645 ms).
- **The heap work doesn't show in the start's time.** What it removed (16,000 blocks, 26 MB) cost a few ms. The
  start is Qt's QML loading (174 ms) and the list's read (270 ms): the call graph below.

### The start, to `ready` (Pin call graph, release `gallery.exe` with the trace stages, scratch copy of the pinterest catalog, 2026-10-31)

Catalog: 189,518 files, 690 folders; the session folder is "All folders", so the grid lists every file.
**15.15 G instructions in 85 threads** from process start to `pg_stage_ready`. SDE's 17.4–18.1 G included threads
that kept counting after it (a 0.96 G SQLite thread: the library's first import check, which starts at `ready`).

| Thread | Instructions | What |
|---|---:|---|
| catalog reader | 10.90 G | the grid's `browse::list_files` (now `list_grid`, `crates/catalog/src/browse/read/grid.rs`) 10.25 G; the tree and the rest 0.65 G |
| main | 1.24 G | QML creation, fonts, a Direct3D 9 probe |
| QSGRenderThread | 1.18 G | the Direct3D 11 device |
| video decoding | 0.93 G | `choose_video_decoding`'s own Direct3D 11 device (the AV1 probe) |
| an NVIDIA driver thread | 0.32 G | driver internals |
| QQmlThread | 0.30 G | QML type loading |

- **The name sort, about 7.0 G (46%).** In `list_files`, SQLite's sorter (`vdbeSorterSort`) is 7.59 G, nearly all
  of it `browse::path_collation` (gone: the list is read in `files_order`): 1.59 M comparisons at about 4,400 instructions each. Unicode lowercasing
  (`FlatMap<Chars, ToLowercase>`, `to_lower`, `ToCasefold`) is 5.67 G, and rusqlite's `String::from_utf8_lossy` of
  both strings on every comparison 1.24 G. The rest of SQLite's work on the list is about 2.6 G.
- **The GPU driver verifying its signature, 2.0 G (13%), in 11 scans.** Each Direct3D device creation reaches NVIDIA's
  loader (`nvldumdx!OpenAdapter`), which checks its DLL's signature, and Windows scans the whole file
  (`imagehlp!RabinKarpFindPatternInBuffer`, about 182 M instructions a scan). Main: 3 scans, 0.55 G, from Qt's
  window creation (`WindowCreationData::create` → `calcPosition` → `GpuDescription::detect` → `Direct3DCreate9`).
  Render thread: 4 scans, 0.73 G (`QRhiD3D11::create`). Video probe: 4 scans, 0.73 G. This is SDE's "imagehlp"
  row, and the GPU opened three times.
- **Text and fonts on the main thread, about 0.3 G:** DirectWrite reading glyph coverage and the character map
  (0.23 G self), and Qt loading kerning pairs.

**Leads** (not acted on): an ASCII fast path in `path_collation` that compares bytes until a non-ASCII byte, and no
per-comparison UTF-8 conversion; caching the video probe's result per adapter and driver version; finding why Qt
creates a Direct3D 9 object during window creation.

### JPEG decoders (`eval/jpeg-decoders`, 2026-10-31)

Moved to [JPEG_DECODERS.md](JPEG_DECODERS.md): libjpeg-turbo against five CPU decoders, nvJPEG's two backends,
NVDEC, restart markers, lossless baseline conversion, and the decoders researched but not measured. Verdict:
keep libjpeg-turbo, and convert progressive files to baseline (2.6× fewer instructions, identical pixels).

### `analyze` and `thumbs::make` on one JPEG (gungraun, Linux build of HEAD 41eaa1c, the bench's 126 KB sample, 2026-10-31)

The functions are named as at that commit: `analyze::analyze` is `Analyzer::analyze` (`crates/analysis/src/analyze.rs`)
and `thumbs::make` is `Maker::make` (`crates/render/src/maker.rs`).

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
| pg_analysis (PDQ, mostly) | 25.5% | 0.0% |
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

- **The thumbnail decode is at full scale.** `DECODE_MARGIN = 4` (`crates/render/src/maker.rs`) asks for ≥ 1024 px high, and the image is 1017
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
  (`crates/catalog/src/lib.rs`). Each count now has its own index where one exists
  (`files_present`, `files_to_import`, `analysis_failed`), so this profile no longer describes it. Startup and the rest of the CLI are a few percent.
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
- **Region markers** beyond the start: the start's stages are functions (`crates/trace/src/lib.rs`) that both SDE
  and the Pin call graph stop at. Other regions (a scan, a thumbnail pass) would get stages the same way.
- **Repeatability under SDE:** check that two runs of the same command give the same count (single-threaded paths
  should be identical). Under callgrind/gungraun they are (see above).
- **More bench inputs:** a PNG, a WebP, a large (12 MP) JPEG, and one that decodes with DCT scaling.
- **Regression thresholds:** gungraun can fail a run over a limit (`--callgrind-limits`). Given the ~0.05% layout
  noise between unrelated commits, 1% would be a starting point.
