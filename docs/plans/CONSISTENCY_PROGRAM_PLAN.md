# Plan (rev 11, after ten adversarial validation rounds): finish the deferred consistency-audit items, to the MOST proper end state

Status: planned, not started. Written 2026-11-21 against HEAD `1b361db7`, the commit that resolved the open audit findings
and renamed the platform keys. Its findings are `docs/plans/CONSISTENCY_AUDIT.md`, and its "Start here" line names the first step. On 2026-11-21 the GPU
track's packages and binaries were set up outside Phase 0 (the PNG harness, the encoder and colour tests, and the
nvimgcodec bench venv); `docs/plans/PROGRESS.md` will record that once it exists.

## Context

After commit `1b361db7` about 62 audit findings are still Deferred or Fixed in part. The owner's standard: "MOST proper; churn is not a concern". Two earlier mistakes of mine are corrected here:

1. Churn-softened choices were reopened: E-019, AD-011, N-039, D-063, T-020. (N-034 was reopened too, but the audit's rule turned out to be a deliberate grain rule that the code already follows: decision 15.)
2. "Needs another environment" was wrong for most RUN items: this machine is Windows with the Qt 6.12 kit, MSVC, MSYS2, the Windows SDK, an RTX 2060 + CUDA 13.4, an Intel Pin kit, `just`, `cargo-deny`, FFmpeg headers and libclang. Seven items stay external (Phase 0).

Two owner decisions were asked in plan mode: **E-019 strict** (anyhow only in cli, desktop, gallery; libraries use typed errors, enforced by a manifest test) and **D-001 one folder, one row**. Every other choice in this plan is a default I took; they are listed under "Decisions taken by default" so the owner can override any of them.

Working rules (the owner's saved preferences): edit with the Edit/Write tools, never scripts; batch validation (finish a batch of edits, then validate once); compile natively on Windows (the Linux container pass, `just check-linux`, needs Docker Desktop and is required at the end of Phases 6, 7 and 8, because CI's `linux` job runs `just lint`, `modgraph`, `test`, `qmltest` and `test-tools` on ubuntu and lints Linux-cfg code; if Docker is unavailable, say so and list the Linux-cfg edits for CI to verify); never run the hour-long static Qt build; no background tasks or monitors (run commands in the foreground; a command that exceeds the 10-minute cap is run by the owner, who pastes the result); commit only when the owner asks; never launch the app against the owner's real catalog (`.env`'s `PG_CATALOG_DIR`, `D:/PhotoGallery/catalog`, 515 MB, with `thumbs.sqlite` beside it and prefs in `D:/PhotoGallery/gallery.json`): `just shots` and `cargo test` isolate their state (`tools/compare_shots.py:366` sets `PG_CATALOG_DIR`/`PG_PREFS`; tests use `PG_TEST_DIR`), a manual run does not, so use a scratch copy. No branches: history goes straight to `main` (the owner said so earlier). Commit messages follow the repo style: an imperative subject of at most about 72 characters, a body wrapped near 120 columns that may end with "Checks run: ... Not run: ...", and the trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

**Start here**: create `docs/plans/PROGRESS.md` (markdownlint's default rules apply: an H1 first line, e.g. `# Progress: consistency-audit program`, one status paragraph giving the date and HEAD as the other plans do, then a table with its `|---|` delimiter row and the columns step, commit or "uncommitted", gate result; one final newline; delete it, or list it in the root README, when the program ends), then do Phase 0 "Backup first, then setup" in the order given. At the end of Phases 6, 7 and 8 also run `just check-linux`. A "disposition" anywhere below means the finding's `**Disposition: ...**` line in `docs/plans/CONSISTENCY_AUDIT.md`. The ledgers are `docs/plans/CLIPPY_LEDGER.md` and `docs/plans/REFACTOR_LEDGER.md`; the Fluent files are in `crates/client/locales/en/` (`text.ftl`, `ui.ftl`, `cli.ftl`).

## Gates

- **G-check** = `cargo check --workspace --all-targets --locked`, run through the repo's environment. No `just` recipe runs it today and plain `cargo` lacks qmake on PATH, so Phase 0 adds this recipe to the Justfile (`_qt` only runs `tools/check_qt.py` and exports nothing; every recipe already gets the Qt PATH from the Justfile's global `export PATH`, fed by `set dotenv-load`): `check: _qt` with the body `cargo check --workspace --all-targets --locked` (a `#` doc comment ABOVE the recipe in the Justfile's style, "Type-check the whole workspace against the lockfile (cargo)"; no `#` comment or single quote inside the recipe line, because the shell is cmd, `Justfile:10-12`; no `check` recipe exists today). It does not cover `fuzz/` and `eval/*-decoders` (separate workspaces); `tools/lint.py:26-28` runs `cargo metadata --locked` on them.
- **G-lint** = `just lint`: the `lint` recipe (`Justfile:133`, depends on `_qt _lint-tools`) runs `tools/lint.py` (fmt, clippy `-D warnings` incl. the wasm and `dhat-heap` variants, `boundaries.py`, cargo-machete, ruff, markdownlint, `cargo doc --workspace --no-deps --document-private-items` with `-D warnings`) and then the separate `lint-qml` recipe, which reads the last build's `plugin.qmltypes` (`lint_qml.py:1-5`). The pinned lint tools must be installed first.
- **G-full** = `just build debug` first, then G-lint, then `cargo test --workspace` and `just test-tools`. Add `just modgraph` in Phases 0, 6, 7 and A-028. Rebuild after any QObject-bridge change.
- **G-qt** = `just build debug` and `just qmltest`.
- **Cadence**: G-check once per edit batch; G-full once per phase, except Phases 7, 9 and 10, which span sessions: G-check per step, G-full at each layer-group end (Phase 7), each crate (Phase 9), each sub-item (Phase 10) and at the definition of done. If the Phase 0 baseline is red, fix it or record the failure before Phase 1; do not proceed on red.
- **Checkpoints**: a "commit point" is a stable state. Commits are made only when the owner asks; `docs/plans/PROGRESS.md` is updated at every commit point (with "uncommitted" when no commit was requested), so the working tree plus that table is the checkpoint. Ratchets that shrink monotonically: `tools/anyhow-allow.txt` (Phase 7) and the list of crates carrying `#![warn(missing_docs)]` (Phase 9).
- **Rules new code must obey** (workspace `Cargo.toml:69-105`, `clippy.toml`, `ruff.toml`, `crates/README.md`):
  - Rust: `unwrap_used`, `expect_used`, `indexing_slicing`, `panic`, `print_*`, `todo`, `as_conversions` and every `cast_*` are denied outside tests (tests are exempt via `clippy.toml`; non-test crates such as `testkit` are not), so use `.lock().unwrap_or_else(PoisonError::into_inner)` (the repo idiom, `thumbcache/src/writer.rs:59`), `get`/`last_mut`/`pop`, `u8::from`/`try_from` and `char`/slice helpers instead of `as`, `[i]` and `&s[..]`. `allow_attributes` and `string_slice` warn and `unreachable_pub` warns, all fatal under `-D warnings`: every suppression is `#[expect(.., reason = "..")]`, and an `expect` that does not fire is itself a warning. `clippy.toml` bans `HashMap`, `HashSet`, `RandomState` and `std::env::temp_dir`.
  - Python (`ruff.toml`: line length 130, py312; ERA, SLF, S, PTH, RET, ARG, TRY, N enabled): standard library only (`pgtools.py:1-3`), flat imports in the style of `tools/test_ci_files.py:21` (`from pgtools import REPO`, sorted first-party), `pathlib` only, no imports of underscore names, and Rust snippets in tests live inside string literals (a comment would trip ERA001).
  - Docs: `crates/README.md` rule 1 says "symbols, not line numbers": anything written into `crates/README.md` names symbols, never lines (this plan's cites are scaffolding only). Comments say why, not what; the Phase 9 docs state meaning, not the item's name. Justfile recipes carry a `#` doc comment ABOVE them in the style "The X (tools/y.py)". `.env.example` entries are a commented `# PG_X=value` with a one-line description above. Every new tool or data file gets a row in `tools/README.md`.
  - `just fmt` rewrites files repo-wide (`ruff check --fix .`, `tools/format_qml.py`): run it deliberately on touched files, never as a gate.
- **Atomic steps** (no commit point inside each): Phase 0's deletion of the two build `.ps1` files together with the `Consumers` rewrite (and no original is deleted until the owner has confirmed its port, and committed it if they asked for commits); Phase 3's last site migration together with the env enforcement rule and the registry test (any intermediate state is red); Phase 4's SQL batch, `baseline.sql` edit, `merge_folder` and carry; Phase 7's `Error::Other` removal together with the `app` edits; Phase 10's N-039 (the `text.ftl` rewrite and every reference edit are one step; ids resolve at run time and `cargo check` stays green while every message is missing, so run the existing parity test `message.rs:346-356` before and after). "Atomic" means do not stop in the middle of the step; it does not mean commit. Commit the current state before Phase 10's in-place disposition rewrites, so the old text survives in git.
- **Cites drift**: re-grep every file:line cite before using it once Phase 1 has run (Phases 1, 3, 4 and 5 shift lines in `catalog/*`, `cli/src/lib.rs`, `thumbcache/src/lib.rs`). Phases 4 and 5 precede Phase 7.
- **Size** (files): Phase 1 about 80, 2 about 10, 3 about 30, 4 about 25, 5 about 25, 6 about 30, 7 about 250, 8 about 45, 9 about 100 (700 items), 10 about 60.

## Validation history (rounds 1-9)

- Round 1: D-001 "refuse old catalogs cleanly" was false (they hit `RebuildCarry`); the CLI printer join order could deadlock; per-crate lint tables are impossible under `[lints] workspace = true`; `crate-layers.txt` cannot take a class token; T-005's command matched nothing; about 20 findings were uncovered.
- Round 2: `Env` cannot implement `Fn`; the D-001 range changes the parameter count at many SQL sites; `app` violates the visibility rule; a crate-level `#![expect(unsafe_code)]` contradicts the ledger; N-039 and D-063 were mis-scoped.
- Round 3: `lint-qml` needs a prior build; every `--locked` run fails after a manifest edit unless the lockfiles change with it; `Error::Other` removal breaks `app` mid-sequence; N-030 covered two renames of about fifteen.
- Round 4: the jobs error design had to become a boundary struct (panics are encoded as `io::Error`); the D-001 key is a spelling key, not a volume key; the thumbcache pool's `close()` must wait for checked-out readers; Phase 8 pilots first.
- Round 5 (dry runs): the jobs design needs four classes, explicit closure annotations, parallel `*_typed` entry points and a generic `guard`; the D-001 anchor needs a 0x00 terminator; the POSIX root has no upper bound; the upsert must be select-first; the carry had three more gaps.
- Round 6 (collisions and sources): the audit never wrote the N-030 rule; `Resolver::name` must not become `key()`; the Phase 6 exemption machinery was dead code; the env scanner flags 12 sites until they migrate; only specta's `collect` feature emits unsafe; catalog needs 12 to 16 variants.
- Round 7: N-034 reverses a deliberate grain rule; N-016 has no rule in the audit and only 2 real violators; `keep.ps1` is not a port and must be kept; AD-015's gate was wrong; PF-024 has no manifest step; the Pin kit exists here; Phase 2's file list was over-wide; the document itself had damage from patching (rewritten as rev 8).
- Round 8 (read-only verification, coverage rebuild, pre-mortem): no backup step for the owner's real 515 MB catalog and a silent empty-catalog path when a carry fails; the Linux CI job would go red under Phases 6 to 8 without `just check-linux`; E-011 and C-001 were uncovered; `check: _qt`, the 20 and 18 crate lists, the audit line numbers and every cited tool verified; PF-024's manifests may not exist on this machine.
- Round 9 (N-039 and Phase 7 dry runs, cold read): N-039 needs no new existence test (the parity test already proves it) and must not touch stored notice codes; exiftool and writeback must convert in one step (writeback downcasts exiftool's `io::Error`); thumbcache's `damage_in` and edit's foreign errors need `source()` chains preserved; the `work.rs:302` exception cite in the audit does not exist; leftover contradictions from patching were removed.

- Round 10 (conventions, Phase 10 dry runs, fidelity to the findings): the repo's lint rules (`unwrap_used`, `as_conversions`, `indexing_slicing` denied) shape the new code (poisoning idiom, no casts); `unsafe_code` becomes `deny`, which is CU-005's literal ask and makes decision 1 a reversal; AD-011's examples include field visibility; C-009 lists five divergent readers; AD-002, A-023, A-031, U-035, D-061 and U-029 each pointed at the wrong files or were partly stale; A-017 needs F-48's criterion first; the GPU track is mostly blocked here by missing `PG_HOME`/libjpeg-turbo.

## Decisions taken by default (the owner may override any)

1. `unsafe_code = "deny"` at the workspace level (CU-005's literal ask: `forbid` where there is no unsafe, `deny` with a per-item `#[expect]` elsewhere, examples and tests included). This reverses an earlier default of `warn`: `deny` is workable because `expect` works under it, and it is the "most proper" form.
2. `NeighbourPreload` is not created (D-063): it would share about two declarations.
3. Every `text.ftl` id gets a `msg-` prefix (N-039), because `ui-` and `cli-` already carry a per-file prefix.
4. The alias const `pg_render::THUMBS_VERSION` and its sibling `THUMB_OUTPUT_VERSION` are removed (T-018).
5. Eleven `app` modules (`app, catalogs, config, edit, error, images, inbox, library, prefs, reader, views`), four `dbfile` modules and `dedup::union_find` become private (AD-011); `app::yandex` and `app::reexport` stay public.
6. The retained originals (`doctor.ps1`, `qmltest.ps1`, `build-jpeg-turbo.ps1`, `build-static-ffmpeg.ps1`, `win-build.cmd`, `win-env.cmd`, and `profile.ps1` if its Python port runs) are deleted after their ports are confirmed. `keep.ps1` is NOT an original (it is the producer that `keep.py` judges) and stays.
7. `is_io_read`'s behaviour is preserved (Phase 7): the typed classification maps to Read exactly where today's check does. A panic is never `Read`; a stricter rule is a separate later decision.
8. The old-layout catalog carry is built (Phase 4) although `RELEASED == []` makes that path pre-release only, because `merge_folder` is needed anyway for a key-version bump. Dropping the carry needs the owner's word; if dropped, replace the carry test with a "moved aside intact" test.
9. Empty values read as unset for every name read through `process` (not just `PG_*`), and clap's `env =` is dropped from `--help` (Phase 3). `PG_PERF_LOG`'s `-` difference stays.
10. `catalog::Error::Other` is removed (the audit's E-019/E-020 dispositions keep an anyhow exception for catalog; find its text with grep and overwrite it; the earlier cite to `work.rs:302` in the audit does not exist).
11. The thumbcache reader pool replaces the thread-local, so a reader outlives its thread and `Drop` now runs in production (Phase 5). The finding itself names only writeback's leaked static (`lib.rs:165-167`) and a test `Box::leak`, and its disposition leaves "Arc ownership" to the owner: widening to app, cli, library and the tests, and the pool, are this plan's.
12. `size_mb` keeps its name and serialised form; no serde alias is added; U-001 relabels text only.
13. The `Notice` macro's `code()` stays as the one documented exception to the key/name/code rule (N-030), because renaming it collides with `Key`/`KeyBuf::key()`.
14. `#[non_exhaustive]` is not used on the new error enums (internal workspace crates; exhaustive matches catch new variants at compile time); `CLIPPY_LEDGER.md:245` already records `exhaustive_enums` as "decided: not adopted", so the `crates/README.md` note is a pointer, not a new rule. Error naming rule (also in the README): a crate with one error enum calls it `Error` (as app and catalog do); a crate with several names each `<What>Error` (`ReadError`, `MakeError`).
15. N-034 stays as is: the audit's rule is by grain (singular names at the app layer, plural/batch names at the store layer, documented at `app/src/inbox.rs:726-728`) and the code already follows it. Its disposition is rewritten to say so; unifying the layers would erase that distinction.
16. N-016: the audit chose no style, so this plan chooses: a test name is a sentence stating what holds (as most already are); bare nouns are renamed; platform-specific tests keep their `_windows`/`_posix` suffix. Written into `crates/README.md`.

## Phase 0: baselines, runs and tool-port cleanup

Phase 0 changes no product code except the T-005 fix (if the flake is real), the new Justfile `check` recipe, the deletions below and the GPU/CUDA track.

### Backup first, then setup

Before anything else copy the owner's real catalog folder (`D:/PhotoGallery/catalog`, 515 MB) and `D:/PhotoGallery/gallery.json` to a dated folder outside the repo, and repeat that before the first manual run after Phases 4, 5 and 7. Then, in this order:

1. `just lint-tools-setup` (installs into `.venv` and `tools/lint/node_modules`; needs network and npm; cargo-deny, cargo-machete, just and cargo-modules must be at their pins).
2. `rustup target add wasm32-unknown-unknown` (checked by `tools/lint.py:24` for `pg-model` and `pg-client`).
3. `just setup` (FFmpeg headers for `video/build.rs:98`; `.deps/ffmpeg` already exists here, so it is a no-op on this machine).
4. Add the `check: _qt` recipe, then `just build debug`.
5. Record green G-check, G-lint, G-full and G-qt.

### Recipe notes

- `just shots` writes `target/shots/*`; only `--record` rewrites the tracked `tools/shots/reference/<sha10>` folders. A fixture-hash mismatch makes compare ask for a re-record: never run `--record` without the owner.
- `just modgraph` runs `cargo install --locked cargo-modules --version 0.28.0` every time (network, writes `~/.cargo/bin`) and takes minutes.
- `just deny` needs the network for the advisory DB; it writes outside the repo.
- `just qmltest` needs CMake and the MSVC environment and writes `target/qmltest`.
- `just doctor` and `just test-tools` write nothing and need no network. No Docker or GPU is needed for any Phase 0 recipe; `just check-linux` (Docker Desktop) is the exception, required at the end of Phases 6, 7 and 8.

### Runs

- `just doctor`, `just qmltest`, `just shots`, `just deny` (closes ML-018's licence check), `just modgraph` (A-028 gate: "yes, `core` reaches `roots`" means A-028 moves the types; "no" closes A-028 as Moot).
- DOC-036: grep the generated `bindings.rs` under `target\` for `AV_NOPTS_VALUE` (`just build debug` already builds `pg-video`).
- E-002's `screenshot.cpp` fix is an exit code 1 for a window that never settles, which a normal `just shots` run never exercises: add a forced-timeout shot (a tiny `--wait`) or leave E-002 Fixed in part.

### T-005 (flake)

Run the real tests 20 times each (5 runs miss a 2-in-5 flake about 8% of the time): `thumbs::tests::a_file_that_doesnt_decode_is_recorded_and_one_unread_is_not` (`crates/thumbs/src/lib.rs:702`) and `a_file_that_doesnt_decode_is_recorded_until_its_borders_are_found` (`crates/library/tests/borders.rs:109`). If either fails, trace from evidence and fix it before Phase 1.

### AD-015

The finding (restored by `git show e63aa037:docs/plans/CONSISTENCY_AUDIT.md`, heading at line 2844) says the two tools' mode lists are documented differently. They are different tools by design (`eval/jpeg-decoders/harness/harness.cpp` near lines 265-279 has list, time, repeat, throughput, dump; `eval/jpeg-decoders/src/main.rs` has time, compare, repeat). Gate: each tool's usage text or README names its own modes correctly; then AD-015 is Fixed (or No change).

### PF-024

`manifest.json` is written only at the end of a full build, and `--configure-only` returns before it, so do not re-run the hours-long builds. Locate the existing manifests first (they were written 2026-11-13, `docs/reference/STATIC_BUILD.md:10-12`; `.env` sets no `PG_HOME` and `%LOCALAPPDATA%\gallery` holds only `cache`, so find the recipes' output folder in their scripts) and diff them; if none exist on this machine, PF-024 stays Deferred; pass means identical after path normalisation, else fix the port. The Qt manifest needs the full Qt build: "not verifiable here".

### Retained originals

`tools/{doctor,qmltest,build-jpeg-turbo,build-static-ffmpeg}.ps1`, `eval/jpeg-decoders/win-build.cmd` and `win-env.cmd`. Their findings (D-036, D-047/048, D-053..057, E-011, E-014, C-001, C-017, TL-012, P-011, P-014) get dispositions rewritten; E-011 and C-001 name `profile.ps1`/`qmltest.ps1` and follow the `profile.ps1` outcome below.

- "Green" means the Python port's documented dry or help step exits 0 and its output matches the original's for the same inputs. Then delete the original, one commit point per tool.
- `win-build.cmd` cannot run as it stands (`vendor/` is empty, audit line 780): confirmation is `win_build.py` only.
- Deleting the two build `.ps1` files breaks `tools/test_ci_files.py` `class Consumers` (starts at :241; the two `.ps1`-reading tests at 242-255 are the P-011/D-054 pin guard): rewrite them to check the Python recipes and keep the pin guard.
- Sweep stale references: `tools/pgtools.py:162` (`qmltest.ps1`; line 34 names `profile.ps1`), `crates/desktop/cpp/gui_app.cpp:29`, `eval/jpeg-decoders/sources.py:3,13`, `eval/jpeg-encoder/build.ps1:24`, `docs/reference/STATIC_BUILD.md:302`, and the originals paragraph of `tools/README.md:76-83` (`compare-shots.ps1` does not exist: delete that line).
- `profile.ps1` (TL-008, UI-014): the Pin kit exists at `%LOCALAPPDATA%\Programs\sde\pinkit` (`pin.exe`). Build the tool with `tools/profiling/callgraph/build.py`, run `profile.py` once; if it works, delete `profile.ps1` and close TL-008 and UI-014 (the `doctor` and `qmltest` halves of UI-014 close with their `.ps1` deletions); if not, leave them Deferred with the reason.
- Keep: `keep.ps1` and the other `tools/profiling/pairing/*.ps1` (no ports), `eval/jpeg-encoder/build.ps1` (no port).

### GPU/CUDA track (independent; each item ends Done or Deferred with a reason)

- What this machine has: nvcc 13.4 (`CUDA_PATH`), the RTX 2060, MSVC 14.44/14.51 (via vcvars), cmake, ninja, `nvjpeg.h`/`nvjpeg.lib` in CUDA 13.4, Python 3.12 with torch. What it lacks: `PG_HOME`/`PG_JPEG_EVAL` (unset, so no `tools/jpeg-eval` tree, no libjpeg-turbo build and no `turbojpeg-static.lib`), a nvimgcodec Python package, the libpng/spng sources, and network-fetched repos (`win_build.py` clones from GitHub). Check `PG_LLVM_DIR\bin\clang-cl.exe` before assuming clang-cl is absent.
- U-029 is runnable: the file is `eval/jpeg-encoder/encode_gpu.cu` (not under `jpeg-decoders/harness`); part of the finding is stale (`EGPU_MAX_DIM` is already 65535 in `encode_gpu.h:23` and `k_gather64` is already 64-bit); what remains is the `k_dct<<<(unsigned)nb, 64>>>` cast at line 722 and `grid_colour` clamping separately from `grid_exact` (about 10 lines). Verify with `eval/jpeg-encoder/build.ps1`, which needs `PG_HOME` or `-JpegSrc`/`-JpegBuild` (an older `build/test_encoder.exe` exists).
- E-063: the only two macros are `CU()` in `dec_gpu.cpp:16` and `CUDA_TRY` in `encode_gpu.cu:44`: unify the format; `encode_gpu.cu` builds with nvcc and MSVC alone, `dec_gpu.cpp` needs the harness tree and clang-cl.
- E-002 `colour_check.cu` (`eval/jpeg-decoders/harness/colour_check.cu`; the exit-1 `kMaxDiff` code already exists) needs libjpeg-turbo: Deferred unless the owner sets `PG_HOME` and fetches it. U-030 is `eval/png-decoders/harness/alloc_count.cpp` (not CUDA; needs libpng/spng sources): Deferred. E-017 needs a nvimgcodec run (`nvidia-nvimgcodec-cu13==0.10.0.9`, network): Deferred.
- P-004 asks for the harness pin to move to the fork's parajpeg at `8e2c53a` (it pins `f45081d` in `linux_build.py:158`, `win_build.py:96`, `win-build.cmd:50`) and a re-measure: the pin is also in `JPEG_DECODERS.md:489`, and `8e2c53a` is on the `darktorres` fork while the harness clones `github.com/anniigr/parajpeg`, so the URL changes as well as the SHA. The text edit is doable here; the re-measure needs the whole `win_build` chain (clang-cl, network), so P-004 stays Deferred unless the owner provides those.
- V-030 needs both nvJPEG toolkits (only 13.4 is present): may stay partial.
- Gate: `encode_gpu.cu` builds with nvcc here and its harness runs once (U-029, E-063 for that file), and `just lint` stays green; every item needing the missing pieces above stays Deferred with that reason, unless the owner sets `PG_HOME` and fetches libjpeg-turbo first.

### External (stay Deferred)

P-023 (GitHub release per action SHA), P-024 (apt snapshot), P-027 (live Yandex), P-010/CU-024 (rustup-init SHA-256, also `docker/Dockerfile:81` and `tools/profiling/ci_setup.py:51`), TL-007 (needs a CI run), DOC-072 (target-hardware timings).

## Phase 1: renames and rule text (compiler-guided)

### PF-026

Rename `WriteKind::Turns`/`Write::Turns` to `Rotation`.

- Target only the two enums: 38 `(WriteKind|Write)::Turns` lines (the repo has 99 `::Turns` lines, because `Turns::NONE` and similar match) plus the declarations `Turns,` (`model/src/write.rs:34`) and `Turns(Turns)` (`:92`). Files include `model/src/{write,notice}.rs:1693`, `writeback/src/{lib,file}.rs`, `catalog/src/{queue,core,writers,ledger,schema,test_support}.rs`, `catalog/src/browse/read/tests.rs`, `client/src/text.rs:534`, `client/src/text/notice.rs:179`, `library/tests/writeback.rs`.
- Rename the `write_turns!` macro to match, deliberately (it produces the stored key `rotation`): sites in `model` (3) and `catalog` (`browse/read/grid.rs:45`, `browse/read/state.rs:21`, `pair.rs:542`, `queue.rs:25,114,123,133,277,286,295`).
- No string changes: `client/src/text/notice.rs:177-180` already emits the selector `"rotation"`; `text.ftl:292,296` matches it only because `*[turns]` is the starred default arm (renaming that arm to `*[rotation]` is cosmetic and safe); `WriteKind` serialises through its manual `key()` (no strum); no `Rotation` identifier or glob import collides. The stored key `rotation`, the SQL `CHECK`, `.rotation.pg-write` and `catalog/src/queue.rs:402-403` do not change.
- Fix the stale doc at `write.rs:2`, add a serialisation test, and update the audit's disposition at `CONSISTENCY_AUDIT.md:2559` ("WriteKind::Turns is not renamed"). It lands before REFACTOR_LEDGER F-38 (strum derives for `WriteKind`), which is out of scope here.

### N-030

The audit (`CONSISTENCY_AUDIT.md:2115-2119`) says only "the naming rule (key/from_key, name, code) is decided" and names four divergent items (`Format::name`, `Ext::name`, `Op::code`, `Resolver::name`). The rule below, and the wider list, are THIS plan's extension: write the rule into `crates/README.md` FIRST.

- **Rule**: `key()`/`from_key()` = stored code, `name()` = display text, `code()` = numeric only. Fourteen types already follow it (detection.rs, media.rs `Kind`, query.rs, write.rs, `Shape`, ...).
- **Rename to `key()`/`from_key()`**: `Provider::name`/`from_name` (`desktop/src/lib.rs:196`, `client/src/image.rs:27,37`, `tests/image_laws.rs:44`); the stored-string `code()` methods of `Source`, `ScanNote`, `StartOverReason`, `EditOp`, `ViewOp`, `FilePass`, `LedgerKind` (+ `from_code`), `Op` (`notice.rs:790,815`), `FolderFailure` (`catalog/src/types.rs:173,182`); `RefusedReason::code` (`model/src/refusal.rs:61`; used by its `Display` and as an FTL selector, not stored; its output differs from its serde tag); `Format::name`/`from_name` (`model/src/format.rs:30,46`; `name` is a strum `.into()`); `Ext::name` (`media.rs:96`).
- **Not `key()`**: `Resolver::name` (`model/src/notice.rs:778`) returns a composite `String` such as `next-success(edit-rate)` (a doc and golden token; one call at `notice.rs:1303` plus `resolver_table.golden`): rename it `token()`.
- **Stay as they are**: `Level::from_code` (numeric), `desktop::picker::Purpose::code/from_code` (an `i32` bound to QML), `platform::dir::FsKind::name` (display text), and the `Notice` macro's `code()` (`notice.rs:957`; decision 13: note in the README that `Part::Code(note.key())` at `notice.rs:1199-1209` and `Key.code` then look inconsistent).
- The serde tests at `notice.rs:1964-1982` compare `serde_json::to_value(x)` with `x.code()` and so pin the rename; no `.ftl`, `.qml` or golden file calls these methods. The compiler finds the call sites.

### N-016

Decision 16 chooses the style (the audit, `CONSISTENCY_AUDIT.md:2033`, says "the test-name style is not chosen"). The only bare-noun tests today are `library/tests/video.rs:134 videos` and `thumbcache/src/lib.rs:597 eviction`; `holder` (three crates), `controller` (`client/tests/controller/panels.rs:129`), `images` (`app/src/yandex/images.rs:286`), `normalized` (`catalog/src/schema.rs:809`), `jpeg` (`container/src/lib.rs:1075`) and `disks` (`library/src/lanes.rs:226`) are helper functions, not tests: leave them. Rename the two, sweep doc cites that name tests (`docs/plans/IMPORT_FAILURES_PLAN.md:185`, `MULTI_DRIVE_LANES_PLAN.md:456`), and write the rule into `crates/README.md`.

### N-034

Decision 15: no rename. Rewrite the disposition (it is the audit's own `No change`) to state the grain rule and cite `app/src/inbox.rs:726-728`.

### N-017 (QML; own commit point; G-qt)

- Rename the 18 `*Asked` signals, their `onXAsked` handlers and `function onSimilarAsked`/`onSimilarCommandLineAsked` (`Main.qml:801,804`) to `*Requested`: about 50 lines in `crates/desktop/{qml,tests/qml}` incl. `crates/desktop/tests/qml/tst_viewer.qml:154` (re-count by grep).
- The rename is mechanical (`Hud.qml:44,101` and `RatingBar.qml:25,45` `findSimilarAsked` become `findSimilarRequested`; `similarCommandLineAsked` becomes `similarCommandLineRequested`) except Main's `similarAsked`, which becomes `similarSearchRequested` (`TestHooks.qml:48` already has an unrelated `similarRequested`).
- Leave `showAsked` (a function, `Main.qml:123,353,873`) and `*Toggled`. `crates/desktop/qml/Wheel.qml` is an unrelated singleton.

### T-018 remainder and T-027

- `render/src/lib.rs:11` is a const ALIAS (`pub const THUMBS_VERSION: i64 = pg_model::versions::THUMBS_VERSION;`); `:14` `THUMB_OUTPUT_VERSION` is too, but its users are already on `pg_model::versions::` (`thumbcache/src/lib.rs:786`, `schema.rs:156,211,216,231,236`). Switch every `THUMBS_VERSION` use to `pg_model::versions::*`, then remove both aliases.
- About 10 code uses: `library/src/{lib.rs:39,runner.rs:370}`, `library/tests/{lanes.rs:73,video.rs:241,writeback.rs:100,167,306,940,1027,1083}`. About 10 doc mentions: `catalog/src/{lifecycle.rs:420,types.rs:299,ledger.rs:80}`, `thumbcache/src/{lib.rs:91,schema.rs:380}`, `library/src/lib.rs:32`, `model/src/versions.rs:3,24,43` (line 3 says "the owning crate re-exports its constant under the same name": reword), `crates/README.md:58`, `docs/webclient/WEB_CLIENT_{RECONCILED.md:320,DESIGN.md:1343}`. Find all with `grep -rn 'pg_render::THUMBS_VERSION\|crate::THUMBS_VERSION\|THUMB_OUTPUT_VERSION'`.
- Move the assert at `render/src/lib.rs:17` to `model/src/versions.rs` (`const _: () = assert!(THUMBS_VERSION > 0)`); fix the intra-doc link at `render/src/maker.rs:54` (an unresolved link fails `cargo doc -D warnings`).
- T-027 leftovers (gate: zero hand-rolled generators remain, except the QML one): `crates/codec/src/codecs/convert.rs:274-278`, `crates/analysis/src/borders/stats.rs:90-95`, `crates/bench/benches/ir/pairing.rs:19-39` (an xorshift64*) move to `pg_testrng` via `pg-testkit` (all three crates already depend on it); `analysis/src/borders.rs:186-192` is a hash mixer, not a stream (leave); `desktop/tests/qml/tst_gridrow.qml:409` is a QML LCG that cannot use Rust (documented exception).

### N-010 / stored-turns naming

The finding is already Fixed in part (`Placeholder.turns` became `shown_turns`; `FileState.turns` no longer exists, `state.rs:21` is `pending_turns`). What remains is the Rust fields named `rotation`: `FileState.rotation` (`model/src/state.rs:16`) and the same-named fields on `Entry`, `Version` and `thumbcache::types::Source` (`types.rs:19`); `rotation` is a Rust identifier in about 36 files. Rename those to `turns` and write the rule in the README (`turns` for the count and the `Turns` type; `rotation` only as the write key and legacy stored names). The SQL strings stay: thumbcache `entries.rotation` (`schema.rs:25`, `reader.rs:177,206`, `writer.rs:408`), catalog `analysis.rotation` and `placeholders.rotation` (`queue.rs:402-403`). No schema change and no `THUMBS_VERSION` bump. N-010's disposition ties to PF-026: rewrite both in the same commit point.

Gate: G-check per batch, G-full at phase end, G-qt for N-017.

## Phase 2: U-004 client turns end to end

- **Today's chain**: QML `int` -> `shell_bridge.rs:181` `i32` -> `shell.rs:251` `Event::Turn{turns: i32}` -> `controller.rs:163` `Command::Turn{turns: i32}` (passed through at :310, built as ±1 at :388) -> `shell.rs:485` -> `edits.rs:23` `rotate(i32)` -> `Turns::of(i64::from(turns))` at `edits.rs:25`. Normalisation to 0..3 happens only there; `Library::turn` already takes `Turns`.
- **Change**: `Command::Turn`/`Event::Turn` (`crates/client/src/controller.rs:113-116,163,310,388`) carry `Turns`, built with `Turns::of(1)`/`Turns::of(-1)` (`-1` becomes 3, identical to today's normalisation). Convert once at the QML boundary: `desktop/src/shell_bridge.rs:181`, `shell.rs:251-253,485-487`, `desktop/src/gallery/edits.rs:23-26` (keep the early return on `Turns::NONE` at :26, so `turn(0)` and `turn(4)` stay no-ops; the QML only sends ±1, `Viewer.qml:304,307`), and `client/tests/controller_laws.rs:160`. The outward properties (`gallery_bridge.rs:70,81`, `viewport_bridge.rs:84`, `gallery/{items,state,model}.rs`) are already derived from `Turns`: leave them alone. QML stays `int`.
- **Serde**: `Turns` (`model/src/turns.rs:5`) has none; `model` already depends on serde (`Rating` uses `#[serde(try_from = "u8", into = "u8")]`, `rating.rs:7-8`). Add `#[serde(from = "i64", into = "u8")]` with `From<i64>` through `Turns::of` and a `From<Turns> for u8` written without `as`, so output is a bare integer and an old recorded `-1` loads as 3 (wasm builds of `pg-model`/`pg-client` are unaffected: serde is already there).
- **Replay**: `Event` and `Command` derive Serialize/Deserialize (`controller.rs:37,143`) and `replay.rs:16-23` records `Update{event, commands}`, so a desktop session does record Turn events; the format check (`controller/replay.rs:47-62`) is unchanged because the JSON shape is. `client/tests/replay/session.jsonl` has no Turn events. Add a proptest strategy that generates `Event::Turn` with signed values (none exists today) and a signed round-trip test.

Gate: G-full + G-qt.

## Phase 3: C-006 typed PG_* env module (+ C-009)

### Inventory

About 25 production read sites (49 more sites are in tests and examples); 12 of them lie outside the allowlist and must migrate: `app/src/{app.rs:86,config.rs:26}`, `cli/src/{args.rs:30,lib.rs:112}`, `desktop/src/{command_line.rs:199,controller.rs:61,lib.rs:116,159,picker.rs:53,trace.rs:31}`, `exiftool/src/lib.rs:28`, `gallery/src/main.rs:26` (`PG_DHAT_UNTRIMMED`, a runtime read: register it). The injection seam already exists: `Layout::resolve(var: impl Fn(&str) -> Option<OsString>, ..)` (`library/src/layout.rs:29`), `dirs::default_home/user_folder/folder_caution`.

### Design

- Extend `crates/platform/src/env.rs` into a typed layer over that seam. An `Env` type cannot implement `Fn` (it needs unstable `fn_traits`), so the process reader is a plain `pub fn process(name: &str) -> Option<OsString>`: a fn item satisfies every existing `impl Fn(&str) -> Option<OsString>` parameter (`layout.rs:29`, `platform/dir.rs:100,127`, `dirs.rs:11,17,22,28`, `picker.rs:44`) and every test closure.
- The layer adds `enum Var`, a `VarSpec` table in `crates/platform/src/env.rs` (name, kind, doc, consumer: runtime/compile/C++/test) as the single source for docs and checks, and typed readers that take the lookup so tests can inject: `path(var, Var)` and `flag_in(var, Var) -> Parsed { Unset, On, Off, Invalid(String) }`.
- Platform is the right layer: no crate below it reads env in production (`codec/decode.rs:440` and `dedup/join.rs:2911-2933` read env only in `#[ignore]` tests and cannot import it, so the scanner exempts tests).

### Behaviour changes (record each in the `VarSpec` table)

- `process` filters empty values for EVERY name it reads, including `HOME`, `USERPROFILE`, `OneDrive*` and `LOCALAPPDATA` in `sync_folder`/`home_for`, not just `PG_*` (clap's `env=` and `var_os` differ on `""`). `PG_DHAT_UNTRIMMED`, `QT_AUDIO_BACKEND`, `PG_RECORD` and `PG_DEV` treat an empty value as set today.
- Route `env::flag` (`env.rs:7`, which reads `std::env::var` directly) through `process`, so there is one reader.
- `PG_PERF_LOG`: leave the `-` difference; `desktop/src/trace.rs:29-31` documents it as deliberate.

### Unify

- Drop clap's only `env =` (`cli/src/args.rs:17`; changes `--help` text, no test pins it) so `Layout::resolve` is the sole reader.
- One `PG_THUMB_CAP_GIB` parser (desktop `lib.rs:159` and the CLI `cap_gib`, `args.rs:196`; both end in `Cap::from_gib`).
- One OneDrive/USERPROFILE/HOME helper (`platform/src/dir.rs:127-133` `sync_folder` against `platform/src/dirs.rs`; `desktop/src/picker.rs:53` only forwards `var_os`).
- Resolve `PG_EXIFTOOL` once into the ExifTool start options. That injection point removes the `set_var` in `library/tests/crop_tags.rs:53,83` and is a Phase 3 deliverable that Phase 8 relies on.

### C-009

When a flag variable holds an unrecognised value, warn once: `pg_platform::env` cannot log, so the reader returns `Parsed::Invalid` and the caller logs through `pg_log`. The finding lists five divergent readers, all of which join the `VarSpec` table with an explicit spelling set: `PG_TRACE` (only `1` today), `PG_DEV` (1/0/true/false), `PG_SHARED_DEVICE` (only `0`), `PG_FORCE_AV1` (read as an int), `PG_REQUIRE_TOOLS` (any value means on, `exiftool/src/lib.rs:295` and `platform/src/testing.rs:77`; CI sets `PG_REQUIRE_TOOLS=1`). One spelling set (the one `parse` accepts); the Rust flags go through `flag_in`, and the C++ `env_flag` (`desktop/cpp/gpu_device.cpp:44`), which keeps its own parser (changing that needs a proven cxx bridge call available before Qt starts), is extended to the same set for `PG_SHARED_DEVICE` and `PG_FORCE_AV1`; test its spellings against `parse`. The `build.rs` reads (`desktop/build.rs`, `video/build.rs`) stay on the allowlist as a documented exception to "a single module": say so in C-006's disposition.

### Registry check and docs

- `tools/test_ci_files.py` has no Rust/C++ scanner: write one (Rust and C++ sources). The "PG_* token is registered or in `.env.example`" test ignores: child-process handshake keys (`PG_HOLDER_*`, `PG_THUMB_*`, `PG_TESTING_HELD`, `PG_PLATFORM_*`), probe/dump keys (`PG_PROBE_*`, `PG_JOIN_*`, `PG_LABEL`, `PG_QML_RESOURCES`), test inputs (`PG_TEST_CATALOG`, `PG_REHEARSAL_CATALOG`, `PG_JPEG_COMPARE_DIR`), history docs (`PG_THUMB_CAP_GB` appears only in the audit and `NOTIFICATIONS_DESIGN.md:327`) and `PG_FFMPEG_BIN` (build-script-emitted: `video/build.rs:87` sets it via `cargo:rustc-env`, `video/src/ffi/load.rs:171` reads it with `option_env!`). `PG_BENCH_DATA` (compile time) against `PG_BENCH_DATA_SOURCE` (Justfile/.env) is not a conflict.
- Add to `.env.example`/docs: `PG_DEV`, `PG_DHAT_UNTRIMMED`, `PG_TRACE`, `PG_RECORD`, `PG_*_CONTENT_KEY`, `PG_BENCH_DATA`, and the two C++-only keys `PG_FORCE_AV1` and `PG_SHARED_DEVICE` (`gpu_device.cpp:52,80`; they need the C++ scan, which treats Qt/OS-owned names as upstream and does not register them: `QT_D3D_ADAPTER_INDEX` at `gpu_device.cpp:83`, `QT_QPA_PLATFORM`, `QT_QPA_FONTDIR`, `WINDIR` at `gui_app.cpp:25-27`).
- Wire both checks into `just lint`/`just test-tools`.

### Enforcement (the LAST step of Phase 3, after every site above is migrated)

- `tools/boundaries.py` `production_files()` already scans `crates/*/src/**` plus `build.rs` with `#[cfg(test)]` stripped. The rule's regex is `\benv::(var|var_os|vars|vars_os|set_var|remove_var)` only (production code legitimately uses `args_os`, `current_dir`, `current_exe`, `temp_dir`; `option_env!`/`env!` are compile-time; the `\b` keeps `my_env::var` out). The typed layer's readers are named `process`, `path`, `flag_in`, never `var_os`/`vars`, so the rule does not match them.
- The scanner does not strip comments: reword the doc comments that contain `std::env::var_os` (`platform/src/dirs.rs:4`, `picker.rs:43`) or strip `//` first. It misses `std::env\n::var`, `use std::env as e` and `use std::env::{var}` plus a bare call; none occurs today.
- Allowlist: `desktop/build.rs:121,130`, `video/build.rs`, `gallery/build.rs:4`, `platform/src/env.rs`, `desktop/src/lib.rs:118` (`set_var`, one documented process-start call) and `platform/src/testing.rs` (`PG_TEST_DIR`; that module is `cfg(any(test, feature = "testing"))`, which `CFG_TEST` at `boundaries.py:50` does not strip, so it is scanned).

Gate: G-full.

## Phase 4: D-001 one folder, one row (edits `baseline.sql`)

### Meaning

D-001 delivers one row per case-folded path spelling. It does not unify a re-lettered drive or a verbatim `\\?\Volume{GUID}` spelling with a `D:` spelling: `rules.rs:89-90` parses those as different anchors, files are identified by `(volume, file_ref)`, and today's `UNIQUE(path)` behaves the same. Keying on volume identity would be a separate schema decision; say so in the disposition.

- The fold is the SAME `Name::push_lower` that `is_under`/`same_part`/`same_anchor` use, so the key and its oracle agree. Rust lowercase differs from NTFS's `$UpCase` table (σ/ς, ſ, İ, Kelvin sign): add tests documenting those residual cases and append a new finding for the NTFS-fidelity gap to `CONSISTENCY_AUDIT.md` (next free id).
- Also outside the key, stated as known gaps: NTFS trailing dots and spaces, 8.3 short names, junctions. `..` is kept as a literal part (as `is_under` does, so the oracle agrees); empty/`.` parts and mixed or trailing separators are already dropped by `parse_bytes` (`rules.rs:104-128`).

### Keys

- `crates/model/src/rules.rs`: add `PathRules::path_key(&Path)` built from PARSED parts (never raw bytes). Per part: `folded part 0x01` with 0x01/0x02 escaped, so `a/b` is not a prefix of `a/bc`. POSIX uses `escaped(exact part) 0x01` (no lowered half: POSIX `is_under` is exact). The `lower 0x00 exact 0x01` shape of `push_folder_key` (`rules.rs:237-244`) is for `match_key` only.
- The anchor term comes FIRST and is terminated by 0x00: `lower(anchor) 0x00`, where the anchor is a drive (`C:`), the UNC/verbatim anchor (`\`) or empty (a name never holds NUL, `rules.rs:101` rejects it). Without that terminator the drive path `C:\x` and the relative path `\c:x` (empty anchor, single part `c:x`) produce the same bytes, and the drive root `C:` would claim every relative part starting with `c:`: add a test for exactly that case.
- There is no extra terminator after the last part: a folder's whole key is deliberately a prefix of its descendants' keys. Write one worked example per OS rule (Windows `C:\Photos\Raw`, POSIX `/photos/raw`) into the doc comment and pin it in a test.
- Range: export `prefix_upper_bound` (private at `rules.rs:325`). A subtree is `path_key >= K AND path_key < upper(K)`; the `OR path_key = K` branch is redundant. For an escaped key ending in 0x01 the bound `P 0x02` is exact (no non-descendant falls inside; `rules.rs:532` already tests the 0x01/0x02 siblings). The POSIX root `/` has an empty key and `prefix_upper_bound` returns `None` there (`rules.rs:513`): bind `(?2 IS NULL OR path_key < ?2)` (a bare `path_key < NULL` matches nothing) and test the scan-everything case.
- Lint-safe code: `prefix_upper_bound` and the escaping use `last_mut`/`pop`/`get` and `u8::from`/`try_from`, never `as`, `[i]` or `&s[..]` (`as_conversions`, `indexing_slicing` and `string_slice` apply to non-test code).
- Bump `KEY_ALGO_VERSION` (`model/src/versions.rs:51`, doc at 48). `ORDER BY path_key` keeps "a folder before the folders under it" (`removal.rs:158-159`, `ledger.rs:307`).

### The range equals the oracle

The range query is DEFINED to equal `removal.rs:344-356` `folder_ids_under` (a full scan with `rules.is_under`, already case-folded). Today's `types.rs:90` `folders_under` is exact-bytes, so this deliberately changes it to the folded semantics. `folder_ids_under` stays as the test oracle only.

### Bounds are two parameters, not three

`under_root!` (`sql.rs:59-62`) takes `?1 ?2 ?3`; its callers splice it with hard-coded tails and `params![under[0], under[1], under[2]]`. A count mismatch fails only at run time and `cargo check` cannot catch it: land all sites, the `baseline.sql` edit, `merge_folder` and the carry in one batch, then run `cargo test -p pg-catalog`. After the change `under_params` returns `[K, upper(K)]`: every site drops its third binding and the `OR path_key = K` branch is deleted; do not renumber by hand. Find every site with `grep -rn 'under_params\|under\['` (about 12 sites in 5 files):

- `types.rs:69` (`-> [Vec<u8>; 3]` becomes `[Vec<u8>; 2]`), `types.rs:90,92`.
- `pair.rs:93,169` (the only `?4..?10` tail), `:180-182`, `:320-324`, `:385`, `:441`, `:448` (`?4,?5`), `:455`.
- `lookup.rs:150,153`.
- `writers.rs:97-98,144,149-153` (inline `path = ?1 OR (...)`, bounds `?3..?5` become `?3,?4`), `writers.rs:252`.
- `roots.rs:405-420` (inline; the `[at, lo, hi]` destructure at :420 and `Vec<[Vec<u8>; 3]>` at :413,416; `path != *at` uses the exact element, so `K` serves as `at`).

### Schema

- Rename the drive-less `folders.key` to `match_key` (users: `lookup.rs:129`, `lifecycle.rs:222,238,472,548,586`, index `folders_key` `baseline.sql:239`, `stmt_journal_probe.rs:45-47,335`, `fixture/state.rs:23`, `schema.rs:527,556-559,758`, `versions.rs:48` doc; `browse/read/scope.rs` does NOT use it).
- Add `folders.path_key BLOB NOT NULL UNIQUE`; drop `UNIQUE(path)` on `folders` (`path` becomes the display spelling).
- `folder_failures` (`baseline.sql:162-171`, `UNIQUE(path)` at :165, `ON CONFLICT(path)` in `types.rs:192-194` `FOLDER_FAILED`): add `path_key` as the unique/conflict key (a 7th bound parameter; both callers change: `writers.rs:195`, `roots.rs:240`).
- `roots`, `root_removals` (`baseline.sql:199`, `INSERT OR IGNORE` `removal.rs:171`, `WHERE path` 312,336,388,420), `scans.root` and `scan_leases.root` stay exact-bytes (they are built by the rules join): state this in the doc comment. The view `thumb_sources` (`baseline.sql:260`) reads `folders.path`: unchanged.
- Other queries that change: the lookup `SELECT id FROM folders WHERE path = ?1` (`writers.rs:146-147`), the `ScanWriter.folder_ids` cache (key it by `path_key`), `roots.rs:181,313` (`sweep_folder_failures` compares raw bytes today), `Catalog::folder_refused` (`roots.rs:236-245`), `removal.rs:110,158-159,201,218,241,292`.

### Fill and upsert

- `ScanWriter::folder` today is SELECT then INSERT (`writers.rs:213-215`): keep that shape. SELECT by `path_key` first; on a miss `INSERT ... RETURNING id`; on a hit with a different spelling `UPDATE folders SET path = ?`; on a hit with the same spelling write nothing (an unconditional `ON CONFLICT DO UPDATE` would rewrite every folder row on every rescan). It runs once per folder per scan because `folder_ids` caches.
- It still needs `match_key` for `Folder { key }` (`writers.rs:208,219`), so it computes both keys; `fixture/state.rs` `add_folder` computes both too. An unparseable stored path keeps a fallback key as `recompute_keys` does (`lifecycle.rs:227-236`) so `NOT NULL UNIQUE` cannot collide.

### Existing catalogs

A drifted catalog with a `key_version` mark goes to `Action::RebuildCarry` (`classify.rs:75`); `copy_tables` copies by common column names, so the new NOT NULL column fails and the old file is moved aside (a surfaced notice, `rebuild.rs:79`, pinned by `lifecycle.rs:771-789`). Implement the carry (decision 8):

- **Copy mechanics.** `copy_tables` runs with `foreign_keys = ON` (`schema.rs:123`, `rebuild.rs:111`) and copies `folders` before `files` (`rebuild.rs:144-146`). So copy `folders` specially in Rust (read the old `path` with `CAST(path AS BLOB)`, since the old layout stores TEXT, `lifecycle.rs:668,745`; compute both keys; merge collisions) while building an old-id to kept-id map; then copy `files` with `folder_id` mapped through it. `carry_sequences` (`rebuild.rs:253`) needs no change (it takes the max).
- **`folder_failures`** needs a special copy, not just a merge: `path_key` is NOT NULL and absent from the old file; when two rows merge, the kept row's `root` wins.
- **`meta.key_version`**: `copy_tables` writes the old `meta` values over the seeded ones except `path_rules` and `vacuum_owed` (`rebuild.rs:~137`), so a carried catalog would hold new-layout keys with the OLD `key_version` and the next open would run `recompute_keys` again: exclude `key_version` from the meta copy (or make `recompute_keys` idempotent).
- **`merge_folder(keep, drop)`** serves both the carry and a `key_version` bump (`recompute_keys`, `lifecycle.rs:219-250`), in this order: re-point `files.folder_id`, merge `folder_failures` by `path_key`, update `match_key`/`order_key` per surviving id, then delete only the emptied duplicate folder row.
- **Duplicate files.** A merge can create duplicate `(folder_id, name)` files (no such UNIQUE exists; `UNIQUE(volume, file_ref)` at `baseline.sql:208` rules out same-identity duplicates; NULL-identity displaced rows exist, `writers.rs:160`). Never delete files, ratings, analysis or `file_writes` (they cascade); keep both rows and report the collisions in the ledger/notice.
- **Stored paths not re-keyed** (state in the doc comment): `file_writes.temp` (a queued temp file path), the `RulePath` values inside `notices.data` JSON (`notice.rs:467-478`), and the `thumb_sources` view's `folder` column (the survivor's spelling is used for a dropped folder's files, harmless on Windows); `views.folder`/`session.folder` use the drive-free `join_merged` and are untouched.
- **Real-catalog safety.** The first post-Phase-4 open of a catalog rebuilds it: `core.rs:197-198` calls `rebuild`; the automatic `back_up` (`core.rs:221`) runs only for `Action::Upgrade`, never for a carry or a key-version bump; the old file is moved aside intact to `catalog.sqlite.replaced` with its sidecars (`rebuild.rs:89`); if the carry errors, `rebuild.rs:75-79` silently rebuilds an EMPTY catalog and reports `Carried::Nothing`, so the user sees an empty library while the rows sit in `.replaced`. The disk needs about 3x the catalog size (515 MB here) and this path has no `ensure_room_to_migrate` check: add one. Tests: the old file is kept intact when a carry cannot proceed, and the `Carried::Nothing` path leaves `.replaced` loadable.
- **Cost**: run the catalog bench in `crates/bench` before and after (one extra unique index and one key computation per folder).

### Tests

`removal.rs:709-730,737-762` (the case test now asserts one row), `roots.rs:497-545`, `core.rs:660-675,829-833`, `lifecycle.rs:745,1075-1099,544,575` (the 200,000-file test also asserts the folder count), `browse/read/tests.rs:495-600`, `fixture.rs:97`, layout/golden tests, plus new:

- An old-layout catalog with two case spellings carries over with roots, ratings and notices intact (extend `lifecycle.rs:771-789`).
- The range query equals the oracle on a tree with case variants.
- `a/b` vs `a/bc`, UNC, verbatim, drive-letter case, non-BMP, unpaired surrogates, σ/ς, ſ, İ, Kelvin sign.

Gate: G-full.

## Phase 5: A-032 `Arc<ThumbCache>`

### Root cause first

`crates/thumbcache/src/reader.rs:214-233` keeps reader connections in a thread-local keyed by file path; `ThumbCache::close()` (`lib.rs:373-379`) clears only the calling thread's. Replace it with a per-cache pool `Mutex<Vec<Connection>>` (`Connection` is `Send`; callers `lib.rs:165,292,298` do not nest or hold a reader across blocking calls).

- Lock poisoning: `unwrap` and `expect` are denied, so every lock and `Condvar` wait (in `with_reader`, the drop guard and `close()`) uses `.lock().unwrap_or_else(PoisonError::into_inner)` and the matching `wait_while` form, as `thumbcache/src/writer.rs:59` does.
- `with_reader` takes a connection out, RELEASES the mutex, runs `f`, then pushes it back (holding the mutex through `f` would serialise the parallel read path).
- Idle pooled connections hold no read transaction (`reader.rs:54` opens it inside `f`), so checkpoint behaviour is unchanged; connection count equals peak concurrent readers, as threads today. Two caches over one file no longer share connections (an improvement).

### `close()` must wait

- Add a `closed` flag and a checked-out counter (Condvar), decremented by a DROP GUARD (a panic inside the reader closure would otherwise leave `close()` waiting forever).
- `close()`/`Drop` order: set `closed` (a read after `close()` opens a fresh one-shot connection that is dropped on return, since the doc says "reads keep working", `lib.rs:371-373`; test it), wait for checked-out readers to return, stop the writer (so its `wal_checkpoint(TRUNCATE)` does not race a mid-read reader), then drain the pool, so the last connection closed merges the WAL (keeps `close_merges_the_wal_and_leaves_no_files`, `lib.rs:424`).
- `pending_in`/`pending_on`/`orphans_in` open their own connections outside the pool and `close()`; that is today's behaviour, keep it. `ThumbCache` stays `Sync` (the pool is behind a `Mutex`). The writer's `wal_checkpoint(TRUNCATE)` (`writer.rs:265`) may stall or fail while a pooled connection is mid-read: keep today's `busy_timeout` behaviour and test it.
- Test: read from a spawned thread, join it, drop the cache elsewhere, assert the `-wal` file is gone.

### Replace the leak

Production `app/src/app.rs:107`, `cli/src/lib.rs:635`; fields/params `app.rs:40`, `app/src/images.rs:107,121`, `app/src/library.rs:50,103,271,287`, `library/src/shared.rs:142,157`, `library/src/runner.rs:43,55`, `writeback/src/lib.rs:167,230,254`; test leaks `app/src/library.rs:487,589`, `library/src/runner.rs:371`, `library/tests/lanes.rs:78`, `library/tests/writeback.rs:96,162,301,935,1022,1078`, `writeback/src/lib.rs:814`, `cli/src/lib.rs:1023`.

### Shutdown semantics change

Leaked caches were never dropped; `Drop` now runs in production.

- `app::shutdown`'s explicit `close()` stays the authority. `Drop` runs only when the last clone goes, possibly after `forwarder.join()` (`app.rs:203-207`), so `forward_thumb_events` must hold the receiver only, and the closure given to `Service::start` must hold only `inbox`, `events` and its `conn` (`lib.rs:108-110`) so the last `Arc<ThumbCache>` is never dropped on the writer thread.
- **CLI `Write` (`cli/src/lib.rs:633-643`): call `close()` explicitly, THEN join the printer.** The printer loops until the Bus closes; the Bus's `Arc` is held by the cache writer closure (`lib.rs:108-110`) and `Service::stop` (`jobs/src/service.rs:137-141`) joins the writer, so `close()` releases it even while `Arc` clones live; joining the printer first is the hazard. Today the Write arm never joins (`_printer`): follow the `finish` closure at `cli/src/lib.rs:315-320`, fix the stale comments at `cli/src/report.rs:17-19` and `cli/src/lib.rs:632-633`, and add a CLI Write-arm test with a timeout (none exists today).

Gate: G-full.

## Phase 6: AD-011 visibility ("one public path per item")

### Rule

A module is either public API (`pub mod`, documented with `//!`) or private. Every item is reachable by exactly ONE public path. A root `pub use` may re-export only from a PRIVATE module, never from a `pub mod`. `pub mod fault` is exempt (`PUBLIC_API_EXEMPT = {"fault"}`; no root `pub use` comes from `fault`, `fixture` or `fixtures` today). `pg_app::reexport` and `pg_app::yandex` stay public with no root re-export (a documented layer boundary `desktop` names by path).

The check reports exactly 16 violations today (dry run over all 34 `lib.rs` files): app 11 (`app/src/lib.rs:31-41`), dbfile 4 (`lib.rs:14-17`), dedup 1 (`lib.rs:8`). Catalog, codec, jobs, thumbcache, yandex, log, metadata, video and writeback comply.

### Check in `tools/boundaries.py`

- `CHILD_MOD` (`boundaries.py:48`) already accepts an optional `pub` as a non-capturing group: make it capturing (`^\s*(pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*[;{]`). `check_reexports` (`boundaries.py:216`) uses `findall` and must handle the tuples.
- Add `check_public_paths(files)` (per crate, only `crates/<c>/src/lib.rs`; flag each `PUB_USE` whose first segment, after stripping `self::`, names a `pub mod` not in `PUBLIC_API_EXEMPT`) as a fifth section of `main()`. Keep `REEXPORT_BOUNDARIES`.
- No attribute parsing (if it is ever added, read the whole run of contiguous `#[...]` lines above the `mod`, because `#[doc(hidden)]` often sits between). No class token in `crate-layers.txt` (its parser treats every token as a dependency).
- Rewrite `crates/README.md` rule 2 (~line 137, which allows a `pub use` for "documented public API or a documented layer boundary") AND its repeat at `:173-174` ("a `pub use` names a child module except in `app/src/reexport.rs`"), the numbered four-rule docstring at `tools/boundaries.py:1-20` (with the sections of `main()`, which gain the new check and the Phase 3 env rule) and the `boundaries.py` row of `tools/README.md`, all in the SAME commit point as the check, so the docs and the check agree.
- New `tools/test_boundaries.py`, modelled on `test_modgraph.py` (keys must be paths under `REPO`, because `rel()` and `crate_of()` need them). Failing cases: `pub mod a; pub use a::X;`, a multi-line `pub use a::{\n B,\n C};`, `pub use self::a::*`. Passing cases: `mod a; pub use a::X;`, `pub(crate) mod a; pub use a::X;`, `pub mod fault;` alone.
- Known gaps, accepted: `pub use` from a `pub mod` in a non-root file, two mods on one line, `pub use` inside an inline `pub mod x { }` in `lib.rs`.

### Per-crate actions (re-grep `pg_<crate>::<module>::` before editing)

- dbfile: privatize its 4 `pub mod`, keep the 4 re-exports (0 external module-path users).
- dedup: privatize `union_find`; keep `pub mod join` (named externally 5 times, no root re-export).
- app: privatize all eleven modules (decision 5). `views`: keep the root re-export and ADD `query_from_json` and `sort_from_json` to the root `pub use` (`desktop/src/command_line.rs:233,236` name them by path). `inbox`: its only external mention is a doc comment (`catalog/src/inbox.rs:35`, `pg_app::inbox::LISTED`; catalog does not depend on app), so re-export `LISTED` at the root or reword the comment.
- jobs, codec, writeback, catalog already comply; client's `controller` submodules named externally stay public (`listing` 5, `replay` 5, `panels` 3, `size` 3, `start` 2, `deadlines` 1, `input` 1); scan/exiftool/crop/container/perf/query already export at the root; model/platform unchanged (about 1,440 import lines; a module-path API by design).

### Field visibility (the rest of AD-011's examples)

Three of the finding's five examples are field-visibility inconsistencies (`catalog/src/lease.rs:15` `pub(crate)` fields, `catalog/src/pair/coverage.rs:8` fully public fields, `metadata/src/xmp.rs:32` `pub` with `pub(crate)` fields). Rule (write it in `crates/README.md`): a plain data record has all fields `pub`; a type that holds an invariant has private or `pub(crate)` fields and accessors. Apply it to those three types, then grep the crates for other mixed-visibility structs. Also privatize any `client` `controller` submodule that has no external path user (the finding's `controller.rs:17-23` example), keeping the ones named externally.

### Cascade

The measured cascade is small: about 9 to 11 `pub` to `pub(crate)` edits (app `error` three fns, `inbox` five consts, dbfile `judge_conn`; the other modules have none), so the real risk is `dead_code` after privatization, not `unreachable_pub`.

The workspace sets `unreachable_pub = "warn"` (root `Cargo.toml:69-72`) and CI uses `-D warnings`, so privatizing a module makes every `pub` item that is not re-exported an error until it is `pub(crate)`, which may then raise `dead_code`. Budget a `pub` to `pub(crate)` sweep per crate in the same commit point as its privatization, and run `just modgraph` (`tools/modgraph-allow.txt` is empty; privatizing can create cycle groups).

Gate: G-check, then G-full, whole workspace including desktop, plus `just check-linux` (the `pub` to `pub(crate)` sweep can raise `dead_code` under Linux cfg).

## Phase 7: E-019 strict typed errors

### Design (to be validated by the spike below)

Evidence: the pipeline encodes a panic as `io::Error::other(Panicked)` (`jobs/src/pipeline.rs:281`) and `read_error` (:576) unwraps it; `is_io_read` (`failure.rs:15`) tests `!is_panic` first because of that; the `rebuild` flag (:332) keys on `is_panic`. A typed `is_read()` that only asks "is an `io::Error` in the chain" would classify a panic as an unreadable file, which is retried forever and never recorded. So:

- **Boundary struct.** `pg_jobs` gets `Failed { class: Panic | Read | Data | Other, text: String, source: Option<Box<dyn Error + Send + Sync>> }`, which implements `std::error::Error` with `source()` (so `is_busy`-style chain walks survive). Four classes: borders and import record every non-Read failure, but thumbs records Panic and Undecodable (`Data`) while it retries and does NOT record `Other` (`thumbs/src/lib.rs:113-130`). Panics and cancellation stop being encoded as errors.
- **Conversions.** Each library crate implements `From<MyError> for pg_jobs::Failed` (legal: `MyError` is local), setting `class` once at the crossing and rendering the chain text once. Jobs owns `From<io::Error>` (Read) and `From<anyhow::Error>` (a transitional chain walk like today's `is_io_read`, mapping a `Panicked` source to Panic). A blanket `impl<E: Error> From<E>` is impossible: it would overlap every per-crate impl and cannot choose a class.
- **`Cause`.** `Cause::of` and `is_panic` become matches on `Failed`; the `Cause` enum stays (public data inside `WriteError`; `desktop/src/library.rs:66-69` matches it); the `undecoded` string hack (`failure.rs:21`) goes.
- **Behaviour is preserved (decision 7)**: `class = Read` exactly where `is_io_read` is true today (ReadFailed, or any `io::Error` in the chain). Its only behaviour consumers are `borders/src/lib.rs:93`, `import/src/hashes.rs:112`, `thumbs/src/lib.rs:117`.
- **Rejected**: a shared `Failure`/`ErrorChain` type implementing `std::error::Error` for all crates (anyhow under another name). Libraries return their own `thiserror` enums; the template is `catalog::Error` (`error.rs:16-77`: `#[from] Io`, structured variants) minus its `Other` escape hatch. Expect one enum of 4 to 8 variants for small crates, about 11 for edit and about 13 for thumbcache (use `Damaged(&'static str)`-style families for repeated messages), plus `From` impls. Only 5 crates use `thiserror` today (app, catalog, library, render, yandex); it is already a workspace dependency.

### Pipeline mechanics

- **Inference is not free.** `E: Into<Failed>` is only a bound, so every closure whose body has only `?` and `Ok(..)` fails with "type annotations needed" (`crop/src/lib.rs:110-111`, `borders/src/lib.rs:144`, `import/src/hashes.rs:69`, `thumbs/src/lib.rs:376,396`), and each of the four stages (init, process, sink, flush; `pipeline.rs:201-203,425-429`) has its own error type.
- **So the design has named parameters.** New `run_typed`/`batched_typed` entry points take `process: Fn(..) -> Result<R, E>` with `E: Into<Failed>`, init pinned to `Result<S, Failed>`, and sink/flush receiving item results as `Result<R, Failed>` (the flush closures name that type, `borders/src/lib.rs:73`, `hashes.rs:95`). Every migrated closure gets an explicit `-> Result<_, MyError>`. They return `Result<(), Failed>`. The old anyhow signatures stay beside them until the last consumer moves, so each crate migrates once without touching jobs again.
- **`guard` stays generic on the error**: it returns `Result<T, Guarded<E>>` (`Err(E)` or `Panicked(String)`), because `writeback/src/lib.rs:453-497` calls it on a typed closure and then downcasts (`is_panic` at :459, `io::Error` kinds at :469,478, `CopyStarted` at :779).
- **Other internals that change**: the reader channel becomes `Result<Lease, Failed>` (`pipeline.rs:244,263,281`); `Outcome::SetupFailed` (:111) and the `"worker setup"`/`"read"` contexts (:364,578); `library/src/lanes.rs:158-160` (builds `Panicked` for a panicked lane) builds a `Failed` of class Panic.

### Distinctions the enums must keep (each is a downcast or chain walk today)

- `writeback/src/lib.rs:469-480` (io `ErrorKind` to offline/original_gone), `:779,898` (`CopyStarted`).
- `crop/src/lib.rs:311-313` (`ReplaceError`); `edit/src/rotate.rs:60,73,279` (`Imperfect`/`NothingLeft`).
- `catalog/src/error.rs:187-191` (`is_busy` walks the chain for `rusqlite::Error`).
- `scan/src/lib.rs:886` (`RootRemoved`; a `catalog::Error` found inside an anyhow chain: it keeps working while scan stays on anyhow and must be re-expressed when scan migrates); `app/src/inbox.rs:1276` (matches `Error::Other` and downcasts `Injected`).
- `desktop/src/lib.rs:174`, `desktop/src/providers.rs:127` (`pg_app::Error`); `render/src/maker.rs:62` (`pg_video::Unopened`); `dbfile/src/judge.rs:37` (`Damage::in_chain`); `platform/src/picker.rs:115` (`io::Error::get_ref`).

### Steps

- **7.0 Characterise first.** Add tests pinning today's behaviour before any change: per pass, retry-vs-record for a representative read error, a decode error and a PANIC (the misclassification case); every case of `thumbs::Failed::of` (`thumbs/src/lib.rs:113-130`: Panic and Undecodable recorded, Unreadable and Other retried, Other not recorded; include a truncated-file decode error with an `io::Error` at its root); an ExifTool spawn, pipe or thread failure still retries (class Read, as `is_io_read` makes it today) while its command and timeout failures do not; the user-visible error text (see "Text"); the CLI Write-arm shutdown (Phase 5). Also add `pg_platform::error::chain` here: platform has no `error` module today. These run at every later step.
- **7.0b Enforcement and allowlist.**
  - A `tools/test_ci_files.py` test using `cargo metadata --no-deps` (as `boundaries.py:58-77` already does) sees `dependencies[].kind` including dev and build, `anyhow.workspace = true` and `[target.'cfg(..)'.dependencies]`.
  - Permanent allowance: cli, desktop, gallery (gallery has no anyhow today). Temporary allowance: a new flat file `tools/anyhow-allow.txt` (one crate per line; the existing `tools/modgraph-allow.txt` is empty and has a different `source: members` format, so the test needs its own loader), seeded from `cargo metadata` with the 18 library crates that have anyhow today (analysis, app, borders, catalog, codec, crop, edit, exiftool, import, jobs, library, render, scan, testkit, thumbcache, thumbs, video, writeback). The test passes on day one; each crate step removes its line; the file ends empty.
  - Tests and examples that use anyhow through the normal dependency (about 36 lines: library/tests 22, analysis/examples 2, catalog/examples 1, import/examples 7, library/examples 4) switch to typed errors or `Box<dyn Error>`. `pg_app::reexport::Cause` (`app/src/reexport.rs:15`, used by desktop) changes with `Cause::of`.
  - Write the strict rule into REFACTOR_LEDGER F-14 and mark it decided (the finding's literal ask).
- **7.1 Spike** (jobs and its tests plus one synthetic consumer; one commit point). Implement `Failed`, the compat impls and the `*_typed` entry points with the old API intact; validate against the 7.0 tests. Pass: all 7.0 tests green, including the panic case and every `thumbs::Failed::of` case, with the old and new entry points compiling side by side. If `Into<Failed>` plus explicit closure annotations proves too noisy at the eight call sites, fall back to pinning process to `Result<R, Failed>` too (each crate converts with `.map_err(Failed::from)` at the closure's end). The seven `Pass::run` impls (`library/src/passes.rs:56,134,156,187,222,253,304`) share one lane closure (`library/src/lanes.rs:138-148`, `lane.rs:81`), so the library lane stays on the anyhow conversion until the library step.
- **Per-crate steps, in layer order** (checked against `tools/crate-layers.txt`; `dbfile` has no anyhow): testkit first (its helpers return `anyhow::Result`, `testkit/src/lib.rs:84,100,109,148,203`, and most crates dev-depend on it), codec, edit, video, catalog (together with the `app` edits at `app/src/inbox.rs:1226` and `app/src/library.rs:399`, which build or map `Error::Other`, so the workspace compiles at every step), analysis, render, scan, thumbcache, borders, import, thumbs, exiftool together with writeback (ONE step: `writeback/src/lib.rs:469,478` downcast exiftool's bare `io::Error` to choose offline/original_gone, which anyhow sees through `.context` layers but a typed wrapper hides), crop (after writeback), library, app. Per step: G-check (one batch); clippy plus tests at layer-group end (catalog group, passes group, runner group).
- **Final step**: remove the old anyhow API, `Panicked` and `is_io_read` from jobs; jobs (and `jobs-testkit`, which layers on `jobs model`) converts its own remaining anyhow uses then and its line leaves the allowlist last.
- **Sites to migrate**: pipeline call sites `run` at `cli/src/borders.rs:79`, `crop/src/lib.rs:102,237`; `batched` at `borders/src/lib.rs:133`, `import/src/lib.rs:96`, `import/src/hashes.rs:54`, `thumbs/src/lib.rs:361`, `thumbs/src/placeholders.rs:94`; jobs' own tests (`pipeline.rs:1493-1623`, `panics.rs:79-80`); the direct `guard` caller `writeback/src/lib.rs:453`; the three `is_io_read` branches (`borders:93`, `hashes.rs:112`, `thumbs:117`); the four `Cause::of` callers (`thumbs/src/placeholders.rs:205`, `import/src/lib.rs:180`, `import/src/hashes.rs:118`, `crop` via `failure.rs:44`). The library lane (`run_each`, `lanes.rs:134-165`, with `lane.rs:81` formatting `{e:#}`) sits above the pipeline.
- **Lockfiles and machete, every step**: add `thiserror` to the crate and remove `anyhow` from its manifest in the SAME commit point (`cargo-machete` runs on `crates` and flags unused deps); run `cargo update -w`; for the crates `fuzz/` depends on (codec and edit have anyhow in `fuzz/Cargo.lock:362,388`) regenerate with `cargo metadata --manifest-path fuzz/Cargo.toml` and verify with `--locked`; commit both lockfiles. `cargo deny` sees no new crate.

### Scope (approximate; re-count by grep)

The macro column counts `src` only, the `.context` column counts all targets, the table omits borders 0/2 and render 0/3, and cli and desktop stay on anyhow: about 107 `anyhow!/bail!/ensure!` and about 213 `.context/.with_context` (catalog 18/41, codec 5/34, edit 20/24, video 19/6, thumbcache 16/6, jobs 8/3, crop 5/15, exiftool 5/6, testkit 3/9, thumbs 3/7, writeback 3/14, scan 2/3, import 0/17, library 0/13, analysis 0/10) and 106 `:#}` sites across `crates/`. `thumbcache` has no error type and gets one (22 uses).

### Catalog step in detail (the widest single step)

- `catalog::Error::Other` is `#[from] anyhow::Error` (`error.rs:77`), so every `?` on an anyhow result in a catalog function converts into it. It becomes named variants, overriding the anyhow exception recorded for catalog in the E-019/E-020 dispositions (`CONSISTENCY_AUDIT.md:1139-1145`; grep for the exact text, which names the `file {raw} has malformed stored hashes` message in `work.rs`, and say so in the disposition).
- `Other` is built or matched at `browse/read/{tree.rs:36,listing.rs:44-45}`, `crates/catalog/src/work.rs:302`, `writers.rs:836,848`, `error.rs:175` (the blanket `From<rusqlite::Error>`, the biggest source of `Other` values) and outside catalog at `app/inbox.rs:1226,1276` and `app/library.rs:399`.
- Anyhow enters through 14 `with_context` sites in `core.rs:136-294,406`, plus `schema.rs:133-208`, `rebuild.rs:42-189`, `lifecycle.rs:55-80`, `serde_json` conversions at `grid.rs:116`, `inbox.rs:377`, `pair.rs:272`, `writers.rs:875,917`, and 6-7 distinct "stored folder path has a NUL" style messages in `lookup.rs`, `ledger.rs`, `types.rs`, `roots.rs`, `tree.rs`, `details.rs`.
- Expect 12 to 16 named variants, one per context family (open/reopen; upgrade and the version marks; taken zone and build mark; lock file and open marker; backup, move-aside and rebuild; schema baseline and migration step; stored-NUL, hashes and failed-folder reason; json encode), each with a `#[source]`, plus `Sqlite(rusqlite::Error)` built in the `From<rusqlite::Error>` impl. `is_busy` (`error.rs:187-191`) then matches `Sqlite` directly and recurses through the other variants' `source()`; rewrite its test (`error.rs:225-241`, which wraps with `.context`).
- Public signatures that change: `fixture::insert`/`set_state` (`fixture.rs:93,135`, public under the `fixture` feature), `fixture::folder_row`, `rebuild::db_file`, `fixture/state.rs:8`, and `examples/stmt_journal_probe.rs` (20 anyhow uses). No downstream matcher is exhaustive (`scan/lib.rs:841`, `app/reader.rs:232`, `app/error.rs:44`, `library/folders.rs:77,84`, `cli/lib.rs:349`, `app/inbox.rs:663,976,1275`), so new variants break nothing there.

### First crates in detail (dry run)

- **exiftool** (1 file, about 70 lines, 9 variants): `Start` (two message texts at `lib.rs:61` and `:201`), `MissingPipe`, `Thread`, `Pipe` (bare `io::Error` at :110-111), `RatingFailed`, `CopyFailed`, `OrientationFailed`, `Timeout`, `Exited`. `From<pg_exiftool::Error> for Failed` (the enum is `Error`, per decision 14's naming rule): Start, Pipe, Thread map to Read; command failures and Timeout to Other. Callers: `crop/src/lib.rs:338,386` (`?` into anyhow), `writeback/src/lib.rs:592,613`. It converts with writeback (see the order).
- **edit** (about 12 files, 150-200 lines, about 11 variants): `Imperfect(String)` and `NothingLeft` stop being `.is::<T>()` checks and are matched directly (`rotate.rs:60,73,279`, `gif.rs:219`; keep the `t.perfect` guard at `jpeg.rs:54`); the foreign errors (png, image_webp, weezl, turbojpeg, `pg_codec`) keep `source()` (`#[source]`/`transparent`, no flattening), or a truncated PNG whose root is an `io::Error` flips from Read to Data; about 10 `Option::context` sites in `gif.rs` become explicit `ok_or(..)`; the `{:#}` use is only in tests (`rewrite.rs:44,56`) and edit does not depend on `pg-platform` (layer-legal to add, or keep `chain` out of non-test code); `fuzz/Cargo.lock` regenerates.
- **testkit** (1 file, about 50 lines, 5 variants: run-folder, make-folder, gif-frames, bad-pixels, encode/apng with a `&'static str` step): 10 public functions return `anyhow::Result` and about 60 dependents `.unwrap()`, so no call site changes as long as the enum is `Error + Send + Sync + 'static`; pin the message order because `.context` text moves into the Display.
- **thumbcache** (6 files plus call sites, about 200 lines, about 13 variants): `damage_in` (`schema.rs:131`) calls `Damage::in_chain` and needs `source()` chains, so the open-path variants carry `#[source] rusqlite::Error` (otherwise damaged caches are no longer started over); plain `?` on sqlite errors uses one `Sqlite` variant (`#[from]` is ambiguous across `schema.rs`, `reader`, `writer.rs:182,292,299,319` and `maintenance.rs`); callers: `app/images.rs:288`, `app/library.rs:423` (`.map_err(failed)` formats `{e:#}`), `thumbs/src/lib.rs:219,220,238,240,243,337,413,415`, `writeback/src/lib.rs:676`, `library/src/{scans.rs:12 (returns Vec<(FileId, anyhow::Error)>: its signature changes),removals.rs:30,36,39,coordinator.rs:787,lib.rs:38}`, `cli/src/lib.rs:438,545`; tests `writer.rs:529,531` (`anyhow!("busy")`), `lib.rs:856,876,1010,1403`.
- **Public anyhow surfaces elsewhere** (converted in their own steps): `render::MakeError::Undecodable(anyhow::Error)` (`maker.rs:56`), `scan::ScanError::Failed(anyhow::Error)` (`lib.rs:329,465`), `library::folders::Finish(anyhow::Error)` (`folders.rs:62`), `jobs::Outcome::SetupFailed(anyhow::Error)` (`pipeline.rs:111`). None of the planned enums holds a non-Send field.

### Text

`pg_platform::error::chain(&dyn Error) -> String` replaces `{:#}`, but equality is NOT automatic: `ReadFailed` displays a message plus its io source (`platform/src/file.rs:36-47`), `ScanError::Failed` prints `{e:#}` itself (`scan/src/lib.rs:347`), `#[error(transparent)]` forwards both, and `.context(path)` becoming a `path` field changes order unless the Display repeats it. Pin one chained message per crate and re-run the pins at every step: `app/src/error.rs:74,80,87`, `app/src/app.rs:104,211`, `library/src/lane.rs:81`, `thumbs/src/lib.rs:86,652`, `writeback/src/lib.rs:323,677`, `thumbcache/src/writer.rs:216,231,266`, `thumbcache/src/lib.rs:856,876,1010,1403`, `thumbcache/tests/other_process.rs:70`, `borders/src/lib.rs:107`, `scan/src/lib.rs:1184`, `catalog/src/lifecycle.rs:727`, `catalog/tests/other_process.rs:63`, `edit/src/rewrite.rs:44,56`, `model/src/notice.rs:1042` (`error_text`), plus the notice goldens.

### Definition of done

`tools/anyhow-allow.txt` is empty, the manifest test is green with only cli, desktop and gallery permitted, decision 14 is in `crates/README.md`, and G-full plus `just check-linux` are green (the error enums in `platform`'s Linux code are only checked there). Gate per step: G-check; G-full and `just check-linux` at each layer-group end and at done.

## Phase 8: CU-005 unsafe policy

### 8.0 Pilots first, with a pass rule

Whether `unsafe extern "system" {` and `#[unsafe(no_mangle)]` trigger `unsafe_code`, whether derive macros emit `unsafe`, and whether cxx-qt bridges and the bindgen `include!` in `video/src/ffi.rs` trip the lint are unverified. Pilot on `trace`, `container`, `model`, `cli`, `video` and one cxx-qt bridge.

- Pass means `just lint` (not just `cargo check`) is green with the `forbid`/`expect` attributes in place. An `#[expect]` is only added where the lint fires (an unfulfilled expect is itself a warning).
- Fallback: a per-item or per-module `#[expect(unsafe_code)]` on the macro-emitting module, or, for a crate whose derives emit unsafe, document the omission of `forbid` in the ledger.
- Known from the dependency sources: serde_derive, strum_macros, thiserror-impl and clap_derive emit no `unsafe`; `model`'s specta derive emits `#[allow(unsafe_code)] unsafe fn` only under specta's `collect` feature (`DO_NOT_USE_collect`), which the workspace does not enable, so never enable it in `model`; `unsafe extern` blocks alone may not fire `unsafe_code` (then no expect there); `tests/` and `examples/` are separate roots outside a crate's `forbid`, so under the workspace `deny` they need their own expects (listed below).

### Mechanism

- Every manifest is `[lints]` + `workspace = true` and Cargo rejects extra lint tables beside it, so staging uses crate-root attributes: `#![forbid(unsafe_code)]` in every crate except video, platform, desktop, dedup and trace (catalog, library and container have unsafe only in examples/tests, which are separate roots).
- For unsafe code use per-item or per-module `#[expect(unsafe_code, reason = "FFI: <call>")]`, never a crate-level expect, inside the same `cfg` as the item.
- Set `unsafe_code = "deny"` at `[workspace.lints.rust]` (decision 1; it replaces the root comment's "rust lints stay `warn`" for this one lint). A crate-root `forbid` outranks the command-line `deny` (known rustc behaviour; the 8.0 pilot confirms it), and `allow_attributes` (root `Cargo.toml:84`) does not fire on `#[expect]`.
- Rewrite the `unsafe_code` row at `CLIPPY_LEDGER.md:155` (its level column says `warn` and its Work column says unsafe crates get a crate-root `#![deny(unsafe_code)]`, because `forbid` cannot be overridden by `#[expect]`; 153-154 are the table header) to "workspace `deny`, `forbid` in unsafe-free crates, per-item `expect` elsewhere" and update the ledger's "Current configuration" bullets (`:7-15`) in the same edit. That is CU-005's own ask, so there is no deviation to record.

### Items

- `trace`'s `#[unsafe(no_mangle)]` inside the `stages!` macro: one `#[expect]` on the `stages!` invocation (an expect written inside the macro body is emitted per item, which is also fulfilled), if the pilot shows the lint fires.
- `dedup` AVX2 expects inside the `cfg(target_arch)` items.
- desktop is more than `*_bridge.rs`: module-scoped expects for the cxx-qt bridge modules (including `app_icon`, `clipboard`, `live_reload`), placed on the `#[expect(unreachable_pub, ...)]` lines of `desktop/src/lib.rs:6-37`; per-item expects for hand-written unsafe (`animation.rs:45,47`, `deadline.rs:17,30,36`, `video_source.rs:25,27`, `frame_sink.rs:22,24` `unsafe impl Send/Sync`, `gpu_device.rs`, `live_reload.rs:7,30`, `lib.rs:118` `set_var`, `lib.rs:196,217,221` unsafe blocks).
- Examples/tests: `catalog/examples/stmt_journal_probe.rs:203,227,533`, `container/examples/png_read_bench.rs:141,148`, `library/tests/crop_tags.rs:53,83` (removed by Phase 3). `video/build.rs`: the generated bindings are not editable, so the expect goes on the including module.

### Standing check and order

A Python test asserting that every crate not on the unsafe list has `#![forbid(unsafe_code)]`, wired into `just test-tools`. Phase 8 follows Phase 7 only because both edit the same files. Linux-cfg items (`platform` Linux halves, `audio.rs`) are checked only by `just check-linux`, which is therefore a required gate here: a missing `expect` on a Linux-only unsafe block, or an unfulfilled one, turns CI's `linux` job red under `-D warnings`. Gate: G-full on Windows plus `just check-linux`.

## Phase 9: AD-004/005/009 `missing_docs`

- Counts are lower bounds (the plan's earlier totals, model about 272, client about 184, catalog about 100, include enum variants; a heuristic over items and fields alone gives model 161, client 74, catalog 73, app 48, platform 34, jobs 25, codec 23, library 21, thumbcache 17, yandex 15, container 14, the rest under 10). About 24 crates are under 20 and seven (cli, exiftool, gallery, metadata, testkit, testrng, trace) are at zero: one cheap first wave, then the 15-35 group, then app, client, catalog, and model last; Phase 6 runs first so app and dbfile shrink. Stage per crate by crate-root `#![warn(missing_docs)]`, one commit point per crate; the list of crates carrying the attribute is the ratchet. When the workspace is clean, flip to a workspace `missing_docs = "warn"` (+ CI `-D warnings`) and delete the crate-root attributes. No blanket `#[allow]`. Move `missing_docs` out of "excluded" in `CLIPPY_LEDGER.md:329-330` and update its "Current configuration" bullets (`:7-15`). The docs state meaning, not the item's name.
- **Pilots before committing** (pass: `just lint` green with the attribute on):
  - The `messages!` macro (`client/src/text/message.rs:71`) forwards variant docs but not field attributes (69 of about 131 variants have fields): extend its grammar to accept field docs; the hand-written `Notice*`/`SourceLabel` variants need field docs too.
  - `model/src/detection.rs`: `Edge`/`Kind`, their variants and `from_key`/`opposite` have no docs (hand-written, so flagged); strum's own output is documented (`enum_iter.rs` emits a `#[doc]`).
  - `desktop`'s modules are private (`mod x;`), so `missing_docs` may not fire there and an `#[expect(missing_docs)]` would be unfulfilled: add an `expect` only if the lint fires.
- `cargo doc ... -D warnings` already runs in `just lint`; nothing to add. Visibility (Phase 6) and error types (Phase 7) come first so each surface is documented once.
- Gate: G-check per crate; G-full per crate commit point and at done.

## Phase 10: reopened items and partials (one commit point per sub-item, each with a PROGRESS row)

### N-039

- Ids in `text.ftl` are topic-prefixed already (247 ids: `notice-` 103, `yandex-` 36, `filter-` 16, ...). `ui-` (178 ids) and `cli-` (56) carry a per-file prefix (`ui-` is asserted at `client/src/text/words.rs:122`; `words.rs:123` only asserts that `text.ftl` has no `ui-` ids); `text.ftl` is the only file without one and has no prefix check at all.
- Decision 3: rename every `text.ftl` id to `msg-<existing id>` (nothing starts with `msg-` today). The dry run found no intra-file references in `text.ftl` (no `{ other-id }`, no terms, no attributes, no functions), so only the id at the start of each message changes; the indented `*[variant]`, `[one]`/`[other]` keys stay; the rewrite must preserve lines 195-196 (`catalog-read-only =The ...`, no space after `=`, which the test parsers accept) and update the comments naming ids (`text.ftl:64,165,402`).
- Code edits, about 160 lines in 4 or 5 files: the 129 `messages!` table literals (`crates/client/src/text/message.rs:129-315`, form `Variant => "id"`, plus doc lines 79,82,83); the 14 `source_id()` literals (`crates/client/src/text/notice.rs:45-58`, plus comments at :1,42); the two `concat!("notice-", $code, "-title"/"-body")` lines (`crates/model/src/notice.rs:966,973`; the 51 codes stay) and the test `format!("notice-{}-title", ...)` at `:1596-1597`, plus comments at `notice.rs:21,26,56,131,170,944`; `crates/client/src/text.rs:3`.
- Do NOT touch the stored notice codes, which are not text ids: `"scan-note"` (`catalog/src/inbox.rs:632,639,824`, `library/src/passes.rs:106`, `library/src/runner.rs:489,509`), `"not-a-database"`, `"scan-folders"`, `"status-poll"`, `"ledger-write-*"` (`notice.rs:298,435,782,818-820`), the Yandex CLI options and test folder names (a blanket replace would break them; this is why the rename is done by hand with Edit).
- `cli.rs:622` is `crates/client/src/text/cli.rs:622` and concerns `cli-` ids in `cli.ftl`: it needs no edit and is the model for the new prefix check. No QML, C++, desktop, app, library or cli crate holds a `text.ftl` id literal (they use `Message::Variant`); the golden files hold codes, not ids.
- Each id differs, so `replace_all` does not help: rewrite `text.ftl` (419 lines) once with Write, then Edit the code references. Existence is already proven by the parity test `message.rs:346-356` (`the_messages_and_the_english_entries_name_the_same_ids`, built from `Message::samples()`, which covers the table, every `Notice::samples()` title and body and every `Source::ALL`; a missing entry also fails `every_message_is_worded_by_its_entry`), so no new existence test is needed: run it (`cargo test -p pg-client -p pg-model`) before and after, and add the prefix assertion `every text.ftl id starts with "msg-"` next to `client/src/text/words.rs:123`. After the edits, grep as a no-script check: no `"` followed by a bare family prefix (`"(notice|source|yandex|filter|pass|catalog|...)-`) remains in Rust except the stored codes above, and `msg-` appears 247 times in the ftl, 129 times in the table, 14 in `source_id` and in the two `concat!` lines.
- Rename `Hint::Up4`/`Down1` and their ids `ui-hint-up-4`/`ui-hint-down-1` to `Hint::UpFour`/`DownOne` and `ui-hint-up-four`/`ui-hint-down-one` (hint ids live in `ui.ftl`; `keys.rs:258-282` maps each `Hint` to its id).
- Gate: G-full + G-qt.

### D-063

- One shared `WheelStepper.qml` with a `reset()` hook, for `Viewer.qml:349-361` (a WheelHandler in a Loader'd `Item`, never reset) and `YandexPreview.qml:56-66,184-190` (in the Flickable, reset in `open()` at :52), with a placement that still lets the Flickable take the wheel first.
- Interface: `WheelHandler { property real notches; signal stepped(int steps) (already negated: positive means next); function reset() { notches = 0 } }` accumulating `angleDelta.y / Wheel.notch` and emitting `stepped(-trunc(notches))`. Both copies behave identically today (default handler, divide by `Wheel.notch`, accumulate, trunc); the only difference is that Yandex zeroes its accumulator in `open()` and Viewer never does (resetting in Viewer when it hides is a harmless fix). Delete `wheelNotches`/`notchesFor` from `YandexPreview.qml` (nothing else references them).
- `WheelListView.qml` has no accumulator and does not use it; `GalleryGrid.qml:88` (Ctrl-zoom) stays out; `crates/desktop/qml/Wheel.qml` is an existing notch singleton (`notch: 120`, used at `Viewer.qml:355`): keep it distinct from `WheelStepper.qml` in `build.rs` and the qmldir. No `NeighbourPreload` (decision 2; record the reason in the disposition).
- Registration: a plain component, so it goes in `qml_files([...])` at `crates/desktop/build.rs:28-67`, not `QmlFile::singleton`; `qmltest.py:56` globs `*.qml` and builds its own qmldir; `lint_qml.py:9` copies "as the qmldir lists them" and needs a build first.
- Add a `tst_` test. Check first that `mouseWheel` reaches a `WheelHandler` under the offscreen test host; if it does not, move the accumulation arithmetic into a JS function inside `WheelStepper.qml` and test that function directly.
- Gate: G-qt + `just shots`.

### T-020

- `pg_testkit::skip` already exists (`platform/src/testing.rs:91-92`, re-exported). One mechanism, two named functions: `skip` (prints and passes; for DATA gates) and `require_tool` (a RENAME of `missing_tool`, `testing.rs:72-85`, which already fails under `PG_REQUIRE_TOOLS`, set by `ci.yml:51`; for external TOOLS).
- The five env-gated `#[ignore]`s (`catalog/src/browse/read/tests.rs:405,539`, `catalog/tests/rehearsal.rs:89`, `codec/src/decode.rs:437`, `dedup/src/join.rs:2908`; `audio.rs:107` is a config gate and `testing/tests.rs:276` a subprocess helper) are data gates and use `skip`. All five fail today when their variable is absent (`expect`/`unwrap`/`panic!`), and nothing in CI sets those variables or runs `--ignored`, so `skip` (which only prints and does not read `PG_REQUIRE_TOOLS`) preserves CI.
- Delete `tool_missing` and the `exiftool_missing` wrappers (`library/tests/crop.rs:192`, `writeback.rs:124`); `exiftool/src/lib.rs:285` is `start_or_skip` (keep it if it is a tool gate); other tool-gate users: `exiftool/src/lib.rs:291`, `library/tests/crop.rs:71`, `library/tests/video.rs:38,136,256`.
- Caveats: dropping `#[ignore]` means `cargo test` now runs the rehearsal and `PG_JOIN_*` dump tests whenever their env vars are set (say so in the README with the choice rule); `dump_join` must gate on BOTH `PG_JOIN_DUMP` and `PG_JOIN_OUT`; the doc comments that say `-- --ignored` (`tests.rs:402-403,536-537`, `decode.rs:433-436`) must be rewritten; the skip message names the variable (a typo now passes silently); the rename also touches the re-export at `testkit/src/lib.rs:19` and the comment at `.github/workflows/ci.yml:44` (`pg_testkit::missing_tool`).
- Gate: G-full.

### Partials (each with its own gate)

- **U-001**: `crates/query/src/lib.rs:87` already defines `MB = 1_048_576` commented as MiB; relabel text only (decision 12), only where the value is binary: the audit's other cites (`cli lib.rs:237` `gigabytes()` at 1e9, `report.rs:128` mbps, `lifecycle.rs:268`) are decimal rates and totals and stay. No `Cap::from_gb` docs remain outside the audit. Gate: G-full.
- **U-035**: `library/examples/pairs_probe.rs:75-78` already prints `cycles ... M` (the audit's "prints raw" is stale) and `import/examples/pairs_load_probe.rs:102` divides by 1e6 the same way. The only Rust formatter is `client/src/text.rs:294` `group_digits(u64)` and `pg-library` does not depend on `pg-client`, so add a small `millions(u64) -> String` helper in the lowest crate both probes reach (`pg-model`, a new `num` module if none exists), matching `medians.py:21`, use it in both probes, and rewrite the disposition. Gate: G-check.
- **D-051**: `version_key` is at `tools/fork-updates.py:49`; `tools/profiling/callgraph/build.py:24` `newest(pattern)` compares the digits in a path (a path picker, not a version-tuple compare). Share `version_key(version_text)` through `pgtools` between them (the hyphenated `fork-updates.py` is not importable), and correct the audit's stale text at `CONSISTENCY_AUDIT.md:866`. Gate: `just test-tools` + ruff.
- **D-061**: the three predicates differ (`dirs.rs:48` `is_absolute_for` is text-based and cross-OS, accepts `C:\`, `C:/` and any leading `\\`; `native.rs:215` `form_of` is Windows-only, UTF-16, requires `C:\` and rejects `/`, verbatim and device prefixes; `picker.rs:19` `has_drive_or_share` uses `Path` components, so it also accepts drive-relative `C:foo` and rejects `\\?\Volume{...}\`). Pick one `pub` predicate (in `dirs.rs` or `platform`) with the `form_of` rule for verbatim volume GUIDs, and pin it with a `#[cfg(windows)]` table test over `C:/x`, `C:foo`, `\\?\Volume{...}\`, UNC and device paths. It changes what the picker hands the shell (the audit says so): about 60 lines in 3 files. Gate: G-full.
- **A-023**: mostly done already: `bench_thumbs.py` imports `vips_thumbnail` from `common`, and `verify_types.py` holds `ROTATIONS`, `Checkpoint` and `COLUMNS`. `Features` and `PairRunner` use torch/kornia (`verify.py:71,79,154-191,376-384,586`), so `verify_synth.py:23-33` is a legitimate consumer of that module. What remains is moving the torch-free pieces (`Prefetcher`, `work_image`, the argument and output helpers) into a new `eval/verify_common.py` (about 120 lines) so non-torch tools can import them; `just test-tools` cannot import torch, so the gate is ruff plus a non-torch import test. Rewrite the disposition to say what was already fixed.
- **A-028**: apply the Phase 0 `just modgraph` result (see Phase 0). Gate: G-full + `just modgraph`.
- **A-031**: the patches apply to the repo's own sources (not to Qt): `app-build.patch` (303 lines; it also creates `crates/desktop/cpp/static_qt.cpp`, 77 lines) and `pg-video.patch` (235 lines; gated on `PG_FFMPEG_STATIC` and the `pg_ffmpeg_static` cfg, not on `PG_QT_STATIC`). Move that wiring into `crates/desktop/build.rs`, `src/gui_app.rs`, `cpp/static_qt.cpp`, `crates/video/build.rs` and `src/ffi/load.rs` behind the env vars, default off; re-point `tools/test_ci_files.py:275-292` (it reads both patches for `STATIC_FFMPEG_LIBS`) at `build.rs`; update `docs/reference/STATIC_BUILD.md:186-195` (the apply step). Gate: default G-check/G-full unchanged, plus `cargo check` with `--cfg pg_static_qt` and `PG_FFMPEG_STATIC` as STATIC_BUILD.md does, plus the recipe's configure step; the link is not exercised (the full static Qt build is not run). Medium size.
- **AD-002**: the Python rule is `ANALYSIS_DRAFT_SIDES = (512, 320)` at `eval/common.py:75` (not `nvjpeg.py`); on the Rust side the real source is `crates/analysis/src/analyze.rs:29,31` (`DECODE_MIN_LONG`, `DECODE_MIN_SHORT`) and `eval/jpeg-decoders/src/lib.rs:78-81` repeats it as inline literals in `denom()`. Hoist those literals into named consts, then add one test in `tools/` that reads the three places and asserts they agree (about 40 lines). Gate: `just test-tools`.
- **ML-017**: the six remaining stale cites listed in the finding's own text. **E-048**: keep image's "color" spelling and say why, pinned by the decode test (close).
- **A-017, LAST in this phase, and only after F-48's facade criterion is decided** (`REFACTOR_LEDGER.md` F-48 says "the S-12 facade criterion is undecided", F-49 is an unbuilt spike, F-46 an unbuilt startup gate; write the criterion into the ledger first, or A-017 stays Deferred).
  - The audit's 117 is every `function` declaration in the QML (Main 32, Viewer 17, InboxPanel 15, FilterBar 10, LogPanel 8, YandexPanel 6, GalleryGrid 5, the rest small; F-45's 106 is stale), and many are signal re-emits or view resets (`Viewer.qml:160-200`). The real logic is about 25 to 30 functions: Main (`updateFacets`, `relayout`, `gridLoaded`, `treeLoaded`, `select`, `applyView`, `askRemoveFolder`/`askDiscardRemoval`, `openPanel`/`closePanel`, `reloadAll`, `scrollTo`), InboxPanel (`select`, `moveTo`, `requestFiles`, `dismissSelected`, `retrySelected`), FilterBar (`countFacets`, `reloadQuery`, the facet handlers).
  - A facade is a `#[qinvokable]` or event on an existing bridge plus a `pg_client::controller` event and command plus a cargo replay test (the nearest pattern is `desktop/src/{controller,shell,shell_bridge}.rs`). `just qmltest` runs against stubs (`tests/qml/stubs`, "check the QML and nothing Rust", `Justfile:169-174`), so the stubs change too and the logic is only proven by `cargo test` and `just shots`: gate G-full + G-qt, not G-qt alone.
  - One component per commit point; Main is about 4 files and 300 to 500 lines, the others 2 to 3 files and 100 to 200 lines each.

### Docs and dispositions, after every phase

Sweep the docs that describe the old state (`CLIPPY_LEDGER.md`, `REFACTOR_LEDGER.md` F-14/F-38/F-45, `crates/README.md`, `tools/README.md`, `.env.example`). Rewrite each touched disposition IN PLACE as `**Disposition: Fixed.** <what changed, where>` (or keep `Deferred.` with the reason); never append beside the old line. Forms in use: `Fixed.`, `Deferred.`, `Fixed in part.`, `No change.`, `Moot.`, `Decided and applied.`.

- Four lines state the OPPOSITE decision and must be replaced: N-039 (audit 2171), D-063 (936), T-020 (3845), and PF-026's note at 2559; N-034 (2139-2141) keeps `No change` but gets the grain rule stated. Also check E-045 (1295, "No change", pending renames).
- Findings the plan newly closes: ML-018 (via `just deny`), E-002 (`screenshot.cpp`, with the forced-timeout shot).
- Nothing parses the audit; markdownlint (every `**/*.md`, default rules with MD013 and MD060 off) is the only gate, so run it on the file after the edits. A table in `docs/plans/PROGRESS.md` is accepted (escape any `|` inside cells).

## Verification summary

- Standing checks, each with a Python test so it cannot rot, all wired into `just lint`/`just test-tools` (CI runs both): anyhow-in-manifest (`cargo metadata`, Phase 7.0b); `std::env` outside the env module (regex `\benv::(var|var_os|vars|vars_os|set_var|remove_var)`, Phase 3); one-public-path-per-item (Phase 6); `forbid(unsafe_code)` presence (Phase 8); PG_* registry against docs (Phase 3). The modgraph tools read `crate-layers.txt` and are unaffected (no class token).
- Per phase: G-full (see Cadence for the multi-session phases), plus `just check-linux` at the end of Phases 6, 7 and 8. Phases 4, 5 and 7 must be green before the next phase starts.
- Highest-risk regressions and their guards: the D-001 carry on a real catalog (old-layout carry test, "delete nothing", old file kept intact); Phase 7 text and retry behaviour (the 7.0 characterisation tests incl. the panic case, notice goldens); Phase 5 shutdown order (the new CLI Write-arm test with a timeout).

## Not verifiable here

Linux-cfg code and the Linux halves of `platform` (only via `just check-linux`, which needs Docker Desktop; desktop is never linted on Linux); cxx-qt-generated unsafe scope and `missing_docs` behaviour on the private desktop modules (the Phase 8/9 pilots decide); the C++ env parser is tested for spelling parity, not run; Qt 6.12 behaviour of the new QML components (`just qmltest`/`just shots`; the reference images are from `262004b119` and may be stale); `just deny` depends on the advisory DB at run time; the Qt static manifest and the static link (the full static Qt build is not run); V-030 needs both nvJPEG toolkits; the external items in Phase 0.

## Critical files

`crates/jobs/src/{failure,panics,pipeline}.rs`, `crates/thumbcache/src/{reader,lib}.rs`, `crates/platform/src/env.rs`, `crates/library/src/layout.rs`, `crates/catalog/src/{baseline.sql,types.rs,removal.rs,writers.rs,lifecycle.rs,rebuild.rs,classify.rs,roots.rs}`, `crates/model/src/{rules,turns,write,versions}.rs`, `crates/client/src/controller.rs`, `crates/desktop/{build.rs,src/lib.rs}`, `tools/boundaries.py`, `tools/test_ci_files.py`, `crates/README.md`, root `Cargo.toml` (workspace lints), `Justfile` (new `check` recipe), `docs/plans/PROGRESS.md` (new), `tools/README.md` (rows for `tools/test_boundaries.py`, `tools/anyhow-allow.txt`, the env scanner and the extended `test_ci_files.py`).
