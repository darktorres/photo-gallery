# Notifications ledger: what is still open

The notifications plan was implemented at `e634c83`. This ledger lists what is still not done or incomplete. It was
checked on 2026-11-14 by reading the code at HEAD `472eaf5`, then validated row by row by four independent reviews. Items
marked *read* were found by reading the code and have not been reproduced.

A row stays while any part of its proper fix is missing. Rows whose fix is in and whose test passes are removed. This
pass removed 20 rows and the resolver plan, which is implemented: the resolvers are enforced by triggers (`pg_model`'s
`Trigger` and `resolves`, `pg_app`'s `StateOp::Resolve`). The full workspace suite passes except `facets_filter`, which
fails at HEAD too (`crates/catalog/src/browse/read/tests.rs:702`: the fixture writes analysis version 1 against
`IMPORT_VERSION` 2, so no file is pending-free). `just qmltest` passes (100 tests), including the log panel's keyboard test.

## Summary

| Section | Rows | Notes |
|---|---:|---|
| Behaviour | 0 | |
| Tests | 7 | Missing tests. N-12 waits on the dedup decision (N-36). |
| Waiting on a person | 12 | Decisions, including the nine design rows. |
| Pending | 3 | Performance and screenshot runs not made. |
| Not reviewed | 1 | By-hand checks, which need a person at the app. |
| Total | 23 | |

Kind legend: `incorrect` is wrong behaviour; `missing` is a fix that is absent; `shortcut` is a fix that is present but
not the proper one; `untested` is a fix without a test that would catch its regression; `undeclared-deviation` is code
that differs from the plan without a recorded decision; `stale-doc` is a document that says something false or points
at a source that no longer exists; `pending` is work not done.

## Behaviour

No open rows.

## Tests

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-12 | Feature plan test 3 | untested | `crates/catalog/src/inbox.rs` (`upsert`) | Stored-notice dedup compares the whole JSON. The only variant with a `detail` field is `CacheStartedOver`, an event with no key, which never reaches that compare. No kept state has `detail`, so as written the test cannot fail. | Re-scope first: decide what dedup means for kept states (N-36). Then test it, or drop the test. | **Blocked** by N-36. |
| N-13 | Feature plan test 4 | untested | `crates/catalog/src/schema.rs`; `lifecycle.rs:1092-1135`; `ledger.rs:626-690`; `inbox.rs:464` | Some trigger effects are tested. The test that checks the trigger names only checks that they exist. The triggers were not checked one by one. | Check each trigger against the effect tests above, and add a test for each one that has none. | **Open.** Narrowed by validation. |
| N-17 | Fix plan test 1 | untested | `crates/scan/src/lib.rs:1309`, `:1093` | The nearest test bounds how long one write transaction holds the lock on a 200k walk. No test checks that a dismissal and a post land within one batch write's time. | Add that test. | **Open.** Needs a timed harness across the 200k walk. |
| N-18 | Fix plan test 2 | untested | `crates/scan/src/lib.rs` | The write-back test has no Retry or version bump. No test shows a walk that returns an error gives its lease back. | Add the Retry and version-bump cases, and the error-return case. | **Open.** The error-return case is in; the Retry and version-bump cases are not written. |
| N-21 | Fix plan test 5 | untested | `crates/catalog/src/ledger.rs`; `crates/catalog/src/lifecycle.rs` | Missing: a rotation write-back followed by `--retry`; `EXPLAIN` at 200k files with 50k hidden rows. | Add each remaining test. | **Open.** The `set_import_version` test is in; the other two are not written. |
| N-24 | Fix plan test 8 | untested | `crates/thumbs/src/lib.rs` | No test that the ledger is flushed when the pass errors or is stopped. | Add the test. | **Open.** The stopped case is in; the error path has no seam to inject a failure. |
| N-29 | Fix plan test 13 | untested | `crates/library/tests/writeback.rs` | The 20-byte JPEG test is now a cut inside SOF. `writeback.rs` has no borders case. | Replace the test, and add the borders case. | **Open.** The cut is fixed; the borders case is blocked: writeback has no borders path, which needs a decision. |

## Waiting on a person

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-30 | Fix plan 8 (D1) | undeclared-deviation | `crates/desktop/qml/InboxPanel.qml:204`, `:35` | `ListView.currentIndex` is left at -1, and the selection is held in `selectedKey`. | Either bind the current item to the key (focus and assistive technology need it), or change the plan. | **Waiting on a person.** |
| N-31 | Fix plan 8 (E) | undeclared-deviation | `crates/log/src/lib.rs:240-246`, `:258` | The plan says the writing path keeps its `try_lock`. Only `Logger::panic` uses `try_lock`; `Logger::log` takes a blocking `lock()`. | Either restore the `try_lock` with its raw-line fallback, or change the plan. | **Waiting on a person.** |
| N-32 | Design table | undeclared-deviation | `crates/desktop/src/notices_bridge.rs:111-112`; `crates/desktop/src/shell.rs` | Design says `markRead`, `reveal(fileId)` and `perform(action)`. Code has `setViewing`, `Shell.reveal` and one method per action. `markRead` and `perform` are not in `crates/`. | Change the code, or change the design. | **Waiting on a person.** |
| N-33 | Design table | undeclared-deviation | `Shell.devMode`; `Command::SaveDevMode` | Design says `devMode` on `settings_bridge.rs`. Code has a read-only `Shell.devMode`, saved by `Command::SaveDevMode`. | Change the code, or change the design. | **Waiting on a person.** |
| N-34 | Design table | undeclared-deviation | `crates/app/src/app.rs:172-175` | Design says a keeper-close failure is only logged (`WalNotMerged`). Code only logs it, and only after the inbox has closed, as `CatalogNotClosed`. Nothing is stored. The behaviour matches the design; only the name differs. | Rename the notice to match the design, or change the design. | **Waiting on a person.** |
| N-35 | Design table | undeclared-deviation | `crates/catalog/src/inbox.rs:252` | Design says dismissed events are kept until purged after 30 days. Code deletes the row at dismissal (`inbox.rs:252`). The 30-day purge applies to dismissed states (`:280`). | Change the code, or change the design. | **Waiting on a person.** |
| N-36 | Design table | undeclared-deviation | `crates/catalog/src/inbox.rs:312-314` | Design says stored-notice dedup compares summary fields only. Code compares the whole JSON (`data IS excluded.data`), `detail` included. This decision blocks N-12. | Change the code, or change the design. | **Waiting on a person.** |
| N-37 | Design table | undeclared-deviation | `crates/catalog/src/ledger.rs:649-671` | Design says an `IMPORT_VERSION` bump re-badges the import entry once. The test asserts it does not (`ledger.rs:671`, with the check in `quiet` at `:656`). | Change the code, or change the design. | **Waiting on a person.** |
| N-38 | Design table | undeclared-deviation | `crates/catalog/src/schema.rs` | Design says root removal is cleaned by a trigger on `roots`. Code uses a foreign key with `ON DELETE CASCADE`, which has the same effect. | Change the code, or change the design. | **Waiting on a person.** |
| N-39 | Design table | undeclared-deviation | `crates/thumbcache/src/types.rs` (`Event`; `Event::effect`) | Design says five cache events with `into_notice`. Code has seven events (`TouchesWritten`, `Stored`) and `Event::effect`. `into_notice` appears nowhere in the code. | Change the code, or change the design. | **Waiting on a person.** |
| N-40 | Design table | undeclared-deviation | `crates/client/src/controller/panels.rs:80` | Design says `Panels.modals` is an `ArrayVec<PanelKind, 8>`. Code uses `Vec<PanelKind>`. | Change the code, or change the design. | **Waiting on a person.** |
| N-41 | Design table | stale-doc | `docs/temp/NOTIFICATIONS.md` (removed) | The "design says" column has no source in the repo. The design text was introduced in `006dccd`, the commit that added the notifications plan. The removed `NOTIFICATIONS.md` never contained it, so the table compares against history only. | Decide where the design lives. Then point each row at its source, or drop the rows that have none. | **Waiting on a person.** |

## Pending

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-43 | Performance gate 1 | pending | `docs/perf/baseline.json` (commit `1f26831`) | `just perf` has not run since the thumbs-pass restructure. Nine later commits touch the thumbs, runner, thumbcache or render paths, including `e065886` and `a70d174`. | Run `just perf` and record the result. | **Not run.** |
| N-44 | Performance gate 2 | pending | `docs/perf/budgets.json` | `just perf --budgets` (the F1 comparison) has not run. The catalog has changed since the budgets were measured. | Run it and record the result. | **Not run.** |
| N-45 | Screenshots 1 | pending | `tools/shots/reference/c44fe07347` | The newest reference, `c44fe07347`, is 27 commits behind `472eaf5`. `tools/shots/reference/3edb43b63b` is 98 commits behind and is not the newest. `tools/compare_shots.py` compares against the newest reference. | Run `just shots`, then `just shots --record`, and commit the new reference as `tools/shots/reference/<sha10>`. | **Not run.** |

## Not reviewed

| ID | Source | Kind | Where | Problem | Proper fix | Status |
|---|---|---|---|---|---|---|
| N-46 | By-hand checks 1 | pending | the running app | Not run by the audit: dismiss the import entry, add a corrupt file, rescan (expect "1 since dismissed", and it survives opening the inbox). A thumbnail Retry does not re-badge import. A failing file mid-pass shows the link within about 2 s and opens the updating list. A write-stopped state clears after a healthy restart. A sub-folder rescan keeps the root's other unreadable folders. The keyboard-only inbox works. A dismissal during a CLI import sticks. | Run them by hand, and record the results. | **Not run.** |
