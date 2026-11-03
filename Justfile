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
# Not Qt's FFmpeg: the build already put ours there (crates/README.md "Build"). `just deploy release pdb` also puts
# Qt's PDBs there (Qt's "Debug Information Files", when installed): Pin reads a DLL's PDB only from beside it, for
# the instruction profiles (tools/profiling/callgraph).
deploy profile="release" pdb="": (build profile)
    {{ qt }}/bin/windeployqt.exe --release {{ if pdb == "pdb" { "--pdb" } else { "" } }} --qmldir crates/desktop/qml --no-translations --no-ffmpeg target/{{ profile }}/gallery.exe

# Run the release build: the app, or a command (`just run scan D:\Photos`, `just run status`).
run *args:
    cargo run --release -- {{ args }}

# Run the debug build, whose window reloads when a QML file changes.
dev *args:
    cargo run -- {{ args }}

# The unit and integration tests.
test *args:
    cargo test --workspace {{ args }}

# Format the Rust and the Python code.
fmt:
    cargo fmt --all
    ruff format .

# Every check: formatting, clippy, dependencies (cargo-deny, cargo-machete), the Python, the Markdown and the QML.
# cargo-machete reads only our crates: vendor/ keeps the upstream's lints (vendor/README.md).
lint: && lint-qml
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    $env:RUSTDOCFLAGS = "-D warnings"; cargo doc --workspace --no-deps --document-private-items
    cargo deny check
    cargo machete crates
    ruff format --check .
    ruff check .
    markdownlint --ignore target --ignore .playwright-mcp --ignore vendor docs "**/README.md"

# Clippy (-D warnings) for every crate but the desktop, in WSL, where the Linux-only code compiles (tools/wsl-check.sh).
wsl-check:
    wsl -d Ubuntu-26.04 -u root -- bash tools/wsl-check.sh

# qmllint and the QML compiler on the QML, failing on any warning or anything not compiled to C++. Uses the
# pg.gallery types of the last build, so build first.
lint-qml:
    pwsh -NoLogo -NoProfile -File tools/lint-qml.ps1 -Qt {{ qt }}
