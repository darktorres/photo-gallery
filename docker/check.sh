#!/bin/bash
# The Linux job of CI (.github/workflows/ci.yml), step by step, in the container's /work (the repository). Each step
# stops the run on failure, as in CI. Run it through the launcher, from the repository root: `just check-linux`
# (tools/linux.py check), which builds the image and mounts the working tree here.
set -euo pipefail

step() { printf '\n== %s\n' "$*"; }

step "Formatting"
cargo fmt --all --check

step "Clippy (the whole workspace)"
cargo clippy --workspace --all-targets --locked -- -D warnings

# A test that can't check what it is for returns early and passes, after it prints `skipped: <reason>`
# (pg_testkit::skip) or `<tool> not found: skipped` (missing_tool). --show-output keeps the printed lines of the
# passing tests too (libtest captures them otherwise), so the skipped lines reach the log that is checked below. A skip
# fails this step: the container runs as `pg`, so none of the permission-based tests should skip, and any skip is a
# test that checked nothing.
step "Tests (the whole workspace)"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
cargo test --workspace --locked -- --show-output 2>&1 | tee "$log"
skips=$(grep -E 'skipped: |: skipped$' "$log" || true)
if [ -n "$skips" ]; then
    printf '\nFAILED: tests skipped, so they passed without checking anything:\n%s\n' "$skips" >&2
    exit 1
fi

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
