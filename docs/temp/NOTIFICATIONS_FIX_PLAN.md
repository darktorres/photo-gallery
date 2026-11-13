# Notifications fix plan: what is still open

Status (2026-11-12): the plan was implemented at `e634c83`, and the code has changed since (modularity review 4,
catalog version 9). This file lists only what is still open, checked against HEAD `898b709` by reading the code. Nothing
was built or run. Items marked *read* were found by reading the code and have not been reproduced; the rest were
checked directly. The reference for the code is [NOTIFICATIONS.md](NOTIFICATIONS.md). [NOTIFICATIONS_PLAN.md](NOTIFICATIONS_PLAN.md)
keeps the feature-level list; some items here are on it too.

## Done here, tracked elsewhere

- **C2, the inbox's own reads off the GUI thread:** done. The failed-files ledger (`crates/desktop/src/failed_files.rs`),
  the Copy of failed files (`crates/desktop/src/notices.rs`) and Reveal (`crates/desktop/src/shell.rs`, `reveal`) read
  through the catalog reader's session, and the inbox's marks, dismissals and retries are queued to its writer. The
  window's other catalog work on the GUI thread is listed in [GUI_THREAD_PLAN.md](GUI_THREAD_PLAN.md).
- **The scan's lock:** done. A scan lists outside any transaction and writes each batch in one short write transaction,
  and a pause writes the batch before it waits (`crates/scan/src/lib.rs`, the module docs and `checkpoint`). What is
  still open is the test gap under Tests, item 2.

## Behaviour

1. **`session-not-read` is never cleared.** A successful session save resolves only `session-not-saved`
   (`crates/desktop/src/shell.rs`, the `Command::SaveSession` arm). The model says the next successful save clears
   both (`crates/model/src/notice.rs`, `lifecycle`). A failed startup read stays on the inbox until it is dismissed.
   - Fix: resolve `session-not-read` on the same save.
   - Test: a read failure, then a successful save, leaves no entry.

2. **An import failure can show while the grid hides it** (*read*). `analysis_import` (`crates/catalog/src/schema.rs`,
   `TO_9_CHEAPER_WRITES`) sets the row's `hidden` from `files.missing` and ignores `pending`. The `files_failures_hidden`
   trigger recomputes `hidden` only when `pending` changes. If `pending` is already 1 when the analysis is inserted and
   stays 1, nothing fires, and the row stays visible while the grid's failed filter (`pending = 0`) hides it. It clears at
   the next analysis, but it breaks "the inbox and the grid agree by construction".
   - Fix: after any analysis insert, `hidden` must equal `missing OR (import AND pending)`, whatever `pending` was before.
   - Test: a pending file, analysed with a stale size, has its import row hidden.

3. **A scan lease can expire during one slow listing** (*read*). The walk renews its lease at each folder boundary
   (`checkpoint`) and after each batch write (`walk_leased`, `crates/scan/src/lib.rs`). A single folder listing that
   takes longer than `SCAN_LEASE_SECS` (60 s) renews nothing. A second walk can then take the lease while the first is
   still writing. The comment at `crates/scan/src/lib.rs:566` says a walk held up anywhere renews the lease, and that is
   not true.
   - Fix: renew inside the listing path, or check the renewal on the batch path and stop.
   - Test: a listing that outlasts the lease does not let an overlapping walk in.

4. **Stored `root-refused` rows are not converted.** `convert_notify` (`crates/catalog/src/convert.rs:127`) deletes only
   `scan-unreadable` and `mount-refused`. Rows left by the retired `root-refused` decode as `Unknown`. The migration has
   already run on migrated catalogs, so the fix is a new migration step, or a decision to accept those rows as `Unknown`.

5. **`ledger-not-written` is still a multi-pass notice in the code.** `Notice::LedgerNotWritten` carries a `FilePass`
   (`crates/model/src/notice.rs`, around line 569), and the lifecycle has rows for borders and import. Only the thumbnails
   post it. Make it thumbnails-only in the type and regenerate the table in `NOTIFICATIONS.md`.

6. **"Mark all read" has no control.** The catalog and `Notices` are bounded to the displayed ids (A3, D1), but nothing in
   the inbox panel calls `mark_all_read`. Add the control, or decide the plan does not need it.

7. **The log view's keyboard scroll does not stop following.** `LogPanel.qml` clears `following` on a wheel scroll only.
   Qt does not emit `movementEnded` for keyboard moves, so the next record snaps the view to the end. Fix it, with a test
   that a keyboard scroll stops following.

8. **Two places where the plan and the code disagree, and need a decision:**
   - D1: `ListView.currentIndex` is left at -1 and the selection is held in `selectedKey` (`InboxPanel.qml`). Either bind
     the current item to the key (focus and assistive technology need it), or change D1.
   - E: the plan says the writing path keeps its `try_lock`. Only `Logger::panic` uses `try_lock`
     (`crates/log/src/lib.rs`); `Logger::log` takes a blocking lock. Either restore the `try_lock` with its raw-line
     fallback, or change E.

## Tests

1. **W1, timed:** with a 200k-file walk running and paused, a dismissal and a post land within one batch write's time.
   Missing.
2. **W2, write-back:** the test covers a write-back between a listing and its batch. It does not cover a Retry or a
    version bump in between (the import is stood in for by `mark_pending`). No test shows a walk that errors out gives its
    lease back.
3. **W1, coalescing:** nothing pins how a `Post` coalesces past a same-key `Dismiss` or `MarkRead` queued between two
    posts. Decide the outcome and test it.
4. **`a_clear_of_nothing_writes_nothing`** checks only `data_version`, so it passes whether or not the clear takes the
    write lock. Hold `BEGIN IMMEDIATE` on another connection and check the clear still returns.
5. **A6, import ledger:** missing tests for a rotation write-back followed by `--retry` (`crates/catalog/src/ledger.rs`);
    a folder removed and re-added; `set_import_version` firing the hidden trigger only for rows whose flags change; and
    `EXPLAIN` at 200k files with 50k hidden rows.
6. **A2, pruning:** `states_are_never_pruned_and_an_update_prunes_nothing` (`crates/catalog/src/inbox.rs`) runs its update
    check at exactly the bound. Insert `KEPT_EVENTS` and then more first, so that pruning on update would fail it.
7. **Cache writer:** the `Failing` state machine in `crates/thumbcache/src/writer.rs` has no test that `TouchesWritten`
    or `Stored` follows a failure.
8. **Thumbs pass:** no test that the ledger is flushed when the pass errors or is stopped (`crates/thumbs/src/lib.rs`).
9. **Runner:** no test that a restart in `tend` posts nothing (`crates/app/src/library.rs`).
10. **B2, folders:** the reparse test in `crates/scan/src/lib.rs` does not assert the ledger. No test shows a refused
    non-root folder swept by its parent's walk (`crates/catalog/src/roots.rs` tests the root only).
11. **B4:** no test for the once-per-kind decode log (`crates/app/src/inbox.rs`).
12. **B5:** no test for bidi isolation of path arguments (`crates/client/src/text/notice.rs`).
13. **D1 and D2:** no desktop test for the log model's seq offset (`crates/desktop/src/log_model.rs`), or for marking the
    displayed rows. Only the pure `list_diff` is tested.
14. **T, not written:** `THUMBS_VERSION > 0` as a compile-time assert (`crates/render/src/lib.rs`); replacing the 20-byte
    JPEG test in `crates/render/src/maker.rs` (around line 285, `whole.get(..20)`) with a file cut inside SOF; a borders
    case in `crates/library/tests/writeback.rs`.

## Screenshots

1. **F3:** run `just shots` against the current reference (`tools/shots/reference/3edb43b…`, 51 commits behind HEAD) to
    show the old scenarios still match. Then `just shots --record` and commit the new reference as
    `tools/shots/reference/<sha10>`. Not run.

## Performance gate

1. `just perf --budgets` for the F1 comparison against `docs/perf/baseline.json`, as restated in `docs/perf/budgets.json`.
    Check the current result, since the catalog has changed since the budgets were measured.

## By-hand checks

1. Not run by this audit: dismiss the import entry, add a corrupt file, rescan (expect "1 since dismissed", and it
    survives opening the inbox); a thumbnail Retry does not re-badge import; a failing file mid-pass shows the link
    within about 2 s and opens the updating list; a write-stopped state clears after a healthy restart; a sub-folder
    rescan keeps the root's other unreadable folders; the keyboard-only inbox; a dismissal during a CLI import sticks.
