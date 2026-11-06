# CI speed: findings from the warm-run investigation

Companion to `CI_SPEED_PLAN.md`. Records what was found while investigating why `cxx-qt-lib` is rebuilt
on the Windows warm run, what the vendored `qt-build-utils` patches do, and what was validated. Written
as findings, not decisions: no option below has been chosen or implemented.

Status (2026-11-06): §§1-9 describe the tree before §10. The `vendor/` copies are gone (`vendor/` holds only
`README.md`), and the patched crates are pinned forks (§10). `ci.yml` now sets `CARGO_UNSTABLE_CHECKSUM_FRESHNESS`
and `CARGO_BUILD_FINGERPRINT: "content"` on the pinned nightly, and no longer sets `RUSTC_BOOTSTRAP` (commit 2b35a09).

## 1. Why cxx-qt-lib is not cached on the warm run

- `[patch.crates-io]` in the root `Cargo.toml` routes `qt-build-utils` to `vendor/qt-build-utils`, a path
  package (before the fork pin in §10). cxx-qt-lib 0.10.0 depends on it as a build-dependency, through cxx-qt-build (registry).
- Cargo's freshness check treats path packages by source mtime, compared against the dep-info outputs.
  Registry packages (crates.io) are not mtime-checked. A dirty path build-dependency marks every dependent
  dirty, so cxx-qt-lib's build script reruns and the crate rebuilds.
- Result: `cxx-qt-lib` (a registry crate, so "not our code") rebuilds because of a path package it
  depends on through its build script. The patched path is the cause, not the crate itself.
- Scratch confirmation: registry crates were not rebuilt on mtime changes; path crates were.

## 2. What is patched in vendored qt-build-utils

Three behaviours differ from upstream `qt-build-utils` 0.10.0. Vendor `Cargo.toml` is identical to upstream;
`LICENSE-APACHE` and `LICENSE-MIT` are extra. The patches:

1. **Ordering in `register_qml_module`.** Upstream runs qmlcachegen before qmltyperegistrar. The vendored
   copy runs qmltyperegistrar first, so the module's `plugin.qmltypes` exists when qmlcachegen reads it.
   Without this, qmlcachegen sees stale or missing types and falls back to the interpreter for new bindings.
2. **Foreign types.** The vendored copy passes Qt's own `*_metatypes.json` files (`C:/Qt/6.12.0/msvc2022_64/metatypes`,
   94 files) to qmltyperegistrar as `--foreign-types`. Without them, Qt QObject and value types are opaque:
   QVideoSink loses its properties and prototype; QVideoFrame loses the MapMode enum. On the real module,
   they also remove four qmllint `Member "length" not found on type QString/QStringList` errors (section 6).
3. **Pre-stage of QML copies.** The vendored copy copies the QML files beside the qmldir in
   `OUT_DIR/qt-build-utils/qml_modules/...`, so qmlcachegen can resolve them. Upstream does not.

`vendor/README.md` describes these, with a QString rationale. **Correction to an earlier conclusion:** a
probe (a small cxx-qt crate, not the real module) did not reproduce the `QString has no length` error, so I
had called the README wrong. The real module does show it: without foreign types, qmllint reports
`Member "length" not found on type "QString"` (Main.qml:22) and on `QStringList` (Main.qml:337, 417, 468).
With foreign types, the gate reports none. So the README's rationale is supported for the real app. The
probe was the wrong test. Its wording still needs a precise statement (which types, which tool). Not edited:
an owner decision, and another agent is working in the repo.

## 3. Options for the three behaviours (not chosen)

Investigated, none implemented:

1. Keep the vendored patches (status quo); accept the rebuild.
2. Upstream fix: none in cxx-qt 0.10.0 (checked the registry sources; no hook or flag for the ordering or
   foreign types).
3. Reproduce the three patches from `build.rs` with the registry `qt-build-utils` (no path crate, so no
   mtime rebuild). Evaluated; **only patch 3 is replicable**:
   - **Patch 3 (QML copies): replicable.** `build.rs` creates `OUT_DIR/qt-build-utils/qml_modules/pg/gallery/`
     and copies the QML files there before `CxxQtBuilder::build()`. The scratch probe (registry lib, PRESTAGE=1)
     built and generated the module (`res/U1`, `res/U2`, exit 0). Not verified in the real repo build, and the
     path layout is internal to qt-build-utils 0.10.0 (fine while pinned).
   - **Patch 1 (ordering): not replicable.** The registrar must run before qmlcachegen inside `build()`, and it
     needs the moc `*_metatypes.json` of the module's own types. Those exist only inside `build()`
     (`cxx-qt-build` `build_qml_modules`, `moc_cpp_files`), and are not left in `OUT_DIR` after the build
     (searched a real build: none). Nothing public hands them to `build.rs`.
   - **Patch 2 (foreign types): output replicable, input not.** Running `qmltyperegistrar --foreign-types`
     on the same metatypes gives the built `plugin.qmltypes` exactly (0 diff, section 6). But that needs the
     moc JSON from patch 1's blocker, so it can't be run after the build either.
   - **Consequence:** with the registry lib, patches 1 and 2 are lost. Patch 1's cost is a one-build lag in
     qmlcachegen's types; patch 2's cost is the 190 qmlcachegen and 8 qmllint warnings (section 6), so the
     gate would fail. Keeping 1 and 2 requires a path crate, which keeps the rebuild.
   - **Verdict:** option 3 does not remove the rebuild. It addresses only one of the three patches.
4. Old-mtime workaround: set the vendored path crates' mtimes to an old fixed time so cargo sees them clean.
   This stops the rebuild, but a content change under an old mtime is silently ignored. Guarding it needs
   a `cache-hit` check; the guard has **not** been validated in CI.
5. Not evaluated in depth.
6. **Patch source experiments** (scratch, `scratchpad/src-exp`, stand-in crate `itoa` as a build-dependency
   patched by `[patch.crates-io]`; "touch" = mtimes bumped, content unchanged; "content" = a real edit):

   | Patch source | Touch only | Content change | Verdict |
   |---|---|---|---|
   | path (today) | rebuilds | rebuilds | the CI problem |
   | git, pinned `rev` (file:// repo, package in a subdirectory) | fresh | rebuilds (new rev: "the path to the source changed") | **candidate** |
   | directory source (`cargo vendor`, all crates-io replaced) | fresh | **fresh: stale binary** (no checksum error when the checksum json is not updated) | hazard; needs a cache key that hashes the vendored tree |
   | path, old fixed mtime (2000-01-01) | fresh | **fresh: stale binary** | hazard (confirmed) |
   | path, old mtime, then a newer mtime, same content | rebuilds | — | the mtime-only fix is unsafe |

   Candidate detail: a git dependency pinned to a commit that contains `vendor/qt-build-utils` (the repo
   itself, or a fork). Cargo fingerprints git sources by rev, not mtime, so a content change needs a new rev
   (`cargo update -p` to move it) and an unchanged rev never rebuilds. Not tested: the GitHub fetch in CI, and
   the real cxx-qt chain with it (the test used `itoa` with `file://` git). Cost: the patched code is
   committed, pushed and pinned by rev, so each change is a two-commit workflow.

sccache caches registry crate rustc invocations by content (verified with serde_json), but not path crates
(verified in scratch). Its `cc` C++ path is **not** tested.

## 4. CI findings

- **The warm test has been run, on an exact cache match.** Run 42120785368 (commit 23de045) was rerun as
  attempt 2 (rust-cache: `Restored from cache key "v0-rust-windows-Windows_NT-x64-bc11f7b7-47886e10" full match: true`).
  It still recompiled **56 units**, the same as the partial-restore attempt: all 13 patched crates, `cxx-qt`,
  `cxx-qt-build`, `cxx-qt-lib`, `image`, `turbojpeg`, and all 28 `pg-*` crates. An exact cache hit does not avoid
  the rebuild: cargo treats the checkout's mtimes as newer than the cached dep-info (section 8, section 9).
  Earlier runs 42104315625 (3c4479e) and 42115356080 (7d95160) were cold (318 units each). Logs are in `$TMPDIR`
  (`job-127578982177.log` is attempt 1 of 42120785368; the rerun is `rerun-job.log`).
- The `CARGO_LOG=cargo::core::compiler::fingerprint=info` diagnostic (plan item 4) prints no dirty reasons in
  cargo 1.99.1. `cargo build -v` prints `Dirty ... the file ... has changed`, and should replace it.
- The Cargo.toml change that added workspace lints (commit between 3c4479e and 7d95160) changes the
  Windows rust-cache key, so the previous Windows cache does not apply.
- The 23de045 run's cache exists as `bc11f7b7`. A rerun on that same commit would be the warm test; pushing
  or rerunning CI needs the user's go-ahead (plan item 1).

## 5. QML lint gate: it works under PowerShell

`tools/lint-qml.ps1` (`just lint-qml`, run by `just lint` in the Windows CI job, under pwsh) is **correct**
on the current tree. An earlier version of this file said it was blind to qmlcachegen's warnings under
PowerShell. That was wrong; the cause was my own bash harness.

**Cause of the earlier "28 warnings" and "PowerShell is blind" results:** Git Bash rewrites command-line
arguments that look like Unix paths. `--resource-path /qt/qml/pg/gallery/qml/GridRow.qml` became
`C:/Program Files/Git/qt/qml/...` (visible as the first line of the generated `.cpp`:
`// C:/Program Files/Git/qt/qml/pg/gallery/qml/GridRow.qml`). qmlcachegen then could not resolve the module,
and reported `GridCell was not found` and related warnings. PowerShell does not rewrite arguments, so the
gate's call is correct.

Evidence:

- With `MSYS_NO_PATHCONV=1` (Git Bash path conversion disabled), bash qmlcachegen on all 28 files gives 0
  warnings. The generated `.cpp` header is `// /qt/qml/pg/gallery/qml/GridRow.qml`.
- The gate under PowerShell gives `QML: no warnings; all 1113 bindings and functions compiled to C++ (28 files).`
  and exits 0. PowerShell's qmlcachegen stderr is 0 bytes, which is correct for a clean compile.
- The gate fails on a real error. A scratch copy of the gate with one added `crates/desktop/qml/Canary.qml`
  (an undefined type `NoSuchCanaryType { }`) prints
  `crates/desktop/qml/Canary.qml:4:5: NoSuchCanaryType was not found. Did you add all imports and dependencies? [import]`
  and exits 1: `1 QML warning(s); 1113 of 1113 bindings and functions compiled to C++.`
- The qmlcachegen warning text is printed only with `--verbose`; the gate passes it.
- Environment variables are identical in bash and PowerShell. Qt's stderr-forcing switches
  (`QT_FORCE_STDERR_LOGGING`, `QT_ASSUME_STDERR_HAS_CONSOLE`) had no effect, and were not needed.

Unexplained, not affecting the gate: `qmlcachegen --version` and `qmllint --version` printed nothing when run
unredirected from PowerShell, though redirected to a file they print. The gate's captured calls (qmllint JSON
via pipe, qmlcachegen via stderr redirect) work.

Not verified: that the Windows CI job gives the same result (CI has not run on this tree). The gate's logic
is the same there.

## 6. Foreign types effect on the QML lint (corrected, real module)

Measured with the module built without foreign types (`scratchpad/lint-without`, 28 files, path conversion
disabled). Compare with the gate, which uses the module with foreign types:

| | with foreign types (gate) | without foreign types |
|---|---|---|
| qmllint warnings | 0 | 8 |
| qmlcachegen warnings | 0 | 190 |

The 8 qmllint warnings: four `Member "length" not found on type QString/QStringList` (Main.qml), two
`Cannot assign binding of type QStringList to QList<QString>` (TestHooks.qml), one of the same kind
(Viewer.qml:37), and `Cannot assign binding of type QString to QUrl` (Viewer.qml:378).
The 190 qmlcachegen warnings are mostly `Cannot generate efficient code for internal conversion with
incompatible or ambiguous types` (QString/double/bool stored as QVariant, in FilterBar, TopBar, and others),
plus `Retrieving the metatype of an unregistered type` and `calling method with argument affected by side effects`.

The earlier 28 and 220 figures were wrong for the same path-conversion reason.

The regenerated `plugin.qmltypes` with foreign types matches the built one exactly (0 diff).

## 7. Still open

- Whether the 28 qmlcachegen warnings the gate does not produce are relevant: they are not (section 5);
  the earlier "28 warnings" figure was the path bug. The app currently has 0 QML warnings on the gate.
- Whether the Windows CI job agrees with the local gate result (not run).
- Old-mtime plus `cache-hit` guard, end to end in CI.
- sccache with the `cc` C++ path.
- PRESTAGE in the real repo build (scratch only so far).
- Correcting the wording of `vendor/README.md`'s QString rationale (section 2).
- A true warm CI run (section 4).
- Harness caveat: any scratch harness that passes `/...` arguments from Git Bash needs `MSYS_NO_PATHCONV=1`.
  The earlier scratch outputs (`lint-built`, `lint-with`, `lint-without`, `regtest`) were produced without it,
  so their `--resource-path` results are not valid.

## 8. Forking the patched crates

The rebuild is not specific to `qt-build-utils`. The partial-restore run (42120785368, attempt 1) recompiled 46
units. Of them:

- 13 are the patched crates themselves (fast_image_resize, fdeflate, gif, image-webp, png, qt-build-utils,
  quick-xml, rusqlite, thumbhash, tiff, turbojpeg-sys, zune-core, zune-jpeg).
- 5 are their dependents: cxx-qt, cxx-qt-build, cxx-qt-lib, image, turbojpeg.
- 28 are the workspace's own `pg-*` crates.

So a fork removes 18 of the 46 rebuilds. The 28 workspace crates would still rebuild, since they are path packages
too (checkout mtimes are newer than the cached dep-info). That part is a separate problem.

The patches are almost all allocation-reuse changes (`README.md` says every pixel, event and error is upstream's),
plus a few fixes or build tweaks (rusqlite's statement cache key; turbojpeg-sys's `WITH_CRT_DLL`; qt-build-utils).
The sizes of the local changes against upstream at the vendored version, ignoring whitespace: fdeflate 124 lines,
rusqlite 31, turbojpeg-sys 3, qt-build-utils 82, zune-core 129, zune-jpeg 315, quick-xml 202, thumbhash 215,
image-webp 1067, fast_image_resize 769, png 476, gif 912, tiff 1594.

Option: a git dependency pinned to a rev for each (all 13 in one repo: one rev, cargo finds the packages by
subdirectory). Tested in scratch on the stand-in crate: touching the checkout does not rebuild; a new rev
rebuilds; a repo with a root `[workspace]` and an excluded `vendor/` works.

Trade-offs:

- For: removes the third-party rebuilds (18 of 46 here) without a hazard; each change is an explicit rev.
- Against: each vendor change is commit, push, rev bump (two commits); the fork must be kept up to date by hand
  (or dropped where upstream has the change); a git fetch in CI; the real cxx-qt chain is not tested with it.
- Some patches could go upstream, and the rusqlite and turbojpeg-sys ones may already be fixed in a newer release.
  Not checked (no network lookup this pass).
- It does not touch the 28 workspace crates. Those need a separate answer (section 9).

## 9. The 28 workspace crates

Cause: the workspace's `pg-*` crates are path packages. Cargo marks a path package dirty when a source's mtime is
newer than its cached dep-info, and a fresh checkout is always newer. An exact cache hit does not change this
(section 4).

Options examined:

1. **`cargo -Z checksum-freshness`** (unstable; compares checksums, not mtimes, for path packages). Tested on the
   path-patched stand-in crate (scratch `src-exp/P`):

   | Change | Result |
   |---|---|
   | none | fresh |
   | touch (mtimes bumped, content unchanged) | fresh (the mtime rule is off) |
   | content edit | dirty: "file size changed ... the dependency `itoa` was rebuilt" |

   It is the only option tested so far that avoids the mtime rebuild for path packages, workspace crates included,
   with no hazard. It also suggests the forks in section 8 are unnecessary for the 18, if the flag covers the real
   chain. **Not tested on the real chain** (cxx-qt build scripts, `rerun-if-changed`, QML staging).
   Needs nightly cargo (the local `nightly` toolchain works; `RUSTC_BOOTSTRAP=1` on stable also works in principle):
   on stable, `-Z` is rejected. The CI job would then use a nightly toolchain or set the variable, and the flag can
   change or be removed with a nightly.

2. **Set source mtimes to their last commit time after checkout.** Not tested here. Hazard: correctness depends on
   commit times being later than the build times they follow. A rebase, an amend or a clock skew can give a
   changed file an old mtime, which cargo treats as fresh (the same failure as section 8's old-mtime test).

3. **sccache for the workspace crates.** The earlier scratch test (section 3) found that sccache did not cache path
   crates, so this does not remove the rebuild, though it could make it fast if it hits. Not re-tested.

4. **Measure before deciding.** The rebuild cost of the 28 workspace crates is not measured. Most of the time is in
   the third-party crates. `cargo build --timings` on the warm run would show whether 28 `pg-*` crates matter.

**Result of option 1 on the real chain** (scratch copy `D:/pgx/repo`, target `D:/pgx/target`, 1.99.1 with
`RUSTC_BOOTSTRAP=1 -Z checksum-freshness`; cold build of 212 units, exit 0). A short path was needed: under
`scratchpad\` the CMake FileTracker failed (FTK1011, path over the Windows limit), which is a scratch-setup
artifact, not a CI one. Logs in `D:/pgx/t1.log`, `t2.log`, `t3.log`.

| Test | Units rebuilt | What it shows |
|---|---|---|
| T1: touch every file, no content change | 19 | The 13 patched crates, `cxx-qt`, `cxx-qt-build`, `cxx-qt-lib`, `qt-build-utils`, `image`, `png` are **not** rebuilt. Only `turbojpeg-sys` among the third-party crates (below) |
| T2: edit `crates/cli/src/borders.rs` (leaf) | 2 | `pg-cli`, `pg-gallery`: exact |
| T3: edit `vendor/png/src/lib.rs` (patched crate) | 19 | `png`, `image`, and the `pg-*` crates that depend on them: exact |

The residual 19 in T1 are all mtime checks, which the flag does not cover:
The reasons are cargo's own, from `cargo build -v` (each `Dirty` line names the file or the crate):

- `turbojpeg-sys`: its build script declares two mtime-checked inputs in `rerun-if-changed`: `build.rs` (line 10)
  and `bindings.rs` (line 251). With both lines present, cargo reports "the file `vendor\turbojpeg-sys\build.rs` has
  changed". With the `build.rs` line removed (test C), it reports `bindings.rs`. Removing one line leaves the other
  trigger, so both lines must go for the script to stay fresh. The script runs cmake, which is expensive. Cascades to
  `turbojpeg`, `pg-codec`, `pg-video`, and the `pg-*` crates above them.
- `pg-desktop`: "the file `crates\desktop\src/animation.rs` has changed". The desktop build script declares no inputs,
  so this is the dependency-info mtime rule, not a build-script rerun.
- `pg-gallery`: "the file `crates\gallery\resources/app.ico` has changed", from its `rerun-if-changed` line.
  Both cascade to the other `pg-*` crates.
- Cargo's fingerprint log (`CARGO_LOG`, set in `ci.yml`; the target is `cargo::compiler::fingerprint=info` since
  commit 2b35a09, before it was `cargo::core::compiler::fingerprint=info`) prints the same reasons on CI.

So the flag does what the forks in section 8 would have done for the 12 patched crates that don't rerun a build
script (13 minus `turbojpeg-sys`), without forking. It does not remove the rebuilds caused by `rerun-if-changed` and dependency-info mtime
checks, which are cargo's own mtime checks, not part of the flag. Those 19 units are what a warm CI run would still
rebuild with the flag. Not tested: CI itself (RUSTC_BOOTSTRAP on the Windows job, the rust-cache restore), and
`turbojpeg-sys` with its own build script removed from the rebuild.

**Scratch tests of the remaining 19** (scratch copy `D:/pgx/repo`, same flag; logs `D:/pgx/stamp-run3.txt`,
`D:/pgx/stamp-a.txt`, `D:/pgx/A-touch*.log`, `D:/pgx/B-touch.log`). Each touch test follows a warm build.

| Test | Touch-all result | Notes |
|---|---|---|
| Baseline (T1 as above) | 19 units, 3 build-script runs | |
| A: `turbojpeg-sys` build script skips `cmake.build()` when a hash of `libjpeg-turbo/` and `build.rs` matches the last build (stamp in `OUT_DIR`) | 19 units, 3 build-script runs; stamp hit in both touch runs; cmake did not run | The cmake work is removed, but the cascade is not. The rerun itself dirties the dependents, so skipping the work does not stop it |
| B: remove `cargo:rerun-if-changed=resources/app.ico` from `crates/gallery/build.rs` | 19 units, 3 build-script runs | No change. Without the line, cargo falls back to rerunning the script when any file in the package changes, so the touch test still reruns it |

Conclusions:

- The cost is the rerun of the build scripts and the propagation to dependents, not the cmake work. A content-hash
  skip inside a build script does not help, because cargo has already marked the dependents dirty.
- Removing the `app.ico` line does not help under a touch. The two build scripts (`pg-desktop`, `pg-gallery`) rerun
  on any package change either way.
- The two build scripts would have to stop rerunning on mtime alone: the only real change is an edit to the
  listed inputs, and cargo's mtime check cannot tell a touch from an edit. Not tested: a `turbojpeg-sys` run without its
  `rerun-if-changed=build.rs` line, which is the remaining candidate for the `turbojpeg-sys` part.
- **Removing `rerun-if-changed=bindings.rs` (and `build.rs`) makes a content edit silently stale.** Test I: with both
  turbojpeg-sys lines removed, appending a line to `bindings.rs` rebuilt nothing (0 units, 0 build-script runs). The build
  script copies `bindings.rs` into `OUT_DIR` at runtime, so the copy stays old. Why: with no `rerun-if-changed`, the
  build script's fingerprint records no file inputs, only env vars, and its dep-info lists only `build.rs` (what rustc
  compiled). `bindings.rs` is read at runtime, so cargo never sees it. Keep the `bindings.rs` line.
- Removing both turbojpeg-sys lines does cut the touch-all rebuild from 19 units to 2 (test D). It is not safe.
- Test J (a byte flipped in `app.ico` with its line removed) rebuilt `pg-gallery`, but the reason printed was
  "precalculated components changed", not the file. That does not show `app.ico` is tracked, so it is unresolved.
  Keep the `app.ico` line.
- So the `turbojpeg-sys` cascade (17 of 19 units) stays until its rerun trigger can be replaced with something
  that is both content-checked and not mtime-based. Not found yet.

Recommendation: the flag is worth adopting for the patches (verified in T1 and T3), so the forks in section 8 would
probably not be needed. The 19 residuals remain the warm-run cost with the flag. Before adopting it in CI, check them on the CI
runner, since they set the warm-run cost. The stamp idea (A) is not worth carrying into the repo.

## 10. Outcome: the patched crates are git pins on our forks

The recommendation in §9 is superseded. The 13 patched crates, and dhat, are now git dependencies pinned by `rev` to
`github.com/darktorres/*` forks, one per upstream repo. Each pinned commit holds the vendored tree exactly, minus the
four packaging files (`Cargo.toml.orig`, `.cargo-ok`, `.cargo_vcs_info.json`, `Cargo.lock`). The `vendor/` directories
are gone; `vendor/README.md` stays as the rationale for each local change.

Pins (see the root `Cargo.toml`, `crates/gallery/Cargo.toml`, `fuzz/Cargo.toml`, `eval/jpeg-decoders/Cargo.toml`): fast_image_resize,
fdeflate, image-gif (`gif`), image-webp, image-png (`png`), image-tiff (`tiff`), quick-xml, rusqlite, thumbhash,
cxx-qt (`qt-build-utils`), rust-turbojpeg (`turbojpeg-sys`), zune-image (`zune-core`, `zune-jpeg`), dhat-rs.

Why it works, from two checks:

- Cargo does not mtime-check git packages the way it checks path packages. Touching the sources of `png`, `quick-xml`,
  `qt-build-utils` and `zune-core` in `~/.cargo/git/checkouts` left all four `Fresh`, and a no-op build took 0.7 s.
- rust-cache keeps the artifacts and checkouts of packages outside the workspace root. Its save step (`save.ts` and
  `cleanup.ts` at `d1527c8`) prunes only the workspace-root packages, which is where the `vendor/` path crates were.

Validated locally: each pinned tree matches the vendored files, file by file; `cargo build` (dev) passed, with a one-time
rebuild of 4m13s; `cargo check -p pg-gallery --features dhat-heap` passes; `cargo test --workspace` with the Qt bin
directory on `PATH` ran 74 suites, 463 passed, 0 failed (two catalog tests skipped without `PG_TEST_CATALOG`);
`cargo deny check sources` is ok, with one warning, that `dhat-rs` in `allow-git` matches no crate under the default
features.

Not validated:

- The Windows CI warm run after the push. The expectation is no cxx-qt rebuild, but it has not been measured.
- Whether the rust-cache restore keeps the mtimes of `~/.cargo/git`. The turbojpeg-sys build script still reruns on the
  mtimes of `bindings.rs` and its include paths in its checkout. If the restore gives those files new mtimes, that crate's
  cascade comes back. Check the CI log for `Dirty turbojpeg-sys`.
- Release build and clippy after the migration.

The turbojpeg-sys fork's content-hash trigger (`TURBOJPEG_SYS_INPUTS_HASH`, `tools/content-hash.ps1`, the Justfile's
`inputs`) is removed. A git pin has no local files to hash, so the hash had nothing left to watch.
