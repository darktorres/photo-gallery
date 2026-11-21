# Progress: consistency-audit program

Status as of 2026-11-21, against the commit that holds this file (the Phase 0 fixes are `24681a94` and `f1348b95`). The
plan is `docs/plans/CONSISTENCY_PROGRAM_PLAN.md`. This file is scratch state: delete it when the program ends.

Phase 0 is open. Its runs are done except those noted; two baseline gates are red and need an owner decision (see below).

| Step | Commit or state | Gate result |
|---|---|---|
| Backup: catalog folder and gallery.json to `D:/PhotoGallery/backups/pre-consistency-program-2026-11-21` | verified copy, outside the repo | byte sizes match; gallery.json identical |
| `just lint-tools-setup`, `rustup target add wasm32-unknown-unknown`, `just setup` | done | tools at their pins; FFmpeg already at the pin |
| `check` recipe in the Justfile (the plan's G-check) | `24681a94` | `just check` green, no warnings |
| `just build debug` | done | green |
| G-lint (`just lint`) | fixed this pass: doc link in `crates/catalog/src/error.rs`; 17 files of ruff format; 27 ruff check findings; 11 markdownlint findings; the QML lint regression from `1b361db7` (`Filters.toggle` took a typed enum, which QML cannot resolve; now an int mapped in Rust by `filters::kind_of`); `deny.toml` `pg-import` wrapper for rusqlite | green (`just lint`, `lint-qml` clean) |
| G-full: `cargo test --workspace --locked` and `just test-tools` | done | 1451 Rust tests passed; 95 Python tests passed, 3 skipped |
| G-qt: `just qmltest` | done | 143 passed, 0 failed |
| `just doctor` | done | every required part in place |
| `just deny` | fixed this pass (the `pg-import` wrapper) | advisories, bans, licences and sources ok |
| `just modgraph` | RED: three module cycles. Catalog: classify, core, ledger, lifecycle, rebuild, writers. Cli: crate root and report. Desktop: facet_choices::state and facet_choices_bridge::qobject. The allowlist is empty. The cycles come from the audit-fix commits. | owner decision: break the cycles now, or accept them as Phase 6 debt |
| `just shots` | RED: the inbox fixture changed since reference `914c44ca` was recorded (inbox, inbox-expanded). Fixing it needs `just shots --record`, which rewrites tracked reference images. | owner decision: re-record |
| T-005 (`a_file_that_doesnt_decode...`, two tests) | 20 of 20 runs each pass; each filter matched one test | no flake observed |
| AD-015 (mode lists) | audit disposition rewritten: No change | the cites resolve; the two tools are different tools |
| DOC-036 (`AV_NOPTS_VALUE`) | audit disposition rewritten: Fixed; dead allowlist entry removed from `crates/video/build.rs` | `pg-video` checks green; the generated bindings contain no `AV_NOPTS_VALUE` |
| PF-024 (static manifests) | not run: no `manifest.json` from the 2026-11-13 builds was found, and `PG_HOME` is `D:/PhotoGallery` | stays Deferred until a manifest exists |
| Retained originals: `doctor.ps1`, `qmltest.ps1`, `build-jpeg-turbo.ps1`, `build-static-ffmpeg.ps1`, `win-build.cmd`, `win-env.cmd`, `profile.ps1` | not deleted. Deleting the two build scripts needs `tools/test_ci_files.py` `class Consumers` rewritten; the stale references are listed in the plan | next session |
| GPU track: packages and binaries (PNG C harness, `colour_check.exe`, `test_encoder.exe`, decoder harness, nvimgcodec venv at `tools/jpeg-eval/venv`) | done, outside Phase 0 | encoder fix committed as `ea0db5ae`; `venv-nv10` is a leftover (owner to delete) |
| GPU track: U-029 code change in `encode_gpu.cu` | not started | |
| Phase 1 (renames and rule text) | not started | |
