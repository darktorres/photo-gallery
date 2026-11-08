#!/bin/bash
# The Linux job of CI (.github/workflows/ci.yml), step by step, in the container's /work (the repository). Each step
# stops the run on failure, as in CI. Run it from the repository root, with the container from docker/README.md:
#   docker run --rm --security-opt systempaths=unconfined -v "$PWD:/work" -v pg-cargo-target:/cargo-target -v pg-cargo-registry:/root/.cargo/registry pg-dev
set -euo pipefail

step() { printf '\n== %s\n' "$*"; }

step "Formatting"
cargo fmt --all --check

step "Clippy and tests (the whole workspace)"
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

step "Docs (warnings are errors)"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked

step "wasm32 (the shared model and the client stay free of platform code)"
cargo check -p pg-model -p pg-client --locked --target wasm32-unknown-unknown

step "Lockfiles of the workspaces outside the root (fuzz, eval/jpeg-decoders)"
cargo metadata --locked --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null
cargo metadata --locked --manifest-path eval/jpeg-decoders/Cargo.toml --format-version 1 >/dev/null

step "Supply chain (licences, advisories, unused dependencies)"
cargo deny check
cargo machete crates

step "Python and Markdown"
ruff format --check .
ruff check .
markdownlint "**/*.md"

printf '\nAll Linux checks passed.\n'
