# CI speed: plan and open items

Working list for making `.github/workflows/ci.yml` faster, from the Windows job (Qt app) and the Linux job. Items are
checked off as they are done. Nothing here is committed or pushed until the owner says so.

## Measured

| Item | Time | Source |
|---|---|---|
| Windows job, total | 74m37s | run 42087043728, before the deploy step was removed |
| Linux job, total | 8m25s | latest run (18:43:56 to 18:52:21 UTC) |
| Lint step | 23m03s | the same Windows run |
| Clippy (`cargo clippy --workspace --all-targets -- -D warnings`) | 17m27s | saved log |
| `cargo doc` (`RUSTDOCFLAGS=-D warnings`, `--no-deps --document-private-items`) | 5m02s | saved log |
| Tests step | 20m39s | the same Windows run; mixes compiling and running |
| Post rust-cache save | 3m42s | the same Windows run |
| Windows job timeout | 90 min | `timeout-minutes` in `ci.yml` |

Inside clippy, the first ~1m40s are normal checking of the vendored and workspace crates. Then ~15m50s pass with no
output, starting after the `pg-bench` check line at 18:50:24 UTC.

Inside `cargo doc`, the first ~4m are normal. Then ~4m pass with no output, from `pg-container` at 19:06:59 until
`pg-desktop` starts at 19:11:13 UTC.

Both silent stretches come just before `pg-desktop` and `pg-gallery`. The clippy log shows "Compiling pg-desktop" and
"Compiling pg-gallery" (their build scripts) at 18:50:09 and 18:50:10. Cargo doesn't print build-script runs, so the
stretches are most likely those build scripts (cxx-qt and Qt). Not confirmed.

Lint's other commands are small: fmt 2s, the wasm32 check 2s, `cargo deny` about 4s, `cargo machete` about 2s, ruff
about 1s, markdownlint about 6s, `lint-qml` about 14s.

The screenshot steps (PR only) run `cargo build -p pg-gallery` in dev (`tools/compare-shots.ps1`, line 101).

## Hypotheses, not yet measured

- H1. The `pg-desktop` and `pg-gallery` build scripts (cxx-qt, Qt) rerun on every run, in clippy and in `cargo doc`.
- H2. The 13 vendored path crates (the `[patch.crates-io]` entries, then under `vendor/`; now git pins, item 13) are marked dirty on every run, because
  a fresh checkout gives their sources newer mtimes than the cached artifacts. This is cargo's mtime rule, reproduced in
  a scratch project. Whether it happens in CI is not confirmed. `turbojpeg-sys` is a C build script, so it matters.
- H3. Dependencies compile twice in the same job: as metadata for clippy and `cargo doc`, and fully for the tests.
  `[profile.dev.package."*"] opt-level = 3` makes the test build's dependency pass slow.
- H4. `cargo doc` rechecks `image` and `fast_image_resize` even though clippy just checked them (seen in the log).
- H5. Lint and Windows lint duplicate the Linux job: `cargo deny`, `cargo machete`, ruff and markdownlint run in both.

## Diagnostics (temporary; in the committed tree on main, pushed status not checked here)

- `justfile`, `lint` recipe: `cargo clippy --workspace --all-targets --timings -- -D warnings`. Writes
  `target/cargo-timings/`.
- `.github/workflows/ci.yml`, Windows job `env`: `CARGO_LOG: cargo::compiler::fingerprint=info` (narrowed from
  `cargo::core::compiler::fingerprint=info` in commit 2b35a09). Logs why each unit is dirty.
- `.github/workflows/ci.yml`, a step after Lint: uploads `target/cargo-timings/` as the `cargo-timings` artifact, with
  `if: always()`.

Remove all three once the diagnosis is done (item 19).

## Action list

### 1. Diagnose (needs a push, which needs the owner's go-ahead)

- [ ] 1. Push the diagnostic changes above.
- [ ] 2. Run twice on the same `Cargo.lock`: the first run is cold, the second warm. Check the rust-cache restore line in
  each for a hit or a miss.
- [ ] 3. Open the `cargo-timings` artifact. Read the build-script durations, especially `pg-desktop`, `pg-gallery`,
  `turbojpeg-sys` and `rusqlite`. Settles H1.
- [ ] 4. Grep the fingerprint log for `Dirty`. Shows which units rebuild every run and why. Settles H2 and H4.
- [ ] 5. Read the test step's compile and run times (needs item 8 first, or the per-step times from the run).

### 2. Remove duplicated work (no measurement needed)

- [ ] 6. Windows lint: drop `cargo deny`, `cargo machete`, ruff and markdownlint, which the Linux job runs. Drop the ruff
  and markdownlint install from the Windows job. About 1 minute. Settles H5.
- [ ] 7. Keep the lock-file check in one job only. It costs 22s on Windows and the Linux job runs it too.

### 3. Restructure the Windows build (after item 1)

- [ ] 8. Split the test step into `cargo test --workspace --no-run` and a run step, so the compile time and the test time
  show separately.
- [ ] 9. Merge clippy into the test build: `RUSTC_WORKSPACE_WRAPPER=clippy-driver` with `CLIPPY_ARGS="-D warnings"`. One
  dependency pass in the same job. `CLIPPY_ARGS` is in `clippy-driver.exe` (found by a string match), but the merged
  build hasn't been run. A lint failure would then show in the test-build step. Interacts with item 11.
- [ ] 10. Alternative to item 9: parallel jobs (lint, test, PR-only screenshots) sharing one cache with `shared-key`.
  Wall time becomes the longest job rather than the sum. Each job pays the Qt install (1m38s) and a cache restore.
  Standard runners are believed free for public repos; not confirmed. Items 9 and 10 can be combined.
- [ ] 11. Decide where `cargo doc` runs. Either move it to Linux for the non-Qt crates
  (`--exclude pg-desktop --exclude pg-gallery`), so the Qt crates' docs go unchecked, or keep it on Windows.

### 4. Attack the rebuilds (after item 1)

- [ ] 12. If the `pg-desktop` and `pg-gallery` build scripts rerun every time (H1): check their `rerun-if-changed` lines in
  the `qt-build-utils` fork (`darktorres/cxx-qt`, pinned in the root `Cargo.toml`) and cxx-qt, and narrow them so unrelated files don't trigger a rerun.
- [x] 13. H2 is resolved by pinning the crates to our forks (FINDINGS §10): cargo does not mtime-check git packages, so
  the vendor-file mtime hack is not needed.
- [ ] 14. If the `cargo doc` recheck is a real invalidation (H4): find its cause in the fingerprint log.

### 5. Cache and profile tuning

- [ ] 15. Set `CARGO_PROFILE_DEV_DEBUG=line-tables-only` in the Windows job. Expect a smaller `target/`, a faster cache
  save (3m42s now) and restore, and faster linking. Measure.
- [ ] 16. Save the rust-cache only on `main` for PRs, with `save-if`. Removes the save from PR runs. PR runs still restore
  `main`'s cache.
- [ ] 17. Decide on `[profile.dev.package."*"] opt-level = 3` after item 8. If opt-level 0 for dependencies cuts the test
  build more than it adds to the test run, it's a win. Check that build scripts aren't affected.

### 6. Linux job (not the bottleneck: 8m25s)

- [ ] 18. Key the gungraun-runner cache on the gungraun version, not on the whole `Cargo.lock`. A dependency change
  currently invalidates it, which costs about a minute.

### 7. Guardrails and verification

- [ ] 19. Remove the diagnostics (`--timings`, `CARGO_LOG`, the artifact step) once the diagnosis is done.
- [ ] 20. Measure each change with two runs and compare per-step times from `gh run view --json jobs`.
- [ ] 21. The 52-minute Windows projection came from the old run, which still had the deploy step. The first run of
  the current workflow gives the real numbers.
- [ ] 22. The 90-minute timeout is close to the 74-minute run. Raise it only if a cold run still needs more after the
  changes.

## Out of scope here

This file covers CI speed only. Other open items from the public-release work are not listed here: the owner decisions
(license, the Yandex client, whether `docs/REVIEW.md` ships).
