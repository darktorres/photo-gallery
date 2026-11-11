# The project's commands (https://just.systems): `just` lists them.
#
# This machine's values come from `.env` (copy `.env.example`), which `dotenv-load` puts in every recipe's environment.
# A variable already set in the real environment wins over `.env`.

set dotenv-load := true

# On Windows the recipes run under cmd, which every Windows has. just's default there is `sh`, which Git for Windows has
# but does not put on PATH, so a recipe run from PowerShell or cmd could not start. Under cmd, `#` starts no comment and
# single quotes quote nothing: a recipe line takes neither (comments go above it, arguments in double quotes).
set windows-shell := ["cmd.exe", "/d", "/c"]

# The pinned third-party build `just setup` fetches: BtbN's GPL shared build of FFmpeg 9.0 (the release's asset, and its
# SHA-256 as GitHub's digest). CI runs `just setup` too, so these are the only place the pin is written.
ffmpeg_url := "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-30-13-01/ffmpeg-n9.0.2-39-g7adfbe40dc-win64-gpl-shared-9.0.zip"
ffmpeg_sha256 := "7c09a4c6bd73a5013963f06b1c6175326605cbc2585444ebe0f740ef4f35b4c9"

# The Python the Linux launcher (tools/linux.py) runs under, on this machine (`just shots` and the lint use it too).
py := env('PG_PYTHON', if os() == "windows" { "python" } else { "python3" })

# Qt's kit folder (QT_DIR, Qt 6.12.0 msvc2022_64). Its bin goes on PATH for every recipe: cxx-qt-build finds qmake there,
# and the tests of the desktop crate need Qt's DLLs. Recipes that link Qt depend on `_qt`, which fails when it is unset.
qt := env('QT_DIR', '')
path_sep := if os() == "windows" { ";" } else { ":" }
export PATH := if qt == '' { env('PATH') } else { qt + '/bin' + path_sep + env('PATH') }

[private]
default:
    @just --list

# The pinned FFmpeg (the two values above) is downloaded once, checked against its SHA-256, and unpacked to .deps/ffmpeg.
# To move to another pin, delete .deps/ffmpeg and change the two values.

# Fetch the pinned FFmpeg into .deps/ffmpeg (tools/setup_ffmpeg.py), unless it is already there.
setup:
    {{ py }} tools/setup_ffmpeg.py --url {{ ffmpeg_url }} --sha256 {{ ffmpeg_sha256 }} --destination .deps/ffmpeg

# The Python tools' own tests (tools/test_*.py, stdlib unittest).
test-tools:
    {{ py }} -m unittest discover -s tools -p "test_*.py"

# Says which of this machine's prerequisites are missing and how to fix each one (tools/doctor.py). Builds nothing.
doctor:
    {{ py }} tools/doctor.py

# Build the app: `release` (default) or `debug` (reads the QML from crates/desktop/qml and reloads it on change).
build profile="release": _qt
    cargo build {{ if profile == "release" { "--release" } else { "" } }}

# Build, then put the Qt DLLs and QML modules next to gallery.exe (needed once per profile, and after Qt updates).
# qoffscreen is included on purpose: the test hooks run the window offscreen (QT_QPA_PLATFORM=offscreen), and windeployqt
# otherwise deploys only qwindows.
# Not Qt's FFmpeg: the build already put ours there (crates/README.md "Build"). `just deploy release pdb` also puts
# Qt's PDBs there (Qt's "Debug Information Files", when installed): Pin reads a DLL's PDB only from beside it, for
# the instruction profiles (tools/profiling/callgraph).
deploy profile="release" pdb="": (build profile)
    {{ qt }}/bin/windeployqt.exe --release {{ if pdb == "pdb" { "--pdb" } else { "" } }} --qmldir crates/desktop/qml --no-translations --no-ffmpeg --include-plugins qoffscreen target/{{ profile }}/gallery.exe

# Run the release build: the app, or a command (`just run scan D:\Photos`, `just run status`).
run *args: _qt
    cargo run --release -- {{ args }}

# Run the debug build, whose window reloads when a QML file changes.
dev *args: _qt
    cargo run -- {{ args }}

# The unit and integration tests. Qt's bin is on PATH (above): pg-desktop's tests link Qt and need its DLLs. FFmpeg isn't
# on PATH: pg-video loads its DLLs by full path (crates/video/src/ffi.rs), so no PATH order decides which copy runs.
test *args: _qt
    cargo test --workspace --locked --timings {{ args }}

# The eval tools' Python tests (stdlib unittest). The viewer imports Pillow. CI runs it in the Windows job.
test-eval:
    {{ py }} -m unittest discover -s eval -p "test_*.py"

# Format the Rust and the Python code.
fmt:
    cargo fmt --all
    ruff format .

# The lint tools' versions. ruff and markdownlint are pinned in tools/lint (their lockfiles, which CI installs from);
# cargo-deny and cargo-machete are pinned here, and CI's install step names the same two.
cargo_deny_version := "0.20.2"
cargo_machete_version := "0.9.2"

# The Linux container's pins (docker/Dockerfile): the launcher reads them from here with `just --evaluate`, so each is
# written once. CI's Linux job runs the same image, and its `just` is the one in the install step (just_version).
just_version := "1.59.0"
cargo_fuzz_version := "0.13.2"
qt_linux_version := "6.12.0"
# The aqtinstall commit that installs Qt for Linux, as CI's jurplel/install-qt-action pins it for the Windows job.
aqtinstall_commit := "076e1659807d0b362a3ed684d54c2e9c775eb9c7"

# ruff goes in `.venv` and markdownlint in `tools/lint/node_modules` (both git-ignored), so other projects on this machine
# keep their own versions. Run it once, and again after a pin changes.

# Installs ruff and markdownlint at their pinned versions, inside the repository (tools/check_lint_tools.py --install).
lint-tools-setup:
    {{ py }} tools/check_lint_tools.py --install

# Fails with the fix when QT_DIR is missing or has no qmake (tools/check_qt.py).
[private]
_qt:
    {{ py }} tools/check_qt.py

# Fails when a lint tool is not the version CI uses, in the repository (`just lint-tools-setup`) or on PATH. That is every
# tool the lint and the module check run: ruff and markdownlint from the lockfiles, and the four pinned here (just itself
# included, so a `just` other than CI's is caught the first time a recipe runs).
[private]
_lint-tools:
    {{ py }} tools/check_lint_tools.py --cargo-deny {{ cargo_deny_version }} --cargo-machete {{ cargo_machete_version }} --just {{ just_version }} --cargo-modules {{ cargo_modules_version }}

# Every check: formatting, clippy, dependencies, the crate boundaries (tools/boundaries.py: the crate layers, acyclic dev
# edges, re-exports, platform code), the Python, the Markdown and the QML (tools/lint.py, every OS). cargo-machete reads
# only our crates: vendor/ keeps the upstream's lints (vendor/README.md).
lint: _qt _lint-tools && lint-qml
    {{ py }} tools/lint.py

# The module-cycle check (docs/MODULARITY_PLAN.md, M-0): the compiler's module graph of each library crate on this
# machine's target (tools/modgraph/compiler_graph.py), and the text scan (tools/modgraph/module_graph.py), which sees the
# cxx-qt bridge modules the compiler does not. cargo-modules is a tool, not a dependency: its version is pinned here and
# in CI's install steps (.github/workflows/ci.yml; bump both; the container reads it from here). A missing or different
# one is installed at the pin. A cycle group that is not on the allowlist (tools/modgraph-allow.txt) fails the check, and
# the allowlist is empty (M-16): any module cycle fails. CI runs it on both targets.
cargo_modules_version := "0.28.0"

# The module-cycle check (above). Its install of cargo-modules does nothing when the pinned version is installed.
modgraph: _qt
    cargo install --locked cargo-modules --version {{ cargo_modules_version }}
    {{ py }} tools/modgraph/module_graph.py
    {{ py }} tools/modgraph/compiler_graph.py

# The survey a modularity review starts from (docs/MODULARITY_REVIEW_4.md, M-48): every public item and where it is
# used (tools/modgraph/api_surface.py, which needs cargo-modules; the JSON goes to target/modgraph), every internal
# dependency edge and what goes through it (edges.py), and the duplicate code (dupes.py). Reports, not gates: they match by name.
modgraph-report: _qt
    {{ py }} tools/modgraph/api_surface.py
    {{ py }} tools/modgraph/edges.py
    {{ py }} tools/modgraph/dupes.py

# Screenshot comparison: the test-hook matrix on one fixture, the working tree against a recorded reference, pixel by
# pixel (tools/compare_shots.py). `just shots --record <commit>` builds that commit and records its screenshots under
# tools/shots/reference (once); `just shots` then builds only the working tree. `--expect a,b` declares scenarios
# that are expected to differ; `--self` runs the working tree against itself; `--only a,b` runs some scenarios.
shots *args: _qt
    {{ py }} tools/compare_shots.py {{ args }}

# The QML tests (S-14.3): Qt Quick Test on the QML that stays in QML (crates/desktop/tests/qml), offscreen, against the
# components staged as the pg.gallery module (tools/qmltest.py). The stubs stand in for the Rust types, so the tests
# check the QML and nothing Rust.
qmltest: _qt
    {{ py }} tools/qmltest.py

# The Linux checks, the callgrind benches and the fuzzer run in the pg-dev container (docker/, tools/linux.py). It is the
# only Linux environment this repository uses. Needs Docker Desktop running; `just linux-build` builds the image.

# The Linux job of CI, step by step, on the working tree (docker/check.py). `check-linux-head` checks the committed HEAD.
check-linux *args:
    {{ py }} tools/linux.py check {{ args }}

check-linux-head *args:
    {{ py }} tools/linux.py check --head {{ args }}

# Builds the image (docker/Dockerfile) at the pins above. The other recipes build it when it is missing.
linux-build *args:
    {{ py }} tools/linux.py build {{ args }}

# The instruction-count gate (S-16.2): the callgrind benches in the container, compared with docs/perf/baseline.json at
# ±1% (tools/perf_check.py). After a change that should move the counts: `just perf --update`. The bench data is the
# folder PG_BENCH_DATA_SOURCE names in .env, mounted read-only into the container.
perf *args:
    {{ py }} tools/linux.py perf {{ args }}

# Each fuzz target for a short time, in the container (fuzz/, tools/fuzz_smoke.py): the fuzz smoke test (S-18.3).
# `just fuzz-smoke 300` runs five minutes per target.
fuzz-smoke *args:
    {{ py }} tools/linux.py fuzz {{ args }}

# The callgrind benches without the gate: gungraun's options (`just linux-bench --save-baseline=before`). Saves no archive
# for a filtered run (docs/PROFILING.md).
linux-bench *args:
    {{ py }} tools/linux.py bench {{ args }}

# An eval script's Linux part, in the container's eval image (docker/README.md): the JPEG comparisons
# (`just linux-eval eval/jpeg-decoders/linux_build.py`, then `linux.py`, `harness_linux.py`, `restart.py`, `encoders.py`
# with their steps). Its sources, builds and inputs stay in the pg-eval volume; JPEG_SRC_DIR and PG_JPEG_ENC_DIR (.env)
# are mounted when set. The ANN bake-off's CPU worker runs the same way, through `tools/linux.py ann-worker`.
linux-eval *args:
    {{ py }} tools/linux.py eval {{ args }}

# A shell in the container, on the working tree.
linux-shell:
    {{ py }} tools/linux.py shell

# qmllint and the QML compiler on the QML, failing on any warning (tools/lint_qml.py, every OS; build first).
lint-qml: _qt
    {{ py }} tools/lint_qml.py
