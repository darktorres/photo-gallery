# The project's commands (https://just.systems): `just` lists them.

set windows-shell := ["pwsh", "-NoLogo", "-NoProfile", "-Command"]

# The Qt kit .cargo/config.toml builds against.
qt := "C:/Qt/6.12.0/msvc2022_64"

[private]
default:
    @just --list

# Build the app: `release` (default) or `debug` (reads the QML from crates/desktop/qml and reloads it on change).
build profile="release":
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
run *args:
    cargo run --release -- {{ args }}

# Run the debug build, whose window reloads when a QML file changes.
dev *args:
    cargo run -- {{ args }}

# The unit and integration tests. Qt's bin is on PATH: pg-desktop's tests link Qt and need its DLLs. One line: `just`
# runs each line of a recipe in its own shell, so a PATH set on one line is gone by the next.
test *args:
    $env:PATH = "{{ qt }}/bin;" + $env:PATH; cargo test --workspace {{ args }}

# Format the Rust and the Python code.
fmt:
    cargo fmt --all
    ruff format .

# Every check: formatting, clippy, dependencies (cargo-deny, cargo-machete), the Python, the Markdown and the QML.
# cargo-machete reads only our crates: vendor/ keeps the upstream's lints (vendor/README.md).
lint: && lint-qml
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo check -p pg-model -p pg-client --target wasm32-unknown-unknown
    $env:RUSTDOCFLAGS = "-D warnings"; cargo doc --workspace --no-deps --document-private-items
    cargo deny check
    cargo machete crates
    ruff format --check .
    ruff check .
    markdownlint --ignore target --ignore .playwright-mcp --ignore vendor docs "**/README.md"

# Screenshot comparison: the test-hook matrix on one fixture, the working tree against a recorded reference, pixel by
# pixel (tools/compare-shots.ps1). `just shots -Record <commit>` builds that commit and records its screenshots under
# tools/shots/reference (once); `just shots` then builds only the working tree. `-Expect a,b` declares scenarios
# that are expected to differ; `-Self` runs the working tree against itself; `-Only a,b` runs some scenarios.
shots *args:
    pwsh -NoLogo -NoProfile -File tools/compare-shots.ps1 {{ args }}

# The QML tests (S-14.3): Qt Quick Test on the QML that stays in QML (crates/desktop/tests/qml), offscreen, against the
# components staged as the pg.gallery module (tools/qmltest.ps1). The stubs stand in for the Rust types, so the tests
# check the QML and nothing Rust.
qmltest:
    pwsh -NoLogo -NoProfile -File tools/qmltest.ps1 -Qt {{ qt }}

# The instruction-count gate (S-16.2): the callgrind benches in WSL, compared with docs/perf/baseline.json at ±1%
# (tools/perf-check.sh). After a change that should move the counts: `just perf --update`.
perf *args:
    wsl -d Ubuntu-26.04 -u root -- bash tools/perf-check.sh {{ args }}

# Each fuzz target for a short time, in WSL (fuzz/, tools/fuzz-smoke.sh): the nightly gate (S-18.3). `just fuzz-smoke 300`
# for five minutes per target.
fuzz-smoke *args:
    wsl -d Ubuntu-26.04 -u root -- bash tools/fuzz-smoke.sh {{ args }}

# Clippy (-D warnings) and the tests for every crate but the desktop, in WSL, where the Linux-only code compiles (tools/wsl-check.sh).
wsl-check:
    wsl -d Ubuntu-26.04 -u root -- bash tools/wsl-check.sh

# qmllint and the QML compiler on the QML, failing on any warning or anything not compiled to C++. Uses the
# pg.gallery types of the last build, so build first.
lint-qml:
    pwsh -NoLogo -NoProfile -File tools/lint-qml.ps1 -Qt {{ qt }}
