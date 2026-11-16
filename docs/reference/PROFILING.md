# Profiling

How the app's CPU cost is measured, and what the measurements say. Costs are counted in **instructions executed**, not
time: the tools below instrument every instruction, so the counts are exact and do not depend on clock speed or
machine load. Every number states the binary, the command and the data it was measured on. The open items and the
leads that were not acted on are in [PROFILING_PLAN.md](../plans/PROFILING_PLAN.md).

## Tools

| Tool | Gives | Runs on | Use |
|---|---|---|---|
| dhat (`--features dhat-heap`) | Every Rust heap allocation by backtrace: count, size, lifetime, what was live at the peak and at exit | Windows | Heap questions. Qt's, FFmpeg's and libjpeg-turbo's allocations are not in it |
| Intel SDE | Exact instructions per thread and per function, with call counts | Windows | **Chosen** for CPU on Windows: no build changes, it reads our PDBs. No call graph |
| Pin call graph (`tools/profiling/callgraph`) | SDE's counts plus the call graph and inclusive costs, in callgrind's format | Windows | Call graphs of the Windows app |
| callgrind / gungraun | Per-function counts, call graph, cache simulation, run-to-run diffs | pg-dev container (`docker/`) | Regression benches. It measures the Linux build of the per-file code, not ours |
| DynamoRIO (`inscount`, `drcachesim`) | Totals; cache simulation | Windows | Considered for cache questions; not used |
| ETW PMC counting and sampling (WPR with a `HardwareCounter` profile) | Hardware counts per thread (instructions, cycles, mispredicts) at each context switch; cycles sampled by instruction address | Windows | Works with VBS on (checked 2026-11-15, i7-8700). One sampled source per trace; mispredict sampling records nothing here. See "The one-worker scan on the hardware" |
| `-C instrument-coverage` | Exact execution counts per code region | Windows | A proxy for loop counts, not instructions |

---

## Tool: dhat

Every Rust heap allocation of a `gallery` run, by the backtrace that made it: how many, how big, how long they live,
what was live at the global peak and what is left at exit. `dhat` is a patched dhat-rs 0.3.3 (`vendor/README.md`,
"dhat") that takes whole backtraces in seconds where upstream takes minutes.

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
  Untrimmed, the stacks are up to 55 frames deep and reach the thread's start. The largest site was `pdq::dct16`'s
  zeroed buffer (105.7 MB in 305 blocks, at `pdq.rs:117` at the time); `crates/analysis/src/pdq.rs` now keeps the
  hasher's buffers between images.

## Tool: Pin call graph

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

- **Stages:** see [Startup](#startup). `-stop STAGE` stops every thread when one enters it, writes the profile and ends
  the process, so nothing after it is counted. SDE's `-control stop:…` didn't do that: threads started later kept
  counting. `-start STAGE` zeroes the counts.
- **Threads** are named from `SetThreadDescription` (Rust's and Qt's thread names) in `PREFIX.<pid>.threads`, with
  each thread's total and every image loaded.
- **Symbols:** Pin reads a PDB only from beside its DLL or from where it was built; it ignores `_NT_SYMBOL_PATH` and
  DIA's registry path. Where it has one (our exe, Qt with `just deploy release pdb`) a function is Pin's routine.
  Elsewhere Pin's routines are the exports, each spanning to the next (SDE's wrong names), so the tool names a
  function by the address it was entered at, `ntdll.dll+0x1fa14`. `tools/profiling/callgraph/symbolize.py` (run by
  `profile.py`) then names those with dbghelp from Microsoft's symbol server, cached in `D:\symbols`, turns the
  positions (instruction addresses) into source lines, and demangles Rust's v0 names with `llvm-cxxfilt`. It needs the
  Windows SDK's x64 debugging tools. Windows' public PDBs have no lines (`???`), and NVIDIA's driver has no symbols
  (nearest export).
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

## Tool: Intel SDE

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
- **Wrong names in DLLs without PDBs:** they get the nearest export's name, e.g. `av_parser_close` in
  `avcodec-6.dll`. Treat those rows as "somewhere in that DLL".
- **Self counts only:** each function's count excludes its callees. There is no call graph and no kcachegrind view:
  the [Pin call graph](#tool-pin-call-graph) has both.
- **Threads:** counts are per thread. For pipelines, add the threads up; the split between them varies from run to
  run.
- **Whole run:** the totals include startup, opening the catalog and exit. `-control stop:enter_func:pg_stage_ready:bcast`
  stops at a startup stage (`crates/trace/src/lib.rs`; Rust's `::` paths can't be named there, hence the C names),
  but threads started after the stop still count.
- **Slowdown:** about 10–100×, roughly valgrind's.

## Tool: callgrind / gungraun

For repeatable instruction counts per function (regression benches), with callgrind's call graph and its inclusive
costs, which SDE lacks. The catch: it measures the **Linux** build of the per-file code (`pg-analysis`, `pg-render`).
That build uses glibc's allocator and FFmpeg 8 (Ubuntu's `libavcodec` 62.11; the Windows app ships FFmpeg with
`avcodec-63`). libjpeg-turbo is built from the same vendored source, but with GCC instead of MSVC.

**Environment:** the pg-dev container, built from `docker/Dockerfile`. Its packages are in `docker/apt-packages.txt`
and `docker/apt-packages-bench.txt` (valgrind among them), and the image installs the `gungraun-runner` of the version
in `Cargo.lock`. It replaced a WSL2 distro (`Ubuntu-26.04`) on 2026-11-09 (`62b4540`). CI's setup is
`tools/profiling/ci_setup.py`.

**The bench** is `crates/bench/benches/ir.rs` (gungraun 0.20, a Linux-only dependency of `pg-bench`). On other
systems the target's `main` only prints where to run them, so `cargo clippy --all-targets` still builds it on Windows.
It measures `Analyzer::analyze` and `Maker::make` (height 256) on one synthetic photo,
`crates/bench/benches/ir/sample.jpg` (126 KB, 1280×1017 baseline JPEG, quality 80, 4:2:0), which
`tools/profiling/make_bench_jpeg.py` generates: 1/f noise, the spectrum of a natural photo but few hard edges. It's
committed, so CI runs the same benches. Its numbers are not comparable with the ones from the real photo that the
benches used before (the old numbers are no longer in the repo). `PG_BENCH_JPEG` at build time picks another file.
Reading it and creating the `Analyzer` and the `Maker` happen in gungraun's `setup`, outside the measured function.
Their buffers start empty, so each run measures a worker's first file. The bench also has catalog benches
(`crates/bench/benches/ir/catalog.rs`: the grid's list, the tree and the background job, on a synthetic catalog made by
`tools/profiling/bench_data.py`); `tools/profiling/bench.py` (`just linux-bench`) takes a bench name to run only those.

```powershell
just linux-build                          # once, and after a Dockerfile or Cargo.lock (gungraun) change
just perf                                 # the committed HEAD's benches, compared with docs/perf/baseline.json
just linux-bench --save-baseline=before   # gungraun's own options, without the gate
```

`just perf` benches the **committed HEAD**: the launcher exports it (`git archive`) into the container, so the Windows
tree's uncommitted work stays out and the build runs on the container's own disk. Arguments of `just linux-bench` go
to gungraun (e.g. `--save-baseline=before`, then `--baseline=before`). A full run's `.out` files are also archived,
zstd-compressed, to `docs/perf/callgrind/<sha>.tar.zst` (the benched HEAD's short sha, committed). A run given a bench
name (a filter) is partial and is not archived; options such as `--save-baseline` do not make a run partial. An
option's value goes after `=`: `--save-baseline before` is refused, because `before` could be a bench name.

**The large dataset** (2026-11-11). Three catalog benches measure what the inbox and the scan do at scale:

- `scan_walk` (`first`, `again`): `pg_scan::scan_with` over a synthetic tree (`pg-scan`'s `bench` feature,
  `pg_scan::synthetic`: 100 folders of 100 files and 4 folders it can't open, so the folder ledger is written) into a
  catalog in memory. No file system under it and no bench data, so it runs wherever the benches do. `again` walks the
  same tree over the catalog that has it, as a rescan does.
- `import_flush`: one import batch (500 files, one in five failing) stored in one transaction, on the large layout, so
  the import ledger's triggers run over its 50,000 failures.
- `inbox_refresh` (`closed`, `open`): `Catalog::ledger_entries` on the large layout, with the inbox closed (each
  ledger's cheap state) and open (the counts too).

The large layout is made like the default one, on the Windows side, under a name ending in `-large`
(`v6-windows-rules-large`, the bench's `LARGE_LAYOUT`): `synthetic_library.py --profile large` (400 folders of 500
files, a quarter of them broken JPEGs that the import fails, small pictures so the library is a few GB), the CLI's
scan, import and thumbnails over it, then `bench_data.py v6-windows-rules-large <catalog folder> <bench-data>`; the
commands are in `bench_data.py`'s docstring. For a `-large` layout `bench_data.py` also seeds the failure ledger to
100,000 rows (`--ledger-rows`; the import's own rows first, then thumbnails' and borders' rows), when the copy's schema
has `file_failures.hidden`; an older schema is left unseeded, with a note.

**Budgets** (`docs/perf/budgets.json`): a change that moves some counts on purpose is gated once by
`just perf --budgets` instead of `--update`. Each budget names a bench (`scan-walk` matches gungraun's `scan_walk.<id>`;
`-` and `_` are the same) and the range its change must fall in, as fractions of the baseline (`min`, `max`, either
optional; `"max": -0.90` is "must fall by 90% or more"). The benches with no budget keep the ±1% tolerance. A budget no
bench matches fails the gate (a renamed bench isn't passed unseen), a bench that misses its budget fails, and
`--budgets` is refused with `--update`: a miss is not re-baselined. The budgets of the notifications fix plan's F1:

| Bench | Budget | Why |
|---|---|---|
| `scan-walk.again` | ≤ +10% | batching and the scan lease |
| `scan-walk.first` | ≤ +20% | the pairing reads the fresh rows once per tier, in each batch's own transaction |
| `import-flush` | ≤ +30% | the import ledger's two triggers per analysis: a frame of 10-25k instructions each, firing or not |
| `pending-import` | ≤ +2% | the larger schema's fixed cost around one probe |
| `inbox-refresh.closed` | ≤ −90% | the ledger's cheap state on the partial index instead of the counts |
| `inbox-refresh.open` | ≤ −70% | the counts read while the inbox is shown are linear in the live rows |
| `pending-thumbs` | ≤ +18% | the pass's ATTACH parses the catalog's larger schema: a fixed cost per pass |

What callgrind can't see (how long a write lock is held, an I/O wait) is measured by the scan crate's wall-clock tests
(each write transaction of a 200k-file synthetic walk under a stated bound), not by a bench.

**Reading the results.** Unpack an archive with `tar --zstd -xf docs/perf/callgrind/<sha>.tar.zst -C some/dir`. Read
the results with `cgtab.py`, in the container (`just linux-shell`):

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
   `FFMPEG_DIR`, which made the Linux build look for Windows DLLs). `crates/video/build.rs` now finds FFmpeg's headers on
   Linux through pkg-config (the development packages in `docker/apt-packages.txt`), and on Windows in `.deps/ffmpeg`.
   `LIBCLANG_PATH` still has to be set on Linux, because libclang's folder depends on the Ubuntu release: the container
   sets it (a link, `/opt/libclang`, that the Dockerfile makes) to the newest `/usr/lib/llvm-*/lib`, and the CI workflow
   does the same for the runner.
3. `build.rs` copies the FFmpeg DLLs on Windows targets only.
4. `.gitattributes` keeps `*.sh` at LF: they run in the container straight from the Windows checkout, where
   `core.autocrlf` would add `\r`.

**Noise between builds:** the code is deterministic, but code generation is not stable across unrelated changes. From
41eaa1c to e9d93b6 both benchmarks rose about 0.05% (29 k instructions), with no change to the code they run.
`cgtab.py diff` puts all of it in EXIF parsing: `kamadak-exif` reading through a `Chain` reader instead of a plain
`Cursor`. e9d93b6 added about 1,000 lines to `thumbs.rs` (now `crates/thumbcache/src/lib.rs`), which changes how rustc
splits `pg-core` (since split into the per-concern crates) into codegen units and so what gets inlined where. Treat
changes under ~0.1% from unrelated commits as layout, not cost.

**`cgtab.py`** reads the raw callgrind format instead of scraping `callgrind_annotate`, whose columns shift with
`--show` and which splits each function into one row per source file. That row split made `analyze` look like 78% of
its own benchmark. The script:

- Handles compressed names (`(id) name` the first time, then `(id)`, with separate id spaces for fn/cfn, ob/cob and
  the file keys). Not handling them is what made a direct parse fail elsewhere.
- Gets exact call counts and per-edge inclusive costs from `calls=` lines.
- Groups by function (`--by fn`), crate, source file or object. Crate and file attribution follow `fi=`/`fe=`, so
  inlined code is counted where it came from. Inlined std code (`core_arch` SIMD intrinsics, iterators) goes to the
  crate of the function it was inlined into; without that, `fast_image_resize`'s AVX2 kernels show up as "std".
- `diff` merges by function name, because the object name carries a cargo build hash, and takes `--rename OLD=NEW`
  for renamed functions. Code without a symbol (named by its address, which moves between builds) becomes one row per
  object. `--tsv` prints every row.

Checked against gungraun and `callgrind_annotate` on both benchmarks: the totals match, the inclusive counts of
`box_1d`, `tj3Decompress8`, `tj3Compress8` and `precompute_coefficients` and the self count of `__sin_fma` match, and a
diff of two identical runs is empty. Like `callgrind_annotate`, it leaves out direct recursion from inclusive cost, but
indirect recursion (A → B → A) is still counted twice.

---

## Startup

The start's stages are functions with plain C names in `crates/trace/src/lib.rs` (`pg_stage_main` … `pg_stage_ready`),
so that SDE and the Pin call graph can stop at them. Rust's `::` paths can't be named in a stop argument, hence the C
names. The app's printing of them is `crates/desktop/src/trace.rs`: with `PG_TRACE=1` each stage's milliseconds since
process start are printed. The stages in the tables below are `init_done`, `qml_load`, `qml_loaded`, `tree_shown`,
`first_frame`, `list_shown`, `ready` and `round_done`.

The job's rounds are stages too: `round_done` is marked after each round finishes, however it ended
(`crates/library/src/runner.rs`, `crates/desktop/src/library.rs`). A round's passes write on their own.

---

## Measurements

### Heap at the start

Heap allocations of the app's start on the default catalog (dhat, Rust side only), before and after three rounds of
fixes (2026-11-01, 2026-11-02). The dhat profiles are no longer on disk, so these totals cannot be re-derived; they are
the record. Under dhat and offscreen the stage times were not comparable to the release ones below.

| | Before | After the first fixes | After the second | After the third |
|---|---:|---:|---:|---:|
| Allocated | 38.9 MB in 16,523 blocks | 13.1 MB in 686 | 12.9 MB in 521 | 11.9 MB in 354 |
| Peak | 17.2 MB in 2,125 blocks | 11.4 MB in 131 | 11.4 MB in 131 | 11.5 MB in 118 |
| At exit | 1.9 KB in 26 blocks | 2.1 KB in 26 | 2.1 KB in 26 | 1.2 KB in 15 |

- **The list** is 189,518 entries of 56 bytes (`crates/model/src/grid.rs:41`), 10.6 MB. Before the fixes its `Vec` was
  built without a size hint, so it doubled to 262,144 slots (14.7 MB, 4.1 MB unused) and was the peak. After the third
  fixes it is one block, sized to its exact count.
- **The rest** after the third fixes: the rows (537 KB), the placeholders' RGBA (101 blocks, 290 KB, handed to Qt
  without a copy), and per-thread buffers and connections. The cached JPEGs are read into a per-thread buffer, so a
  cache hit allocates nothing in steady state.

The heap totals of a scan, an import and the borders and thumbnail passes are in [HEAP_PROFILE.md](HEAP_PROFILE.md).

### The start's stages before and after the two heap plans

Release, `PG_TRACE=1`, offscreen, 2026-11-02. `gallery-prev.exe` (2e1b6ca, before both plans) against `gallery.exe`
(1d3685b), on the default catalog: 189,518 files, "All folders" by name, file 8838 selected, nothing to do. Each run
was `--wait=2 --screenshot=…`, with `QT_QPA_PLATFORM=offscreen`. The two builds alternated, one warm-up round and then
five measured. The medians are in ms from `pg_stage_main`. The previous build's process start to `main` was bimodal:
19–20 ms in 2 runs, 122–126 ms in 3. That is before any of our code runs, and the new build's was 18–19 ms in all 5. So
the stages are measured from `main`.

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

- **The list is shown 42 ms sooner.** Reading and laying out 189,518 entries no longer grows the list's `Vec` (29.4 MB
  copied before) or builds an aspects `Vec`. The reset no longer makes the top rows' cells, which the view then left
  (54714b1).
- **`ready` is 41 ms later, but it now marks more.** Since 54714b1 the view is placed on the selected file before any
  cell is made, so that file's rows are made between `list_shown` and `ready` (`gridLoaded`,
  `positionViewAtIndex`). Before, `ready` came with the list, while the top rows' cells were made, and the selected
  file's rows were made after it. The end of the first round, which follows both, is the same (651 → 645 ms).
- **The heap work doesn't show in the start's time.** What it removed (16,000 blocks, 26 MB) cost a few ms. The start
  is Qt's QML loading (174 ms) and the list's read (270 ms): the call graph below.

### The start, to `ready`

Pin call graph, release `gallery.exe` with the trace stages, scratch copy of the pinterest catalog, 2026-10-31. Catalog:
189,518 files, 690 folders; the session folder is "All folders", so the grid lists every file.
**15.15 G instructions in 85 threads** from process start to `pg_stage_ready`. SDE's 17.4–18.1 G included threads that
kept counting after it (a 0.96 G SQLite thread: the library's first import check, which starts at `ready`).

| Thread | Instructions | What |
|---|---:|---|
| catalog reader | 10.90 G | the grid's `browse::list_files` (now `list_grid`, `crates/catalog/src/browse/read/grid.rs`) 10.25 G; the tree and the rest 0.65 G |
| main | 1.24 G | QML creation, fonts, a Direct3D 9 probe |
| QSGRenderThread | 1.18 G | the Direct3D 11 device |
| video decoding | 0.93 G | `choose_video_decoding`'s own Direct3D 11 device (the AV1 probe) |
| an NVIDIA driver thread | 0.32 G | driver internals |
| QQmlThread | 0.30 G | QML type loading |

- **The name sort, about 7.0 G (46%), is gone.** At the time, in `list_files`, SQLite's sorter (`vdbeSorterSort`) was
  7.59 G, nearly all of it `browse::path_collation`: 1.59 M comparisons at about 4,400 instructions each. Unicode
  lowercasing (`FlatMap<Chars, ToLowercase>`, `to_lower`, `ToCasefold`) was 5.67 G, and rusqlite's
  `String::from_utf8_lossy` of both strings on every comparison 1.24 G. The rest of SQLite's work on the list was about
  2.6 G. The list is now read in `files_order`, and `path_collation` no longer exists.
- **The GPU driver verifying its signature, 2.0 G (13%), in 11 scans.** Each Direct3D device creation reaches NVIDIA's
  loader (`nvldumdx!OpenAdapter`), which checks its DLL's signature, and Windows scans the whole file
  (`imagehlp!RabinKarpFindPatternInBuffer`, about 182 M instructions a scan). Main: 3 scans, 0.55 G, from Qt's window
  creation (`WindowCreationData::create` → `calcPosition` → `GpuDescription::detect` → `Direct3DCreate9`). Render
  thread: 4 scans, 0.73 G (`QRhiD3D11::create`). Video probe: 4 scans, 0.73 G. This is SDE's "imagehlp" row, and the GPU
  opened three times. The app now shares one device with Qt Quick (`crates/desktop/src/gpu_device.rs`), so the video
  probe no longer opens a second one.
- **Text and fonts on the main thread, about 0.3 G:** DirectWrite reading glyph coverage and the character map (0.23 G
  self), and Qt loading kerning pairs.

### Library scan, wall clock

Wall-clock time on this machine, not instructions, so these are a record, not a `perf` result. They were taken with the
shell script `scan-timing.sh`. Its Python successor is `tools/profiling/scan_timing.py`, with the same arguments:
`python3 tools/profiling/scan_timing.py GALLERY LIBRARY WORK [--drop-caches]`.

**WSL, 2026-11-07.**

- **Binary:** `gallery` release build (`cargo build --release -p pg-gallery --bin gallery`, WSL Ubuntu-26.04, Qt 6.12
  for Linux, nightly-2026-11-06), from the tree at `262004b`. The crates are the same as `22ae0ff`: `262004b` changed
  no crate.
- **Library:** `tools/profiling/synthetic_library.py --folders 100 --per-folder 100 --seed 1`: 10,000 files (mostly
  JPEG, some PNG, GIF and WebP), on WSL's ext4 disk (`/dev/sdf`), not tmpfs. Renumbered in place, which the scan pairs
  (the same folder, size, mtime and identity under a new name).
- **Command:** `scan-timing.sh GALLERY LIBRARY WORK [--drop-caches]` (the shell script; the Python one takes the same
  arguments), run as root for `--drop-caches`. Each scan is into a fresh catalog;
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
- **tmpfs under a scanned root** (the S-17.7 manual check): a tmpfs folder of 20 files inside a library with 20 plain
  files (device numbers 178 and 2128). Two scans; 40 rows and 0 missing after each. No row flipped.
- **The cold VM** (the first run after `wsl --shutdown`, same library, same binary): first 1.353 s, unchanged 0.129 s,
  renumbered 0.245 s. The VM's own cache was empty at the restart; this run did not drop the caches.
- **Device change across a restart** (the S-17.7 end-to-end check, with the HEAD binary): a 40-file library on the
  Windows drive (drvfs), scanned, one rating set to 4, and imported (40 analysis rows). Across `wsl --shutdown` the
  library's device number changed (201 to 145), and the rescan found all 40 rows under a new identity: same ids and
  names, none missing, the rating and the analysis kept. A second restart changed it again (145 to 74), with the same
  result. Not done here: the plan's first step, a catalog built with the older binary; this check used HEAD's binary
  for both scans.
- **Device change across a restart, after 94a2530** (2026-11-11, HEAD `3aa7efd`): the headless `gallery` built in the
  container (`just linux-gallery`), run in WSL Ubuntu-26.04, nothing built there. A 40-file library
  (`synthetic_library.py --folders 4 --per-folder 10 --seed 1`) on drvfs (`/mnt/d`), scanned and imported (40 analysis
  rows), one rating set to 4. The scan says the volume has no UUID (`ScanNote::VolumeFallback`), so its identity is the
  device number. A distro restart inside one VM boot (`wsl --terminate Ubuntu-26.04`, the same boot id) changed the
  device 85 to 52; `wsl --shutdown` (a new boot id) changed it 52 to 74. After each, the rescan found all 40 files under
  a new identity (`Files found again`: 40): the same ids, names and file IDs, none missing or unpaired, the rating and
  the 40 analysis rows kept. The older-binary step can't be run: the history is squashed at `88e780b`, which already
  holds S-17.7, so there is no earlier build to make the catalog.

**Windows, 2026-11-07.** The shell script, under Git Bash with the Windows Python; `gallery.exe` release build of the
working tree, whose crates are HEAD's, on NTFS, the 10,000-file library on `D:`: first 1.316 s, unchanged 0.146 s,
renumbered 0.322 s. The renumbered scan paired every row: 10,000 rows, none missing.

The `just perf` gate has run in the pg-dev container since 2026-11-09. Its baseline was last re-measured at `e7576f0`
(the modularity plan's Validation 4, git e428ab2).

### `analyze` and `thumbs::make` on one JPEG

Gungraun, Linux build of HEAD 41eaa1c, the bench's 126 KB sample, 2026-10-31. The functions are named as at that commit:
`analyze::analyze` is `Analyzer::analyze` (`crates/analysis/src/analyze.rs`) and `thumbs::make` is `Maker::make`
(`crates/render/src/maker.rs`).

Two runs gave exactly the same counts (63,494,636 and 50,648,725 instructions, gungraun: "No change"), so a change of
even 0.1% shows. gungraun's estimated cycles, from callgrind's cache simulation, are 86.4 M and 68.7 M.

> **The resize rows are from the old resampler.** They were measured with Lanczos3 through `fast_image_resize`. The
> thumbnail path has since moved to Magic Kernel Sharp 2021 in linear light, one resize for every pipeline
> (`crates/codec/src/resize.rs`, `a70d174`). The Lanczos rows describe the pipeline that no longer runs; the new
> resize has not been measured.

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

**The decode is at full scale.** `DECODE_MARGIN = 4` (now `crates/render/src/display.rs:24`) asks for ≥ 1024 px high,
and the image is 1017 high, so no DCT scaling is applied. That costs 28.5 M here against `analyze`'s 21.2 M. The margin
is a quality decision (`THUMBNAILS.md`, "Thumbnails"): Lanczos3 from at least 4× the target is the thumbnail path, and
this measures its price for images just under 4× the thumbnail height.

**The PDQ box filter's cost.** `pdq::box_1d` is 21% of `analyze`, and the cache misses are concentrated there. It has
65% of the benchmark's L1 read misses and 84% of its L1 write misses. `dct16` makes 2 passes, and each runs it over
every row and then over every column (`stride = width`). That gives 1,838 calls (2 × 919 rows plus columns of the PDQ
input) at 7.4 k instructions each.

### `gallery status`

Release `gallery.exe` of 2026-10-31 15:41, pinterest catalog, 2026-10-31. Catalog: 189,518 files, all analyzed, 5,216
PDQ pairs, no thumbnails. 18 s under SDE.

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
  (`crates/catalog/src/lookup.rs`). Each count now has its own index where one exists (`files_present`,
  `files_to_import`, `analysis_failed`), so this profile no longer describes it. Startup and the rest of the CLI are a
  few percent.
- **The counts are exact:** `countStep` ran 384,252 times = 189,518 files + 189,518 analyzed + 5,216 PDQ pairs, the
  rows those queries match. The `missing = 1`, `failed` and `file_writes` counts match no rows but still scan.
- The 579 k `sqlite3BtreeTableMoveto` calls are rowid lookups. The two `analysis ⋈ files` joins explain about 379 k
  (one lookup per `analysis` row each). The other ~200 k are probably the `pending` query; not checked.

### JPEG decoders and encoders

Moved to [JPEG_DECODERS.md](JPEG_DECODERS.md): libjpeg-turbo against five CPU decoders, nvJPEG's two backends, NVDEC,
restart markers, lossless baseline conversion, and the decoders researched but not measured. Verdict: keep
libjpeg-turbo, and convert progressive files to baseline (2.6× fewer instructions, identical pixels). Encoders are in
[JPEG_ENCODERS.md](JPEG_ENCODERS.md).

---

### The pairing pass

The tier-1 pairs after the import (`pg_import::pairs`, the join in `pg_dedup::join`), measured for
[PAIRING_PASS_PLAN.md](../plans/PAIRING_PASS_PLAN.md). The data is the user's library (189,520 files, 187,804 pictures
with orientation hashes). The release build ran on an i7-8700 (6 cores, 12 threads). Every run was on a scratch copy,
never the live catalog.

**The scratch catalog.** Start from the copy kept before migration v9
(`<catalog dir>\catalog.sqlite.pre-v9.bak`) placed in a scratch folder as `catalog.sqlite`. Let the CLI migrate it
(`gallery --catalog-dir <scratch> status`): it comes out at v12, with 5,277 pairs and 189,520 files. Copy that file
aside as the source of every run: the scripts copy it fresh for each run, since a pass changes it. The pairs a full pass
makes (`gallery --catalog-dir <copy> pair --again`, then `present_pdq_pairs` dumped as `a b distance`) are the oracle:
5,277 pairs, which every change to the pass must reproduce exactly.

**The machine is shared.** The CPU load before runs varied from 24% to 82%. So every figure is a median of interleaved
runs, and `sweep.ps1` flags and leaves out the runs that start above 30%.

| Script (`tools/profiling/pairing/`) | Measures |
|---|---|
| `sweep.ps1 -Catalog <src> -Workers 1,2,4,6,12 -Reps 4 [-Owed N] [-Slice N] [-CacheKib N]` | `pairs_probe`'s wall clock and its parts (`PairStats::timings`) per worker count; medians |
| `peak_mem.ps1 -Catalog <src> -Owed 0,1,30000,187804 [-Slice N] [-CacheKib N]` | peak working set and peak commit (MiB, as the script prints them), sampled every 15 ms |
| `set_owed.py <catalog> <N>` | makes N pictures owe their pairs, spread over the ids (either layout) |
| `dhat_top.py <dhat-heap.json>` | the heap sites live at the peak (`target/dhat/gallery.exe --catalog-dir <copy> pair`) |

**Callgrind, one join at a time** (pg-dev container, `pairs_probe` built there, `PG_PROBE_WORKERS=1`):
`valgrind --tool=callgrind --zero-before=<join entry point> --dump-after=<join entry point> pairs_probe <copy>`.
That writes one dump per slice's join, and `cgtab.py` reads each one. The entry point is `pg_dedup::join::join`; after
the rebuild it is the joiner's per-slice method, kept `#[inline(never)]` so the toggle has a symbol to match. Line-level
counts need the Rust sources at the path the binary names: link `rust-src` at `/rustc/<commit hash>`.

#### Before the rebuild (HEAD `0b82126`)

Pass at 12 workers: **7.4 s median** (11.6 - 12.1 s on 1 worker, 7.2 - 7.4 s at 6, flat past 6 workers). It ran in
slices of 50,000 owed pictures, 4 for the whole library.

Re-measured with `sweep.ps1` once the pass reported its timings, on a quieter machine (2 reps):

| Workers | Total | Load | Join | Write statements | Commits |
|---|---|---|---|---|---|
| 1 | 10.63 s | 0.27 s | 9.27 s | 0.34 s | 0.71 s |
| 12 | 6.56 s | 0.28 s | 5.15 s | 0.35 s | 0.74 s |

The join is 87% of the pass at 1 worker and 78% at 12. The commits cost twice the statements.

| Part | Wall (12 workers) | Instructions (1 worker) | Cause |
|---|---|---|---|
| Join preparation | 1.5 - 1.7 s | 17.7 G | `is_symmetric` recomputed for every picture on every slice: a bit-by-bit 16x16 transpose, 486 ns each |
| Sorts | 2.1 - 2.5 s | 2.6 G | 16 chunks x 4 slices x 2 sides of a 65,536-bucket counting sort of 40-byte entries (561 ms per slice on one thread); the stored side re-sorted every slice; 64 fresh 30 MB buffers |
| Writes | 1.1 - 1.4 s | 2.8 G | 187,804 `UPDATE`s of the `analysis` row: 98.7 MB of WAL, 22 - 24 checkpoints; `commit()` alone 0.75 - 1.1 s |
| Load | 0.34 - 0.56 s | 1.49 G | rusqlite decoding (0.93 G), three scans of `analysis JOIN files`, a temp B-tree for `ORDER BY`, a second 50 MB copy of the hashes |
| Scan stages | 0.9 s (5.2 s on 1 worker) | 54.6 G | 1.78 G vector iterations, 7.1 G lane slots for 2.9 G useful candidates |
| `Catalog::prepare` | about 130 ms | 0.41 G | `mark_unheld_missing` scans 190 k rows (64 ms); the expected schema built twice |

| Memory | Peak commit | Rust heap at the peak (dhat) |
|---|---|---|
| Nothing owed | 62 MB | |
| One owed picture | 366 MB | 201.9 MB |
| All 187,804 owed | 415 MB | 247.7 MB: `Vec<PairingPicture>` 79.7 MB (17 growth steps allocate 159 MB), the `Vec<Picture>` copy 48.3 MB, the join's base entries 30 + 21 MB, one stage 66 MB |

The other 167 MB of the all-owed peak is SQLite's page cache (the app's 256 MB `cache_size`, filled by the load), the
binary and the stacks. A pass for one owed picture takes 1.3 - 1.6 s (0.18 s with nothing owed), almost all of it fixed
cost: load 0.3 s, the symmetry check 0.43 s, the stored side's sorts 0.35 s, startup.

#### After block B: the data and the join's layout

Block B made the stored hashes the 4 kept orientations (migration 13, with `pairs_owed` as a queue table). The join now
keeps one permuted store, sorts with a radix into three arrays, applies the ordering rule as a suffix, and chunks on the
diagonals. Slices are still 50,000. Medians of 3 quiet runs:

| Workers | Total | Load | Join | Write statements | Commits |
|---|---|---|---|---|---|
| 1 | 7.79 s (10.63) | 0.08 s | 7.31 s | 0.18 s | 0.21 s |
| 6 | 3.17 s | 0.08 s | 2.64 s | 0.16 s | 0.31 s |
| 12 | 2.85 s (6.56) | 0.08 s | 2.27 s | 0.17 s | 0.31 s |

- **Correctness:** the migrated copy's pass gives the oracle's 5,277 pairs exactly. 0 of 187,808 blobs failed the
  transpose check, and integrity and foreign-key checks are clean.
- **The file:** 209.4 MB at v12, 177.8 MB (169.6 MiB) at v13 after the `VACUUM` and a pass. The migration, `VACUUM`
  included, and a full pass together took 6.3 s at 12 workers.
- **The commits** vary from 35 to 310 ms, with checkpoints; the statements are half of what they were. Block C found
  that most of this, and part of the "before" commits too, is the probe's own `pairs_again`: its WAL was merged by the
  pass's first commits. The load's 0.08 s was read from pages `pairs_again` had just cached.

#### After block C: the pass's connections and `prepare`

Block C reads the pictures on a read-only connection of their own (`Catalog::reader`), bounds the catalog's cache while
the pass writes (`Catalog::bounded_cache`), and makes the slice and the cache size parameters of the pass
(`pairs::Settings`). `prepare` now looks for unheld extensions only when `meta.held_exts` is another build's set, uses the
cached current schema, and does its one-off work on a 16 MB cache. The probe now closes and reopens the catalog after
`pairs_again`, so it times the pass alone (cold cache, no WAL of its own to merge). Medians of 3 quiet runs, the default
16 MB cache:

| Workers | Total | Load | Join | Write statements | Commits |
|---|---|---|---|---|---|
| 1 | 7.78 s (10.63) | 0.15 s | 7.35 s | 0.22 s | 0.03 s |
| 6 | 3.03 s | 0.14 s | 2.63 s | 0.23 s | 0.03 s |
| 12 | 2.63 s (6.56) | 0.14 s | 2.22 s | 0.23 s | 0.03 s |

**The cache size** of the pass's connections (all 187,804 owed, 12 workers, owed rows set by `set_owed.py` so the probe
reads nothing first; 2 runs each, which agreed within 2 MiB):

| Cache | Peak commit, all owed | Peak commit, one owed | Load | Write statements |
|---|---|---|---|---|
| 16 MB (chosen) | 131 MiB | 92 MiB | 0.15 s | 0.19 s |
| 32 MB | 150 MiB | 92 MiB | 0.15 - 0.18 s | 0.19 s |
| 64 MB | 184 MiB | 100 MiB | 0.16 - 0.17 s | 0.19 s |
| 256 MB (the app's) | 214 MiB | 147 MiB | 0.19 s | 0.21 s |

Nothing owed peaks at 2 MiB of commit: the pass is one probe. Before the rebuild these peaks were 415 MB (all owed) and
366 MB (one owed).

**`prepare` at every start** (the probe's whole process with nothing owed, 7 - 9 interleaved runs): **42 ms** when the
mark is this build's, against 111 - 126 ms when it is stale and the files are read for unheld extensions. A scan's writes
stamp the mark (`ScanWriter`, and the pairing after a scan), so a row a build with more extensions records is looked at
again at the next start of a build with fewer.

**Migration 13 under the app's pragmas** (gate G2b; the copy aside, the data step and `VACUUM`, from the 209.4 MB v12
copy, 3 runs):

| | Wall | Peak commit | Peak WAL | File after |
|---|---|---|---|---|
| The app's 256 MB cache | 3.6 - 4.2 s | 438 MiB | 169 MiB | 176.4 MB |
| A 16 MB cache for `prepare`'s work (kept) | 4.3 - 4.4 s | 221 MiB | 169 MiB | 176.4 MB |

The rest of the peak is `VACUUM`'s copy of the file, which `temp_store = MEMORY` builds in RAM. It happens once, at the
first start of a build with migration 13. The pass on the migrated copy gives the oracle's 5,277 pairs exactly.

#### After block D: the join's engine

Block D sorts each stage on 1 - 2 sorter threads while the workers scan the one before (a ring of two stage sets), adds
a few-owed path (each chunk's query values in a bitmap, one pass over the store), makes the slice 200,000 (one slice
for this library), and, with several slices, writes a slice while the next one's join runs. The probe now prints the
join's parts: `prepare` (the slice's query side), `sort` (the time the scan waited for its sorts) and `scan`. Medians of
3 quiet runs, default settings, all 187,804 owed:

| Workers | Sorters | Total | Load | Join | Sort wait | Scan | Write statements | Commits |
|---|---|---|---|---|---|---|---|---|
| 1 | 0 | 5.15 s (10.63) | 0.15 s | 4.70 s | 0.68 s | 3.91 s | 0.23 s | 0.04 s |
| 2 | 0 | 3.37 s | 0.17 s | 2.87 s | 0.71 s | 2.06 s | 0.29 s | 0.04 s |
| 4 | 1 | 2.02 s | 0.15 s | 1.58 s | 0.05 s | 1.44 s | 0.24 s | 0.04 s |
| 6 | 1 | 1.50 s | 0.15 s | 1.09 s | 0.05 s | 0.93 s | 0.23 s | 0.04 s |
| 12 | 2 | 1.27 s (6.56) | 0.14 s | 0.83 s | 0.06 s | 0.68 s | 0.24 s | 0.04 s |

The plan's targets: at 12 workers 5x or better, met (5.2x). On one worker 3.5x, **not met** (2.1x): what is left there is
the scan itself, 3.9 s on one thread. A pass for a few files, 0.35 s, met (below).

**Correctness on the real data** (`oracle_d.ps1` in the scratch folder; every case gives the oracle's 5,277 pairs
exactly, nothing left owed): all owed in one slice at 12 workers and at 1; all owed in four slices of 50,000, with the
write overlap; and 300, 3,000 and 30,000 owed, with those pictures' pairs deleted first (14, 143 and 1,553 of them), on
the few-owed path (300) and the sorted path (3,000 and 30,000, and 3,000 on one worker).

**G3, the sorter threads** (total, 2 runs each):

| Threads | 0 sorters | 1 sorter | 2 sorters |
|---|---|---|---|
| 2 | **3.17 s** | 4.37 s | 4.39 s |
| 4 | 2.28 s | **1.96 s** | 2.63 s |
| 6 | 1.99 s | **1.51 s** | 1.65 s |
| 8 | 1.93 s | **1.41 s** | 1.46 s |
| 12 | 1.80 s | 1.51 s | **1.22 s** |

Below 4 threads a sorter costs a worker the scan needs more; from 10 the scan is fast enough that one sorter falls
behind (it waited 0.33 s at 12). `join::default_sorters`: 0 up to 3 threads, 1 from 4 to 9, 2 from 10.

**G5, the slice** (total, 2 runs each, the machine busier than for the table above):

| Slice | 1 worker | 6 workers | 12 workers |
|---|---|---|---|
| 50,000 (4 slices) | 7.50 s | 2.13 s | 1.85 s |
| 100,000 (2 slices) | 5.88 s | 1.63 s | 1.47 s |
| 200,000 (1 slice, kept) | 5.08 s | 1.57 s | 1.44 s |

Every slice sorts the whole stored side again and sets up every probe again, so one slice is cheapest. The plan's
400,000 on a synthetic 500,000-picture corpus was not run: this library is one slice at 200,000, and a larger slice is a
question for a larger library, whose stored side alone already passes the cap (about 0.5 KB per picture).

**G6, the few-owed path against the sorted path** (join, 2 runs each; the few-owed path forced on with `PG_PROBE_FEW`,
the sorted path forced with 0):

| Owed | Few-owed, 12 workers | Sorted, 12 workers | Few-owed, 1 worker | Sorted, 1 worker |
|---|---|---|---|---|
| 1 | 60 ms | | 169 ms | 274 ms |
| 30 | | 256 ms | 260 ms | 264 ms |
| 300 | 92 ms | 261 ms | 385 ms | 297 ms |
| 1,000 | 189 ms | 264 ms | 880 ms | 388 ms |
| 2,500 | 308 ms | 280 ms | 2,228 ms | 508 ms |

The few-owed path's work is spread over every thread; the sorted path's floor (sorting the stored side, about 0.2 s) is
not. So the threshold is per thread: 1,000 query entries (125 owed pictures) per thread, `FEW_ENTRIES_PER_THREAD`. A
first version, which looked each hit up by binary search and read the query's full hash, was 2 - 3x slower per hit
(1,000 owed: 324 ms on 12 workers, 2,862 ms on 1); the kept one sorts each chunk's queries as a stage does and tests a
value and its six low-bit neighbours with one bitmap word.

**G4, the kernel's loop nest: not built.** The swapped loop (stored entries in the vector, the query broadcast) was a
candidate because at 50,000 a stored bucket averaged 11.5 entries against 6.1 queries. At one slice of 200,000 a query
bucket averages 23 (1.5 M entries over 65,536 values) against the same 11.5 stored, so the current loop already
vectorizes the longer side. Register-tiling the query vectors across the 17 probes was not tried either. The one-worker
scan, 3.9 s, is where a better kernel would show.

**Peak memory** (default settings, 12 workers, 2 runs that agreed within 2 MiB):

| Owed | Peak commit | Join | Path |
|---|---|---|---|
| 0 | 2 MiB | | (one probe) |
| 1 | 75 MiB | 91 ms | few-owed |
| 300 | 76 MiB | 184 ms | few-owed |
| 3,000 | 118 MiB | 292 ms | sorted |
| 30,000 | 148 MiB | 393 ms | sorted |
| 187,804 | 254 MiB | 822 ms | sorted |

The cap was 330 MB at every setting; before the rebuild these were 366 MB (one owed) and 415 MB (all owed). A pass with
one owed picture takes 0.24 s in all (0.15 s of it the load), against 1.3 - 1.6 s before.

#### One join under callgrind, one worker (P-13, `98c6973`)

`pairs_probe` built in the pg-tools container, on the migrated copy, all 187,804 owed, `PG_PROBE_WORKERS=1`,
`PG_PROBE_SORTERS=0`: `valgrind --tool=callgrind --cache-sim=yes --branch-sim=yes --zero-before='<pg_dedup::join::Joiner>::join'
--dump-after='<pg_dedup::join::Joiner>::join'`. The join: **36.7 G instructions, 452 M mispredicted branches** (8.8%
of 5.1 G conditional branches), 120 M D1 read misses, 128 M D1 write misses. The prototype had predicted 19 G for the
scan; it counted the iterations right (670 M vector iterations, 204 M (stored entry x probe) setups, both as predicted)
and the cost of each wrong.

| Part | Instructions | Mispredicts | Cache | Where |
|---|---|---|---|---|
| The scan (`scan_avx2`, with its inlined intrinsics and bounds checks) | 26.8 G | 413 M | 66 M D1 read misses | `join.rs` `scan_avx2`, `suffix` |
| - the ordering rule's pointer (`suffix`'s `while`) | 3.9 G | **216 M** | | a data-dependent walk per stored entry |
| - the vector loop's exit (`while y < qhi`) | 1.8 G | **197 M** | | one per setup: the trip count changes with every stored entry |
| - the popcounts and compares (intrinsics) | 11.4 G | | | 17 instructions per 4 lanes |
| - the lanes that pass (`consider`) | small | 0.8 M | | 670 M lane tests, almost none pass |
| The sorts (`StageSet::sort`, 16 stages) | 4.7 G | 12 M | 50 M D1 read, 126 M D1 write misses | the radix's second pass scatters into 256 buckets x 3 arrays |
| The slice's query side (transposes, permutation) | 0.5 G | | | |

Callgrind's branch model has no history, so its 413 M overstate the hardware's (289 M, next section), and its estimate
of the mispredicts' cost here (6 - 8 G cycles, "roughly 1.5 s") is superseded by the cycle samples below. The full
hashes the candidates read from the 48 MB store (`Sides::words`) do not show: so few lanes pass the head test that
`consider` is negligible (the hardware agrees: under 1% of the scan's cycles).

#### The one-worker scan on the hardware (P-13, `98c6973`)

Same pass (all owed, `PG_PROBE_WORKERS=1`, `PG_PROBE_SORTERS=0`), the Windows release build, i7-8700, VBS on. Two WPR
traces: counters read at every context switch (`HardwareCounter` with `InstructionRetired`, `BranchMispredictsRetired`,
`UnhaltedCoreCycles` on `CSwitch`; deltas charged to the thread switched out), and `UnhaltedCoreCycles` sampled every
65,536 cycles (keyword `PmcProfile`, `SampledCounter`), the samples mapped to the probe's image through the loader
events. Both read with a ctypes ETW reader (`OpenTraceW`/`ProcessTrace`; the counters are the
`EVENT_HEADER_EXT_TYPE_PMC_COUNTERS` extended item, the samples PerfInfo opcode 47). Sampling mispredicts
(`BranchMispredictsRetired` or `BranchMispredictions`) recorded no event on this machine, and a profile with two sampled
sources kept only one.

- **The scan's threads: 28.9 G instructions, 16.6 G cycles (1.74 per cycle), 289 M mispredicts.**
- Cycle samples in `scan_avx2`: 267,367 (17.5 G cycles, the sampling run's scan 4.9 s against 3.9 s unsampled):

| Part of `scan_avx2` | Share | Per unit |
|---|---:|---|
| The vector loop | 47.9% | about 12 cycles per iteration (669 M); its floor is about 6 (23 fused uops at 4 a cycle, 6 of them on port 5) |
| The ordering rule's pointer (`suffix`'s walk) | 23.0% | about 19 cycles per setup (204 M) |
| A stored entry's setup (code, head words, broadcasts) | 25.1% | of which 12.1% on the three instructions where the walk returns: its exit's recovery |
| The bucket and probe loops | 3.1% | |
| Lanes that pass, `consider`, the store reads | 0.9% | |

- The sorts: 46,616 samples (about 3 G cycles). The radix's second pass (three arrays) is 24% of them; zeroing the
  output and scratch arrays every stage (`clear` then `resize`) 9%.
- **Loop counts replayed on the real hashes** (every setup's suffix from the sorted codes): 668,985,053 vector
  iterations and 204,314,684 setups, callgrind's counts. A fixed trip count per (bucket, probe), the whole query
  bucket for every stored entry, would be 1,273,969,575 iterations (1.90x). 75% of the setups meet a query bucket of
  17 - 32 entries, so a special case for small buckets changes nothing (1.00x at 4 or 8 entries).

What this says about the leads (P-13). A kernel with fixed trip counts doubles the loop that is already half the scan
and runs at twice its floor, against the walk's 23 - 35%: a loss unless the loop then runs near its floor, and at 12
workers it loses the hyperthread's cover for its stalls. The ordering rule needs no lane mask in any kernel: `consider`
rejects what is not owed. The swapped loop has the same counts. The second-pass rewrite saves at most part of 24% of
the sorts (about 0.17 s on one worker, hidden behind the scan from 4 threads). What the samples point to is the walk:
its exit is unpredictable (it advances 0 - 3 entries a time), and an 8-wide compare from the pointer gives the same start
with one branch that almost never fires (the pointer moves at most a bucket, about 23 entries, per probe).

**After the vector start** (`suffix_avx2`, the AVX2 kernel's only change; the codes padded by eight). Same pass, HEAD's
build and the new one interleaved, 5 runs each, a fresh catalog copy per run, CPU load 10 - 34% before the runs;
medians in ms:

| Workers | Build | Pass | Join | Sort wait | Scan (min - max) |
|---:|---|---:|---:|---:|---:|
| 1 | `98c6973` | 5,003 | 4,576 | 683 | 3,802 (3,784 - 3,890) |
| 1 | vector start | 4,356 | 3,890 | 667 | **3,109** (3,053 - 3,226) |
| 12 | `98c6973` | 1,279 | 849 | 112 | 655 (637 - 688) |
| 12 | vector start | 1,258 | 808 | 185 | **523** (513 - 547) |

The scan is 18% faster on one worker and 20% on 12; the oracle's 5,277 pairs are identical on all seven paths of the
check (one slice, slices of 50,000, one worker, 300 / 3,000 / 30,000 owed). On 12 workers most of the gain moves into
the sort wait (112 -> 185 ms): two sorters no longer keep ahead of a 523 ms scan, so the join there is now bound by the
sorts, and the sorts' costs (the second pass 24%, the zeroing 9%) are no longer hidden.

#### The scan kernel (pairing plan, second round, step 3)

The kernel tests a stored entry against eight query entries at once: the stage's head (`w` `u32` words of the chunks
after its own), the stored side's in rows and the query side's in columns, against a bound exact for the pairs the
stage reports (`31 - 2 x earlier chunks outside the head - [the probe flips a bit]`, at least 4). Same machine and
build as above; turbo off (the power plan's maximum processor state at 99%).

**The head's width per stage.** Each stage's one-worker scan at one to five words, two runs each (median ms; in
parentheses, the candidates tested in full when over 1 M; from 2 words on most stages are at the floor of about 19,500
true near-pairs):

| Stage | W = 1 | W = 2 | W = 3 | W = 4 | W = 5 | Chosen |
|---:|---:|---:|---:|---:|---:|---:|
| 0 | 2,441 (152.9 M) | 1,942 (57.6 M) | 312 | 335 | 360 | 3 |
| 1 | 2,948 (149.1 M) | 1,678 (31.4 M) | 289 | 333 | 335 | 3 |
| 2 | 2,598 (151.7 M) | 1,207 (14.8 M) | 289 | 334 | 349 | 3 |
| 3 | 2,944 (148.0 M) | 779 (5.5 M) | 284 | 308 | 340 | 3 |
| 4 | 2,701 (147.1 M) | 457 (1.7 M) | 378 | 321 | 340 | 3 |
| 5 | 3,069 (140.6 M) | 340 | 272 | 316 | 367 | 3 |
| 6 | 2,525 (123.3 M) | 286 | 289 | 333 | 350 | 2 |
| 7 | 2,539 (88.3 M) | 258 | 275 | 367 | 345 | 2 |
| 8 | 1,800 (48.4 M) | 270 | 341 | 316 | 360 | 2 |
| 9 | 1,284 (18.3 M) | 266 | 324 | 313 | 343 | 2 |
| 10 | 611 (4.6 M) | 382 | 294 | 324 | 343 | 2 |
| 11 | 328 | 275 | 288 | 308 | 394 | 2 |
| 12 | 231 | 270 | 304 | 339 | 343 | 1 |
| 13 | 230 | 258 | 313 | 312 | 348 | 1 |
| 14 | 221 | 330 | 319 | 328 | 356 | 1 |
| 15 | 214 | 276 | 291 | 332 | 340 | 1 |

A stage scans about 150 M lanes. One more word costs about 24 ms a stage (0.16 ns a lane; the columns at 3 - 5 words,
where the candidates are at the floor). A false candidate costs from 14 ns, when nearly every lane is one and the
store's lines are reused (stage 0 at one word: 152.9 M for 2.1 s over its 3-word time), to about 100 ns when they are
few and each misses the cache (stage 3 at two words, 5.5 M for 0.49 s; stage 4, 1.7 M for 0.17 s). A width is decided
where the candidates are few, so the rule takes the 100 ns. The rule (`FULL_TEST_COST`, `widths()` in
`crates/dedup/src/join.rs`) takes, per stage, the `W` that minimises `W + 600 x P(Bin(32 W, 1/2) <= bound(c, W))`, a
false candidate's chance of passing a head of uniformly random bits. It gives 3 words for stages 0 - 5, 2 for 6 - 11
and 1 for 12 - 15, the fastest column of each row bar noise. The same table comes out for any cost ratio from about
500 to past 1,200. Whole passes on that table against uniform widths (one worker, scan ms): 3,860 at the table, 4,316 at
3 words everywhere, 4,636 at 4 and 8,582 at 2. With the final kernel, all owed: one worker 3,513 - 3,547 at the table,
3,831 - 3,865 at 3 words, 3,898 - 3,911 at 4 - 4 - 3 - 2 (stages in sixes, sixes, fours); 12 workers 558 - 568, 590 -
593 and 689 - 693.

**The plan's flat list, measured and dropped.** The plan had each run write (stored entry, eight queries) slots into a
list that fits in L1, and the kernel loop over the list, removing the loop exit of every (stored entry, probe), the
mispredict that dominated the old kernel. It did remove them, but every stored entry then costs about 78 instructions
to build its slots (the suffix, its conversions, the list's bounds and pointers reloaded from the stack) on top of
about 40 per slot. Counters on context switches, the whole pass, all owed:

| Kernel | Workers | Cycles | Instructions | Mispredicts | Scan (ms) |
|---|---:|---:|---:|---:|---:|
| step 2 (4 lanes, 128-bit head) | 1 | 18.87 G | 34.3 G | 131 M | 4,411 |
| flat list | 1 | 17.57 G | 43.8 G | 67 M | 3,994 |
| each stored entry's slots in a loop | 1 | **16.36 G** | 33.9 G | 95 M | **3,605** |
| step 2 | 12 | 25.15 G | 34.4 G | 134 M | 676 |
| flat list | 12 | 27.03 G | 43.9 G | 73 M | 709 |
| each stored entry's slots in a loop | 12 | **22.40 G** | 34.0 G | 97 M | **570** |

On one worker the list's fewer stalls outweighed its extra instructions; on 12, two threads to a core, the sibling
already filled those stalls and the instructions are what cost, as the one-worker section above foresaw for fixed trip
counts (at 6 workers, one to a core, the two were level). Testing every slot of the bucket for every stored entry, the
ordering rule as a lane mask, with no list and no suffix, was worse again (50.1 G instructions): the lanes before the
suffix are half the work in a pass where every picture is owed.

**Small buckets.** A query bucket of at most eight entries (most of them when a slice is 50,000 pictures, and nearly
all with 3,000 owed) is loaded once: its heads and pictures, and its lanes before the end. Each stored entry is then
one straight-line test, the ordering rule a mask of those lanes. Before that, these buckets took the list's or the
suffix's overhead per stored entry: four slices of 50,000 on one worker went from 9.5 s of scan to 6.3 s. The
few-owed path tests a hit on the full hash, kept beside the query's code: the few query pictures stay in cache, so a
head saves nothing there.

**Kept** (6 pairs per workload against step 2, alternating, a fresh catalog copy per run; medians in ms, cycles in M;
the change's Hodges-Lehmann estimate on cycles with its 95% interval):

| Workload | Pass | Cycles | Sort wait | Scan | Cycles, estimate |
|---|---:|---:|---:|---:|---|
| all owed, 1 worker | 5,808 -> 4,974 | 18,312 -> 15,728 | 708 -> 764 | 4,410 -> 3,540 | -14.0% [-15.5, -10.2] |
| all owed, 12 workers | 1,476 -> 1,476 | 25,168 -> 21,438 | 130 -> 272 | 670 -> 542 | -14.9% [-15.3, -14.4] |
| 300 owed, 1 | 566 -> 577 | 1,773 -> 1,814 | 228 -> 242 | 68 -> 64 | +4.9% [-3.6, +27.5] |
| 300 owed, 12 | 338 -> 344 | 3,300 -> 3,314 | 7 -> 6 | 70 -> 77 | +1.1% [-1.0, +2.0] |
| 3,000 owed, 1 | 896 -> 873 | 2,793 -> 2,754 | 234 -> 248 | 360 -> 342 | -1.6% [-3.6, +21.5] |
| 3,000 owed, 12 | 602 -> 582 | 3,966 -> 3,782 | 204 -> 212 | 82 -> 75 | -4.8% [-8.6, +0.7] |
| four slices of 50,000, 12 | 2,300 -> 2,025 | 51,278 -> 39,215 | 330 -> 466 | 1,478 -> 1,046 | -23.3% [-24.2, -22.6] |

No workload's interval lies above zero, and three are real gains. On 12 workers the full pass's wall clock does not
move: its scan is 19% shorter, and the time moves into the sort wait (130 -> 272 ms). The two sorters do not keep up
with the faster scan, and the columns cost the sort about 8% more on one worker. Step 5's thread team is what removes
that wait. The pairs are the oracle's 5,277, identical on all seven paths of the check.

---

## Where the other results are

- [START_PROFILE.md](START_PROFILE.md): where the start goes, thread by thread, and the startup before and after the
  thumbnail cache changes.
- [HEAP_PROFILE.md](HEAP_PROFILE.md): heap totals of a scan, an import and the borders and thumbnail passes.
- [JPEG_DECODERS.md](JPEG_DECODERS.md) and [JPEG_ENCODERS.md](JPEG_ENCODERS.md): the JPEG measurements.
- [THUMBNAILS.md](THUMBNAILS.md): the thumbnail resize, encode and priority measurements.
- [PROFILING_PLAN.md](../plans/PROFILING_PLAN.md): what is still to measure, and the leads not acted on.
