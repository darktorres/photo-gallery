# Consistency-audit program: standing rules and gates

**Status (2026-11-22): Phases 0 to 10 are executed and committed**, through `ec85b503` on local `main` (not pushed), with the last plan-related commit at `d47681f6`. The open items are listed in `docs/plans/PROGRESS.md` and in `docs/plans/CONSISTENCY_AUDIT.md`. This file keeps the standing rules, gate definitions and checks. The phase-by-phase design text was removed as done; it is in git at `c1d477f3`, and the phase commits run from `e2c3b4a5` (Phase 1) to `ec85b503`.

## Working rules

The owner's saved preferences still apply: edit with the Edit and Write tools, never scripts; batch validation (finish a batch of edits, then validate once); compile natively on Windows (the Linux container pass, `just check-linux`, needs Docker Desktop); never run the hour-long static Qt build; no background tasks or monitors (run commands in the foreground; a command that exceeds the 10-minute cap is run by the owner, who pastes the result); commit only when the owner asks; never launch the app against the owner's real catalog (`.env`'s `PG_CATALOG_DIR`, `D:/PhotoGallery/catalog`, with `thumbs.sqlite` beside it and prefs in `D:/PhotoGallery/gallery.json`): `just shots` and `cargo test` isolate their state (`tools/compare_shots.py` sets `PG_CATALOG_DIR` and `PG_PREFS`; tests use `PG_TEST_DIR`), a manual run does not, so use a scratch copy. No branches: history goes straight to `main`. Commit messages follow the repo style: an imperative subject of at most about 72 characters, a body wrapped near 120 columns, and the `Co-Authored-By` trailer the session specifies.

## Gates

- **G-check** = `cargo check --workspace --all-targets --locked`, run through the repo's environment: `just check` (`Justfile`). It does not cover `fuzz/` or `eval/*-decoders` (separate workspaces); `tools/lint.py` runs `cargo metadata --locked` on them.
- **G-lint** = `just lint`: `tools/lint.py` (fmt, clippy `-D warnings` including the wasm and `dhat-heap` variants, `boundaries.py`, cargo-machete, ruff, markdownlint, and `cargo doc` with `-D warnings`), then the separate `lint-qml` recipe.
- **G-full** = `just build debug`, then G-lint, then `cargo test --workspace` and `just test-tools`. Add `just modgraph` when the change touches module boundaries. Rebuild after any QObject-bridge change.
- **G-qt** = `just build debug` and `just qmltest`.
- **Checkpoints**: `docs/plans/PROGRESS.md` records open items only; it is updated at every commit point. Ratchets that shrink monotonically: `tools/anyhow-allow.txt` and the list of crates carrying `#![warn(missing_docs)]`, which is now empty because `missing_docs` is warn workspace-wide.

## Rules new code must obey

- Rust: `unwrap_used`, `expect_used`, `indexing_slicing`, `panic`, `print_*`, `todo`, `as_conversions` and every `cast_*` are denied outside tests (tests are exempt via `clippy.toml`; non-test crates such as `testkit` are not). Use `.lock().unwrap_or_else(PoisonError::into_inner)` (the repo idiom), `get`/`last_mut`/`pop`, `u8::from`/`try_from`, and `char` or slice helpers instead of `as`, `[i]` and `&s[..]`. `allow_attributes`, `string_slice` and `unreachable_pub` are fatal under `-D warnings`: every suppression is `#[expect(.., reason = "..")]`, and an `expect` that does not fire is itself a warning. `clippy.toml` bans `HashMap`, `HashSet`, `RandomState` and `std::env::temp_dir`.
- Python (`ruff.toml`: line length 130, py312; ERA, SLF, S, PTH, RET, ARG, TRY, N enabled): standard library only, flat imports in the style of `tools/test_ci_files.py`, `pathlib` only, and no imports of underscore names. Rust snippets in tests live inside string literals.
- Docs: `crates/README.md` rule 1 says "symbols, not line numbers": anything written into `crates/README.md` names symbols, never lines. Comments say why, not what. Justfile recipes carry a `#` doc comment above them. `.env.example` entries are a commented `# PG_X=value` with a one-line description above. Every new tool or data file gets a row in `tools/README.md`.
- `just fmt` rewrites files repo-wide: run it deliberately on touched files, never as a gate.
- Audit dispositions are rewritten in place as `**Disposition: Fixed.** <what changed, where>` (or `Deferred.` with the reason, `Fixed in part.`, `No change.`, `Moot.`, `Decided and applied.`). Never append beside the old line.

## Standing checks

Each check has a Python test so it cannot rot, and runs in `just lint` or `just test-tools` (CI runs both):

- anyhow in the manifests (`cargo metadata`, `test_ci_files.py` `AnyhowAllowlist`);
- `std::env` outside the env module (`boundaries.py`, regex `\benv::(var|var_os|vars|vars_os|set_var|remove_var)`);
- one public path per item (`boundaries.py`, with `test_boundaries.py`);
- `forbid(unsafe_code)` presence (`test_ci_files.py` `UnsafePolicy`);
- the `PG_*` registry against the docs (`test_ci_files.py`).

The environment-reader check (`check_env` in `tools/boundaries.py`) runs inside `just lint` and has no unit test of its own.

## Open items

**Updated 2026-11-22.** Still external: `just check-linux` must run again after `ec85b503`; the static Qt build and link; V-030 (the nvJPEG wheel's header diff); E-017 and P-004 (a GPU run and the win_build chain); the C++ env parser, which is only spelling-tested; QML shots after Phase 10 (the reference set is `tools/shots/reference/f4bbb48419`). `just deny` passed at `ec85b503`; it still needs the advisory database at run time. The full list with owners is in `docs/plans/PROGRESS.md`.
