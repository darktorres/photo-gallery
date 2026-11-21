# The repository's tools

Every script here is Python (standard library, except where a row says otherwise), and runs the same way on Windows and
Linux unless its row says Windows only. The Justfile calls most of them through `{{ py }}` (`PG_PYTHON` in `.env`, else
`python` on Windows and `python3` elsewhere); `just --list` names the recipes. Run a script with `--help` to see its
arguments, where it has any.

Exit codes, as the profiling tools use them (`docs/plans/DECODE_ONCE_PLAN.md` section 6 gives the plan's reasons): 0 ok;
1 failed (a measured or compared result is bad); 2 usage or a missing argument; 3 an input is unreadable or has the wrong
shape. The other tools end with `sys.exit(message)`, which exits 1.

| Script | What it does | OS | How it runs |
|---|---|---|---|
| `pgtools.py` | The helpers the other tools share: the repository's folder, the `.env` loader, a command runner, the MSVC environment | both | imported |
| `check_qt.py` | Fails with the fix when `QT_DIR` is not the Qt kit the Justfile pins (`qt_version`; its `bin/qmake` must report that version) | both | `_qt` in the recipes that need Qt |
| `check_lint_tools.py` | The lint tools at their pins: ruff and markdownlint from `.venv` and `.tools` (or PATH); `--install` makes them, from their lockfiles | both | `just lint-tools-setup`, `just lint` |
| `lint.py` | The lint steps, the same list on every OS (fmt, clippy, wasm32, docs, the lockfiles, machete, ruff, markdownlint, the crate boundaries; cargo-deny is `just deny`, its own CI step) | both | `just lint`, `docker/check.py` |
| `boundaries.py` | The workspace's structural rules: the crate layers (`crate-layers.txt`), acyclic dev edges, re-exports, and the platform rule (OS calls and branches only in pg-platform) | both | `lint.py` |
| `lint_qml.py` | The QML lint: qmllint and the QML compiler, failing on any warning | both | `just lint-qml` |
| `doctor.py` | Checks the machine for what the build and tests need, and says how to fix each missing part | both | `just doctor` |
| `setup_ffmpeg.py` | Fetches the pinned FFmpeg build: the archive, its SHA-256 checked before anything is unpacked | Windows | `just setup`, CI |
| `qmltest.py` | The QML tests: Qt Quick Test, offscreen, on the QML that stays in QML | both | `just qmltest`, CI |
| `compare_shots.py` | The screenshot comparison: the working tree against a recorded reference, pixel by pixel, from two template catalogs per side (`default`, and `broken` for the inbox scenarios); the manifest keeps a hash per template and for `shots/shot.log` | both | `just shots` (`--record`, `--base`, `--self`, `--only`, `--expect`, `--python`) |
| `shots/fixture.py` | The screenshot matrix's fixture: the synthetic library, and the catalog's views and settle checks | both | `compare_shots.py` |
| `linux.py` | The Linux side, in the `pg-dev` container: the checks of CI's Linux job, the benches, the fuzz run, and the eval's Linux parts (the JPEG comparisons, the ANN bake-off's CPU worker) | both (drives Docker) | `just check-linux`, `just linux-build`, `just perf`, `just linux-bench`, `just fuzz-smoke`, `just linux-eval`, `just linux-shell`; `eval/ann/methods.py` (`ann-worker`) |
| `perf_check.py` | The instruction-count gate: the benches of the committed HEAD against the baseline, at ±1%, or with `--budgets` each bench named in `docs/perf/budgets.json` against its budget | Linux (container) | `just perf` through `linux.py` |
| `fuzz_smoke.py` | Each fuzz target (`fuzz/`) for a short time | Linux (container) | `just fuzz-smoke` through `linux.py` |
| `perf-compare.py` | The instruction-count comparison (a baseline against a run, at a tolerance or per-bench budgets) | both | `perf_check.py`, `scan_timing.py`; by hand |
| `fork-updates.py` | Checks the forks under github.com/darktorres against the newest release of each crate upstream | both | `.github/workflows/fork-updates.yml`; by hand |
| `make_icon.py` | Makes the app icon from mascot 03, as a multi-size Windows icon | both | by hand, after changing the mascot |
| `make_test_video.py` | The video reader's test input (`crates/video/tests/sample.mp4`), made with the FFmpeg on PATH | both | by hand |
| `ci/github_files.py` | Writes `GITHUB_ENV` and `GITHUB_PATH` for the steps after it (the content keys, the FFmpeg `bin`) | CI | `.github/workflows/ci.yml` |
| `ci/eval_pins.py` | Installs the Pillow and numpy pins of `eval/requirements.txt` for CI's eval tests | CI | `.github/workflows/ci.yml` |
| `profiling/ci_setup.py` | The Linux system setup of CI: the apt packages, the FFmpeg and libclang links, Rust | Linux | `.github/workflows/ci.yml` |
| `profiling/bench.py` | The callgrind benches of the committed HEAD, in the container | Linux (container) | `linux.py bench`, `perf_check.py` |
| `profiling/bench_data.py` | The catalog benches' data, from a catalog folder; a `-large` layout gets its failure ledger seeded (`--ledger-rows`, 100,000 by default) | both | by hand, before `just perf` (`docs/reference/PROFILING.md`) |
| `profiling/make_bench_jpeg.py` | The per-file benches' input: a deterministic synthetic photo | both | `bench.py`; by hand |
| `profiling/scan_timing.py` | The wall-clock timing of a library scan (three timed scans, each into its run's catalog) | Linux | by hand (`docs/reference/PROFILING.md`) |
| `profiling/synthetic_library.py` | A synthetic photo library for profiling: small by default, or `--profile large` (200,000 files, a quarter of them broken JPEGs, `--broken`) for the large bench data | both | `scan_timing.py`, the shots fixture, `bench_data.py`'s data |
| `profiling/cgtab.py` | Callgrind output as tables: self and inclusive cost, and the diff of two runs | both | by hand (`docs/reference/PROFILING.md`) |
| `profiling/callgraph/build.py` | Builds `callgraph.dll` into `target/pin` with the Pin kit (clang-cl, LLVM) | Windows only | by hand (`docs/reference/PROFILING.md`) |
| `profiling/callgraph/profile.py` | The instruction profile of the gallery under Pin, one file per thread | Windows only | by hand (`docs/reference/PROFILING.md`) |
| `profiling/callgraph/symbolize.py` | Turns `callgraph.dll`'s profiles into named ones, from the PDBs (dbghelp) | Windows only | `profile.py` |
| `build_jpeg_turbo.py` | Builds libjpeg-turbo (JPEG 8.0, static, the dynamic CRT) for Qt's JPEG plugin and the app's turbojpeg | Windows only (MSVC) | by hand, into `PG_HOME\static\jpeg-turbo` or `--work` |
| `build_static_ffmpeg.py` | Builds the static FFmpeg 9.0 for pg-video and Qt Multimedia, with its CRT and link checks | Windows only (MSVC, MSYS2) | by hand, into `PG_HOME\static\ffmpeg` or `--work` |
| `build_static_qt.py` | Builds the static Qt 6.12.0 (the pinned source, checked by SHA-256), on the libjpeg-turbo and static FFmpeg prefixes; `--configure-only` stops after configure | Windows only (MSVC, MSYS2's Perl) | by hand, into `PG_HOME\static\qt` or `--work`, with `--jpeg` and `--ffmpeg` |
| `modgraph/module_graph.py` | The intra-crate module graph: which module refers to which | both | `just modgraph`, `docker/check.py` |
| `modgraph/compiler_graph.py` | The compiler's module graph and its cycle groups (`cargo modules`) | both | `just modgraph`, `docker/check.py` |
| `modgraph/allowlist.py` | The module-cycle allowlist (`modgraph-allow.txt`, empty: any module cycle fails) | both | `just modgraph`, `docker/check.py` |
| `modgraph/split_check.py` | The group-level check of a planned split of one Rust file | both | by hand (`the modularity plan (git e428ab2)`) |
| `modgraph/make_html.py` | A self-contained `graph.html` of the crate and module graphs | both | by hand |
| `modgraph/unreachable_pub.py` | Counts rustc's `unreachable_pub` warnings per crate | both | by hand (`the modularity plan (git e428ab2)`) |
| `test_setup_ffmpeg.py`, `test_ci_files.py` | Unit tests of `setup_ffmpeg.py` and the CI helpers | both | `just test-tools` |

The Python tools' tests run with `just test-tools` (stdlib `unittest`, the `test_*.py` files here). `eval/` has its own
tests (`just test-eval`).

## Folders and files

- `static-ffmpeg/static-check.c`: the link check that `build_static_ffmpeg.py` compiles against the static libraries.
- `static-qt/*.patch`: the static build's edits to the crate sources (`app-build.patch` to `crates/desktop`, `pg-video.patch` to `crates/video`; the diffs name those paths), not to Qt's sources. `build_static_qt.py` has no patch step, so apply them from the repo root with `git apply` before it runs.
- `lint/`: the lint tools at their pins: `requirements.txt` (ruff, with hashes) and `package.json` with its lockfile
  (markdownlint-cli). `just lint-tools-setup` installs from them.
- `shots/`: the screenshot fixture and its reference recordings (`reference/<sha10>/`), and the inputs kept outside
  `fixture.py` so its hash doesn't move for them: `broken/broken.jpg` (a valid SOI and APP0, then deterministic bytes
  that are no JPEG: the import fails it, with the same error every time), which the `broken` template adds as a second
  library folder from the fixed `target/shots/broken`, and `shot.log` (JSON records at fixed times, every level, a
  multi-line detail, legacy plain lines), which the `log` scenario shows through `--log-view`; and `lanes.txt` (the status
  bar's rows: a lane working, one paused with no known total, one paused and idle, one in its hold, and the crop), which
  the `lanes` and `pseudo-lanes` scenarios show through `--lane-rows`.
- `crate-layers.txt`: each crate's allowed normal dependencies on the workspace's crates, bottom layer first
  (`boundaries.py`).
- `profiling/`, `modgraph/`: as in the table.

## PowerShell originals, kept until a Windows run confirms their Python ports

These are not run by anything now. Each is removed once its replacement has run on Windows (the plan's rule):
`doctor.ps1` (`doctor.py`), `qmltest.ps1` (`qmltest.py`),
`compare-shots.ps1` (`compare_shots.py`), `build-jpeg-turbo.ps1` (`build_jpeg_turbo.py`),
`build-static-ffmpeg.ps1` (`build_static_ffmpeg.py`), `profiling/callgraph/profile.ps1` (`profile.py`), and
`profiling/callgraph/build.py`. `eval/jpeg-decoders/win-build.cmd` and `win-env.cmd` (`win_build.py`,
`win_env.py`) are the same kind of original.
