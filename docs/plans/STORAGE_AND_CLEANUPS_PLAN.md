# Behaviour changes: priorities, serial, Windows storage, import retry, typed options

Part B of one plan that was split in phase 0. The combined text is in commit `a8efee20`. The other parts are `HEADLESS_SESSION_PLAN.md` (Part A) and `ADAPTIVE_READERS_PLAN.md` (Part C).

**Global phase order:** A1, A2 (`HEADLESS_SESSION_PLAN.md`); B3, B4 (this document); C5 to C10 (`ADAPTIVE_READERS_PLAN.md`). Phase numbers are global. Phase B3 and phase B4 both wait for A2, because the CLI still uses the items they remove.

## Context

Part A removes the CLI. Four behaviours in the code are there because the CLI or its flags needed them. They can go once the CLI is gone, or they have to change on their own:

- `Priority::Idle` is the CLI's `thumbs` default. The app's thumbnails and placeholders use it too.
- `--serial` and the `serial_drives` setting are an escape hatch for drives the probe cannot tell apart.
- Retry for import failures exists only in the CLI. The app's Retry button skips them on purpose: `Catalog::retry_failures` returns early for `FilePass::Import` (`crates/catalog/src/ledger.rs:192-196`).
- `pipeline::Options` has fields that any caller can set to zero (`crates/jobs/src/pipeline.rs:33-50`). The pool and the CLI clamp them with `.max(1)`.

There is also a storage gap on Windows. A volume on a virtual or RAID disk gets its own lane, which competes with the real disk underneath it.

## Decisions (made by the user)

- `Idle` is never used. `BelowNormal` is the one background class, for every pass in the app and the CLI.
- `--serial` and `serial_drives` are removed on both platforms. The Linux gap they covered is accepted: an opaque device and the real disk under it get separate lanes.
- Storage support starts small. On Windows, only NTFS on plain physical disks is supported. VHD/VHDX, Storage Spaces, RAID, iSCSI and unknown buses are refused. USB stays allowed; its kind is `Unknown`, and the controller measures it. Linux is unchanged: ext4 and similar are still accepted, and opaque devices stay `DriveKey::Opaque`.
- Retry works for import failures in the app.
- No back-compat (see Part A).

## 1. Priorities

- Users of `Priority::Idle`: the thumbnails pass (`crates/library/src/passes.rs:273`), the placeholders pass (`passes.rs:320`), and the CLI's `thumbs` (`cli/src/lib.rs:531`, removed with Part A).
- On Windows, `Idle` is `THREAD_PRIORITY_IDLE`. It affects CPU scheduling only. The workspace never uses Windows' background mode: a search for `THREAD_MODE_BACKGROUND`, `SetPriorityClass` and `ioprio` found no matches.
- The "6.5× slower" figure in the CLI's comment (`cli/src/lib.rs:529-530`) is not documented anywhere in the repo's docs. `DECODE_ONCE_PLAN.md` and `START_PROFILE.md` mention Idle, and should be checked for what they say.
- **Change:** every pass runs at `BelowNormal`. Remove `Priority::Idle` (`crates/platform/src/priority.rs`) and the `priority` parameter of `Inner::options`, which becomes constant. `Normal` stays for work the user is waiting for.
- **Measure first.** Time thumbnails at `BelowNormal` against `Idle`, on the same files, on an idle machine and with a competing CPU load. Record the result in the repo. Then remove `Idle`.

## 2. `serial`

- Remove `Features.serial_drives` (`crates/model/src/features.rs:16,21,30`), `Prefs.serial_drives` and its golden JSON (`crates/app/src/prefs.rs:54,65,170,266,307`), `crates/client/src/features.rs:73`, `crates/library/src/runner.rs:294,343,353`, and `coordinator.rs:426-427`.
- Remove the `serial` parameter of `lanes::plan`, `groups`, `command_groups` and `assign` (`lanes.rs:42,59,85,88,131,133,184`). The fail-closed `any(Option::is_none)` test at `lanes.rs:185` stays.
- Tests: `lanes.rs:269` and `crates/library/tests/lanes.rs:137`.
- No UI toggle exists (searched `crates`). The setting is reachable only by editing `gallery.json`.
- Docs: `docs/reference/STORAGE.md:51` and `docs/plans/MULTI_DRIVE_LANES_PLAN.md`.

## 3. Windows storage rule

- **Where the check goes.** `check_ntfs` (`crates/scan/src/refusal.rs`), after the existing network, removable, link and NTFS checks.
- **Not a new fact on `RootFacts`.** `drive::win` imports `crate::dir::{RootFacts, SystemFacts}` (`crates/platform/src/drive.rs:235`). A `dir` to `drive` call would therefore make a module cycle, and `just modgraph` fails on any cycle.
- **Design.** `scan::refusal` already depends on `platform`. It takes a second seam, `&dyn Drives`, as `lanes::probe_place` does. On Windows it refuses when `Drives::of(folder)` is `DriveKey::Opaque`. Linux also returns `Opaque`, so the gate is Windows only. The check reuses the probe's single opaque rule (`OPAQUE_BUSES`, `drive.rs:185`) rather than repeating it. Tests inject fakes.
- **Wiring.** Add one `RefusedReason` variant in `pg_model::refusal`, with its `key()`. Add a `Fact` so that a probe that can't be read refuses the folder (fail closed). The words go in `crates/wording/src/lib.rs:663-686` and `text.ftl:98+`. No notice kind is added, so the `KINDS` checklist in `crates/model/src/notice.rs` is untouched.
- **Folders already in the library.** A folder on such a disk gets the same refusal at its next scan, through the folder ledger.
- **Test impact.** Every scan test on Windows now runs the real check through `SystemDrives`. These include `scan::check_folder` (`crates/scan/src/lib.rs:417,435`) and `folders::add_folder` (`crates/library/src/folders.rs:37`). A CI runner or dev machine whose temp folder is on a disk with a virtual, unknown or RAID bus would fail them. Verify on the Windows CI runner before merging. Inject a fake in tests that do not mean to test the check.
- **Unverified risk.** Ordinary SATA or NVMe disks behind a RAID driver (Intel RST "RAID On") may report a RAID bus type and be refused. Measure this on such a machine, then decide whether to accept it and say so in the refusal's words.
- **SCSI.** "SCSI" is taken to mean iSCSI, which is refused. Ordinary SCSI and SAS buses stay allowed. This needs confirming.

## 4. Retry for import failures

- **Today.** `Catalog::retry_failures(Import)` returns early (`ledger.rs:192-196`). The CLI's `import --retry-errors` passes `retry_errors` down through `pg_import::import_while` into `Catalog::pending_import` and `pending_import_sql` (`work.rs:62-79, 448-466`).
- **Change.** `retry_failures(Import)` marks the present files whose analysis failed as pending, in one bulk `UPDATE`:
  `UPDATE files SET pending = 1 WHERE missing = 0 AND id IN (SELECT file_id FROM analysis WHERE error IS NOT NULL)`.
  It does not reuse `mark_pending`, which loops per id (`work.rs:595`). The next import pass retries those files. The ledger's trigger hides their rows while they are pending (`file_failures_hidden`, `ledger.rs:803`).
- **Removed.** The `retry_errors` parameter of `import_while`, `pending_import` and `pending_import_sql`, along with the UNION branch. About 30 call sites change: `core.rs:758-892`, `work.rs:647-759`, `writers.rs`, `test_support.rs:60,132`, `bench catalog.rs:151,182`, `library/tests/pairs.rs:82,132`, `stop_at_batch.rs:50,54` and `scan/src/lib.rs:2103`. The two tests of the retry form (`core.rs:759-765`, `work.rs:759`) go.
- **The CLI flag** `import --retry-errors` has no replacement. The app's Retry does the same job.
- **Stale text that changes:**
  - the tests `ledger.rs:691-694` and `ledger.rs:769`;
  - the doc comment `ledger.rs:189-191`, which cites `gallery import --retry`;
  - the module doc `ledger.rs:86`;
  - the descriptions of Retry in `crates/app/src/inbox.rs:77,507`.
- `Catalog::mark_pending` and `pairs_again` stay. They have callers in benches, tests, `scan`, `library/examples/pairs_probe.rs` and `lifecycle.rs:799-803`.
- **Tests.** Retry marks only present failed files, and a file that fails again is listed again.

## 5. Typed pipeline options

This is the typing half of `docs/plans/TYPE_MISMATCH_VALIDATION.md` item 2. The CLI flags that item was written for are gone with the CLI. The typing still matters, because any caller of `pipeline::Options` can build an invalid pool.

- **Change.** `workers` and `slots` become `NonZeroUsize`. `buffer_bytes` becomes a `BufferBytes` newtype in `pg-model`. It refuses zero and checks the byte conversion. `readers` becomes the typed `Readers` of the adaptive plan (phase C9). Until then, `readers` is `NonZeroUsize`.
- **Slot maximum.** `Pool::with_budget` preallocates `Vec::with_capacity(slots)` (`pipeline.rs:794`), so a huge slot count is a huge allocation. Slot counts above a stated maximum are refused. The maximum is to be chosen.
- **Arithmetic.** `Options::budget` and `Options::slots` add with plain arithmetic (`pipeline.rs:81, 89`), which wraps in release builds. Make them checked, and refuse on overflow.
- **Clamps that stay.** `Budget::new`, `Budget::set` and `Governor::new` keep their clamps (`governor.rs:76-77, 87`). `pipeline.rs:1465` pins `Budget::new(0) == 1`. The "no silent clamp" rule covers `Options`, not `Budget`.
- **Soft limit, unchanged.** The budget is a target. A file larger than the budget is read alone (`pipeline.rs:800-801, 836`). No value is refused for being large. A strict bound, which would refuse or split large files, was considered and not chosen.
- **Literals that change.** Every `pipeline::Options { … }` literal changes. The sites are the test module (`pipeline.rs:1035-1333`, seven literals), `pipeline.rs:1496`, `crates/library/tests/borders.rs:45`, `crates/library/tests/hashes.rs:35`, `crates/library/examples/pairs_probe.rs:68`, `inner.rs:65` and the `opts()` helpers in `crates/library/tests/*.rs`. A full count is needed before implementation.
- **Tests.** Zero cannot be built, so none is needed for it. The pool tests keep their assertions. `BufferBytes` refuses zero and overflow, and a slot count above the maximum is refused.

## 6. SEC-043

Closed by the removal of `--buffer-mb` (phase A2) and by the typed options (phase B3). The entry at `docs/plans/SECURITY_FLOW_REVIEW.md:753` is updated when phase A2 lands. Its statement that `borders` takes no pipeline flags was wrong. The flag is on five commands: `import`, `hash`, `pair`, `thumbs` and `borders` (`crates/cli/src/args.rs:92, 115, 121, 137, 165`). That correction is made in phase 0.

## Phases

- **B3 Cleanups** (after phase A2 has deleted the CLI, so nothing else uses the removed items): measure `BelowNormal` against `Idle` and record it, then remove `Priority::Idle` and the priority parameter of `Inner::options`; remove `serial_drives` and the `serial` parameters; make Retry work for imports and remove the `retry_errors` parameter; type the `Options` fields, leaving `readers` to phase C9. Tests and docs are updated in the same phase.
- **B4 Windows storage rule:** the new refusal behind its own seam, its words, and tests with fakes. Verify on the Windows CI runner before merging, because every scan test on Windows runs the real check. Run the Intel RST check on a machine that has one.

## Verification

- `cargo test -p pg-jobs --locked`, `-p pg-library --locked`, `-p pg-catalog --locked`, `-p pg-scan --locked` and `-p pg-model --locked`.
- Priority: the measurement record in the repo, committed before `Idle` is removed.
- `just check`, `just lint`, `just modgraph`, `cargo doc -D warnings`, `cargo machete`.
- Retry: a test that marks only present failed files, and a test that a file failing again is listed again.
- Storage: fake `Drives` tests for each bus type (virtual, file-backed, RAID, iSCSI, unknown, USB, SATA, NVMe), on the Windows CI runner.
- Serial: the `lanes.rs` tests run with the parameter gone, and the fail-closed test still passes.
- Typed options: `BufferBytes` tests for zero, overflow and the slot maximum.

## Risks and unverified

- RAID-mode machines may be refused (Intel RST).
- Thumbnail speed at `BelowNormal` against `Idle` has not been measured.
- The Linux opaque gap is accepted. Whether it matters in practice is unknown.
- "SCSI" is read as iSCSI. This needs confirming.
- The count of `Options` literal sites needs a grep at implementation time.
- An existing test may assert that Retry does nothing for imports. The search for it is part of phase B3.

## Critical files

- `crates/platform/src/priority.rs`, `crates/platform/src/drive.rs`.
- `crates/jobs/src/pipeline.rs`, `crates/jobs/src/governor.rs`.
- `crates/library/src/inner.rs`, `coordinator.rs`, `lanes.rs`, `runner.rs`, `lane.rs`, `passes.rs`.
- `crates/model/src/features.rs`, `crates/model/src/refusal.rs`, `crates/app/src/prefs.rs`, `crates/client/src/features.rs`.
- `crates/scan/src/refusal.rs`, `crates/wording/`.
- `crates/catalog/src/ledger.rs`, `work.rs`; `crates/import/src/lib.rs`; `crates/app/src/inbox.rs`.
