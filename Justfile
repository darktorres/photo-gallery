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

# The WSL distro the callgrind, fuzz and WSL-check recipes run in (PG_WSL_DISTRO overrides it).
wsl_distro := env('PG_WSL_DISTRO', 'Ubuntu-26.04')

# Qt's kit folder (QT_DIR, Qt 6.12.0 msvc2022_64). Its bin goes on PATH for every recipe: cxx-qt-build finds qmake there,
# and the tests of the desktop crate need Qt's DLLs. Recipes that link Qt depend on `_qt`, which fails when it is unset.
qt := env('QT_DIR', '')
export PATH := if qt == '' { env('PATH') } else { qt + '/bin;' + env('PATH') }

[private]
default:
    @just --list

# Fails with the fix when QT_DIR is missing or has no qmake.
[private]
_qt:
    @if (-not $env:QT_DIR) { throw 'QT_DIR is not set: put the Qt kit folder in .env (QT_DIR=C:/Qt/6.12.0/msvc2022_64)' }; if (-not (Test-Path "$env:QT_DIR/bin/qmake.exe")) { throw "no qmake.exe in $env:QT_DIR/bin: QT_DIR must be the Qt 6.12.0 msvc2022_64 kit" }

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

# Format the Rust and the Python code.
fmt:
    cargo fmt --all
    ruff format .

# The lint tools' versions. CI installs the same ones (.github/workflows/ci.yml, and pip and npm in its Linux job): bump both.
ruff_version := "0.17.3"
markdownlint_version := "0.50.0"
cargo_deny_version := "0.20.2"
cargo_machete_version := "0.9.2"

# Fails when a lint tool is not the version above (tools/check-lint-tools.ps1).
[private]
_lint-tools:
    pwsh -NoLogo -NoProfile -File tools/check-lint-tools.ps1 -Ruff {{ ruff_version }} -Markdownlint {{ markdownlint_version }} -CargoDeny {{ cargo_deny_version }} -CargoMachete {{ cargo_machete_version }}

# Every check: formatting, clippy, dependencies (cargo-deny, cargo-machete), the Python, the Markdown and the QML.
# cargo-machete reads only our crates: vendor/ keeps the upstream's lints (vendor/README.md).
lint: _qt _lint-tools && lint-qml
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked --timings -- -D warnings
    cargo check -p pg-model -p pg-client --locked --target wasm32-unknown-unknown
    $env:RUSTDOCFLAGS = "-D warnings"; cargo doc --workspace --no-deps --document-private-items --locked --timings
    cargo deny check
    cargo machete crates
    ruff format --check .
    ruff check .
    markdownlint "**/*.md"

# The checks only Windows can run, for CI's Windows job: clippy and the docs over the code that compiles only there
# (cfg(windows): gallery, platform, desktop), and the QML. The other checks of `lint` run once, in CI's Linux job.
lint-windows: _qt && lint-qml
    cargo clippy --workspace --all-targets --locked --timings -- -D warnings
    $env:RUSTDOCFLAGS = "-D warnings"; cargo doc --workspace --no-deps --document-private-items --locked --timings

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

# The instruction-count gate (S-16.2): the callgrind benches in WSL, compared with docs/perf/baseline.json at ±1%
# (tools/perf-check.sh). After a change that should move the counts: `just perf --update`. The bench data comes from
# PG_BENCH_DATA_SOURCE (a Windows path in .env); WSLENV passes it to WSL as the WSL path.
perf *args:
    $env:WSLENV = 'PG_BENCH_DATA_SOURCE/p:' + $env:WSLENV; wsl -d {{ wsl_distro }} -u root -- bash tools/perf-check.sh {{ args }}

# Each fuzz target for a short time, in WSL (fuzz/, tools/fuzz-smoke.sh): the fuzz smoke test (S-18.3). `just fuzz-smoke 300`
# for five minutes per target.
fuzz-smoke *args:
    wsl -d {{ wsl_distro }} -u root -- bash tools/fuzz-smoke.sh {{ args }}

# Clippy (-D warnings) and the tests for every crate but the desktop, in WSL, where the Linux-only code compiles (tools/wsl-check.sh).
wsl-check:
    wsl -d {{ wsl_distro }} -u root -- bash tools/wsl-check.sh

# qmllint and the QML compiler on the QML, failing on any warning or anything not compiled to C++. Uses the
# pg.gallery types of the last build, so build first.
lint-qml: _qt
    pwsh -NoLogo -NoProfile -File tools/lint-qml.ps1
