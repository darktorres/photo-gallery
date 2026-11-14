# Notifications: open items

Status (2026-11-14): the notifications feature is built (S1 to S3, then the fix plan, implemented at `e634c83`). The
reference for the code is [NOTIFICATIONS.md](NOTIFICATIONS.md) in this folder. This file is the one list of what is still
open, merged from the feature-level list and the fix plan. Each item was checked against HEAD `472eaf5` by reading the
code. Nothing was built or run. Items marked *read* were found by reading the code and have not been reproduced.

## Done here, tracked elsewhere

- **C2, the inbox's own reads off the GUI thread:** done. The failed-files ledger (`crates/desktop/src/failed_files.rs`),
  the Copy of failed files (`crates/desktop/src/notices.rs`) and Reveal (`crates/desktop/src/shell.rs`, `reveal`) read
  through the catalog reader's session, and the inbox's marks, dismissals and retries are queued to its writer. The
  window's other catalog work on the GUI thread is listed in [GUI_THREAD_PLAN.md](GUI_THREAD_PLAN.md).
- **The scan's lock:** done. A scan lists outside any transaction and writes each batch in one short write transaction,
  and a pause writes the batch before it waits (`crates/scan/src/lib.rs`, the module docs and `checkpoint`). What is
  still open is the test gap under Tests, item W2.

## Behaviour

1. **Progress poke.** While `failed > 0`, the desktop's change pump should poke the inbox on `Change::Progress`, so the
   "N failed" link follows within about 1 s. Today `on_pass` (`crates/desktop/src/library.rs:132`) only calls
   `refresh_progress` on progress. The inbox poke (`library.rs:158`) fires only on job rounds.
2. **Crop failures.** `crop::Stats` keeps only the first failure (`error: Option<FileFailure>`, `crates/crop/src/lib.rs:32`),
   so `Notice::Cropped` carries only `first_error` (`crates/model/src/notice.rs`). Keep every failure: add `keep_failure`
   to `crop::Stats` (as the thumbs and scan passes do, `crates/jobs/src/failure.rs`) and carry the list into `Cropped`.
   No document defines the `refused` field an earlier note named for `Cropped`; decide whether it is needed before
   adding it.
3. **`session-not-read` is never cleared.** A successful session save resolves only `session-not-saved`
   (`crates/desktop/src/shell.rs:497`, the `Command::SaveSession` arm). The model says the next successful save clears
   both (`crates/model/src/notice.rs:1023`, `lifecycle`). A failed startup read (`crates/desktop/src/controller.rs:52`)
   stays on the inbox until it is dismissed.
   - Fix: resolve `session-not-read` on the same save.
   - Test: a read failure, then a successful save, leaves no entry.
4. **An import failure can show while the grid hides it** (*read*). `analysis_import` (`crates/catalog/src/schema.rs:354`,
   `TO_9_CHEAPER_WRITES`) sets the row's `hidden` from `files.missing` and ignores `pending`. The `files_failures_hidden`
   trigger (`schema.rs:283`) recomputes `hidden` only when `pending` or `missing` changes. A failed analysis whose size
   differs from the file's leaves `pending` at 1 (the `stale!()` check, `crates/catalog/src/sql.rs:8`), so nothing
   fires, and the row stays visible while the grid's failed filter (`pending = 0`) hides it. It clears at the next
   analysis, but it breaks "the inbox and the grid agree by construction". The grid side is not traced here.
   - Fix: after any analysis insert, `hidden` must equal `missing OR (import AND pending)`, whatever `pending` was before.
   - Test: a pending file, analysed with a stale size, has its import row hidden.
5. **A scan lease can expire during one slow listing** (*read*). The walk renews its lease at each folder boundary
   (`checkpoint`, `crates/scan/src/lib.rs:577` and `:591`) and after each batch write (`:528`). A single folder listing
   that takes longer than `SCAN_LEASE_SECS` (60 s) renews nothing. A second walk can then take the lease while the first
   is still writing. The comment at `crates/scan/src/lib.rs:566` says a walk held up anywhere renews the lease, and that
   is not true.
   - Fix: renew inside the listing path, or check the renewal on the batch path and stop.
   - Test: a listing that outlasts the lease does not let an overlapping walk in.
6. **Retired `root-refused` rows are not converted.** `convert_notify` (`crates/catalog/src/convert.rs:127`) deletes only
   `scan-unreadable` and `mount-refused`, so rows left by the retired `root-refused` decode as `Unknown` and stay in the
   inbox. `Notice::RETIRED` already lists the kind (`crates/model/src/notice.rs:847`). Adding it to `convert_notify` only
   helps catalogs that have not migrated yet. Migrated catalogs need a new migration step, or a decision to accept those
   rows as `Unknown`.
7. **`ledger-not-written` is still a multi-pass notice in the code.** `Notice::LedgerNotWritten` carries a `FilePass`
   (`crates/model/src/notice.rs:569`). Only the thumbnails pass posts it: `ledger_written` is called with
   `FilePass::Thumbs` alone (`crates/library/src/runner.rs:569`), and the notice's lifecycle keys on the pass
   (`notice.rs:1007`). Make it thumbnails-only in the type and regenerate the table in `NOTIFICATIONS.md`.
8. **"Mark all read" has no control.** The catalog and `Notices` are bounded to the displayed ids (A3, D1), but nothing in
   the inbox panel calls `mark_all_read`: the Rust method exists (`crates/app/src/inbox.rs:392`), and the window's bridge
   (`crates/desktop/src/notices_bridge.rs`) exposes no such call. Add the control, or decide the plan does not need it.
9. **The log view's keyboard scroll does not stop following.** `LogPanel.qml` calls `scrolled()` on `movementEnded` and on
   a wheel scroll only (`crates/desktop/qml/LogPanel.qml:93-94`), and has no key handler. The Qt source confirms
   `movementEnded` fires only when a flick or drag stops moving (`qquickflickable.cpp:3485`). That a keyboard move never
   reaches it is not traced or reproduced. Reproduce it first, then fix it, with a test that a keyboard scroll stops
   following.
10. **Two places where the plan and the code disagree, and need a decision:**
    - D1: `ListView.currentIndex` is left at -1 (`crates/desktop/qml/InboxPanel.qml:204`) and the selection is held in
      `selectedKey` (`:35`). Either bind the current item to the key (focus and assistive technology need it), or change D1.
    - E: the plan says the writing path keeps its `try_lock`. Only `Logger::panic` uses `try_lock`
      (`crates/log/src/lib.rs:258`); `Logger::log` takes a blocking `lock()` (`:240-246`). Either restore the `try_lock`
      with its raw-line fallback, or change E.

## Tests

1. **QML stubs** (`crates/desktop/tests/qml/stubs/pg/gallery/`): add `Notices.qml` and `LogModel.qml` with their `qmldir`
   lines, `devMode` in `Settings.qml` and `globalAction` in `Shell.qml`. Run `just qmltest` first to see which tests fail
   without them. Not run; the directory has neither file.
2. **`THUMBS_VERSION` is nonzero:** a test against the real constant (`crates/render/src/lib.rs:14`), or a compile-time
   assert. No test asserts it now.
3. **Stored-notice dedup ignores `detail`.** The write path is `upsert` (`crates/catalog/src/inbox.rs:289`), which compares
   the whole JSON, so this test waits on the dedup row in the design table below.
4. **Every trigger fires:** a test inserts through each (`crates/catalog/src/schema.rs` only checks that they exist).
5. **Root removal from the CLI's `remove` drops the root's notices.** There is no `roots` trigger: `notices.root` is a
   foreign key with `ON DELETE CASCADE` (`crates/catalog/src/schema.rs:82`). Test it through `remove_root`, which the CLI
   calls at `crates/cli/src/lib.rs:218`. The existing tests are catalog-level only.
6. **`markRead` with a displayed serial leaves later rows new.** Only the newest path is covered.
7. **`crates/catalog/src/fixture/raw.rs`:** a `older_thumbs_version` / `start_thumbs_version` pair beside the import pair
   (`older_import_version` / `start_import_version`). The file has no thumbs pair.
8. **W1, timed:** with a 200k-file walk running and paused, a dismissal and a post land within one batch write's time.
   Missing. The nearest test, `no_write_transaction_of_a_200_000_file_walk_holds_the_lock_long`
   (`crates/scan/src/lib.rs:1309`), bounds how long one write transaction holds the lock; it does not pause a walk or
   post a notice.
9. **W2, write-back:** `a_write_back_between_the_listing_and_the_batch_is_kept_and_a_real_change_is_not_blocked`
   (`crates/scan/src/lib.rs:1202`) covers a write-back between a listing and its batch. It does not cover a Retry or a
   version bump in between (the import is stood in for by `mark_pending`). A panic test gives the lease back
   (`a_walk_that_panics_gives_its_lease_back`, `:1161`), but no test shows a walk that returns an error gives its lease
   back. The lease is released on drop (`crates/catalog/src/core.rs:293`), so this is a test gap, not a known bug.
10. **W1, coalescing:** nothing pins how a `Post` coalesces past a same-key `Dismiss` or `MarkRead` queued between two
    posts. The `Work` tests (`crates/app/src/inbox.rs:1238`) cover repeated posts only. Decide the outcome and test it.
11. **`a_clear_of_nothing_writes_nothing`** (`crates/app/src/inbox.rs:1636`) checks only `data_version`, so it passes
    whether or not the clear takes the write lock. Hold `BEGIN IMMEDIATE` on another connection and check the clear still
    returns.
12. **A6, import ledger:** missing tests for a rotation write-back followed by `--retry`. The nearest test,
    `the_imports_rows_come_back_with_their_serials` (`crates/catalog/src/ledger.rs:646`), covers a metadata write-back made
    by a rating, not a rotation. Also missing: a folder removed and re-added (no test by that name);
    `set_import_version` firing the hidden trigger only for rows whose flags change; and `EXPLAIN` at 200k files with 50k
    hidden rows (no `50_000` fixture in `crates/catalog/src`).
13. **A2, pruning:** `states_are_never_pruned_and_an_update_prunes_nothing` (`crates/catalog/src/inbox.rs:668`) runs its
    update check at exactly the bound. Insert `KEPT_EVENTS` and then more first, so that pruning on update would fail it.
14. **Cache writer:** the `Failing` state machine in `crates/thumbcache/src/writer.rs` (`:175`) has no test that
    `TouchesWritten` or `Stored` follows a failure. No test in the crate or in `crates/app` references either event.
15. **Thumbs pass:** no test that the ledger is flushed when the pass errors or is stopped (`crates/thumbs/src/lib.rs`).
    The nearest tests cover unwritable ledgers, not flushes.
16. **Runner:** no test that a restart in `tend` posts nothing (`crates/app/src/library.rs:125`). The nearest test,
    `write_stopped_is_posted_once_where_the_writer_is_given_up` (`:407`), covers a writer given up, not a restart.
17. **B2, folders:** the reparse tests in `crates/scan/src/lib.rs:855-898` assert stats, not the ledger. No test shows a
    refused non-root folder swept by its parent's walk: the refused-folder test (`crates/catalog/src/roots.rs:443`) uses
    the root only, and the sweep test (`roots.rs:348`) covers unreadable folders only.
18. **B4:** no test for the once-per-kind decode log (`crates/app/src/inbox.rs:710`, the set of kinds already logged).
19. **D1 and D2:** no desktop test for the log model's seq offset (`crates/desktop/src/log_model.rs`, which has no test
    module), or for marking the displayed rows. The pure `list_diff` is tested (six tests in
    `crates/desktop/src/list_diff.rs`).
20. **Other T items:** replace the 20-byte JPEG test in `crates/render/src/maker.rs` (`whole.get(..20)` at line 290) with
    a file cut inside SOF; add a borders case in `crates/library/tests/writeback.rs`, which has none.

## Design: code and the design disagree

Each row needs a decision: change the code or change `NOTIFICATIONS.md`. The "code does" column was checked against HEAD
on 2026-11-14; the "design says" column was not checked against `NOTIFICATIONS.md`.

| Topic | Design says | Code does |
|---|---|---|
| `Notices` API | `markRead`, `reveal(fileId)`, `perform(action)` | `setViewing`; `Shell.reveal`; one method per action |
| Dev-mode property | `devMode` on `settings_bridge.rs` | Read-only `Shell.devMode`, saved by `Command::SaveDevMode` |
| Keeper-close failure | Log only (`WalNotMerged`) | Stored as `CatalogNotClosed` (`crates/app/src/app.rs`) |
| Dismissed events | Kept until purged after 30 days | Row deleted at dismissal (`crates/catalog/src/inbox.rs`) |
| Stored-notice dedup | Summary fields only | Whole JSON, `detail` included (`crates/catalog/src/inbox.rs`) |
| `IMPORT_VERSION` bump | Re-badges the import entry once | Test asserts it does not (`crates/catalog/src/ledger.rs`) |
| Root removal cleanup | Trigger on `roots` | Foreign key `ON DELETE CASCADE` (same effect) |
| Cache events | Five variants, `into_notice` | Seven variants (`TouchesWritten`, `Stored`), `Event::effect` |
| `Panels.modals` | `ArrayVec<PanelKind, 8>` | `Vec<PanelKind>` (`crates/client/src/controller/panels.rs`) |

## Performance gate

1. `just perf` against `docs/perf/baseline.json` has not run since the thumbs-pass restructure. The baseline is commit
   `1f26831` (2026-11-11). Nine later commits touch the thumbs, runner, thumbcache or render paths, including `e065886`
   (2026-11-11) and `a70d174` (2026-11-13, a change to the output). Not run.
2. `just perf --budgets` for the F1 comparison against `docs/perf/baseline.json`, as restated in `docs/perf/budgets.json`.
   Check the current result, since the catalog has changed since the budgets were measured. Not run.

## Screenshots

1. **F3:** run `just shots` against the current reference (`tools/shots/reference/3edb43b63b`, 98 commits behind HEAD
   `472eaf5`) to show the old scenarios still match. Then `just shots --record` and commit the new reference as
   `tools/shots/reference/<sha10>`. Not run.

## By-hand checks

1. Not run by this audit: dismiss the import entry, add a corrupt file, rescan (expect "1 since dismissed", and it
   survives opening the inbox); a thumbnail Retry does not re-badge import; a failing file mid-pass shows the link
   within about 2 s and opens the updating list; a write-stopped state clears after a healthy restart; a sub-folder
   rescan keeps the root's other unreadable folders; the keyboard-only inbox; a dismissal during a CLI import sticks.

## Checked and closed

- **`PassFailed` is not a retirement candidate.** The status bar's "N failed" link text comes from `pass_failed`
  (`crates/client/src/text.rs:261`), which renders `Message::PassFailed`. `refresh_progress` calls it
  (`crates/desktop/src/library.rs:175`). There is no `PassFailed` arm in `pass_status`. Keep the message and the
  `pass-failed` entry (`crates/client/locales/en/text.ftl:72`).
- **B5, bidi isolation of path arguments:** covered. `a_notices_paths_are_bidi_isolated`
  (`crates/client/src/text/notice.rs:248`) checks the path values of notices with a folder, a root and a file.
