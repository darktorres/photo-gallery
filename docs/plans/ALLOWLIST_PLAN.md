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
`#[expect]` exceptions; and the tooling and CI exceptions of Phase L (Python lint ignores and `noqa` comments, the
test-input list, one CI waiver). Out of scope because empty: `tools/modgraph-allow.txt`, `PLATFORM_GLUE`,
`check_feature_coverage.ALLOWED`.

End state: an empty `REEXPORT_BOUNDARIES`, no `PUBLIC_API_EXEMPT`, `ENV_READERS` reduced to the registry reader's two
functions, an exact-name `SYSTEM_DLL` policy that fails the build, a bindgen allowlist that checks itself, no advisory
ignores, no `rusqlite` or codec wrapper for a crate that touches another crate's database or codecs, an `allow-git` that
lists only forks carrying changes upstream has not taken, the owed lint refactors done (including the front ends' printing,
the desktop's `exit` and the `unsafe_code` sites that need not exist), single-use licenses as per-crate exceptions, every
Rust workspace under the same deny, clippy and fmt gates, and the tooling and CI exceptions of Phase L tightened or
justified.

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

## Order and dependencies

Phases 7 and 8 of the consistency program are committed (`389d81f8`, `bf0e2ba7`). The working tree now holds a different
uncommitted pass: `missing_docs = "warn"` (root `Cargo.toml:74`, marked AD-004) and the doc comments it needs, across
about 136 files, including `crates/{client,container,dbfile,dedup,log,perf,query,yandex,jobs-testkit,metadata}` and, in
the crates this plan edits, `analysis`, `app`, `catalog`, `codec`, `edit`, `jobs`, `library`, `desktop/build.rs`,
`gallery/build.rs` and others (`git status` for the live list), plus `docs/plans/{CLIPPY_LEDGER,CONSISTENCY_AUDIT}.md`.
`Cargo.lock`, `fuzz/Cargo.lock`, `tools/test_ci_files.py` and `crates/README.md` are clean. In the table below, "Phase 7"
in a "Waits for" cell means "the in-progress working tree is committed (or the phase runs on a branch from a commit that
has it)": every phase that edits a file that pass touches (check `git status` for that file) waits for it. Phase F's
`crates/video/build.rs` is clean today (only `video/src/{error,lib}.rs` are dirty), so F could run earlier, but it shares
`crates/video` with that pass. `missing_docs` changes what the new code must look like: see the Ground rules.

| Phase | What | Waits for | Notes |
|---|---|---|---|
| A | dead and stale entries | nothing | edits `tools/`, `deny.toml`, one README sentence |
| E | `SYSTEM_DLL` exact-name policy | nothing | only `tools/` and `docs/reference/STATIC_BUILD.md` |
| L.4, L.5 | noqa audit, CI waiver | nothing | tools, config, CI only |
| B | remove `pdqhash` | Phase 7 | rewrites `Cargo.lock` |
| C | environment reads | Phase 7 | with L.1 (same test class); edits `test_ci_files.py`, `env.rs` |
| D | `edit`'s rewrite tree and testkit image fixtures into `pg-codec` | Phase 7, A, B | before I; rewrites `Cargo.lock`, `fuzz/Cargo.lock`; A and B edit the same `deny.toml` `image` wrapper line |
| F | bindgen self-check | Phase 7 | `crates/video` is dirty |
| G | forks | Phase 7 | `Cargo.toml`, locks |
| H | `pg-app`'s 14 types into `pg-model` | Phase 7 | before I |
| I | remove the `testkit` re-export boundary | D and H | both D and I edit `testkit/Cargo.toml`, `testkit/src/lib.rs` and the `testkit:` layers line; I deletes `REEXPORT_BOUNDARIES` only once H has emptied it |
| J | `rusqlite` tooling users | Phase 7 | independent of H, I |
| K | per-site lint exceptions | Phase 7 | independent of H, I; the `type_complexity` item in K.6 waits for D |
| L.1, L.7 | `TEST_INPUTS`; the `perf_check` switch and the `env.rs:203` doc | with C | same test class and the same `env.rs` that C edits |
| K.8-K.10 | front-end printing, desktop `exit`, `unsafe_code` sites that need not exist | the `missing_docs` pass | `cli`, `gallery`, `desktop`, `container`, `dedup` are dirty; K.10's `boundaries.py` rule goes with C's `boundaries.py` edits |
| K.11 | `pg-model` and `pg-client` `no_std`, `text/` split into its own crate, the `clippy.toml` copies | H | `model` and `client` are dirty; do after H so H's new modules are written `no_std`-clean; touches ~28 importer files in 9 crates and `tools/crate-layers.txt` |
| L.8-L.12 | `ANYHOW_PERMANENT`, markdownlint, three ungated workspaces, `PSEUDO`, `TEST_INPUTS` as constants | nothing, except L.10 with G and L.12 with C | tools, config and CI; L.10 regenerates nothing but adds gates for the locks G regenerates |
| L.2, L.3 | `NOT_REGISTERED`, `ruff` ignores | the `missing_docs` pass, for L.2 only | L.2 renames constants in `catalog`/`thumbcache` (dirty); L.3 edits only `ruff.toml` and the Python sites it names (`tools/test_ci_files.py` is clean) |

---

## Phase A: dead and stale entries (no behaviour change)

**Depends on:** nothing. **Touches:** `tools/boundaries.py`, `tools/test_boundaries.py`, `deny.toml`, one sentence of
`crates/README.md`.

1. **Delete `PUBLIC_API_EXEMPT`.** No crate root re-exports `fault` (`pub mod fault` exists in `borders`, `thumbs`,
   `writeback`; no root has `pub use fault::...`), and with the constant set to `set()` the check already returns no
   failures on the current tree, so the deletion changes nothing there.
   - `tools/boundaries.py`: remove the constant and its comment (lines 68-69), the sentence in the rule-3 docstring (lines
     12-13), the `name not in PUBLIC_API_EXEMPT` filter in `check_public_paths` (line 242) and its docstring mention (line
     235).
   - `tools/test_boundaries.py`: fix the module docstring (lines 1-2); delete `test_an_exempt_pub_mod_is_public_on_its_own`
     (it passes without the exemption, so it tests nothing); add `test_a_root_reexport_from_pub_mod_fault_is_a_second_path`
     asserting that `"pub mod fault;\npub use fault::X;\n"` gives one failure (the existing tests use the name `a`; do not
     add a copy of `test_a_root_reexport_from_a_pub_mod_is_a_second_path` under another name).
   - `crates/README.md:137`: drop the `PUBLIC_API_EXEMPT` sentence (find it by text). The mentions in
     `docs/plans/CONSISTENCY_PROGRAM_PLAN.md:338,345` are historical; leave them.
2. **Remove the stale `pg-library` wrapper from the `image` ban** (`deny.toml:55`). `cargo tree -i image@0.25.10` shows
   only `pg-codec` and `pg-testkit` depend on it directly. Fix the comment on `deny.toml:53-54`, which still says
   `pg-library`'s tests write image fixtures through a dev-dependency.

**Done when:** `python tools/boundaries.py` shows no failure beyond the baseline; the guard test **fails before** the
deletion (the exemption makes that input produce no failure) and passes after, so it is added in the same commit; `just deny`.

## Phase B: remove `pdqhash` (clears the advisory exception, `image 0.23` and two wrapper entries)

**Depends on:** Phase 7. **Touches:** `crates/analysis` (`src/pdq.rs`, `Cargo.toml`), `Cargo.toml`, `Cargo.lock`,
`deny.toml`.

`pdqhash` is a dev-dependency of `pg-analysis` used only by `reference()` in the test module of
`crates/analysis/src/pdq.rs` (line 312), called by `matches_reference_crate` (line 365). The only path to
RUSTSEC-2025-0056 is `pdqhash` → `image 0.23.14` → `tiff 0.6.1` → `miniz_oxide 0.4.4` → `adler 1.0.2`.

1. **Record golden vectors while the crate is still present, only where our hasher already equals the reference.** Our
   runtime DCT matrix (`pdq.rs:126-137`, built with `f64::cos`) differs from pdqhash's `DCT_MATRIX` bit patterns in 808 of
   1024 entries (recomputed independently), so the port matches the reference only on the hash bits, and only on smooth
   images: noise, 1x1 and flat images make coefficients near-tied or near zero, where a last-bit difference flips a bit,
   so recording the reference's output for them would give vectors our hasher fails.
   - Add a temporary `#[ignore]` test that prints both `Hasher::hash_rgb` and `reference()` for a candidate set, run it
     once (`cargo test -p pg-analysis -- --ignored --nocapture`; the one exception to batching cargo runs), keep only the
     candidates where they are equal (the four existing sizes 512x384, 300x512, 64x64, 97x211, plus other smooth ones that
     agree), paste the reference's values into the real test with Edit, and delete the printer.
   - The images must be platform-independent. `test_image` uses `f32::sin`/`cos`, whose last bit may differ between
     Windows and Linux libm, so write a dedicated integer-only smooth generator (a low-order polynomial or a 2-D triangle
     wave with a long period). `pg_testkit::textured_rgb` is pseudo-noise mod 256 and `gradient_rgb` is a 64-pixel
     sawtooth that `pdq.rs:293` says aliases on the 64x64 grid, so neither is smooth. Re-check that ours still equals the
     reference on them before recording.
   - Noise and degenerate-size coverage already exists in the self-consistency tests (`columns_at_once_change_no_bit`, the
     dihedral tests) and stays there.
2. Rename the test to `matches_reference_vectors`; it compares `Hasher::hash_rgb` to the recorded 32-byte hashes. Update the
   `pdq.rs:3` comment, which names the crate it follows, and reword any claim in the module doc that the port is
   "bit-exact" and "tested against" the crate: it agrees on the hash bits for the recorded smooth images, not on the matrix
   or on every input. Historical documents that say bit-exact (`docs/plans/PLAN.md:284`,
   `docs/plans/NIGHTLY_AND_BUILD_FLAGS_PLAN.md:144,194`, `docs/perf/callgrind/1bb039084.md:191`) are not rewritten.
3. Remove `pdqhash` from `crates/analysis/Cargo.toml` (dev-dependencies) and the workspace table (`Cargo.toml:41`); let
   cargo rewrite `Cargo.lock`.
4. `deny.toml`: delete the `[advisories] ignore` entry (lines 9-12), the `pdqhash` ban (lines 66-67) and `pdqhash` from the
   `image` wrappers. Rewrite the `multiple-versions` comment (line 42): duplicates unrelated to `pdqhash` remain (bitflags,
   codespan-reporting, cxx-qt-gen, libloading 0.8/0.9, miniz_oxide 0.8/0.9, syn 2/3, thiserror 1/2, unicode-width,
   quick-error (via `rusty-fork`), and `weezl` until Phase D lands (`gif 0.14` and `tiff 0.11` use 0.2, `pg-edit` is
   the other user; D.6 removes edit's, after which `weezl` is not duplicated)). `bitflags` is not on that list: its 1.3
   version's only user is `png 0.16`, which leaves with `image 0.23`, so only 2.x remains. Leaving with `pdqhash`:
   `image 0.23`, `png 0.16`, `gif 0.11`, `tiff 0.6`, `miniz_oxide 0.3/0.4`, `adler`, `bitflags 1.3`, and their private
   dependencies (`deflate`, `jpeg-decoder`, `num-iter`, `num-rational`, `num-integer`, `byteorder`, `adler32`,
   `scoped_threadpool`, `rayon`, `rayon-core`, `crossbeam-deque`, `crossbeam-epoch`, `either`: verify each with `cargo
   tree -i` before listing it). Say the remaining duplicates come from build tools and mixed-version dependencies, and
   keep `multiple-versions` at `warn`.
5. **Cross-platform rule.** The vectors stay bit-exact, with no tolerance. If a recorded hash differs on Linux
   (`just check-linux`), that is a real defect: the hashes are stored in the catalog and compared across runs, so a hash
   that depends on the platform's libm is a correctness bug. The source is known: the `f64::cos` in `dct_matrix` is the
   hasher's only libm call. In that case make it a constant table **generated from our own current runtime matrix**, not
   copied from pdqhash (using pdqhash's table would change the hashes already stored in catalogs). Do not loosen the
   assertion. This branch is unlikely to fire: the f32 sums run in a fixed order, `pdq.rs` has no `mul_add`, SIMD or
   `cfg(target_*)`, and a one-ulp f64 libm difference changes an f32-rounded entry about 2^-29 of the time (roughly 2e-6 per
   platform pair over 1024 entries). The vectors still matter because nothing else pins PDQ bits (`hash_rgb` appears only
   in `pdq.rs`).
6. After the removal run `cargo deny check licenses` and prune `licenses.allow` of every entry it reports as not
   encountered (keep the reason comments for the rest). Then tighten what remains:
   - A single-use anomalous license belongs in `[[licenses.exceptions]] crate = "..."`, not in the global list:
     `CDLA-Permissive-2.0` (a data license, used only by `webpki-root-certs`), and `BSL-1.0` and `Unlicense` if each has
     one user. A new crate under that license then needs a decision. The mainstream licenses (MIT, Apache-2.0,
     BSD-3-Clause, ISC, Zlib, Unicode-3.0) stay global. (This reverses the earlier note that `licenses.exceptions` is
     "not an allowlist here": once used, it is one more list, justified per crate.)
   - `[graph] all-features = false` leaves optional dependencies outside the advisory, license and ban graph:
     `specta` (`pg-model`), `dhat` and `pg-desktop` (`gallery`), although `lint.py` clippies the `specta` and `dhat-heap`
     features. Set `all-features = true` (or list those features) and fix what it surfaces; whether it adds new
     duplicates or licenses is UNVERIFIED until run.
   - `multiple-versions = "warn"` stays only as a decision, not as a default: the proper end state is `deny` with a
     `[[bans.skip]]` entry per known duplicate (`syn@2`, `thiserror@1`, `libloading@0.8`, `miniz_oxide@0.8`, and so on),
     each with a reason, so a new duplicate fails instead of scrolling past. The trade is one reviewed skip list
     (itself an allowlist) for a build that fails on new duplicates. Decide it here; if it is adopted, do it after Phase
     G, because retiring forks changes which duplicates exist.

**Done when:** before/after `cargo deny check`; `cargo tree -i adler` finds nothing and `Cargo.lock` has no `image 0.23`,
`tiff 0.6`, `png 0.16`, `gif 0.11`, `bitflags 1.3` or `miniz_oxide 0.3`/`0.4`; `cargo test -p pg-analysis` on Windows and `just check-linux` for the cross-platform
vectors; `just deny`.

## Phase C: environment reads (`ENV_READERS` from 5 entries to 1)

**Depends on:** Phase 7. **Touches:** `crates/platform/src/{env.rs,audio.rs}`, `crates/desktop/src/lib.rs`,
`tools/{boundaries.py,pgtools.py,test_boundaries.py,test_ci_files.py,test_pgtools.py,README.md}` (`tools/README.md:19`
describes `test_boundaries.py` as only "the one-public-path check": reword it), `crates/README.md`. Phase L.1 is done
together with this phase.

1. **Move the `QT_AUDIO_BACKEND` write into `pg-platform`.**
   - `env.rs`: add `pub(crate) unsafe fn set_process(name: &str, value: &str)` wrapping `std::env::set_var` with the SAFETY
     contract "no other thread is running". Under edition 2024 the call inside needs its own `unsafe {}` block with a
     `// SAFETY:` comment (`unsafe_op_in_unsafe_fn`, `undocumented_unsafe_blocks`). Keep the spelling `env::set_var` out of
     its doc comment: `ENV_READ` runs on raw text, comments included, and only the function bodies are blanked.
   - `unsafe_code`: the workspace sets `unsafe_code = "deny"` (`Cargo.toml` `[workspace.lints.rust]`) and every other unsafe
     module in `crates/platform/src` carries `#![expect(unsafe_code, reason = "...")]` (`dll.rs:5`, `dir.rs:6`, `file.rs:3`).
     Add one unconditional `#![expect(unsafe_code, reason = "...")]` to `env.rs` (after its `//!` block; `unsafe fn
     set_process` fulfils it on every target). `audio.rs` already has
     `#![cfg_attr(target_os = "linux", expect(unsafe_code, reason = "FFI: the PipeWire calls ..."))]` (lines 6-9):
     replace that with an unconditional `#![expect(unsafe_code, ...)]` rather than adding a second one. The new
     non-`cfg` `pub unsafe fn` and its `unsafe {}` block fulfil it on Windows, and the Linux-only PipeWire blocks on Linux
     (`unsafe_code` fires on an `unsafe fn` declaration and on an `unsafe {}` block whatever the platform).
   - `audio.rs`: add `pub unsafe fn prefer_pulseaudio_if_pipewire_fails()` with a `# Safety` doc section
     (`clippy::missing_safety_doc` runs under `-D warnings`; `crates/platform/src/dll.rs:40` is the template), holding the
     logic now at `crates/desktop/src/lib.rs:147-149` (`process("QT_AUDIO_BACKEND").is_none() &&
     pipewire_context_fails()`). It needs its own `unsafe { crate::env::set_process(..) }` block with a `// SAFETY:` comment
     and calls `set_process` unconditionally (its guard is the early `if`, not a `cfg`): gating the call would leave
     `set_process` dead code on Windows and `-D warnings` fails there. It is a no-op off Linux because
     `pipewire_context_fails()` is false there, so the Windows CRT caveat in `gpu_device.cpp` does not apply. Update the
     module doc (line 4), which points at the desktop file.
   - `desktop/src/lib.rs`: replace the block with one call, keeping the "first thing `run` does, before any thread" SAFETY
     comment.
   - `tools/boundaries.py`: delete the `crates/desktop/src/lib.rs` entry and reword the module docstring lines 18-21 ("the
     one documented `set_var`"); update `crates/README.md` (the `ENV_READERS` sentence, near `:171`).
2. **Replace the three build-script entries with a rule.** Build scripts read the environment by cargo's protocol; the real
   constraint is that every `PG_*` name they touch is registered with `Reader::Build`.
   - `boundaries.py` `check_env`: exempt a build script by `path.name == "build.rs" and path.parent.parent == REPO /
     "crates"`, not by name alone, so a `src/build.rs` module does not escape. Exactly three exist (`desktop`, `gallery`,
     `video`).
   - `tools/test_ci_files.py` `EnvironmentRegistry` already scans every `.rs` under `crates/` (`env_sources`, lines 305-313)
     and parses `Reader` with `REGISTERED_SPEC` (`:296`). Add one test there: every `PG_*` name found in the code of a
     `build.rs` (strip `//` comments first: `desktop/build.rs:6,72` and `video/build.rs:64` name variables in comments) is
     registered with `Reader::Build`. `PG_DESKTOP_CONTENT_KEY`, `PG_VIDEO_CONTENT_KEY` and `PG_FFMPEG_BIN` all are today
     (`PG_FFMPEG_BIN` is only a `cargo:rustc-env` write at `video/build.rs:86`). `boundaries.py` does not import
     `test_ci_files.py` (it pulls `ci.*` and `install_exiftool`), so there is no second parser.
   - `docs/plans/CONSISTENCY_PROGRAM_PLAN.md:515` (A-031) plans `PG_FFMPEG_STATIC` and `PG_QT_STATIC` in a build script;
     they are unregistered today, so register them as `Reader::Build` when that lands, or this test fails.
3. **Narrow `env.rs`'s exemption** from the whole file to the bodies of `process` and `set_process`, so a stray read
   elsewhere in `env.rs` still fails.
   - Move `function_body` (`test_ci_files.py:317`) to `tools/pgtools.py` and import it from both tools; add a
     `test_pgtools.py` case for it. `boundaries.py` imports only the standard library today, so `from pgtools import
     function_body` is its first sibling import; it resolves because `lint.py` runs `python tools/boundaries.py` without
     `-I` (the script directory is on `sys.path`), so do not add `-I`.
   - Make `ENV_READERS` a `dict[str, tuple[str, tuple[str, ...]]]` mapping the path to (its reason, the function headers
     `"pub fn process("` and `"pub(crate) unsafe fn set_process("`), keeping the existing reasons (reword the docstring at
     `boundaries.py:21` and the README sentence to match).
   - In `check_env`, for a key in it, blank each body with `text = text.replace(body, re.sub(r"[^\n]", " ", body), 1)` (this
     keeps line numbers, as `production()` does for `cfg(test)` items) and then run `ENV_READ` over the blanked text.
     `function_body` raises a bare `ValueError` if a header drifts (a visibility change, say): catch it and report a failure.
     It ends at the first `"\n}"`, which both bodies satisfy. The only `env::` match in `env.rs` today is line 215, inside
     `process`.
4. `tools/test_boundaries.py`: add tests (a `build.rs` may call `env::var`; a non-build file may not; `env.rs` may read only
   inside `process`/`set_process`).

**Done when:** `cargo test -p pg-platform`; `cargo check -p pg-desktop` (needs `PG_QT_DIR`); `just check-linux` for the
Linux-only path; the ignored audio test (`cargo test -p pg-platform audio -- --ignored`, needs Linux and an empty
`PIPEWIRE_CONFIG_DIR`) run by hand in a Linux container, since `just check-linux` takes no test arguments and fails on any
"skipped" line; the new boundary tests fail if `std::env::var` is added to a non-build file or an unregistered name to a
`build.rs`.

## Phase D: move `edit`'s codec-level rewriting into `pg-codec`

**Depends on:** Phase 7; before Phase I. **Touches:** `crates/{edit,codec,testkit}`, every crate using testkit's image
fixtures, `deny.toml`, `tools/crate-layers.txt`, `crates/README.md`, `Cargo.lock`, `fuzz/Cargo.lock`.

`deny.toml:53` says the image decoders and encoders are `pg-codec`'s, but `pg-edit` links `turbojpeg`, `png` and `image-webp`
directly (production code in `crates/edit/src/rewrite/{jpeg,png,webp}.rs` and `crop.rs:50`). Those modules share `op`,
`raster`, `chunk` and `buffers`, so the whole `rewrite/` tree (9 files plus `rewrite.rs`, 791 lines at the review snapshot;
re-count after Phase 7) moves as one unit. Six parts, in order:

### D.1 Move the tree and re-point it

1. Create `crates/codec/src/rewrite/` (`pub mod rewrite;` in `lib.rs`) with the nine files of `crates/edit/src/rewrite/` and
   the module file `crates/edit/src/rewrite.rs`. Its test needs the `gif_rgb` fixture, which also moves (D.5).
2. `op.rs` already calls `pg_codec::decode::transform`; it becomes a local call. Re-point every `pg_codec::`
   self-reference in the moved tree to `crate::` (`grep -rn 'pg_codec::' crates/edit/src/rewrite*`):
   - `raster.rs:18` and its doc link at `:12`;
   - `op.rs:5` (`use pg_codec::decode::transform`, which becomes a `crate::decode::transform` import);
   - the one call in the `rewrite.rs` test (`:60`).
   `crate::max_decode_bytes()` works because `lib.rs` re-exports it.
3. `crates/codec/Cargo.toml`: move `pg-container` from `[dev-dependencies]` (it is there for a PNG-chunk test helper) to
   `[dependencies]`, and add `weezl` (used only by `rewrite/gif.rs`). `tools/crate-layers.txt:15` becomes
   `codec: container model`. `container` is a leaf, so layering and the dev-edge cycle check stay clean.

### D.2 Errors: no new error enum

`pg_codec::Error` already has `Invalid`, `Decode`, `Encode` and the `pub(crate)` helpers (`invalid`, `decode`, `encode`), and
`pg_edit::Error::Codec(#[from] pg_codec::Error)` exists.

1. Add two variants to `pg_codec::Error`, `Imperfect(String)` and `NothingLeft`, keeping edit's texts verbatim
   (`#[error("not a perfect transform: {0}")]`, `#[error("nothing of the GIF's image is left")]`). `Display` is `{step}` or
   `{0}` in both enums, so no output text changes. `pg_codec::Error` is not `#[non_exhaustive]` and nothing matches its
   variants exhaustively.
2. Write the moved code against codec's helpers. The 23 `Error::bare` sites inside `rewrite/` (count at the review
   snapshot, `crop.rs:44` and `:50` excluded: `bmp` 5, `gif` 4, `jpeg` 3, `png` 6, `raster` 1, `webp` 4); find them with
   `grep -n "Error::bare" crates/edit/src/rewrite`:
   - integer conversions (`TryFromIntError`, at `bmp.rs:26,42,43,44,46`, `png.rs:30` (two on the line), `gif.rs:105,106,
     110`, `raster.rs:33`, `webp.rs:24` (two)) become `Invalid(String)`;
   - codec-library calls become `Decode` or `Encode` by direction: decode at `png.rs:17,28` and `webp.rs:16,21`; encode at
     `png.rs:44-46` and `webp.rs:27`; the `weezl` encode error at `gif.rs:101` is an `Encode`; `Transformer::new` and the
     transform error at `jpeg.rs:38,42` are `Error::decoder` and `Error::encoder` (S4);
   - `Error::step(...)` at `gif.rs:31` (`"damaged GIF"`) becomes `Error::decode(...)`; the one at `buffers.rs:10` is a
     `Vec::try_reserve_exact` allocation failure whose text is already `"not enough memory to decode the image"`:
     `Error::decode("not enough memory to decode the image", e)` (`error.rs:58`) changes only the variant, and keeps the
     `TryReserveError` as its source (not `Invalid(String)`, which would drop it).
3. `pg_edit::Error` drops `Imperfect` and `NothingLeft` (`edit/src/lib.rs:19-23`; fix the doc comment at `lib.rs:8-9`). Its
   remaining failures are edit's own (`crop.rs:35,47,53,57,60,65,83,89`, `rotate.rs:50`). With `crop.rs:50` moved to
   `jpeg::size` (D.3), `crop.rs:44` (`u32::try_from(step)`) is the only `Error::bare` left: use
   `Error::invalid(e.to_string())` there and delete `Step`, `Source`, `step` and `bare` from edit's `Error`, which becomes
   `Invalid` plus `Codec`.
4. Callers change. After the move `rewrite::jpeg::rewrite` and `gif::rewrite` return `pg_codec::Result`, so nothing wraps
   the error in `Codec`: `rotate.rs:61,74` match `Err(pg_codec::Error::Imperfect(_))` and
   `Err(pg_codec::Error::NothingLeft)` with `Err(e) => return Err(e.into())`; the test at `rotate.rs:279` calls
   `rewrite::jpeg::rewrite` directly and matches the bare codec variant. No other crate matches on `pg_edit::Error`
   variants (`crop/src/lib.rs:38` and `writeback/src/error.rs:21` only use `#[from]`).

### D.3 Edit's call sites, visibility and `jpeg::size`

1. `crates/edit/src/{crop,rotate}.rs`: switch `crate::rewrite::...` to `pg_codec::rewrite::...` (call sites
   `crop.rs:43-70`, `rotate.rs:58-77`). Edit keeps format detection, orientation totals and metadata handling.
2. Visibility. The modules are `pub(crate)` today (`edit/src/rewrite.rs:10-18`): `bmp`, `chunk`, `gif`, `jpeg`, `op`, `png`,
   `webp` become `pub mod` in codec (with docs), while `buffers` and `raster` stay private. These items become documented
   `pub` items (the workspace warns on `unreachable_pub`, `Cargo.toml:72`): `chunk::{WebpKind, png_animated, webp_kind}`,
   `op::Op`, `{jpeg,png,webp,gif,bmp}::rewrite` and `jpeg::mcu`.
3. The two helpers only tests use (`chunk::png_chunk` at `rotate.rs:87`, `raster::pack` at `rotate.rs:88`) are not made API:
   `rotate.rs:87` needs only `pg_container::png_chunk(..).unwrap()` (edit keeps `pg-container` as a dev-dependency), and
   `pack` serves only the 2-bit gray case at `rotate.rs:175`, so the fixture PNG writer (D.4) takes one sample per byte plus
   a bit depth and packs internally. If a wrapper is ever needed it must be a wrapper `fn`, not a `pub use`:
   `check_reexports` (`tools/boundaries.py:268`) rejects `pub use crate::...`.
4. `jpeg::size`. `crop.rs:50` is production code that calls `turbojpeg::read_header(&out)` (the cropped file's size check),
   and the tests call it at `crop.rs:243`. Add `pg_codec::rewrite::jpeg::size(bytes) -> Result<(usize, usize)>` (not a
   changed return type for `rewrite`: `rotate.rs:61,278` consume its `Vec<u8>`; it is a second `read_header` site beside
   `codec/src/encode.rs:120`); `crop.rs` and its test call it.

### D.4 Tests and fixtures in edit

Unit tests move with their modules (`rewrite.rs`, `gif.rs`). The edit tests that build fixtures directly with codec crates
switch to codec fixtures, so edit needs no codec crate even as a dev-dependency:

- `turbojpeg::Compressor` at `crop.rs:180-232`; `image_webp::WebPEncoder` at `crop.rs:141-301` and `rotate.rs:225-229`;
  `png::Encoder` at `rotate.rs:168-182` (Rgb8, Rgb16, Rgba8, Gray2).
`pg_codec::fixtures` gains:
- a PNG writer taking channels and bit depth (not `png::ColorType`/`BitDepth`);
- a lossless WebP writer that can carry EXIF (`set_exif_metadata`, `crop.rs:153`, `rotate.rs:228`);
- a JPEG writer for both subsamplings, over `JpegEncoder::new(quality, Chroma::{Half,Full})` (takes an `Rgb`, so add a thin
  `&[u8]` wrapper), taking `Chroma` and not `turbojpeg::Subsamp`.
Fixture signatures must not name `png` or `turbojpeg` types: edit's tests name them today (`rotate.rs:168-175`,
`crop.rs:221`) and would keep the dependency. The assertion at `crop.rs:240` (`header.subsamp == subsamp`) compares
`jpeg::mcu(&out)` instead (16x16 for 2x2 subsampling, 8x8 for none).

### D.5 Take the image fixtures out of `pg-testkit`

`pg-codec` dev-depends on `pg-testkit`, so `testkit` cannot use codec's encoders (dev-edge cycle, rule 2 of
`boundaries.py`); remove the cause.

1. Move `png_rgb`, `bmp_rgb`, `gif_rgb`, `tiff_rgb`, `gif`, `png_rgba`, `apng`, `encode` and `Encoding` from
   `crates/testkit/src/lib.rs` into `pg_codec::fixtures` (feature `fixture`); `rgb_pattern`, `textured_rgb`, `other_rgb`,
   `gradient_rgb` and `letterboxed_rgb` are raw RGB and stay.
2. Keep the `image` crate's encoders for `encode`/`Encoding` (writing JPEG through turbojpeg would change fixture bytes;
   before changing any encoder check the size- and hash-sensitive assertions, e.g. `library/tests/crop.rs:22`,
   `library/tests/writeback.rs:27`).
3. The moved fixtures return `pg_codec::Result` (built with `Error::invalid`, `Error::encode`) instead of testkit's own
   `Error`; `pg_testkit::Error` has no users outside testkit and callers use `.unwrap()` or `Box<dyn Error>`.
4. `testkit` drops `image` and `png`; it keeps `thiserror` (its `Error::RunFolder` and `Folder` remain) and loses
   `Error::Encode`, `GifFrame` and `Pixels`. Its docs that still describe image fixtures change with them:
   `testkit/src/lib.rs:3-10`, the `Error` doc ("the encoder's or the file system's"), and the `Cargo.toml` comment at
   `:11-14` ("encoders the fixtures use (`image`, `png`)"); Phase I's step 4 then only has the re-export wording left.
5. Every crate that calls the fixtures adds `pg-codec = { features = ["fixture"] }` as a dev-dependency:
   - needs a new dev-dependency entry (it holds `pg-codec` only as a normal dependency without `fixture`): `analysis`,
     `bench`, `cli`, `render`; also new edges: `metadata` (`src/lib.rs:113`, `apng` and `gif`), `exiftool`, and `thumbs`
     (it has no `pg-codec` at all: `thumbs/src/lib.rs` calls `pg_testkit::png_rgb` twice; the edge closes no cycle, since
     `codec: model`);
   - add the feature to an existing dev-dependency: `app`; `library` (10 files) already has it with `fixture`;
   - `edit`: after the move only `crop.rs` and `rotate.rs` use fixtures; it keeps its `pg-testkit` dev-dependency for
     `rgb_pattern`, and its `pg-render` and `pg-metadata` dev-dependencies stay used.
   No cycle: `codec` depends only on `model` and, after D.1, `container`, whose tests use only `pg-testrng`. `codec` keeps
   its `pg-testkit` dev-dependency (for `Rng`) until Phase I swaps it for `pg-testrng`.

### D.6 Cleanup

- Remove `image-webp`, `png`, `turbojpeg`, `weezl` from `crates/edit/Cargo.toml`, and move `pg-container` to edit's
  `[dev-dependencies]` (after the move it appears in edit only in tests: `crop.rs:165,263,269`, `rotate.rs:91,273`; machete
  flags it otherwise); the `edit:` line of `tools/crate-layers.txt` becomes `edit: codec metadata model` (`check_layers`
  fails otherwise).
- `deny.toml`: drop `pg-edit` from the `turbojpeg`, `png` and `image-webp` wrappers; also drop `pg-testkit` from the `image`
  and `png` wrappers (D.5 removed its direct use) and reword the comment at `deny.toml:53` ("and the test fixtures'
  (pg-testkit)"). This assumes Phases A and B have landed: today the `image` line is
  `["pg-codec", "pg-testkit", "pg-library", "pdqhash"]`, and `pg-library` (Phase A) and `pdqhash` (Phase B; it is a real
  parent of `image 0.23` until then, so it must not be dropped earlier) are removed there. After A, B and D it reads
  `image: [pg-codec]` and `png: [pg-codec, image]`; the three edits touch the same lines, so D runs after A and B.
- Docs that name `pg_edit` as owner of the rewrites: `codec/src/codecs/limits.rs:6`, `codec/src/encode.rs:3`,
  `codec/src/lib.rs` module doc, and the "file generators" line in `crates/README.md` (near `:122-125`, found by text).
- `fuzz/Cargo.lock` is a separate workspace lock and regenerates; `fuzz/fuzz_targets/edit_rotate.rs` uses only
  `pg_edit::rotate::rotate`, which is unchanged.

**Done when:** `cargo test -p pg-codec -p pg-edit -p pg-crop -p pg-writeback -p pg-library` (the moved tests pass with only
fixture and path changes); `python tools/boundaries.py` (layers ok, dev edges acyclic); `just modgraph`; `just deny` (the
full `cargo deny check`) and `cargo tree -i turbojpeg -e normal --depth 1` lists only `pg-codec` (without `--depth 1` it
prints the whole transitive tree).

## Phase E: `SYSTEM_DLL` exact-name policy (`tools/build_static_ffmpeg.py:64`)

**Depends on:** nothing. **Touches:** `tools/build_static_ffmpeg.py`, a new tools test, `docs/reference/STATIC_BUILD.md`.

`SYSTEM_DLL` only feeds a stderr warning (lines 290-291).

1. Replace the name-prefix regex with `is_system_dll(name)`: true for a name in `SYSTEM_DLLS` (exact file names such as
   `KERNEL32.dll`, `USER32.dll`, `bcrypt.dll` that every supported Windows provides) or one of the `api-ms-win-` /
   `ext-ms-win-` API-set prefixes (the loader guarantees those). Match case-insensitively (lower-case the name first):
   `dumpbin` prints mixed case, and the old regex used `re.IGNORECASE`. Do not test for the file in
   `%SystemRoot%\System32` at build time: that answers for the build machine's Windows version, not the supported targets.
   The old regex also matched by prefix (`bcrypt` accepted any `bcrypt*`); exact names fix that. The list stays an
   allowlist because accepting a system DLL is a policy decision; what changes is that it is exact and enforced.
2. Move the VC runtime into an explicit `REDISTRIBUTED` set with the reason (ships with the VC redistributable, a
   deployment requirement, not Windows): `VCRUNTIME140` only. `MSVCP140` is the C++ runtime and the plain-C `static-check`
   does not import it, so that entry of the old regex is dead; exact names also reject `VCRUNTIME140_1.dll`, which the old
   prefix accepted (correct for a C program).
3. `non_system` becomes a failure (`sys.exit`), raised after the manifest is written and before the `done:` line (near line
   290), so the run is still recorded when a guard fires. The check must not pass vacuously: `dumped.returncode` is
   unchecked today and an unmatched output gives an empty `dependents` (`docs/plans/SECURITY_FLOW_REVIEW.md` SEC-159, cite
   by symbol: its line numbers are stale), so also fail when `dumpbin` exits non-zero or `dependents` is empty
   (`KERNEL32.dll` is always present). Nothing else reads `non_system` or the manifest. Do not name a local
   `libraries = [` (`tools/test_ci_files.py:267` matches the first such list).
4. Add a small unit test beside the other `tools/test_*.py` (`main()` is behind `if __name__`, so importing the script is
   safe).
5. Update `docs/reference/STATIC_BUILD.md:83` ("must show no non-system DLLs") and `:106` ("imports only system DLLs"):
   `VCRUNTIME140.dll` is not a system DLL and is accepted only through `REDISTRIBUTED`.

**Done when:** the classifier test passes on `KERNEL32.dll` and `api-ms-win-crt-runtime-l1-1-0.dll` (accepted),
`VCRUNTIME140.dll` (accepted only through `REDISTRIBUTED`), `bcryptprimitives.dll` and a made-up DLL (rejected unless
listed); `just test-tools` (ten test files today, eleven with this one). No FFmpeg build is run. No manifest from an
earlier build exists on disk (`D:\PhotoGallery\static\ffmpeg` is absent), so the exact-name list cannot be checked here
against a real `dependents` list; because the check now fails the build, expect to add names at the first real build.

## Phase F: bindgen allowlist self-check (`crates/video/build.rs`)

**Depends on:** Phase 7. **Touches:** `crates/video/build.rs`.

Bindgen silently emits nothing for a name that matches no symbol; `docs/plans/PROGRESS.md:24` records a dead
`AV_NOPTS_VALUE` entry found only by audit.

1. **Remove the dead entry first.** `LIBSWRESAMPLE_VERSION_MAJOR` is in `VARS` (`build.rs:43`) but is not generated:
   `libswresample/swresample.h` is not in `headers` (`build.rs:155`), and nothing in `crates/video/src` uses `SWRESAMPLE`
   (`load.rs:136-145` reads only the avutil, avcodec, avformat and swscale majors). Remove `SWRESAMPLE` from the `VARS`
   alternation (the `LIBRARIES` list and DLL copy are separate and stay), otherwise the new check fails on day one.
2. **Add the check inside `generate_bindings`** (not in `main`), at the end of its body, calling a new function placed
   after it. Format once: `let text = bindings.to_string();` (`Display for Bindings` exists in bindgen 0.73.2,
   `lib.rs:1106`, and runs rustfmt in a subprocess), check `text`, then `std::fs::write(out.join("ffmpeg.rs"), &text)`
   (`build.rs` has no `use std::fs`; keep the existing `map_err` text) instead of `write_to_file`, which would format a
   second time. If rustfmt is missing bindgen falls back to the raw token stream, whose spacing differs
   (`pub const X : u32`), so match const names whitespace-tolerantly. It fails the build if:
   - a `TYPES` entry has no `pub struct NAME`/`pub type NAME` followed by a non-identifier character (`AVCodec` is a prefix
     of `AVCodecContext`, `AVDictionary` of `AVDictionaryEntry`, `AVPacket` of `AVPacketSideData`, so a substring match
     would pass vacuously; all 14 are present on the real file under a word-boundary match);
   - a `VARS` entry matches no `pub const`. Enum constants are emitted under prefixed names
     (`AVMediaType_AVMEDIA_TYPE_VIDEO`, `AVPixelFormat_AV_PIX_FMT_RGB24`, `AVPacketSideDataType_AV_PKT_DATA_DISPLAYMATRIX`,
     `AVColorSpace_AVCOL_SPC_*`, `AVColorRange_AVCOL_RANGE_*`); only `#define`s (`AV_LOG_FATAL`, `AVFMT_FLAG_CUSTOM_IO`,
     `LIB*_VERSION_MAJOR`) come out bare. So match `#define` names by exact name; enum members by exact name or a suffix
     `_<entry>`; the prefix patterns by a name containing `_<prefix>`.
   - an entry is never used: also fail (or at least report) a `TYPES`/`VARS` entry that nothing in `crates/video/src/**`
     names, as an exact name, a `_<entry>` suffix or a prefix. `AVPacketSideData` is in `TYPES` but named nowhere in `src`
     (only `AVPacketSideDataType_AV_PKT_DATA_DISPLAYMATRIX` is used, and bindgen's recursive allowlisting emits the struct
     anyway: in a built `ffmpeg.rs` four emitted structs have `*mut AVPacketSideData` fields, at `side_data` and
     `coded_side_data`), so the presence check passes vacuously for it: delete it if this check fires. The enum
     constants `AV_PKT_DATA_DISPLAYMATRIX`, `AVMEDIA_TYPE_VIDEO` and `AV_PIX_FMT_RGB24` appear in `src` only under their
     enum-prefixed names, so the use check must use the suffix rule too.
   Without a regex build dependency: split `VARS` into exact names and prefixes (`AVCOL_SPC_`, `AVCOL_RANGE_`) and build the
   `LIB<name>_VERSION_MAJOR` names from the four libraries in use; pass bindgen the equivalent anchored patterns (it
   allowlists the unprefixed name, which is why the constants above are emitted). Verified against a real
   `target/debug/build/pg-video-*/out/ffmpeg.rs`: `AVCOL_SPC_` matches 20 constants and `AVCOL_RANGE_` 4, and the only
   failing entry today is `LIBSWRESAMPLE_VERSION_MAJOR`.
3. **`tools/static-qt/pg-video.patch` must still apply.** It is applied with `git apply` (`docs/reference/STATIC_BUILD.md:194`).
   Its first hunk (`@@ -82,6 +82,19`) uses `main`'s lines `generate_bindings(&headers.include)?;` / blank / `if windows {`
   as context and its second (`@@ -151`) uses `generate_bindings`'s doc line and signature, so the call goes inside the
   body, away from the signature, and `main`'s lines 82-90 are not touched. The static path also returns after
   `generate_bindings`, so the check covers the static build too. The same patch's `crates/video/src/ffi/load.rs` hunk is
   already stale (it uses `use anyhow::Result;` as context and Phase 7 changed that line): the whole patch must be
   regenerated after Phase 7. That is not this plan's task but is recorded as a dependency of the static-Qt build.

**Done when:** `git apply --check --include=crates/video/build.rs tools/static-qt/pg-video.patch` passes (it did, at offset
-3, on the current `build.rs` and after the planned edits); `cargo build -p pg-video` (needs `just setup` headers) passes;
a temporary bogus `TYPES` entry makes the build fail, then is removed.

## Phase G: forks (`sources.allow-git`, 14 entries)

**Depends on:** Phase 7 for anything touching `Cargo.toml`/locks. **Touches:** `vendor/README.md`, `Cargo.toml`, `fuzz/`,
`eval/`, `tools/{fork-updates.py,build_jpeg_turbo.py}`, later `.cargo/config.toml`, docs. **External actions** (upstream PRs)
are opened only after the user confirms each one.

**Policy.** The header of `vendor/README.md` says the crates are built from forks pinned by `rev` and that nothing is copied
into that directory any more. This plan does not reverse that. A pinned fork is a legitimate, reviewable source, so
`allow-git` is a source policy like `licenses.allow`, not an exception to empty out. The goal is that it lists only forks
that carry changes upstream has not taken (a measured-worthwhile allocation or performance change, a bug fix pending
upstream, or, for `parajpeg`, a Rust wrapper with no upstream equivalent), each justified in `vendor/README.md`.
`unknown-git = deny` stays. Four parts:

### G0 Repair `vendor/README.md`

- Eleven sections (`qt-build-utils`, `png`, `fdeflate`, `thumbhash`, `fast_image_resize`, `quick-xml`, `image-webp`, `gif`,
  `tiff`, `zune-core`, `zune-jpeg`) still describe "Copy of ... copy the new version from the registry".
- The `png` section does not mention the fork's tRNS commit, and the `cxx-qt` section's "one above `7d5897ad`" is wrong:
  the fork is two ahead ("pin vendored qt-build-utils 0.10.0", "a content key for cxx-qt-build's rerun lines").
  The `zune-jpeg` section's second and third changes (`set_skip_metadata`, a `rustfmt.toml`) are real (verified in that
  section) and are now in the G2 table's zune-image row, which said only "allocation reuse".
- The `dhat` section says it is pinned in `crates/gallery/Cargo.toml` (lines 404, 413, 416) when `gallery` uses
  `workspace = true` and the pin is `Cargo.toml:28`; its line 4 says every fork is pinned in `[patch.crates-io]`, which is
  false for `dhat` and `parajpeg`.
- The `turbojpeg-sys` section omits a third fork commit (`945d853`, drops the content-hash rerun trigger), and its "To
  update" list names the `rev` in the root `Cargo.toml`, `eval/jpeg-decoders/Cargo.toml` and `fuzz/Cargo.toml` but not
  `tools/build_jpeg_turbo.py` `FORK_COMMIT` (imported by `eval/jpeg-decoders/linux_build.py` and `win_build.py`) or
  `docs/reference/STATIC_BUILD.md:20`; add both.
Bring the sections in line with the header.

**Done when:** `grep -in "copy of" vendor/README.md`, `grep -in "copy the new version" vendor/README.md` and
`grep -n "crates/gallery/Cargo.toml" vendor/README.md` return only sections that really are copies (none, under the policy).

### G1 rusqlite flags (ordered after the upstream cache fix, not now)

- The fork's own `libsqlite3-sys/build.rs` (lines 340-359, read) adds `-DSQLITE_DEFAULT_MEMSTATUS=0` and
  `-DSQLITE_DEFAULT_LOOKASIDE=1200,200` and then reads `LIBSQLITE3_FLAGS`, so the feature composes with a developer's own
  flags.
- A `.cargo/config.toml` `[env]` entry would not (a developer's own `LIBSQLITE3_FLAGS` would replace the defaults, or with
  `force = true` the other way round). So the feature stays in the fork while the fork is still needed for the
  statement-cache fix.
- When that fix is released upstream and the fork would otherwise go: move the two flags to `.cargo/config.toml` (none
  exists), drop `photo-gallery-defaults` from `Cargo.toml:45`, and write the non-composition into the file's comment.

**Done when:** `cargo clean -p libsqlite3-sys`, then a build with `CC_ENABLE_DEBUG_OUTPUT=1` and `-vv` shows the two `-D`
flags (cc 1.10 logs compiler invocations only then, through `cargo:warning=` lines; a cached build does not rerun; that
cargo shows those warnings for a git dependency's build script is UNVERIFIED, hence `-vv`); `cargo test -p pg-catalog -p
pg-dbfile`.

### G2 Measure, then decide per fork

About ten forks exist to reuse allocations. Re-measure each against current upstream before changing anything. There is no
`just bench` recipe (the Justfile has `perf`, callgrind instruction counts in Docker, and `linux-bench`), and neither
measures Windows wall time. Name the instrument per fork:

- **Allocation-reuse forks:** allocation counts from the repo's own `dhat-heap` profile.
  - It counts only Rust-allocator blocks of a `gallery import --workers 1` run (not C/C++ allocations).
  - It needs a clean build of about nine minutes (`cargo build --profile dhat`, `docs/reference/PROFILING.md:29`) and then a
    run of unmeasured length, and depends on the `dhat-rs` fork itself, so measure before retiring that row.
  - The comparison arm is the current upstream release, built by removing that fork's patch and pin lines and relocking in a
    scratch checkout.
- **Linux instruction counts:** `just perf` (baseline `docs/perf/baseline.json`).
- **The MSVC-specific `turbojpeg-sys` change:** an in-repo `pg-codec` decode run built natively on Windows (for example a
  small decode loop over the sample folder in a `pg-codec` example).
  - The 7.48 to 4.30 ms in `docs/reference/JPEG_DECODERS.md:13` and `vendor/README.md:57` names no instrument and is the
    app's own build.
  - `eval/jpeg-decoders` is the Linux/container harness (`vips`, `ffmpeg-next`) and its `win_build.py` builds libjpeg-turbo
    with its own Release (`/O2`) CMake build, so it cannot show the fork's `/Od` regression; `crates/bench` is Linux-only
    gungraun/Valgrind.
Outcome per fork: upstream the change and drop the fork once released, drop the change if the gain is gone, or stay a pinned
fork with the measured gain written in `vendor/README.md`.

| Fork | vs upstream (ahead / behind) | Local change | Planned outcome |
|---|---|---|---|
| fdeflate | 1 / 0 | `Decompressor::reset` | upstream PR, then drop |
| image-png | 2 / 0 | scratch decode, tRNS compare expansion (commit "Expand tRNS with fixed-size pixel and key compares": a speed change, not a bug fix; `vendor/README.md` does not describe it) | PR the tRNS compare expansion; scratch is a G2 decision |
| rusqlite | 2 / 2 | statement-cache key bug, and the `photo-gallery-defaults` feature (SQLite compile flags, G1) | PR the cache fix; the defaults feature has no upstream path, so the fork stays unless the flags measure as worthless (then G1's `.cargo/config.toml` route) |
| rust-turbojpeg | 3 / 0 | `WITH_CRT_DLL`, MSVC optimisation flags, and a third commit (`945d853`) that drops the content-hash rerun trigger | upstream PR; keep until merged (decode 7.48 to 4.30 ms per README) |
| cxx-qt | 2 / 6 | `content_key_env`, QML fixes | upstream PRs |
| dhat-rs | 1 / 0 | backtraces before the lock | upstream PR |
| parajpeg | 2 / 0 | our Rust crate and shim over unchanged C++, mutex for an upstream hang | PR the mutex fix (and the wrapper if the author wants it); otherwise stays a fork |
| thumbhash, image-tiff | 1 / 0 | allocation only | measure; likely drop |
| image-gif, image-webp, quick-xml, fast_image_resize | 1/0, 1/1, 1/8, 1/8 | allocation reuse | measure; upstream PR, stay a pinned fork, or drop |
| zune-image (zune-jpeg, zune-core) | 1/42 | allocation reuse (`Scratch`), plus `JpegDecoder::set_skip_metadata` (reads the EXIF, XMP, ICC, IPTC and MPF segments only when asked) and a `rustfmt.toml` that turns formatting off (`vendor/README.md`, `## zune-jpeg`) | measure; `set_skip_metadata` is a feature, so PR it or keep the fork; 42 commits behind: rebase first |

**Upstream PRs to open (each only after the user confirms it):** rusqlite statement-cache key, fdeflate `reset`, png tRNS,
`turbojpeg-sys` flags, cxx-qt (`content_key_env` and the QML fixes), dhat, the `parajpeg` mutex.

**Done when:** before any fork change, that fork's measurement (above) is recorded in `vendor/README.md`.

### G3 Retiring a fork: every pin goes, not only the patch line

**Pins.**

- `[patch.crates-io]` (`Cargo.toml:143-157`); `fuzz/Cargo.toml:78-88`.
- Direct pins outside the patch table: `dhat` at `Cargo.toml:28`, `parajpeg` at `:40`, `rusqlite`'s feature at `:45`,
  `eval/png-decoders/Cargo.toml:14,28` (png and fdeflate), `eval/jpeg-decoders/Cargo.toml:32` (a `[patch]`).
- The static-Qt build's hard-coded `FORK_COMMIT` in `tools/build_jpeg_turbo.py:40` (also named in
  `docs/reference/STATIC_BUILD.md:20`, and imported by `eval/jpeg-decoders/linux_build.py:25` and `win_build.py:27`);
  retiring `turbojpeg-sys` breaks those build paths, which this plan never runs.
- Separate lockfiles in `fuzz/`, `eval/jpeg-decoders` and `eval/png-decoders` that must be regenerated.
- The fork's `allow-git` line and its `vendor/README.md` section.
- Documentation that names fork commits and goes stale: `docs/webclient/scan-optimizations-validation.md:64` (rusqlite
  `9d76035`, already stale against the pin `ac441bb`), `docs/reference/JPEG_DECODERS.md:243`,
  `docs/reference/PNG_DECODERS.md:14,274`, `docs/reference/STATIC_BUILD.md:20`.

**`tools/fork-updates.py` cannot detect a stale pin.** It reads the root `Cargo.lock` for `git+https://github.com/darktorres/`
sources (`forked_crates`, line 30) and only compares each locked fork's version with the crates.io release
(`newest_release`), so a left-behind pin reads as current and a retired fork vanishes from the report.

- The check that works is `grep -rn "darktorres/<fork-repo>"` over `Cargo.toml`, `fuzz/`, `eval/*/Cargo.*`, `tools/*.py`,
  `docs/reference` and `docs/webclient`, expected to return nothing for a retired fork. `docs/plans` is dated history and
  is not edited (it names `darktorres/fast_image_resize` at `GPU_BATCH_FEASIBILITY.md:129` and `darktorres/quick-xml` at
  `EXIFTOOL_PORT_PLAN.md:66`, so including it would never return nothing); a bare `darktorres` also matches the
  project's own URLs in `site/index.html` and `README.md`. Two comments that name the turbojpeg fork without the repo
  path (`eval/jpeg-decoders/linux.py:31`, `sources.py:13`) are not matched: check them by hand.
- Extending the script to the other locks (it takes one `--lock PATH`: a loop and a multi-path argument) is still worth
  doing for version drift.
- `parajpeg` is not on crates.io (the API answers 404) while the root lock has a darktorres `parajpeg` source, so
  `forked_crates` yields a row `newest_release` cannot read (`UNREADABLE`, exit 2, a failing step); the last runs
  (2026-11-16) were green, so check whether that commit is pushed.

**Done when:** after a fork is retired, `just deny`, `cargo test` of the dependents and the grep above (nothing left for that
fork's repo) are clean; for `parajpeg`, `cargo test` from a checkout of the fork (it is a git dependency, not a workspace
member, so `-p parajpeg` fails) plus the ignored `compares_real_jpegs` in `crates/codec/src/decode.rs`.

## Phase H: `pg-app` owns no re-exports (14 plain-data types move to `pg-model`)

**Depends on:** Phase 7; before Phase I. **Touches:** `crates/{app,catalog,jobs,library,thumbcache,writeback,model,desktop,
cli,bench,scan,thumbs}`, `tools/boundaries.py`, docs.

`crates/app/src/reexport.rs` re-exports 15 names so the desktop need not depend on `catalog`/`library`/`jobs`/`thumbcache`/
`writeback`; the desktop names them in 11 lines of 9 files. 14 are types, plus the function `panic_caught`.

1. **The 14 types are plain data, so they move to `pg-model`; no mirror types.** None holds a `rusqlite` type or
   crate-private state; every field type is already a model type (`Notice`, `RulePath`, `RootId`, `FileId`, `LedgerKind`,
   `MediaType`, `BorderFacet`, `pg_model::progress::Progress`) or plain (`String`, integers, `Duration`). `pg-model` already
   has `serde` and `strum`, needs no new dependency, and none of the 14 needs `thiserror`. Use the alias names the front end
   already uses, since `pg_model::refusal::RefusedReason` and `pg_model::grid::Entry` exist and a bare `Refusal` or `Entry`
   would collide:

   | Today | New name | Module in `pg-model` |
   |---|---|---|
   | catalog `Facets` (`browse/read/facets.rs`) | `Facets` | `facets` (new) |
   | catalog `Refusal` (`error.rs`) | `CatalogRefusal` | `refusal` (exists, beside `RefusedReason`) |
   | catalog `Entry` (`inbox.rs`) | `InboxEntry` | `inbox` (new) |
   | catalog `EntryId` (`inbox/id.rs`) | `InboxEntryId` | `inbox` |
   | catalog `Counts` (`ledger/state.rs`) | `InboxCounts` | `inbox` |
   | catalog `FailedFolder` (`ledger/state.rs`) | `FailedFolder` | `ledger` (new) |
   | catalog `FailedFile` (`ledger.rs`) | `FailedFile` | `ledger` |
   | catalog `FolderFailure` (`types.rs`) | `FolderFailure` | `ledger` |
   | catalog `Begun` (`removal.rs`) | `Begun` | `removal` (new) |
   | jobs `Change` (`events.rs`) | `Change` | `job` (new) |
   | jobs `Cause` (`failure.rs`) | `Cause` | `job` |
   | library `LaneProgress` (`shared.rs`) | `LaneProgress` | `progress` (exists, beside `Progress`) |
   | thumbcache `Cap` (`config.rs`) | `ThumbCap` | `cache` (new) |
   | writeback `WriteError` (`lib.rs`) | `WriteError` | `write` (exists, beside `WriteKind`, `Write`) |

2. **Module docs and catalog-side renames.** Update the `lib.rs` module doc of `pg-model`, and `refusal.rs`'s own doc, which
   opens "Why the scanner refuses a folder" while `CatalogRefusal` is about a catalog file. Inside catalog, `Counts`, `Entry`
   and `EntryId` become `InboxCounts`, `InboxEntry` and `InboxEntryId` (or are imported `as` those names), and `Refusal`
   becomes `CatalogRefusal`. `CatalogRefusal` has `&'static str` fields (`NewerMark.mark`, `PathRules.wanted`): fine today,
   but it rules out a serde derive later.

Inherent impls cannot stay behind once a type moves (orphan rule); one step per impl:

1. **`FolderFailure::key` / `from_key`** (the enum is at `catalog/src/types.rs:153`, its `impl` at `:162-175`) are pure and
   wasm-safe, and the desktop calls `folder.failure.key()` (`desktop/src/failed_files.rs:94`, `notices.rs:210`) without
   depending on `pg-catalog`: they move with the type. Only the SQL call sites (`ledger.rs:307`, `roots.rs:246`,
   `writers.rs:203`) stay in catalog, as callers.
   - Catalog-internal imports to re-point to `pg_model::ledger::FolderFailure`: `roots.rs:48` and its test imports at
     `:439`, `ledger.rs:102`, `ledger/state.rs:7`, `writers.rs` (any `crate::types::FolderFailure` or
     `crate::FolderFailure`; `types.rs` itself needs an import if it still uses the type).
   - Other crates that name it: `scan/src/lib.rs:70`, `cli/src/lib.rs:856`, `library/src/runner.rs:529`.
2. **`EntryId::entry`** (`inbox/id.rs:18`) is pure and moves with the type.
3. **`Cap`'s whole inherent `impl`** (`thumbcache/src/config.rs:41-63`: `from_gib`, `parse_gib`, `from_bytes`, `bytes`) moves
   with it; it is pure (`pg_model::num` and std).
   - The tuple field stays private in the model, and the private `const MAX_CAP_BYTES` (`config.rs:38`, used only by
     `from_gib`) moves with it.
   - Its tests move too: `config.rs` `a_cap_is_a_positive_number_of_gib_as_text`, and the whole cap test in
     `thumbcache/src/lib.rs` (`:410-420`, including its `from_bytes(-1)` and `from_bytes(0)` cases).
   - Callers to rewrite: every `pg_thumbcache::Cap` and `Cap::` hit. Grep `\bCap\b` outside `thumbcache/src/config.rs`;
     at the review snapshot: `app/src/config.rs:17`, `cli/src/args.rs:136,197`, `cli/src/lib.rs:1040`,
     `library/src/lib.rs:83`, `library/src/runner.rs:369`, `library/tests/video.rs:244`, six sites in
     `library/tests/writeback.rs`, `thumbs/src/lib.rs:533,592,777,827`, `desktop/src/lib.rs:192`
     (`pg_app::reexport::ThumbCap::parse_gib`), `writeback/src/lib.rs:805` (a test), `crates/README.md:72`, and
     thumbcache's own code and tests (`lib.rs:100,105,487,502,759,1134,1155`; the tests need `use
     pg_model::cache::ThumbCap`, since `use super::*` no longer brings it in).
4. **`Refusal::message`** (`catalog/src/error.rs:127`, about 50 lines of English, one caller: the thiserror attribute at
   `error.rs:72`) **stays in catalog** as a private function on `&Refusal`, because `pg-model` states "Nothing here is
   English" (`crates/model/src/refusal.rs:1-3`) and `pg_client::text` words outcomes. After the move `error.rs:72` becomes
   `#[error("{}", refusal_message(.reason, .path))]` (thiserror accepts this); rename `catalog_refusal()`
   (`app/src/error.rs:40`) and the `Refusal` uses in `classify.rs`, `core.rs`, `lifecycle.rs`, `lib.rs`.
5. **`Cause`.** `Cause::of(&Failed)` becomes `impl From<&Failed> for Cause` in `pg-jobs` (`&Local` counts as local, so it is
   orphan-legal; used only in `jobs/failure.rs` and `writeback/lib.rs:487`). `Cause`'s `Display` is `f.write_str(text)`
   (`jobs/src/failure.rs:171-177`), has no English, and **stays in the model unchanged**.
6. **`WriteError`'s `Display`** (`writeback/src/lib.rs:102-115`, English words for the command line and the queue's `error`
   column) becomes a free function in `pg_writeback` (`WriteError` only). Its three sites are updated:
   `writeback/src/lib.rs:499` (`catalog.write_failed(&job, &error.to_string(), offline)`, the queue's `error` column),
   `cli/src/lib.rs:720` (`error: error.to_string()` into `Line::WriterStoppedFailing`), and the test near `:887`. There are no
   `#[error("{0}")]` embeddings and no client text tables; the desktop matches the variants
   (`desktop/src/library.rs:66-69,237`).
7. **`panic_caught`** is a function, not data: `pg-app` exposes its own `pub fn panic_caught` wrapping the jobs one, in a
   `pg-app` module (since `reexport.rs` goes), and `desktop/src/panic_log.rs:21` (`pg_app::reexport::panic_caught()`) is
   rewritten to call it.
8. **Rewrite every cross-crate use, not only the desktop's.** About 35 files outside `desktop` name the moved types by old
    paths (`pg_catalog::ledger`, `inbox`, `pg_jobs::events`, `pg_thumbcache::Cap`, `pg_writeback::WriteError`,
    `shared::LaneProgress`): 15 are catalog's own; about 20 are in other crates (`app` 5, `cli` 2, `library` 6, `bench`,
    `scan`, `thumbcache` 2, `thumbs`, `writeback`, `jobs/failure.rs`), e.g. `library/tests/writeback.rs` (11 hits),
    `thumbs/src/lib.rs` (5), `bench/benches/ir/catalog.rs` (3). Many hits are types that stay (`Bus`, `Summary`): grep per
    name. The old paths cannot stay as re-exports: `boundaries.py` rejects them. Six child-module `pub use` lines name moved
    types and each loses the moved name (four also carry types that stay):
    - `catalog/src/lib.rs:54` (`pub use error::{Error, Refusal, Result}`), `:56` (`removal::{Begun, Discarded, Driven,
      Marker}`), `:63` (`FolderFailure`);
    - `catalog/src/ledger.rs:109` (`Counts`, `FailedFolder`), `catalog/src/inbox.rs:62` (`EntryId`);
    - `thumbcache/src/lib.rs:40` (`Cap`).
    Other users of moved names the first count missed: `Begun` in `app/src/library.rs`, `bench/benches/ir/catalog.rs`,
    `cli/src/lib.rs:452-469,981`, `library/tests/writeback.rs:1133,1164,1195,1229`.
9. Delete `reexport.rs`; remove the app entry from `REEXPORT_BOUNDARIES`. `reexport.rs`'s "depends on `pg-app` alone"
    wording goes with it (the desktop in fact depends on seven crates); the surviving claim is in `desktop/Cargo.toml`'s
    comment ("Not here: catalog, ..."): reword it to "the lower data crates are reached through `pg-app` and `pg-model`
    only". Update the docs that mention `reexport`: `crates/README.md:112,137,187,196` and
    `docs/reference/NOTIFICATIONS_DESIGN.md:104,197,227,320,336`.

**Done when:** `cargo check` the workspace; `just lint`; `just modgraph` (H adds modules to `pg-model`); the desktop's own
tests; `python tools/boundaries.py` shows no failure beyond baseline.

## Phase I: remove the `testkit` re-export boundary (`REEXPORT_BOUNDARIES`, testkit entry)

**Depends on:** D and H. **Touches:** `crates/testkit`, `crates/testrng`, the consumer crates' manifests and sources,
`tools/{boundaries.py,crate-layers.txt}`, docs.

`crates/testkit/src/lib.rs` re-exports `pg_platform::testing::{held, missing_tool, odd_name, skip, tool_missing}` (line 20)
and `pg_testrng::Rng` (its `pub use`, near line 137). A std-only leaf crate below `platform` is not possible
(`platform/src/testing.rs` reads `crate::env` and `windows_sys`), so the helpers stay in `platform`; the re-exports go.

1. **Delete both `pub use` lines.** 20 code files in 11 crates (plus one doc comment, `platform/src/env.rs:34`) use them
   through `pg_testkit::`:
   - `Rng` users need only `pg-testrng` as a dev-dependency (documented as available to any crate): `analysis`
     (`borders/stats.rs`, `pdq.rs`), `bench` (`ir/pairing.rs`), `catalog` (`browse/read/tests.rs`, `writers.rs`), `codec`
     (`codecs/convert.rs`), `metadata` (`container.rs`), `thumbcache` (`lib.rs`).
   - Platform-helper users already depend on `pg-platform` normally, so each needs only the `testing` feature enabled for its
     dev builds (a dev-dependency entry with `features = ["testing"]`): `app` (`yandex/save.rs`), `cli` (`lib.rs`, which
     already imports `pg_platform::testing` directly), `desktop` (`command_line`, `gui_app`, `screenshot`), `exiftool`
     (`lib.rs`), `library` (`layout.rs`, `runner.rs`, `tests/{borders,crop,video,writeback}.rs`).
   - `pg-testkit` enables `testing` in every test build today, so feature unification does not change in a way that matters.
     The dev graph is checked for cycles only, so the consumers' new dev edges need no layers entry.
2. **`testkit` itself.** Its only use of `pg-testrng` is the `pub use`, so `testkit/Cargo.toml:17` has an unused dependency
   (machete in `just lint` flags it): remove it, and change `testkit: platform testrng` in `tools/crate-layers.txt` to
   `testkit: platform` in the same commit. Two other crates are left with nothing from testkit after D and I and must
   change their dev-dependency or machete fails: `codec` (only `Rng` at `codecs/convert.rs:274`: use `pg-testrng`) and
   `metadata` (`Rng` at `container.rs:234`, `{apng, gif}` at `lib.rs:116`: `pg-testrng` and `pg-codec = { path =
   "../codec", features = ["fixture"] }` as a dev-dependency, which adds no cycle since `codec` depends only on `model`; this
   is one more reason D precedes I, since `pg_codec::fixtures` holds only `orient`, `webp_lossy_rgb` and `tiff_rgb8` until D
   moves `apng` and `gif` there).
3. **`tools/boundaries.py`:** `REEXPORT_BOUNDARIES` loses the testkit entry; with H done it is empty, so delete the constant
   and its exemption branch in `check_reexports` (a cross-crate `pub use` is never allowed), reword the docstring lines 9
   and 24 that name it, and add a `test_boundaries.py` test that a `pub use` of another crate fails everywhere.
   Precondition: re-run `python tools/boundaries.py` first; any cross-crate re-export it reports (for example
   `crates/platform/src/error.rs:3: pub use pg_model::error::chain`, in the in-progress Phase 7 work) must be resolved
   first, by Phase 7. If Phase 7 lands with one still there, Phase I's first step is to delete it and have callers name the
   original path (re-exporting from `pg-platform` is itself a second public path), or the deletion makes the failure
   permanent.
4. **Stale references:** `docs/reference/NOTIFICATIONS_DESIGN.md:197`, `crates/README.md:122-125` and `:186`,
   `crates/platform/src/env.rs:34` (names `pg_testkit::missing_tool`), `crates/library/tests/borders.rs:2`,
   `crates/testkit/src/lib.rs:3-4` and `Cargo.toml:11-13` ("re-exports them"), `crates/testrng/Cargo.toml:12` and
   `src/lib.rs:4` ("`pg-testkit` re-exports it"), `docker/check.py:7` (names `pg_testkit::skip`, which lives in
   `pg_platform::testing`), `.github/workflows/ci.yml:50` ("`pg_testkit::missing_tool`"). The comment on the testkit
   entry in `tools/boundaries.py` (`:44`) goes with the entry. (`crates/README.md:137` has no testkit mention; its sentence goes in Phase A.)

**Done when:** `python tools/boundaries.py` (testkit entry gone, `REEXPORT_BOUNDARIES` empty and removed);
`cargo check --workspace --all-targets` once for the 20 changed code files in 11 crates; the test suites of the touched
crates.

## Phase J: tooling users of `rusqlite` go through the database's owner

**Depends on:** Phase 7. **Touches:** `crates/{thumbs,thumbcache,library,import,catalog,bench,dbfile}`, `deny.toml`,
`tools/crate-layers.txt`, `tools/profiling/bench_data.py` (option 2 of the bench), docs.

`deny.toml` lists seven direct users. Owners, kept: `pg-catalog`, `pg-dbfile`, `pg-thumbcache`, and `pg-query` (it
registers the SQL functions on a connection, `register_functions` in `crates/query/src/lib.rs:425`). The rest touch another
crate's database:

1. **`pg-thumbs`, `crates/thumbs/src/fault.rs`.** `refuse_stores`/`allow_stores` open the thumbnail cache's file and create
   or drop a trigger. Move the two functions into a `fault` module of `pg-thumbcache` and have the callers
   (`library/src/runner.rs:354`, `thumbs/src/lib.rs:697,702`) call `pg_thumbcache::fault::...` directly (no re-export, so no
   new exception). Wiring: a new `fault = []` feature in `pg-thumbcache`; `pg-thumbcache = { features = ["fault"] }` in the
   dev-dependencies of `library` and `thumbs`; remove `thumbs`'s now-empty `fault` feature and `library/Cargo.toml:48`'s
   `pg-thumbs/fault`. `pg-thumbs` drops `rusqlite`, and with it the comments that explain them
   (`thumbs/Cargo.toml:13-14` on the `fault` feature and `:25` on the dependency).
2. **`pg-import`, `examples/pairs_load_probe.rs` and `pairs_write_probe.rs`.** Probes that run the catalog's pairing queries.
   - *Why they cannot move as written:* they import `pg_import::pairs::CACHE_KIB` (both) and `WRITE` (write probe); moving
     them to `crates/catalog/examples/` would need `pg-import` as a catalog dev-dependency, a cycle with `import`'s normal
     dependency on `catalog` (rule 2 of `boundaries.py`). The constants are the pairing pass's measured tuning values
     (`SLICE`, `CACHE_KIB`, `WRITE` at `import/src/pairs.rs:34,39`, documented against `PROFILING.md`, used by
     `Settings::default()` at `:66-67`), not catalog data, so they stay.
   - *What to do:* the probes take them from the `PG_PROBE_CACHE_KIB` (both probes) and `PG_PROBE_WRITE` (write probe only)
     environment variables: the repo's names (`library/examples/pairs_probe.rs:44-45`, documented at
     `import/src/pairs.rs:41-44`; neither probe imports `SLICE`, and `pairs_load_probe.rs:186` already reads
     `PG_PROBE_WORKERS`; the probes already use positional arguments for the catalog, run count and variants, so more
     positionals would collide with variant names). No defaults, a deliberate departure from
     `library/examples/pairs_probe.rs` (which falls back to `Settings`): a probe in catalog cannot read `Settings`, and a
     mirrored default drifts (`docs/plans/CONSISTENCY_AUDIT.md:3220` records the write probe's copy of `WRITE` as `2_000`
     against the pass's `4_000`), so a probe fails when one is unset.
   - Move them to `crates/catalog/examples/` (it already has `stmt_journal_probe.rs`, and `rusqlite` is a normal dependency
     of catalog). This reverses an earlier move out of `catalog/examples` (`CONSISTENCY_AUDIT.md:3386`), deliberately: the
     cycle that caused it is gone once the constants are not imported.
   - `pg-import` drops `rusqlite` and with it its whole `[dev-dependencies]` block (the two probes are its only
     `pg_testkit` users) and the comment at `import/Cargo.toml:23`. `PG_PROBE_\w+` is already in `TEST_INPUTS`.
   - Fix the stale paths and invocations: `docs/reference/PROFILING.md:962,1038` (the commands must now show
     `PG_PROBE_CACHE_KIB`, and `PG_PROBE_WRITE` for the write probe, on the command line), `import/src/pairs.rs:41-44`,
     `deny.toml:68-71`, and the module docs of both probes, which name `pg_import::pairs::CACHE_KIB`
     (`pairs_load_probe.rs:6`, `pairs_write_probe.rs:8,70`).
3. **`pg-bench`, `benches/ir/catalog.rs:68`.** The `Connection::open` sits in a loop over `catalog.sqlite` and
   `thumbs.sqlite` and sets WAL on both, so a catalog API alone cannot replace it.
   - *Evidence that the pragma is needed:* `pending_thumbs` (`catalog.rs:157-166`, `setup = copy_data`) goes through
     `thumbs::pending_in`, which does a raw `Connection::open` plus `ATTACH` (`thumbcache/src/reader.rs:62-66`) and sets only
     `query_only`, `temp_store` and `cache_size`, with no owner opening either file first. The source files are not WAL
     (verified with Python's sqlite3: `VACUUM INTO` output has header bytes 18-19 = `01 01`, non-WAL, and a plain `VACUUM`
     of a WAL database keeps `02 02`; `tools/profiling/bench_data.py` builds `thumbs.sqlite` with `VACUUM`, `catalog.sqlite`
     with `VACUUM INTO` and then a plain `VACUUM` in `seed_ledger`, with no `journal_mode` anywhere). Dropping the pragma
     would change what `pending_thumbs` measures, so `std::fs::copy` alone is not safe.
   - *Choice (the helper is the cheaper recommendation):*
     - add an "open a copied database file in WAL mode" function to `pg-dbfile` (named as a bench and test-copy helper, since
       `dbfile` states each owner keeps its own policy) and give `bench` the `dbfile` dependency (and its line in
       `tools/crate-layers.txt`); the loop is one call to it; or
     - make `bench_data.py` reopen each output after `VACUUM INTO` and set WAL (needs a `BENCH_LAYOUT` bump from
       `v1-windows-rules` and regenerating every layout, the large ones included).
   - *Not an option:* opening both files through their owners. `ThumbCache::open` (`thumbcache/src/lib.rs:95`) starts a
     writer thread and may start the file over or heal it, which would change what `pending_thumbs` measures.
   - `pg-bench` drops `rusqlite` either way.
4. Afterwards `deny.toml` reads `rusqlite: ["pg-catalog", "pg-dbfile", "pg-thumbcache", "pg-query"]` and the comment on lines
   68-71 (which explains the three extra users) is deleted.

**Done when:** `cargo check --workspace --all-targets` (examples compile; the `bench` crate builds its benches on Linux
only, so the bench change needs `just check-linux`); `just deny` with the shortened wrapper list; the tests that use the
moved fault helper (`thumbs/src/lib.rs:692`, `library/src/runner.rs:351`; they stay where they are, only the helper moves,
and nothing in `thumbcache` tests it) pass with `thumbcache`'s `fault` feature; one moved probe is run by hand with
`PG_PROBE_CACHE_KIB` (and `PG_PROBE_WRITE`) set, and fails with a clear message when unset; `just modgraph`;
`tools/check_feature_coverage.py` rule 2 (a dev-dependency with `features` counts) passes.

## Phase K: per-site lint exceptions

**Depends on:** Phase 7 (K.1-K.7 as before; K.8-K.10 are new this round and touch `cli`, `gallery`, `desktop`, `container`,
`catalog` examples and `dedup`). **Touches:** the crates named in each row below. **Rule:** one commit per lint group; read the call
sites and write the per-site list back into this plan before editing a "Fix" group.

The workspace has no `#[allow]` (`allow_attributes = "warn"`) but 225 expectations (recounted with a multi-line search
over `crates/`; `fuzz/`, `eval/` and `tools/` have none): 211 written `#[expect(...)]` and 14 written
`#[cfg_attr(cond, expect(...))]` (a plain `#[expect(` grep misses the latter). 66 are `unsafe_code` (FFI and OS
calls: desktop 39, platform 15 (6 plain, 9 `cfg_attr`: `audio.rs:6`, `clock.rs:3`, `console.rs:4`, `drive.rs:14`,
`memory.rs:3`, `message.rs:3`, `picker.rs:11`, `sqlite.rs:8`, `testing.rs:9`), dedup 5, catalog 3, video 2, container 1,
trace 1; keep, not in the table) and 159 are other lints (single- and multi-line, plus crate-level `#![expect]` in examples
and tests; the four `cfg_attr` `dead_code` and one `cfg_attr` `unwrap_used` are in K.6 and K.3). An attribute can name
several lints, so the rows sum to more than 159. The per-lint rows below were counted on the plain form and match; recount
after Phase C, which adds one `unsafe_code` expect (`env.rs`: 66 becomes 67) and turns `audio.rs`'s from a `cfg_attr`
into a plain `#![expect]` (the `cfg_attr` count 14 becomes 13).

| Lint | Sites | Where | Disposition |
|---|---|---|---|
| `unreachable_pub` | 49 | 48 in `crates/desktop` (cxx/cxx-qt bridges: `lib.rs`, `*/state.rs`), 1 in `video/src/ffi.rs` | Try one `bridges` module for the desktop's 48; keep `ffi.rs` (K.7) |
| `as_conversions` | 11 | `crates/model/src/num.rs` only | Keep: the one place conversions are written |
| casts | 16 | `cast_possible_truncation` 8, `cast_sign_loss` 5, `cast_precision_loss` 3, all in `model/src/num.rs` (lines 17-122), the same attributes as `as_conversions` | Keep, together with it |
| `unwrap_used` | 27 | test helpers in `tests/` files (`library`, `catalog`, `app`), two in `platform` | Fix (K.3) |
| `panic` | 11 | benches (6), fault injection (`borders`, `writeback`, 3), test helpers (2) | Fix benches and helpers; keep fault injection (K.3) |
| `too_many_arguments` | 7 | `scan/src/lib.rs` (3), `jobs/src/pipeline.rs` (2, one multi-line), `codec/src/decode.rs`, `catalog/src/pair.rs` (test); find by text | Fix (K.1) |
| `expect_used` | 9 | `cli/src/report.rs:25,84`, `codec/src/kernel.rs:41`, `dedup/src/union_find.rs:90`, `desktop/src/app.rs`, 4 client test files | Fix (K.2) |
| `string_slice` | 6 | `desktop` 3 (`keymap.rs`, `qml_text.rs`, `words.rs`), `yandex/src/parse.rs` 2, `metadata/src/lib.rs` 1 | Fix: `split_once`, `strip_prefix` or `str::get` (K.4) |
| `indexing_slicing` | 6 | `dedup/src/join.rs:130,1017,1070`, `metadata/src/container.rs:20`, `model/src/pdq.rs:78`, `dedup/src/union_find.rs:16` | Keep three (`pdq.rs:78`, `join.rs:1017,1070`), fix the two `const fn`s, one candidate (K.4) |
| `disallowed_methods` | 7 | `platform/src/names.rs` 5, `platform/src/testing.rs` 1, `app/src/yandex.rs:553` | Keep, one small refactor (K.5) |
| `print_stderr` 11 / `print_stdout` 5 | 16 | entry points, the log fallback, examples, benches, `platform/src/testing.rs` 2 | Fix the front ends' blanket (K.8); keep the log fallback, examples and benches; merge the two in `testing.rs` into one private function (one `#[expect]`) |
| `exit` | 2 | `desktop/src/lib.rs` (`fatal`, `-> !`, ends in `process::exit(1)`), `platform/src/process.rs` | Fix the desktop one (K.9); keep `process.rs` (a child that must skip destructors) |
| singles | 7 attributes (10 lint mentions, 9 distinct) | `ffi.rs`, `op.rs`, `classify.rs`, `writers.rs`, `controller.rs`, `convert.rs`, `dir.rs` | mixed (K.6) |

### K.1 `too_many_arguments` (7): parameter structs, one design per function

The lint fires above 7 (`clippy.toml` sets no threshold).

**scan** (`crates/scan/src/lib.rs`)

- Today (find by text; the full design, with signatures and every edit site, is S1 below): `take` (6 parameters, no lint),
  `list_here` (8), `walk_leased` (8) and `checkpoint` (8) do not share the same state.
  - `list_here` takes `&ScanLease`, `renewed`, `renew_every`, `pool` and no `ctl`.
  - `checkpoint` takes `&mut ScanLease` plus `batch`/`stats`, locals of a thread-scope closure.
  - `walk_leased` creates `pool` and `renewed` itself and gets `renew_every` from `pacing`.
- Design (S1): a `Renewal { at, every }` value (the lease heartbeat, with `due()` and `renewed()`) passed to `take`,
  `list_here` and `checkpoint` in place of `renewed` and `renew_every`; and `walk_leased`'s `root`, `root_dir`,
  `root_device`, `root_volume` grouped as one `Start<'a, D>` value, destructured on the function's first line so `root`
  stays a plain `&RulePath` for `checkpoint` and `write_batch`.
- Counts after: `take` 5, `list_here` 7, `checkpoint` 7, `walk_leased` 5.
- `walk_leased` has one caller (in `walk_paced`), `checkpoint` one (in `walk_leased`); `batch`/`stats` stay locals. The
  tests call only `walk_paced`, whose signature is unchanged.
- A full `Walker` struct only if this is not enough, and then with two lifetimes (`&'a mut ScanLease<'a>` pins the inner
  lifetime to the outer borrow because `&mut T` is invariant in `T`: see Rejected approaches).

**jobs** (`crates/jobs/src/pipeline.rs`)

- Today: `run_timed` has 8 parameters (`items, read, opts, ctl, init, process, sink, timing`), `batched` 9 (`items, read,
  opts, ctl, init, process, batch, writer, flush`), public `run` 7 (no lint).
- Design (S2): group only the shared data parameters `{items, opts, ctl}` into one `Pass<'a, T>` struct and keep everything
  else as parameters (`batch` and `writer: W` stay parameters; moving `writer` into the struct changes ownership for no
  gain). Give `run` the same `Pass` so the entry points keep one shape. (`Pass` is the name: `Job` is already a type in
  `pg-scan`, and `schedule::Pass` is a trait in another module of `pg-jobs`, which does not collide.)
- Counts after: `run_timed` 6, `batched` 7, `run` 5. Both lints are at or under the threshold, so the multi-line
  `#[expect(clippy::too_many_arguments)]` on `batched` (`pipeline.rs:421-424`) becomes unfulfilled and is deleted.
- Callers: see S2 (the grouped struct there is `Pass`, and the callers are listed once, with current lines).

**codec** (`crates/codec/src/decode.rs`)

- Today: `decode_into` has 8 parameters counting `self`; `decode_stored_into` has 7.
- Design: group the request (`format`, `exif`, `min`, `turns`) into one value. Keep `out` and `spare` as separate `&mut`
  parameters: that is what lets callers keep their capacity, and a buffers struct built per call from two disjoint fields
  gains nothing (in analysis they are separate `Analyzer` fields, `analyze.rs:51,154`, and `image` is read after the decode).
- Counts after: `decode_into` 5, `decode_stored_into` 4.
- Callers: see S3 (nine call sites plus the internal call; `render::display::decode_into` is a wrapper with its own
  signature, so `maker.rs` does not change).

**catalog test helper** (`catalog/src/pair.rs:605`): takes a small `Found` struct.

### K.2 `expect_used` (9): hold the invariant by construction

- `codec/src/kernel.rs:41` `mks2021()` returns `Result`, propagating `Filter::new`'s typed error instead of asserting it
  cannot happen. Both callers (`resize.rs:91,97`) are in `Result` methods already. `Filter::new` returns
  `Result<Self, CreateFilterError>`, a `thiserror` enum deriving `Debug, Clone, Copy, PartialEq, Eq` (read in the fork's
  checkout under `~/.cargo/git/checkouts/fast_image_resize-*`), so it meets `Error + Send + Sync + 'static` for
  `Error::resize(step, e)`: verified.
- `dedup/src/union_find.rs:90`: `number()` goes; the structure is built once from a checked count (a typed error above
  `u32::MAX`) and indexed by `u32` by construction. `dedup` has no error type (`grep 'enum Error' crates/dedup` is empty). The
  count is derived inside `from_edges` after `ids.sort_unstable(); ids.dedup()`, so the caller (`cli/src/dups.rs:84`),
  which holds only the edges, cannot supply it: make `from_edges` return `Result<UnionFind, _>` with
  `let n = u32::try_from(ids.len())?`, then `parent: (0..n).collect()` and `size: vec![1; pg_model::num::index(n)]` (not
  `n as usize`: `as_conversions` bars it outside `num.rs`).
  - `number()` has three uses, not one: `.map(number)` in `from_edges`, `self.parent[x] = number(grandparent)` in `root`,
    and `self.parent[small] = number(big)` in `union`. Hold the links as `u32` and index through `num::index`, so none
    needs a fallible conversion once `n` is checked.
  - `dedup` has no `thiserror` dependency (its manifest lists only `pg-model`): add `thiserror.workspace = true` for
    the one `pg_dedup::Error` variant, or write the impl by hand.
  - The two in-file tests (the `from_edges` calls near `:104` and `:125`) handle the `Result`; `dups.rs:84` becomes
    `UnionFind::from_edges(&edges)?.groups()`.
- `cli/src/report.rs:25,84`: run without the progress printer, and say so, when the thread cannot start (`thumb_events` has 6
  callers; the join handle becomes an `Option`).
- Client tests (crate-level `#![expect]`): the uncovered sites are the strategies at `controller_laws.rs:51-53`
  (`Factor::new(f).expect(..)`, `RowHeight::from_finite(h).expect(..)`, `Scale::new(s).expect(..)`), which map generated
  floats: use `.prop_filter_map("..", Factor::new)` and the same for the other two. The helpers called with constants
  (`row()` with 200, and `row(230)`, `row(100)` and the like in `size.rs`) are the other uncovered sites: `controller_laws.rs:24`
  (`row`), `:35` (`Scale::new(1.0).expect`), `size.rs:12,16` (`row`, `scale(v)`), `listing.rs:20`, `panels.rs:132`. Do
  not use `RowHeight::DEFAULT` for them: it is 220 (`model/src/size.rs:25`), not 200, and the tests assert `row(230)`
  after a grow. `RowHeight::new`, `Scale::new` (through `positive()`) and `Factor::new` are non-`const` because their
  bodies are (`ROW_HEIGHT.contains(&px).then_some(..)`, `.ok().map`): rewrite each body as a plain comparison, then add
  `const`; or let the helpers return `Result` and the tests `-> Result`. A `const fn new` alone does not remove the
  `.expect` from `row(px)`, which is an ordinary runtime function. Clippy's `allow-expect-in-consts` (default `true`,
  verified in its `lint_configuration.md`) exempts `expect` only in "code always evaluated at compile time", that is
  const items and const blocks, not a `const fn` body. So the working route for the constants is a `const` item:
  `const ROW_200: RowHeight = RowHeight::new(200).expect("..")` (with `new` rewritten as a plain comparison and `const`;
  `Option::expect` is const), used by the helpers; or let the helpers return `Result`. Only `controller_laws.rs` carries a crate-level `#![expect(expect_used)]`; the
  `controller/{size,listing,panels}.rs` ones are module-level. Each stays fulfilled while any `.expect` remains in its
  scope, so fix all of them before deleting the attribute. The `.expect`s inside test bodies (`controller_laws.rs:77,87,
  107,220`, `size.rs:28,34,35`, `listing.rs:167,180`) are already covered by `allow-expect-in-tests`.
- `desktop/src/app.rs`: read per site.

### K.3 `unwrap_used` (27) and `panic` (11)

- `unwrap_used` also has one `cfg_attr(not(test), expect(unwrap_used))` at `catalog/src/fixture/raw.rs:10`, a module-level
  expectation for the `fixture` feature: read it with the rest.
- `unwrap_used`: clippy's `allow-unwrap-in-tests` does not cover helper functions in integration-test files (the
  `#[expect]`s exist, and an unfulfilled `#[expect]` would warn). Make the helpers return `Result` and use `?`, with tests
  `-> Result`.
  - Verified against clippy's source (`clippy_utils/src/lib.rs`, `is_in_test`): code counts as test code if it is (a) in a
    function whose name is a `#[test]` item of the same module (only under `--test`), (b) under a parent with
    `#[cfg(test)]`, matched as `CfgEntry::NameValue { name: test, .. }` only, so `#[cfg(all(test, unix))]` and
    `#[cfg(all(test, target_os = "linux"))]` modules do not count (`names.rs`, `priority.rs`), or (c) in a file whose
    path `starts_with("tests")` as rustc is given it: cargo runs rustc from the workspace root with
    `crates/<crate>/tests/x.rs`, so (c) fails here (inferred from the repo's expects on `tests/` helpers; the path cargo
    passes is not read). A helper that is not a `#[test]` fn is covered by none of them, which is why the expects exist.
    A `#[test]` function inside a `cfg(all(test, ..))` module is still covered through (a).
  - `platform/src/names.rs:379` (`cfg(all(test, unix))`) and `platform/src/priority.rs:69` (`nice_after`,
    `cfg(all(test, target_os = "linux"))`; it is `thread::spawn(..).join()`, so its error is `Box<dyn Any + Send>`) are
    Linux-only and are not compiled on this Windows host: their fixes are `Result` helpers too, validated by
    `just check-linux`, not by `just lint` alone.
- `panic`:
  - Fault injection (`borders/src/fault.rs`, `writeback/src/fault.rs`) keeps its expects (a panic is the point).
  - Test helpers (`client/tests/tree_laws.rs:17`, `model/tests/path_laws.rs:31`): `Result` as above.
  - Benches: route the five literal-parse panics through the existing `or_stop` helper (`bench/benches/ir/linux.rs:30`),
    which is itself the one remaining `panic` expect. All five sites are `Option`s (`RULES.parse` at `catalog.rs:124,239`,
    `ThumbHeight::new` at `:160`, `roots.first()` at `:178`, `catalog.root_id(..)` at `:278`) and `or_stop` takes a
    `Result<T, E: Display>`, so each becomes `or_stop(x.ok_or("..."), what)`.

### K.4 `string_slice` (6) and `indexing_slicing` (6)

- `string_slice`: three of the six are in test code; all six are fixable with `split_once`, `strip_prefix` or `str::get`
  (`yandex/src/parse.rs`, `metadata/src/lib.rs:74`, the three `desktop` files).
- `indexing_slicing`: three are keeps, by construction; the two `const fn`s can lose theirs.
  - `join.rs:130` (`widths`) and `container.rs:20` (`nul_terminated`) are `const fn`s, where `get` and iterators are
    unavailable, but slice patterns and `split_at_mut`/`copy_from_slice` are not: `nul_terminated` becomes
    `let (head, _) = out.split_at_mut(text.len()); head.copy_from_slice(text);` and `widths` walks `out` with
    `while let [slot, tail @ ..] = rest { *slot = best; rest = tail; }`. Verified in the pinned toolchain's own std docs
    (`rustc 1.101.0-nightly`): `copy_from_slice` is `const` since 1.87, `split_at_mut` since 1.83.
  - `pdq.rs:78` `transpose_rows` is `#[inline(always)]` with a comment recording a 14 ms regression when that changed.
  - `join.rs:1017,1070` are hot passes indexed by `usize::from(u8)`.
  - (A newtype with `Index<u8>` would carry the same `#[expect]` inside its impl.)
  - Only `UnionFind` (`union_find.rs:16`) is a candidate, and only if `ids`, `parent` and `size` (the impl-level reason is
    that the three vectors have one length) all become one `Vec<Node>`; merging only `parent` and `size` still leaves
    `ids[x]` indexed.

### K.5 `disallowed_methods` (7)

Keep: each is the one owner of the call the lint bans elsewhere.

- One possible merge: `platform/src/names.rs:177` calling the private `typed_path` (`:207`) clashes on the stated reasons
  (`typed_path`: "the parts that do not exist are taken as typed"; `:177` passes an already-resolved existing path: "the
  real path is already the catalog's spelling"). Merge only by giving the helper a neutral name and reason, so 5 becomes 4;
  otherwise keep both expects.
- `app/src/yandex.rs:553` is a real caller outside the owner, but its reason ("kept as they gave it") is a format choice:
  keep it.

### K.6 Singles (7 attributes, 10 lint mentions)

- Keep: `dead_code`, `non_camel_case_types`, `non_upper_case_globals`, `unreachable_pub` and one `type_complexity` are ONE
  attribute on bindgen's `mod generated` (`video/src/ffi.rs:11`, "every constant of the enums"). `useless_conversion`
  (`platform/src/dir.rs:373`, `cfg(target_os = "linux")`, reason "needed on 32-bit"): keep unless a 32-bit target is
  dropped; it cannot be checked on this Windows host.
- Fix:
  - `neg_cmp_op_on_partial_ord` (`codec/src/codecs/convert.rs:191`): `!(v < 1.0)` becomes `v.is_nan() || v >= 1.0`.
  - `unnecessary_wraps` (`catalog/src/writers.rs:865`): keep only the `#[cfg(any(test, feature = "fixture"))]` definition,
    delete the `cfg(not(..))` stub with its expect, and put the same `cfg` on both `injected_ledger_fault()?;` statements
    (`:884` and `:912`, two call sites).
  - The four `cfg_attr(.., expect(dead_code))` (`catalog/src/rebuild.rs:175` `TRANSIENT`, `catalog/src/schema.rs:32`,
    `dedup/src/join.rs:307,312` on the `Popcnt` and `Avx2` variants): read each; keep one that is dead only on some `cfg`, delete the item
    where it is dead on all.
  - `large_enum_variant` (`client/src/controller.rs`): box the variant.
  - `permissions_set_readonly_false` (`catalog/src/classify.rs`): read it.
  - The other `type_complexity` (`Op::frame`, at `edit/src/rewrite/op.rs:61` today and `codec/src/rewrite/op.rs` after Phase
    D) takes a named type; do it after Phase D.

### K.7 `unreachable_pub` (49) in the desktop bridges

The desktop's 48 are generated-code expectations repeated per bridge (about 29 modules in `lib.rs`, two each, plus the
`*/state.rs` structs). The earlier note that one parent module would fail because `cxx-qt-build` derives module paths
from file paths was not supported: in the pinned cxx-qt (`~/.cargo/git/checkouts/cxx-qt-*/*/crates/cxx-qt-build/src/lib.rs`
near `:895-915`) the only layout constraint found is "only one directory is supported per QmlModule for rust_files"
(QTBUG-93443), and `desktop/build.rs` already passes its bridges as one explicit `.files([...])` list (`:81`). So:

- Try it: move the `*_bridge.rs` files into one directory (`src/bridges/`), with the state structs they need, and declare
  `#[expect(unsafe_code, ..)] #[expect(unreachable_pub, ..)] mod bridges;` once in `lib.rs`, replacing about 58 attributes.
  `live_reload.rs` is registered through `.file()` (`build.rs:126`) and stays where it is. Update the paths in
  `build.rs`'s `.files([...])` and any `cpp/` include that names a bridge by file stem.
- Whether it builds is UNVERIFIED until `cargo check -p pg-desktop` (needs `PG_QT_DIR`); revert to per-module expects
  if `cxx-qt-build` objects, and write the reason where the expects stay.
- Rejected: `#[doc(hidden)] pub` (it creates API) and `pub(crate)` on the state structs (E0446).

### K.8 `print_stdout` / `print_stderr` in the front ends

`cli/src/lib.rs` (crate-level, near `:6`) and `gallery/src/main.rs` (near `:9`) carry blanket expects for printing. The
repo already has the lint-free idiom `let _ = writeln!(std::io::stderr(), ..)` (`desktop/src/lib.rs`, `fatal`), and
`println!` panics on a closed pipe (`gallery scan | head`). Route all output through one small `out`/`err` module in
`pg-cli` (one expect, or `writeln!` returning `io::Result` and propagated), and have `gallery/src/main.rs` print through
`pg_cli` instead of carrying its own. That narrows the 11 + 5 sites to about two. The log fallback, examples and benches
keep theirs.

### K.9 `exit` in the desktop

`desktop/src/lib.rs` `fatal(message) -> !` ends in `std::process::exit(1)`, and `run()` already returns `i32` and already
`return`s other failures (`refuse_screenshot`). Make `fatal` return `i32` (1) after the message box and change its three
call sites (`:157`, `:173`, `:209`) to `return fatal(..)`: destructors then run (the GPU thread, the log) and the dhat
hole noted at `gallery/src/main.rs:23` closes. `gallery/src/main.rs:35` `std::process::exit(1)` becomes
`return ExitCode::FAILURE` for the same reason (it is not linted only because clippy exempts `main`; the dhat profile is
written only if `main` returns). Only `platform/src/process.rs` (a child that must skip destructors) is a real keep.

### K.10 `unsafe_code` sites that need not exist

Of the 66 kept `unsafe_code` expects, three groups can go (each is example- or helper-only):

- `container/examples/png_read_bench.rs` (`:128-149`) hand-declares kernel32 `GetFileInformationByHandle` through
  `unsafe extern "system"`: an OS call outside `pg-platform`, which `boundaries.py` misses because it matches only the
  `windows_sys` crate name (`OS_CRATE`). Move the bench to a crate allowed to depend on `pg-platform` (for example
  `pg-bench`) and use the `file_id` that `pg_platform::dir` listing already produces. Also add `extern "system"` /
  `extern "C"` outside `platform`, `trace`, `video` and `desktop` to `boundaries.py`'s platform rule, with a test.
- `catalog/examples/stmt_journal_probe.rs` (`:232`, `:255`, `:556`): raw SQLite C calls through `ffi::`. Check whether
  `rusqlite::ffi` already provides each; where it does, the expect is still needed (the call is `unsafe`), so these stay
  unless the probe is rewritten on `rusqlite`'s safe API (`Connection::db_status`, `set_db_config`); read before deciding.
- `dedup/src/join.rs` `scan_popcnt` (`:1437`) is an `unsafe fn` with `#[target_feature(enable = "popcnt")]` whose body
  only calls the safe `scan_scalar`; its sibling `suffix_avx2` is already a safe `#[target_feature] fn`. Make it safe
  and delete its expect; the call site (`:2054`) already holds the `unsafe` block. UNVERIFIED against a build; `scan_avx2`
  keeps its expect (its loads are `unsafe` blocks).
Module-wide `#![expect(unsafe_code)]` in `platform/src/{dir,dll,file,priority,process,volume}.rs` and `video/src/ffi.rs`
is deliberate: `undocumented_unsafe_blocks` still forces a SAFETY comment on every new block, which is the per-site check.

### K.11 The three `clippy.toml` files (`clippy.toml`, `crates/model/clippy.toml`, `crates/client/clippy.toml`)

Clippy reads the nearest `clippy.toml` and does not merge, so a crate that needs extra bans must repeat the root's
settings. They are not pure copies, which is why a single shared file is not possible:

- The five `allow-*-in-tests` lines and the three hash-collection `disallowed-types` bans are identical in all three.
- The root's `disallowed-methods` also names `pg_platform::names::rule_path_in` and `rule_path_of`. An earlier draft said
  `model` and `client` cannot repeat these because they do not depend on `pg-platform`, and that clippy rejects an
  unresolvable path. That is wrong, and clippy's own UI test `toml_unloaded_crate` shows why: a `disallowed-*` path whose
  first segment is a crate that is not loaded in the crate being linted is ignored silently; only a path into a loaded
  crate that names a missing item warns ("does not refer to a reachable function", suppressed per entry by
  `allow-invalid = true`). It also matches the repo: the Phase 8 commit message records `just lint` run clean, and 11
  crates with no own `clippy.toml` and no `pg-platform` dependency (`analysis`,
  `bench`, `codec`, `container`, `dedup`, `edit`, `jobs-testkit`, `metadata`, `query`, `testrng`, `yandex`) already
  inherit those two bans and lint clean. So `model` and `client` could use the root's method list as it stands; the only
  reason they have files of their own is the extra bans below.
- `model` and `client` add what keeps them wasm-safe: `Instant`, `SystemTime`, `thread::spawn` and four `std::fs` calls.
  These compile on `wasm32-unknown-unknown` (`tools/lint.py:24` checks only that they build), so the lint is the only
  guard; they are real, not decoration.

Verdict. Per-crate `clippy.toml` files are supported but they are a workaround for the missing inheritance, not the
idiomatic tool: `clippy.toml` configures lints, `[workspace.lints]` sets their levels, and "this crate must not touch the
platform" is properly a structural rule. For `model` the structural rule is available:

- **`pg-model`: make it `#![no_std]` with `extern crate alloc`, and delete `crates/model/clippy.toml`.** `Instant`,
  `SystemTime`, `thread`, `fs`, and also `net`, `process` and `env` (which the ban list never covered) then do not exist
  in the crate, so the guarantee is compile-time and cannot drift from a list. The std use in `model/src` is small:
  almost all of it is `core`/`alloc` (`fmt`, `error::Error` is `core::error::Error`, `mem`, `ops`, `str::from_utf8`,
  `Duration` is `core::time::Duration`, `BTreeMap`/`BTreeSet`/`Cow` are `alloc`). The std-only items are:
  - `analysis.rs:4` `std::io::{self, Read}` (a function that reads a hash from a reader): move it to the crate that
    owns a reader (`pg-analysis` or its caller), leaving the model the value type;
  - `notice.rs:1458` `static TABLE: OnceLock<Vec<ResolverRow>>`: build the table per call, or make it a `const` table or
    `match`, whichever the call frequency allows (read the call sites first);
  - dependencies (checked in the registry sources): `blake3` 1.8.7 has a `std` feature that is on by default and
    `model` uses only `blake3::hash` and `Hasher::new`/`update`/`finalize` (`analysis.rs:18,23`), none of which needs it;
    `serde`, `strum` and `arrayvec` are `cfg_attr(not(feature = "std"), no_std)`; `serde_json` has an `alloc` feature.
    `#![no_std]` restricts only the crate's own source, and cargo unifies the dependencies' features across the build,
    so do **not** touch the workspace's `default-features` for any of them: the guarantee does not depend on it. The
    optional `specta` feature does need `std`: its derive expands to `::std::vec::Vec`, `::std::borrow::Cow` and
    `::std::any::type_name`, and `specta` has no `no_std` support. So the attribute is required, not cautious: write
    `#![cfg_attr(not(feature = "specta"), no_std)]` (the wasm32 check builds default features, so the guarantee holds
    there; `lint.py` clippies the `specta` build separately, where it does not) and, under the feature, `extern crate
    std;` so the derive's `::std` paths resolve.
  - tests use `std`: add `#[cfg(test)] extern crate std;`.
- **`pg-client`: one crate cannot be `no_std`, but its std part can be split off.** `fluent-bundle` 0.16 (read in the
  registry) has no `no_std` support and, with its `intl-memoizer`, uses `std::collections::HashMap`, `RefCell` and
  `Mutex`. But it is used only under `client/src/text/` (`cli.rs`, `message.rs`, `notice.rs`, `words.rs`: about 3,000 of
  the crate's 7,842 lines), and the split is clean: no code outside `text/` calls into it (two doc-comment mentions
  only), and `text/` calls nothing else in `client` (it needs `pg-model` only). `serde_json`, used once in
  `controller/replay.rs`, has an `alloc`-only mode (`default-features = false, features = ["alloc", "float_roundtrip"]`;
  confirm the feature set). So:
  - Move `client/src/text.rs` and `text/` into a new crate (name it for what it does, for example `pg-wording`), layered
    next to `pg-client` above `pg-model`; add its line to `tools/crate-layers.txt`; it keeps `fluent-bundle`,
    `unic-langid` and the `locales/` files (check where they live and what reads them: `tools/` and the web client may).
  - Rewrite the importers: about 28 files in 9 crates name `pg_client::text` (`desktop` 16, `cli` 3, `app`, `gallery`,
    `jobs`, `scan`, `writeback` 1 each; `model` and `client` have doc mentions only). Grep `client::text`; the
    `pg_client::text::render` call in `desktop/src/lib.rs` is one. The `pg-client` doc that says `text::hint` joins
    words moves with it. Phase H's own text names `pg_client::text` too (the `write_error_words` doc, the "Nothing
    here is English" argument in step 6 and the `Refusal::message` note, and the `model` doc mentions): re-point them
    in this step, since K.11 runs after H.
  - Make `pg-client` `#![no_std]` with `extern crate alloc` (`Atomic*`, `RefCell`, `mem`, `ops`, `Duration` are all in
    `core`/`alloc`; `AtomicBool` is in `words.rs`, which moves out), with `#[cfg(test)] extern crate std;`, and delete
    `crates/client/clippy.toml`.
  - The new wording crate is std-based but must stay platform-free (it builds for wasm32 with the client), so it keeps the
    one remaining crate-level `clippy.toml`, with a header saying why (`fluent-bundle` needs `std`, so the ban list is the
    only guard) and the sync test below.
  The split is justified by the guarantee, not by the file count: any std-based crate that must stay platform-free needs
  a lint file until clippy gains inheritance, so the floor is the root plus one. Verified against clippy's own book
  (`book/src/configuration.md`): the file is looked up from `CLIPPY_CONF_DIR`, else `CARGO_MANIFEST_DIR`, else the
  current directory, then up the parent directories, and the first one found is the only one read (no merging; the
  `".."` value extends a list's defaults, not another file). Inheritance and per-package settings are open upstream
  requests (`rust-lang/rust-clippy` #1295 "enable inheriting of clippy.toml", #7353 "project-wide and package-specific
  settings"), as are reading the config from `Cargo.toml` (#10890, #17201); the book also calls the file format
  unstable. So repeating the root in a crate file, with a test that guards the repeat, is the documented limit of the
  mechanism, not a repo habit. What the split buys is that `model` (all) and `client`'s logic (about 4,800 of 7,842 lines, the
  controller, layout, grid, keys, tree and viewport) can no longer touch a clock, thread, file, socket, process or
  environment, by construction rather than by a ban list that already missed `net`, `process` and `env`.
Interaction with Phase H: H moves 14 types into `pg-model`. All use only `core`/`alloc` items (`String`, `Vec`,
integers, `core::time::Duration`, `Box`), so none blocks `no_std`; but do `no_std` after H (or in the same pass) so
nothing added by H uses `std::` paths such as `std::collections::HashMap` or `std::io`, and write H's new files with
`alloc::string::String`-style imports only if `no_std` has landed first.
Result: three files become two (the root and the wording crate's), and `model` and `client` carry the guarantee in the
compiler. If `no_std` for `model` is blocked (an unmovable `std::io` use, or `blake3`
needs `std`), keep `model/clippy.toml` and treat it like `client`'s.

The sync test (`ClippyFiles` in `tools/test_ci_files.py`, `:225`) then has three gaps to close (for the wording crate,
and for `model` or `client` if either file has to stay; the test's hard-coded names change from `("model", "client")`):

1. It hard-codes `("model", "client")`. Glob `crates/*/clippy.toml` instead and assert the set is exactly those two plus
   the root's, so a third crate-level file (which would silently drop the root's allowances and bans) fails the test,
   with a guard test on a synthetic file.
2. It compares `disallowed-methods` only by `std::env::temp_dir`. Assert that every root `disallowed-methods` path that
   resolves in the crate (that is, not under `pg_platform::`) is in the crate's list, so a new root method ban cannot be
   forgotten in the crate files.
3. `crates/client/clippy.toml`'s header is stale: it lists "the hash-collection bans and allow-panic-in-tests and
   allow-print-in-tests" (three of the five keys the file now repeats) and does not name the test. Reword it as
   `crates/model/clippy.toml`'s header reads.
Considered and rejected: enforcing the purity rules with a text rule in `boundaries.py` (clippy resolves paths
semantically, through aliases and re-exports; a text match does not), and generating the files from the root plus a
per-crate fragment (the test already enforces the sync; generation adds a tool for the same guarantee). Retire the
remaining crate file when #1295, #7353 or the `Cargo.toml` configuration (#10890, #17201) lands upstream: re-check those
issues at each toolchain bump. Also rejected: `CLIPPY_CONF_DIR` per crate (it is one process-wide environment variable;
`.cargo/config.toml` `[env]` cannot vary it by package), and moving the ban list to the root with crate-level expects in
every crate that does I/O (it trades one file for a crate-level expect in most of the workspace's crates, not counted
here, and loses the per-site discipline). The
`allow-*-in-tests` keys are switches, not lists, and stay.

(Honest note on how this item was reached: the first verdict, "keep, it is a known workaround", was largely deference to
the arrangement the repo already had, and one of its reasons was false (above). The `no_std` route was not evaluated
until asked; it was checked against `model/src` and the two manifests, not built. The first `client` verdict stopped at
`fluent-bundle` unread, which was the same shortcut; the registry source and the `text/` boundary were read on the next
ask.)

**Done when:** `just lint` is clean after each lint group, and a fixed site's `#[expect]` is deleted with it (an `#[expect]`
left on a fixed site is reported as unfulfilled, which is how stale ones surface); the per-lint count in the table above
drops by the number of sites fixed in that group. The Linux-only sites (`names.rs:379`, `priority.rs:69`, `dir.rs:373`)
are validated by `just check-linux`.

## Phase L: tooling and CI exceptions

**Depends on:** L.1 and L.7 with Phase C; L.2 after the `missing_docs` pass (it renames constants in dirty crates); L.3,
L.4 and L.5 nothing. A second search of the tools, Python
config and CI found these.

1. **`tools/test_ci_files.py:292` `TEST_INPUTS`** (9 regex entries): `PG_*` names exempt from the environment registry, the
   sibling of `ENV_READERS`. `crates/platform/src/env.rs:93` states the design: a test's or probe's own input is
   deliberately NOT in `VARS` (which is compiled into production and would need a new `Reader` variant) and is listed here
   with its reason, so the list is legitimate by design; what can be tightened is its precision. Every family is built from
   fixed literals: `PG_HOLDER_{PATH,MODE,READY,RELEASE}` (`catalog/tests/other_process.rs`, `dbfile/tests/live.rs`),
   `PG_THUMB_{HOLDER,CATALOG,READY,RELEASE}`, two `PG_PLATFORM_*` names plus `PG_TESTING_HELD` (a stdout marker, not an
   environment variable: `testing/tests.rs:260,280`), 13 `PG_PROBE_*` (`library/examples/{pairs,scan}_probe.rs`,
   `catalog/examples/stmt_journal_probe.rs`, `import/examples/pairs_load_probe.rs`), `PG_JOIN_*` (3), `PG_LABEL`,
   `PG_QML_RESOURCES` (also read by `tools/qmltest.py` and `main.cpp`), and the catalog/rehearsal/JPEG-compare entry (3).
   - Replace the `\w+` wildcards with exact names, 32 in all (a long list: the price of precision), and add a test that
     every listed name still occurs in the sources, since an exact list rots the way the `PG_FFMPEG` entry did (a new
     unregistered `PG_PROBE_X` then fails until listed with a reason).
   - Delete the shadowed `PG_FFMPEG` entry (the name occurs in the sources, but `.env.example:76` documents it and the
     test checks `documented` first, so the entry never applies).
   - Keep `PG_QML_RESOURCES` in `TEST_INPUTS`: it appears only in `crates/desktop/tests/qml_host/main.cpp` and
     `tools/qmltest.py`, a harness input, not a setting.
2. **`tools/test_versions.py:22` `NOT_REGISTERED`** (3 entries: `BASELINE_VERSION`, `FOREIGN_USER_VERSION`,
   `EARLIER_OUTPUT_VERSION`): rename all three so the version-constant pattern does not match, which empties the list.
   - Privacy would not help: the `CONST` regex makes `pub` optional, and `test_the_scan_finds_a_version_constant_and_skips_the_
     rest` asserts that `pub(crate) const BASELINE_VERSION` matches. It does match indented constants.
   - Renames: `FOREIGN_USER_VERSION` (`thumbcache/src/lib.rs:770,789`) and `EARLIER_OUTPUT_VERSION` (`:826`) in thumbcache,
     `BASELINE_VERSION` (about 25 uses in `catalog/src/schema.rs`) in catalog, e.g. `BASELINE_SCHEMA`. The new names must
     not contain `VERSION` at all (the regex matches any `\w*VERSION\w*`, so `FOREIGN_USER_VERSION_OLD` would still match):
     e.g. `FOREIGN_USER_STAMP`, `EARLIER_OUTPUT_STAMP`.
   - Follow the mentions in `docs/reference/PROFILING.md:510,622,990`, `docs/reference/NOTIFICATIONS_DESIGN.md:267,362` and
     the fixture at `tools/test_versions.py:54`.
3. **`ruff.toml:34` `ignore`** (10 global suppressions: E741, PLW2901, RUF001, N812, TRY003, PLC0415, S101, S311, S603,
   S607) and `extend-exclude = ["docs/research"]`.
   - Only `N812` and `RUF001` are eval-only: move them to `per-file-ignores` for `eval/**` (`N812` hits `eval/accel.py:265`,
     `eval/bench_decode.py:73`, `eval/models.py`, `eval/verify.py` x2, `eval/jpeg-decoders/nvjpeg.py`; `RUF001` eval 3,
     tools 0).
   - The others occur in `tools/` too: `S101` at `tools/test_ci_files.py:269,276,280` and `docker/check.py:51` (plus 8 in
     `eval/`); `S311` in `tools/profiling/pairing/grow_catalog.py:114` and
     `tools/profiling/synthetic_library.py:70,101,130,141`; `PLC0415` in `tools/check_feature_coverage.py:125`,
     `tools/ci/github_files.py:53`, `tools/compare_shots.py:464`, `tools/profiling/callgraph/{build,symbolize}.py` and
     `tools/test_check_feature_coverage.py:111`; `PLW2901` (tools 3, eval 6). Fix those sites (or fix the three `PLW2901`
     loop-variable reassignments), or scope a per-file ignore to exactly those files; a per-file ignore for `S101` must name
     `docker/check.py` too.
   - `S603`, `S607` and `TRY003` are used in `tools/` and stay. `E741` occurs only in `tools/modgraph` (`dupes.py` 4,
     `edges.py` 1), so the `ruff.toml` comment's reason ("crop sides") is stale: reword it, or rename the five variables
     and drop the ignore. `PLW2901` also occurs in `eval/` (6: `bench_decode` 3, `common` 2, `models` 1), beside tools' 3.
   - `extend-exclude = ["docs/research"]` stays: it holds four verbatim upstream `.py` files; say so in the comment.
4. **Python `# noqa` / `# type: ignore`** (43 sites in 24 files, almost all under `eval/`; 6 in `tools/`): the Python
   counterpart of Phase K. Audit them the same way (read each, fix or keep with a reason). The `# type: ignore[operator]`
   at `eval/jpeg-decoders/linux.py:145` has no type checker behind it (no mypy or pyright config exists): delete it, or
   add a checker. Breakdown: 17 `BLE001`, 12 `E402`
   (from `sys.path` inserts), 5 `E731`, 4 `S310`, plus singles.
5. **`.github/workflows/ci.yml:210` `continue-on-error: true`** on the screenshot comparison ("informational until it is
   stable"): a waiver whose exit criterion (`ci.yml:16`, "becomes required once it is stable") leaves "stable" undefined.
   The step is gated `if: github.event_name == 'pull_request'` (`ci.yml:209`), so it never runs on `main`, and
   `continue-on-error: true` makes a failed step report its job green. Define "stable" as 10 consecutive pull-request runs in
   which the step's own outcome (not the job's conclusion) is success, write that next to the `continue-on-error` line,
   and make the step required when it is met (nothing enforces this automatically: the check is a manual look at each
   run's step outcomes).
6. **Note, no action:** `#[ignore = "..."]` tests (7, each with a stated reason; six need an external dataset or variable) are
   not a list to refactor, but their `PG_*` inputs are the `TEST_INPUTS` entries in item 1.
7. **`tools/perf_check.py:38-40`**: `PG_BENCH_SKIP_CATALOG` switches on `--skip pg-bench/ir/catalog/` and is registered
   nowhere (neither `VARS`, `.env.example` nor `TEST_INPUTS`, because the registry scan reads only `crates/`); it is also
   forwarded into the container by `tools/linux.py:324`. Document it where the other switches are, or remove it. Also fix
   `crates/platform/src/env.rs:207` and `.env.example:121`, which say `PG_BENCH_DATA` is "set by `just bench`", a recipe that
   does not exist (the Justfile has `perf` and `linux-bench`): it is set by `tools/profiling/bench.py:76`
   (`PG_BENCH_SKIP_CATALOG` is already documented in the `perf_check.py` docstring, lines 5-7).

8. **Crate lists in `tools/test_ci_files.py`.**
   - `ANYHOW_PERMANENT = {"cli", "desktop", "gallery"}` (`:371`, enforced at `:396,407`) is the gate that remains once
     `tools/anyhow-allow.txt` is empty. `crates/gallery/Cargo.toml` has no `anyhow` dependency (and `crates/gallery/src`
     has no mention of it; `git log -S anyhow -- crates/gallery/Cargo.toml` finds no commit, so it never had one), so that
     entry is stale. Why no test caught it: `test_the_permanent_crates_are_crates` (`:405-407`) checks only that each
     permanent name is a directory under `crates/` (`present` is the set of manifests, not of `anyhow` users), and
     `test_every_listed_crate_still_depends_on_anyhow` (`:401`) covers only `anyhow-allow.txt`, not `ANYHOW_PERMANENT`.
     So the stale-entry check exists for one list and not for the other. Fix both: drop `gallery`, and make the third test
     assert `ANYHOW_PERMANENT <= crates_depending()` (so a permanent crate that stops using `anyhow` fails it), with a
     guard test on a synthetic case. Also fix the comments that say `gallery` keeps `anyhow` (`test_ci_files.py:369`,
     and the header of `tools/anyhow-allow.txt`, "cli, desktop and gallery keep anyhow for good"). `cli` (4 files) and
     `desktop` (3 files) do use it in code. Write a reason beside the set for `cli` and `desktop` (the current one, "their errors are reported,
     not matched", is thin: `desktop` already matches a typed error with `downcast_ref::<pg_app::Error>()` near
     `desktop/src/lib.rs:203`), or retire them too if their `anyhow` can be replaced by the typed errors.
   - `UNSAFE_CRATES = {"dedup", "desktop", "platform", "trace", "video"}` (`:419`) is the rule CU-005 and is self-checking
     through its guard tests: it stays (add it to "stays"). `catalog` and `container` hold `unsafe_code` expects only in
     examples, which K.10 removes or keeps; their `lib.rs` still `forbid`s unsafe.
9. **`.markdownlint.jsonc`** disables `MD013` and `MD060`, and `README.md:3` has `markdownlint-disable-next-line MD033`.
   - `MD033`: replace the inline disable with `"MD033": {"allowed_elements": ["p", "img"]}` in the config
     (`markdownlint-cli` 0.50.0 is installed).
   - `MD060` (table style; options `style` = `aligned`/`any`/`compact`/`tight`, default `any`, and `aligned_delimiter`,
     read in the installed `markdownlint` docs): the repo's tables use `|---|---|` delimiter rows with padded cells
     (`docs/reference/HEAP_PROFILE.md:27`, `JPEG_DECODERS.md:48`), which is neither `compact` (`| --- | --- |`) nor
     `tight` (no padding anywhere), so enabling the rule means reformatting those tables (a table formatter is already
     installed globally on this machine: `markdown-table-formatter`, `markdown-table-prettify`; pick one and pin it).
     `MD013`: keep it disabled (the prose wraps near 100 columns and tables are long), or set
     `{"tables": false, "code_blocks": false, "line_length": 130}`.
10. **Three Rust workspaces outside every gate.** `fuzz/`, `eval/jpeg-decoders` and `eval/png-decoders` each have their own
    `Cargo.lock` with the same forks and crates.io dependencies, but `tools/lint.py` runs only `cargo metadata --locked`
    on them (lines 26-28), and `just deny`, clippy, `cargo fmt` and machete do not cover them (`Justfile:84` formats the
    root workspace only). An advisory in `fuzz/Cargo.lock` is invisible. Add `cargo deny --manifest-path <each>/Cargo.toml
    check` to `just deny`, and clippy and `fmt --check` for `fuzz/` (it runs in CI as `fuzz-smoke`, so it is shipped test
    code); the eval ones get deny only. Phase G regenerates these locks, so do it with G.
11. **Screenshot scenarios outside the gate.** `tools/compare_shots.py` `PSEUDO` (3 scenarios) are "run on the working
    tree only ... not compared", next to the `continue-on-error` of item 5. Record references for them (the pseudo-locale
    is deterministic) or state in item 5 that they are unchecked by design. `tools/shots/lanes.txt` and `INPUT_FILES` are
    inputs, not exemptions.
12. **`TEST_INPUTS` alternative (item 1).** The rejection of `VARS` rows (compiled into production) does not rule out a
    test-only module: `pg_testkit::inputs` with `pub const HOLDER_PATH: &str = "PG_HOLDER_PATH"` and the rest, so a name
    is defined once, used through a constant (a typo is a compile error), and `test_ci_files.py` parses that one file; the
    Python list and its rot test go away. It needs the child-process helpers and probe examples to read names through the
    constants (`testkit` is already a dev-dependency of all of them). Measured: the name families in item 1 occur 81
    times in `crates/` (27 distinct names; code, docs and comments together), so the edit is mechanical and moderate. A
    name read by C++ or Python (`PG_QML_RESOURCES`) cannot use a Rust constant and stays in the list. Take it for the
    Rust-only names.

Not allowlists, checked: `deny.toml` has no `[bans] skip` or `allow-registry` (it has no `licenses.exceptions` today
either; Phase B step 6 may add some);
`[licenses.private] ignore = true` is the stated policy for the workspace's own crates; `tools/modgraph/split_check.py`
`SKIP` is a heuristic stoplist; `.markdownlintignore` holds only build output directories.

**Done when:** `just test-tools` (`unittest discover` over `tools/test_*.py`) after the `TEST_INPUTS` and `NOT_REGISTERED`
changes; `cargo test -p pg-catalog -p pg-thumbcache --no-run` for the renamed constants; `ruff check .` and `just test-eval`
(ruff also covers `eval/`, `docker/check.py` and `.claude/hooks/`) after the `per-file-ignores` change; the CI run itself
for the `continue-on-error` change; for L.4, each of the 43 sites is fixed or carries a reason; for L.7,
`grep -n "just bench" crates .env.example` finds nothing and `PG_BENCH_SKIP_CATALOG` is documented or gone.

---

## Concrete designs (the former sketches, written against the code as read)

Each design below was written from the current source (Phase 7 in progress). Code is shown as it should read after the
change; "edit list" items are the exact sites. None of it has been compiled. Names were checked for collisions in the
crate they go into (`Renewal`, `Start` in `scan`; `Request` in `codec`: free; `Pass` in `jobs`: no type of that name, and
the trait `schedule::Pass` is in another module and is not glob re-exported, so it does not collide).

### S1 scan (K.1): `Renewal` and `Start`

```rust
/// When the walk's lease was last renewed, and how long it may go without one (`Pacing::renew_every`).
struct Renewal {
    at: Instant,
    every: Duration,
}

impl Renewal {
    fn new(every: Duration) -> Renewal {
        Renewal { at: Instant::now(), every }
    }
    /// The lease has gone `every` without a renewal.
    fn due(&self) -> bool {
        self.at.elapsed() >= self.every
    }
    /// The lease was just renewed (a batch's write renews it too).
    fn renewed(&mut self) {
        self.at = Instant::now();
    }
}

/// Where a walk starts: the root folder's path, its open handle, its device and its volume.
struct Start<'a, D> {
    path: &'a RulePath,
    dir: D,
    device: Option<u64>,
    volume: u64,
}
```

New signatures (parameter counts; the lint fires above 7):

```rust
fn take(pool: &Pool, key: &Key, ctl: &Control, lease: &ScanLease<'_>, renewal: &mut Renewal) -> Result<Taken>   // 5 (was 6)
fn list_here<L: Listing>(lease: &ScanLease<'_>, renewal: &mut Renewal, lister: &mut L, rules: PathRules,
                         pool: &Pool, job: &Job, dir: Option<L::Dir>) -> Result<Outcome>                       // 7 (was 8)
fn checkpoint(lease: &mut ScanLease<'_>, root: &RulePath, ctl: &Control, batch: &mut Batch,
              stats: &mut ScanStats, renewal: &mut Renewal, pool: &Pool) -> Result<bool>                       // 7 (was 8)
fn walk_leased<L: Listing>(lease: &mut ScanLease<'_>, start: Start<'_, L::Dir>, ctl: &Control,
                           lister: &mut L, pacing: Pacing) -> Result<ScanStats>                                 // 5 (was 8)
```

Edit list (line numbers from the snapshot read; find by text):

- `take` (`:686-723`): drop `renewed: &mut Instant` and `renew_every: Duration`; `if checked.elapsed() >= renew_every`
  becomes `>= renewal.every`; `*renewed = Instant::now();` becomes `renewal.renewed();`.
- `list_here` (`:793-824`): the `beat` closure becomes
  `if lost.is_none() && renewal.due() { match renew(lease) { Ok(()) => renewal.renewed(), Err(e) => lost = Some(e) } }`
  (it captures `renewal` mutably and `lost`, as it captured `renewed` before; the `rows` closure still borrows `lease`
  shared). Delete the `#[expect(clippy::too_many_arguments, ...)]` at `:792`.
- `checkpoint` (`:1269-1303`): `if renewed.elapsed() >= renew_every { renew(lease)?; *renewed = Instant::now(); }`
  becomes `if renewal.due() { renew(lease)?; renewal.renewed(); }`; the two other `*renewed = Instant::now();` (`:1272`,
  `:1285`) become `renewal.renewed();`. Delete the expect at `:1269`.
- `walk_leased` (`:973-983`, delete the expect at `:972`): first line of the body
  `let Start { path: root, dir: root_dir, device: root_device, volume: root_volume } = start;` (so every later use of
  `root`, `root_dir`, ... is unchanged); inside the scope closure `let mut renewed = Instant::now();` (`:1004`) becomes
  `let mut renewal = Renewal::new(pacing.renew_every);`; the calls at `:1036`, `:1046`, `:1052`, `:1061` pass
  `&mut renewal` and drop `pacing.renew_every`; `renewed = Instant::now();` after `write_batch` (`:1212`) becomes
  `renewal.renewed();`.
- `walk_paced` (the one caller, `:897-898`; `walk` at `:848` only calls `walk_paced`):
  `walk_leased(&mut lease, Start { path: root, dir: root_dir, device: root_device, volume: root_volume }, ctl, lister,
  pacing)`.
Borrow check: `Renewal` is a plain value owned by the scope closure and lent `&mut` to one callee at a time, as `renewed`
was; `Start` is moved into `walk_leased` and destructured at once, so `root` stays a `&RulePath` and nothing else in the
body changes. This corrects the earlier note that `root` would be used as `.path` in `checkpoint` and `write_batch`: it
is not, because of the destructuring.

### S2 jobs (K.1): `Pass`

```rust
/// What a per-file pass runs over and under: its items, its options and its control. [`run`] and [`batched`] take one.
pub struct Pass<'a, T> {
    /// What the pass runs over: item `i` is read, processed and sunk by its index.
    pub items: &'a [T],
    /// The pass's readers, workers, buffers and priority.
    pub opts: &'a Options,
    /// The pass's pause, stop and progress.
    pub ctl: &'a Control,
}
```

No `derive(Clone, Copy)`: the derive would add `T: Clone`/`T: Copy` bounds, and nothing needs `Copy` (each function
destructures `pass` once, binding only `&` fields, and `batched` passes `pass` whole to `run_timed` as its last use of it).
If a caller wants to reuse one `Pass`, add `impl<T> Clone for Pass<'_, T> { fn clone(&self) -> Self { *self } }` and
`impl<T> Copy for Pass<'_, T> {}` by hand.

```rust
pub fn run<T, S, R, I, P>(pass: Pass<'_, T>, read: impl Fn(&T, &mut NativePath) -> Read + Sync,
    init: impl Fn() -> Result<S, I> + Sync, process: impl Fn(&mut S, usize, &[u8]) -> Result<R, P> + Sync,
    sink: impl FnMut(usize, Result<R, Failed>, usize) -> Result<bool, Failed>) -> Result<(), Failed>      // 5 (was 7)
fn run_timed<T, S, R, I, P>(pass: Pass<'_, T>, read: .., init: .., process: .., mut sink: .., timing: &Timing)
    -> Result<(), Failed>                                                                                  // 6 (was 8)
pub fn batched<T, S, R, W, I, P>(pass: Pass<'_, T>, read: .., init: .., process: .., batch: Batch, writer: W,
    flush: impl FnMut(&mut W, &mut Vec<(usize, Result<R, Failed>)>) -> Result<bool, Failed> + Send)
    -> (W, Result<(), Failed>)                                                                             // 7 (was 9)
```

Edit list:

- Each of the three starts with `let Pass { items, opts, ctl } = pass;`, so their bodies are unchanged. `batched`'s call
  `run_timed(items, read, opts, ctl, init, process, <sink closure>, timing)` (`:490-514`) becomes
  `run_timed(pass, read, init, process, <sink closure>, timing)`. `run`'s call at `:226` likewise
  (`run_timed(pass, read, init, process, sink, &timing)`).
- Delete the `#[expect(clippy::too_many_arguments, ..)]` on `run_timed` (`:232`) and on `batched` (`:434-438`): at 6 and 7
  parameters neither lint fires, so each would be an unfulfilled expectation.
- Every caller: `run(&items, read, &opts, &ctl, init, process, sink)` becomes
  `run(Pass { items: &items, opts: &opts, ctl: &ctl }, read, init, process, sink)`; the same for `batched`. Callers:
  `batched` in `borders/lib:169`, `import/hashes:54`, `import/lib:132`, `thumbs/lib:407`, `thumbs/placeholders:95`; `run` in
  `cli/borders:80`, `crop/lib:171,325`; 13 test callers in `pipeline.rs` (7 `run`, `:1031-1341`; 6 `batched`,
  `:1512-1642`). `run_timed` has no other caller than `run` and `batched`. (Counts verified; line numbers drift, grep.)
`T: Sync` stays on the functions (not on `Pass`).

### S3 codec (K.1): `Request`

In `crates/codec/src/decode.rs` (it imports `crate::error::{Error, Result}`; Phase 7 moved `Error` into `error.rs`, so the
two `Error` variants of D.2 go there):

```rust
/// What a decode is asked for: the file's format (`Format::sniff`), its EXIF orientation (`pg_metadata::orientation`, 1
/// without one), the least size ([`MinSize`]) and the catalog's quarter turns.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    /// The file's format (`Format::sniff`).
    pub format: Format,
    /// The file's EXIF orientation (1 without one).
    pub exif: Orientation,
    /// The least size the pixels may be decoded to.
    pub min: MinSize,
    /// The catalog's quarter turns, composed with `exif`.
    pub turns: Turns,
}

pub fn decode_into(&mut self, bytes: &[u8], request: Request, out: &mut Rgb, spare: &mut Vec<u8>) -> Result<Info>   // 5 (was 8)
pub fn decode_stored_into(&mut self, bytes: &[u8], request: Request, out: &mut Rgb) -> Result<Info>                  // 4 (was 7)
```

Edit list: `decode_into` calls `self.decode_stored_into(bytes, request, out)?`; `decode_stored_into` starts with
`let Request { format, exif, min, turns } = request;` and its body is unchanged; delete the expect at `:130`; `dimensions`
(5 with `self`, no lint) keeps its arguments. The callers (grep over all `*.rs` finds exactly these, no fuzz, bench or eval
Rust file): `analysis/src/analyze.rs:122,138`, `analysis/examples/stage_times.rs:44`, `render/src/display.rs:40,54`,
`edit/src/lib.rs:90` (inside `testing::stored`, `decode_stored_into`), the codec tests `decode.rs:376,429,503`, and the
internal call `decode.rs:133`. Each builds `Request { format, exif, min, turns }` from the four arguments it passes today
(`turns` is the caller's value, or `Turns::NONE`, which exists, where it passes none). `maker.rs:179,190,202` and
`display.rs:60` go through `render::display::decode_into`, a wrapper that computes `format` and `exif` itself
(`display.rs:37-40`) and keeps its own signature, so they do not change. `analysis`, `render` and `edit` each add
`Request` to their `use pg_codec::decode::{...}`; the codec tests get it through `super::*`. `out` and `spare` stay
separate `&mut` parameters. `Request`'s doc names `pg_metadata::orientation` as plain text, not an intra-doc link
(`pg-codec` does not depend on `pg-metadata`).

### S4 `jpeg.rs` in codec, and `crop.rs` (D.2, D.3)

The moved `rewrite/jpeg.rs` after the error mapping (`Error::decoder` and `Error::encoder` exist in
`codec/src/error.rs:42,50` and set `step: error.to_string()`, so the text each `Error::bare` showed is unchanged; they
need `turbojpeg::Error: std::error::Error + Send + Sync + 'static`, which compiling will confirm):

```rust
use crate::{Error, Result};
use turbojpeg::{Transform, TransformCrop, TransformOp, Transformer};
use super::op::Op;

pub fn rewrite(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    /* body as today, except: */
    let Some(progressive) = pg_container::jpeg_is_progressive(bytes) else {
        return Err(Error::Imperfect("the JPEG's structure is damaged before its scan".into()));
    };
    /* ... */
    match Transformer::new().map_err(Error::decoder)?.transform_to_vec(&t, bytes) {
        Ok(out) => Ok(out),
        Err(e) if perfect => Err(Error::Imperfect(e.to_string())),
        Err(e) => Err(Error::encoder(e)),
    }
}

/// The JPEG's minimum coded unit, width and height: the grid a lossless crop starts on.
pub fn mcu(bytes: &[u8]) -> Result<(usize, usize)> {
    let header = turbojpeg::read_header(bytes).map_err(Error::decoder)?;
    Ok((header.subsamp.mcu_width(), header.subsamp.mcu_height()))
}

/// The JPEG's width and height in pixels, from its header.
pub fn size(bytes: &[u8]) -> Result<(usize, usize)> {
    let header = turbojpeg::read_header(bytes).map_err(Error::decoder)?;
    Ok((header.width, header.height))
}
```

`crates/edit/src/crop.rs:43-55` after the change:

```rust
let (mw, mh) = rewrite::jpeg::mcu(bytes)?;
let grid = |n: u32, step: usize| u32::try_from(step).map(|s| n.next_multiple_of(s)).map_err(|e| Error::invalid(e.to_string()));
let (l, t) = (grid(l, mw)?, grid(t, mh)?);
/* ... unchanged ... */
let out = rewrite::jpeg::rewrite(bytes, exact(l, t))?;
let got = rewrite::jpeg::size(&out)?;
let want = (num::index(width - l - r), num::index(height - t - b));
if got != want {
    return Err(Error::invalid(format!("the crop came out {}×{}", got.0, got.1)));
}
```

(`?` converts `pg_codec::Error` through `Error::Codec(#[from])`.) The `Imperfect` text is identical to today's; edit's
`rotate.rs:61,74` then match `Err(pg_codec::Error::Imperfect(_))` as in D.2.

### S5 Phase H: the `pg-model` modules and the replacement functions

New files in `crates/model/src/` (each a `//!` doc line plus the items moved **verbatim**, doc comments and derives
included, with `use` lines pointing at the same items under `crate::`); add `pub mod cache; pub mod facets; pub mod inbox;
pub mod job; pub mod ledger; pub mod removal;` to `lib.rs` (keep its alphabetical order) and extend its module list doc
(the outcomes the front ends word: inbox entries and counts, the folder ledger, facet counts, removals, job events and
causes, the thumbnail cache's cap). Item by item, with the source to move from:

| New file | Items moved verbatim | From |
|---|---|---|
| `cache.rs` | `ThumbCap` (was `Cap`), `const MAX_CAP_BYTES`, its `impl`, the test `a_cap_is_a_positive_number_of_gib_as_text` | `thumbcache/src/config.rs:33-64, 77-91` |
| `facets.rs` | `Facets` | `catalog/src/browse/read/facets.rs:20-32` |
| `inbox.rs` | `InboxEntry`, `InboxEntryId` + `fn entry`, `InboxCounts` | `catalog/src/inbox.rs:72-`, `inbox/id.rs:5-27`, `ledger/state.rs:61-66` |
| `ledger.rs` | `FolderFailure` + `key`/`from_key`, `FailedFolder`, `FailedFile` | `catalog/src/types.rs:151-177`, `ledger/state.rs:68-`, `ledger.rs:140-` |
| `removal.rs` | `Begun` | `catalog/src/removal.rs:36-` |
| `job.rs` | `Change`, `Cause` + its `Display` | `jobs/src/events.rs:128-143`, `jobs/src/failure.rs:163-191` |
| `refusal.rs` (exists) | `CatalogRefusal` (was `Refusal`, no `message`) | `catalog/src/error.rs:97-125` |
| `write.rs` (exists) | `WriteError` (no `Display`) | `writeback/src/lib.rs:90-98` |
| `progress.rs` (exists) | `LaneProgress` | `library/src/shared.rs:125-` |

"Verbatim" has one exception: rustdoc links to items that stay behind become plain code spans, because `pg-model` cannot
link up the layers (`Entry`'s `[`crate::ledger::Summary::seen_to`]`, `Begun`'s `[`Catalog::begin_removal`]`,
`Refusal::OlderSchema`'s `[`crate::Catalog::open`]`; grep each moved item for the bracketed-link form). `Cap`'s second doc sentence (the
`--cap-gib` and `PG_THUMB_CAP_GIB` text) stays in the model's docs, as it describes the value's users; and the
`cache.rs` snippet below keeps today's per-method docs (an earlier draft dropped them, which `missing_docs` rejects). Write
`write.rs` imports `crate::job::Cause`, and `ledger.rs` imports `crate::ids`, `crate::notice::LedgerKind` and
`crate::rules::Path` for its fields (checked: every field type is already in `pg-model`).

`cache.rs` as it should read (only the names and the `num` path change):

```rust
//! The thumbnail cache's size limit as a value.

/// A cache's size limit, in bytes: above zero and under `i64::MAX`, so no cap evicts every thumbnail or saturates. The
/// front ends read the number (the command line's `--cap-gib`, the app's `PG_THUMB_CAP_GIB`) and word their own errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbCap(i64);

/// The largest cap, in bytes: under `i64::MAX` (about 9.22e18).
const MAX_CAP_BYTES: f64 = 9.0e18;

impl ThumbCap {
    /// `gib` GiB (2^30 bytes) as a cap, the fraction of a byte dropped; `None` for zero, negative, NaN, infinite and
    /// absurd sizes.
    pub fn from_gib(gib: f64) -> Option<ThumbCap> {
        let bytes = gib * 1_073_741_824.0;
        (1.0..MAX_CAP_BYTES).contains(&bytes).then(|| ThumbCap(crate::num::f64_to_i64(bytes)))
    }

    /// The cap a number of GiB written as text names: the `--cap-gib` flag and the `PG_THUMB_CAP_GIB` variable both read
    /// it this way (see [`ThumbCap::from_gib`]). Surrounding spaces are ignored.
    pub fn parse_gib(text: &str) -> Option<ThumbCap> {
        text.trim().parse::<f64>().ok().and_then(ThumbCap::from_gib)
    }

    /// `bytes` as a cap; `None` unless it is above zero.
    pub const fn from_bytes(bytes: i64) -> Option<ThumbCap> {
        if bytes > 0 { Some(ThumbCap(bytes)) } else { None }
    }

    /// The cap in bytes, as the cache stores it.
    pub const fn bytes(self) -> i64 {
        self.0
    }
}
```

(The method docs are today's, moved unchanged: they now satisfy `missing_docs` as well as describing the front ends.)

The functions and impls that stay in the owning crates:

```rust
// catalog/src/error.rs: the attribute and the private function (the arms are today's `Refusal::message`, unchanged)
#[error("{}", refusal_message(.reason, .path))]
Refused { path: String, reason: Box<CatalogRefusal> },

/// The text of this refusal for the catalog at `path`.
fn refusal_message(refusal: &CatalogRefusal, path: &str) -> String {
    match refusal {
        CatalogRefusal::NewerSchema { found, current, written_by } => { /* as today */ }
        /* ... the other eight arms, as today ... */
    }
}

// jobs/src/failure.rs: replaces `impl Cause { pub fn of(..) }`
impl From<&Failed> for Cause {
    fn from(failed: &Failed) -> Cause {
        if failed.is_panic() { Cause::Panicked(failed.text().to_string()) } else { Cause::Error(failed.text().to_string()) }
    }
}
// call sites: failure.rs:149 (Cause::from(failed)), the tests at :211 and :219 (Cause::from(&failed)), writeback/src/lib.rs:489 (Cause::from(&e))

// writeback/src/lib.rs: replaces `impl Display for WriteError`
/// The English words for a write error, for the command line and the queue's `error` column; the app words it through
/// `pg_client::text`.
pub fn write_error_words(error: &WriteError) -> String {
    match error {
        WriteError::Failed(cause) => cause.to_string(),
        WriteError::CopyIncomplete { file, temp, cause } => format!(
            "{file} may be incomplete: copying its change into it failed ({cause}); the whole new content is in {temp}"
        ),
    }
}
// call sites: writeback/src/lib.rs:499 (`&error.to_string()` -> `&write_error_words(&error)`), cli/src/lib.rs:720, the test near :887
```

`Facets` stays built by `impl Catalog { pub fn facets(..) -> Result<Facets> }` (an inherent impl on the local `Catalog`,
legal after the move). The catalog-side imports become `use pg_model::{facets::Facets, inbox::{InboxCounts, InboxEntry,
InboxEntryId}, ledger::{FailedFile, FailedFolder, FolderFailure}, refusal::CatalogRefusal, removal::Begun}`; the SQL call
sites of `FolderFailure::key`/`from_key` (`ledger.rs:307`, `roots.rs:246`, `writers.rs:203`) are unchanged.

---

## Allowlist content that stays, with the reason

- `tools/crate-layers.txt`: it is the rule (exact internal edges, reviewed in the same commit as the change).
- `deny.toml` `licenses.allow`: a policy list, pruned to what is used (Phase B step 6).
- `ENV_READERS`: the registry reader's two functions (`process`, `set_process`), nothing else.
- bindgen `TYPES`/`VARS`: the mechanism that bounds generated FFI; kept, with Phase F's check.
- Single-owner bans (`ureq`, `quick-xml`, `cxx-qt`, `cxx-qt-lib`, `gif`, `webp`, `tiff`, `zune-*`): real owners.
- `deny.toml` `allow-git`: a source policy naming the pinned forks that stay (Phase G).
- Phase L leaves in place: the `ruff.toml` suppressions `S603`, `S607`, `TRY003` (and `E741`, unless its five variables
  are renamed), `TEST_INPUTS` as an exact-name
  reasoned list, and `modgraph/split_check.py` `SKIP`.
- `UNSAFE_CRATES` (`tools/test_ci_files.py`): the CU-005 rule, self-checking through its guard tests.
- Phase K keeps: the `unsafe_code` expects (66, 67 once Phase C adds `env.rs`'s; minus the K.10 sites that go; FFI and OS
  calls), `as_conversions` and the casts in `num.rs`, the owners of banned calls, the log fallback's printing,
  `platform/src/process.rs`'s `exit`, fault-injection panics, the generated-code expects if K.7's single module does not
  build, and the three `indexing_slicing` sites `pdq.rs:78`, `join.rs:1017` and `join.rs:1070`.

## Decisions (settled: the most proper option, churn not a concern)

1. Phase E: the DLL check **fails** the step. Unknown DLLs are either added to `REDISTRIBUTED` with a reason or are a real
   problem; any legitimate OS DLL the first real build flags is added deliberately.
2. Phase G: forks stay forks (the repo's recorded decision); nothing is vendored in-repo. `parajpeg` keeps its pinned fork;
   the mutex fix is sent upstream.
3. Phase G: open upstream PRs for every generic fix (rusqlite cache, fdeflate reset, png tRNS, turbojpeg flags, cxx-qt, dhat,
   parajpeg mutex). Each allocation-only fork is upstreamed, dropped, or kept with a measured gain. Each PR is
   outward-facing, so it is opened only after the user confirms that PR.
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
