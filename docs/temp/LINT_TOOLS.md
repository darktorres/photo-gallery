# Lint and analysis tools: status and decisions

A ledger of the Rust checks the project runs, the ones it could run, and the ones it has not yet adopted. Recorded
2026-11-05 on the Windows toolchain (stable 1.99.1, with nightly toolchains installed).

## Already in `just lint`

| Tool | What it checks | Notes |
|---|---|---|
| `cargo fmt --check` | Formatting | Workspace only. Vendored crates keep their own formatting. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lints | Workspace lints are set in `Cargo.toml` (`[workspace.lints.clippy]`, which sets the phase lints of `docs/CLIPPY_LEDGER.md`, starting with `undocumented_unsafe_blocks`). |
| `cargo check -p pg-model -p pg-client --target wasm32-unknown-unknown` | Build for wasm | Keeps the pure crates free of platform calls. |
| `cargo doc --workspace --no-deps --document-private-items` with `RUSTDOCFLAGS=-D warnings` | Doc comments | Broken intra-doc links and missing docs fail the build. |
| `cargo deny check` | Licences, advisories, banned crates, sources | Configured in `deny.toml`. Covers advisories, so `cargo-audit` is not needed. |
| `cargo machete crates` | Unused dependencies | Heuristic. `cargo-udeps` is more precise but needs nightly. |

## Installed, not in the gate

| Tool | What it checks | Status | Notes |
|---|---|---|---|
| `cargo miri` | Undefined behaviour at run time | Installed (nightly toolchains present) | Targeted at the `unsafe` and FFI code. Slow, so run on demand, not per commit. |
| `cargo outdated` | Dependencies with newer versions | Installed | Informational. |
| `cargo msrv` | Minimum supported Rust version | Installed | Not set anywhere in the project yet. |
| `cargo report future-incompat` | Upcoming breakage from the compiler | Built in | Run after toolchain upgrades. |

## Not installed

| Tool | What it checks | Priority | Notes |
|---|---|---|---|
| `typos` | Spelling in code, docs and comments | High before release | Cheap. Would catch misspellings across all tracked files. |
| `cargo-hack` | Every feature combination builds | High | Covers the gated `fixture` and `dhat-heap` features, which clippy sees only in the default configuration. |
| `cargo-udeps` | Unused dependencies, precise | Low | Needs nightly. Machete already runs. |
| `cargo-geiger` | Count of `unsafe` in the dependency tree | Low | Informational. |
| `cargo-audit` | Advisory database | Not needed | `cargo deny` already checks advisories. |
| `taplo` or `cargo-sort` | TOML formatting and key order | Low | Manifests are hand-ordered now. |
| `cargo-mutants` | Whether the tests catch real changes | Low | Slow. Useful on the catalog and model crates. |
| `cargo-llvm-cov` | Test coverage | Low | Informational. |
| `cargo-semver-checks` | Breaking API changes | Not needed | Every crate is `publish = false`. |

## Lint configuration not yet decided

- Clippy's `pedantic` and `nursery` groups are off. Turning them on is a configuration choice, and would mean
  deciding which lints to allow.
- `[workspace.lints.clippy]` sets most lints to `warn` or `deny`, and `-D warnings` is applied only in `just lint`.

## Open decisions

1. Add `typos` and `cargo-hack` to `just lint`, or keep them as on-demand checks.
2. Whether to enable `pedantic` or `nursery` clippy groups, and which lints to allow.
3. Whether to run `cargo miri` on the `unsafe` crates, and how often.
