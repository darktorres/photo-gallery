# Progress: consistency-audit program

Status as of 2026-11-21, against the commit that holds this file (`261ea501` and the commits after it). The plan is
`docs/plans/CONSISTENCY_PROGRAM_PLAN.md`. This file is scratch state: delete it when the program ends.

Phase 0 is finished. Its last item, `profile.ps1`, is removed: `profile.py` runs now that `callgraph/build.py` runs the Pin kit's wrappers.
Phases 1 to 3 are committed (`e2c3b4a5`, `3f8f8cbd`, `2e93ff10`). Phase 4 (D-001, one folder one row) is in the working tree, uncommitted. Its gates are in the last rows of the table. The first open of the owner's real catalog after this change rebuilds it (the schema changed), and the old file is kept as `catalog.sqlite.replaced`; that open has not been run here.

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
| Phase 1: PF-026 (`WriteKind::Turns` to `Rotation`, `write_turns!` to `write_rotation!`; stored key unchanged) | `e2c3b4a5` | green (see the gates row below) |
| Phase 1: N-030 (`key`/`from_key` for stored spellings; `Resolver::token`), N-016 (two test names), N-034 (grain rule, No change), N-010 (`rotation` fields to `turns`) | `e2c3b4a5`; naming rule in `crates/README.md` | green (see the gates row below) |
| Phase 1: T-018 (`pg_render` aliases removed; `pg_model::versions::` everywhere) and T-027 (generators to `pg_testkit::Rng`) | `e2c3b4a5` | green (see the gates row below) |
| Phase 1: N-017 (QML `*Asked` to `*Requested`) | `e2c3b4a5` | green: `just build debug`, `just qmltest` (143 passed) |
| Phase 1 gates, run once at the end of the phase | `e2c3b4a5` | `just check` green, no warnings; `just lint` green (rustfmt fixes in `queue.rs`, `runner.rs`, `lanes.rs`; a doc link in `refusal.rs`); `just test` 1452 Rust tests passed; `just test-tools` 93 passed, 3 skipped |
| Phase 2: U-004 (client turns end to end) | `3f8f8cbd` | `Turns` carries `Event::Turn`/`Command::Turn`; the QML's signed count is converted once in `Shell::turn`; `Turns` serde reads a signed count and writes a bare number |
| Phase 2 gates, run once at the end of the phase | `3f8f8cbd` | `just check` green; `just lint` green; `just build debug` and `just qmltest` (143 passed); `just test` 1455 Rust tests passed (3 new: the serde test in `model`, and two controller law tests); `just test-tools` 93 passed, 3 skipped |
| Phase 3: C-006 (typed `PG_*` environment module) and C-009 (flag spellings and bad values) | uncommitted | `pg_platform::env` has the readers, the `Var` enum and the `VARS` registry; `Layout`, the CLI, the desktop, the thumbnail cap (`Cap::parse_gib`), the ExifTool program (`pg_exiftool::Program`, read once per run; `save_with`/`cut_with` for the tests), and the trace and command-line flags all read through it; the crop tests no longer set the environment; `boundaries.py` has the environment rule; `test_ci_files.py` has the registry check and the C++ spelling check; `.env.example` documents the registered names |
| Phase 3 gates, run once at the end of the phase | uncommitted | `just check` green, no warnings; `just lint` green (rustfmt fixes in `controller.rs` and `trace.rs`); `just build debug` and `just qmltest` (143 passed); `just test` 1459 Rust tests passed; `just test-tools` 97 passed, 3 skipped |
| Phase 4: D-001 (one folder, one row: `path_key` UNIQUE, `match_key`, the subtree range, the carry that merges spellings) and a new finding D-078 (NTFS's case table) | uncommitted | pending: the gates row below |
| Phase 4 gates, run once at the end of the phase | uncommitted | `just check` green, no warnings; `just lint` green (rustfmt in 13 places, clippy's type-complexity and double-comparison, a doc link to a private helper); `just build debug` green; `just test` 1468 Rust tests passed (new: the subtree-range oracle on both rule sets, the residual-fold pin, the rules' worked examples, the carry of two spellings into one folder); `just test-tools` 97 passed, 3 skipped. Not run: the catalog bench before and after (the plan's cost note), `just shots` (no rendering change expected), `just modgraph` (not a Phase 4 gate). |
