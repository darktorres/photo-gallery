# UI and QML flow review

This report lists the findings of a flow-based review of the desktop UI: the QML front end, the Rust bridges and the controller it drives. The review started from each user action and each path that ends in a write, a notice or a focus change, and traced it through the code it reaches. It was run as a read-only workflow that produced 281 findings across 74 flows.

**Status: validated.** Each finding was checked against the live code at the cited sites, and against the Qt source under `C:\Qt\6.12.1\Src` where a finding depends on Qt behaviour. Verdicts for the 90 open findings: 77 confirmed, 13 partial, 0 refuted. The 183 findings that were fixed, already fixed or not real have been removed. A confirmed verdict means the mechanism is in the cited code. Where a finding also claims a consequence that was not traced, the note says so. Duplicates are marked and point to the finding they repeat.

## Method

1. A mapping step listed the user flows and the cited sites on each.
2. Each flow was traced from its entry point through the code it reaches, and each finding records a trigger, a consequence and the sites it cites.
3. Each finding was then validated: the cited lines were read in context (about four lines either side), and the claim was traced into the callees and into Qt where needed. Findings that cite lines past the end of the file are recorded as citation errors below; their mechanism was checked only where the code could be found elsewhere.

## Coverage

| Measure                     | Value |
|-----------------------------|------:|
| Flows traced                |    74 |
| Findings open               |    90 |
| Flows with open findings    |    50 |
| Sites recorded as not flows |    18 |

Sites recorded as not flows were not traced; the mapping output gives a reason for each one.

## Summary

These counts cover the 90 open findings.

| Severity | Count |
|----------|------:|
| high     |     2 |
| medium   |    20 |
| low      |    68 |

| Category               | Count |
|------------------------|------:|
| missing-feedback       |    26 |
| wrong-message          |    17 |
| stale-result           |    14 |
| unhandled-close        |     7 |
| focus                  |     3 |
| double-fire            |     4 |
| stale-state            |     3 |
| wrong-target           |     2 |
| wrong-position         |     1 |
| visual-flicker         |     1 |
| unneeded-relist        |     1 |
| stuck-error           |     1 |
| stale-toast            |     1 |
| stale-open-panel       |     1 |
| shadowed-key           |     1 |
| no-window-state        |     1 |
| missing-start          |     1 |
| missing-keyboard-path  |     1 |
| missing-confirmation   |     1 |
| misleading-hint        |     1 |
| keyboard-parity        |     1 |
| invisible-characters   |     1 |

| Verdict   | Count |
|-----------|------:|
| CONFIRMED |    77 |
| PARTIAL   |    13 |
| REFUTED   |     0 |

Duplicates: 3 findings repeat a mechanism already recorded under another ID and are marked as such.

## Citation errors

These findings cite a line past the end of the file. The rest of each finding was checked where the code could be found.

| Finding | Cited site                                |
|---------|-------------------------------------------|
| UI-076  | `crates/desktop/src/filters.rs:270`       |
| UI-206  | `crates/desktop/src/yandex_bridge.rs:449` |
| UI-206  | `crates/desktop/src/yandex_bridge.rs:453` |
| UI-206  | `crates/desktop/src/yandex_bridge.rs:455` |

## Shared fixes

Several Needs bullets name these mechanisms. Each is defined once here, and each finding that names one is fixed by the same design.

- **Outcome path.** Each catalog read or write in the app, client and desktop crates returns a `Result`. An error becomes one `Outcome` (scope, kind, detail), and one mapping turns it into a notice. The failure appears where the user is looking: a toast for an action, an inbox entry for a background write, or an in-panel error state for a listing or details read. No site posts an ad hoc message, and no site only logs.
- **Listing state.** A listing is `Ok(rows)` or `Failed(error)`. QML shows `Failed` as an error state. Earlier rows are kept only when marked stale. A failed read is never replaced by an empty list.
- **Revisions.** Each async request (listing, count, search, mark, reveal) carries a revision. A reply is applied only if its revision is the latest for its target. Older replies are dropped, not merged.
- **Focus policy.** A hidden item cannot hold focus. Overlays and dialogs release focus when they hide and return it to a named owner. Rows and tiles that take keys are focus scopes, so the keyboard and the mouse reach the same actions.
- **Durable queue.** Inbox writes (dismiss, retry, mark) are persisted before they are acknowledged, so a shutdown drain cannot drop a write the user has already made.
- **Start gate.** A failed tree read is a `TreeFailed` state the gate handles. The absence of a folder never leads to `OpenAll`, and session saves wait until the start completes.

## Flows

| ID  | Name                                                      | Findings |
|-----|-----------------------------------------------------------|---------:|
| U01 | Startup and first list                                    |        2 |
| U03 | Window close flushes the session                          |        2 |
| U04 | Command-line test hooks                                   |        3 |
| U05 | Screenshot settle hook                                    |        1 |
| U06 | Crash box on panic                                        |        1 |
| U07 | Live reload of QML (development)                          |        3 |
| U08 | Grid keyboard keys (through the shell)                    |        1 |
| U09 | Global shortcuts (inbox, log, dev mode)                   |        1 |
| U10 | Select a photo by click                                   |        1 |
| U14 | Grid scrollbar drag                                       |        1 |
| U16 | Include subfolders toggle                                 |        1 |
| U18 | Apply a saved view                                        |        2 |
| U20 | Delete a saved view                                       |        1 |
| U21 | Filter bar open and close, facet counts                   |        1 |
| U22 | Filter facet toggles                                      |        3 |
| U27 | Add a folder through the picker                           |        1 |
| U28 | Remove a folder (with confirmation)                       |        1 |
| U29 | Discard a pending removal                                 |        3 |
| U30 | Rescan all folders                                        |        2 |
| U32 | Scan lanes: pause a lane, open its failed count           |        1 |
| U33 | Lane progress polling                                     |        1 |
| U35 | Save cropped copies (dialog)                              |        2 |
| U36 | Cut borders from originals (dialog)                       |        1 |
| U38 | Rate a photo in the viewer (star bar, keys)               |        1 |
| U39 | Turn a photo in the viewer                                |        2 |
| U40 | Mark or unmark a border (viewer N key, HUD)               |        2 |
| U41 | Show or hide borders (viewer B key)                       |        1 |
| U42 | Step through pictures in the viewer (wheel, keys, swipe)  |        1 |
| U44 | Viewer re-render: fit size and region tiles               |        2 |
| U45 | Loupe (10x under the cursor)                              |        2 |
| U47 | Video and animation transport                             |        3 |
| U48 | Find similar from the viewer                              |        3 |
| U49 | Yandex search results load                                |        2 |
| U50 | Pick Yandex results, shift-range pick                     |        1 |
| U51 | Select all of a Yandex group                              |        2 |
| U52 | Yandex full-size preview                                  |        2 |
| U53 | Save picked Yandex results (dialog)                       |        3 |
| U54 | Close the Yandex panel and its preview                    |        2 |
| U55 | Yandex thumbnails settle                                  |        1 |
| U57 | Status bar toast                                          |        2 |
| U58 | Open the inbox (status bell, shortcut)                    |        1 |
| U59 | Inbox select and details                                  |        1 |
| U60 | Copy an inbox entry                                       |        4 |
| U61 | Dismiss an inbox entry, clear all                         |        4 |
| U62 | Retry an inbox entry                                      |        3 |
| U63 | Inbox failed-file list                                    |        1 |
| U64 | Reveal a file from the inbox                              |        3 |
| U65 | Reveal a folder from the inbox                            |        1 |
| U69 | Log view (dev mode)                                       |        1 |
| U74 | Grid thumbnails load and cells display                    |        3 |

## U01: Startup and first list

Entry: `crates/desktop/qml/Main.qml:860 (Component.onCompleted: setScale, sortIndex, relayout, folderModel.reload, shell.reloadList)`

### UI-002 [medium/stale-result] Startup folder fallback overwrites the saved session, including on a failed tree read — CONFIRMED

- Status: Done (build and tests). The desktop dispatches TreeFailed on a failed tree read, and the client gate never opens All folders from absence.
- Needs: Fix. Make a failed tree read a TreeFailed state the start gate handles; never derive OpenAll from absence; hold session saves until the start completes. Files: crates/client/src/controller/start.rs, crates/client/src/controller.rs, crates/desktop/src/folders.rs.
- Verdict: CONFIRMED
- Note: folders.rs:184 falls back to an empty tree on read error. Main.qml:292 sends treeLoaded(false), start.rs:59 emits OpenAll, and listing::update returns SaveSession unconditionally (listing.rs:77). listing_event maps it at once (controller.rs:495), with no started gate, so the start options are saved too.
- Confidence: high
- Trigger: The session folder is a specific folder, and at startup (or after a scan) the tree does not contain it, or the tree read fails (folders.rs falls back to an empty tree and posts TreeFailed). The gate then sends OpenAll.
- Consequence: The remembered folder and the recursion setting are replaced by All folders and saved at once, even though only a read failed. Start options such as --query/--sort are saved into the session too. The save is not deferred until the start is over, unlike the timer saves, so the stored session no longer matches what the user last had. A transient read error permanently changes the user's remembered view.
- Sites: `crates/desktop/src/folders.rs:184`; `crates/client/src/controller/start.rs:59`; `crates/client/src/controller.rs:495`; `crates/client/src/controller.rs:532`; `crates/desktop/src/shell.rs:513`

### UI-004 [low/wrong-message] A failed first listing shows the empty-folder message — CONFIRMED

- Status: Done (build and tests). Rows are kept on a failed read, and the GalleryGrid error label uses ui-listing-failed.
- Needs: Fix. Carry ListingFailed to QML as an error state instead of unwrap_or_default, and keep the rows on failure. Same fix as UI-280. Files: crates/desktop/src/gallery/model.rs, crates/desktop/qml/GalleryGrid.qml.
- Verdict: CONFIRMED
- Note: gallery/model.rs:104 applies an empty list on a failed read (unwrap_or_default) after observe posts ListingFailed. GalleryGrid.qml:97 then shows the empty-state text while the toast reports the error.
- Confidence: medium
- Trigger: The listing read fails at startup or on a folder change (catalog unopened or query error). open_listing applies an empty list.
- Consequence: The grid shows the empty-state text ('No images' or 'No folders') as if the folder were empty. Only the toast reports the failure.
- Sites: `crates/desktop/src/gallery/model.rs:104`; `crates/desktop/qml/GalleryGrid.qml:97`

## U03: Window close flushes the session

Entry: `crates/desktop/qml/Main.qml:142 (onClosing: shell.flush())`

### UI-011 [medium/unhandled-close] Closing mid-save drops the unwritten Yandex results silently — CONFIRMED

- Status: Done (validated).
- Needs: Fix. save_all checks closed and reports unfinished hits through the shared outcome path, recorded in the inbox. Read save_one first. Files: crates/app/src/yandex.rs, crates/app/src/app.rs.
- Verdict: CONFIRMED
- Note: Main.qml:142 flush does not touch Yandex. App::stop only logs (app.rs:156-158). save_all's loop (yandex.rs:372-376) does not check `closed`, and the `closed` checks at 425, 456 and 539 are in other paths. Hits left unwritten are lost without a shown outcome. save_one was not read.
- Confidence: medium
- Trigger: The user starts saving several Yandex results, then closes the window before the save ends (or within the 2 s stop wait). Main.qml:142 flush does not touch Yandex.
- Consequence: The window's exit calls Search::stop (yandex.rs:498-515, which sets closed and waits 2 s at STOP_WAIT yandex.rs:102). save_all (yandex.rs:372) never checks closed, so it keeps writing until the process ends. Hits not yet written are never saved, and the 'saved N, failed M' outcome (yandex.rs:407-411) is never recorded or shown. App::stop only logs a warning (app.rs:156-157). The user gets no sign that the save was incomplete.
- Sites: `crates/desktop/qml/Main.qml:142`; `crates/app/src/yandex.rs:372`; `crates/app/src/yandex.rs:498`; `crates/app/src/yandex.rs:499`; `crates/app/src/app.rs:156`

### UI-012 [low/stale-result] Last file saved at close can be the previous selection — CONFIRMED

- Status: Partial (validated). The controller arms the save on selection. SaveLastFile still reads SHOWN synchronously, and the shutdown order is unchanged.
- Needs: Fix. Flush reads the pending selection, not only the shown file, so a selection made just before close is saved. Files: crates/desktop/src/shell.rs, crates/desktop/src/gallery.rs.
- Verdict: CONFIRMED
- Note: SaveLastFile reads gallery::shown_file() on the window thread (shell.rs:96). SHOWN is set only by publish_shown from the model's queued closures (gallery.rs:58, model.rs:290, 298), so a selection still in the queue at close is not what is saved. Latent, needs a close inside the queue latency.
- Confidence: low
- Trigger: A selection key is pressed (shell.rs:433-439 posts set_current_item to the model lane), and the window closes before that queued closure runs. The flush reads the shown file synchronously.
- Consequence: SaveLastFile (shell.rs:95-97) reads SHOWN, which is updated only by publish_shown in queued model closures (gallery.rs:57-58, gallery/model.rs:290,298). The next start then selects the previous file rather than the one the user last selected. The effect is latent and needs a close within the queue latency of a selection.
- Sites: `crates/desktop/src/shell.rs:96`; `crates/desktop/src/shell.rs:435`; `crates/desktop/src/gallery.rs:58`

## U04: Command-line test hooks

Entry: `crates/desktop/qml/TestHooks.qml:83 run() and :121 runAfterLibraryStart()`

### UI-018 [low/missing-feedback] Panel hooks dropped silently by the modal rule and the dev-mode gate — CONFIRMED

- Status: Done (build and tests). Inert and conflicting hook combinations are named at parse time, and a refused shortcut is logged (GlobalRefused).
- Needs: Fix (test hooks). Reject inert or conflicting hook combinations at parse time with an error. Files: crates/desktop/src/command_line.rs, crates/desktop/qml/TestHooks.qml.
- Verdict: CONFIRMED
- Note: controller.rs:427 returns no commands for any open modal that is not switchable (only inbox and log are), so --inbox after --library does nothing. controller.rs:432-433 ignores Global::Log without dev mode. Nothing reports either refusal.
- Confidence: high
- Trigger: --library=1 or --menu=views or --save-view=1 or --find-similar=`<photo>` combined with --inbox=1 or --log=1 (those run after the modal hooks), or --log=1 without dev mode.
- Consequence: The inbox or log view does not open and nothing says why, so a scripted run continues with the wrong panel showing.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:111`; `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:117`; `D:/GitHub/photo-gallery/crates/client/src/controller.rs:427`; `D:/GitHub/photo-gallery/crates/client/src/controller.rs:433`

### UI-019 [low/stale-state] --inbox-expand without --inbox persists into the user's first inbox open — CONFIRMED

- Status: Done (validated). command_line.rs rejects --inbox-expand without --inbox, and the pass is spent on the accepted open.
- Needs: Fix (test hooks). Make --inbox-expand a one-shot pass consumed by the first open it applies to, not a value reset on close. Files: crates/desktop/qml/TestHooks.qml, crates/desktop/qml/Main.qml.
- Verdict: CONFIRMED
- Note: TestHooks.qml:116 sets the expand pass. The reset is in the inbox open-change handler (Main.qml:64-69), and it runs only on a change to closed. The first user open is an open change, so the reset is skipped and expandRow (Main.qml:833) uses the stale pass.
- Confidence: high
- Trigger: --inbox-expand=`<ledger>` given without --inbox=1; the inbox is opened later by the user (Ctrl+Shift+N).
- Consequence: The hook's expand pass is still set when the user first opens the inbox, so that ledger's entry opens expanded. The reset runs only when the inbox closes.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:116`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:494`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:68`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:833`

### UI-020 [low/missing-feedback] --viewer-keys and --zoom without --view are silently ignored — CONFIRMED

- Status: Done (validated).
- Needs: Fix (test hooks). Validate at parse time that --viewer-keys and --zoom require --view. Files: crates/desktop/src/command_line.rs.
- Verdict: CONFIRMED
- Note: The viewer keys and --zoom sit inside `if (commandLine.view >= 0)` (TestHooks.qml:92-109), so they do nothing without --view. command_line.rs:278-279 reports --yandex-preview without --find-similar as a bad option, but nothing reports these.
- Confidence: high
- Trigger: --viewer-keys=LB or --zoom=2 given without --view=N.
- Consequence: The scripted run proceeds with no viewer and no message. command_line.rs names --yandex-preview given without --find-similar as a bad option, so this case is inconsistent with the stated rule against silent hook no-ops.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:92`; `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:107`; `D:/GitHub/photo-gallery/crates/desktop/src/command_line.rs:278`; `D:/GitHub/photo-gallery/crates/desktop/src/command_line.rs:279`

## U05: Screenshot settle hook

Entry: `crates/desktop/src/lib.rs:214 screenshot::schedule(engine)`

### UI-023 [low/stale-result] Settle check can pass before the library job's first progress is seen — PARTIAL

- Status: Done (validated). Library.scanPending is declared and set on add-folder and rescan, and screenshot.cpp reads it.
- Needs: Fix (checked: partly; the race needs an add-folder request in the same run, and the 300 ms quiet gate does not protect the first ready tick). The settle waits for a pending scan request or the first progress. Test harness only. Files: crates/desktop/cpp/screenshot.cpp, crates/desktop/src/library.rs, crates/desktop/src/shell.rs (331-332), crates/desktop/qml/TestHooks.qml (154-155).
- Verdict: PARTIAL
- Note: The race needs a job that starts after the start. A plain start (Main.qml:879) requests no scan, so progress stays Idle and the settle check is right to pass. The window only sees a non-idle job when a scan is requested in the same run, for example by the add-folder hook (TestHooks.qml:122, which runs after library.start). The trigger as written (library folders alone) does not reach it.
- Confidence: medium
- Trigger: A --screenshot run with library folders where the window becomes ready and quiet for 300 ms before the lane coordinator thread publishes its first non-idle progress. Main.qml startHooks calls library.start() synchronously; Library::start sets busy from progress() right away, and progress() is Idle until a lane control begins on the coordinator's own thread.
- Consequence: busy.jobs stays 0 and idle() returns true, so the image is taken before the job has started, and the status-bar lane rows and the busy state are missing from the screenshot. The settle log reports 'library job running: 0' even though a job is about to start. This is test-harness only, and the window is short, so it is timing-dependent.
- Sites: `crates/desktop/cpp/screenshot.cpp:66`; `crates/desktop/src/library.rs:223`; `crates/desktop/src/library.rs:175`; `crates/library/src/runner.rs:173`; `crates/desktop/qml/Main.qml:879`

## U06: Crash box on panic

Entry: `crates/desktop/src/panic_log.rs:17 install (panic hook) and :32 alert::error_box(shown)`

### UI-029 [low/unhandled-close] Closing the window waits for an open panic box on the writes thread — CONFIRMED

- Status: Done (validated). The panic box still blocks the writes thread, but exit waits at most 2 s.
- Needs: Fix. worker::stop must not wait on a writes thread that a panic box is blocking; the box must not hold up the exit it is reporting on. Files: crates/desktop/src/worker.rs, crates/desktop/src/panic_log.rs, crates/desktop/src/lib.rs.
- Verdict: CONFIRMED
- Note: worker::stop joins the writes thread (worker.rs:54-63), and the box from a writes-thread panic blocks that thread (panic_log.rs:25). The window's exit therefore waits until the box is dismissed (lib.rs:221).
- Confidence: medium
- Trigger: The writes thread panics and its box is still open when the user closes the window. The GUI thread leaves app.exec and calls worker::stop, which joins that thread.
- Consequence: The window disappears but the process stays alive until the box is dismissed, so the app appears hung on close. If the box is hidden behind other windows, the close can look stuck.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:216`; `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:221`; `D:/GitHub/photo-gallery/crates/desktop/src/worker.rs:57`

## U07: Live reload of QML (development)

Entry: `crates/desktop/src/lib.rs:195 read_qml_from_source and :210 reload_on_change`

### UI-030 [high/missing-start] Reloaded window never starts the library: no folders, no change events — CONFIRMED

- Status: Done (build and tests). The reloaded window's QML starts its own library on libraryStartDue. Not exercised at runtime.
- Needs: Fix (dev build only). The reloaded window must start its own library; the early return in Library::start (library.rs:148) must key on the new object. Files: crates/desktop/src/library.rs, crates/desktop/src/pump.rs, crates/desktop/qml/Main.qml.
- Verdict: CONFIRMED
- Note: Library::start returns early when running() is Some (desktop library.rs:148-151), before load_roots (:152). The reloaded window's new Library therefore has empty roots, and the pump is attached to the deleted object. Live reload is debug-only.
- Confidence: high
- Trigger: Any save of a QML file in crates/desktop/qml while a debug build runs (after the start is over). The reload deletes the Library QObject and makes a new one.
- Consequence: The reloaded window's Library has empty roots, so the grid's empty state reads 'no folders' (GalleryGrid.qml:97) and the Library panel lists no folders. Scans, imports, progress and file changes never reach the window (the change pump is tied to the deleted object), until the app restarts. The window shows a wrong library state as correct.
- Sites: `crates/desktop/qml/Main.qml:105`; `crates/desktop/src/shell.rs:464`; `crates/client/src/controller/start.rs:80`; `crates/desktop/qml/Main.qml:879`; `crates/desktop/src/library.rs:148`; `crates/desktop/src/pump.rs:60`; `crates/desktop/qml/Main.qml:628`

### UI-032 [medium/stale-open-panel] Reload with a panel open leaves its mirror set, blocking shortcuts and the inbox — CONFIRMED

- Status: Done (validated). live_reload.cpp invokes shell.windowReloaded, and the client clears the modal mirror.
- Needs: Fix (dev build only). The controller owns the modal mirror: reconcile it with the reloaded window, not by patching teardown. Files: crates/client/src/controller/panels.rs, crates/desktop/cpp/live_reload.cpp, crates/desktop/qml/Main.qml.
- Verdict: CONFIRMED
- Note: The modal mirror is only changed by PanelOpened and PanelClosed (panels.rs:127-135), and teardown emits neither. The global() rule (controller.rs:427) then blocks inbox, log and dev-mode keys while a non-switchable modal stays in the mirror. Debug-only.
- Confidence: medium
- Trigger: The inbox, the library panel or a modal dialog is open when a QML file is saved. The reloaded window does not reopen it, and Qt does not emit visibleChanged during teardown, so no panelClosed is sent.
- Consequence: The controller still lists the panel as open. With the inbox open, the Inbox key and the status-bar bell send a close for a panel that is not shown, so the inbox cannot be reopened until restart. With a non-switchable modal open (library, dialog, folder picker, views menu), every global key (inbox, log, dev-mode toggle) is ignored until the user opens and closes that panel.
- Sites: `crates/client/src/controller.rs:427`; `crates/client/src/controller.rs:453`; `crates/desktop/qml/ModalOverlay.qml:18`; `crates/desktop/qml/Main.qml:565`; `crates/desktop/qml/Main.qml:846`

### UI-033 [low/no-window-state] A failed QML load on reload leaves no window and no way to quit from the UI — CONFIRMED

- Status: Done (build and tests). The fallback window and its ui-dev-reload-failed and ui-quit strings are in place.
- Needs: Fix (dev build only). A failed QML load shows an actionable error and keeps a quit path. Files: crates/desktop/cpp/live_reload.cpp.
- Verdict: CONFIRMED
- Note: live_reload.cpp:56 sets quitOnLastWindowClosed to false. After the window is deleted at reload (:95), a failed engine.load (:100) leaves no window. quit_on_close only quits on a visibleChanged(false) of a window that exists (:40). Debug-only.
- Confidence: medium
- Trigger: A QML file is saved with a syntax or binding error during a debug run, so engine.load produces no root window.
- Consequence: The window vanishes and the process keeps running with no window, because quitOnLastWindowClosed is false. The only feedback is a qWarning on the console. The user has no on-screen message and cannot close the app from the UI.
- Sites: `crates/desktop/cpp/live_reload.cpp:56`; `crates/desktop/cpp/live_reload.cpp:57`; `crates/desktop/cpp/live_reload.cpp:100`; `crates/desktop/cpp/live_reload.cpp:40`

## U08: Grid keyboard keys (through the shell)

Entry: `crates/desktop/qml/GalleryGrid.qml:73 Keys.onPressed -> GalleryGrid.qml:32 keyPressed`

### UI-037 [low/stale-result] A key pressed right after an arrow acts on the previous photo — CONFIRMED

- Status: Done (build and tests). Act goes to OnCurrent, which the desktop resolves on the model lane.
- Needs: Fix. Step goes through the controller's current, so the next key sees the moved item. QML sends intents only. Files: crates/desktop/src/shell.rs, crates/client/src/controller/input.rs, crates/client/src/controller.rs.
- Verdict: CONFIRMED
- Note: The arrow's Step is applied on the gallery lane (shell.rs:476-484), but the controller's current (input.current, controller.rs:384) changes only when QML reports CurrentChanged (Main.qml:141). A key handled in the same pass therefore acts on the old item. Rate, turn and mark write to the file.
- Confidence: medium
- Trigger: Two keys arrive in the same event-loop pass, for example Right then 5, or Right then Enter, or Home then Enter. The arrow's move is queued on the grid model's lane, and the controller's current item (its mirror, updated only when QML reports CurrentChanged) has not caught up when the second key is handled.
- Consequence: The rating, turn, border mark or viewer goes to the old photo while the selection visibly moved to the new one. The user sees the wrong photo changed, and the change is written to the file.
- Sites: `crates/desktop/src/shell.rs:476`; `crates/client/src/controller/input.rs:84`; `crates/client/src/controller.rs:384`

## U09: Global shortcuts (inbox, log, dev mode)

Entry: `crates/desktop/qml/GlobalShortcuts.qml:24-40 Shortcut blocks with onActivated at :28, :34, :40`

### UI-038 [low/stale-result] Lane 'N failed' sets the inbox expand pass before the controller may refuse the open — CONFIRMED

- Status: Done (validated). globalActionExpanding and panelOpenRequested(kind, expandPass) are declared, and the pass goes with the accepted open.
- Needs: Fix. Pass the expand request with the open, and apply it only when the controller accepts the open. Files: crates/desktop/qml/Main.qml, crates/client/src/controller.rs.
- Verdict: CONFIRMED
- Note: Main.qml:565 sets inboxExpandPass before globalAction, and the controller can refuse it (controller.rs:427). The reset runs only in the closed branch (Main.qml:68), so the stale pass expands the first later open (InboxPanel.qml:134-142). Severity low: it needs a non-switchable modal open at the click.
- Confidence: low
- Trigger: A non-switchable modal (for example the Views menu) is still in the controller's open-panel mirror when the status bar's 'N failed' button is clicked. Main.qml sets inboxExpandPass to that lane's pass, then globalAction(Inbox) is ignored by Controller::global (controller.rs:426-429), so the inbox does not open. The user then opens the inbox later with Ctrl+Shift+N.
- Consequence: The stale pass is not cleared, since it resets only on the inbox closing (Main.qml:68). The next manual open of the inbox expands that old ledger automatically (InboxPanel expand(), InboxPanel.qml:133-143), with its details and file list shown, which the user did not ask for. The failed lane click gave no feedback at the time.
- Sites: `crates/desktop/qml/Main.qml:565`; `crates/desktop/qml/Main.qml:566`; `crates/desktop/qml/Main.qml:567`; `crates/desktop/qml/Main.qml:68`; `crates/desktop/qml/InboxPanel.qml:138`

## U10: Select a photo by click

Entry: `crates/desktop/qml/GridCell.qml:182 MouseArea onClicked -> cellClicked (GridCell.qml:34) -> GalleryGrid cellClicked -> Main.qml:632-634 (win.current = i)`

### UI-040 [low/missing-feedback] Status line goes blank for a selected file whose details are unavailable — CONFIRMED

- Status: Done (build and tests). The tri-state detail is in place, and ui-details-unavailable is defined.
- Needs: Fix. A tri-state detail (loading, unavailable, loaded) instead of Option flattening. Also correct the doc's claim that the state persists. Files: crates/desktop/src/gallery/state.rs, crates/desktop/src/gallery/items.rs, crates/desktop/src/gallery/model.rs.
- Verdict: CONFIRMED
- Note: Details are read with .flatten() (state.rs:156) and an info miss gives an empty string (items.rs:195). The cache keeps (file, None) (state.rs:135). The finding says the state persists until the list reloads, but the two-entry cache also evicts when other reads come in, so that part is overstated.
- Confidence: medium
- Trigger: (a) The user clicks a file and its details read is still in flight: the status line shows only the rating, or nothing, and the key hints are replaced until the read returns. (b) The file has left the catalog while it is still in the open list: the read returns Ok(None), which is cached as (file, None). That state persists until the list is reloaded, with no notice.
- Consequence: The status bar shows an empty line for a selected photo, and in case (b) it stays empty with no message while the grid still draws the cell as selected.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/gallery/state.rs:156`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/items.rs:195`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/model.rs:283`

## U14: Grid scrollbar drag

Entry: `crates/desktop/qml/GalleryGrid.qml:123 onPositionChanged (while pressed)`

### UI-050 [low/wrong-position] Readout is placed from the logical position, not the drawn handle — PARTIAL

- Status: Done (validated).
- Needs: Fix (checked: confirmed). Centre the readout on (visualPosition + visualSize / 2) times the track height, and keep the number on the logical position. Files: crates/desktop/qml/GalleryGrid.qml (readout at 165), Qt quicktemplates/qquickscrollbar.cpp (visualArea, 134-147).
- Verdict: PARTIAL
- Note: The readout uses gridBar.position (GalleryGrid.qml:138). The handle is placed from visual.position (qquickscrollbar.cpp:229). I did not check whether position and visual.position differ once minimumSize applies, which is what the 24 px figure depends on.
- Confidence: high
- Trigger: A library large enough that the bar's minimumSize (48 px) exceeds its size ratio (roughly contentHeight above height squared over 48, i.e. thousands of files), and any drag.
- Consequence: The readout's centre sits up to about 24 px above the handle's centre at the top of the track and about 24 px below it at the bottom. The number is correct, but the bubble does not sit on the handle it describes.
- Sites: `crates/desktop/qml/GalleryGrid.qml:138`; `C:/Qt/6.12.1/Src/qtdeclarative/src/quicktemplates/qquickscrollbar.cpp:229`

## U16: Include subfolders toggle

Entry: `crates/desktop/qml/TopBar.qml:97 onToggled -> recursiveToggled -> Main.qml:519-522`

### UI-055 [medium/wrong-message] Empty folder without subfolders says All folders over an empty grid — CONFIRMED

- Status: Done (validated). An empty non-recursive folder reads "Drive roots".
- Needs: Fix. Bind the top-bar label to the listing actually shown, so an empty non-recursive listing is not labelled All folders with 0 files. Files: crates/client/src/tree.rs, crates/desktop/src/folders.rs, crates/desktop/qml/TopBar.qml.
- Verdict: CONFIRMED
- Note: shown_as (tree.rs:113-117) gives All when root_files is 0, and listing_kind_of maps that to KIND_ALL (folders.rs:95-100). TopBar.qml:75 then labels All folders while the box is unchecked and the non-recursive listing is empty.
- Confidence: high
- Trigger: The library has no files directly in any drive root, so every photo is in a subfolder. The user is on All folders with Include subfolders checked, then unchecks it. The non-recursive empty-folder listing is empty.
- Consequence: The grid goes empty and the top bar still reads All folders with 0 files. The tree highlights its All row while the checkbox is unchecked, so the label contradicts both the unchecked box and the empty grid. The code's comments (tree.rs:109-112, folders.rs:87-89) say the All label is deliberate to match the highlighted row, so this is a misleading state rather than a crash.
- Sites: `D:/GitHub/photo-gallery/crates/client/src/tree.rs:117`; `D:/GitHub/photo-gallery/crates/desktop/src/folders.rs:99`; `D:/GitHub/photo-gallery/crates/desktop/qml/TopBar.qml:75`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:520`

## U18: Apply a saved view

Entry: `crates/desktop/qml/ViewsMenu.qml:41 onTriggered -> viewChosen (ViewsMenu.qml:22) -> TopBar viewChosen -> Main.qml:538 win.applyView`

### UI-062 [low/missing-feedback] A failed read of the chosen view is silently a no-op — CONFIRMED

- Status: Done (validated).
- Needs: Fix. A failed view read reports through the shared outcome path, as save and delete already do. Files: crates/desktop/src/shell.rs, crates/desktop/qml/Main.qml.
- Verdict: CONFIRMED
- Note: shell.rs:379-381 returns false on a failed view read with no notice. Main.qml:324-325 then returns at once. views_outcome (shell.rs:115-117) is used for save and delete only.
- Confidence: high
- Trigger: The catalog read behind views().view(name) fails (a database error) when a view is chosen from the menu.
- Consequence: shell.apply_view returns false with no notice, and Main.applyView returns at once. The user clicks a view and nothing happens, with no message. Saving and deleting views report failures through views_outcome, so this path is the odd one out.
- Sites: `crates/desktop/src/shell.rs:379`; `crates/desktop/qml/Main.qml:324`; `crates/desktop/src/shell.rs:115`

### UI-065 [low/stale-result] A view applied before the start is over can be replaced by all folders — CONFIRMED

- Status: Done (build and tests). The client gate never opens All folders from absence, and the desktop dispatches TreeFailed.
- Needs: Fix. Same start-gate fix as UI-002: a TreeFailed state, and no OpenAll from absence. Files: crates/client/src/controller/start.rs.
- Verdict: CONFIRMED
- Note: start.rs:59-64 has no started check, so TreeLoaded{false} emits OpenAll whenever it arrives. Main.qml:292 sends it for a view whose folder is missing from the tree (folders.rs:246 gives -2). This needs a view applied before the tree arrives, a narrow window.
- Confidence: medium
- Trigger: A view whose folder is not in the tree is applied before the start gate has its tree (before TreeLoaded). Main.qml:292 then sends shell.treeLoaded(false), and the gate emits OpenAll with no started check.
- Consequence: The user's applied view is replaced by the All folders listing without a notice, and the filter bar keeps the view's query. The window is only usable while the start runs for a brief time, so the window is narrow.
- Sites: `crates/client/src/controller/start.rs:59`; `crates/client/src/controller/start.rs:63`; `crates/client/src/controller.rs:357`; `crates/desktop/qml/Main.qml:292`

## U20: Delete a saved view

Entry: `crates/desktop/qml/ViewsMenu.qml:60 onTriggered -> viewDeleteRequested -> TopBar.qml:53 -> Main.qml:539 shell.deleteView`

### UI-069 [low/missing-confirmation] Saved view deleted immediately with no confirmation or undo — CONFIRMED

- Status: Done (build and tests). The delete dialog, its build entry and its strings are in place.
- Needs: Fix. Confirm before deleting, using the same dialog pattern as RemoveFolderDialog. Files: crates/desktop/qml/ViewsMenu.qml, crates/desktop/qml/Main.qml.
- Verdict: CONFIRMED
- Note: ViewsMenu.qml:60 calls viewDeleteRequested on a single trigger, and Main.qml:539 forwards it to shell.deleteView (shell.rs:326-329) with no confirmation. The other destructive dialogs, such as RemoveFolderDialog, do confirm.
- Confidence: high
- Trigger: A single click on a name in Views > Delete view. Only one submenu level away from Save current view.
- Consequence: The view's stored folder, query, sort and name are permanently removed by one misclick. Other destructive actions in this window (RemoveFolderDialog, DiscardRemovalDialog, CutBordersDialog) confirm first, so this path is inconsistent.
- Sites: `crates/desktop/qml/ViewsMenu.qml:60`; `crates/desktop/qml/Main.qml:539`

## U21: Filter bar open and close, facet counts

Entry: `crates/desktop/qml/TopBar.qml:103 onClicked -> filtersToggled -> Main.qml:518 shell.toggleFilters (shell_bridge.rs:130)`

### UI-073 [low/missing-feedback] Chip counts show zeros or the previous folder's counts until the count returns — CONFIRMED

- Status: Done (build and tests). The pending state reaches the chips, and ui-count-pending is defined.
- Needs: Fix. A pending state until counts arrive for the current folder and query, not 0 or the previous folder's counts. Files: crates/desktop/src/facet_choices.rs, crates/desktop/qml/FilterBar.qml.
- Verdict: CONFIRMED
- Note: facet_choices.rs:41 maps a count that has not arrived to 0 (shown.unwrap_or(0)), so the chips show 0 until the first count. After a folder change the previous rows stay until the new count is delivered.
- Confidence: high
- Trigger: The bar is opened for the first time (no count has run yet), or the folder or query changes while the bar is open, and the new count is still being read off the GUI thread.
- Consequence: The chips show 0 and the extension row is empty on first open. After a folder change they show the previous folder's counts, with no pending indicator. The numbers look current until the count arrives. A large catalog can take noticeable time.
- Sites: `crates/desktop/src/controller.rs:28`; `crates/desktop/src/facet_choices.rs:41`; `crates/desktop/qml/FilterBar.qml:143`

## U22: Filter facet toggles

Entry: `crates/desktop/qml/FilterBar.qml:157 (types), :170 (ratings), :182 (extensions), :194 (orientations), :237 (status) onClicked -> filters.toggle; :249 onClicked (borders, bordersBlocked when borders are off)`

### UI-076 [low/unneeded-relist] An unparsed bound keystroke re-lists and clears the selection — CONFIRMED

- Status: Already fixed (validated). filters.rs:78-88 and 149-157 compare before set_query. Candidate for removal, pending your call.
- Needs: Fix. Compare the parsed bound before arming the relist, so set_query runs only on change. Correct the citation: filters.rs:270 is out of range; the mechanism is at filters.rs:133-136. Files: crates/desktop/src/filters.rs, crates/desktop/qml/FilterBar.qml.
- Verdict: CONFIRMED
- Note: edit() (filters.rs:78-82) never compares, and set_query calls edited() unconditionally (filters.rs:84-88). That arms the 300 ms pause (controller.rs:301-303), which lists from the top with keep=false. The citation filters.rs:270 is out of range (the file has 191 lines). The mechanism is at filters.rs:133-136.
- Confidence: high
- Trigger: With a min or max field open (no value), the user types text that does not parse ('.', '-', 'lots', or a partial date). The query does not change, but set_bound still calls set_query, which re-arms the 300 ms pause, which lists the folder.
- Consequence: After the pause the grid reloads from the top: the selection is cleared (selection_after with keep false) and the view scrolls to the beginning, although the filter did not change. The user loses their place while typing.
- Sites: `crates/desktop/qml/FilterBar.qml:101`; `crates/desktop/src/filters.rs:136`; `crates/desktop/src/filters.rs:97`; `crates/desktop/src/filters.rs:270`

### UI-077 [low/focus] Keyboard toggle of a chip loses focus and focus is not returned to the grid — CONFIRMED

- Status: Done (validated). A chip toggle returns focus to the grid.
- Needs: Fix. Focus returns to the grid after a chip toggle, under the shared focus policy. Files: crates/desktop/src/facet_choices.rs, crates/desktop/qml/Main.qml.
- Verdict: CONFIRMED
- Note: facet_choices.rs:62-64 resets the whole model when the rows change, which destroys the focused chip delegate. Main.qml:317-320 (showListing) takes no focus action, and nothing in the chip path calls grid.takeFocus.
- Confidence: medium
- Trigger: Tab to a filter chip and press Space or Enter. The toggle makes the facet's rows change, the Repeater resets, and the focused chip is destroyed. showListing does not move focus anywhere.
- Consequence: The keyboard focus falls to the window root, so the grid's arrow and other keys have no receiver until the user clicks the grid. Other controls (sort, descending, applyView) call grid.takeFocus(); this path does not.
- Sites: `crates/desktop/src/facet_choices.rs:58`; `crates/desktop/src/facet_choices.rs:62`; `crates/desktop/qml/Main.qml:317`; `crates/desktop/qml/Main.qml:532`

### UI-078 [low/stale-result] Total reads N of 0 until the first count arrives after start or a view applies — CONFIRMED

- Status: Done (build and tests). countsPending drives the TopBar label, and ui-count-pending is defined.
- Needs: Fix. Show a pending state until the first count, or request the count at start. Files: crates/desktop/src/filters.rs, crates/desktop/qml/TopBar.qml.
- Verdict: CONFIRMED
- Note: total and matching start at 0 (filters.rs:38-39) and change only when a count is applied. updateFacets counts only when the bar is open or a filter is active (Main.qml:146-149), so the 'N of 0' label shows until that count arrives. The interval is brief.
- Confidence: high
- Trigger: The app starts with a filter in the session query (or a saved view with a filter is applied) while the bar is closed. Filters.total and matching start at 0 and are set only when a count is delivered, which is requested only on listingChanged.
- Consequence: The top bar shows 'N of 0' (or the previous folder's total after applyView) for the time between the list showing and the count arriving, which can be visible on a large catalog.
- Sites: `crates/desktop/src/filters.rs:38`; `crates/desktop/qml/TopBar.qml:84`; `crates/desktop/qml/Main.qml:146`

## U27: Add a folder through the picker

Entry: `crates/desktop/qml/LibraryPanel.qml:153 onClicked addRequested -> Main.qml:699 addPicker.pick(0)`

### UI-096 [medium/unhandled-close] Added folder's scan request is lost when the app quits before it runs — CONFIRMED

- Status: Done (validated). The scan_requests table is in baseline.sql, as you confirmed.
- Needs: Fix. Persist pending scan requests in the catalog, so a scan requested before quit runs on the next start. Files: crates/app/src/library.rs, crates/library/src/coordinator.rs.
- Verdict: CONFIRMED
- Note: add_folder queues the scan only when the job runs (app library.rs:377), and the queue is in memory (coordinator.rs:277-282). I found no scan requested at start (the grep of library.start shows none), so existing roots are not re-queued on the next launch.
- Confidence: high
- Trigger: The user adds a folder (a large one, so its walk takes a while) and closes the window or quits before the scan finishes. Or the job is stopped at exit with the scan still queued.
- Consequence: The folder is recorded in the catalog and listed in the library panel and tree, but no file of it is scanned. The scan request lives only in the in-memory queue (Coordinator reg.scans), and nothing on start re-queues existing roots. The folder stays empty after the next launch until the user presses Rescan all. The user sees a folder with no photos and no error.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\library.rs:377`; `D:\GitHub\photo-gallery\crates\library\src\coordinator.rs:280`; `D:\GitHub\photo-gallery\crates\library\src\runner.rs:95`; `D:\GitHub\photo-gallery\crates\library\src\passes.rs:63`

## U28: Remove a folder (with confirmation)

Entry: `crates/desktop/qml/LibraryPanel.qml:71 onClicked removeRequested -> Main.qml:700 askRemoveFolder (Main.qml:214-218)`

### UI-104 [low/stale-result] Grid keeps listing the removed folder's images until the removal job ends — PARTIAL

- Status: Partial (build and tests). Begun::Started now only loads roots; the grid and tree refresh on Scanned, published after hide_pages. No separate tree refresh at Started.
- Needs: Fix (checked: confirmed; the stale window ends at hide_pages, not at the final publish). Hide the folder's rows when the removal starts, or skip the reload at Begun::Started. Files: crates/desktop/src/library.rs (290-292), crates/desktop/qml/Main.qml (155), crates/library/src/coordinator.rs (959, 1012).
- Verdict: PARTIAL
- Note: The removal's final publish is Scanned (coordinator.rs:847), so the grid refreshes only then. Whether the Begun::Started reload still lists the removed folder's files depends on the listing query, which I did not trace.
- Confidence: medium
- Trigger: The user confirms the removal of a folder with many files. The removal runs as a background job (hide, delete in batches) that can take a long time.
- Consequence: The reload at Begun::Started re-lists the grid while the removed folder's files are still visible. Hiding and deletion then happen with no grid refresh until the job's final Scanned. The user sees images the dialog said were being deleted, and the tree no longer shows their folder.
- Sites: `crates/desktop/src/library.rs:276`; `crates/desktop/src/library.rs:277`; `crates/desktop/qml/Main.qml:129`; `crates/library/src/coordinator.rs:847`

## U29: Discard a pending removal

Entry: `crates/desktop/qml/LibraryPanel.qml:103 onClicked discardRequested -> Main.qml:701 askDiscardRemoval (Main.qml:229)`

### UI-106 [low/wrong-message] Discard failures are worded as 'folder couldn't be removed from the library' — CONFIRMED

- Status: Done (build and tests). Notice::DiscardFailed is declared, with its wording.
- Needs: Fix. A discard-failure message separate from the removal-failure message, as a new string key. Files: crates/desktop/src/library.rs, crates/client/locales/en/text.ftl.
- Verdict: CONFIRMED
- Note: The desktop posts FolderNotRemoved for every discard failure (desktop library.rs:134, :144), so the title is "couldn't be removed from the library" (text.ftl:280), while the folder had already left the library.
- Confidence: high
- Trigger: A discard fails in any way: the pending-removals read fails, the removal is refused because a library folder overlaps it, or the discard itself errors.
- Consequence: The user is told the folder could not be removed from the library. The folder already left the library when the removal began, and the failure is about discarding the changes. The message misdescribes the situation.
- Sites: `D:\GitHub\photo-gallery\crates\desktop\src\library.rs:134`; `D:\GitHub\photo-gallery\crates\desktop\src\library.rs:144`; `D:\GitHub\photo-gallery\crates\client\locales\en\text.ftl:280`

### UI-107 [medium/wrong-message] Discard shows success while a temp copy is left on disk — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Record the leftover temp as a pending cleanup in the catalog and retry it. The notice is secondary. Files: crates/app/src/library.rs, crates/catalog/src/removal.rs.
- Verdict: CONFIRMED
- Note: discard_removal logs a failed temp unlink as a warning and carries on (app library.rs:420-422), then returns success. The dialog promises the temp copies are deleted (ui.ftl:74). The user gets no notice.
- Confidence: medium
- Trigger: A .pg-write temp of a kept write cannot be unlinked during Discard (a sharing violation from another program, such as antivirus or Explorer, or any other non-NotFound error). The rows are deleted and the removal is committed.
- Consequence: The dialog promised 'their temporary copies are deleted'. The row disappears and no error is shown. The temp, which holds the discarded change, stays in the user's photo folder, and the only record is a log line the user never sees.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\library.rs:420`; `D:\GitHub\photo-gallery\crates\app\src\library.rs:421`; `D:\GitHub\photo-gallery\crates\client\locales\en\ui.ftl:74`; `D:\GitHub\photo-gallery\crates\catalog\src\removal.rs:65`

### UI-109 [low/wrong-target] Discard finds its removal by lossy display text, so two removals can match — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Address the pending removal by its row id, not display text. Files: crates/desktop/src/library.rs, crates/model/src/rules.rs.
- Verdict: CONFIRMED
- Note: library.rs:139 finds the pending removal by comparing rules.display(p) with the folder text, and display is lossy (rules.rs:207-211, documented as lossy). Two removals with the same display text match the first. Latent, as the finding says.
- Confidence: medium
- Trigger: Two pending removals whose paths display identically, because display() is lossy for non-UTF-8 or unpaired-surrogate names. The user clicks Discard on the second row.
- Consequence: find() takes the first match, so the first removal is discarded. The changes of the folder the user did not choose are dropped, and the second stays listed. This is latent: it needs two such paths.
- Sites: `D:\GitHub\photo-gallery\crates\desktop\src\library.rs:139`; `D:\GitHub\photo-gallery\crates\model\src\rules.rs:207`

## U30: Rescan all folders

Entry: `crates/desktop/qml/LibraryPanel.qml:157 onClicked rescanRequested -> Main.qml:702 library.rescan() (library.rs:288)`

### UI-110 [low/double-fire] Second Rescan during a walk re-walks that folder — CONFIRMED

- Status: Done (validated).
- Needs: Fix. The coordinator queue holds each root at most once, keyed by root. Files: crates/library/src/coordinator.rs, crates/library/src/passes.rs.
- Verdict: CONFIRMED
- Note: A preempted walk is pushed back to the front of its lane queue (passes.rs:100). A second rescan adds the same path to the lane's queue when it is not already there (coordinator.rs:509-511), and the lane's own queue does not hold the path while it is being walked. The folder is therefore queued twice and walked twice.
- Confidence: high
- Trigger: The user clicks Rescan all again while a scan is walking a library folder. Nothing in the panel shows the job is busy, so a repeat click is likely. The second request is queued, the lane's walk is preempted, and once the walk is resumed the same folder sits in the queue twice.
- Consequence: The folder is walked fully twice in a row, with no change in result. On a large library this costs hours of disk reads, and the lane restarts its progress. The user sees the scan restart from the beginning.
- Sites: `crates/library/src/coordinator.rs:275`; `crates/library/src/coordinator.rs:509`; `crates/library/src/coordinator.rs:513`; `crates/library/src/passes.rs:63`; `crates/library/src/passes.rs:100`; `crates/desktop/qml/LibraryPanel.qml:154`

### UI-112 [low/missing-feedback] Rescan silently switches the listing to All when the listed folder has no present files left — CONFIRMED

- Status: Done (build and tests). The gate never changes the listing, and TreeLoaded takes no argument.
- Needs: Fix. Absence never switches the listing to All; the folder stays listed in its current state. Same start-gate fix as UI-002. Files: crates/client/src/controller/start.rs, crates/desktop/src/folders.rs.
- Verdict: CONFIRMED
- Note: The tree counts only present files (tree.rs:11, 15: missing = 0). After a rescan marks the listed folder's files missing, the folder is not in the tree, so folders.rs:246 gives -2 and Main.qml:292 sends treeLoaded(false). The gate then emits OpenAll with no started check (start.rs:59-63).
- Confidence: medium
- Trigger: The user lists folder F, then F's files are deleted or moved on disk, and Rescan all marks them missing. After the scan the tree no longer has F, because the tree counts only present files. Main.treeLoaded then sends shell.treeLoaded(false).
- Consequence: The start gate sends OpenAll before its started check, so the grid silently changes to All folders with no message. The user loses the folder view they had. The Main.qml comment documents the rule, but nothing tells the user. A failed tree read after a scan has the same effect, with the tree replaced by an empty one.
- Sites: `crates/client/src/controller/start.rs:59`; `crates/desktop/src/folders.rs:246`; `crates/desktop/qml/Main.qml:292`; `crates/catalog/src/browse/read/tree.rs:15`; `crates/desktop/src/folders.rs:181`

## U32: Scan lanes: pause a lane, open its failed count

Entry: `crates/desktop/qml/LaneRow.qml:117 onClicked -> pauseRequested (LaneRow.qml:39) and LaneRow.qml:82 onTapped -> failedRequested (LaneRow.qml:41)`

### UI-116 [low/wrong-target] Failed link can open the inbox with no entry expanded — PARTIAL

- Status: Done (validated).
- Needs: Fix (checked: confirmed for an unreadable-only pass). Disable the link when the pass has no ledger row, or give unreadable files a ledger row. Files: crates/desktop/src/notices.rs (174-180, 637-645), crates/desktop/qml/LaneRow.qml, crates/desktop/qml/Main.qml (913).
- Verdict: PARTIAL
- Note: The link is enabled by the pass's bit in notices.rs:173 and LaneRow.qml:37, and the lookup at Main.qml:833-834 then finds no row. Whether unreadable files really leave no ledger row was not traced in the runner.
- Confidence: high
- Trigger: A pass whose only failures are unreadable files, which the runner says but does not record in any ledger. The UnreadableFiles row sets the pass's bit in Notices.passes, so LaneRow's link is enabled. The pass has no ledger entry, so the lookup behind expandRow finds no row. The user clicks the lane's N failed.
- Consequence: The inbox opens with nothing expanded and no message. The link promises the pass's failed-file ledger, and that ledger is not listed. The user has to find the unreadable-files row themselves.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:173`; `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:525`; `D:/GitHub/photo-gallery/crates/desktop/qml/LaneRow.qml:37`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:833`; `D:/GitHub/photo-gallery/crates/desktop/qml/InboxPanel.qml:134`

## U33: Lane progress polling

Entry: `crates/desktop/qml/Main.qml:412-417 Timer (250 ms, running while laneRows.settling) and Main.qml:132 onProgressed`

### UI-119 [low/stale-result] A held-back count change is shown only at the next file or phase end — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Emit a trailing progress update when the throttle holds one back, so the row never trails the last count. Files: crates/jobs/src/control.rs, crates/desktop/qml/Main.qml.
- Verdict: CONFIRMED
- Note: control.rs:416-424 throttles the progress notice by NOTICE_INTERVAL, and the desktop timer that refreshes the lane rows runs only while laneRows.settling is set (Main.qml:412-417). The comment at control.rs:413-416 that calls it intentional was not read.
- Confidence: medium
- Trigger: A lane's files complete in a burst within 100 ms of the last notice, then one file takes a long time (for example a large RAW decode or a slow import). The bar and count are read on each progress pass and on the settling timer, and the timer runs only while a row is held.
- Consequence: The row keeps showing the count from before the last file in the burst (for example 39 of 40 while 40 are done) until the slow file finishes or the phase ends. The row's bar and count can trail by up to one file's duration. The library's comment documents this as intentional (control.rs:413-416). The desktop side does not poll while a lane is working, so nothing corrects it sooner.
- Sites: `crates/jobs/src/control.rs:420`; `crates/desktop/qml/Main.qml:415`

## U35: Save cropped copies (dialog)

Entry: `crates/desktop/qml/FilterBar.qml:32 saveCropsRequested (emitted from one of the onClicked handlers at FilterBar.qml:273 or :285, not traced in detail) -> Main.qml:579 askSaveCrops (Main.qml:193) -> cropPicker.pick(1)`

### UI-127 [low/missing-feedback] Folder chosen, but nothing is saved and nothing says so — CONFIRMED

- Status: Done (validated).
- Needs: Fix. "Nothing queued" (no borders, or library not running) reports through the shared outcome path. Files: crates/desktop/src/gallery/edits.rs, crates/desktop/qml/Main.qml, crates/app/src/library.rs.
- Verdict: CONFIRMED
- Note: saveCroppedCopiesPicked returns n and queues nothing when borders_found is empty (edits.rs:169-174). In the not-started case, app library.rs:349-352 does nothing when the library is not running. Main.qml:176 drops the return value, so the user gets no message in either case.
- Confidence: medium
- Trigger: (a) The user chooses a folder while no file in the list has Borders::Found any more, so borders_found() is empty; or (b) the library has not started yet (the crop button is usable in the short window after the list shows but before startHooks calls library.start()).
- Consequence: saveCroppedCopiesPicked returns 0 and QML ignores the return value, so no job is queued and no message appears. In case (b) the save_crops call is a silent no-op, and the user sees nothing happen after choosing a folder. The nothing-to-crop popup is shown only at click time.
- Sites: `crates/desktop/src/gallery/edits.rs:171`; `crates/desktop/qml/Main.qml:176`; `crates/app/src/library.rs:349`

### UI-128 [low/wrong-message] "Had nothing to crop" also counts changed and missing files — PARTIAL

- Status: Done (validated).
- Needs: Fix (checked: confirmed). Give changed or missing files their own outcome, separate from nothing-to-crop (crates/crop/src/lib.rs:133, 141, 198, 472; wording at crates/wording/locales/en/text.ftl:238). Files: crates/crop/src/lib.rs, crates/wording/locales/en/text.ftl.
- Verdict: PARTIAL
- Note: The wording is confirmed: the notice says "{skipped} had nothing to crop" (text.ftl:209), and crop lib.rs:137 counts every Skipped as skipped. Whether Written::Skipped covers files that changed or went missing (crop lib.rs:119) was not traced.
- Confidence: high
- Trigger: Save cropped copies runs over a list where some bordered files changed on disk since their borders were found (size or mtime differ), or were removed from the library.
- Consequence: The completion notice says these files "had nothing to crop". They did have borders and were not copied because they changed or went missing, so the wording misstates why they were not saved.
- Sites: `crates/client/locales/en/text.ftl:209`; `crates/crop/src/lib.rs:119`; `crates/crop/src/lib.rs:137`

## U36: Cut borders from originals (dialog)

Entry: `crates/desktop/qml/FilterBar.qml:39 cutBordersRequested (handler at FilterBar.qml:273 or :285) -> Main.qml:580 askCutBorders (Main.qml:198) -> cutDialog.open with count from gallery_bridge.rs:172 cut_borders_count`

### UI-130 [medium/wrong-message] Thumbnail-forget error reported as a failed cut — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Forget errors are their own thumbnail outcome, not CutFailed. Files: crates/library/src/passes.rs.
- Verdict: CONFIRMED
- Note: cut_done posts a CutFailed for each forget error (passes.rs:48-50). The BordersCut summary is posted separately when the cut finishes (coordinator.rs:656-665), so the user sees a failed-cut entry beside a success for the same originals.
- Confidence: high
- Trigger: A cut batch rewrites the originals, but the thumbnail cache refuses to forget one or more cut files (store error during forget).
- Consequence: The inbox shows Cutting the borders failed once per failing file, next to Borders cut from the originals for the same job. The user is told the cut failed when the originals were rewritten, and the thumbnail error is not named as such.
- Sites: `crates/library/src/passes.rs:48`; `crates/library/src/passes.rs:49`

## U38: Rate a photo in the viewer (star bar, keys)

Entry: `crates/desktop/qml/RatingBar.qml:83 onClicked -> rateAsked (RatingBar.qml:22) -> Hud.qml:41 rateAsked -> Viewer.qml:507 onRateAsked; keys Viewer.qml:267-276 (Rate action, Viewer.qml:276)`

### UI-142 [medium/wrong-message] Rating popup shows success before the write, and still shows it when the write fails — CONFIRMED

- Status: Done (validated). edits.rs emits ratingWritten after a successful write, and the popup is shown from it.
- Needs: Fix. Show the popup from rating_written, and show a failure in it through the shared outcome path. Files: crates/desktop/qml/Viewer.qml, crates/desktop/qml/RatingPopup.qml, crates/desktop/src/gallery/edits.rs.
- Verdict: CONFIRMED
- Note: Viewer.qml:521 shows the popup on rated, before the write. The write runs on the worker (edits.rs:108-112). On failure the model is not updated (rating_written is not called) and the error is posted by edit_outcome (app.rs:36-38).
- Confidence: medium
- Trigger: Any rating (a star click or a key) where the catalog or file write fails on the writes' thread (edits.rs:109 returns Err, for example a locked or unwritable file).
- Consequence: The popup confirms the new star over the next photo for about half a second, before the write has run. On failure the item keeps its old rating (rating_written is never called) and the only sign is the inbox notice 'A rating couldn't be saved', which shows after the viewer has already moved on. The user sees success, then an error elsewhere.
- Sites: `crates/desktop/qml/Viewer.qml:521`; `crates/desktop/qml/RatingPopup.qml:16`; `crates/desktop/src/gallery/edits.rs:110`; `crates/desktop/src/app.rs:37`

## U39: Turn a photo in the viewer

Entry: `crates/desktop/qml/Viewer.qml:277-278 keys TurnForward and TurnBack -> turn(+1/-1) (Viewer.qml:190); RatingBar.qml:66 and :91 turnAsked -> Hud.qml:42 -> Viewer.qml:508`

### UI-144 [low/missing-feedback] Turn shows nothing until the catalog write lock is free — CONFIRMED

- Status: Done (validated). The pending turn is set at press, and taken back if the write fails.
- Needs: Fix. Show a pending turn state at press, from pending_turns, before the write. Files: crates/desktop/qml/Viewer.qml, crates/desktop/src/gallery/edits.rs.
- Verdict: CONFIRMED
- Note: Viewer.turn sends turned and resets the view at once (Viewer.qml:190-193). The rotation is queued on the worker (edits.rs:27-34) and its result is applied only after turn_queued. The 5 s wait is the catalog's busy timeout, which I did not read.
- Confidence: high
- Trigger: Press R or click ⟲/⟳ while another connection holds the catalog write lock (an import flush, a scan, or the writer's own transaction). The queue write waits up to 5 s on the window-writes thread, and the picture and the 'turning…' label change only after that write succeeds.
- Consequence: For up to 5 s the photo does not change and no 'turning…' label appears, although the zoom has already reset, so the press looks ignored and may be repeated. If the lock stays busy past the timeout, the turn is dropped; only an inbox notice ('A turn couldn't be saved') says so.
- Sites: `crates/desktop/src/gallery/edits.rs:31`; `crates/desktop/qml/Viewer.qml:190`; `crates/app/src/library.rs:438`; `crates/catalog/src/core.rs:319`

### UI-147 [low/stale-result] A turn can be counted twice if a refresh read lands between the catalog write and turn_queued — CONFIRMED

- Status: Partial (build and tests). The desktop keeps unwritten presses on the model, so a refresh does not drop them. Residual: a second press before the first write is undercounted until the turned files are re-read.
- Needs: Fix (checked: confirmed; the window is narrow). Key pending turns by revision, or stop counting in turn_queued what the refresh read already includes. Files: crates/desktop/src/gallery/edits.rs, crates/client/src/grid.rs.
- Verdict: CONFIRMED
- Note: edits.rs:44 adds the turn to pending_turns from the model thread, and turned_request reads that state. refresh_turned applies only when the revision is unchanged (grid.rs:405-408). The interleaving with turn_queued and whether edit() bumps the revision were not fully traced. Low severity, as the finding says.
- Confidence: low
- Trigger: Press R while a FilesChanged refresh is reading the same pending file. The read is taken after the turn is committed to the catalog, and its result is applied before turn_queued runs on the model thread.
- Consequence: The entry's pending_turns counts the new turn twice, so the HUD and the image show an extra quarter turn. It self-corrects when the writer writes the queued turns and FilesChanged re-reads the file. The window is very narrow unless the worker thread is preempted between the commit and the queue call.
- Sites: `crates/desktop/src/gallery/edits.rs:30`; `crates/desktop/src/gallery/edits.rs:44`; `crates/client/src/grid.rs:405`

## U40: Mark or unmark a border (viewer N key, HUD)

Entry: `crates/desktop/qml/Viewer.qml:284 MarkBorder key -> Viewer.qml:114 borderMarkToggled; RatingBar.qml:59 borderMarkAsked -> Hud.qml:43 -> Viewer.qml:509`

### UI-148 [medium/double-fire] Two quick border-mark toggles collapse into one while the first write is pending — CONFIRMED

- Status: Done (validated). Uses the existing shell.toggleBorderMark.
- Needs: Fix (checked: confirmed; the UI-036 fix covers only the grid). Route the viewer's N key and RatingBar's border button through the toggle the grid uses, so the target comes from border_mark_target. Files: crates/desktop/qml/Viewer.qml, crates/desktop/qml/RatingBar.qml, crates/desktop/src/gallery/edits.rs, crates/desktop/src/shell.rs.
- Verdict: CONFIRMED (duplicate of UI-036)
- Note: Duplicate of UI-036 extended to the viewer: Viewer.qml:284 sends !borderMarked from the viewer's own viewed value, which is also stale until the write lands. Both presses ask for marked = true, and edits.rs:137 drops the second.
- Confidence: high
- Trigger: Press N (grid or viewer) on an unmarked photo with borders found, then press N again before the first write lands. Writes wait for the catalog write lock during an import (worker.rs header), so the window can last seconds and the overlay gives no feedback in that time. The grid toggle reads the stored mark, which is unchanged until the write. The viewer sends the value from its stale viewedBorderMarked. Both send marked=true, and set_border_marked drops the second request as equal to the queued target.
- Consequence: An even number of presses inside the window leaves the photo marked when the user meant to unmark it. The result disagrees with the number of presses, and the user sees no change until the write lands, so nothing tells them to correct it.
- Sites: `crates/desktop/src/gallery/items.rs:131`; `crates/desktop/src/shell.rs:504`; `crates/desktop/qml/Viewer.qml:284`; `crates/desktop/src/gallery/edits.rs:137`; `crates/desktop/src/gallery/edits.rs:129`

### UI-150 [low/missing-feedback] N on a photo with no borders found does nothing and says nothing — CONFIRMED

- Status: Done (validated).
- Needs: Fix. N on a photo with no borders reports through the shared outcome path. Files: crates/desktop/src/gallery/edits.rs, crates/client/src/keys.rs.
- Verdict: CONFIRMED
- Note: keys.rs:118 binds N for any selected item. set_border_marked returns early for an item without borders Found or Marked (edits.rs:134-136), so the key does nothing, and no message is posted.
- Confidence: high
- Trigger: Select a photo with no borders found (the common case) in the grid and press N. The key is bound for any selected item, and the grid hint lists N as not a border.
- Consequence: Nothing changes and no message appears, so the user cannot tell whether the key was received or the photo was refused. The viewer does not bind N unless borders are found, so the two paths differ.
- Sites: `crates/client/src/keys.rs:118`; `crates/desktop/src/gallery/edits.rs:134`

## U41: Show or hide borders (viewer B key)

Entry: `crates/desktop/qml/Viewer.qml:283 Borders key -> bordersToggled (Viewer.qml:113) -> Main.qml:662-665 shell.toggleBorders (shell_bridge.rs:134), or featurePopup when borders are off`

### UI-154 [low/misleading-hint] Grid key hint advertises B and N with border detection off — CONFIRMED

- Status: Done (validated). Main.qml passes the detection flag to the grid hint.
- Needs: Fix. Build the grid hint from the enabled features, so B and N appear only when detection is on. Files: crates/client/src/keys.rs, crates/desktop/src/keymap.rs.
- Verdict: CONFIRMED
- Note: keymap.rs:153-155 calls keys::grid_hint, which always lists Borders and NotBorder (keys.rs:288-291) with no feature check. The status bar shows it with nothing selected (Main.qml:468).
- Confidence: medium
- Trigger: Border detection is off and no item is selected. The status bar shows the grid key hint, which lists B borders and N not a border regardless of the feature.
- Consequence: The hint offers keys that do nothing except open the 'Border detection is off' popup. N also needs a selected item, so it cannot be used from that hint line at all.
- Sites: `crates/desktop/qml/Main.qml:468`; `crates/desktop/src/keymap.rs:153`; `crates/client/src/keys.rs:288`

## U42: Step through pictures in the viewer (wheel, keys, swipe)

Entry: `crates/desktop/qml/Viewer.qml:302-313 WheelHandler (notches), :274-275 Keys StepBack/StepForward, :346 touch swipe -> viewer.step (Viewer.qml:170)`

### UI-158 [medium/wrong-message] Viewer hint says wheel zoom, but the wheel steps pictures — PARTIAL

- Status: Done (build and tests). Decision B: the hint reads Hint::Zoom with ui-hint-zoom, and ui-hint-wheel-zoom is removed.
- Needs: Fix (decision: B, change the hint). Verify the wheel branch (Viewer.qml:300-312) first. The hint is generated from the real bindings and no longer says wheel zoom. Files: crates/client/locales/en/ui.ftl, crates/client/src/keys.rs, crates/desktop/qml/Viewer.qml.
- Verdict: PARTIAL
- Note: The hint lists WheelZoom for stills (keys.rs:304; ui.ftl:231), and the wheel handler calls viewer.step(-n) (Viewer.qml:306-311). I did not read the branch above it at Viewer.qml:300-312 that decides whether a notch zooms when the image is not zoomed.
- Confidence: high
- Trigger: A still image is open. The hint line at the top right reads wheel zoom. The user scrolls expecting zoom. Each notch moves to the previous or next picture and resets the view, losing the zoom and pan.
- Consequence: The hint misleads the user about what the wheel does. Users skip pictures and lose their view position, and cannot find the zoom that the hint promised. The zoom keys are pinch, + and -, and double-click 1:1.
- Sites: `D:/GitHub/photo-gallery/crates/client/locales/en/ui.ftl:231`; `D:/GitHub/photo-gallery/crates/client/src/keys.rs:304`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:310`

## U44: Viewer re-render: fit size and region tiles

Entry: `crates/desktop/qml/Viewer.qml:130-131 onFitWidthChanged/onFitHeightChanged -> fitSettle (Viewer.qml:146-150) -> settleFit (Viewer.qml:140); Viewer.qml:384 ImageLayers onRegionDue`

### UI-166 [low/missing-feedback] A failed region, or an offline original seen while zoomed, shows nothing and logs nothing — CONFIRMED

- Status: Done (build and tests). Region and Original failures are logged, the HUD shows fullFailed and regionFailed, and ui-region-failed is defined.
- Needs: Fix. Region and original failures show in the HUD, and Original errors are logged. Files: crates/desktop/qml/Viewer.qml, crates/app/src/images.rs, crates/desktop/src/providers.rs.
- Verdict: CONFIRMED
- Note: Hud.qml:95 shows "original unavailable" only from fitFailed (Viewer.qml:498). full_image logs only Undecodable (images.rs:317-319), and providers.rs:125-128 skips logging for Original and Undecodable. An Original error or a failed region is therefore invisible to the user and the log.
- Confidence: high
- Trigger: While zoomed, the original goes offline or is removed, or a region request fails for any reason (the file is rewritten past the 10 s wait, or the crop fails). The fit that is already on screen stays Ready.
- Consequence: The Hud reads only the fit's status, so no original unavailable line appears. Region failures are not in the Hud at all. For an Original error, full_image logs nothing (only Undecodable is logged), and providers skips logging Original, so the failure is invisible to both user and log. The view silently keeps the soft fit.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:498`; `D:/GitHub/photo-gallery/crates/desktop/qml/ImageLayers.qml:92`; `D:/GitHub/photo-gallery/crates/app/src/images.rs:317`; `D:/GitHub/photo-gallery/crates/desktop/src/providers.rs:125`

### UI-167 [low/wrong-message] Any fit failure is labelled original unavailable, including undecodable files and rewrite timeouts — CONFIRMED

- Status: Done (build and tests). viewedFitError drives the HUD, and ui-fit-undecodable and ui-fit-rewriting are defined.
- Needs: Fix. Distinct fit error kinds for undecodable files and rewrite timeouts, each with its own string. Files: crates/desktop/qml/Hud.qml, crates/client/locales/en/ui.ftl, crates/app/src/images.rs.
- Verdict: CONFIRMED
- Note: Hud.qml:95 uses fitFailed, which is true for any fit error, so an Undecodable or Rewriting failure (images.rs:189) shows the same "original unavailable" text.
- Confidence: medium
- Trigger: A fit request fails with Error::Undecodable (a file that does not decode) or with Error::Rewriting (the 10 s write wait ran out), while the original is present.
- Consequence: The Hud says the original is unavailable for a file that is there, which misleads the user about the cause. The Rust side logs these, but the user does not see it.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Hud.qml:95`; `D:/GitHub/photo-gallery/crates/client/locales/en/ui.ftl:60`; `D:/GitHub/photo-gallery/crates/app/src/images.rs:189`

## U45: Loupe (10x under the cursor)

Entry: `crates/desktop/qml/Viewer.qml:282 Loupe key -> loupe toggle (Viewer.qml:66 loupe property); Viewer.qml:472-484 Loupe with cursor from HoverHandler (Viewer.qml:469)`

### UI-171 [low/missing-feedback] L with the pointer off the picture shows nothing, but the label says loupe is on — CONFIRMED

- Status: Done (validated). Decision A: the loupe falls back to the last pointer position, or the picture centre.
- Needs: Fix (decision: A, keyboard loupe). The loupe follows the last known pointer position, or the picture centre when the pointer has not been over the picture. Files: crates/desktop/qml/Loupe.qml, crates/desktop/qml/Viewer.qml, crates/desktop/qml/Hud.qml.
- Verdict: CONFIRMED
- Note: Loupe.qml:30 requires hovered, which comes from the HoverHandler in the viewport (Viewer.qml:469). A keyboard L with the pointer off the picture sets the flag and the label without showing anything.
- Confidence: high
- Trigger: The user uses the keyboard only, or parks the pointer on the frame or outside the picture, then presses L. The loupe needs hovered (Loupe.qml:30, from the HoverHandler at Viewer.qml:469), so nothing shows until the pointer enters the picture. Meanwhile the Hud label reads 'loupe 10x' (Hud.qml:93).
- Consequence: The keyboard path does not reach the same result as the mouse path: the loupe appears only after the pointer moves over the image, with no hint that it needs the pointer there.
- Sites: `crates/desktop/qml/Loupe.qml:30`; `crates/desktop/qml/Viewer.qml:469`; `crates/desktop/qml/Hud.qml:93`

### UI-172 [low/missing-feedback] Full-size decode failure leaves the loupe silently absent — PARTIAL

- Status: Done (validated).
- Needs: Fix (checked: confirmed). Add a full-decode error to the HUD line, from full.status === Image.Error (ImageLayers.qml:34 fullReady, :127-130), and keep the existing log. Files: crates/desktop/qml/ImageLayers.qml, crates/desktop/qml/Hud.qml.
- Verdict: PARTIAL
- Note: A failed full decode would leave the full layer in Error and the loupe absent, and Hud shows only the fit status (Viewer.qml:498). fullReady (ImageLayers.qml:33) was not read, so whether it stays false is not settled.
- Confidence: medium
- Trigger: A still whose fit decodes (the scaled decode, display.rs:67) but whose full decode fails (MinSize::Full, display.rs:71; for example a truncated file or a memory failure on a very large original). The user presses L. full.status goes to Error, so fullReady stays false (ImageLayers.qml:33) and the loupe never appears.
- Consequence: The loupe silently does nothing and the label still says 'loupe 10x'. The only original-unavailable message is tied to the fit (Viewer.qml:498), so the user gets no explanation. A later 1:1 view also shows nothing, because the region and the full both fail.
- Sites: `crates/desktop/qml/ImageLayers.qml:121`; `crates/desktop/qml/Viewer.qml:498`; `crates/desktop/src/providers.rs:118`; `crates/render/src/display.rs:71`

## U47: Video and animation transport

Entry: `crates/desktop/qml/VideoTransport.qml:57 playAsked, :79 muteAsked, :68 onMoved seekAsked, :87 onMoved volumeAsked; keys Viewer.qml:286 PlayPause, :287 Mute`

### UI-179 [low/missing-feedback] Video catalog-lookup failure is silent — CONFIRMED

- Status: Done (validated).
- Needs: Fix. A lookup failure sets the player error, as device errors already do. Files: crates/desktop/src/video_source.rs.
- Verdict: CONFIRMED
- Note: video_source.rs:36-38 on Err logs a warning and calls player.clear(), which sets no playerError. The HUD shows a player error only when playerError is set (Hud.qml:96), so the failure is silent apart from the log.
- Confidence: high
- Trigger: Open a video whose catalog lookup fails (the file is removed from the catalog while the viewer is open, or the catalog read errors).
- Consequence: The poster stays, the HUD shows nothing, and nothing indicates the video will not play. A failed device open does show Could not open source device.
- Sites: `crates/desktop/src/video_source.rs:36`; `crates/desktop/src/video_source.rs:38`

### UI-180 [low/missing-feedback] Animation that cannot be read or decoded fails silently — PARTIAL

- Status: Done (build and tests). The animation player declares playerError, and the HUD shows it for animations. Not exercised at runtime.
- Needs: Fix (checked: partly; animation.rs has 72 lines, so :76 and :112 are out of range). Have Player::start report read and decode failures (crates/render/src/player.rs:79, :96, :114) to AnimationPlayer, and show them in the HUD for animations too. Files: crates/render/src/player.rs, crates/desktop/qml/MediaView.qml, crates/desktop/qml/Hud.qml.
- Verdict: PARTIAL
- Note: Citation error: animation.rs:76 and :112 are both past end of file (LINE OUT OF RANGE). The HUD part holds: Hud.qml:96 shows playerError only for Video kind, so an animation failure gets no message. The animation decode path that would set an error was not read, so the failure mechanism is not verified.
- Confidence: medium
- Trigger: Open an animated GIF, APNG or WebP whose file cannot be read or whose frames fail to decode (for example a corrupt file, or one removed after listing while its thumbnail is still cached). Press K.
- Consequence: The still poster stays, the HUD shows no message, and K does nothing visible. Only video reports playerError into the HUD.
- Sites: `crates/desktop/src/animation.rs:76`; `crates/desktop/src/animation.rs:112`; `crates/desktop/qml/Hud.qml:96`

### UI-181 [low/wrong-message] HUD shows untranslated Qt English error text — CONFIRMED

- Status: Partial (build and tests). Player errors show the catalogued playback line. The open and format wording was dropped, because this build's QML compiler cannot compare the player's error enum; restoring it needs a C++ helper.
- Needs: Fix. Map the backend error to a catalogued string; do not show Qt's English. Files: crates/desktop/qml/MediaView.qml, crates/desktop/qml/Hud.qml, crates/client/locales/en/ui.ftl.
- Verdict: CONFIRMED
- Note: MediaView.qml:101 forwards errorString unchanged to playerFailed. Hud.qml:96 wraps it in ui-player-error (ui.ftl:61, "— { $text }"), so the prefix is translated and the body is Qt's English. The string is at qffmpegmediadataholder.cpp:210 ("Could not open source device.").
- Confidence: high
- Trigger: Any playback error from the device or the FFmpeg backend, for example an unopenable file.
- Consequence: The HUD shows the catalogued prefix followed by Qt's English string (for example Could not open source device.). The text is not catalogued, is not translated, and is not pseudo-localized.
- Sites: `crates/desktop/qml/MediaView.qml:101`; `crates/desktop/qml/Hud.qml:96`; `crates/client/locales/en/ui.ftl:61`; `C:/Qt/6.12.1/Src/qtmultimedia/src/plugins/multimedia/ffmpeg/playbackengine/qffmpegmediadataholder.cpp:210`

## U48: Find similar from the viewer

Entry: `crates/desktop/qml/Hud.qml:44 findSimilarAsked (RatingBar.qml:45 same signal) -> Viewer.qml:510 onFindSimilarAsked -> Viewer.qml:116 findSimilar -> Main.qml:672 win.findSimilar (Main.qml:377)`

### UI-184 [low/double-fire] Double-clicking the find-similar button opens and immediately closes the panel — CONFIRMED

- Status: Done (validated).
- Needs: Fix (checked: confirmed; yandexLoader sits above the viewer). Ignore the second click of a double-click on the find-similar button, or have the panel ignore clicks within the double-click interval of opening. Files: crates/desktop/qml/RatingBar.qml, crates/desktop/qml/YandexPanel.qml.
- Verdict: CONFIRMED
- Note: RatingBar.qml:45 opens the panel on the first click (openPanel makes it visible, YandexPanel.qml:42-47). The panel's full-window MouseArea (YandexPanel.qml:105-107) then receives the second click and calls close(). The stacking of the panel over the viewer was not read, but the comment at YandexPanel.qml:103-104 says no click reaches the viewer below.
- Confidence: medium
- Trigger: The user double-clicks '≈' in the viewer. The first click opens the panel and starts the search. The second click lands on the panel's full-window background MouseArea, which closes the panel.
- Consequence: The panel flashes open and closes. The search still runs in the background, and reopening shows its progress, so no result is lost, but the user sees a surprising close with no message.
- Sites: `crates/desktop/qml/RatingBar.qml:45`; `crates/desktop/qml/YandexPanel.qml:107`

### UI-185 [low/missing-keyboard-path] Find similar has no keyboard path in the viewer — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Add find-similar to the shared viewer key map (keys.rs), so every mouse action has a key. Files: crates/client/src/keys.rs, crates/desktop/qml/Viewer.qml.
- Verdict: CONFIRMED
- Note: keys.rs viewer() (keys.rs:124-...) has no find-similar arm, and ViewerAction (Viewer.qml:265) has no find-similar entry. The only keyboard route is the command-line --find-similar hook (YandexPanel.qml:36-40).
- Confidence: high
- Trigger: A keyboard-only user opens a still image in the viewer and wants a find-similar search. No key is bound to it (keys.rs viewer() has no such arm; the ViewerAction enum lacks it).
- Consequence: Only the mouse button reaches the search, and it is hidden in zen mode (H). The command line's --find-similar is the only other route. The gap may be intentional, but the flow has no keyboard equivalent.
- Sites: `crates/client/src/keys.rs:124`; `crates/desktop/qml/Viewer.qml:490`; `crates/desktop/qml/RatingBar.qml:41`

### UI-186 [low/missing-feedback] Dead results are dropped from the list with no note — CONFIRMED

- Status: Done (validated).
- Needs: Fix. The status line says how many results were dropped as dead, through the shared outcome path. Files: crates/desktop/src/yandex.rs, crates/app/src/yandex/images.rs.
- Verdict: CONFIRMED
- Note: yandex.rs:372-374 calls drop_dead without setting any status, and the status comes only from poll.status (yandex.rs:375-376), which carries the search's note and not the download failures.
- Confidence: medium
- Trigger: A search finds several results whose files cannot be fetched or decoded. The fetchers mark them dead, and poll() reports them, so drop_dead removes the rows.
- Consequence: The 'Other sizes (N)' and 'Similar images (N)' counts drop with no status line saying that some results could not be loaded. The user cannot tell a short list from a failed one.
- Sites: `crates/desktop/src/yandex.rs:372`; `crates/desktop/src/yandex.rs:241`

## U49: Yandex search results load

Entry: `crates/desktop/qml/YandexPanel.qml:31-34 show -> yandex.searchFile; command line: Main.qml:382-385 findSimilarFromCommandLine -> YandexPanel.qml:37-40 showCommandLine -> yandex.searchCommandLine (yandex_bridge.rs:79, yandex.rs:129)`

### UI-190 [medium/wrong-message] 'No results.' shown when every found result failed to load — CONFIRMED

- Status: Done (validated). The dropped-results status is set, which hides the No results label.
- Needs: Fix. Status text when every result failed to load, instead of "No results". Files: crates/desktop/qml/YandexPanel.qml, crates/app/src/yandex.rs, crates/client/src/text.rs.
- Verdict: CONFIRMED
- Note: YandexPanel.qml:255 shows "No results" when not busy, both counts are 0 and status is "". Status is set at search completion (app/yandex.rs:462) and maps to empty when no note is given (text.rs:415), before any download fails. So it stays empty and the rows dropped by drop_dead (yandex.rs:372-374) leave the empty-result label.
- Confidence: high
- Trigger: Yandex returns results, but every result file fails to download or decode (for example the image hosts are unreachable). Each result is settled dead, and drop_dead removes all rows. The search ended with no note, so the status stays empty.
- Consequence: The panel shows 'No results.' as if Yandex found nothing. The user is not told that the results were found and failed to download.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:261`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:255`; `D:/GitHub/photo-gallery/crates/app/src/yandex.rs:462`; `D:/GitHub/photo-gallery/crates/client/src/text.rs:415`

### UI-193 [low/focus] No keyboard path to pick or open an individual result — CONFIRMED

- Status: Done (validated). Tab reachability at runtime is not tested.
- Needs: Fix. Make ResultTile a focusable FocusScope with keys for pick and open. This also fixes UI-196. Files: crates/desktop/qml/ResultTile.qml, crates/desktop/qml/YandexPanel.qml.
- Verdict: CONFIRMED
- Note: ResultTile.qml:97-113 is a MouseArea only, with no focus or key handling. YandexPanel.qml:97-102 accepts only Close, so no tile can be picked or opened from the keyboard.
- Confidence: medium
- Trigger: A keyboard-only user has opened the Yandex panel and tries to pick a result or open one at full size.
- Consequence: Only the mouse can pick a single result (a click, or Shift-click for a range) or open it (a double-click). Tab reaches the group Select all buttons and Close, but no tile can take focus, so the keyboard path does not reach the mouse path's results.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/ResultTile.qml:100`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:98`

## U50: Pick Yandex results, shift-range pick

Entry: `crates/desktop/qml/ResultTile.qml:100 onClicked -> toggled (ResultTile.qml:34) -> YandexPanel.qml:223, :245 yandex.togglePick; ResultTile.qml:35 rangeTo -> YandexPanel.qml:224, :246 yandex.pickTo`

### UI-197 [low/unhandled-close] Quitting while a save runs cuts it short with no record of what was saved — CONFIRMED

- Status: Done (validated). Caveat: a save still running after the 2 s stop wait is not recorded at exit.
- Needs: Fix. Report an unfinished save through the outcome path, as UI-011 does. Files: crates/app/src/app.rs, crates/app/src/yandex.rs.
- Verdict: CONFIRMED
- Note: app.rs:156-158 logs a warning when yandex.stop() returns false and then carries on with shutdown. Nothing reaches the user or the inbox, so an unfinished save is not reported.
- Confidence: medium
- Trigger: The user picks several results, starts Save, and quits the app while the files are still being written or fetched.
- Consequence: Yandex::stop waits at most 2 s, then the app shuts down while the save worker is still running. The remaining picked files are never written, and no 'saved N of M' status is shown. Files already written are complete, because write_new renames a temp file into place. This is a documented trade-off (a slow request is left to end with the process), but the user is not told the save was incomplete.
- Sites: `crates/app/src/app.rs:156`; `crates/app/src/yandex.rs:498`; `crates/app/src/yandex/images.rs:194`

## U51: Select all of a Yandex group

Entry: `crates/desktop/qml/YandexPanel.qml:81 onClicked -> yandex.setPicked(first, count, picked)`

### UI-198 [medium/wrong-message] Second photo's search is silently dropped while busy; old results stay shown — CONFIRMED

- Status: Done (validated). Results carry their photo, so a later search does not land under the new photo.
- Needs: Fix (checked: partly; 76fb1c43 already reports the refusal, but the previous photo's results and picks stay). Clear the results and picks when a new photo's search is accepted, so Save never acts on the previous photo's picks. Files: crates/desktop/src/yandex.rs, crates/desktop/qml/YandexPanel.qml.
- Verdict: CONFIRMED (duplicate of UI-182)
- Note: Duplicate of UI-182 and UI-188 for the viewer's show path: YandexPanel.qml:31-34 opens the panel and calls searchFile, and search_photo returns on Ok(false) (yandex.rs:141) while busy (app/yandex.rs:425-426) without changing status or results.
- Confidence: high
- Trigger: A save or a search is running, the user closes the panel, and opens Find similar on a different photo. show(fileId) calls openPanel() then searchFile, and start() returns Ok(false) because busy is set, so search_photo returns without a status or a change to the results.
- Consequence: The panel shows the previous photo's results and picks under the Find similar title, with the running job's status and no notice that the new photo was not searched. Select all, the picks and Save then act on the old photo's results as if they were the new photo's.
- Sites: `crates/desktop/qml/YandexPanel.qml:31`; `crates/desktop/src/yandex.rs:140`; `crates/app/src/yandex.rs:425`

### UI-200 [low/stale-state] Picks are discarded on close and reopen because each open starts a new search — CONFIRMED

- Status: Done (validated). Picks are kept per photo path.
- Needs: Fix. Key picks by photo id and keep them for the session, so reopening the same photo keeps them. Files: crates/desktop/qml/YandexPanel.qml, crates/desktop/src/yandex.rs.
- Verdict: CONFIRMED
- Note: A new open runs search_photo, which calls show(Results::default()) (yandex.rs:147), so the picks reset. close() only hides the panel (YandexPanel.qml:49-52) and gives no message.
- Confidence: medium
- Trigger: The user picks results (with Select all or single clicks), closes the panel without saving, and opens Find similar again for the same photo. show() runs a new search, and search_photo resets the results and picks to none.
- Consequence: The picks are lost without a message. The user re-picks, and each reopen sends the photo to Yandex again.
- Sites: `crates/desktop/qml/YandexPanel.qml:49`; `crates/desktop/src/yandex.rs:147`

## U52: Yandex full-size preview

Entry: `crates/desktop/qml/ResultTile.qml:36 previewRequested via double-click (ResultTile.qml:108) -> YandexPanel.qml:225, :247 preview.open (YandexPreview.qml:51)`

### UI-203 [low/keyboard-parity] The 1:1 zoom has no keyboard path — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Add the 1:1 key to the shared viewer key map, and use it in the Yandex preview. Files: crates/desktop/qml/YandexPreview.qml, crates/client/src/keys.rs.
- Verdict: CONFIRMED
- Note: YandexPreview.qml:155-160 sets NoFocus on the 1:1 button, and the panel key map (keys.rs:186-193) has no zoom or real-size action. Only a mouse click or a tap reaches 1:1.
- Confidence: high
- Trigger: A keyboard user opens a result in the preview and wants to switch to 1:1. The 1:1 button has focusPolicy NoFocus and no key is mapped to zoom, so only a mouse click or a tap on the picture reaches 1:1.
- Consequence: Keyboard-only users cannot reach the 1:1 view that the mouse path provides. The viewer has a real-size key (Z), but the Yandex preview has none.
- Sites: `crates/desktop/qml/YandexPreview.qml:155`; `crates/desktop/qml/YandexPreview.qml:158`; `crates/client/src/keys.rs:186`

### UI-204 [low/missing-feedback] Preview closes silently when its picture turns out dead — CONFIRMED

- Status: Done (validated). The report is on the status line; the preview still closes.
- Needs: Fix. A dead preview picture reports through the shared outcome path, not a silent close. Files: crates/desktop/src/yandex.rs, crates/desktop/qml/YandexPanel.qml.
- Verdict: CONFIRMED
- Note: images.rs:187-189 publishes a change when a result settles dead. yandex.rs:260 sets the preview item to -1 when its picture is gone, and YandexPanel.qml:288-289 closes the preview. The status line is not changed on this path. The short "Can't load or decode it" label was not traced.
- Confidence: high
- Trigger: The user double-clicks a result whose file is still being fetched in the background (prefetch). The fetch or its decode fails, so Images::settle marks the result Dead and publishes a change. The poll then calls drop_dead, which sets the preview's item to -1 because the shown picture is gone.
- Consequence: The preview closes and the tile vanishes from the grid. The status line does not change and no reason is shown. The 'Can't load or decode it' label appears only until the next poll.
- Sites: `crates/app/src/yandex/images.rs:187`; `crates/desktop/src/yandex.rs:260`; `crates/desktop/qml/YandexPanel.qml:288`

## U53: Save picked Yandex results (dialog)

Entry: `crates/desktop/qml/YandexPanel.qml:164-170 Save button onClicked -> savePicker.pick(2)`

### UI-206 [low/wrong-message] Picks that die while the folder dialog is open are dropped silently, and if all die the save says Pick at least one image — PARTIAL

- Status: Done (validated). The logic is in yandex.rs; the cited yandex_bridge.rs lines are out of range.
- Needs: Fix (checked: confirmed; yandex_bridge.rs:449-455 are out of range, the logic is in crates/desktop/src/yandex.rs:508-531). Check the picks before taking the folder (yandex.rs:508-517), and report dropped picks through the status line in poll (yandex.rs:421-439). Files: crates/desktop/src/yandex.rs, crates/desktop/src/bridges/yandex_bridge.rs.
- Verdict: PARTIAL
- Note: Citation error: yandex_bridge.rs:449, 453 and 455 are past the end of the file (LINE OUT OF RANGE). The save path could not be checked at those lines. The general claim that a pick dying while the folder dialog is open is dropped is consistent with drop_dead (yandex.rs:241) and the poll (yandex.rs:372-374), but the empty-set path into "Pick at least one image" was not read.
- Confidence: medium
- Trigger: A pick's file fails to fetch while the Save folder dialog is open. poll runs during the dialog (Qt events run on the Qt thread during the dialog's own loop) and drop_dead removes the row and its pick. If that was the only pick, download_picked reads an empty set after take has already consumed the folder.
- Consequence: The user is told 'Pick at least one image to save.' although they picked one. The chosen folder is discarded and must be picked again. With several picks the rest are saved and the dead one vanishes from the grid with no message.
- Sites: `crates/desktop/src/yandex_bridge.rs:449`; `crates/desktop/src/yandex_bridge.rs:453`; `crates/desktop/src/yandex_bridge.rs:455`

### UI-208 [low/wrong-message] Failed save with nothing written reads Saved 0, 3 failed — CONFIRMED

- Status: Done (validated). The text is in crates/wording/src/message.rs. The cited crates/client paths do not exist.
- Needs: Fix. Wording for a save that wrote nothing: a new string, not "Saved 0". Files: crates/client/src/text.rs, crates/client/locales/en/text.ftl.
- Verdict: CONFIRMED
- Note: text.rs:346-350 renders YandexSavedFailed with written = 0 when nothing was written, and text.ftl:157 reads "Saved { $written }, { $failed } failed". The string starts with "Saved 0".
- Confidence: high
- Trigger: Every file of the save fails (for example the folder is read-only or was removed after it was chosen).
- Consequence: The status line begins with 'Saved' for a save that wrote nothing. The failure is reported, but the wording suggests partial success.
- Sites: `crates/client/src/text.rs:346`; `crates/client/locales/en/text.ftl:157`

### UI-209 [low/missing-feedback] In-app folder dialog's Up or open failure is shown only in the dimmed status line behind it — CONFIRMED

- Status: Done (build and tests). The dialog's failure label is bound to listError in all three pickers.
- Needs: Fix. Show a folder-list failure as a state inside the dialog. Files: crates/platform/src/picker.rs, crates/desktop/qml/FolderPickerDialog.qml.
- Verdict: CONFIRMED
- Note: picker.rs:170-176 calls report_unlisted on an unlistable folder, which emits failed (picker.rs:160-166). The save picker forwards it to showFailure (YandexPanel.qml:125), so the message goes to the panel's status line. The scrim (ModalOverlay.qml:26, #b0000000) dims that line under the dialog. The Windows-only gate (picker.rs:16) was not read.
- Confidence: high
- Trigger: Off Windows only (NATIVE is cfg!(windows), platform picker.rs:16). The user clicks Up or opens a folder that can't be listed.
- Consequence: The dialog stays on the old folder with no message of its own. The error goes to the panel's status line, which sits under the dialog's dimmed scrim, so it is hard to see while the dialog is open.
- Sites: `crates/desktop/src/picker.rs:174`; `crates/desktop/qml/YandexPanel.qml:125`; `crates/desktop/qml/ModalOverlay.qml:26`; `crates/desktop/qml/FolderPickerDialog.qml:51`

## U54: Close the Yandex panel and its preview

Entry: `crates/desktop/qml/YandexPanel.qml:171 Close button and :107 background onClicked -> close (YandexPanel.qml:49); :97-102 Keys (Escape); YandexPreview.qml:161 Back button and :51-57 open/close`

### UI-210 [high/stale-result] Refused Find similar leaves the previous photo's results on screen — CONFIRMED

- Status: Done (validated). Same fix as UI-198.
- Needs: Fix (checked: partly; the refusal now sets a status, but the previous photo's results stay). Clear the results and picks, or name the photo in the panel, on refusal. The closed-search case also returns Ok(false), so the status text must cover both. Same fix as UI-198. Severity stays high. Files: crates/desktop/src/yandex.rs, crates/desktop/qml/YandexPanel.qml.
- Verdict: CONFIRMED (duplicate of UI-182)
- Note: Duplicate of UI-182 and UI-188 for the close-then-reopen case: YandexPanel.qml:31-34 opens the panel, yandex.rs:141 returns on Ok(false) while busy (app/yandex.rs:425-426), and the panel keeps A's results and state.
- Confidence: high
- Trigger: Find similar on photo A, then close the panel (Escape or Close) while A's search is still uploading or working. Open photo B in the viewer and click Find similar before A's search ends. The app refuses B's search because one is running, and the panel opens showing A's busy state.
- Consequence: The panel shows A's spinner, status and, once A's search finishes, A's similar results, with no message that B's search never started. The user reads A's results as B's. Nothing in the panel names the photo it is showing.
- Sites: `crates/desktop/qml/YandexPanel.qml:33`; `crates/desktop/src/yandex.rs:141`; `crates/app/src/yandex.rs:425`

### UI-211 [low/focus] Hidden save-folder dialog keeps focus, so Escape no longer closes the Yandex panel — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Shared focus policy: a hidden dialog never keeps focus. Files: crates/desktop/qml/ModalOverlay.qml, crates/desktop/qml/FolderPickerDialog.qml.
- Verdict: CONFIRMED
- Note: The same Qt rule (qquickitem.cpp:6641-6645, 6669-6681) means the hidden overlay keeps activeFocus, so Escape reaches ModalOverlay's Keys.onEscapePressed (ModalOverlay.qml:22) and not YandexPanel's handler.
- Confidence: medium
- Trigger: On a platform where FolderPicker.native is false (not Windows): Save picked opens the in-app folder dialog, then Choose, Cancel, Escape or a backdrop click hides it. The hidden ModalOverlay still holds keyboard focus, and Qt keeps delivering keys to it.
- Consequence: The hidden overlay consumes Escape (dismissed calls cancel again) and never reaches YandexPanel's Keys handler, so Escape does not close the panel and the arrows and Space do not reach the preview. Only a click that takes focus recovers the keys. Mouse use still works. On Windows this path does not occur because the native dialog is used.
- Sites: `crates/desktop/qml/ModalOverlay.qml:21`; `crates/desktop/qml/ModalOverlay.qml:22`; `crates/desktop/qml/YandexPanel.qml:127`; `crates/desktop/qml/FolderPickerDialog.qml:85`; `crates/platform/src/picker.rs:16`

## U55: Yandex thumbnails settle

Entry: `crates/desktop/qml/ResultTile.qml:55 onStatusChanged -> thumbSettled (ResultTile.qml:38) -> YandexPanel.qml:222, :244 yandex.thumbSettled`

### UI-213 [medium/unhandled-close] Closing the window waits for an in-flight Yandex download before the process exits — CONFIRMED

- Status: Done (validated). Caveat: close still waits on the providers.cpp waitForDone for a read already in progress.
- Needs: Fix. A cancellation token through the fetcher, honoured at close, instead of waiting for downloads. Files: crates/desktop/src/lib.rs, crates/desktop/cpp/providers.cpp, crates/app/src/yandex/images.rs.
- Verdict: CONFIRMED
- Note: lib.rs:221-224 runs app::stop and then drop(engine) before app::shutdown. providers.cpp:131-135 calls waitForDone on the pool, which blocks while a fetch is running. Yandex::stop cannot cut a request short (app/yandex.rs:100-102), so the process waits for the download to end. The 15-minute body timeout was not read.
- Confidence: high
- Trigger: The user closes the app while a Yandex result's file is being downloaded for a grid thumbnail (the turn row) or the full-size preview (the original). The provider job is blocked in Images::get -> Fetcher download.
- Consequence: The window disappears but the process stays alive until the download finishes or its body timeout (15 minutes) fires. That looks like a hang. Catalog connections and the WAL merge in app::shutdown wait too, since they run after drop(engine). The stop comment in yandex.rs says a slow request is left to end with the process, which is not what happens.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:223`; `D:/GitHub/photo-gallery/crates/desktop/cpp/providers.cpp:131`; `D:/GitHub/photo-gallery/crates/desktop/cpp/providers.cpp:135`; `D:/GitHub/photo-gallery/crates/app/src/yandex.rs:100`; `D:/GitHub/photo-gallery/crates/app/src/yandex/images.rs:194`

## U57: Status bar toast

Entry: `crates/desktop/qml/StatusBar.qml:107-111 Timer -> toastExpired (StatusBar.qml:39) -> Main.qml:562 inboxModel.dismissToast`

### UI-219 [low/stale-toast] A warning toast about an already-read notice survives closing the inbox — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Derive the toast from notice state, so it clears when its notice is marked read. Files: crates/desktop/qml/Main.qml, crates/desktop/src/notices.rs.
- Verdict: CONFIRMED
- Note: StatusBar.qml:110 runs the toast timer only for level 0, so a warning stays until it is dismissed. Main.qml:66 dismisses the toast only on open, and Main.qml:70 closes the inbox with setViewing and no dismissal. notices.rs:466 sets marking on close, which covers the read state.
- Confidence: medium
- Trigger: The inbox is open in an active window when a warning-level state notice is posted. The writer toasts it (news ignores viewing), mark_shown_read marks it read at once, and the toast stays up because only opening the inbox dismisses it. The user then closes the inbox.
- Consequence: A warning stays in the status bar, after the inbox was closed, for a notice the user has already seen and that is already read. It stays until replaced or the inbox is opened again. The toast does not match the read state of the inbox.
- Sites: `D:/GitHub/photo-gallery/crates/app/src/inbox.rs:1038`; `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:466`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:66`

### UI-220 [low/missing-feedback] A failed spawn of the toast follower leaves the toast silently dead — CONFIRMED

- Status: Done (validated).
- Needs: Fix. If the follower thread fails to start, retry it, and report a failure through the outcome path. Files: crates/desktop/src/notices.rs.
- Verdict: CONFIRMED
- Note: notices.rs:344-346 logs a warning when the follower thread fails to start and does nothing else, so no toast or listing update reaches the status bar.
- Confidence: medium
- Trigger: The 'inbox view' thread cannot be started (resource exhaustion) when the window starts.
- Consequence: No toast, and no listing update, ever reaches the status bar for the rest of the session. The only signal is a warn log line; the status bar shows nothing and the unread badge stays at its start value.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:344`; `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:345`

## U58: Open the inbox (status bell, shortcut)

Entry: `crates/desktop/qml/StatusBar.qml:105 TapHandler and :118 onClicked -> inboxRequested (StatusBar.qml:40) -> Main.qml:563 shell.globalAction(Inbox); shortcut GlobalShortcuts.qml:24-30`

### UI-221 [medium/double-fire] Failed mark is re-sent on every listing with no bound — CONFIRMED

- Status: Done (validated).
- Needs: Fix. A failed mark is stored as a failed state and is re-sent only after a new event, not on every listing. Files: crates/desktop/src/notices.rs, crates/app/src/inbox.rs.
- Verdict: CONFIRMED
- Note: inbox.rs:923 sets force when a mark write fails. inbox.rs:903-906 then refreshes and publishes Changed (inbox.rs:1074). notices.rs:400-406 re-marks the shown rows while marking is set, and notices.rs:452 sends them to the writer again. No backoff or bound appears on this path.
- Confidence: high
- Trigger: The inbox is open in an active window and a mark of a shown entry fails in the writer with a non-busy database error (for example SQLITE_LOCKED, a full disk, or a read-only catalog). show() re-marks the rows still unread in the listing that follows.
- Consequence: The writer logs 'the inbox couldn't be changed', force-refreshes, and publishes Changed. The window re-marks the same rows, which fails again. The cycle has no pause and no bound: the writer and the Qt thread keep working, the warning log grows without limit, and the rows repaint on every cycle. It stops only when the inbox closes or the window deactivates.
- Sites: `crates/desktop/src/notices.rs:357`; `crates/desktop/src/notices.rs:404`; `crates/desktop/src/notices.rs:452`; `crates/app/src/inbox.rs:903`; `crates/app/src/inbox.rs:923`; `crates/app/src/inbox.rs:1074`

## U59: Inbox select and details

Entry: `crates/desktop/qml/InboxPanel.qml:331-337 TapHandler -> panel.select (InboxPanel.qml:101); keys InboxPanel.qml:213-241 (Up, Down, Home, End, PageUp, PageDown, Enter or Space for details, Delete, Ctrl+C, R)`

### UI-228 [low/missing-feedback] A failed Retry or Dismiss write is only logged, so the user gets no explanation — PARTIAL

- Status: Done (validated).
- Needs: Fix (checked: confirmed). Report failed Dismiss and Retry writes through the outcome path at the drain error arm (inbox.rs:933-937); NoticesLost counts notices only, so these are silent. Files: crates/app/src/inbox.rs, crates/desktop/src/notices.rs.
- Verdict: PARTIAL
- Note: inbox.rs:922-927 forces a refresh, adds to self.lost and logs a warning, which matches the finding. Whether self.lost is shown anywhere in the UI was not read, so "only logged" is not settled.
- Confidence: high
- Trigger: Click Retry, or dismiss an entry, when the inbox writer's write fails with a non-busy catalog error.
- Consequence: A failed Retry leaves the entry as it was with no message. A failed Dismiss makes the row reappear on the forced re-read with no explanation. The user cannot tell that the action failed.
- Sites: `crates/app/src/inbox.rs:922`

## U60: Copy an inbox entry

Entry: `crates/desktop/qml/InboxPanel.qml:485 onClicked copyRequested; Ctrl+C at InboxPanel.qml:233-235`

### UI-231 [medium/missing-feedback] Copy of a ledger silently truncates at 1000 items with no note — CONFIRMED

- Status: Done (build and tests). The copy line says the newest N of M files. The scan-folders copy still says files, which is a cross-lane gap.
- Needs: Fix. A truncated copy ends with the "newest N of M" line, from the same formatter as the panel. Files: crates/desktop/src/notices.rs, crates/client/locales/en/text.ftl.
- Verdict: CONFIRMED
- Note: notices.rs:142 caps the read at COPIED_FILES = 1000, and notices.rs:512 keeps only failed.items. The counts are dropped, so no "newest N of M" line is added to the copy, unlike the panel's ui-inbox-more.
- Confidence: high
- Trigger: Copy a ledger entry (a pass's failed files, or the scans' folders) that holds more than 1000 rows. The read is limited to COPIED_FILES and the counts are dropped, so the clipboard gets 1000 lines and nothing says the rest are missing.
- Consequence: The user takes the copied text for the full list when it is partial. The inbox panel itself says "The newest N of M files" (ui.ftl ui-inbox-more), but the copy has no equivalent line.
- Sites: `crates/desktop/src/notices.rs:142`; `crates/desktop/src/notices.rs:512`; `crates/desktop/src/notices.rs:182`

### UI-234 [low/shadowed-key] Clicking the details text takes focus and Ctrl+C no longer copies the entry — CONFIRMED

- Status: Done (validated).
- Needs: Fix. The read-only details text does not take focus, so the list keeps Ctrl+C. Shared focus policy. Files: crates/desktop/qml/InboxPanel.qml.
- Verdict: CONFIRMED
- Note: InboxPanel.qml:351-355 is a readOnly TextArea, which takes focus. qquicktextcontrol.cpp:833-836 accepts QKeySequence::Copy and copies the selection only, so the list handler at InboxPanel.qml:233 never runs.
- Confidence: medium
- Trigger: The details of a kept notice are open, and the user clicks in the details text (inboxDetails). The readOnly TextArea takes focus, and Ctrl+C goes to it. The text control handles QKeySequence::Copy by copying only the selection, so the list's Ctrl+C handler never runs. With no selection, nothing is copied. Delete and the arrow keys are shadowed the same way.
- Consequence: Ctrl+C copies just the selected words, or nothing, where the user expects the entry. The keyboard path to Copy no longer matches the mouse path. Nothing reports the difference.
- Sites: `crates/desktop/qml/InboxPanel.qml:351`; `crates/desktop/qml/InboxPanel.qml:233`; `C:\Qt\6.12.1\Src\qtdeclarative\src\quick\items\qquicktextcontrol.cpp:833`

### UI-235 [low/missing-feedback] Copy gives no confirmation, and a ledger copy can land after the user has pasted — CONFIRMED

- Status: Done (validated).
- Needs: Fix. A busy state on Copy, and a confirmation once the clipboard is written from the read result. Files: crates/desktop/qml/InboxPanel.qml, crates/desktop/src/notices.rs.
- Verdict: CONFIRMED
- Note: InboxPanel.qml:481-485 leaves the button enabled and shows no busy state. The clipboard write is queued only after the read (notices.rs:510-520), so the write can land after a paste made in between.
- Confidence: high
- Trigger: Click Copy or press Ctrl+C on a ledger entry while the reader is busy. Nothing changes on screen (no busy state, no toast, the button stays enabled). The clipboard is written only when the read is delivered, so a paste made in the meantime gets the previous clipboard contents.
- Consequence: The user gets no sign that the copy is pending or has finished, and a quick paste returns the old clipboard.
- Sites: `crates/desktop/qml/InboxPanel.qml:481`; `crates/desktop/src/notices.rs:510`

### UI-236 [low/invisible-characters] Clipboard text for a kept notice carries bidi isolate controls — CONFIRMED

- Status: Done (validated). The fix is in crates/wording. The cited crates/client/src/text/notice.rs does not exist.
- Needs: Fix. The copy path uses a plain-text formatter with no isolate characters, rather than stripping them afterwards. Files: crates/client/src/text/notice.rs, crates/desktop/src/notices.rs.
- Verdict: CONFIRMED
- Note: text/notice.rs:21-24 wraps a value in U+2068 and U+2069 through isolated(). copy_words takes notice_line unchanged (notices.rs:183, text.rs:325-329), and nothing strips the characters. Which notice kinds call isolated() was not checked.
- Confidence: medium
- Trigger: Copy a kept notice whose title or body names a path (for example a folder already added, or a scan failure). Its title and body come from args(), which wraps paths in U+2068 and U+2069 through isolated(). copy_words takes notice_line unchanged, so the clipboard gets those invisible characters. Failed-file lines written by the same copy are not wrapped, so the convention is mixed.
- Consequence: Pasting the copied text into another program can carry invisible characters around the path, which can break a paste into a path field or a shell. The copy stays readable.
- Sites: `crates/client/src/text/notice.rs:21`; `crates/client/src/text.rs:325`; `crates/desktop/src/notices.rs:183`

## U61: Dismiss an inbox entry, clear all

Entry: `crates/desktop/qml/InboxPanel.qml:491 onClicked dismissSelected (-> dismissRequested, InboxPanel.qml:52); InboxPanel.qml:498-501 Clear onClicked -> unselect and clearRequested (InboxPanel.qml:54)`

### UI-237 [medium/stale-state] Clear all leaves the entries past 200 that show at once after it — CONFIRMED

- Status: Done (validated). Decision A: clear by filter, through dismiss_all_listed in the catalog.
- Needs: Fix (decision: A, clear every matching entry). Clear all clears by filter in the catalog, up to the 1000 kept entries, not a list of the 200 IDs shown. Files: crates/desktop/src/notices.rs, crates/catalog/src/inbox.rs.
- Verdict: CONFIRMED
- Note: notices.rs:489 takes the ids of the rows the window holds, and the writer clears those. catalog/inbox.rs:211 lists only the first 200 rows afterwards, so the rest appear after the clear. The unread count covers all entries (app inbox.rs:1051). The 1000-event retention was not read.
- Confidence: high
- Trigger: The inbox holds more than LISTED (200) kept entries (events are kept up to 1000) and the user clicks Clear all. The window names only the 200 rows it holds (notices.rs:489), the writer dismisses those, then its refresh (app/inbox.rs:903-905) lists what is left.
- Consequence: The panel empties and then, within a moment, shows the remaining entries (up to the rest of the inbox), and the status bar's unread count is set to count them. Clear all looks like it failed for those entries. The label 'Clear all' (ui.ftl:99) and the 'Clear all dismisses the entries displayed' contract are accurate only for the first 200.
- Sites: `crates/desktop/src/notices.rs:489`; `crates/catalog/src/inbox.rs:211`; `crates/catalog/src/inbox.rs:254`; `crates/app/src/inbox.rs:1051`; `crates/app/src/inbox.rs:1057`; `crates/app/src/inbox.rs:903`

### UI-240 [low/stale-result] A listing computed before a dismissal can put the dismissed entry back until the next listing — CONFIRMED

- Status: Done (validated). Stale listings are filtered rather than dropped, and the outcome is the same.
- Needs: Fix. Sequence numbers on listings; a listing older than the latest is dropped. Files: crates/desktop/src/notices.rs, crates/app/src/inbox.rs.
- Verdict: CONFIRMED
- Note: notices.rs:357 applies a pending listing whenever the Qt thread runs it, and notices.rs:476-484 removes the row at once on dismiss. The order is not checked, so an older listing can add the entry and reset the count (notices.rs:400) until the next refresh. The window in which this happens is short.
- Confidence: medium
- Trigger: The writer publishes a listing that still contains entry E (for example after a thumbnail failure is written). Its pending pass is queued to the Qt thread. The user clicks Dismiss on E before that pass runs. The pass then applies the older listing.
- Consequence: show() inserts E again (diff against the rows already removed by the optimistic dismissal) and resets the unread count to the older listing's value, so E and its badge count reappear for the short time until the writer's next listing. That listing does come, because a successful dismissal triggers a refresh. The window's optimistic state is not re-checked against pending results.
- Sites: `crates/desktop/src/notices.rs:357`; `crates/desktop/src/notices.rs:476`; `crates/desktop/src/notices.rs:400`; `crates/desktop/src/notices.rs:484`; `crates/app/src/inbox.rs:903`

### UI-241 [low/missing-feedback] A dismissal the catalog refuses reappears with no message — CONFIRMED

- Status: Done (validated).
- Needs: Fix. A refused dismissal reverts the optimistic removal and reports through the shared outcome path. Files: crates/app/src/inbox.rs, crates/desktop/src/notices.rs.
- Verdict: CONFIRMED
- Note: inbox.rs:922-929 sets force, adds to lost and logs a warning. The window removed the entry optimistically (notices.rs:476-484), and the forced refresh brings it back. Nothing in the UI reports the failure.
- Confidence: high
- Trigger: A dismissal or Clear all reaches the writer and the catalog write fails with a non-busy error (for example an I/O or disk error). Busy writes are retried, so this is the rare path.
- Consequence: The window had removed the entry optimistically. The writer then forces a refresh (out.force) and the entry, or the whole cleared set, reappears, with only a log line. The panel and status bar say nothing about the failed dismissal, so it looks as if the dismissal was undone for no reason.
- Sites: `crates/app/src/inbox.rs:923`; `crates/app/src/inbox.rs:929`; `crates/desktop/src/notices.rs:478`; `crates/desktop/src/notices.rs:484`

### UI-242 [low/unhandled-close] A dismissal is dropped at shutdown if the catalog stays busy past the 5 s drain — CONFIRMED

- Status: Partial (validated). Writes are not persisted before ack, and an abandoned queue insert drops the dismissal past the 5 s drain. Needs your decision.
- Needs: Fix. Durable queue: persist each dismissal before acknowledging it, so the shutdown drain cannot drop it. Files: crates/app/src/inbox.rs.
- Verdict: CONFIRMED
- Note: inbox.rs:932-943 handles Abandoned by logging the count of notices and changes that were not written and then breaking. Nothing is shown to the user, and the dismissal is not replayed.
- Confidence: high
- Trigger: The user dismisses an entry and then quits while another connection holds the catalog's write lock for longer than the writer's shutdown drain (5 s), for example during a scan.
- Consequence: Inbox::close abandons the queued Dismiss, logging only that the user's changes weren't written. The entry is listed again on the next launch, and the user is told nothing. The window closes without a warning.
- Sites: `crates/app/src/inbox.rs:889`; `crates/app/src/inbox.rs:932`; `crates/app/src/inbox.rs:939`; `crates/app/src/inbox.rs:547`

## U62: Retry an inbox entry

Entry: `crates/desktop/qml/InboxPanel.qml:473 onClicked retrySelected (InboxPanel.qml:125) -> retryRequested (InboxPanel.qml:53); key R at InboxPanel.qml:236`

### UI-244 [low/missing-feedback] A failed Retry shows nothing to the user — CONFIRMED

- Status: Done (validated).
- Needs: Fix. A failed Retry reports through the shared outcome path. Files: crates/app/src/inbox.rs.
- Verdict: CONFIRMED
- Note: inbox.rs:922-929 forces a refresh and logs a warning, with no message to the user. The test at inbox.rs:1562-1564 ("a_failed_retry_wakes_nothing") shows the same path.
- Confidence: high
- Trigger: The user clicks Retry or presses R, and the thumbnails' ledger write fails (for example the catalog is locked past the busy retries, or any non-busy error).
- Consequence: The writer logs one warning, rereads the inbox and republishes it, so the entry and its Retry button stay as they were. There is no toast or message, so the click looks like it did nothing, and the user can't tell a failed retry from one still in progress.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\inbox.rs:922`; `D:\GitHub\photo-gallery\crates\app\src\inbox.rs:1563`; `D:\GitHub\photo-gallery\crates\desktop\src\notices.rs:549`

### UI-245 [low/unhandled-close] A Retry queued at close is dropped without a user-visible trace — CONFIRMED

- Status: Partial (validated). The replayed Retry wakes the job, but the Retry is not persisted before ack and is lost if its queue insert is abandoned.
- Needs: Fix. Durable queue, as UI-242: a queued Retry is persisted before it is acknowledged. Files: crates/app/src/inbox.rs.
- Verdict: CONFIRMED
- Note: inbox.rs:932-943 drops the rest of the queue, including Step Retry with its FnOnce, when the writer is Abandoned. The job is not woken, and only a warning is logged. The Retry variant is at inbox.rs:210.
- Confidence: medium
- Trigger: The user clicks Retry, then the app closes while the catalog is still busy after the 5 s shutdown drain (Inbox::close, inbox.rs:547-555), so the writer abandons the rest of its work.
- Consequence: The Retry's closure, which would have woken the job, is dropped unexecuted. Only a log line records that the user's change wasn't written. The thumbnail rows stay, and the user sees the same entry with Retry on the next launch.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\inbox.rs:932`; `D:\GitHub\photo-gallery\crates\app\src\inbox.rs:210`

### UI-246 [low/stale-result] Thumbnail failures buffered before a Retry can be recorded again after it — PARTIAL

- Status: Done (validated). Fixed by a failure_retries counter, not the ordering in the Needs bullet.
- Needs: Fix (checked: confirmed; impact narrow, the number of buffered failures is not measured). Order the pass's ledger writes through the writer queue, or have record_failures check the listing sequence, so a late flush cannot undo a Retry. Files: crates/thumbs/src/lib.rs, crates/catalog/src/writers.rs, crates/catalog/src/ledger/failure.rs.
- Verdict: PARTIAL
- Note: thumbs lib.rs:329-330 writes the buffered failures on its own connection, and ledger.rs:209 deletes the rows of the pass. The finding says the ordering against Retry was not traced, and that remains open.
- Confidence: low
- Trigger: The user presses Retry while the thumbnails pass is running and has buffered failures that have not yet been flushed (up to 2 s, LEDGER_EVERY). The pass writes through its own connection, not the writer's queue, so its flush can land after the writer's Retry delete.
- Consequence: Those files' failures come back after the Retry as new, unread entries, with a toast. The user sees the Retry not clear them. This was not verified end to end: the pass's buffer contents and the ordering against the Retry were not traced in full.
- Sites: `D:\GitHub\photo-gallery\crates\thumbs\src\lib.rs:329`; `D:\GitHub\photo-gallery\crates\catalog\src\ledger.rs:209`

## U63: Inbox failed-file list

Entry: `crates/desktop/qml/InboxPanel.qml:75 requestFiles, from the ledger rows (InboxPanel.qml:64 show); Main.qml:838 onFilesRequested -> failedFilesModel.show(pass)`

### UI-247 [medium/stale-result] Open list not re-read when a non-newest failed file is cleared — CONFIRMED

- Status: Done (validated). The re-read gate is in notices.rs, keyed on serial and count.
- Needs: Fix. The listing key includes the file count or a revision, not only the newest serial. Files: crates/desktop/qml/InboxPanel.qml, crates/desktop/src/failed_files.rs, crates/catalog/src/inbox.rs.
- Verdict: CONFIRMED
- Note: InboxPanel.qml:78-82 returns early when the key and serial match, and onSerialChanged alone re-tells (InboxPanel.qml:276). clear_failures (ledger.rs:188-) removes a non-newest file without changing the newest serial, so the list is not re-read. The body of tell() was not read, so whether it calls requestFiles() is not settled, but requestFiles() itself would return early.
- Confidence: medium
- Trigger: A pass's ledger entry (for example the thumbnails) is open with its details showing. A file in the list that is not the newest gets its thumbnail made (Inbox::thumb_made, then clear_failures removes its row) with no new failure recorded, so the entry's newest serial does not change.
- Consequence: The list keeps showing the cleared file as failed, and the header line 'The newest N of M files' keeps the old total, until a new failure moves the newest serial or the user reselects the entry. The count text in the details updates through body, but the list does not, so the user sees a stale failed-files list.
- Sites: `crates/desktop/qml/InboxPanel.qml:78`; `crates/desktop/qml/InboxPanel.qml:276`; `crates/catalog/src/inbox.rs:231`; `crates/catalog/src/ledger.rs:188`; `crates/desktop/src/failed_files.rs:129`

## U64: Reveal a file from the inbox

Entry: `crates/desktop/qml/InboxPanel.qml:448-453 Reveal onClicked (file, not folder) -> revealRequested (InboxPanel.qml:59) -> Main.qml:839 win.revealFile (Main.qml:351)`

### UI-251 [medium/missing-feedback] File-gone toast is dimmed behind the open inbox — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Show the file-gone message inside the inbox panel, through the shared outcome path. Files: crates/desktop/qml/Main.qml, crates/desktop/src/shell.rs, crates/desktop/qml/InboxPanel.qml.
- Verdict: CONFIRMED
- Note: shell.rs:353 posts FileNoLongerInLibrary as an info toast, and StatusBar sits under the inbox, which fills the window (Main.qml:819-822 anchors.fill: parent) with the ModalOverlay scrim (ModalOverlay.qml:26, #b0000000). The toast is dimmed as the finding says.
- Confidence: medium
- Trigger: The user clicks Reveal on a file the library no longer has (present_file_folder returns None). Reveal does not close the inbox, so the inbox stays up.
- Consequence: The notice is a transient info toast in the status bar. The status bar sits under the inbox's dim scrim (#b0000000), and the toast clears itself after 6 s. The panel does not change and no message is shown inside it, so the user sees a dimmed line at the bottom or nothing.
- Sites: `crates/desktop/src/shell.rs:353`; `crates/desktop/qml/ModalOverlay.qml:26`; `crates/desktop/qml/Main.qml:820`

### UI-252 [low/wrong-message] A failed listing read is reported as a missing file — PARTIAL

- Status: Done (validated).
- Needs: Fix (checked: partly; shell.rs:447-451 is the Ok(None) branch, which this trigger does not reach). At shell.rs:560-566, check whether the listing failed before posting FileNoLongerInLibrary after an empty listing. Files: crates/desktop/src/shell.rs, crates/desktop/src/gallery/model.rs.
- Verdict: PARTIAL
- Note: model.rs:100-104 turns a listing error into ListingFailed and an empty list (unwrap_or_default), and applies it with the generation. The reveal branch that then posts FileNoLongerInLibrary (shell.rs:447-451 and the code after it) was not read, so the second notice is not verified.
- Confidence: high
- Trigger: The user clicks Reveal, the folder is found, but the grid's listing read for it fails (a catalog read error). open_listing's observe posts ListingFailed and the list is empty.
- Consequence: The empty list is applied and reports count 0 with the reveal's generation. The reveal then treats the file as absent and posts FileNoLongerInLibrary. The user gets two notices, one claiming the file is gone, though it is still in the library. The real cause is the listing failure.
- Sites: `crates/desktop/src/gallery/model.rs:100`; `crates/desktop/src/gallery/model.rs:104`; `crates/desktop/src/shell.rs:451`

### UI-253 [low/missing-feedback] Reveal shows no busy state while its folder is read — CONFIRMED

- Status: Done (validated).
- Needs: Fix. A busy state on the Reveal button while the folder is read. Files: crates/desktop/qml/InboxPanel.qml.
- Verdict: CONFIRMED
- Note: InboxPanel.qml:448-453 has no busy state, and the button stays enabled while the folder is read. Low severity. The notices.rs remark about the reader being busy was not read.
- Confidence: medium
- Trigger: The user clicks Reveal on a file while the reader is busy (for example behind a running grid listing, which notices.rs says is accepted). The read can take a noticeable time.
- Consequence: The Reveal button stays enabled with no label change or spinner, and the inbox stays open until the folder is listed. A second click resubmits the request, which supersedes the first safely, but the user has no sign the first click registered.
- Sites: `crates/desktop/qml/InboxPanel.qml:448`; `crates/desktop/qml/InboxPanel.qml:452`

## U65: Reveal a folder from the inbox

Entry: `crates/desktop/qml/InboxPanel.qml:448-453 Reveal onClicked (folder) -> folderRevealRequested (InboxPanel.qml:60) -> Main.qml:840 shell.revealFolder(row)`

### UI-257 [low/missing-feedback] Reveal of a refused library root lands on a folder with nothing in it, with no message — CONFIRMED

- Status: Done (validated). Caveat: a root whose listing failed gets the same message as a refused root.
- Needs: Fix. A reveal that lands on a refused root reports the refusal through the shared outcome path. Files: crates/desktop/src/shell.rs, crates/catalog/src/roots.rs, crates/client/locales/en/text.ftl.
- Verdict: CONFIRMED
- Note: shell.rs:363 takes the parent of the failed folder with split_file_name, or the folder itself when there is no parent. The refused-root row is written by roots.rs:241-245. A drive root therefore shows itself and a folder under a drive shows a parent outside the library, with no message.
- Confidence: medium
- Trigger: The failed folder is a library root itself (folder_refused writes the root as the failed path, e.g. a refused FAT or non-NTFS drive). Reveal then takes its parent: for a drive root, which has no parent, it shows the failed folder itself. For a folder under a drive, it shows a parent outside the library.
- Consequence: The inbox closes, the grid is empty, and nothing explains why. It looks like the folder is empty rather than refused.
- Sites: `crates/desktop/src/shell.rs:363`; `crates/catalog/src/roots.rs:241`

## U69: Log view (dev mode)

Entry: `crates/desktop/qml/StatusBar.qml:124 onClicked logRequested -> Main.qml:561 shell.globalAction(Log); shortcut GlobalShortcuts.qml:30-34; Main.qml:444-447 openPanel -> logModel.start (log_model_bridge.rs:73)`

### UI-269 [low/wrong-message] Translator comment for the log strings is attached to the inbox key — CONFIRMED

- Status: Already fixed (validated). The comment sits above ui-log at ui.ftl:115-116. Candidate for removal, pending your call.
- Needs: Fix. Move the log comment above ui-log in ui.ftl. Files: crates/client/locales/en/ui.ftl.
- Verdict: CONFIRMED
- Note: ui.ftl:91 holds the log comment, and the next key is ui-inbox (ui.ftl:93). ui-log, ui-log-title and ui-log-empty (ui.ftl:116-118) have no comment of their own.
- Confidence: high
- Trigger: Anyone reading or translating crates/client/locales/en/ui.ftl.
- Consequence: The comment that describes ui-log, ui-log-title and ui-log-empty sits above ui-inbox, and ui-log has no comment of its own, so translators see the log strings described as the inbox's. No runtime text is wrong.
- Sites: `D:/GitHub/photo-gallery/crates/client/locales/en/ui.ftl:91`; `D:/GitHub/photo-gallery/crates/client/locales/en/ui.ftl:116`

## U74: Grid thumbnails load and cells display

Entry: `crates/desktop/qml/GridRow.qml:103 Component.onCompleted take and :106 onCountChanged take; GridRow.qml:78 take()`

### UI-279 [medium/stuck-error] A transient thumbnail failure leaves the error mark on a readable photo — PARTIAL

- Status: Partial (build and tests). The QML retry is unbounded with a 15 s cap, and Qt re-requests on a device pixel ratio change (read in its source). Not yet checked: that the Rust side does not cache a failure as final.
- Needs: Fix (checked: confirmed; an unchanged source is not re-requested after Error, qquickimagebase.cpp:119-123). Stop caching a transient failure as final (providers.rs:107-135), and re-request when the cell's URL is re-bound or the device pixel ratio changes (qquickimagebase.cpp:426-445). Files: crates/desktop/qml/GridCell.qml, crates/desktop/src/providers.rs.
- Verdict: PARTIAL
- Note: GridCell.qml:68 binds the source to the thumbnail URL and :89 shows the error mark when thumb.status is Error. providers.rs:122-132 reports any error as failed(). Whether Qt's Image re-requests an unchanged source after an Error was not read in qquickimage.cpp, and that decides the finding.
- Confidence: medium
- Trigger: A photo's original cannot be read when its cell first loads (an offline or removable drive, or a file held by another process). The provider returns failed(), so the cell's thumb Image goes to Error and shows the red mark. The drive or file becomes readable again, but the row's thumbnail URL (the image key and the thumbnail height) does not change, so the Image never requests again. The mark stays until the cells are recreated, by a thumbnail-size step, a relayout, a list reset, or the row being given different files.
- Consequence: A photo that can now be read shows the failure mark, as if it could not be decoded, until the user changes the size or reloads the list.
- Sites: `crates/desktop/qml/GridCell.qml:68`; `crates/desktop/qml/GridCell.qml:89`; `crates/desktop/src/providers.rs:122`; `crates/desktop/src/providers.rs:132`; `crates/desktop/qml/GridRow.qml:84`

### UI-280 [medium/wrong-message] A failed grid listing shows the empty-folder message and clears the photos — CONFIRMED

- Status: Done (build and tests). Rows are kept on a failed listing, and ui-listing-failed is defined.
- Needs: Fix. A failed listing keeps the photos and shows the error, instead of "No images here". Same fix as UI-004. Files: crates/desktop/src/gallery/model.rs, crates/desktop/qml/GalleryGrid.qml.
- Verdict: CONFIRMED
- Note: model.rs:100-106 applies an empty list when the read fails (unwrap_or_default), and GalleryGrid.qml:96-99 shows "No images here" (or the filter or import text) whenever the count is 0 and not loading.
- Confidence: high
- Trigger: The grid's catalog read fails while a folder is open or being refreshed (the catalog cannot be opened, or the read errors). This happens during a refresh of the same folder after a scan, with the folder's photos already shown.
- Consequence: The grid replaces its rows with an empty list and shows 'No images here.' (or 'Nothing here matches the filters.' while a filter is on). That is a false statement about the folder, and the photos disappear until a later listing succeeds. The only other sign is the separate ListingFailed notice.
- Sites: `crates/desktop/src/gallery/model.rs:104`; `crates/desktop/src/app.rs:30`; `crates/desktop/src/gallery/model.rs:133`; `crates/desktop/qml/GalleryGrid.qml:96`; `crates/desktop/qml/GalleryGrid.qml:99`; `crates/client/locales/en/ui.ftl:53`

### UI-281 [low/visual-flicker] retainWhileLoading is undone by the fade, so a changed thumbnail flashes to the placeholder — CONFIRMED

- Status: Done (validated).
- Needs: Fix. Keep the previous thumbnail visible while the new one loads; fade only once it is ready. Files: crates/desktop/qml/GridCell.qml.
- Verdict: CONFIRMED
- Note: GridCell.qml:70 sets retainWhileLoading. GridCell.qml:74 sets fade to 0 unless the status is Ready, so a new URL makes the status Loading and the retained image fades out through the Behavior at :75. GridCell.qml:58 then shows the blurred ThumbHash until the new image is Ready.
- Confidence: medium
- Trigger: A cell's thumbnail URL changes while the same file is shown: a thumbnail-height step (Ctrl+wheel calls shell.sizeStepped, which changes the thumb height and so every visible row's URL), a turn, or a rewrite (which changes the image key).
- Consequence: The cell's Image is set to Loading, so fade drops to 0 and the retained previous thumbnail fades out. The blurred ThumbHash shows until the new image is Ready, instead of the old thumbnail staying visible. Each size step flashes the whole visible grid.
- Sites: `crates/desktop/qml/GridCell.qml:70`; `crates/desktop/qml/GridCell.qml:74`; `crates/desktop/qml/GridCell.qml:81`; `crates/desktop/qml/GridCell.qml:58`
