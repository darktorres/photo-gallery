# Progress: consistency-audit program

Status as of 2026-11-22, against HEAD `d47681f6` on `main`. The plan is `docs/plans/CONSISTENCY_PROGRAM_PLAN.md`. Phases 0 to 10 are committed. This file lists only what is still open; completed rows were removed, and the full table is in git history before this cleanup. The audit's open findings are in `docs/plans/CONSISTENCY_AUDIT.md`. Delete this file when the program ends.

| Open item | State | Needed |
|---|---|---|
| Backup hashes | `D:/PhotoGallery/backups/pre-consistency-program-2026-11-21` was copied and checked by byte size; the live `gallery.json` has changed since | the owner confirms the hashes |
| Owner's catalog | `D:/PhotoGallery/catalog/catalog.sqlite` was written 2026-11-22 15:03 | the owner confirms that this is the first-open rebuild (Phase 4) |
| `ec85b503` (external items, TL-007) | on local `main`, not pushed; its CI step has not run | push, then check the CI run |
| `just check-linux` | must run again after `ec85b503` (the Dockerfile changed) | the owner runs it in the container |
| U-029 encoder test | `test_encoder.exe` builds; the test run is not done | run it on the GPU machine |
| E-063 Python messages | `gpu.py` and `nvimgcodec_bench.py` use `cuInit: CUDA error (code N)`; `nvjpeg.py` uses `<what>: returned an error (code N)` | align the Python messages |
| E-002 `colour_check` | 4:2:0 and 4:2:2 are from 2026-11-14 and were not re-run; E-002 is not in the open audit at `d47681f6` | re-run `colour_check`, and settle whether E-002 is still open |
| U-030 PNG counts | both harnesses count requested bytes; the comparison is not run | run the comparison |
| E-017 | Deferred | a GPU run of `nvimgcodec_bench` |
| P-004 | Deferred | move the eval harness to the fork's parajpeg rev, the win_build chain, re-measure |
| V-030 | Deferred | diff the declarations against the nvJPEG wheel's header |
| D-061 | Deferred on an owner decision about `\\?\Volume{GUID}` paths | the planned `shell_form` (`docs/plans/MOUNT_POINTS_PLAN.md`), and a Windows check of the dialog |
| N-034 disposition | the text in the audit (`ba070a12`) was not rewritten | rewrite it: No change, with the grain-rule cite |
| N-017 | `showAsked` remains in `crates/desktop/qml/Main.qml` (lines 141, 351, 859) | rename it, or record that the rename covered signals only |
| Phase 4 failed carry | a failed carry rebuilds an empty catalog, reported as `Carried::Nothing` (`rebuild.rs:78-83`); the catalog bench before and after was not run | decide the behaviour; run the bench |
| Phase 5 comment | `crates/cli/src/report.rs:22-24` says the cache is leaked; that is no longer true | fix the comment |
| Phase 6 field visibility | the examples in the plan (`catalog/src/lease.rs`, `catalog/src/pair/coverage.rs`, `metadata/src/xmp.rs`) were not changed | decide whether they need a change |
| Phase 7 characterisation tests | thumbs and jobs panic tests exist; the per-pass retry-or-record tests were not added | add them, or record that the existing tests cover it |
| `docker/check.py:8` | names `missing_tool`, which no longer exists | fix the name |

The open audit findings with no row above are DOC-072, UI-018, P-015, P-016, P-024, P-027 and TL-007 (`docs/plans/CONSISTENCY_AUDIT.md`).
