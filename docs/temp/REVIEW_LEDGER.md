# Review ledger: the 2026-11-08 review of the plan work

A ledger of the findings from the review of commits `b700665..HEAD` against the plan
(`C:\Users\Torres\.claude\plans\lets-fix-all-the-woolly-allen.md`) and the findings in `docs/CODE_REVIEW.md` (deleted in
2cfe50c). The review was read-only: nothing it lists had been fixed when it was written.

Each finding was checked by a reviewer for its area, then verified in batches: one refuter tried to disprove it, and one
judge set its severity and whether the plan covers it. A finding is listed only if it survived both checks. The
severities are the judge's. Four high findings were spot-checked by hand before this ledger was written. Three held. The
C-51 one is documentation drift, so its severity is closer to low than high; it is kept at the judge's level, marked.

On 2026-11-12 every remaining row was checked against the code and docs at HEAD `898b709` and the working tree. Rows
that were fully done, or whose subject no longer exists, were removed. A row stays while any part of its proper fix is
missing. Rows that name `docs/CODE_REVIEW.md` are kept only where the open work is still real; that file's text is gone.

## Summary

| Severity | Findings | Notes |
|---|---:|---|
| High | 1 | |
| Medium | 29 | |
| Low | 11 | |
| Total | 41 | Plus the pending and waiting items below |

Kinds: pending 12, stale-doc 8, undeclared-deviation 8, untested 6, incorrect 5, shortcut 2.

Kind legend: `incorrect` is wrong behaviour; `shortcut` is a fix that is present but not the proper one; `missing`
is a fix that is absent; `untested` is a fix without a test that would catch its regression; `undeclared-deviation`
is code that differs from the plan without a recorded reason; `pending` is work not done or waiting on a person;
`stale-doc` is a document or comment that now says something false; `regression-risk` is a change that could break
something else.

## High

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| L-02 | C-36 | incorrect | `crates/catalog/src/writes.rs:227-235` | `record_temp` runs an `UPDATE` and returns `Ok(())` without checking the row count. If the job's row is deleted between its read and `record_temp`, the update does nothing, but the caller (`writeback/src/lib.rs:480-485`) still writes the file. The file then gets a rating or turn the user cancelled. For a rating, `file_ratings` takes the copied value while the catalog keeps the reverted one, so three-way sync does not resolve it. A process death during the copy leaves the original part-written with no record. `set_rating` (`writes.rs:117-121`) and `queue_rotation` (`writes.rs:143-146`) delete fresh rows, so the window is reachable. The plan has no test for it. | Make `record_temp` return an error or a bool when zero rows change. On that result, `process` removes its own temp, leaves the original untouched, and returns Ok. Add a test that changes the job between the read and `record_temp` (through the fault hook), and asserts the file is unchanged, the row is gone, and the catalog and `file_ratings` agree. | Partly fixed (committed in 905561d): `record_write_temp` (`catalog/src/queue.rs:211`) returns a bool and `writeback/src/lib.rs:579-592` removes its temp and leaves the original. Still open: the test (`writeback.rs:829`) covers turns only, and nothing asserts that the catalog and `file_ratings` agree for a cancelled rating |

## Medium

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| L-05 | C-27 | undeclared-deviation | `crates/jobs/src/pipeline.rs:124` | On `Ok(false)` the code passes the turn and sends `Ok(Lease::empty())`. The plan (lines 85-86) says `Err`. The code is the correct behaviour, since `import/src/lib.rs:93-98` returns false for every video. | Record the deviation in the plan and correct the plan sentence. | Open |
| L-06 | C-27 | untested | `crates/client/src/text.rs:1233` | `file_panicked` and `round_panicked` have no exact-string test; other messages in `text.rs` do. | Add `assert_eq!` tests for `file_panicked("boom")` and `round_panicked("boom")` against the English text in `locales/en/text.ftl:113-115`. | Open |
| L-07 | C-27 | untested | `crates/library/src/borders.rs:43` | No test covers the plan's per-file effects: import `store_failure`, borders "no borders" (`borders.rs:47-52`), crop's failed item not requeued (`crop.rs:159`). Only the pipeline, thumbs and writer panic paths are tested. | Add a `cfg(test)` seam that makes analysis panic for one file. Assert each effect, and that a second pass does not list the file. | Open |
| L-08 | C-27 | stale-doc | `docs/temp/REVIEW.md:28` and `:68` | R-73 and R-111 are still listed as open, but their fixes (C-24, C-27, C-28) are in the tree (`pipeline.rs:163`, `control.rs:233-242`). The header says "51 findings are open". The rule at line 11 says completed findings are removed. | Remove both rows, recount the header, and keep the rule. | Open |
| L-09 | C-24 | pending | `crates/desktop/src/library.rs:330` | Nothing under `crates/desktop/src` calls `Writer::is_running`. `refresh_writes` (lines 331-347) shows the queue count and the latest error only. An uncaught writer-thread panic triggers the hook's message box (`desktop/src/panic_log.rs:23-33`), but nothing after it says the writer has stopped. | Check `is_running` in `refresh_writes` and show a writer-stopped notice, or restart the writer. If neither is done, record the app-side behaviour as an open item in the C-24 row. | Open |
| L-12 | C-36 | undeclared-deviation | `crates/platform/src/file.rs:62` | `open_for_copy` requests `GENERIC_WRITE \| FILE_READ_ATTRIBUTES`. The plan says `GENERIC_WRITE`, `FILE_SHARE_READ` and `OPEN_EXISTING` only. The doc comment at `file.rs:49-50` explains the extra right (`File::metadata` on the handle). | Keep the right, and record it as a deviation in the plan. | Open |
| L-14 | C-36 | pending | `crates/library/tests/writeback.rs:305` | The skip paths at lines 316, 411, 445 and 585 (tests at 305, 401, 439, 574) return early and pass when `hold()` returns None (`crates/testkit/src/held.rs:31-59`, on Unix as root). `pg_testkit::skip` only prints. The container now runs as `pg` (`docker/Dockerfile:70`), but the root-skip tests still pass silently in a root run, and no Windows run is recorded. See L-60. | Record which of these ran on Windows, with results. Have the container run fail or report on `skipped:`, as `docker/check.py` now does. | Open |
| L-15 | C-36 | pending | `crates/library/src/writeback/file.rs` | The C-37 residual says a crash leaves a bare `.pg-write` that "the next write of that file clears". The current names are `.rating.pg-write` and `.turns.pg-write` (`writeback/src/file.rs:45-46`). The next write removes only its own kind's temp. Leftovers under the old bare name from pre-fix builds are never removed. | Decide with the user whether to add a one-time removal of `<name>.pg-write` at writer start, or list it as a residual. | Open |
| L-18 | C-26 | undeclared-deviation | `crates/edit/src/rewrite.rs:486-487` | A still GIF whose only frame has no pixels, or lies off-screen, makes the turn or crop return an error. Multi-frame GIFs return `Ok(None)` and become `Outcome::Catalog`. Turns now map `NothingLeft` to `Outcome::Catalog` (`edit/src/rotate.rs:73`), but crop still propagates it as an error (`edit/src/crop.rs:52-55`). The deviation is not recorded. | Either map crop's `NothingLeft` to a distinct typed outcome, or record in the plan that a still GIF with nothing left fails with an error. | Open |
| L-21 | C-41 | pending | `crates/platform/src/native.rs:262-263` and `:321-323`; `crates/library/tests/writeback.rs:471-473` | The verbatim tests are `#[cfg(windows)]`-gated, so the Linux container skips them. The commit's "Not verified" list omits them, and no Windows run is recorded. | Run `cargo test -p pg-platform` and `cargo test -p pg-library --test writeback` natively on Windows, and record the results. | Open |
| L-27 | C-58 | stale-doc | `docs/temp/PLAN.md:653` | The plan line says "the All count is unchanged". `model/src/tree.rs:330-333` `total()` now adds `root_files`, and `desktop/src/folders.rs:206` uses it for the All row, so the count changes. The code is correct. The `CODE_REVIEW.md` entry that said the same is gone. | Reword the plan line: All now includes the drive roots' own files. | Open |
| L-28 | C-58 | stale-doc | `crates/app/src/reader.rs:245`; `crates/catalog/src/browse/fixtures.rs:87` | The two doc comments are corrected. The contradiction with `docs/TYPE_MISMATCH_VALIDATION.md:182-184` is still not tracked in any open item. | Add the item to the plan. | Open |
| L-29 | C-58 | undeclared-deviation | `crates/client/src/controller/listing.rs:47` | `OpenAll` always sets `recursive = true`. The plan (line 379) says it does so only when the Drive-roots row exists. No deviation is recorded. The code's choice is the sounder one: `("", false)` with no root files would be an empty All row. | Record the behaviour and the reason in the plan. Keep the code. | Open |
| L-30 | C-58 | pending | `crates/desktop/src/folders.rs:206-207` | The Drive-roots row has not been seen in the running Qt app. The commit records "the Qt GUI beyond qmltest" as not verified. | Run the desktop app on a catalog with root-level files. Check the row, its count, its click, the subfolders-switch reveal (Main.qml), and the top-bar label for `("", false)` with and without root files. Record the result. | Open |
| L-32 | C-42 | incorrect | `crates/cli/src/lib.rs:176` | `Event::StartedOver.detail` is documented as a technical detail for logs (`thumbcache/src/lib.rs:258-259`). The CLI now prints it (`cli/src/report.rs:40`). The desktop does not log it: `app/src/app.rs:191-199` forwards the notice without it. | Write `detail` to a log line in the desktop handler, as the CLI does. | Open |
| L-39 | C-16 | pending | `tools/compare-shots.ps1:87` | The shot recipe was not run for the fix commit. `tools/shots/reference/` holds a reference for `3edb43b63b` (it has `borders.png`), not for `cb123b9`. The script is now `tools/compare_shots.py`. | Run `just shots -Expect borders` once, then `just shots -Record cb123b9`. Commit the new reference directory. Record the result. | Open |
| L-40 | C-17 | pending | `crates/desktop/cpp/screenshot.cpp:170` | The by-hand check the plan requires (`--wait=1` with `echo $?`) was not done. The claim that the exit code now reaches the OS is therefore unverified. `crates/gallery/src/main.rs` has no test of the ExitCode mapping. | Run `gallery.exe --screenshot --wait=1` in a normal run (expect 0) and in a no-window run (expect 1, settle log written, no PNG). Check each with `echo $?`. Record the result. | Open |
| L-42 | C-18 | undeclared-deviation | `crates/desktop/src/folders.rs:211-214` | The plan says the "All folders" label uses `text::ui("ui-all-folders")` from Rust. The code returns an empty `ROLE_NAME` for the All and Drive-roots rows, and moves the words into `FolderTree.qml:53-55`, keyed by a new `kind` role. The deviation is not recorded. `folders.rs` has no `#[test]`, and no `tst_folder_tree.qml` exists. | Record the choice in the plan. Add a test that row 0 shows `ui-all-folders` and row 1 shows `ui-drive-roots`. | Open |
| L-44 | C-45 | pending | `eval/verify.py:652` | The pre-fix baseline of the three pair sets and the verifier rerun with the fixed code are not recorded as done. `docs/temp/VERIFIER.md` has a newer section with pre/post tables and a max-noise sweep, but its older re-pick section (lines 97-120) still says the re-pick is pending. The default is still `0.06`. The plan (section 7) requires old and new figures in the topic docs. | Remove the stale re-pick section, or make it match the newer section. Confirm the archived outputs and the `--max-noise` choice, and record both sets of figures. | Open |
| L-46 | C-46 | untested | `eval/jpeg-decoders/gpu.py:32-44` and `:74-75` | The import-order fix and the pillow ceiling check are in place, but no output of running `gpu.py` or `nvimgcodec_bench.py` is recorded. `blocking_sync()` raises `RuntimeError` on a non-zero return code, which stops the whole script. | Run `gpu.py` in venv-eval and `nvimgcodec_bench.py` in the nvImageCodec venv. Record the output. | Open |
| L-47 | C-46 | pending | `docs/temp/JPEG_DECODERS.md:595` | The DLL change is recorded. The pending action, pinning venv-eval's `nvjpeg64_13.dll` back to 13.2.3 and checking it, is recorded in no doc. | Record the pending pin and its check under Not run or Behaviour. | Open |
| L-48 | C-50 | untested | `eval/jpeg-decoders/benches/ir.rs:31` | The C-50 Rust edits were never compiled. `eval/jpeg-decoders` declares its own `[workspace]` (`Cargo.toml:9`), outside the root workspace (`members = ["crates/*"]`). CI runs only `cargo metadata --locked` on it (`ci.yml:292`). The `bail!` path in `src/lib.rs:104` and `main.rs` is also uncompiled. | Run `cargo check --benches` on that manifest in the pg-dev container, and record the result. Then consider adding it to CI. | Open |
| L-54 | Decision 6 | undeclared-deviation | `eval/jpeg-decoders/requirements.txt:6` | Decision 6 records the pins as settled and names only nvImageCodec and nvJPEG. `requirements.txt` also moves `nvidia-libnvcomp-cu13` from 5.3.0.16 to 6.0.0.12 (commit 314a99c, outside the range), and its header changes from "exactly as frozen" to "latest release". No doc names the libnvcomp bump now that `CODE_REVIEW.md` is gone. The plan (lines 58 and 663) leaves the pins to the user. | Name the libnvcomp major bump in the plan's Decision 6. Get the user's confirmation, or reopen the decision. | Open |
| L-55 | Decision 5 | undeclared-deviation | `eval/jpeg-decoders/env.py:2` | Decision 5 (drop `python -I`, allow shared helper modules) overturns the plan's Choice 5 (plan line 56) and the plan's "python -I run of debesheedas.py" verification. It has no "Confirmed by the user" tag. The plan's "To state plainly" list (lines 633-663) omits it. | Get the user's confirmation, or reopen the decision. Add the change to the final summary. | Open |
| L-57 | C-16 | pending | `docs/temp/PLAN.md` (Verification) | No recorded results exist for `cargo fmt`, clippy, the workspace test, the wasm32 check, `just lint`, `just lint-qml`, `just qmltest` or the Python test recipe. The cb123b9 commit message names none of them in its "Not verified" list. | Run each command, and record pass or fail with its output. Anything not run goes on the Not-run list. | Open |
| L-59 | docs | stale-doc | `docs/PLAN.md:346-347` | It says the migration loop exists "with no migrations yet". `crates/catalog/src/schema.rs:25` has `MIGRATIONS = &[TO_2_CHANGE_STAMPS]`, so the statement is false. The plan's list of places to update omits this file. | Add a dated "Superseded" note at that line, in the style of the notes beside it, pointing to migration 2. | Open |
| L-61 | CC-2 | undeclared-deviation | `crates/writeback/src/lib.rs:130` and `:654` | The plan asks for a `#[cfg(test)]` seam that makes `process` panic for one `FileId` (C-27). The code gates the fault seam on `any(test, feature = "fault")` and exposes `pub fn start_with_fault`. The change is needed, since `cfg(test)` does not reach other crates' tests, but it is recorded only in a Cargo.toml comment (`crates/writeback/Cargo.toml:13`) and the lib.rs doc. | Record the decision in the plan's Choices list, and cross-reference it from the C-27 row. | Open |
| L-64 | CC-7 | pending | the plan's Verification list (plan file lines 619-631) | The plan's Verification list ("run once, after all edits") has no recorded results in the repository. Same gap as L-57. | Run the list in pg-dev after the last edit: `docker/check.sh`, plus the wasm32 check that CI runs (`ci.yml:286`). Record each command's result. | Open |
| L-65 | CC-8 | pending | `tools/shots/reference/` | The C-16 row said the shot references are recorded again after the commit. The references were re-recorded in `87fead9` (2026-11-11) for `3edb43b`, not for the fix commit `cb123b9`, and no `-Expect borders` record exists. | Run the C-16 recipe for `cb123b9` and commit the new reference directory. | Open |

## Low

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| L-67 | C-27 | incorrect | `crates/import/src/lib.rs:73`; `crates/thumbs/src/lib.rs:250` | Import stores `format!("{e:#}")`, and thumbs does the same, so a caught panic is stored as bare payload text, not through `Cause::Panicked`. Crop and the writer use `Cause::of`. No desktop reader currently shows the stored import failure, so the impact is small. | Route both through `Cause::of`, as crop and the writer do. | Open |
| L-68 | C-56 | shortcut | `crates/edit/src/rewrite/raster.rs:33,47,77,84` | Only the raw PNG buffer is allocated fallibly (`raster.rs:24`). The packed copy, the `apply_rows` output, the crop output and `pack` use infallible allocation. They are bounded by the cap, but the plan asks for `try_reserve_exact` on the PNG path. | Use `try_reserve_exact` on the other PNG buffers. | Open |
| L-69 | C-25 | stale-doc | `crates/metadata/src/exif.rs:31` | The `Exif` doc says a tag given twice reads as the last one. Orientation reads the last readable entry (`exif.rs:53`). | Reword the doc. | Open |
| L-72 | C-41 | shortcut | `crates/platform/src/native.rs:139-155` | `altered()` uses `cfg_attr(unix, expect(dead_code))`, and the unix arm of `check_folder_names` uses `let _ = path;`. A `#[cfg(windows)]` gate and a `_path` name are the proper form. | Use the `#[cfg(windows)]` gate and the `_path` name. | Open |
| L-76 | C-43 | untested | `crates/yandex/src/fetch.rs:80-104` | The per-request transfer timeouts and the download content-type gate are tested only through `plausibly_image`. The live path is not run. A local-listener test seam would cover it without sending photos externally. | Add a local-listener test seam for the download path. | Open |
| L-77 | C-42 | stale-doc | `crates/thumbcache/src/lib.rs:13` | Damage among the entries is found only by the day's check (`HEAL_INTERVAL`, 24 h), so a damaged entry can be served for up to a day. The lag is stated, but not the user-visible effect. The reviewer did not trace the read path, so the specific consequence (a thumbnail fails to decode) is unverified. | Trace the read path, record the user-visible effect, and consider an immediate heal as a follow-up. | Open |
| L-79 | C-11 | incorrect | `crates/desktop/src/yandex.rs:1` (and `crates/desktop/qml/YandexPanel.qml`, `ResultTile.qml`, `crates/client/src/layout.rs`) | Commit 572918f, which is in range, changes these files. The reviewer was told they were uncommitted and out of scope, but the tree is clean and the commit is present. This review did not check the layout, `show_failure`, the no-folder status, or the picker work, beyond the C-11 spawn, search and download paths. | Review 572918f against the plan's section 5 and the C-11 row, separately from this ledger. | Open |
| L-81 | C-52 | untested | `eval/bench_hdd.py:166-173`; `eval/bench_ann.py:260` | The `--dense` overlap guard is still inline and the `-1` padding in `rerank` has no test or recorded run. The harness flag checks likewise. | Extract the guard into a pure function, and add small tests for it and for the padding. | Open |
| L-82 | C-48 | stale-doc | `eval/jpeg-decoders/nvimgcodec_bench.py:50`; `debesheedas.py:25`; `gpu_encoders.py:32` | The PIL reader returns `np.asarray(Image.open(path))` with no `convert("RGB")` and no shape check. So a P5 file loads as 2-D, and `diff()` reports a size string instead of failing. The impact is latent, since the only producer writes P6. | Convert to RGB and check the shape in the reader, in all three scripts. | Open |
| L-83 | C-49 | incorrect | `eval/jpeg-decoders/encoders.py:61` | `png_of` writes a fixed `<name>.png.tmp` and renames it, and `encode()` calls it from a thread pool without `prepare()`. The four guetzli jobs for one file can write the same temp concurrently, which is the race C-49 was meant to remove. | Use a unique temp name in the same folder. | Open |
| L-88 | CC-6 | stale-doc | `docs/CLIPPY_LEDGER.md:57` | The panic row lists the two writeback panic sites, but cites `crates/writeback/src/lib.rs:701` and `:708`. The real sites are `crates/writeback/src/fault.rs:35` and `:42`. | Correct the references. | Open |

## Pending

Work not done, or not checked on this machine. Each one is open until its command has been run and its result recorded.

- **Verification list** (plan lines 619-631): fmt, clippy, the workspace test, the wasm32 check, `just lint`,
  `just lint-qml`, `just qmltest`, ruff and the Python test recipe. Recorded results: none (L-57, L-64). The Linux subset
  was run in the pg-dev container on this change and passed; the Windows-side runs were run earlier in the session but
  not recorded in the repository.
- **C-16 shot recipe:** `just shots -Expect borders`, then `just shots -Record cb123b9` (L-39, L-65).
- **C-44 and C-45:** the pre-fix baseline of the three pair sets, verifier rerun, `--max-noise` re-pick, and the
  stale re-pick section in `docs/temp/VERIFIER.md` (L-44).
- **C-46 and C-47 GPU reruns:** both venvs, including the blocking-sync helper and the import-order check (L-46).
  The venv-eval nvJPEG DLL pin back to 13.2.3 (L-47).
- **C-50:** the bench compile in `eval/jpeg-decoders` and the callgrind benches (L-48).
- **Live Yandex path** (C-10, C-43): not run, since photos are not sent to external services (L-76 for the test seam).
- **Qt GUI checks:** the Drive-roots row and its labels (C-58, L-30), and the exit-status check (C-17, L-40).
- **Windows runs** of the `#[cfg(windows)]` verbatim tests for C-41 (L-21).
- **Permission-based tests that skip silently as root** (L-14): the container runs as `pg`, but the root-skip tests still
  need a Windows run or a recorded non-root run.
- **C-24:** the desktop never reads `Writer::is_running` (L-09). Open question: restart the writer, or show a notice.
- **C-36 residual:** old-name `.pg-write` leftovers are never removed (L-15). Open question: add a one-time removal at
  writer start, or list it as a residual.
- **C-42:** damage in cache entries is served until the day's check (L-77).

## Waiting on a person

- **Decision 5** (drop `python -I`, shared helper modules): no "Confirmed by the user" tag (L-55).
- **Decision 6** (the nvImageCodec and nvJPEG pins, and the `nvidia-libnvcomp-cu13` bump from 5.3.0.16 to 6.0.0.12):
  no confirmation tag (L-54).
- **Commit 572918f:** the task listed `layout.rs`, `ResultTile.qml`, `YandexPanel.qml` and `yandex.rs` as uncommitted
  and out of scope, but commit 572918f has since landed with those changes. Whether it is in the reviewed range needs
  the owner's confirmation (L-79).
- **The pending verification items above** need someone with the Windows host and the GPU venvs.

## Not reviewed

`crates/client/src/layout.rs`, `crates/desktop/qml/ResultTile.qml`, `crates/desktop/qml/YandexPanel.qml` and
`crates/desktop/src/yandex.rs` were not reviewed, as they were uncommitted when the review started. Commit 572918f has
since landed them; L-79 covers that.
