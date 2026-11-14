# Notifications ledger: what is still open

The notifications plan was implemented at `e634c83`. This ledger lists what is still not done or incomplete. It was
checked on 2026-11-14 by reading the code at HEAD `472eaf5`, then validated row by row by four independent reviews whose corrections are applied. Nothing was built or run, except
`cargo test -p pg-model the_documented_resolver_table`, which passes. Items marked *read* were found by reading the code
and have not been reproduced. The reference for the code is [NOTIFICATIONS.md](../temp/NOTIFICATIONS.md).

A row stays while any part of its proper fix is missing. Rows whose work is done are removed. Work done in this pass:
the GUI-thread reads (C2), the scan lock, the `PassFailed` item (its status-bar text is live) and the bidi-isolation
test, which exists.

## Summary

| Section | Rows | Notes |
|---|---:|---|
| Behaviour | 9 | Open fixes. N-03 is closed by the resolver plan (N-42). |
| Tests | 20 | Missing tests. N-12 waits on the dedup decision (N-36). |
| Waiting on a person | 12 | Decisions, including the nine design rows. |
| Pending | 4 | Plan work, and performance and screenshot runs not made. |
| Not reviewed | 1 | By-hand checks, which need a person at the app. |
| Total | 46 | |

Kind legend: `incorrect` is wrong behaviour; `missing` is a fix that is absent; `shortcut` is a fix that is present but
not the proper one; `untested` is a fix without a test that would catch its regression; `undeclared-deviation` is code
that differs from the plan without a recorded decision; `stale-doc` is a document that says something false or points
at a source that no longer exists; `pending` is work not done.

## Behaviour

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-01 | Feature plan 1 | missing | `crates/desktop/src/library.rs:132`, `:158` | While `failed > 0` the inbox is not poked on `Change::Progress`. `on_pass` only calls `refresh_progress` on progress, and the inbox poke fires only on job rounds. The "N failed" link can lag behind the pass by up to the inbox's 30 s poll (`crates/app/src/inbox.rs:62`). | Poke the inbox on `Change::Progress` while `failed > 0`. | **Open.** |
| N-02 | Feature plan 2 | missing | `crates/crop/src/lib.rs:32`; `crates/model/src/notice.rs` (`Cropped`) | `crop::Stats` keeps only the first failure (`error: Option<FileFailure>`), so `Notice::Cropped` carries only `first_error`. | Add `keep_failure` to `crop::Stats` (as `crates/jobs/src/failure.rs` does for the thumbs and scan passes), and carry the list into `Cropped`. Decide whether the `refused` field an earlier note named is needed; no document defines it. | **Open.** |
| N-03 | Fix plan 1 | incorrect | `crates/desktop/src/shell.rs:497` (`Command::SaveSession`); `crates/model/src/notice.rs:1023`; `crates/desktop/src/controller.rs:52` | A successful session save resolves only `session-not-saved`. A failed startup read (`session-not-read`) stays on the inbox until dismissed, although the model says the next successful save clears both. | The session save emits `Op::Session`, which resolves both session kinds (see the resolver plan, N-42). Test: a read failure, then a successful save, leaves no entry. | **Open.** *read* |
| N-04 | Fix plan 2 | incorrect | `crates/catalog/src/schema.rs:354` (`analysis_import`), `:283` (`files_failures_hidden`); `crates/catalog/src/sql.rs:8` (`stale!()`) | `analysis_import` sets `hidden` from `files.missing` and ignores `pending`. The `hidden` trigger fires only when `pending` or `missing` changes. A failed analysis whose size differs from the file's leaves `pending` at 1, so the import row stays visible while the grid's failed filter (`pending = 0`) hides it. It clears at the next analysis. The grid side is not traced. | After any analysis insert, `hidden` equals `missing OR (import AND pending)`. Test: a pending file, analysed with a stale size, has its import row hidden. | **Open.** *read* |
| N-05 | Fix plan 3 | incorrect | `crates/scan/src/lib.rs:528`, `:566`, `:577`, `:591` | The walk renews its lease at each folder boundary and after each batch write. One folder listing that outlasts `SCAN_LEASE_SECS` (60 s) renews nothing, so a second walk can take the lease while the first is still writing. The comment at `:566` says a walk held up anywhere renews the lease. The renewal doc at `crates/scan/src/lib.rs:140` also claims renewal during a slow listing. `writers.rs:83` ignores the heartbeat row count, so a batch written after expiry is not refused. | Renew inside the listing path, or check the renewal on the batch path and stop. Test: a listing that outlasts the lease does not let an overlapping walk in. | **Open.** *read* |
| N-06 | Feature plan 3; Fix plan 4 | incorrect | `crates/catalog/src/convert.rs:127`; `crates/model/src/notice.rs:847` (`Notice::RETIRED`) | `convert_notify` deletes only `scan-unreadable` and `mount-refused`. Rows of the retired `root-refused` decode as `Unknown` and stay in the inbox. Adding the kind to `convert_notify` helps only catalogs that have not migrated. | Migrated catalogs need a new migration step, or a decision to accept those rows as `Unknown`. | **Open.** |
| N-07 | Fix plan 5 | shortcut | `crates/model/src/notice.rs:569` (`LedgerNotWritten`), `:1007`; `crates/library/src/runner.rs:569` | The type carries a `FilePass`. Only the thumbnails pass posts it (`ledger_written` is called with `FilePass::Thumbs` alone, `runner.rs:569`). The Borders pass also clears it (`runner.rs:546`), and rows from earlier builds exist, so the fix must handle them. | Make the notice thumbnails-only in the type, and regenerate its table in `NOTIFICATIONS.md`. | **Open.** |
| N-08 | Fix plan 6 | missing | `crates/app/src/inbox.rs:392` (`mark_all_read`); `crates/desktop/src/notices_bridge.rs` | The catalog and `Notices` are bounded to the displayed ids, but nothing in the inbox panel calls `mark_all_read`, and the bridge has no such call. Mark-on-read is in `crates/desktop/src/notices.rs:371-388`. | Add the control, or decide the plan does not need it. | **Open.** |
| N-09 | Fix plan 7 | incorrect | `crates/desktop/qml/LogPanel.qml:93-96`; Qt `qquickflickable.cpp:3486` | `scrolled()` runs on `movementEnded`, on a wheel scroll, and from the scroll bar (`LogPanel.qml:96`); there is no key handler. The Qt source fires `movementEnded` whenever any movement ends, including `setContentY`, `cancelFlick` and `timelineCompleted`. Whether a keyboard move from rest reaches it is not traced. | Reproduce first. Then fix it, with a test that a keyboard scroll stops following. | **Open.** *read* |

## Tests

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-10 | Feature plan test 1 | untested | `crates/desktop/tests/qml/stubs/pg/gallery/` | The QML stubs lack `Notices.qml` and `LogModel.qml`, their `qmldir` lines, `devMode` and `globalAction` in the `Shell.qml` stub. `devMode` is a Shell property (`crates/desktop/src/shell_bridge.rs:41`), so it does not belong in `Settings.qml`. | Add them, then run `just qmltest` to see which tests fail without them. | **Open.** Not run. |
| N-11 | Feature plan test 2 | untested | `crates/render/src/lib.rs:14` | No test asserts `THUMBS_VERSION` is nonzero. | A test against the real constant, or a compile-time assert. | **Open.** |
| N-12 | Feature plan test 3 | untested | `crates/catalog/src/inbox.rs:312-314` (`upsert`) | Stored-notice dedup compares the whole JSON. The only variant with a `detail` field is `CacheStartedOver`, an event with no key, which never reaches that compare. No kept state has `detail`, so as written the test cannot fail. | Re-scope first: decide what dedup means for kept states (N-36). Then test it, or drop the test. | **Blocked** by N-36. |
| N-13 | Feature plan test 4 | untested | `crates/catalog/src/schema.rs:956-977`; `lifecycle.rs:1092-1135`; `ledger.rs:626-690`; `inbox.rs:464`; `schema.rs:989` | Some trigger effects are tested (`lifecycle.rs:1092-1135`, `ledger.rs:626-690`, `inbox.rs:464`, `schema.rs:989`). The test at `schema.rs:956-977` only checks that the trigger names exist. The 16 triggers were not checked one by one. | Check each trigger against the effect tests above, and add a test for each one that has none. | **Open.** Narrowed by validation. |
| N-14 | Feature plan test 5 | untested | `crates/catalog/src/schema.rs:82`; `crates/cli/src/lib.rs:218` | There is no `roots` trigger: `notices.root` is a foreign key with `ON DELETE CASCADE`. Removal from the CLI's `remove` is not tested end to end. | Test that the CLI's `remove` drops the root's notices, through `remove_root`. | **Open.** |
| N-15 | Feature plan test 6 | untested | `crates/catalog/src/inbox.rs:453-456`, `:637`, `:507-508` | Single-row `mark_read` tests cover the newest or only row. "Later rows stay new" is checked only through `mark_all_read` (`:507-508`). No test marks an older row and checks that later rows stay new. | Add that test. | **Open.** |
| N-16 | Feature plan test 7 | untested | `crates/catalog/src/fixture/raw.rs:222-235`, `:238-244`; `crates/catalog/src/ledger.rs:732-750` | The fixture has the import pair (`older_import_version` / `start_import_version`) and the borders pair, but no thumbs pair. Thumbs-version behaviour is tested (`ledger.rs:732-750`, through `set_thumbs_version`), so the gap is fixture symmetry, not coverage. | Add `older_thumbs_version` / `start_thumbs_version` beside the other two pairs. | **Open.** |
| N-17 | Fix plan test 1 | untested | `crates/scan/src/lib.rs:1309`, `:1093` | The nearest test bounds how long one write transaction holds the lock on a 200k walk (`:1309`). `a_paused_walk_holds_no_write_lock` (`:1093`) pauses a walk, but posts no notice. No test checks that a dismissal and a post land within one batch write's time. | Add that test. | **Open.** |
| N-18 | Fix plan test 2 | untested | `crates/scan/src/lib.rs:1202`, `:1161` | The write-back test (`:1202`) has no Retry or version bump. The panic test (`:1161`) gives the lease back. No test shows a walk that returns an error gives its lease back. The lease is released on drop (`crates/catalog/src/core.rs:293`). | Add the Retry and version-bump cases, and the error-return case. | **Open.** |
| N-19 | Fix plan test 3 | untested | `crates/app/src/inbox.rs:202-220` (`Work::state`); tests at `:1172`, `:1202`, `:1238` | Nothing pins how a `Post` coalesces past a same-key `Dismiss` or `MarkRead` queued between two posts. `Work::state` merges a Post only with an earlier queued Post, and the merged op takes the newest sequence number, so it lands after an intervening mutation. The tests cover kind ordering, clear supersession and repeated posts, but not an intervening mutation. | Decide the outcome, then test it. | **Open.** |
| N-20 | Fix plan test 4 | untested | `crates/app/src/inbox.rs:1636` | `a_clear_of_nothing_writes_nothing` checks only `data_version`, so it passes whether or not the clear takes the write lock. | Hold `BEGIN IMMEDIATE` on another connection and check the clear still returns. | **Open.** |
| N-21 | Fix plan test 5 | untested | `crates/catalog/src/ledger.rs:649` (the import test); `crates/catalog/src/lifecycle.rs` (`set_import_version`) | Missing: a rotation write-back followed by `--retry` (the nearest test covers a metadata write-back by rating); `set_import_version` firing the hidden trigger only for rows whose flags change, and the `hidden` trigger guard at `schema.rs:283-284`; `EXPLAIN` at 200k files with 50k hidden rows (no `50_000` fixture in `crates/catalog/src`). A folder removed and re-added is already tested (`crates/catalog/src/roots.rs:397-402`), so it is dropped from this row. | Add each remaining test. | **Open.** |
| N-22 | Fix plan test 6 | untested | `crates/catalog/src/inbox.rs:652` | `states_are_never_pruned_and_an_update_prunes_nothing` runs its update check at exactly the bound, so pruning on update would pass it. | Insert `KEPT_EVENTS` and then more first, with a raw `INSERT`: `post_notice` prunes on each insert, so it cannot exceed the bound. `inbox.rs:656-660` does the same for states. | **Open.** |
| N-23 | Fix plan test 7 | untested | `crates/thumbcache/src/writer.rs:184-209` (`Failing`); `crates/thumbcache/src/types.rs:138-147` | The events are tested: `types.rs:138-147` checks that each clears its failure's key. The writer's `Failing` logic, which emits `TouchesWritten` and `Stored` after a failure, has no test. | Add a writer test that a failure followed by a success emits `TouchesWritten` and `Stored`. | **Open.** |
| N-24 | Fix plan test 8 | untested | `crates/thumbs/src/lib.rs` | No test that the ledger is flushed when the pass errors or is stopped. The nearest tests cover unwritable ledgers. | Add the test. | **Open.** |
| N-25 | Fix plan test 9 | untested | `crates/app/src/library.rs:125` (`tend`), `:407` | No test that a restart in `tend` posts nothing. The nearest test covers a writer given up, not a restart. | Add the test. | **Open.** |
| N-26 | Fix plan test 10 | untested | `crates/scan/src/lib.rs:855-898`; `crates/catalog/src/roots.rs:348`, `:443` | The reparse tests assert stats, not the ledger. No test shows a refused non-root folder swept by its parent's walk: the refused-folder test uses the root only, and the sweep test covers unreadable folders only. | Add both tests. | **Open.** |
| N-27 | Fix plan test 11 | untested | `crates/app/src/inbox.rs:710` | No test for the once-per-kind decode log. | Add the test. | **Open.** |
| N-28 | Fix plan test 12 | untested | `crates/desktop/src/log_model.rs`; `crates/desktop/src/list_diff.rs`; `tst_log_panel.qml` | `log_model.rs` has no test module, so the seq offset and the marking of displayed rows are untested. `list_diff` is tested (six tests). `tst_log_panel.qml` tests the panel against a stub, not `LogModel`. | Add the log model tests. | **Open.** |
| N-29 | Fix plan test 13 | untested | `crates/render/src/maker.rs:285` (in `472eaf5`); `crates/library/tests/writeback.rs` | The 20-byte JPEG test (`whole.get(..20)`) should be a file cut inside SOF. The cut is at line 285 in `472eaf5`; the working tree has an uncommitted edit that moves it to 290. `writeback.rs` has no borders case. | Replace the test, and add the borders case. | **Open.** |

## Waiting on a person

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-30 | Fix plan 8 (D1) | undeclared-deviation | `crates/desktop/qml/InboxPanel.qml:204`, `:35` | `ListView.currentIndex` is left at -1, and the selection is held in `selectedKey`. | Either bind the current item to the key (focus and assistive technology need it), or change the plan. | **Waiting on a person.** |
| N-31 | Fix plan 8 (E) | undeclared-deviation | `crates/log/src/lib.rs:240-246`, `:258` | The plan says the writing path keeps its `try_lock`. Only `Logger::panic` uses `try_lock`; `Logger::log` takes a blocking `lock()`. | Either restore the `try_lock` with its raw-line fallback, or change the plan. | **Waiting on a person.** |
| N-32 | Design table | undeclared-deviation | `crates/desktop/src/notices_bridge.rs:111-112`; `crates/desktop/src/shell.rs:337` | Design says `markRead`, `reveal(fileId)` and `perform(action)`. Code has `setViewing`, `Shell.reveal` and one method per action. `markRead` and `perform` are not in `crates/`. | Change the code, or change the design. | **Waiting on a person.** |
| N-33 | Design table | undeclared-deviation | `Shell.devMode`; `Command::SaveDevMode` | Design says `devMode` on `settings_bridge.rs`. Code has a read-only `Shell.devMode`, saved by `Command::SaveDevMode`. | Change the code, or change the design. | **Waiting on a person.** |
| N-34 | Design table | undeclared-deviation | `crates/app/src/app.rs:172-175` | Design says a keeper-close failure is only logged (`WalNotMerged`). Code only logs it, and only after the inbox has closed, as `CatalogNotClosed`. Nothing is stored. The behaviour matches the design; only the name differs. | Rename the notice to match the design, or change the design. | **Waiting on a person.** |
| N-35 | Design table | undeclared-deviation | `crates/catalog/src/inbox.rs:252`; `NOTIFICATIONS.md:124` | Design says dismissed events are kept until purged after 30 days. Code deletes the row at dismissal (`inbox.rs:252`), as `NOTIFICATIONS.md:124` also says. The 30-day purge applies to dismissed states (`:280`). | Change the code, or change the design. | **Waiting on a person.** |
| N-36 | Design table | undeclared-deviation | `crates/catalog/src/inbox.rs:312-314` | Design says stored-notice dedup compares summary fields only. Code compares the whole JSON (`data IS excluded.data`), `detail` included. This decision blocks N-12. | Change the code, or change the design. | **Waiting on a person.** |
| N-37 | Design table | undeclared-deviation | `crates/catalog/src/ledger.rs:649-671` | Design says an `IMPORT_VERSION` bump re-badges the import entry once. The test asserts it does not (`ledger.rs:671`, with the check in `quiet` at `:656`). `NOTIFICATIONS.md:223` agrees with the code. | Change the code, or change the design. | **Waiting on a person.** |
| N-38 | Design table | undeclared-deviation | `crates/catalog/src/schema.rs:82` | Design says root removal is cleaned by a trigger on `roots`. Code uses a foreign key with `ON DELETE CASCADE`, which has the same effect. | Change the code, or change the design. | **Waiting on a person.** |
| N-39 | Design table | undeclared-deviation | `crates/thumbcache/src/types.rs:64-84` (`Event`); `types.rs:98` (`Event::effect`) | Design says five cache events with `into_notice`. Code has seven events (`TouchesWritten`, `Stored`) and `Event::effect`. `into_notice` appears nowhere in the code. `NOTIFICATIONS.md:110` matches the code. | Change the code, or change the design. | **Waiting on a person.** |
| N-40 | Design table | undeclared-deviation | `crates/client/src/controller/panels.rs:80` | Design says `Panels.modals` is an `ArrayVec<PanelKind, 8>`. Code uses `Vec<PanelKind>`. | Change the code, or change the design. | **Waiting on a person.** |
| N-41 | Design table | stale-doc | `NOTIFICATIONS.md` | The "design says" column is not in `NOTIFICATIONS.md`. None of the claims in N-32 to N-40 appear there. The design text was introduced in `006dccd`, the commit that added the notifications plan. That file is absent at `472eaf5`, so the table compares against a source that no longer exists in the repo. `NOTIFICATIONS.md` agrees with the code for N-35, N-37, N-38, N-39 and N-40. | Decide where the design lives. Then point each row at its source, or drop the rows that have none. | **Waiting on a person.** |

## Pending

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-42 | Resolver plan (below) | missing | `crates/model/src/notice.rs` (`Resolver`, `Lifecycle::State`); `crates/app/src/inbox.rs` (`StateOp`) | No code reads a state's resolver at runtime. Clears are hand-written: `resolved("listing-failed")` at `crates/desktop/src/gallery/model.rs:101`, through `crates/desktop/src/app.rs:29` and `:34`. The table in `NOTIFICATIONS.md` is generated from the same declarations, so it can't catch a site that clears the wrong thing. | Implement the four steps of the resolver plan. This also closes N-03. | **Open. Plan written, not started.** |
| N-43 | Performance gate 1 | pending | `docs/perf/baseline.json` (commit `1f26831`) | `just perf` has not run since the thumbs-pass restructure. Nine later commits touch the thumbs, runner, thumbcache or render paths, including `e065886` and `a70d174`. | Run `just perf` and record the result. | **Not run.** |
| N-44 | Performance gate 2 | pending | `docs/perf/budgets.json` | `just perf --budgets` (the F1 comparison) has not run. The catalog has changed since the budgets were measured. | Run it and record the result. | **Not run.** |
| N-45 | Screenshots 1 | pending | `tools/shots/reference/c44fe07347` | The newest reference, `c44fe07347`, is 27 commits behind `472eaf5`. `tools/shots/reference/3edb43b63b` is 98 commits behind and is not the newest. `tools/compare_shots.py` compares against the newest reference. | Run `just shots`, then `just shots --record`, and commit the new reference as `tools/shots/reference/<sha10>`. | **Not run.** |

## Not reviewed

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-46 | By-hand checks 1 | pending | the running app | Not run by the audit: dismiss the import entry, add a corrupt file, rescan (expect "1 since dismissed", and it survives opening the inbox). A thumbnail Retry does not re-badge import. A failing file mid-pass shows the link within about 2 s and opens the updating list. A write-stopped state clears after a healthy restart. A sub-folder rescan keeps the root's other unreadable folders. The keyboard-only inbox works. A dismissal during a CLI import sticks. | Run them by hand, and record the results. | **Not run.** |

## Plan: resolvers enforced (N-42)

Problem. Each kept state carries a `Resolver` (`Lifecycle::State(key, resolver)`, `crates/model/src/notice.rs`), but no
code reads it at runtime. Clears are written by hand: `resolved("listing-failed")` at `crates/desktop/src/gallery/model.rs:101`,
through `resolved` and `resolved_part` (`crates/desktop/src/app.rs:29` and `:34`). The table in `NOTIFICATIONS.md` is
generated from the same declarations, so it can't catch a site that clears the wrong thing. N-03 is one such mismatch.

Design:

1. **One trigger type.** `Trigger` in `pg-model`, with one variant per `Resolver` variant: `Op(Op)`, `Pass(FilePass)`,
   `Status`, `CacheRecovery`, `Save`, `RootWalk`, `RootRemoval`. A success site emits a `Trigger`. It never names a kind.
2. **Matching in the model.** `resolves(resolver, trigger) -> bool` decides which kept states a trigger clears.
3. **Writer applies it.** The inbox writer gets `StateOp::Resolve(Trigger)` (`crates/app/src/inbox.rs`, `enum StateOp`).
   It clears each kept state whose resolver matches.
4. **Resolver derived, not stored.** A row's resolver comes from its kind code and part, both already in `notices`. The
   view and edit parts encode an `Op`, so the reverse map from code to `Op` needs a round-trip test. A stored column
   would keep old meanings in old rows after a resolver changes, and would need a migration each time.
5. **Failure and success in one place.** One wrapper per operation, `observe(op, result)`: on `Err` it posts that
   operation's failure notice, on `Ok` it emits `Trigger::Op(op)`. Posting and clearing then share the `Op` value, so a
   failure can't be posted without a path that clears it. Sites without a `Result` emit their trigger directly.
6. **The table comes from the matcher.** `Notice::resolver_markdown` and the writer use the same mapping, so the doc and
   the behaviour can't disagree except at emission sites.

What stays manual. Whether a success path reaches its `observe` call or its trigger. Each trigger gets a test that
emits it and checks what it clears.

Sites (eleven), and their triggers:

- Library runner: round (`crates/library/src/runner.rs:375`, `Op::Round`); ledger write (`ledger_written`, `Pass(pass)`);
  complete pass (cache full, `Pass(Thumbs)`); scan root walk (`RootWalk`); status poll (`Status`).
- Desktop: listing (`gallery/model.rs:101`), row read (`gallery/state.rs:121`), reload (`gallery/state.rs:145`), facets
  (`filters.rs:165`), tree (`folders.rs:183`), session save (`shell.rs:497`, `Op::Session`), views (`shell.rs`), edits
  (`gallery/edits.rs`).
- Preferences save (`crates/app/src/prefs.rs`, `Save`).
- Cache recovery is already a `thumbcache` event (`Event::effect`, `crates/thumbcache/src/types.rs:98`). Root removal is
  the foreign key's cascade, with no emission.

Out of scope: ledger-backed kinds (`thumbs-failed`, `borders-failed`, `import-failed`, `scan-folders-failed`), which clear
from their ledger rows.

Expected behaviour changes. `session-not-read` clears on the next successful save, closing N-03. No trigger should clear
a kind outside its own family; a test per trigger checks that.

Order. (1) Add `Trigger`, `resolves` and the `Op` round trip, with tests. (2) Route the existing `resolved` calls through
`StateOp::Resolve`, with no behaviour change. (3) Convert the sites to `observe` and triggers, and change the session save
to emit `Op::Session`. (4) Generate the table from the matcher and regenerate `NOTIFICATIONS.md`. Build and test once,
after all four steps.
