# Plan: finish the allowlist work

Status as of 2026-11-23. This document covers the work that remains. Finished work is in the git history.

## Scope

One piece remains: the per-site lint expects in the bench, codec and desktop crates. No fork retirement is pending.

## Ground rules

- Make edits with the Edit/Write tools, not scripts.
- Commit only when asked. Keep each change to one reviewable commit.
- Batch `cargo check` and `cargo test` until a change's edits are all in; do not validate after each edit.
- Compile and check natively on Windows, with `PG_QT_DIR` set (`tools/check_qt.py:18` reads it, and the `_qt`
  prerequisite of `just lint`, `check`, `test` and `modgraph` fails without it). Linux checks go through
  `just check-linux` (Docker), not WSL. Never run the static Qt or static FFmpeg builds to verify a change.
- `missing_docs = "warn"` is on and `-D warnings` makes it an error: every new `pub` item needs a doc comment, and for a
  `pub struct` or `enum` each `pub` field and each variant too.
- Before a change: run `git status` and `python tools/boundaries.py`. Each change ends with `python tools/boundaries.py`
  and `just test-tools` (all `tools/test_*.py` files via `unittest discover`), plus its own checks. Run `just deny` after
  any change to `deny.toml` or the dependency graph.
- Line numbers in this document are a snapshot. Find a site by its quoted text, and re-read the file first.

## 1. Lint expects

Each expect below is an `#[expect]` for a clippy lint the code should not need. Fix the code, or keep the expect with a
reason that is still true.

- **Bench panics:** six `clippy::panic` expectations remain: five in `crates/bench/benches/ir/catalog.rs` (lines 125, 161,
  179, 240, 279) and one in `crates/bench/benches/ir/linux.rs:30`. Return `Result` from the bench helpers instead.
- **String and index expects:** `crates/desktop/src/qml_text.rs:94` and `crates/metadata/src/lib.rs:78` have
  `string_slice` expectations (fix with `split_once`, `strip_prefix` or `str::get`). `crates/metadata/src/container.rs:21`
  (`nul_terminated`, a `const fn`) has an `indexing_slicing` expectation.
- **Type name for `Op::frame`:** `crates/codec/src/rewrite/op.rs:61` has `type_complexity` on `Op::frame`. Name the type.
- **Desktop state modules:** twenty `unreachable_pub` expectations remain, in `crates/desktop/src/*/state.rs`. The plan asked
  to try one `bridges` module for all of the desktop's, which means moving the state modules too. This needs the user's
  decision.
- **Desktop `print_stderr`:** `crates/desktop/src/trace.rs:24` has a `print_stderr` expectation that is not in the earlier
  list (the `PG_TRACE=1` console output). Keep it with its reason, or confirm the reason.

Rejected for the desktop state modules: `#[doc(hidden)] pub` and `pub(crate)` for the bridge state structs. The first
creates API; the second is E0446.

Done when: `grep -rn "clippy::panic\|string_slice\|indexing_slicing\|type_complexity\|unreachable_pub" crates` finds only
the expects that are kept, each with its reason.
