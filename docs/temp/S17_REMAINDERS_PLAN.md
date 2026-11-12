# S-17.3 and S-17.5 remainders: probe evidence (HEAD 2016a9b)

This file was the plan for the S-17.3 and S-17.5 remainders. The work is done by S-17 Units A to F, and is recorded in `docs/REFACTOR_PLAN.md` (rows S-17.3 to S-17.6 and S-17.8; the Qt argv result in row S-17.5). The plan's steps are dropped. What is kept is the evidence that is recorded nowhere else: the two probes below, with their outputs. Both were run on 2026-11-06. Their statements about the code describe it as it was then, not as it is now.

## Posix browse probe

Run on 2026-11-06 at the then-current HEAD, as a temporary test, reverted afterwards. It checked whether the browse reads worked under `PathRules::Posix`, since `browse/read.rs::scope` then called `pg_model::path::scope_keys` (Windows string keys) while the writer and the test fixture wrote `rules.order_key(..)`.

- Setup: on `PathRules::Posix`, folders `/photos`, `/photos/Pics`, `/photos/pics` and `/photos/Pics/sub`, one file in each (ids 1 to 4; 2 and 3 share the name `b.jpg`).
- `list_grid("/photos/Pics", false)` → `[]` (expected `[2]`).
- `list_grid("/photos/Pics", true)` → `[]` (expected `[2, 4]`).
- `list_grid("/photos", true)` → `[]`.
- `list_grid("", true)` → `[1, 2, 4, 3]` (correct).
- `facets("/photos/Pics", false)` → total 0.
- `folder_tree` has three nodes, `/photos`, `/photos/Pics` and `/photos/Pics/sub`. `/photos/pics`, which holds file 3, is missing from the tree, and each node's name is the whole path (one component).

Result: the scope bug was confirmed, and the missing case-distinct folder is a second bug, in the tree. Both were fixed in 278dfe8 (CODE_REVIEW C-23): the string key functions of `pg_model::path` are deleted, and the scope, the tree and the views use the catalog's `PathRules`. `scanned_folders_list_their_files_in_the_grid` in `crates/scan/src/lib.rs` runs a scan to the grid under both rules.

## Qt argv probe

The question: does `QCoreApplication::arguments()` keep a command-line argument that is not valid Unicode exactly, with the argv the app passes to `QGuiApplication` and with the CRT's `__argc/__argv`?

The Qt 6.12 source reading (`qtbase/src/corelib/kernel/qcoreapplication.cpp`): `isArgvModified` compares the argv given with the CRT's `__argc/__argv`. If argv is not modified, `arguments()` ignores it and returns `CommandLineToArgvW(GetCommandLine())`, which is exact UTF-16. If it is modified, it falls back to `QString::fromLocal8Bit(argv[a])`. The source lines were not checked against the installed kit's source, which is not on this machine, and the probe below did not confirm the fallback: do not rely on this reading.

Nothing in the repository read `arguments()` (no `arguments` or `QCommandLineParser` use); Qt reads argv only for its own options (`-platform`, `-style`, ...). So no app path depended on the result.

- Method: a temporary dump in `new_gui_application` (`crates/desktop/cpp/gui_app.cpp`), built with `cargo build -p pg-gallery` (debug), and reverted with `git checkout` afterwards. The dump printed `QCoreApplication::arguments()` as UTF-16 units, the CRT `__argv[1]` bytes, `__wargv`, and `GetCommandLineW()` units. `PG_ARGV_PROBE_MODE=crt` passed `__argc/__argv` to `QGuiApplication`; otherwise the app's argv was passed. `gallery.exe` was launched from PowerShell `ProcessStartInfo` (`UseShellExecute` false, `QT_QPA_PLATFORM=offscreen`, Qt's `bin` first on PATH) with the argument `probe-` + U+00E9 + `x` + U+D800 + `y`.
- Output (argument 2, units): the app's argv: `00E9 0078 D800 0079`; `__argv` passed: `00E9 0078 D800 0079`. `GetCommandLineW` carries `D800`. CRT `__argv[1]` bytes: `70 72 6F 62 65 2D C3 A9 78 EF BF BD 79`, so U+FFFD (lossy): the Rust side's `to_string_lossy` and the CRT's narrow argv both lose it. `__wargv` is NULL (the Rust `main` is narrow), so it cannot be used.
- Decision: Qt's `arguments()` was exact in both modes, so `gui_app.cpp` is left as it is and `__argc/__argv` is not adopted. The Rust side reads `args_os` exactly for the path-valued command-line options. The probe code is not in the tree.
