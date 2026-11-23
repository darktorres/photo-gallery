# Consistency-audit program: open tasks

The working rules and gates below apply to every task. Each task says what it needs. Items with a finding ID (for example E-017) come from the consistency audit; the audit's full text is in git.

## Working rules

The owner's saved preferences still apply: edit with the Edit and Write tools, never scripts; batch validation (finish a batch of edits, then validate once); compile natively on Windows (the Linux container pass, `just check-linux`, needs Docker Desktop); never run the hour-long static Qt build; no background tasks or monitors (run commands in the foreground; a command that exceeds the 10-minute cap is run by the owner, who pastes the result); commit only when the owner asks; never launch the app against the owner's real catalog (`.env`'s `PG_CATALOG_DIR`, `D:/PhotoGallery/catalog`, with `thumbs.sqlite` beside it and prefs in `D:/PhotoGallery/gallery.json`): `just shots` and `cargo test` isolate their state (`tools/compare_shots.py` sets `PG_CATALOG_DIR` and `PG_PREFS`; tests use `PG_TEST_DIR`), a manual run does not, so use a scratch copy. No branches: history goes straight to `main`. Commit messages follow the repo style: an imperative subject of at most about 72 characters, a body wrapped near 120 columns, and the `Co-Authored-By` trailer the session specifies.

## Gates

- **G-check** = `cargo check --workspace --all-targets --locked`, run through the repo's environment: `just check` (`Justfile`). It does not cover `fuzz/` or `eval/*-decoders` (separate workspaces); `tools/lint.py` runs `cargo metadata --locked` on them.
- **G-lint** = `just lint`: `tools/lint.py` (fmt, clippy `-D warnings` including the wasm and `dhat-heap` variants, `boundaries.py`, cargo-machete, ruff, markdownlint, and `cargo doc` with `-D warnings`), then the separate `lint-qml` recipe.
- **G-full** = `just build debug`, then G-lint, then `cargo test --workspace` and `just test-tools`. Add `just modgraph` when the change touches module boundaries. Rebuild after any QObject-bridge change.
- **G-qt** = `just build debug` and `just qmltest`.

## Owner decisions and confirmations

1. **Backup hashes.** The backup at `D:/PhotoGallery/backups/pre-consistency-program-2026-11-21` was checked by byte size. The live `gallery.json` has changed since (332 bytes, written 2026-11-23, against 315 in the backup). *Needs:* the owner confirms the hashes.
2. **Owner's catalog.** `D:/PhotoGallery/catalog/catalog.sqlite` was last written 2026-11-23 17:57. *Needs:* the owner confirms that this is the first-open rebuild (Phase 4).
3. **D-061, the `\\?\Volume{GUID}` paths.** The three drive-or-share predicates are at `crates/platform/src/dirs.rs:48` (`is_absolute_for`), `crates/platform/src/native.rs:216` (`form_of`) and `crates/platform/src/picker.rs:22` (`has_drive_or_share`). *Needs:* the owner's decision, then the planned `shell_form` (`docs/plans/MOUNT_POINTS_PLAN.md:90–97`) replacing the three, then a Windows check of the dialog.
4. **Phase 4, failed carry.** A failed carry rebuilds an empty catalog, reported as `Carried::Nothing` (`crates/catalog/src/rebuild.rs:78-83`). *Needs:* the owner or the team decides the behaviour.
5. **Phase 6, field visibility.** The plan's rule: a plain data record has all fields `pub`; a type that holds an invariant has private or `pub(crate)` fields and accessors. `crates/catalog/src/pair/coverage.rs` is plain data and may already comply. `crates/catalog/src/lease.rs` (`HeldLease`, `pub(crate)` fields) and `crates/metadata/src/xmp.rs` (`MetaScratch`, `pub` type with `pub(crate)` fields) need a decision. *Needs:* a decision on each, then any change.
6. **UI-018, glyphs and separators.** `crates/desktop/qml/YandexPanel.qml:79` uses `" (" + count + ")"`; the HUD, FilterBar and SaveViewDialog use `" · "`. Glyphs remain at `TopBar.qml:91` (▴/▾), `StatusBar.qml:100` (⚠) and `ResultTile.qml:92` (×). *Needs:* a decision on the separator and the glyphs, then a screenshot check.
7. **E-002, `colour_check`.** E-002 is not in the open audit at `d47681f6`. *Needs:* after the re-run in the GPU section below, a decision whether E-002 is still open.

## Runs that need the GPU machine, the nvJPEG wheel, or a full build

- **U-029, encoder test.** `test_encoder.exe` builds; the test run is not done. *Needs:* run it on the GPU machine.
- **U-030, PNG counts.** Both harnesses count requested bytes (`eval/png-decoders/harness/harness.cpp`, `eval/png-decoders/src/main.rs`). *Needs:* run the comparison.
- **E-002, `colour_check`.** The 4:2:0 and 4:2:2 results are from 2026-11-14 and were not re-run. *Needs:* re-run `colour_check`.
- **E-017, failure policy (medium).** The policies differ at these lines: `eval/ann/run.py:343–350` records a failed configuration and continues; `eval/accel.py:142–144` skips an image that will not load; `eval/bench_decode.py:141` has no try around each read; `eval/pdq.py:46–53` returns None on any Exception; `eval/verify.py:579–580` records the pair as an error decision; `eval/jpeg-decoders/nvimgcodec_bench.py:98, 114, 126` catch Exception; `eval/jpeg-decoders/nvjpeg.py:285, 299, 304` catch RuntimeError. *Needs:* a GPU run of `nvimgcodec_bench`, to see the exception it raises, then narrow its catches.
- **V-030, nvJPEG header (low).** `eval/jpeg-decoders/nvjpeg.py:6–7` says the declarations come from torchvision's DLL (header 13.2), not the `nvidia-nvjpeg==13.3.2.43` wheel pinned in `eval/jpeg-decoders/requirements.txt:8`. *Needs:* diff the declarations against the wheel's header, on a machine that has the wheel, or run the GPU check.
- **P-004, parajpeg rev (medium).** Production pins darktorres/parajpeg at `8e2c53a` (`Cargo.toml:40`, `fuzz/Cargo.lock:353`). The eval harness pins anniigr/parajpeg at `f45081d` (`eval/jpeg-decoders/sources.py:19`), and `linux_build.py` and `win_build.py` use that pin. `docs/reference/JPEG_DECODERS.md:489` also says `f45081d`. *Needs:* move the eval harness to the fork's rev, through the win_build chain, then re-measure.
- **Phase 4, catalog bench.** *Needs:* run the catalog bench before and after the failed-carry decision.
- **Static Qt build and link.** *Needs:* run it on a machine that can build the static Qt (`docs/reference/STATIC_BUILD.md`).
- **C++ env parser.** It is only spelling-tested. *Needs:* a real test run.
- **QML shots after Phase 10.** *Needs:* run them against the reference set `tools/shots/reference/f4bbb48419`.
- **`just deny`.** It passed at `ec85b503`; it needs the advisory database at run time. *Needs:* run it with the database available.

## Pushes and network access

- **TL-007 and `ec85b503` (external items).** `ec85b503` is on local `main` and is not pushed; `origin/main` is `c92a28ca`. *Needs:* push, then check that the CI step at `.github/workflows/ci.yml:347` (`cargo check --manifest-path eval/jpeg-decoders/Cargo.toml --locked`) passes.
- **P-024, apt packages (low).** `docker/apt-packages.txt` lists unversioned packages, as does `tools/profiling/ci_setup.py:45` (`mold`) and `crates/video/build.rs:49` (`APT_PACKAGES`). `tools/profiling/bench.py:81–82` says the list is not pinned to a snapshot. The Dockerfile base is pinned by digest (`docker/Dockerfile:7`). *Needs:* an apt snapshot date or exact versions. Choosing them needs network access.
- **P-027, Yandex User-Agent (low).** `crates/yandex/src/http.rs:14` sends `Chrome/108.0.0.0`. *Needs:* a live Yandex check before the User-Agent is changed.

## Code and doc changes

- **DOC-072, measured timings (low).** Two timings have no source: the HDD order speedup (about 3×) at `crates/catalog/src/work.rs:445`, and the 0.8 s pairs pass at `crates/catalog/src/work.rs:558`. The cold-read figures named in the finding were not found in the crates' doc comments. *Needs:* a source (run date, machine, bench) for each remaining figure.
- **E-063, CUDA error text (low).** `eval/jpeg-decoders/gpu.py:45` and `nvimgcodec_bench.py:44` use `cuInit: CUDA error (code N)`; `eval/jpeg-decoders/nvjpeg.py:110` uses `<what>: returned an error (code N)`. The C and C++ macros already match. *Needs:* align the Python messages.
- **N-017, `showAsked`.** Still in `crates/desktop/qml/Main.qml` (lines 141, 351, 859). *Needs:* rename it, or record that the rename covered signals only.
- **Phase 5, stale comment.** `crates/cli/src/report.rs:23-25` says the thumbnail cache is leaked; nothing in `crates/cli` leaks it now. *Needs:* fix the comment.
- **`docker/check.py:9`.** Names `missing_tool`, which exists nowhere in the code. *Needs:* fix the name.
- **Phase 7, characterisation tests.** The thumbs classification tests (`crates/thumbs/src/lib.rs`) and jobs panic tests exist. No ExifTool spawn, pipe or timeout tests were found by name in `crates/exiftool`. *Needs:* add those cases, or record that the existing tests cover them.
- **D-061, the shell step.** Implemented only after item 3 above is decided. *Needs:* the `shell_form` change and the three predicates replaced by it.
