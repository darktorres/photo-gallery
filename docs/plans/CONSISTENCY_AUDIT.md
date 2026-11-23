# Consistency audit: open items

The validated consistency audit had 702 findings. On 2026-11-22 the findings still open were re-checked against the tree at HEAD (branch `merge-plan`), reading each cited location again. Cites below are to that tree.

Removed as resolved in code: U-029, U-030, U-043, PF-024, P-025. The full audit with its original text is in git at `ba070a12`.

Kept: open findings, partly resolved findings, findings resolved in code but not yet run in CI, and one finding whose recorded status was wrong.

| Status | n |
|---|---:|
| Open | 6 |
| Still true, guarded by a test | 1 |
| Partly resolved | 3 |
| Resolved in code, not run in CI | 1 |
| Resolved, but its record was wrong | 1 |
| **Total kept** | **12** |

## doc-accuracy

### DOC-072 [low] Measured timings in doc comments that the code or benches do not reproduce — PARTIAL

**Status: Partly resolved.** The modgraph timing now has its source: `crates/README.md:153` gives the run dates (2026-11-10 and 2026-11-12). The other timings still have none:

- The HDD order speedup (about 3×): `crates/catalog/src/work.rs:445`, no source.
- The 0.8 s pairs pass: `crates/catalog/src/work.rs:558`, no source.
- The cold-read figures the finding named were not found in the crates' doc comments.

Needed: a source (run date, machine, bench) for each remaining figure.

## duplication

### D-061 [low] Drive-or-share test written three times — CONFIRMED

**Status: Open.** The three predicates still exist: `crates/platform/src/dirs.rs:48` (`is_absolute_for`), `crates/platform/src/native.rs:216` (`form_of`), and `crates/platform/src/picker.rs:22` (`has_drive_or_share`). The planned replacement, `shell_form`, is not in the code. `docs/plans/MOUNT_POINTS_PLAN.md:90–97` plans it and keeps the volume mapping until the owner decides. The Windows check of the dialog is still owed, and the owner runs it.

## errors

### E-017 [medium] Per-item failure policy differs across harnesses and sweeps — CONFIRMED

**Status: Open.** The policies still differ, at the current lines:

- `eval/ann/run.py:343–350` records a failed configuration and continues.
- `eval/accel.py:142–144` skips a calibration image that will not load.
- `eval/bench_decode.py:141` reads each file with no try around it.
- `eval/pdq.py:46–53` returns None on any Exception.
- `eval/verify.py:579–580` records the pair as an error decision.
- `eval/jpeg-decoders/nvimgcodec_bench.py:87` catches Exception; `eval/jpeg-decoders/nvjpeg.py:285, 299, 304` catch RuntimeError.

Needed: narrow `nvimgcodec_bench`'s catches. That needs the exception a GPU run raises. The GPU run was not checked here.

### E-063 [low] CUDA error text formats differ across harnesses — CONFIRMED

**Status: Partly resolved.** The C and C++ macros now match: `eval/jpeg-decoders/harness/dec_gpu.cpp:18` (`#x " failed: "`) and `eval/jpeg-encoder/encode_gpu.cu:40` (`"%s failed: %s"`). The Python harnesses still differ. `eval/jpeg-decoders/gpu.py:45` and `nvimgcodec_bench.py:43` use `cuInit: CUDA error (code N)`, and `nvjpeg.py:110` uses `<what>: returned an error (code N)`. The texts the finding quoted are no longer in the code.

## versions

### V-030 [low] nvJPEG header version differs from the pinned wheel — CONFIRMED

**Status: Open.** The docstring now states the difference: `eval/jpeg-decoders/nvjpeg.py:6–7` says the declarations come from torchvision's DLL (header 13.2), not from the `nvidia-nvjpeg==13.3.2.43` wheel pinned in `eval/jpeg-decoders/requirements.txt:8`. The comparison itself was not done. The wheel is not installed in this checkout, and the CUDA 13.4 header on this machine reports 13.2.3.

Needed: diff the declarations against the wheel's header, on a machine that has the wheel, or run the GPU check.

## pins

### P-004 [medium] parajpeg git rev differs between production manifests and eval harness — CONFIRMED

**Status: Still true.**

- Product: `Cargo.toml:40` and `fuzz/Cargo.lock:353` pin darktorres/parajpeg at `8e2c53a`.
- Eval: `eval/jpeg-decoders/sources.py:19` pins anniigr/parajpeg at `f45081d`, and `linux_build.py` and `win_build.py` use that pin. `win-build.cmd`, which the old text cited, is gone.
- `docs/reference/JPEG_DECODERS.md:489` still says `f45081d`.

Needed: move the eval harness to the fork's rev, then re-measure.

### P-015 [medium] Python 3.12.10 literal in ci.yml, Justfile and STATIC_BUILD.md — CONFIRMED

**Status: Still true, guarded by a test.** `.github/workflows/ci.yml:69`, `Justfile:106` and `docs/reference/STATIC_BUILD.md:27` hold the value. The workflow literal is needed, because setup-python runs before `just` is installed. `test_the_workflow_python_is_the_justfile_pin` (`tools/test_ci_files.py:249`) asserts that the workflow matches the Justfile, and the tools suite passes.

### P-016 [medium] MSVC toolset literal duplicated in PowerShell scripts and STATIC_BUILD.md — CONFIRMED

**Status: Resolved, but the record was wrong.** No tracked PowerShell file sets the toolset. The `build-*.ps1` scripts the finding cited exist only under `target/shots`, which is build output. The test the old text named, `test_the_powershell_copies_agree_on_the_msvc_toolset`, does not exist in `tools/`, so nothing checks the toolset in PowerShell. Python reads `MSVC_TOOLSET` from `tools/pgtools.py:27`.

### P-024 [low] Apt packages are unversioned in three places, against a digest-pinned base image — CONFIRMED

**Status: Still true.** `docker/apt-packages.txt` lists unversioned packages. `tools/profiling/ci_setup.py:45` appends `mold` unversioned, and `crates/video/build.rs:50` (`APT_PACKAGES`) is unversioned. `tools/profiling/bench.py:81–82` still says the list is not pinned to a snapshot. The Dockerfile base is pinned by digest (`docker/Dockerfile:7`).

Needed: an apt snapshot date or exact versions. Choosing them needs network access.

### P-027 [low] Yandex User-Agent pinned to Chrome 108, a browser build from 2022 — CONFIRMED

**Status: Still true.** `crates/yandex/src/http.rs:14` sends `Chrome/108.0.0.0`.

Needed: a live Yandex check before the User-Agent is changed.

## tooling

### TL-007 [medium] eval/jpeg-decoders is a separate workspace that no CI step builds — CONFIRMED

**Status: Resolved in code, not run in CI.** `.github/workflows/ci.yml:341–342` runs `cargo check --manifest-path eval/jpeg-decoders/Cargo.toml --locked`. The commit that adds it (`ec85b503`) is on local `main` but not pushed: `origin/main` is `c92a28ca`, so no CI run has executed the step.

Needed: push, then check that the step passes.

## ui-text

### UI-018 [low] Window and dialog strings use different scripts and separators — CONFIRMED

**Status: Partly resolved.** The HUD (`crates/desktop/qml/Hud.qml:89`), FilterBar (`FilterBar.qml:193, 222, 341`) and SaveViewDialog (`SaveViewDialog.qml:58`) now use " · ". The two-space and three-space separators the finding cited are gone. Still different: `YandexPanel.qml:79` uses `" (" + count + ")"`. Glyph choices remain: ▴/▾ in `TopBar.qml:91`, ⚠ in `StatusBar.qml:100`, and × in `ResultTile.qml:92`.

Needed: a screenshot check, which was not run here.
