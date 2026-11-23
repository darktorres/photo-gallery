# Plan: retire or tighten the non-empty allowlists

> Line numbers were refreshed on 2026-11-22 against a working tree that is still changing (the uncommitted
> `missing_docs` pass). Where a cite is not refreshed it is a snapshot: find the site by its quoted text.

## Context

An audit of the repo's allowlists (every non-empty one except `tools/anyhow-allow.txt`, which Phase 7 of the consistency
program already retires) found that most of them paper over something that can be refactored: a test-only dependency that
drags in an unmaintained crate, a crate that links codecs it should not, an environment write in the wrong crate, forks
that could be upstreamed, dead or stale entries, and per-site lint exceptions that are really owed refactors. This plan
removes each entry that has a proper refactor, tightens the ones that stay, and records why the rest stay. Churn is not a
concern; correctness and the intended layering are.

In scope: `PUBLIC_API_EXEMPT`, `ENV_READERS`, `REEXPORT_BOUNDARIES` (`tools/boundaries.py`); `advisories.ignore`, `bans`
wrappers, `sources.allow-git`, `licenses.allow` (`deny.toml`); `SYSTEM_DLL` (`tools/build_static_ffmpeg.py`); bindgen
`TYPES`/`VARS` (`crates/video/build.rs`); the `testkit` re-exports; the tooling users of `rusqlite`; the per-site
`#[expect]` exceptions; and the tooling and CI exceptions (Python lint ignores and `noqa` comments, the test-input list,
one CI waiver). Out of scope because empty: `tools/modgraph-allow.txt`, `PLATFORM_GLUE`,
`check_feature_coverage.ALLOWED`.

End state: an empty `REEXPORT_BOUNDARIES`, no `PUBLIC_API_EXEMPT`, `ENV_READERS` reduced to the registry reader's two
functions, an exact-name `SYSTEM_DLL` policy that fails the build, a bindgen allowlist that checks itself, no advisory
ignores, no `rusqlite` or codec wrapper for a crate that touches another crate's database or codecs, an `allow-git` that
lists only forks carrying changes upstream has not taken, the owed lint refactors done (including the front ends' printing,
the desktop's `exit` and the `unsafe_code` sites that need not exist), single-use licenses as per-crate exceptions, every
Rust workspace under the same deny, clippy and fmt gates, and the tooling and CI exceptions tightened or justified.

## Ground rules

- Make edits with the Edit/Write tools, not scripts.
- Batch `cargo check`/`cargo test` until a phase's edits are all in; do not validate after each edit. (Phase B's one-off
  printer run is the exception because it produces data.)
- Compile and check natively on Windows (PowerShell, with `PG_QT_DIR` set: `tools/check_qt.py:18` reads it, and the `_qt`
  prerequisite of `just lint`, `check`, `test` and `modgraph` fails without it). Linux checks go through
  `just check-linux` (Docker), not WSL. Never run the static Qt or static FFmpeg builds to verify a change.
- No commits or PRs unless asked. Each phase is one reviewable commit; Phase K is the exception (one commit per lint group,
  validated after each group: a batch of edits, not each edit).
- `missing_docs = "warn"` is on and `-D warnings` makes it an error: every new `pub` item needs a doc comment, and for a
  `pub struct`/`enum` that means each `pub` field and each variant too. This covers the plan's new items (`Request`,
  `Pass` and their fields; `jpeg::size`; `pub unsafe fn prefer_pulseaudio_if_pipewire_fails`; `pg_codec::Error::Imperfect`
  and `NothingLeft`; the `pub mod` and fixtures D makes public; every new `pg-model` type, field, variant and method in
  Phase H, so `cache.rs` keeps its per-method doc comments). Items that move keep the docs they have.
- Line numbers are a snapshot of a working tree that keeps changing. Where a step cites a file the in-progress work is
  editing, find the place by the quoted text and re-read the file first.
- Before each phase: run `git status` and `python tools/boundaries.py`, and treat "boundaries passes" as "no failure beyond
  the baseline you just measured". Every phase ends with `python tools/boundaries.py` and `just test-tools` (all
  `tools/test_*.py` files via `unittest discover`: ten today, eleven once Phase E adds its test), in addition to its own
  "Done when" line. `just deny` is a full `cargo deny check`; run it after A, B, D, G and J (each edits `deny.toml` or the
  dependency graph).

## Status and open work

Phases A to F, H to J, and K's lint groups K.1, K.2, K.5, K.8 to K.11 are complete and committed (the phases are in
`284d0505`, `8e96dbd4`, `1849f297`, `5d0b6765`, `b3eb0c31`, `e1c2b046` and the commits after them). Their sections were
removed from this plan; the history has them. Phase G is partly done (G0, the turbojpeg measurement, and the allocation
measurement). What remains:

- **G1, rusqlite flags:** deferred until upstream ships the statement-cache fix. Then move the two flags to
  `.cargo/config.toml` and drop `photo-gallery-defaults`.
- **G2, the rest of the measurements (skipped at your request):** rusqlite's cache fix and flags, dhat-rs, parajpeg,
  cxx-qt; the GIF, TIFF, zune-jpeg and thumbhash paths need samples that exercise them; Linux `just perf`.
- **G3, fork retirements:** none yet. Each needs its measurement and your decision.
- **L.9, MD060:** the table-style rule stays disabled (`.markdownlint.jsonc`). Enabling it, and pinning a style, is
  undecided. The rest of the tooling and CI work is done.
- **K.3, benches:** six `clippy::panic` expectations remain: five in `crates/bench/benches/ir/catalog.rs` and one in
  `crates/bench/benches/ir/linux.rs`. Return `Result` from the bench helpers instead.
- **K.4, strings and indexing:** `crates/desktop/src/qml_text.rs:94` and `crates/metadata/src/lib.rs:78` still have
  `string_slice` expectations (fix with `split_once`, `strip_prefix` or `str::get`). `crates/metadata/src/container.rs:21`
  (`nul_terminated`, a `const fn`) still has an `indexing_slicing` expectation.
- **K.6, singles:** `crates/codec/src/rewrite/op.rs:61` still has `type_complexity` on `Op::frame`. Name the type.
- **K.7, `unreachable_pub` in the desktop:** twenty expectations remain in `crates/desktop/src/*/state.rs`. The plan asked
  to try one `bridges` module for all of the desktop's, which means moving the state modules too. Needs your decision.
- **Desktop `print_stderr`:** `crates/desktop/src/trace.rs:24` has an expectation the plan does not list (the `PG_TRACE=1`
  console output). Keep it with its reason, or confirm.
- **Fork port follow-ups:** `just check-linux` after the `fast_image_resize` pin move; NEON and WebAssembly backends of the
  port are not compiled anywhere.

---

## Allowlist content that stays, with the reason

- `tools/crate-layers.txt`: it is the rule (exact internal edges, reviewed in the same commit as the change).
- `deny.toml` `licenses.allow`: a policy list, pruned to what is used (Phase B step 6).
- `ENV_READERS`: the registry reader's two functions (`process`, `set_process`), nothing else.
- bindgen `TYPES`/`VARS`: the mechanism that bounds generated FFI; kept, with Phase F's check.
- Single-owner bans (`ureq`, `quick-xml`, `cxx-qt`, `cxx-qt-lib`, `gif`, `webp`, `tiff`, `zune-*`): real owners.
- `deny.toml` `allow-git`: a source policy naming the pinned forks that stay (Phase G).
- The tooling and CI exceptions that stay: the `ruff.toml` global suppressions `S603`, `S607` and `TRY003` (used in
  `tools/`), the per-file ignores (`eval/**` and the tools' imports inside functions), the four-name `TEST_INPUTS` list in
  `tools/test_ci_files.py` (the other test inputs are constants in `crates/testkit/src/inputs.rs`), and
  `modgraph/split_check.py` `SKIP`.
- `UNSAFE_CRATES` (`tools/test_ci_files.py`): the CU-005 rule, self-checking through its guard tests.
- Phase K keeps: the `unsafe_code` expects (66, 67 once Phase C adds `env.rs`'s; minus the K.10 sites that go; FFI and OS
  calls), `as_conversions` and the casts in `num.rs`, the owners of banned calls, the log fallback's printing,
  `platform/src/process.rs`'s `exit`, fault-injection panics, the generated-code expects if K.7's single module does not
  build, and the three `indexing_slicing` sites `pdq.rs:78`, `join.rs:1017` and `join.rs:1070`. Also the `unwrap_used`
  expect of `catalog/src/fixture/raw.rs` (outside test builds only, under the `fixture` feature: its 38 helpers are the
  statements the catalog's fixture makes, and a failing statement is the caller's failure, not a result to return).

## Decisions (settled: the most proper option, churn not a concern)

1. Phase E: the DLL check **fails** the step. Unknown DLLs are either added to `REDISTRIBUTED` with a reason or are a real
   problem; any legitimate OS DLL the first real build flags is added deliberately.
2. Phase G: forks stay forks (the repo's recorded decision); nothing is vendored in-repo. `parajpeg` keeps its pinned fork,
   the mutex fix included.
3. Phase G: each fork change is measured, then kept in its pinned fork with the gain written in `vendor/README.md`, or
   dropped. No upstream pull requests: the user declined them.
4. Phase H runs before Phase I; the app entry leaves `REEXPORT_BOUNDARIES` for good.

## Rejected approaches (and why), so they are not re-proposed

- Vendoring `parajpeg` in-repo to empty `allow-git`: contradicts the repo's recorded move from in-repo copies to forks; its
  upstream C++ is unchanged and our commits sit on top, which is what a fork is for.
- Mirror types in `pg-app` for the 14 types: they are plain data, so mirrors only duplicate them.
- A std-only leaf crate below `platform` for the test folders: `platform/src/testing.rs` reads `crate::env` and
  `windows_sys`, and platform's own tests need the helpers.
- A `just bench` recipe as the measurement instrument: it does not exist.
- A System32-existence check for the DLL policy: it answers for the build machine's Windows, not the supported targets.
- A new `pg_codec::rewrite::Error` enum: `pg_codec::Error` already has the variants and helpers; two variants are added.
- Moving `Refusal::message` or `WriteError`'s `Display` into `pg-model`: `pg-model` says "Nothing here is English".
- Keeping `Cap`'s constructors in `thumbcache` while the type moves: the whole inherent `impl` must move (orphan rule).
- Moving `CACHE_KIB`/`WRITE` into `pg-catalog` so the probes can move: they are pairing-pass tuning values, not catalog data.
- Probes taking the values as positional arguments or with defaults: positionals collide with variant names; mirrored
  defaults drift.
- `std::fs::copy` alone in the bench: the copied files are non-WAL and `pending_thumbs` needs WAL.
- A `Stages` trait for `jobs::run_timed`/`batched`: `read`/`init`/`process` are `Fn + Sync` shared across threads while
  `sink`/`flush` are `FnMut` (and `flush` is `Send`), so one implementor would need `&self` shared and `&mut self` at once,
  and every caller would become a hand-written struct.
- A single `Walker<'a, L>` for `scan`: a field `lease: &'a mut ScanLease<'a>` pins the lease's own lifetime to the outer
  borrow (`&mut T` is invariant in `T`; `ScanLease<'c>` itself is covariant in `'c`, as it holds `&'c mut Catalog` and
  `Catalog` has no lifetime), so the lease is borrowed for `'a` for good and the loop's later direct uses fail; it needs
  two lifetimes and rewrites the roughly 300-line loop body.
- A buffers struct for `decode_into`'s `out` and `spare`: built per call from two disjoint fields, it gains nothing.
- Registering test inputs as `VARS` rows: `VARS` is compiled into production; `env.rs:93` says tests' inputs are not there.
- Making `BASELINE_VERSION` private to leave `NOT_REGISTERED`: the scan regex ignores `pub`.
- Reading build-script variables through `pg_platform::env`: `env::process` reads an empty value as unset, while a build
  script reads cargo's own variables where presence is the signal (`desktop/build.rs:121` is a presence-only check on
  `CARGO_CFG_DEBUG_ASSERTIONS`).
- A Hamming-distance tolerance for the PDQ vectors if Linux disagrees: a platform-dependent hash is a correctness bug.
- Raising `multiple-versions` to `deny` in Phase B: the known duplicates unrelated to `pdqhash` would fail it until each has
  a `[[bans.skip]]` entry and a reason. That is the right end state (Phase B step 6 says so), but it is a separate
  decision after Phase G, not part of removing `pdqhash`.
- `#[doc(hidden)] pub` and `pub(crate)` for the desktop's bridge state structs (K.7): the first creates API, the second
  is E0446.
- `eval/jpeg-decoders` as the instrument for the MSVC `turbojpeg-sys` change: it is the Linux/container harness and its
  Windows script builds libjpeg-turbo with `/O2`.
- `tools/fork-updates.py` as a stale-pin detector: it only compares versions with crates.io.
- Treating S101/S311/PLC0415 as eval-only `ruff` ignores: they occur in `tools/`.
