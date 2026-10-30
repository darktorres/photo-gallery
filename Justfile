# The project's commands (https://just.systems): `just` lists them.

set windows-shell := ["pwsh", "-NoLogo", "-NoProfile", "-Command"]

# The Qt kit .cargo/config.toml builds against.
qt := "C:/Qt/6.12.0/msvc2022_64"

[private]
default:
    @just --list

# Build the app: `release` (default) or `debug` (reads the QML from crates/ui/qml and reloads it on change).
build profile="release":
    cargo build {{ if profile == "release" { "--release" } else { "" } }}

# Build, then put the Qt DLLs and QML modules next to gallery.exe (needed once per profile, and after Qt updates).
deploy profile="release": (build profile)
    {{ qt }}/bin/windeployqt.exe --release --qmldir crates/ui/qml --no-translations target/{{ profile }}/gallery.exe

# Run the release build: the app, or a command (`just run scan D:\Photos`, `just run status`).
run *args:
    cargo run --release -- {{ args }}

# Run the debug build, whose window reloads when a QML file changes.
dev *args:
    cargo run -- {{ args }}

# The unit and integration tests.
test *args:
    cargo test --workspace {{ args }}

# Format the Rust code.
fmt:
    cargo fmt --all

# Every check: formatting, clippy, dependencies (cargo-deny, cargo-machete), the Python and the Markdown.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo deny check
    cargo machete
    ruff check .
    markdownlint --ignore target --ignore .playwright-mcp docs "**/README.md"
