# Lint and analysis tools: status and decisions

A ledger of the Rust checks the project runs, the ones it could run, and the ones it has not yet adopted. Recorded
2026-11-05 on the Windows toolchain, and checked against the repository 2026-11-14. The project pins
`nightly-2026-11-06` in `rust-toolchain.toml`, which is the active toolchain (the stable 1.99.1 of the first record is
not the one `cargo` uses here).

## Already in `just lint`

`just lint` runs `tools/lint.py`, after the tool-version checks, and then `lint-qml`. These are its steps.

| Tool | What it checks | Notes |
|---|---|---|
| `cargo fmt --all --check` | Formatting | Workspace only. Vendored crates keep their own formatting. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Lints | Workspace lints are set in `Cargo.toml` (`[workspace.lints.clippy]`, which sets the phase lints of `docs/plans/CLIPPY_LEDGER.md`, starting with `undocumented_unsafe_blocks`). Every crate inherits them with `[lints] workspace = true`. |
| `cargo check -p pg-model -p pg-client --locked --target wasm32-unknown-unknown` | Build for wasm | Keeps the pure crates free of platform calls. |
| `cargo metadata --locked` on `fuzz/Cargo.toml` and `eval/jpeg-decoders/Cargo.toml` | Lockfiles of the two separate workspaces | Fails when a lockfile is stale. |
| `cargo doc --workspace --no-deps --document-private-items --locked` with `RUSTDOCFLAGS=-D warnings` | Doc comments | Broken intra-doc links and other rustdoc warnings fail the build. `missing_docs` is not enabled, so undocumented items pass. |
| `cargo deny check` | Licences, advisories, banned crates, sources | Configured in `deny.toml`. Covers advisories, so `cargo-audit` is not needed. |
| `cargo machete crates` | Unused dependencies | Heuristic. `cargo-udeps` is more precise but needs nightly. |
| `ruff format --check .`, `ruff check .` | Python formatting and lints | ruff is pinned in `tools/lint` (its lockfile). |
| `markdownlint "**/*.md"` | Markdown | markdownlint is pinned in `tools/lint` (its lockfile). |
| `tools/boundaries.py` | Crate layers, acyclic dev edges, re-exports, platform code | Runs after the steps above. |

## Installed, not in the gate

| Tool | What it checks | Status | Notes |
|---|---|---|---|
| `cargo miri` | Undefined behaviour at run time | Not installed. The `miri` component is absent on every toolchain here, and the `cargo-miri.exe` in `~/.cargo/bin` does not run. | Targeted at the `unsafe` and FFI code. Slow, so run on demand, not per commit. Install with `rustup component add miri --toolchain nightly-2026-11-06`. |
| `cargo outdated` | Dependencies with newer versions | Installed (0.19.0) | Informational. |
| `cargo msrv` | Minimum supported Rust version | Installed (0.20.0-beta.4) | `rust-version` is set once, in the root `Cargo.toml` `[workspace.package]` (1.89: let-chains need 1.88, `File::try_lock` needs 1.89), and the members inherit it. Not yet checked with `cargo msrv verify`. |
| `cargo modules` | The module graph | Installed (0.28.0, the version pinned in `Justfile`) | Used by `just modgraph`, which CI runs and `just lint` does not. |
| `cargo report future-incompat` | Upcoming breakage from the compiler | Built in | Run after toolchain upgrades. |

## Not installed

| Tool | What it checks | Priority | Notes |
|---|---|---|---|
| `typos` | Spelling in code, docs and comments | High before release | Cheap. Would catch misspellings across all tracked files. |
| `cargo-hack` | Every feature combination builds | High | Clippy already compiles the `fixture`, `fault`, `testing` and `bench` features, because workspace crates enable them, but only in those combinations. No workspace crate enables `dhat-heap` (gallery) or `specta` (model), so clippy never compiles them. |
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
- `[workspace.lints.clippy]` sets most lints to `warn` or `deny`. `-D warnings` is applied in `just lint`
  (`tools/lint.py`), and also in `eval/jpeg-decoders/linux.py` and `tools/profiling/bench.py`.

## Open decisions

1. Add `typos` and `cargo-hack` to `just lint`, or keep them as on-demand checks.
2. Whether to enable `pedantic` or `nursery` clippy groups, and which lints to allow.
3. Whether to run `cargo miri` on the `unsafe` crates, and how often.
