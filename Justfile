# The project's commands (https://just.systems): `just` lists them.
#
# This machine's values come from `.env` (copy `.env.example`), which `dotenv-load` puts in every recipe's environment.
# A variable already set in the real environment wins over `.env`.

set dotenv-load := true
set windows-shell := ["pwsh", "-NoLogo", "-NoProfile", "-Command"]

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

# Fetch the pinned FFmpeg into .deps/ffmpeg (tools/setup-ffmpeg.ps1), unless it is already there.
setup:
    pwsh -NoLogo -NoProfile -File tools/setup-ffmpeg.ps1 -Url {{ ffmpeg_url }} -Sha256 {{ ffmpeg_sha256 }} -Destination .deps/ffmpeg

# Says which of this machine's prerequisites are missing and how to fix each one (tools/doctor.ps1). Builds nothing.
doctor:
    pwsh -NoLogo -NoProfile -File tools/doctor.ps1

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
    python -m unittest discover -s eval -p 'test_*.py'

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

# Fails when a lint tool is not the version CI uses, in the repository (`just lint-tools-setup`) or on PATH.
[private]
_lint-tools:
    {{ py }} tools/check_lint_tools.py --cargo-deny {{ cargo_deny_version }} --cargo-machete {{ cargo_machete_version }}

# cargo-machete reads only our crates: vendor/ keeps the upstream's lints (vendor/README.md).

# Every check: formatting, clippy, dependencies, the Python, the Markdown and the QML (tools/lint.py, every OS).
lint: _qt _lint-tools && lint-qml
    {{ py }} tools/lint.py

# The module-cycle check (docs/MODULARITY_PLAN.md, M-0): the compiler's module graph of each library crate on this
# machine's target (tools/modgraph/compiler_graph.py), and the text scan (tools/modgraph/module_graph.py), which sees the
# cxx-qt bridge modules the compiler does not. cargo-modules is a tool, not a dependency: its version is pinned here and
# in docker/Dockerfile (bump both). A missing or different one is installed at the pin. A cycle group that is not on the
# allowlist (tools/modgraph-allow.txt) fails the check; the allowlist empties as M-1 to M-20 land (M-16).
cargo_modules_version := "0.28.0"

modgraph: _qt
    if (((cargo modules --version 2>$null) -split '\s+' | Select-Object -Last 1) -ne '{{ cargo_modules_version }}') { cargo install --locked cargo-modules --version {{ cargo_modules_version }} }
    python tools/modgraph/module_graph.py
    python tools/modgraph/compiler_graph.py

# Screenshot comparison: the test-hook matrix on one fixture, the working tree against a recorded reference, pixel by
# pixel (tools/compare-shots.ps1). `just shots -Record <commit>` builds that commit and records its screenshots under
# tools/shots/reference (once); `just shots` then builds only the working tree. `-Expect a,b` declares scenarios
# that are expected to differ; `-Self` runs the working tree against itself; `-Only a,b` runs some scenarios.
shots *args: _qt
    pwsh -NoLogo -NoProfile -File tools/compare-shots.ps1 {{ args }}

# The QML tests (S-14.3): Qt Quick Test on the QML that stays in QML (crates/desktop/tests/qml), offscreen, against the
# components staged as the pg.gallery module (tools/qmltest.ps1). The stubs stand in for the Rust types, so the tests
# check the QML and nothing Rust.
qmltest: _qt
    pwsh -NoLogo -NoProfile -File tools/qmltest.ps1

# The Linux checks, the callgrind benches and the fuzzer run in the pg-dev container (docker/, tools/linux.py). It is the
# only Linux environment this repository uses. Needs Docker Desktop running; `just linux-build` builds the image.

# The Linux job of CI, step by step, on the working tree (docker/check.sh). `check-linux-head` checks the committed HEAD.
check-linux *args:
    {{ py }} tools/linux.py check {{ args }}

check-linux-head *args:
    {{ py }} tools/linux.py check --head {{ args }}

# Builds the image (docker/Dockerfile) at the pins above. The other recipes build it when it is missing.
linux-build *args:
    {{ py }} tools/linux.py build {{ args }}

# The instruction-count gate (S-16.2): the callgrind benches in the container, compared with docs/perf/baseline.json at
# ±1% (tools/perf-check.sh). After a change that should move the counts: `just perf --update`. The bench data is the
# folder PG_BENCH_DATA_SOURCE names in .env, mounted read-only into the container.
perf *args:
    {{ py }} tools/linux.py perf {{ args }}

# Each fuzz target for a short time, in the container (fuzz/, tools/fuzz-smoke.sh): the fuzz smoke test (S-18.3).
# `just fuzz-smoke 300` runs five minutes per target.
fuzz-smoke *args:
    {{ py }} tools/linux.py fuzz {{ args }}

# The callgrind benches without the gate: gungraun's options (`just linux-bench --save-baseline=before`). Saves no archive
# for a filtered run (docs/PROFILING.md).
linux-bench *args:
    {{ py }} tools/linux.py bench {{ args }}

# A shell in the container, on the working tree.
linux-shell:
    {{ py }} tools/linux.py shell

# qmllint and the QML compiler on the QML, failing on any warning (tools/lint_qml.py, every OS; build first).
lint-qml: _qt
    {{ py }} tools/lint_qml.py
