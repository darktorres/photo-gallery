# Notifications ledger: what is still open

The notifications plan was implemented at `e634c83`. This ledger lists what is still not done or incomplete.

It was first checked on 2026-11-14 at HEAD `472eaf5`, then validated row by row by four independent reviews. It was
revalidated on 2026-11-14 at HEAD `62abf68`: each row's cites were re-read, the proposed fixes were challenged by
four read-only reviews, and the corrections are in the rows below. Those verdicts come from reading code. Nothing
was built or run for them. Items marked *read* were found by reading the code and have not been reproduced.

A row stays while any part of its proper fix is missing. Rows whose fix is in and whose test passes are removed.

The full workspace suite should pass now: `facets_filter` failed at `472eaf5` because the fixture wrote analysis
version 1 against `IMPORT_VERSION` 2. `62abf68` makes the fixture write the current version. Re-run the suite to
confirm. `just qmltest` passed (100 tests) at `472eaf5`.

## Summary

| Section | Rows | Notes |
|---|---:|---|
| Behaviour | 1 | N-47: cache recovery notices clear each other. |
| Tests | 7 | Missing tests. Three rows are proposed for closing or narrowing. N-12 waits on D1. |
| Waiting on a person | 12 | Decisions D1 to D8, expanded at the end. |
| Pending | 3 | Performance and screenshot runs not made. Each needs approval (D10, D11). |
| Not reviewed | 1 | By-hand checks. Two become tests; the rest need a person at the app (D12). |
| Total | 24 | |

Kind legend: `incorrect` is wrong behaviour; `missing` is a fix that is absent; `shortcut` is a fix that is present but
not the proper one; `untested` is a fix without a test that would catch its regression; `undeclared-deviation` is code
that differs from the plan without a recorded decision; `stale-doc` is a document that says something false or points
at a source that no longer exists; `pending` is work not done.

## Behaviour

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-47 | Found in revalidation (of N-39) | incorrect | `crates/thumbcache/src/types.rs:111`, `:113`; `crates/model/src/notice.rs:1140-1141`; the event docs at `types.rs:69`, `:73` | `TouchesWritten` and `Stored` both emit `Resolve(Trigger::CacheRecovery)`. Both `CacheTouchesNotWritten` and `CacheNotStored` have `Resolver::CacheRecovery`. So a successful touch write clears a kept not-stored notice, and the reverse. The event docs say each success clears its own failure's notice: "the first time after" that failure. | Give each pair its own trigger and resolver (for example `CacheTouchesRecovered` and `CacheStoreRecovered`). Add a test that a touch success leaves a not-stored notice in place. Update `resolver_table.golden`. | **Open.** Decision D7 only if the cross-clear is intended. |

## Tests

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-12 | Feature plan test 3 | untested | `crates/catalog/src/inbox.rs:369-377` (upsert) | The dedup test cannot fail as written: no kept state carries `detail`. The upsert is already identity-keyed and overwrites `data` in place, so a same-key post with changed detail already yields one row with the new detail. The test that matters checks whether that change bumps `rev` and clears `read`. | Write that test once D1 is decided: post the same key twice with a changed detail, and assert the `rev`/`read` outcome D1 chose. | **Blocked** by D1. |
| N-13 | Feature plan test 4 | untested | `crates/catalog/src/schema.rs:338` (`folder_failures_stamp`), `:754` (`files_added`), `:1127-1128`; `lifecycle.rs:1092-1135`; `ledger.rs:626-690` | Tested: the rating, border-mark and queued-write stamps; the cut and border-side stamps; the import ledger's and hidden trigger's effects; the analysis delete's stamp and pending. `folder_failures_stamp` is now tested by `a_new_folder_failure_is_stamped_past_the_older_ones` (`roots.rs`, uncommitted). Still not tested: `files_added`'s `pending` and `seq` effects. | Extend the `files_added` test to its `pending` and `seq` effects. For the guard, extend `carried_is_every_table_of_the_current_schema` (`schema.rs:850`), which already enumerates `sqlite_schema`. A name list proves a name is listed, not that a test exercises the trigger, so the remaining test is still needed. | **Partly done.** `folder_failures_stamp` passes (`cargo test -p pg-catalog`, 2 of 2). Uncommitted. |
| N-17 | Fix plan test 1 | untested | `crates/scan/src/lib.rs:1448-1464` (`HOLD_BOUND`, `WriteHolds` in `crates/catalog/src/fixture.rs:60-73`) | The premise does not apply. A timed 200k write-hold test already exists and runs under `cargo test`. The scan path never posts or dismisses notices (`post_notice` and `dismiss_all` are called from `crates/catalog/src/inbox.rs` and `crates/app/src/inbox.rs`), so "a dismissal and a post in one batch" has no scan-side meaning. Rows per transaction are not bounded by batch size: `BATCH` is 10,000 entries (`lib.rs:162`), and a folder larger than a batch is written whole (`lib.rs:31`; test at `:1335`). | Close the row. A test is needed only if a notice interaction is added to the scan path. | **Proposed close.** No decision needed; say so if you disagree. |
| N-18 | Fix plan test 2 | untested | `crates/scan/src/lib.rs:1314-1332` (version bump), `:1302-1307` (stamp moved), `:1425-1441` (lease taken) | Retry has a scan-path meaning: it bumps `files.seq`, which the batch is designed to ignore (`lib.rs:34-35`). The version-bump test exists, but it asserts only `pending == 2`, not the stored analysis rows. The error-return case is in (`:1425-1441`). | Strengthen the version-bump test to assert the stored analysis rows. Add a Retry case, unless `:1302-1307` already covers it (check first). | **Open.** |
| N-21 | Fix plan test 5 | untested | `crates/catalog/src/ledger.rs:974-985`, `lifecycle.rs:1044-1046`, `queue.rs:575-576`, `browse/read/tests.rs:658-676`, `writers.rs:778` | EXPLAIN tests already exist in five places, using `raw::query_plan` (`fixture/raw.rs:48`). The missing case is 200k files with 50k hidden rows. | Add a small-fixture plan test with the same WHERE clauses, matching the existing tests. Do not run ANALYZE: production never runs it, so the test would plan differently from what users get. The 200k timing belongs in perf. | **Open.** |
| N-24 | Fix plan test 8 | untested | `crates/thumbs/src/lib.rs:505-515` (cache full), `:519-534` (stopped), `:339-340` (ledger written, then error returned) | The "no seam" premise is wrong. `pipeline::batched` takes a flush closure that returns `Result` (`crates/jobs/src/pipeline.rs:248`). `LedgerWritesFail` exists (`crates/catalog/src/fixture.rs:42-49`). Thumbs has `cfg(test)` fault hooks (`lib.rs:367-384`). | First try a SQL-level fault: a trigger on the thumbs ledger that RAISEs on insert, so `put_batch` fails without any seam. Confirm thumbcache propagates that error (`crates/thumbcache/src/lib.rs:198`). Fallback: `LedgerWritesFail` or the existing fault hooks. The repo already uses process-wide faults (`LEDGER_FAULT`, `pg_borders::fault`), so a test-only fault is within convention. | **Open.** Unverified: that thumbcache propagates the error. |
| N-29 | Fix plan test 13 | untested | `crates/library/tests/writeback.rs` | The borders half is not a gap. Writeback has no borders path: `WriteKind` has only `Rating` and `Turns` (`crates/model/src/write.rs:28-33`). Borders are cropped copies, covered by `crates/library/tests/crop.rs:81` (`cropped_copies_cut_the_borders_and_leave_the_originals`, which needs ExifTool and skips without it). The "20-byte JPEG" test was not found by name in `writeback.rs`. | If D9 agrees, close the borders half by citing `crop.rs:81`. Verify the 20-byte JPEG test before closing the row. | **Decision D9.** |

## Waiting on a person

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-30 | Fix plan 8 (D1) | undeclared-deviation | `crates/desktop/qml/InboxPanel.qml:204` (`currentIndex: -1`), `:148-151` (the reason), `:213-240` (keys drive `selected`) | The option to bind `currentIndex` came from the old fix plan (item 10, `1468581^:docs/temp/NOTIFICATIONS_PLAN.md`); the design never mentions it. The code deliberately keeps selection off the view's current row: a ListView applies model changes late, so a key press in between would act on the wrong entry. Keys work through `selected`, which `tst_inbox_panel.qml:126-142` checks. | Keep the code, and record the decision in the plan. Check whether `Accessible.selected` on each row (`:285`) is enough for assistive technology. | **Decision D3.** |
| N-31 | Fix plan 8 (E) | undeclared-deviation (the ledger's premise was wrong) | `crates/log/src/lib.rs:295` (`log` locks), `:310` (`panic` try_locks), `:297-299` (contended records go to stderr), `:183-200`, `:328-333` (`HOLDING` re-entry guard); design in `006dccd` `docs/NOTIFICATIONS_PLAN.md:161`, `:163` | The design says the normal writer is a Mutex serializing records. The `try_lock` with raw fallback is for the panic writer (`:163`). The ledger merged the two. Code matches the design. A `try_lock` in `log()` would send contended records to stderr, which a GUI build never shows. Deadlock is not reachable today: no file-writing code calls `log`, and `HOLDING` covers re-entry. | Keep the code. Correct the plan's description. Optional: a test that a `log` call inside the write section is caught by `HOLDING`. | **Decision D4.** |
| N-32 | Design table | undeclared-deviation | `crates/desktop/src/notices_bridge.rs:111-112` (`setViewing`), `:115-155` (typed per-action calls), `crates/desktop/src/shell_bridge.rs:247` (`Shell.reveal`) | The design names `markRead`, `reveal(fileId)` and `perform(action)` (`006dccd` lines 75, 171). Code: `markRead` exists internally, not as a QML call (`setViewing` calls `mark_shown_read`, `notices.rs:452-470`, which calls `Inbox::mark_read`). `reveal` takes the file id directly. `perform` is absent, replaced by typed per-action calls, which is the convention in `filters.rs`. | Keep the code. Rewrite the design entries to match. | **Decision D5.** |
| N-33 | Design table | undeclared-deviation | `crates/desktop/src/shell_bridge.rs:40-41` (read-only `Shell.devMode`), `shell.rs:90-93` (`Command::SaveDevMode`); `settings_bridge.rs` holds only `writeBack` and `realSize` | The design says `devMode` on `settings_bridge.rs`. It contradicts its own precedent, "Dev mode follows show_borders" (`006dccd` line 168), which is read-only and saved through a Command, as the code does. The design's test plan (line 176) also names `devMode` in `Settings.qml`. | Keep the code. Fix both design places. | **Decision D5.** |
| N-34 | Design table | undeclared-deviation | `crates/app/src/app.rs:172-176`; `crates/thumbcache/src/types.rs:76` (`WalNotMerged`, still a live cache event); `crates/model/src/notice.rs:535` (`CacheWalNotMerged`) | The design says `WalNotMerged`, but also says the keeper-close notice "gets its own wording, since it is not a cache event" (line 63). Behaviour matches: logged only, after the inbox closes, nothing stored. The keeper may be the catalog's WAL, so the rename changes the concept, not just the label. | Rename the design entry to `CatalogNotClosed`. The name `WalNotMerged` stays with the live cache event. | **Decision D5.** |
| N-35 | Design table | undeclared-deviation | `crates/catalog/src/inbox.rs:314` (`dismiss_all` deletes events); `:339-345` (`prune_inbox`, states only); `:66` (`DISMISSED_FOR`) | The design says dismissed events are kept until purged after 30 days. The code deletes event rows at dismissal. The earlier cite `:252` was inside `inbox_marks`, not the delete. Events have NULL keys, and NULLs never conflict in SQL, so a kept dismissed event could not suppress a re-post anyway. | Change the design to match the code. | **Decision D2.** |
| N-36 | Design table | undeclared-deviation (the ledger's framing was wrong) | `crates/catalog/src/inbox.rs:365` (key is code, root and part), `:369-377` (upsert conflicts on key, overwrites `data` at `:373`, `rev` bumps when `data` changes at `:374-376`); `:393-395` (`key_text`) | The code is already identity-keyed and updates in place. The earlier cite `312-314` was stale. The module doc says news means "its data changed" (`:28-31`, `:102-104`). The open question is only the news rule: does a detail-only change count as news? | Keep the current behaviour and amend the design, if D1 agrees. Then write the N-12 test. | **Decision D1.** |
| N-37 | Design table | undeclared-deviation | `crates/catalog/src/ledger.rs:663-679` (`a_dismissed_entry_that_comes_back_stays_when_read`; the test moved from `:656-671`), `:845-849` (`quiet`), `:862`; `schema.rs:376-380`, `:405-410`, `:409` (`SET error = excluded.error`) | A bump can change import results: `analyze.rs:22-26` says the version is "bumped when what is stored changes", and version 2 changed the PDQ and ThumbHash inputs. For a same-bytes failure with an identical error, the serial is kept and nothing is news, which is right. Gap: a same-bytes failure with changed error text overwrites `error` (`schema.rs:409`) without news, and no test covers it. | Keep quiet for an identical error. For changed error text, decide (D1, part b). | **Decision D1.** |
| N-38 | Design table | undeclared-deviation | `crates/catalog/src/schema.rs:99` (`notices.root` ON DELETE CASCADE), `:328` (`folder_failures` cascade), `:424` (`PRAGMA foreign_keys = ON`); `inbox.rs:370` (notices carry the root); `roots.rs:79`, `:95` (`DELETE FROM roots`) | The design says a trigger on `roots` (`006dccd` lines 52, 70). The code cascades, which covers both removal paths. A notice with a NULL root would survive, but nothing is posted without a covering root. No test was found that removing a root clears its notices. | Change the design to say cascade. Add a removal test if none exists (check first). | **Decision D5.** |
| N-39 | Design table | undeclared-deviation | `crates/thumbcache/src/types.rs:64-83` (seven events), `:98-121` (`Event::effect`) | The design says five events with `into_notice`. `into_notice` exists only in history (`a760ac6:docs/NOTIFICATIONS.md:67`, `e634c83:docs/NOTIFICATIONS_PLAN.md:109`). The code has seven events. Five post and two resolve; the two resolutions are wrong as they stand (N-47). | Accept seven events and amend the design. The N-47 fix is separate. | **Decision D6.** |
| N-40 | Design table | undeclared-deviation | `crates/client/src/controller/panels.rs:80` (`Vec`), `:126-129` (`PanelOpened` retains, then pushes), `:15-24` (`PanelKind`, 8 variants), `:28` (`ALL`) | The design says `ArrayVec<PanelKind, 8>`, "arrayvec is already a dependency" (`006dccd` line 169). No no-allocation requirement is stated. Duplicates are impossible by construction, so the length is bounded. The design also lists `RatingPopup`, which the code lacks. | Accept `Vec`, amend the design, and add `debug_assert!(modals.len() <= PanelKind::ALL.len())`. | **Decision D5.** |
| N-41 | Design table | stale-doc | `docs/temp/NOTIFICATIONS.md` (removed); design history: `006dccd:docs/NOTIFICATIONS_PLAN.md` (lines 52, 63, 169), `a760ac6:docs/NOTIFICATIONS.md:67`, `e634c83:docs/NOTIFICATIONS_PLAN.md:109`, `56bd08c:docs/temp/NOTIFICATIONS_PLAN.md:51-53` (a deviation table, removed in `1468581`) | The "design says" column has no source in the tree. The design exists in four revisions, and they disagree (N-39). The label "undeclared-deviation" is wrong for rows that were declared in a table and then dropped. Precedent: `28024be` moved REFACTOR_DESIGN into `docs/reference`. | Restore one named revision to `docs/reference/`, with its commit in the header. Re-cite each row. Relabel as stale-doc. | **Decision D8.** |

## Pending

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-43 | Performance gate 1 | pending | `docs/perf/baseline.json` (commit `1f26831`); `Justfile:171-172`; `tools/linux.py` | `just perf` has not run since the thumbs-pass restructure. It runs callgrind inside Docker, which counts instructions. The baseline records no host. | Run `just perf` once and record the result. On a tolerance miss, investigate. Do not re-baseline without a decision: `budgets.json` says "Used once", and `tools/perf_check.py:9-13` says a miss "is not re-baselined with --update". | **Decision D10.** |
| N-44 | Performance gate 2 | pending | `docs/perf/budgets.json` | `just perf --budgets` (the F1 comparison) has not run. HEAD is 118 commits past the baseline `1f26831`, and 13 of those commits touch the thumbs, thumbcache, library or render paths. A budget miss may come from later work, not from F1. | Run it after N-43, and record the result with that caveat. | **Decision D10.** |
| N-45 | Screenshots 1 | pending | `tools/shots/reference/c44fe07347`; `tools/compare_shots.py:406-425`; `Justfile:145` (`_qt`) | `compare_shots.py` picks the first commit in `rev-list --date-order HEAD` that has a reference. It picks by history position, not by name. `c44fe07347` is 27 commits behind `472eaf5` (count verified) and 51 behind current HEAD. A new reference is picked up with no rename. `just shots` depends on `_qt`, which is a Qt build. | Record the reference after the UI rows are settled (N-46 included), so it is not stale on the next UI commit. Commit it as `tools/shots/reference/<sha10>`. | **Decision D11.** |

## Not reviewed

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-46 | By-hand checks 1 | pending | the running app; tests named below | Covered already: a thumbnail Retry does not re-badge import (`ledger.rs` near `:865`); dismiss, corrupt file, rescan shows "1 since dismissed" (`ledger.rs:633`, `:676`). Probably covered: a sub-folder rescan keeps the root's other unreadable folders (`roots.rs:361`, `:623`; not read in full). Partly covered: a write-stopped state clears after a start (`a_start_clears_the_writers_states_first`), but the healthy-restart reset (`crates/app/src/library.rs:129-130`) is untested. Uncovered: a dismissal during a CLI import (no `dismiss` call under `crates/cli`). Manual only: the keyboard-only inbox, and the ~2 s link timing when a file fails mid-pass. | Write two tests: the healthy-restart reset, and a dismissal that sticks across two catalog connections. Remove the covered items from the by-hand list. Keyboard and timing stay manual. | **Decision D12** for the two manual items. The two tests need no decision. |

## Decisions needed from you

Each decision lists the options, a recommendation, and what follows from your answer. Rows not listed here need no
decision: their work is clear from the row.

### D1. What counts as news for a kept notice (N-12, N-36, N-37)

- **Question:** when a kept notice's detail changes, does that count as news (bump `rev`, mark it unread)?
- **Options:**
  - (a) Yes: any change to the stored data is news. This is what the code does now (`inbox.rs:374-376`), and it
    matches the module doc.
  - (b) Only the summary fields are news. A detail-only change is overwritten silently.
- **Second part:** a same-bytes failure whose error text changed. Quiet (keep the serial, overwrite the text), or news?
- **Recommendation:** (a) for data, and quiet for a same-bytes re-failure. Check that the error text does not contain
  volatile content (paths with timestamps, counters), or the quiet rule will still churn.
- **If you choose (a):** amend the design, and write the N-12 test to assert the `rev` bump on a detail change.

### D2. Dismissed events: delete at dismissal, or keep 30 days (N-35)

- **Options:**
  - (a) Delete the event row at dismissal (the code).
  - (b) Keep it for 30 days (the design).
- **Recommendation:** (a). A kept event gives no dedup, because events have NULL keys and never conflict. Keeping it
  would only add rows.
- **What would change my mind:** if the same cause re-posts a dismissed event over and over, the fix is to make that
  notice a kept state (its lifecycle), not to retain event rows.
- **If you choose (a):** amend the design. Nothing else changes.

### D3. Keyboard selection in the inbox (N-30)

- **Options:**
  - (a) Keep selection off the view's current row, and record that in the plan (the code).
  - (b) Bind `ListView.currentIndex` to the selected key's row.
- **Trade-off:** (b) may help assistive technology that reads the current item. It risks the late-model-change problem
  that `InboxPanel.qml:148-151` describes.
- **Recommendation:** (a), unless a screen reader check shows it needs the current item. Rows already set
  `Accessible.selected`.

### D4. The logger's lock (N-31)

- **Options:**
  - (a) Keep the blocking Mutex on the normal writer, and correct the plan. This is what the design says.
  - (b) Use `try_lock` with stderr fallback on the normal writer. Rejected: a GUI build has no console, so contended
    records would never reach the file.
  - (c) Add a test that a `log` call inside the write section is caught by the `HOLDING` guard.
- **Recommendation:** (a), with (c) as an optional addition.

### D5. Design-only corrections (N-32, N-33, N-34, N-38, N-40)

For each row, the code already does the sensible thing. The proposal is to accept the code and rewrite the design.

- **N-32:** typed per-action calls, `setViewing`, and `Shell.reveal(fileId)`. Drop `perform` and the QML `markRead`.
- **N-33:** `Shell.devMode` is read-only and saved by a Command, as `show_borders` is. Fix both design places.
- **N-34:** rename the design entry to `CatalogNotClosed`.
- **N-38:** cascade on `notices.root` and `folder_failures.root`, not a trigger on `roots`.
- **N-40:** `Vec` ordered set, with the `debug_assert`.
- **Recommendation:** accept all five. The design is not in the tree (D8), so the rewrite happens in the restored
  copy.
- **Alternative:** change any of these to match the design. I'd only do that for N-38, if a notice can exist without a
  root.

### D6. Cache events (N-39)

- **Options:**
  - (a) Accept seven events, and amend the design.
  - (b) Restore five events. Rejected: `TouchesWritten` and `Stored` are needed to clear the failures they recover from.
- **Recommendation:** (a). The N-47 fix is separate.

### D7. Cache recovery pairing (N-47)

- **Question:** should a touch success clear a not-stored notice (and the reverse)?
- **Options:**
  - (a) Each success clears only its own failure's notice. Needs a separate trigger and resolver per pair. This changes
    `resolver_table.golden`.
  - (b) Keep the shared trigger, and document the cross-clear as intended.
- **Recommendation:** (a). The event docs say "the first time after" its own failure, which describes (a). A touch
  success only shows that the cache file is writable. It does not show that thumbnails can be stored.

### D8. Which design revision to restore (N-41)

- **Options:**
  - (a) `e634c83:docs/NOTIFICATIONS_PLAN.md`: the version the code was built against.
  - (b) `006dccd:docs/NOTIFICATIONS_PLAN.md`: the first version, with the design table.
  - (c) `56bd08c:docs/temp/NOTIFICATIONS_PLAN.md`: the deviation table, which lists these rows with "Each row needs a
    decision."
- **Recommendation:** (a). Its "into_notice" wording is what N-39 measures. Restore it to `docs/reference/` with its
  commit in the header, and re-cite the rows.

### D9. Borders writeback (N-29)

- **Question:** is writing borders back into the originals part of the product?
- **Options:**
  - (a) No. Borders are cropped copies, covered by `crop.rs:81`. Close the borders half of N-29.
  - (b) Yes. That is a missing feature, and it becomes a Behaviour row with its own design.
- **Recommendation:** (a). If (a), first confirm the 20-byte JPEG test exists, since I could not find it by name.

### D10. Performance runs (N-43, N-44)

- **Question:** may I run `just perf` and `just perf --budgets` now?
- **Concern:** the perf gate runs callgrind in Docker (`Justfile:171-172`). Your notes reserve the pg-dev container for
  the Linux CI pass.
- **Recommendation:** approve one run of each, with the rule that a miss means investigation, not re-baseline. Confirm
  that Docker is acceptable for this run.

### D11. Screenshot reference (N-45)

- **Question:** when do I record the new reference?
- **Options:**
  - (a) Now, after a `just shots` run that needs a Qt build (`Justfile:145`).
  - (b) After the UI rows are settled (N-46 and any N-30 change). Recommended, so the reference is not stale on the
    next UI commit.
- **Also confirm:** your notes say not to run the hour-long static Qt build. `just shots` needs `_qt`. Is a normal native
  Qt build acceptable for it?

### D12. The two manual by-hand checks (N-46)

- **Question:** will you run the two items that need the app?
  - The keyboard-only inbox.
  - The ~2 s link timing when a file fails mid-pass.
- **Options:** (a) you run them and record the result here; (b) drop them from the ledger.
- **Note:** the two tests (healthy-restart reset, dismissal across connections) need no decision. I'd write them.

## Ledger changes in this revision

- The header now states the revalidation commit, and that the verdicts come from reading, not running.
- N-17 is proposed for closing: its premise was wrong.
- N-18, N-21, N-24 and N-13 have corrected cites and premises.
- N-29's borders half is proposed for closing, under D9.
- N-30, N-31, N-32 and N-33 record that the code matches the design, or the ledger's premise was wrong.
- N-36 and N-37 record that the code is already identity-keyed, and that the open question is only the news rule.
- N-38 and N-39 record the cascade and the seven-event design.
- N-41 records that the design exists in history, and the "undeclared" label was wrong.
- N-43 and N-44 record that a budget miss is not re-baselined.
- N-45 records that a new reference is picked up by history position.
- N-47 is new.
