# Progress: consistency-audit program

Status as of 2026-11-21, against the commit that holds this file (`261ea501` and the commits after it). The plan is
`docs/plans/CONSISTENCY_PROGRAM_PLAN.md`. This file is scratch state: delete it when the program ends.

Phase 0 is finished. Its last item, `profile.ps1`, is removed: `profile.py` runs now that `callgraph/build.py` runs the Pin kit's wrappers.
Phase 1 has not started.

| Step | Commit or state | Gate result |
|---|---|---|
| Backup: catalog folder and gallery.json to `D:/PhotoGallery/backups/pre-consistency-program-2026-11-21` | verified copy, outside the repo | byte sizes match; gallery.json identical |
| `just lint-tools-setup`, `rustup target add wasm32-unknown-unknown`, `just setup` | done | tools at their pins; FFmpeg already at the pin |
| `check` recipe in the Justfile (the plan's G-check) | `1d6af48d` | `just check` green, no warnings |
| `just build debug` | done | green |
| G-lint (`just lint`) | fixed in `1d6af48d` and `f1348b95`: doc link, QML toggle regression, ruff format and check, markdownlint, `deny.toml` `pg-import` wrapper, dead `AV_NOPTS_VALUE` entry | green, including `lint-qml` (no warnings, 1745 bindings compiled) |
| G-full: `cargo test --workspace --locked` and `just test-tools` | done | 1451 Rust tests passed; 93 Python tests passed, 3 skipped |
| G-qt: `just qmltest` | done | 143 passed, 0 failed |
| `just doctor` | done | every required part in place |
| `just deny` | green after the `pg-import` wrapper | advisories, bans, licences and sources ok |
| `just modgraph` | was red with three cycles from the audit-fix commits; fixed in `4b0f9ea3` (catalog, cli, desktop), no allowlist entry | green: 0 cycle groups, text scan and compiler graph |
| `just shots` | re-recorded from `f4bbb484` into `tools/shots/reference/f4bbb48419` (commit `0f5fc14a`), 26 scenarios | green on rerun; `viewer-turn` differed once by about 196k pixels and is timing-sensitive |
| T-005 (`a_file_that_doesnt_decode...`, two tests) | 20 of 20 runs each pass; each filter matched one test | no flake observed |
| AD-015 (mode lists) | audit disposition rewritten: No change | the cites resolve; the two tools are different tools |
| DOC-036 (`AV_NOPTS_VALUE`) | audit disposition rewritten: Fixed; dead allowlist entry removed from `crates/video/build.rs` | `pg-video` checks green; the generated bindings contain no `AV_NOPTS_VALUE` |
| PF-024 (static manifests) | the static builds are not in use: `D:/PhotoGallery/static` was deleted (4.3 GB) on the owner's instruction, manifests included | closed as not verifiable here; the finding stays Deferred |
| Retained originals: `doctor.ps1`, `qmltest.ps1`, `build-jpeg-turbo.ps1`, `build-static-ffmpeg.ps1`, `win-build.cmd`, `win-env.cmd` | removed; `test_ci_files.py` `Consumers` drops the two PowerShell-agreement tests and their now-unused imports; the stale references in `tools/README.md`, `pgtools.py`, `sources.py`, `gui_app.cpp`, `STATIC_BUILD.md` and `build.ps1` are reworded; the dispositions of D-036, D-047/048, D-053..057, E-011, E-014, C-001, C-017, TL-012, P-011, P-014 are rewritten | `just lint` and `just test-tools` green |
| `profile.ps1` (TL-008, UI-014, C-001, E-011) | `callgraph/build.py` now runs the Pin 4.3 kit's extensionless wrappers under the MSYS2 bash of `DEFAULT_MSYS`; `profile.py` ran against the debug app with a scratch catalog (exit 0, 208 s); the script is removed | TL-008, UI-014, C-001, E-011 dispositions rewritten to Fixed |
| GPU track: packages and binaries (PNG C harness, `colour_check.exe`, `test_encoder.exe`, decoder harness, nvimgcodec venv at `tools/jpeg-eval/venv`) | done; `venv-nv10` removed | encoder fix committed as `ea0db5ae`; the venv imports nvimgcodec 0.10.0 |
| GPU track: U-029 (`encode_gpu.cu`) | the k_dct block-count cast is checked in `encode_chunk` (refused over 0xFFFFFFFF); `test_encoder.exe` builds | Fixed in part; the encoder test run is not done |
| GPU track: E-063 (macro format) | `dec_gpu.cpp`'s CU() now writes `<expr> failed: <error>`, as encode_gpu.cu does; the decoder harness and the encoder build compile | Fixed in part: the Python harnesses still differ |
| GPU track: E-002 (`colour_check`) | `colour_check.exe` ran on two 4:4:4 inputs: 0 mismatched channels | Fixed in part: 4:2:0 and 4:2:2 are from 2026-11-14, not re-run |
| GPU track: U-030 (PNG allocation counts) | the C harness builds with `alloc_count.cpp` | Fixed in part: the comparison is not run |
| GPU track: E-017, P-004, V-030 | not run: E-017 needs a GPU run of nvimgcodec_bench; P-004 needs the win_build chain and a pin change (its URL changes with the SHA); V-030 needs both nvJPEG toolkits | Deferred, with the reasons in the audit |
| Phase 1 (renames and rule text) | not started | |
