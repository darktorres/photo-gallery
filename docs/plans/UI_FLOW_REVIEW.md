# UI and QML flow review

This report lists the findings of a flow-based review of the desktop UI: the QML front end, the Rust bridges and the controller it drives. The review started from each user action and each path that ends in a write, a notice or a focus change, and traced it through the code it reaches. It was run as a read-only workflow that produced 281 findings across 74 flows.

**Status: validated.** Each finding was checked against the live code at the cited sites, and against the Qt source under `C:\Qt\6.12.0\Src` where a finding depends on Qt behaviour. Verdicts: 248 confirmed, 33 partial, 0 refuted. A confirmed verdict means the mechanism is in the cited code. Where a finding also claims a consequence that was not traced, the note says so. Duplicates are marked and point to the finding they repeat.

## Method

1. A mapping step listed the user flows and the cited sites on each.
2. Each flow was traced from its entry point through the code it reaches, and each finding records a trigger, a consequence and the sites it cites.
3. Each finding was then validated: the cited lines were read in context (about four lines either side), and the claim was traced into the callees and into Qt where needed. Findings that cite lines past the end of the file are recorded as citation errors below; their mechanism was checked only where the code could be found elsewhere.

## Coverage

| Measure | Value |
|---|---:|
| Flows traced | 74 |
| Findings | 281 |
| Sites recorded as not flows | 18 |

Sites recorded as not flows were not traced; the mapping output gives a reason for each one.

## Summary

| Severity | Count |
|---|---:|
| high | 11 |
| medium | 85 |
| low | 185 |

| Category | Count |
|---|---:|
| stale-state | 60 |
| ui-logic | 60 |
| error-display | 59 |
| input | 44 |
| lifecycle | 16 |
| text | 16 |
| race | 15 |
| focus | 7 |
| missing-feedback | 2 |
| null-read | 1 |
| stuck-busy | 1 |

| Verdict | Count |
|---|---:|
| CONFIRMED | 248 |
| PARTIAL | 33 |
| REFUTED | 0 |

Duplicates: 49 findings repeat a mechanism already recorded under another ID and are marked as such.

## Citation errors

These findings cite a line past the end of the file. The rest of each finding was checked where the code could be found.

| Finding | Cited site |
|---|---|
| UI-076 | `crates/desktop/src/filters.rs:270` |
| UI-118 | `crates/desktop/qml/StatusBar.qml:564` |
| UI-180 | `crates/desktop/src/animation.rs:76` |
| UI-180 | `crates/desktop/src/animation.rs:112` |
| UI-206 | `crates/desktop/src/yandex_bridge.rs:449` |
| UI-206 | `crates/desktop/src/yandex_bridge.rs:453` |
| UI-206 | `crates/desktop/src/yandex_bridge.rs:455` |
| UI-207 | `crates/desktop/src/yandex_bridge.rs:464` |

## Flows

| ID | Name | Findings |
|---|---|---:|
| U01 | Startup and first list | 5 |
| U02 | Start gate: test hooks, library start, first scan refresh | 5 |
| U03 | Window close flushes the session | 2 |
| U04 | Command-line test hooks | 9 |
| U05 | Screenshot settle hook | 3 |
| U06 | Crash box on panic | 5 |
| U07 | Live reload of QML (development) | 4 |
| U08 | Grid keyboard keys (through the shell) | 4 |
| U09 | Global shortcuts (inbox, log, dev mode) | 1 |
| U10 | Select a photo by click | 2 |
| U11 | Open the viewer by double-click | 2 |
| U12 | Thumbnail size: Ctrl+wheel, slider, keys | 4 |
| U13 | Grid relayout and scroll position after a resize | 2 |
| U14 | Grid scrollbar drag | 3 |
| U15 | Folder tree: expand and open a folder | 3 |
| U16 | Include subfolders toggle | 3 |
| U17 | Sort box and descending toggle | 3 |
| U18 | Apply a saved view | 5 |
| U19 | Save the current view (dialog) | 2 |
| U20 | Delete a saved view | 3 |
| U21 | Filter bar open and close, facet counts | 4 |
| U22 | Filter facet toggles | 4 |
| U23 | Filter bounds and name field | 3 |
| U24 | Duplicates and border-marked choosers | 4 |
| U25 | Clear all filters | 3 |
| U26 | Open the library panel | 7 |
| U27 | Add a folder through the picker | 4 |
| U28 | Remove a folder (with confirmation) | 5 |
| U29 | Discard a pending removal | 5 |
| U30 | Rescan all folders | 3 |
| U31 | Library settings toggles | 2 |
| U32 | Scan lanes: pause a lane, open its failed count | 3 |
| U33 | Lane progress polling | 2 |
| U34 | Folder picker dialog navigation | 4 |
| U35 | Save cropped copies (dialog) | 6 |
| U36 | Cut borders from originals (dialog) | 5 |
| U37 | Popups for refused border actions | 4 |
| U38 | Rate a photo in the viewer (star bar, keys) | 4 |
| U39 | Turn a photo in the viewer | 5 |
| U40 | Mark or unmark a border (viewer N key, HUD) | 5 |
| U41 | Show or hide borders (viewer B key) | 3 |
| U42 | Step through pictures in the viewer (wheel, keys, swipe) | 3 |
| U43 | Zoom, pan, fit and 1:1 in the viewer | 5 |
| U44 | Viewer re-render: fit size and region tiles | 5 |
| U45 | Loupe (10x under the cursor) | 4 |
| U46 | Close the viewer | 2 |
| U47 | Video and animation transport | 7 |
| U48 | Find similar from the viewer | 6 |
| U49 | Yandex search results load | 6 |
| U50 | Pick Yandex results, shift-range pick | 4 |
| U51 | Select all of a Yandex group | 3 |
| U52 | Yandex full-size preview | 4 |
| U53 | Save picked Yandex results (dialog) | 5 |
| U54 | Close the Yandex panel and its preview | 3 |
| U55 | Yandex thumbnails settle | 3 |
| U56 | Mascot click | 1 |
| U57 | Status bar toast | 4 |
| U58 | Open the inbox (status bell, shortcut) | 4 |
| U59 | Inbox select and details | 5 |
| U60 | Copy an inbox entry | 7 |
| U61 | Dismiss an inbox entry, clear all | 6 |
| U62 | Retry an inbox entry | 4 |
| U63 | Inbox failed-file list | 3 |
| U64 | Reveal a file from the inbox | 5 |
| U65 | Reveal a folder from the inbox | 4 |
| U66 | Show failed imports | 2 |
| U67 | Inbox read state and unread count | 2 |
| U68 | Close the inbox | 1 |
| U69 | Log view (dev mode) | 6 |
| U70 | Dismiss any modal panel (Escape or background click) | 1 |
| U71 | Preferences (settings) read and saved | 1 |
| U72 | Controller deadlines and pauses | 3 |
| U73 | Command-line find similar | 4 |
| U74 | Grid thumbnails load and cells display | 3 |

## U01: Startup and first list

Entry: `crates/desktop/qml/Main.qml:860 (Component.onCompleted: setScale, sortIndex, relayout, folderModel.reload, shell.reloadList)`

### UI-001 [medium/missing-scroll] Restored selection is not scrolled into view at startup — CONFIRMED

- Verdict: CONFIRMED
- Note: Command::RestoreSelection (shell.rs:456) only sets the current item and does not emit select_requested. Main.qml:264-270 positions the grid from `current` before the restore runs (the restore is posted to the gallery lane), so it falls to positionAtBeginning. The shell.rs comment at 432 assumes the placement that happens before the restore.
- Confidence: high
- Trigger: Launch with a library larger than one screen, where the session's last file (prefs last_file) is far below the top row. The list is shown with no selection, and the restore happens afterwards.
- Consequence: The status bar shows the restored file's info and the file is highlighted, but the grid stays at the top, so the selected photo is off-screen. Arrow-key steps then move from an invisible item.
- Sites: `crates/desktop/src/shell.rs:456`; `crates/desktop/qml/Main.qml:264`; `crates/desktop/qml/Main.qml:270`; `crates/desktop/qml/Main.qml:141`; `crates/client/src/grid.rs:443`

### UI-002 [medium/stale-result] Startup folder fallback overwrites the saved session, including on a failed tree read — CONFIRMED

- Verdict: CONFIRMED
- Note: folders.rs:184 falls back to an empty tree on read error. Main.qml:292 sends treeLoaded(false), start.rs:59 emits OpenAll, and listing::update returns SaveSession unconditionally (listing.rs:77). listing_event maps it at once (controller.rs:495), with no started gate, so the start options are saved too.
- Confidence: high
- Trigger: The session folder is a specific folder, and at startup (or after a scan) the tree does not contain it, or the tree read fails (folders.rs falls back to an empty tree and posts TreeFailed). The gate then sends OpenAll.
- Consequence: The remembered folder and the recursion setting are replaced by All folders and saved at once, even though only a read failed. Start options such as --query/--sort are saved into the session too. The save is not deferred until the start is over, unlike the timer saves, so the stored session no longer matches what the user last had. A transient read error permanently changes the user's remembered view.
- Sites: `crates/desktop/src/folders.rs:184`; `crates/client/src/controller/start.rs:59`; `crates/client/src/controller.rs:495`; `crates/client/src/controller.rs:532`; `crates/desktop/src/shell.rs:513`

### UI-003 [low/wrong-message] Empty-state text says no folders until the library starts — CONFIRMED

- Verdict: CONFIRMED
- Note: library.roots is empty until library.start() runs load_roots (library.rs:152, called from Main.qml:879 in startHooks). noFolders is therefore true before the start, and GalleryGrid.qml:97 shows the no-folders text whenever the list is empty.
- Confidence: medium
- Trigger: Launch with library folders where the first list is empty (a session folder with no files, or a session filter that matches nothing) and the folder tree is not yet in. The window shows the empty-state label before startHooks has loaded the roots.
- Consequence: A transient 'No folders in the library' message is shown for a library that has folders, until the tree arrives and the start completes. It then switches to the correct text.
- Sites: `crates/desktop/qml/Main.qml:628`; `crates/desktop/qml/GalleryGrid.qml:96`; `crates/desktop/src/library.rs:152`; `crates/desktop/qml/Main.qml:879`

### UI-004 [low/wrong-message] A failed first listing shows the empty-folder message — CONFIRMED

- Verdict: CONFIRMED
- Note: gallery/model.rs:104 applies an empty list on a failed read (unwrap_or_default) after observe posts ListingFailed. GalleryGrid.qml:97 then shows the empty-state text while the toast reports the error.
- Confidence: medium
- Trigger: The listing read fails at startup or on a folder change (catalog unopened or query error). open_listing applies an empty list.
- Consequence: The grid shows the empty-state text ('No images' or 'No folders') as if the folder were empty. Only the toast reports the failure.
- Sites: `crates/desktop/src/gallery/model.rs:104`; `crates/desktop/qml/GalleryGrid.qml:97`

### UI-005 [low/stale-result] A spelling correction at startup saves the start options as the session — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml tree_loaded (shell.rs:130) dispatches FolderSpelled before TreeLoaded. controller.rs:313-315 saves self.listing at once, without the is_started gate (the gate applies only to the timer save at 532). The listing holds the start's query and sort (Controller::new takes the Start listing, controller.rs:215).
- Confidence: medium
- Trigger: The session folder is stored under a different spelling from the tree (for example a case change on disk) and the start has a --query or --sort option. Tree arrival dispatches FolderSpelled before the start is over.
- Consequence: The listing with the start's query and sort is saved as the session before the start is over, contrary to the rule that start options are not saved. The next launch then restores them without the user choosing them.
- Sites: `crates/client/src/controller.rs:313`; `crates/client/src/controller.rs:315`; `crates/desktop/src/shell.rs:130`

## U02: Start gate: test hooks, library start, first scan refresh

Entry: `crates/desktop/qml/Main.qml:876 startHooks (library.start at Main.qml:879)`

### UI-006 [medium/wrong-result-on-failure] Transient tree read error during scan reload moves the listed folder to All and overwrites the session — CONFIRMED

- Verdict: CONFIRMED
- Note: Each tree reload calls folderModel.onLoaded -> treeLoaded (Main.qml:84, 280-292). A tree read error gives an empty tree, so row is -2 and TreeLoaded{false} reaches the gate, which emits OpenAll after the start (start.rs:59 has no started check). The save follows through listing_event (controller.rs:495).
- Confidence: high
- Trigger: The user is viewing one folder (not All). A scan finishes (Library.scanned), so reloadAll runs folderModel.reload(). The catalog's folder-tree read fails once (I/O or busy error). The tree is replaced with an empty one and TreeFailed is posted. The listed folder is not in the empty tree, so the start gate receives TreeLoaded{folder_known:false} after the start is over and emits OpenAll.
- Consequence: The grid silently switches from the user's folder to All folders, the tree collapses to its single All row, and SaveSession overwrites the remembered folder. The user has to re-open the folder by hand. The only signal is a TreeFailed notice in the inbox. The folder is not restored when the next tree read succeeds.
- Sites: `crates/desktop/src/folders.rs:184`; `crates/desktop/src/folders.rs:246`; `crates/desktop/qml/Main.qml:292`; `crates/client/src/controller/start.rs:59`; `crates/client/src/controller/start.rs:63`; `crates/client/src/controller.rs:357`; `crates/client/src/controller/listing.rs:54`

### UI-007 [low/wrong-message] No-folders text stays after a failed roots read until the next scan, add or remove — CONFIRMED

- Verdict: CONFIRMED
- Note: load_roots runs on start (library.rs:152), on taken.scanned (:202), on removal (:143, :276, :281) and on add (:258). Nothing on the grid refresh path re-reads roots, so an empty roots list after a failed read stays until one of those runs.
- Confidence: high
- Trigger: Library.load_roots gets an Err from the catalog (during start or a scan pass). roots is set to an empty list, so Main passes noFolders=true to the grid. The catalog read then recovers, but nothing re-reads roots on the grid refresh path. Only scanned, add, remove or start call load_roots again.
- Consequence: While the open folder is empty, the grid shows 'no library folders' even though folders exist. The inbox does show FoldersUnreadable, but the grid text itself is wrong until the next scan or add.
- Sites: `crates/desktop/src/library.rs:89`; `crates/desktop/src/library.rs:91`; `crates/desktop/qml/Main.qml:628`; `crates/desktop/qml/GalleryGrid.qml:97`

### UI-008 [low/missed-change] Changes published between the job's start and the bus subscription are never delivered — PARTIAL

- Verdict: PARTIAL
- Note: The subscribe-after-start order is real (library.rs:162, app library.rs:111-113). But nothing in the start path requests a scan or import before the subscription: rescan, add and remove all come after the window exists (library.rs:288, 370). The only remaining source is the runner or writer resuming work at start, which I did not trace. So this is latent at most.
- Confidence: medium
- Trigger: Library::start (app) spawns the lane coordinator and the writer, which can publish Scanned or Imported (library shared.rs 182 and 188) before the caller subscribes at desktop library.rs:162. The Bus delivers only to subscribers present at publish time and keeps no history. A job that finishes its only pass in that window never tells the grid, and nothing in the initial refresh (refresh_progress, refresh_writes) re-reads scan or import state.
- Consequence: Imported or scanned files from the very first pass are not shown until some later change or a manual rescan. The window is narrow (only the writer start and a few calls), so this is latent. The comment at library.rs:161 does not cover this gap.
- Sites: `crates/desktop/src/library.rs:162`; `crates/app/src/library.rs:111`; `crates/jobs/src/events.rs:68`

### UI-009 [low/missing-feedback] Change pump failure to start is logged only; the grid stops refreshing silently — CONFIRMED

- Verdict: CONFIRMED
- Note: pump::start's error is only logged (library.rs:171-172). The change pump then never delivers Scanned, Imported, Progress or Writes, and no notice or status text reports it.
- Confidence: high
- Trigger: pump::start fails to spawn its thread (for example the OS is out of threads) at window start. The library runs, but no Scanned, Imported, Progress or Writes change reaches the window, and the failure is written only to the log.
- Consequence: Imports and scans happen with no refresh in the grid, the lane rows or the status line, and the user is not told. Nothing in the inbox or the status bar says so.
- Sites: `crates/desktop/src/library.rs:171`; `crates/desktop/src/library.rs:172`

### UI-010 [low/missing-feedback] Rescan and add-folder give no feedback when the library job never started — CONFIRMED

- Verdict: CONFIRMED
- Note: rescan returns Ok when running() is None (app library.rs:428-435), and the desktop rescan reports nothing on success (desktop library.rs:288-292). add_path posts nothing for a new folder when the job is down (desktop library.rs:253-258), and add_folder does not scan without a running job (app library.rs:377).
- Confidence: high
- Trigger: Library start fails at startup (LibraryUnavailable is posted once). The user then clicks Rescan in the Library panel, or adds a folder. Both return success without scanning anything.
- Consequence: Rescan does nothing and shows nothing. Add-folder saves the folder and refreshes the roots, but no scan is queued and no notice is posted, so the folder shows no files until restart, with no message at the point of the action.
- Sites: `crates/app/src/library.rs:431`; `crates/app/src/library.rs:377`; `crates/desktop/src/library.rs:289`; `crates/desktop/src/library.rs:157`

## U03: Window close flushes the session

Entry: `crates/desktop/qml/Main.qml:142 (onClosing: shell.flush())`

### UI-011 [medium/unhandled-close] Closing mid-save drops the unwritten Yandex results silently — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:142 flush does not touch Yandex. App::stop only logs (app.rs:156-158). save_all's loop (yandex.rs:372-376) does not check `closed`, and the `closed` checks at 425, 456 and 539 are in other paths. Hits left unwritten are lost without a shown outcome. save_one was not read.
- Confidence: medium
- Trigger: The user starts saving several Yandex results, then closes the window before the save ends (or within the 2 s stop wait). Main.qml:142 flush does not touch Yandex.
- Consequence: The window's exit calls Search::stop (yandex.rs:498-515, which sets closed and waits 2 s at STOP_WAIT yandex.rs:102). save_all (yandex.rs:372) never checks closed, so it keeps writing until the process ends. Hits not yet written are never saved, and the 'saved N, failed M' outcome (yandex.rs:407-411) is never recorded or shown. App::stop only logs a warning (app.rs:156-157). The user gets no sign that the save was incomplete.
- Sites: `crates/desktop/qml/Main.qml:142`; `crates/app/src/yandex.rs:372`; `crates/app/src/yandex.rs:498`; `crates/app/src/yandex.rs:499`; `crates/app/src/app.rs:156`

### UI-012 [low/stale-result] Last file saved at close can be the previous selection — CONFIRMED

- Verdict: CONFIRMED
- Note: SaveLastFile reads gallery::shown_file() on the window thread (shell.rs:96). SHOWN is set only by publish_shown from the model's queued closures (gallery.rs:58, model.rs:290, 298), so a selection still in the queue at close is not what is saved. Latent, needs a close inside the queue latency.
- Confidence: low
- Trigger: A selection key is pressed (shell.rs:433-439 posts set_current_item to the model lane), and the window closes before that queued closure runs. The flush reads the shown file synchronously.
- Consequence: SaveLastFile (shell.rs:95-97) reads SHOWN, which is updated only by publish_shown in queued model closures (gallery.rs:57-58, gallery/model.rs:290,298). The next start then selects the previous file rather than the one the user last selected. The effect is latent and needs a close within the queue latency of a selection.
- Sites: `crates/desktop/src/shell.rs:96`; `crates/desktop/src/shell.rs:435`; `crates/desktop/src/gallery.rs:58`

## U04: Command-line test hooks

Entry: `crates/desktop/qml/TestHooks.qml:83 run() and :121 runAfterLibraryStart()`

### UI-013 [medium/wrong-message] --yandex-preview opens a result that does not exist after a failed or empty search — CONFIRMED

- Verdict: CONFIRMED
- Note: app yandex.rs:468 clears busy with no results when the search fails or stops. YandexPanel.qml:59-60 then opens the preview at previewWhenFound on any busy change, with no check of the result count. YandexPreview.qml:51-52 opens the index without a bounds check.
- Confidence: high
- Trigger: Command line --find-similar=`<photo>` --yandex-preview=0 (or N) where the search ends by SearchFailed or Stopped, or with fewer than N+1 results. The thread clears busy (app/src/yandex.rs:468) with no results, and YandexPanel's onBusyChanged opens the preview on the unchecked index.
- Consequence: A full-size preview overlay opens with a blank picture and the caption for an item that is not there, reading 'N+1 of 0'. It looks like a result, so a scripted screenshot or check records a wrong result as correct. The search's own error status sits behind the overlay, so no failure message is shown.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:59`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:60`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPreview.qml:51`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPreview.qml:52`; `D:/GitHub/photo-gallery/crates/app/src/yandex.rs:468`

### UI-014 [low/gate-bypass] B and N test hooks skip the border-detection gate the user's keys apply — CONFIRMED

- Verdict: CONFIRMED
- Note: TestHooks.qml:97-98 and Main.qml:476-478 call shell.toggleBorders and shell.toggleBorderMark directly. Neither checks the feature gate: toggle_borders dispatches Panel(ToggleBorders) (shell.rs:218), toggle_border_mark dispatches ToggleBorderMark (shell.rs:269). The user's keys go through grid_refusal first (shell.rs:161). The B hook saves show_borders through the same path (controller.rs:413).
- Confidence: high
- Trigger: Border detection turned off in the Library panel (prefs), then --viewer-keys=N on a shown file, or --viewer-keys=B.
- Consequence: N marks a file's borders, and B saves the show-borders preference as true (SaveShowBorders is written to prefs). The user's B and N are refused in this state with the 'borders are disabled' popup, so the hook writes state the UI says cannot change.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:476`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:478`; `D:/GitHub/photo-gallery/crates/desktop/src/shell.rs:161`; `D:/GitHub/photo-gallery/crates/client/src/controller.rs:413`; `D:/GitHub/photo-gallery/crates/desktop/src/shell.rs:91`

### UI-015 [low/wrong-message] The B test hook can only show borders, not toggle them — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:478 is `if (!win.showBorders) shell.toggleBorders()`, so the hook can only turn borders on. With borders shown it does nothing, unlike the user's B.
- Confidence: high
- Trigger: --borders=1 (or prefs with borders shown) together with --view=N --viewer-keys=B: the B hook does nothing, while the user's B hides the borders.
- Consequence: The hook's behaviour contradicts its own comment ('the toggle'). A scripted check expecting the borders hidden gets them shown or left alone.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:477`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:478`

### UI-016 [low/gate-bypass] --save-crops writes cropped copies past the filter bar's gates and gives no message when nothing qualifies — PARTIAL

- Verdict: PARTIAL
- Note: The no-message half holds: a zero count writes nothing and the picker's result is discarded (edits.rs:170-174, Main.qml:176). The write-while-off half was not confirmed: the hook path skips FilterBar.qml:274, but borders_found (edits.rs:194) only selects entries already marked Found, so copies are written with detection off only if those marks survive the toggle. I did not trace that.
- Confidence: high
- Trigger: --save-crops=`<folder>` while border detection is off, or when no listed file has borders found.
- Consequence: Cropped copies are written even though the user's 'Save cropped copies' button refuses (borders disabled), and with none to crop the picked folder is consumed with no message, since the count is discarded.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:486`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:176`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/edits.rs:171`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/edits.rs:172`; `D:/GitHub/photo-gallery/crates/desktop/qml/FilterBar.qml:274`

### UI-017 [low/null-read] --view takes any index, so the viewer can open on an item that is not in the list — CONFIRMED

- Verdict: CONFIRMED
- Note: TestHooks.qml:92 calls viewerShowRequested with any index, and Main.qml:651 sets galleryModel.viewing without a range check (gallery_bridge.rs:58, model.rs:110). The viewer is visible with index >= 0 (Viewer.qml:155) and its image key is empty for an out-of-range item (items.rs:41). shell.rs:153 hands the grid's keys to the viewer while it is open. viewer_step clamps (shell.rs:174, shell_bridge.rs:98).
- Confidence: high
- Trigger: --view=`<n>` with n >= the list's count, including any --view on an empty library (for example --view=50 on 10 files).
- Consequence: The viewer opens on no item: no image, and grid keys are routed to the viewer until Escape. A --viewer-keys=r3 then rates nothing and moves the viewer to the last item, because the step clamps.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:92`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:487`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:651`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:155`; `D:/GitHub/photo-gallery/crates/desktop/src/shell.rs:153`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/items.rs:41`

### UI-018 [low/missing-feedback] Panel hooks dropped silently by the modal rule and the dev-mode gate — CONFIRMED

- Verdict: CONFIRMED
- Note: controller.rs:427 returns no commands for any open modal that is not switchable (only inbox and log are), so --inbox after --library does nothing. controller.rs:432-433 ignores Global::Log without dev mode. Nothing reports either refusal.
- Confidence: high
- Trigger: --library=1 or --menu=views or --save-view=1 or --find-similar=`<photo>` combined with --inbox=1 or --log=1 (those run after the modal hooks), or --log=1 without dev mode.
- Consequence: The inbox or log view does not open and nothing says why, so a scripted run continues with the wrong panel showing.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:111`; `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:117`; `D:/GitHub/photo-gallery/crates/client/src/controller.rs:427`; `D:/GitHub/photo-gallery/crates/client/src/controller.rs:433`

### UI-019 [low/stale-state] --inbox-expand without --inbox persists into the user's first inbox open — CONFIRMED

- Verdict: CONFIRMED
- Note: TestHooks.qml:116 sets the expand pass. The reset is in the inbox open-change handler (Main.qml:64-69), and it runs only on a change to closed. The first user open is an open change, so the reset is skipped and expandRow (Main.qml:833) uses the stale pass.
- Confidence: high
- Trigger: --inbox-expand=`<ledger>` given without --inbox=1; the inbox is opened later by the user (Ctrl+Shift+N).
- Consequence: The hook's expand pass is still set when the user first opens the inbox, so that ledger's entry opens expanded. The reset runs only when the inbox closes.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:116`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:494`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:68`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:833`

### UI-020 [low/missing-feedback] --viewer-keys and --zoom without --view are silently ignored — CONFIRMED

- Verdict: CONFIRMED
- Note: The viewer keys and --zoom sit inside `if (commandLine.view >= 0)` (TestHooks.qml:92-109), so they do nothing without --view. command_line.rs:278-279 reports --yandex-preview without --find-similar as a bad option, but nothing reports these.
- Confidence: high
- Trigger: --viewer-keys=LB or --zoom=2 given without --view=N.
- Consequence: The scripted run proceeds with no viewer and no message. command_line.rs names --yandex-preview given without --find-similar as a bad option, so this case is inconsistent with the stated rule against silent hook no-ops.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:92`; `D:/GitHub/photo-gallery/crates/desktop/qml/TestHooks.qml:107`; `D:/GitHub/photo-gallery/crates/desktop/src/command_line.rs:278`; `D:/GitHub/photo-gallery/crates/desktop/src/command_line.rs:279`

### UI-021 [low/gate-bypass] The viewer draws borders while border detection is off — CONFIRMED

- Verdict: CONFIRMED
- Note: The grid uses `win.showBorders && win.bordersOn` (Main.qml:627). The viewer uses `win.showBorders` alone (Main.qml:649). Turning detection off does not clear showBorders, and BordersTurnedOff only changes the listing filters (listing.rs:70-75), not the entries' borders, so the viewer keeps drawing them (Viewer.qml:456).
- Confidence: medium
- Trigger: The user turns B on with detection on, then turns detection off in the Library panel, then opens a photo in the viewer.
- Consequence: The viewer draws the border frame and the cut overlay while the grid and the detection setting say borders are off. The grid's gate is missing from the viewer's showBorders.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:627`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:649`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:456`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:709`

## U05: Screenshot settle hook

Entry: `crates/desktop/src/lib.rs:214 screenshot::schedule(engine)`

### UI-022 [low/unhandled-close] Window closed during the settle wait drops the screenshot silently — CONFIRMED

- Verdict: CONFIRMED
- Note: lib.rs:216 exec returns when the window closes, and shutdown (lib.rs:221-224) has no screenshot step. The settle poll timer (screenshot.cpp:163-165) is a child of the engine, so nothing writes the PNG or the settle log. The exit status path was not traced.
- Confidence: high
- Trigger: A --screenshot run where the window is closed before it settles (within --wait seconds, default 10). Release builds keep Qt's default quitOnLastWindowClosed (the only setQuitOnLastWindowClosed(false) is in live_reload.cpp, debug-only).
- Consequence: exec returns, the app shuts down, and the poll timer (a child of the engine) is dropped with no take() call. No PNG and no .settle.txt are written, nothing is printed on stderr, and the process exits 0, so a harness that reads only the exit status sees success with no image.
- Sites: `crates/desktop/src/lib.rs:216`; `crates/desktop/src/lib.rs:223`; `crates/desktop/cpp/screenshot.cpp:165`

### UI-023 [low/stale-result] Settle check can pass before the library job's first progress is seen — PARTIAL

- Verdict: PARTIAL
- Note: The race needs a job that starts after the start. A plain start (Main.qml:879) requests no scan, so progress stays Idle and the settle check is right to pass. The window only sees a non-idle job when a scan is requested in the same run, for example by the add-folder hook (TestHooks.qml:122, which runs after library.start). The trigger as written (library folders alone) does not reach it.
- Confidence: medium
- Trigger: A --screenshot run with library folders where the window becomes ready and quiet for 300 ms before the lane coordinator thread publishes its first non-idle progress. Main.qml startHooks calls library.start() synchronously; Library::start sets busy from progress() right away, and progress() is Idle until a lane control begins on the coordinator's own thread.
- Consequence: busy.jobs stays 0 and idle() returns true, so the image is taken before the job has started, and the status-bar lane rows and the busy state are missing from the screenshot. The settle log reports 'library job running: 0' even though a job is about to start. This is test-harness only, and the window is short, so it is timing-dependent.
- Sites: `crates/desktop/cpp/screenshot.cpp:66`; `crates/desktop/src/library.rs:223`; `crates/desktop/src/library.rs:175`; `crates/library/src/runner.rs:173`; `crates/desktop/qml/Main.qml:879`

### UI-024 [low/wrong-message] A failed settle-log write also drops the image, with a message about the PNG path — CONFIRMED

- Verdict: CONFIRMED
- Note: screenshot.rs:50 writes the log with `?`, so an error returns before the PNG is written. screenshot.rs:40 then reports it as "can't save `<png path>`", naming the PNG and not the log.
- Confidence: medium
- Trigger: The PNG path is close to the file system's name-length limit, so that '`<png>`.settle.txt' is too long while '`<png>`' fits. An example is a name of about 250 bytes with a '.png' suffix.
- Consequence: The `?` on the log write returns before the image is written, so no PNG is saved. The only message is "screenshot: can't save `<png path>`: File name too long", which names the PNG and not the log file that failed. The doc comment's promise that the log goes first 'so it is there even when the image can't be written' does not cover the reverse case.
- Sites: `crates/desktop/src/screenshot.rs:50`; `crates/desktop/src/screenshot.rs:40`

## U06: Crash box on panic

Entry: `crates/desktop/src/panic_log.rs:17 install (panic hook) and :32 alert::error_box(shown)`

### UI-025 [high/dropped-work] Writes queued behind a panicking write are silently dropped — PARTIAL

- Verdict: PARTIAL
- Note: Writes queued while the panic box is up are dropped when the thread unwinds, which is the defect as described, and nothing reports it. The consequence overreaches, though: worker::run carries out any job whose send fails (worker.rs:47-48), so writes after the thread has died run inline on the Qt thread and are not lost.
- Confidence: medium
- Trigger: A write on the 'window writes' thread (rotate, rate, border mark, prefs save, session save) panics. The panic box blocks that thread, so the user keeps rating, turning, or marking while the box is up, and those writes queue behind it. Dismissing the box unwinds the thread, which drops the receiver and every queued closure without running it.
- Consequence: Ratings, turns, border marks, preference saves and session saves the user made after the panic are silently lost. No notice is posted, because the write closures that would report failure never run. Preferences stay changed on screen but are not saved, so they revert at the next start.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/worker.rs:28`; `D:/GitHub/photo-gallery/crates/desktop/src/worker.rs:29`; `D:/GitHub/photo-gallery/crates/desktop/src/worker.rs:42`; `D:/GitHub/photo-gallery/crates/desktop/src/settings.rs:20`

### UI-026 [medium/stuck-state] Border mark queue keeps a never-written target after the writes thread dies — CONFIRMED

- Verdict: CONFIRMED
- Note: A dropped border-mark write leaves its mark_writes entry (state.rs:170 reads its back). A toggle equal to that stale target returns early (edits.rs:137) with no write and no notice. This follows the dropped-write case in UI-025.
- Confidence: medium
- Trigger: A queued set_border_marked write is dropped with the writes thread (finding 1). Its entry stays in mark_writes, because border_mark_done never runs for it.
- Consequence: mark_target keeps returning the dropped, unwritten value for that file. A toggle that matches it is silently ignored (no write, no notice), so the item shows the old border state and the user cannot clear or set the mark with that click. The item stays in its previous state with no feedback until a different toggle clears the entry.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/gallery/edits.rs:137`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/edits.rs:140`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/state.rs:170`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/edits.rs:142`

### UI-027 [low/modal-block] Recovered panics on helper threads still show a box that blocks their thread, and startup waits for it — CONFIRMED

- Verdict: CONFIRMED
- Note: panic_log.rs:25 shows the box for any panic that is not caught, on any thread. The gpu device thread's panic therefore blocks there, and the main thread waits in join (lib.rs:202-205). The module doc (panic_log.rs:4-5) describes only process-ending panics. The join fallback to make_gpu_device is the recovery the finding names.
- Confidence: high
- Trigger: A panic on the 'gpu device' thread (not caught, and the app recovers through the join fallback) during startup. The hook shows the error box before the window is visible, and the main thread is waiting in join at lib.rs:202.
- Consequence: The user sees an 'internal error' box with no window behind it, and startup does not continue until the box is dismissed, even though the app would have recovered by making the device itself. The module doc (panic_log.rs:4-5) says only a process-ending panic shows the box, but the code shows it for every uncaught panic on any thread.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/panic_log.rs:25`; `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:202`; `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:205`; `D:/GitHub/photo-gallery/crates/desktop/src/panic_log.rs:4`

### UI-028 [low/missing-feedback] Panics before the hook is installed are neither logged nor shown — CONFIRMED

- Verdict: CONFIRMED
- Note: The gpu thread starts at lib.rs:136, and the log (lib.rs:146) and panic hook (lib.rs:150) come after it. A panic before the install reaches only the default hook, which writes to stderr, and the gpu thread's join fallback (lib.rs:202-205) hides it.
- Confidence: high
- Trigger: A panic in the startup code before line 150, after the gpu thread is already running. This covers command_line::init, trace::install, Config::from_env and pg_log::init, or the 'gpu device' thread, which runs in the same window.
- Consequence: Only the default hook runs, which writes to stderr that a GUI build does not have. There is no gallery.log record and no box. A panic on the main startup path exits silently, and a gpu thread panic is hidden by the join fallback at lib.rs:203-205.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:136`; `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:146`; `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:150`

### UI-029 [low/unhandled-close] Closing the window waits for an open panic box on the writes thread — CONFIRMED

- Verdict: CONFIRMED
- Note: worker::stop joins the writes thread (worker.rs:54-63), and the box from a writes-thread panic blocks that thread (panic_log.rs:25). The window's exit therefore waits until the box is dismissed (lib.rs:221).
- Confidence: medium
- Trigger: The writes thread panics and its box is still open when the user closes the window. The GUI thread leaves app.exec and calls worker::stop, which joins that thread.
- Consequence: The window disappears but the process stays alive until the box is dismissed, so the app appears hung on close. If the box is hidden behind other windows, the close can look stuck.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:216`; `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:221`; `D:/GitHub/photo-gallery/crates/desktop/src/worker.rs:57`

## U07: Live reload of QML (development)

Entry: `crates/desktop/src/lib.rs:195 read_qml_from_source and :210 reload_on_change`

### UI-030 [high/missing-start] Reloaded window never starts the library: no folders, no change events — CONFIRMED

- Verdict: CONFIRMED
- Note: Library::start returns early when running() is Some (desktop library.rs:148-151), before load_roots (:152). The reloaded window's new Library therefore has empty roots, and the pump is attached to the deleted object. Live reload is debug-only.
- Confidence: high
- Trigger: Any save of a QML file in crates/desktop/qml while a debug build runs (after the start is over). The reload deletes the Library QObject and makes a new one.
- Consequence: The reloaded window's Library has empty roots, so the grid's empty state reads 'no folders' (GalleryGrid.qml:97) and the Library panel lists no folders. Scans, imports, progress and file changes never reach the window (the change pump is tied to the deleted object), until the app restarts. The window shows a wrong library state as correct.
- Sites: `crates/desktop/qml/Main.qml:105`; `crates/desktop/src/shell.rs:464`; `crates/client/src/controller/start.rs:80`; `crates/desktop/qml/Main.qml:879`; `crates/desktop/src/library.rs:148`; `crates/desktop/src/pump.rs:60`; `crates/desktop/qml/Main.qml:628`

### UI-031 [medium/stale-viewer-flag] Reload with the viewer open leaves the grid ignoring its keys — CONFIRMED

- Verdict: CONFIRMED
- Note: The new window's Viewer starts closed (index -1), and no visibleChanged close is sent for the deleted one, so the controller's viewer_open flag stays set (controller/panels.rs:125). shell.rs:153 then returns false for every grid key. Debug-only, by live reload.
- Confidence: medium
- Trigger: The viewer is open (a photo was double-clicked) and a QML file is saved. The reload deletes the window without a close, so the controller never hears the viewer close.
- Consequence: The new window shows the grid, but the controller still says the viewer is open, so shell.keyPressed returns false and the grid does not act on arrows, Enter or rating keys. The user has to double-click a cell and close the viewer to recover.
- Sites: `crates/desktop/src/shell.rs:152`; `crates/desktop/qml/Main.qml:631`; `crates/desktop/qml/Main.qml:655`; `crates/desktop/cpp/live_reload.cpp:95`; `crates/desktop/qml/Viewer.qml:155`; `crates/desktop/src/gallery/state.rs:83`

### UI-032 [medium/stale-open-panel] Reload with a panel open leaves its mirror set, blocking shortcuts and the inbox — CONFIRMED

- Verdict: CONFIRMED
- Note: The modal mirror is only changed by PanelOpened and PanelClosed (panels.rs:127-135), and teardown emits neither. The global() rule (controller.rs:427) then blocks inbox, log and dev-mode keys while a non-switchable modal stays in the mirror. Debug-only.
- Confidence: medium
- Trigger: The inbox, the library panel or a modal dialog is open when a QML file is saved. The reloaded window does not reopen it, and Qt does not emit visibleChanged during teardown, so no panelClosed is sent.
- Consequence: The controller still lists the panel as open. With the inbox open, the Inbox key and the status-bar bell send a close for a panel that is not shown, so the inbox cannot be reopened until restart. With a non-switchable modal open (library, dialog, folder picker, views menu), every global key (inbox, log, dev-mode toggle) is ignored until the user opens and closes that panel.
- Sites: `crates/client/src/controller.rs:427`; `crates/client/src/controller.rs:453`; `crates/desktop/qml/ModalOverlay.qml:18`; `crates/desktop/qml/Main.qml:565`; `crates/desktop/qml/Main.qml:846`

### UI-033 [low/no-window-state] A failed QML load on reload leaves no window and no way to quit from the UI — CONFIRMED

- Verdict: CONFIRMED
- Note: live_reload.cpp:56 sets quitOnLastWindowClosed to false. After the window is deleted at reload (:95), a failed engine.load (:100) leaves no window. quit_on_close only quits on a visibleChanged(false) of a window that exists (:40). Debug-only.
- Confidence: medium
- Trigger: A QML file is saved with a syntax or binding error during a debug run, so engine.load produces no root window.
- Consequence: The window vanishes and the process keeps running with no window, because quitOnLastWindowClosed is false. The only feedback is a qWarning on the console. The user has no on-screen message and cannot close the app from the UI.
- Sites: `crates/desktop/cpp/live_reload.cpp:56`; `crates/desktop/cpp/live_reload.cpp:57`; `crates/desktop/cpp/live_reload.cpp:100`; `crates/desktop/cpp/live_reload.cpp:40`

## U08: Grid keyboard keys (through the shell)

Entry: `crates/desktop/qml/GalleryGrid.qml:73 Keys.onPressed -> GalleryGrid.qml:32 keyPressed`

### UI-034 [medium/shadowed-key] Ctrl+R, Ctrl+digit and Ctrl+N act as the plain grid keys — CONFIRMED

- Verdict: CONFIRMED
- Note: keys.rs:114-118 match Digit, R and N with no check of mods.ctrl, and only F is guarded (:116). Ctrl+R therefore turns, Ctrl+digit rates and Ctrl+N marks. The same is true of Ctrl+B (:117), which the finding left out. Qt shortcuts for the window (Ctrl+Shift+N/L/D) are a separate path and were not traced for conflicts.
- Confidence: high
- Trigger: A photo is selected in the grid and the user presses Ctrl+R (or Ctrl+1 to Ctrl+5, or Ctrl+N). The key map ignores the Ctrl modifier for R, digits and N, so the plain action runs.
- Consequence: Ctrl+R turns the selected photo a quarter turn and writes the turn into the original file on disk (losslessly, but in place), with no confirmation or undo. Ctrl+digit writes a rating through the catalog and, with write-back on, into the file. Ctrl+N marks the borders. Ctrl+F is already guarded and does nothing, so the handling is inconsistent.
- Sites: `crates/client/src/keys.rs:114`; `crates/client/src/keys.rs:115`; `crates/client/src/keys.rs:118`; `crates/client/src/keys.rs:116`; `crates/desktop/src/keymap.rs:61`

### UI-035 [medium/wrong-selection] Arrow keys with no selection skip the first photo — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.rs:479 calls neighbor_of_selection, which uses current.max(0) (nav.rs:28), so with nothing selected Right and Down are taken from item 0. Left and Up clamp back to 0. This is the first-item case the doc (nav.rs:26) says should be selected.
- Confidence: high
- Trigger: A list is shown with nothing selected (a folder just opened, so the status bar shows the key hints). The user presses Right or Down.
- Consequence: Right selects the second photo and Down selects the photo under the first one's row, while the documented behaviour is that the first item is selected when none is. Enter then opens the wrong photo. Only Left and Up land on the first item.
- Sites: `crates/desktop/src/gallery/nav.rs:26`; `crates/desktop/src/gallery/nav.rs:28`; `crates/client/src/layout.rs:117`

### UI-036 [medium/double-fire] Two quick N presses leave the border mark set instead of toggling it off — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.rs:504 reads border_marked, which reads the committed entry (items.rs:130). A second press before the first write lands asks for the same mark that edits.rs:137 already has pending, so it is dropped.
- Confidence: high
- Trigger: The user presses N twice within the time the first border-mark write takes. The write waits for the catalog write lock, so this happens during an import or a busy catalog. The item has borders found and is not yet marked.
- Consequence: The first press queues a mark. The second press computes its toggle from the committed state (still unmarked), so it asks for 'marked' again, which matches the pending target and is dropped. The item ends marked, though two presses should have restored it. The user sees a wrong result as correct.
- Sites: `crates/desktop/src/shell.rs:504`; `crates/desktop/src/gallery/items.rs:131`; `crates/desktop/src/gallery/edits.rs:137`; `crates/desktop/src/gallery/state.rs:169`

### UI-037 [low/stale-result] A key pressed right after an arrow acts on the previous photo — CONFIRMED

- Verdict: CONFIRMED
- Note: The arrow's Step is applied on the gallery lane (shell.rs:476-484), but the controller's current (input.current, controller.rs:384) changes only when QML reports CurrentChanged (Main.qml:141). A key handled in the same pass therefore acts on the old item. Rate, turn and mark write to the file.
- Confidence: medium
- Trigger: Two keys arrive in the same event-loop pass, for example Right then 5, or Right then Enter, or Home then Enter. The arrow's move is queued on the grid model's lane, and the controller's current item (its mirror, updated only when QML reports CurrentChanged) has not caught up when the second key is handled.
- Consequence: The rating, turn, border mark or viewer goes to the old photo while the selection visibly moved to the new one. The user sees the wrong photo changed, and the change is written to the file.
- Sites: `crates/desktop/src/shell.rs:476`; `crates/client/src/controller/input.rs:84`; `crates/client/src/controller.rs:384`

## U09: Global shortcuts (inbox, log, dev mode)

Entry: `crates/desktop/qml/GlobalShortcuts.qml:24-40 Shortcut blocks with onActivated at :28, :34, :40`

### UI-038 [low/stale-result] Lane 'N failed' sets the inbox expand pass before the controller may refuse the open — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:565 sets inboxExpandPass before globalAction, and the controller can refuse it (controller.rs:427). The reset runs only in the closed branch (Main.qml:68), so the stale pass expands the first later open (InboxPanel.qml:134-142). Severity low: it needs a non-switchable modal open at the click.
- Confidence: low
- Trigger: A non-switchable modal (for example the Views menu) is still in the controller's open-panel mirror when the status bar's 'N failed' button is clicked. Main.qml sets inboxExpandPass to that lane's pass, then globalAction(Inbox) is ignored by Controller::global (controller.rs:426-429), so the inbox does not open. The user then opens the inbox later with Ctrl+Shift+N.
- Consequence: The stale pass is not cleared, since it resets only on the inbox closing (Main.qml:68). The next manual open of the inbox expands that old ledger automatically (InboxPanel expand(), InboxPanel.qml:133-143), with its details and file list shown, which the user did not ask for. The failed lane click gave no feedback at the time.
- Sites: `crates/desktop/qml/Main.qml:565`; `crates/desktop/qml/Main.qml:566`; `crates/desktop/qml/Main.qml:567`; `crates/desktop/qml/Main.qml:68`; `crates/desktop/qml/InboxPanel.qml:138`

## U10: Select a photo by click

Entry: `crates/desktop/qml/GridCell.qml:182 MouseArea onClicked -> cellClicked (GridCell.qml:34) -> GalleryGrid cellClicked -> Main.qml:632-634 (win.current = i)`

### UI-039 [low/pre-start-selection-not-saved] Click before the start is over is not remembered — CONFIRMED

- Verdict: CONFIRMED
- Note: CurrentChanged arms the save (controller.rs:292). The timer fires into the Timer::SaveSession arm, which does nothing before the start (controller.rs:536). Started saves only with apply_view (controller.rs:361), so the pre-start selection is not remembered.
- Confidence: medium
- Trigger: The list is shown before the folder tree has loaded (start not yet over). The user clicks a photo, and the 1 s SaveSession pause fires, or the window closes, before Started. Nothing saves the selection after Started, unless a view was applied at start (or the user clicks again after it).
- Consequence: The session's last file (prefs last_file) stays at the previous selection. On the next launch a different photo is selected than the one the user had clicked. No work is lost.
- Sites: `D:/GitHub/photo-gallery/crates/client/src/controller.rs:292`; `D:/GitHub/photo-gallery/crates/client/src/controller.rs:532`; `D:/GitHub/photo-gallery/crates/client/src/controller.rs:536`; `D:/GitHub/photo-gallery/crates/client/src/controller.rs:361`

### UI-040 [low/missing-feedback] Status line goes blank for a selected file whose details are unavailable — CONFIRMED

- Verdict: CONFIRMED
- Note: Details are read with .flatten() (state.rs:156) and an info miss gives an empty string (items.rs:195). The cache keeps (file, None) (state.rs:135). The finding says the state persists until the list reloads, but the two-entry cache also evicts when other reads come in, so that part is overstated.
- Confidence: medium
- Trigger: (a) The user clicks a file and its details read is still in flight: the status line shows only the rating, or nothing, and the key hints are replaced until the read returns. (b) The file has left the catalog while it is still in the open list: the read returns Ok(None), which is cached as (file, None). That state persists until the list is reloaded, with no notice.
- Consequence: The status bar shows an empty line for a selected photo, and in case (b) it stays empty with no message while the grid still draws the cell as selected.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/gallery/state.rs:156`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/items.rs:195`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/model.rs:283`

## U11: Open the viewer by double-click

Entry: `crates/desktop/qml/GridCell.qml:183 onDoubleClicked -> GalleryGrid cellDoubleClicked -> Main.qml:635 viewerItem.show(i)`

### UI-041 [low/double-fire] Uncached file's details read five times on viewer open — CONFIRMED

- Verdict: CONFIRMED
- Note: state.rs:137 issues read_details on every miss and records nothing as pending. refresh_viewed calls info and four border_rejected in a row (model.rs:323-333), and each call misses before any delivery, so five reads go out.
- Confidence: high
- Trigger: Double-click an image whose details are not in the gallery model's two-entry details cache (first view of that file in this session, or after another file evicted it). refresh_viewed runs info(item) and then border_rejected for each of the four edges, all in one synchronous call before any read has been delivered.
- Consequence: Five identical catalog reads of the same file are issued. Each delivery runs details_read, which runs refresh_derived and refresh_viewed again. The displayed values end up correct, so the user sees no wrong result. This is wasted work and extra UI refreshes on the open path.
- Sites: `crates/desktop/src/gallery/state.rs:137`; `crates/desktop/src/gallery/model.rs:323`; `crates/desktop/src/gallery/model.rs:327`; `crates/desktop/src/gallery/model.rs:329`; `crates/desktop/src/gallery/model.rs:331`; `crates/desktop/src/gallery/model.rs:333`

### UI-042 [low/missing-feedback] Viewer opens onto an empty picture area until the fit renders — CONFIRMED

- Verdict: CONFIRMED
- Note: ImageLayers.qml:70 binds the fit to fitSource, which is empty until Rust's render is ready. Viewer.qml has no BusyIndicator or placeholder (grep found none), so the viewer shows the black area until the render lands.
- Confidence: medium
- Trigger: Double-click a photo whose fitted render is not yet cached at the current window size, for example the first open of a file, or the first open after a resize. The Rust render (Lanczos) is asynchronous.
- Consequence: The viewer covers the grid with a black area and only the counter and the info line. No busy indicator or placeholder (for example the grid's thumbnail) is shown until the render is Ready. A large original may leave the viewer looking blank for a noticeable time. Nothing gets stuck: the picture appears when the render completes, and a failure shows the Hud's original-unavailable text.
- Sites: `crates/desktop/qml/ImageLayers.qml:70`; `crates/desktop/qml/Viewer.qml:85`

## U12: Thumbnail size: Ctrl+wheel, slider, keys

Entry: `crates/desktop/qml/GalleryGrid.qml:77 onWheel -> sizeStepped (GalleryGrid.qml:36); crates/desktop/qml/TopBar.qml:142 onMoved -> sizeChosen (TopBar.qml:56)`

### UI-043 [medium/overshoot] Ctrl+wheel applies a full 1.1 step per wheel event, not per notch — CONFIRMED

- Verdict: CONFIRMED
- Note: GalleryGrid.qml:77 gives every wheel event the full 1.1 or 1/1.1 factor (size.rs:81 applies it directly). Its magnitude is not checked against angleDelta, so a high-resolution wheel compounds per event.
- Confidence: medium
- Trigger: Hold Ctrl and roll a high-resolution mouse wheel (free-spin or driver-level sub-notch deltas). Each event has a small angleDelta.y but gets the full 1.1 (or 1/1.1) factor, so one physical notch that arrives as ten events scales the row height by about 1.1^10 (roughly 2.6x), and a longer roll hits the 60 or 800 limit.
- Consequence: The thumbnails jump far more than one notch's worth. The user cannot make a fine adjustment with a high-resolution wheel, and the overshoot is clamped at the limits, so the size lands somewhere the user did not choose.
- Sites: `crates/desktop/qml/GalleryGrid.qml:77`; `crates/client/src/controller/size.rs:81`

### UI-044 [low/anchor-lost] Relayout loses the top-row anchor when the probe point falls in the row gap — CONFIRMED

- Verdict: CONFIRMED
- Note: topItem probes indexAt(10, contentY + 1) (GalleryGrid.qml:42). In a row gap that returns -1, so anchor is -1 (Main.qml:242) and the positionAt at Main.qml:249 is skipped.
- Confidence: medium
- Trigger: No item is selected, and the view is scrolled so that contentY+1 lies in the 4 px gap between two rows (about 1 in 50 to 70 scroll positions with 220 px rows). Then press Ctrl+plus or Ctrl+minus, or Ctrl+wheel, so the grid relayouts.
- Consequence: topItem() returns -1, relayout() skips the positionAt, and the view keeps its raw pixel offset under rows that have changed height. The row the user was looking at shifts, so the view jumps to another part of the library.
- Sites: `crates/desktop/qml/GalleryGrid.qml:42`; `crates/desktop/qml/Main.qml:242`; `crates/desktop/qml/Main.qml:249`

### UI-045 [low/wrong-step] Slider keyboard arrows move the row height by 0.1 px per press — CONFIRMED

- Verdict: CONFIRMED
- Note: Qt's Slider keyPressEvent steps by increase() and decrease(), which use 0.1 when stepSize is 0 (qquickslider.cpp:661, 675, 754-788), and it emits moved each time. TopBar.qml:138-142 sends each value to sizeChosen, so presses change the height only after the value rounds to a new row height.
- Confidence: high
- Trigger: Click or Tab to the size slider, then press Left or Right. The Slider has no stepSize, so Qt steps by 0.1 and emits moved() each time.
- Consequence: Each press sets a fractional value that shell.rs rounds back to the same row height, so the grid does not change for about six presses. The knob barely moves, so the keyboard path to the size is far slower than the mouse drag and shows no feedback for long stretches.
- Sites: `crates/desktop/qml/TopBar.qml:138`; `crates/desktop/src/shell.rs:193`

### UI-046 [low/focus] Size changes from the slider or size box leave focus on the control, so Ctrl+plus/minus does nothing — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:544 sends size changes from the top bar with no takeFocus, unlike the sort path (Main.qml:532). The slider keeps focus, so Ctrl+plus and Ctrl+minus go to the slider and not the grid.
- Confidence: high
- Trigger: Drag the size slider or edit the size box and press Enter, then press Ctrl+plus or Ctrl+minus (or an arrow key) without first clicking the grid.
- Consequence: The grid never gets the key, so the keyboard resize and grid navigation are dead until the user clicks a thumbnail. Sort, direction and saved-view changes return focus to the grid, so this path is inconsistent with the rest of the top bar.
- Sites: `crates/desktop/qml/TopBar.qml:142`; `crates/desktop/qml/Main.qml:544`; `crates/desktop/qml/Main.qml:532`

## U13: Grid relayout and scroll position after a resize

Entry: `crates/desktop/qml/GalleryGrid.qml:38 gridResized -> Main.qml:637 onGridResized -> shell.gridResized (shell_bridge.rs:106)`

### UI-047 [medium/lost-anchor] topItem() returns -1 in the row spacing, losing the scroll anchor — CONFIRMED

- Verdict: CONFIRMED
- Note: Same topItem defect as UI-044 (GalleryGrid.qml:42). The refresh path adds the anchor set in aboutToLoad (Main.qml:91), which takes -1 from the gap and then positionAtBeginning on the loaded index. The relayout half is UI-044's.
- Confidence: high
- Trigger: Nothing selected, and the grid is scrolled so the viewport's top pixel (contentY + 1) falls in the 4 px gap between two rows. Then either (a) a library refresh that keeps the view (Reload, from a finished scan or import: refreshGrid -> reloadList -> Event::Reload -> open_list(keep=true)), or (b) a relayout (a Ctrl+wheel row-height step, or a window width change). Roughly 4/(row height + 4) of all scroll positions hit this.
- Consequence: (a) The refresh anchors on -1, so the grid reaches gridLoaded with index -1 and calls positionAtBeginning: the view jumps to the top of the library, losing the user's place. (b) relayout skips positioning when anchor is -1 and keeps the raw contentY, so after the row heights change the view shows different files from the ones that were on screen. The selected-item path is not affected.
- Sites: `crates/desktop/qml/GalleryGrid.qml:42`; `crates/desktop/qml/GalleryGrid.qml:44`; `crates/desktop/qml/Main.qml:91`; `crates/desktop/qml/Main.qml:242`; `crates/desktop/qml/Main.qml:249`; `crates/desktop/qml/Main.qml:270`; `crates/client/src/grid.rs:448`

### UI-048 [low/wrong-action] Ctrl+wheel with no vertical delta shrinks the thumbnails — CONFIRMED

- Verdict: CONFIRMED
- Note: GalleryGrid.qml:77 maps any angleDelta.y <= 0 to 1/1.1, so a Ctrl wheel with no vertical delta shrinks the row height.
- Confidence: medium
- Trigger: Ctrl is held while a wheel event arrives with angleDelta.y == 0 (a horizontal or tilt-wheel scroll, or any zero-vertical Ctrl wheel event). The WheelHandler (acceptedModifiers Ctrl) takes it.
- Consequence: The row height steps down by 1/1.1 instead of staying put, which relayouts the grid and changes the thumbnail size. The user sees the thumbnails shrink on a horizontal scroll they did not mean as a zoom.
- Sites: `crates/desktop/qml/GalleryGrid.qml:77`

## U14: Grid scrollbar drag

Entry: `crates/desktop/qml/GalleryGrid.qml:123 onPositionChanged (while pressed)`

### UI-049 [medium/stuck-busy] Scrollbar stays pressed when the drag is released outside the grid — CONFIRMED

- Verdict: CONFIRMED
- Note: qquickscrollbar.cpp:317-321 returns before setPressed(false) (line 332) when the release point is outside the vertical padding span. pressed stays true, which holds the Binding at GalleryGrid.qml:119-121 and keeps the readout visible (:133).
- Confidence: high
- Trigger: The user drags the scroll handle above the top or below the bottom of the grid (for example to reach the first or last photo, with the pointer over the top bar or status bar) and releases the mouse there. The release point is outside the bar's vertical span.
- Consequence: gridBar.pressed stays true. The 'file ≈ N of M' readout stays on screen over the right edge of the grid at the bar's last position until the user presses and releases inside the bar's span. The handle keeps its pressed colour and stops following the view on wheel, keys or list reloads, because the Binding on position is disabled while pressed. The bar shows a drag that has already ended.
- Sites: `crates/desktop/qml/GalleryGrid.qml:121`; `crates/desktop/qml/GalleryGrid.qml:133`; `C:/Qt/6.12.0/Src/qtdeclarative/src/quicktemplates/qquickscrollbar.cpp:320`

### UI-050 [low/wrong-position] Readout is placed from the logical position, not the drawn handle — PARTIAL

- Verdict: PARTIAL
- Note: The readout uses gridBar.position (GalleryGrid.qml:138). The handle is placed from visual.position (qquickscrollbar.cpp:229). I did not check whether position and visual.position differ once minimumSize applies, which is what the 24 px figure depends on.
- Confidence: high
- Trigger: A library large enough that the bar's minimumSize (48 px) exceeds its size ratio (roughly contentHeight above height squared over 48, i.e. thousands of files), and any drag.
- Consequence: The readout's centre sits up to about 24 px above the handle's centre at the top of the track and about 24 px below it at the bottom. The number is correct, but the bubble does not sit on the handle it describes.
- Sites: `crates/desktop/qml/GalleryGrid.qml:138`; `C:/Qt/6.12.0/Src/qtdeclarative/src/quicktemplates/qquickscrollbar.cpp:229`

### UI-051 [low/race] A list applied during a drag snaps the view to the selected file while the bar is held — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:264-270 positions the grid when a list is shown, with no check for a drag in progress, and the GalleryGrid Binding is off while pressed (GalleryGrid.qml:121). The view jumps while the handle stays where it was.
- Confidence: medium
- Trigger: The user holds the scroll handle while a new list is applied (an import or a scan finishes, or the folder is read again) and a file is selected that is outside the view.
- Consequence: The view jumps to the selected file while the handle stays where the pointer left it. The view returns to the drag position only on the next pointer move, so the handle and the view disagree in the meantime. Without a selection the view stays at the top row, so this case is only the selected one.
- Sites: `crates/desktop/qml/Main.qml:265`; `crates/desktop/qml/Main.qml:92`

## U15: Folder tree: expand and open a folder

Entry: `crates/desktop/qml/FolderTree.qml:52 TapHandler, :65 onClicked, :69 onDoubleClicked -> toggleRequested (FolderTree.qml:18); folderOpened (FolderTree.qml:16)`

### UI-052 [medium/double-fire] Expand arrow also lists the folder — CONFIRMED

- Verdict: CONFIRMED
- Note: The arrow's TapHandler (FolderTree.qml:52) and the delegate's onClicked (:65) both get the press, since the arrow sits inside the delegate. The click sets the tree row and calls folderOpened (Main.qml:605-608). Qt delivers the press to both unless one takes an exclusive grab, which I did not check in the delegate.
- Confidence: medium
- Trigger: A single click on the expand arrow of a folder that has children, not on its name.
- Consequence: The arrow is documented as expand or collapse only (FolderTree.qml header: the arrow or a double click expands it). Its press also reaches the row delegate's onClicked, which sets the tree row and calls folderOpened, so the grid switches to that folder. The grid's selection is cleared (grid.rs:278 keep=false, then selection_after gives None at grid.rs:443-444) and the view jumps to the top (Main.qml:269-270 positionAtBeginning). A user who only wanted to see subfolders loses their place in the current folder.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/FolderTree.qml:52`; `D:/GitHub/photo-gallery/crates/desktop/qml/FolderTree.qml:65`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:605`

### UI-053 [medium/double-fire] Double click on the arrow toggles the folder twice — CONFIRMED

- Verdict: CONFIRMED
- Note: qquicktaphandler.cpp:414 emits tapped on every tap, so a double click on the arrow calls toggleRequested twice (FolderTree.qml:52). The delegate's onDoubleClicked (:69) may add a third call. I did not confirm that third call.
- Confidence: medium
- Trigger: A double click on the expand arrow of an expandable folder.
- Consequence: TapHandler.tapped is emitted for every tap (qquicktaphandler.cpp:414, with the per-tap tapCount), so the two taps call toggleRequested twice. folderModel.toggle expands then collapses, and the tree looks unchanged. The delegate's onDoubleClicked (FolderTree.qml:69) may add a third toggle if the press reaches the delegate, so the end state is either unchanged or expanded, not a predictable result. Each tap also lists the folder (finding 1), so two OpenList commands are issued.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/FolderTree.qml:52`; `D:/GitHub/photo-gallery/crates/desktop/qml/FolderTree.qml:69`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:604`; `D:/GitHub/photo-gallery/crates/desktop/src/folders.rs:202`

### UI-054 [low/wrong-message] Listing failure in the empty folder reads without a folder name — CONFIRMED

- Verdict: CONFIRMED
- Note: display_merged (rules.rs:178-188) of the empty Merged is "" because the parts loop does nothing. model.rs:99 and :102 then format "{shown}: {e}", so the notice starts with a bare ": ". The toast text itself was not read.
- Confidence: medium
- Trigger: The catalog read for the listing fails while the empty folder (All, or Drive roots) is the listed folder.
- Consequence: The error string is built as the folder name, a colon, then the error. The folder name is display_merged of the empty folder, which is the empty string (rules.rs:178-188), so the notice text begins with a bare separator. The grid still shows as empty with no separate error, so the wording is wrong for the situation. The rendered toast text was not read, so the exact display is unverified.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/gallery/model.rs:99`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/model.rs:102`

## U16: Include subfolders toggle

Entry: `crates/desktop/qml/TopBar.qml:97 onToggled -> recursiveToggled -> Main.qml:519-522`

### UI-055 [medium/wrong-message] Empty folder without subfolders says All folders over an empty grid — CONFIRMED

- Verdict: CONFIRMED
- Note: shown_as (tree.rs:113-117) gives All when root_files is 0, and listing_kind_of maps that to KIND_ALL (folders.rs:95-100). TopBar.qml:75 then labels All folders while the box is unchecked and the non-recursive listing is empty.
- Confidence: high
- Trigger: The library has no files directly in any drive root, so every photo is in a subfolder. The user is on All folders with Include subfolders checked, then unchecks it. The non-recursive empty-folder listing is empty.
- Consequence: The grid goes empty and the top bar still reads All folders with 0 files. The tree highlights its All row while the checkbox is unchecked, so the label contradicts both the unchecked box and the empty grid. The code's comments (tree.rs:109-112, folders.rs:87-89) say the All label is deliberate to match the highlighted row, so this is a misleading state rather than a crash.
- Sites: `D:/GitHub/photo-gallery/crates/client/src/tree.rs:117`; `D:/GitHub/photo-gallery/crates/desktop/src/folders.rs:99`; `D:/GitHub/photo-gallery/crates/desktop/qml/TopBar.qml:75`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:520`

### UI-056 [low/focus] Subfolders toggle leaves focus on the checkbox, unlike sort and descending — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:519-521 calls setRecursive and followListedRow with no takeFocus. The sort (:531-532) and descending (:535-536) paths do call it.
- Confidence: medium
- Trigger: The user clicks Include subfolders (or Tabs to it and presses Space), then presses arrow keys or Enter expecting the grid to respond.
- Consequence: Keyboard focus stays on the CheckBox after the toggle. Grid keys do not reach the grid until the user clicks it. The sort box and the descending button both return focus to the grid, so this path is inconsistent with them. There is no keyboard shortcut for subfolders, so the checkbox is the only keyboard route.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:519`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:531`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:532`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:521`

### UI-057 [low/wrong-highlight] Tree highlight falls back to All when the listed folder is missing from the tree — CONFIRMED

- Verdict: CONFIRMED
- Note: revealListing returns -2 for a folder missing from the tree (folders.rs:246), and followListedRow clamps it with Math.max(row, 0) (Main.qml:299), so the highlight falls to All. It clears when the tree arrives (shell.rs treeLoaded(false), UI-006 chain).
- Confidence: medium
- Trigger: A scan removed the listed folder from the tree, and the tree reload has not arrived yet (it is read off the GUI thread). The user toggles Include subfolders in that window.
- Consequence: revealListing returns -2 for a folder not in the tree, and followListedRow clamps it to row 0. The tree then highlights All while the grid still lists the removed folder's files. This is brief and latent: treeLoaded with folder_known=false later lists all folders and fixes the view.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:299`; `D:/GitHub/photo-gallery/crates/desktop/src/folders.rs:246`

## U17: Sort box and descending toggle

Entry: `crates/desktop/qml/TopBar.qml:111 onActivated -> sortActivated (Main.qml:523-533); TopBar.qml:116 onClicked -> descendingToggled (Main.qml:534-537)`

### UI-058 [medium/wrong-state] Re-picking the current sort resets the toggled direction — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.sort_by dispatches SortChoice (shell.rs:306-310), and listing::update sets listing.sort = sort_choice(index) (listing.rs:57-58), which gives the default direction (sort.rs:25-28). That is followed by OpenList and SaveSession (listing.rs:77), so re-picking the same key resets the direction and re-lists.
- Confidence: high
- Trigger: Pick By file size, click the arrow to ascending, then open the sort box and click By file size again (or press Enter on it). The popup accept emits activated for the unchanged index, and the pick sets the choice's default direction.
- Consequence: The ascending order the user chose is silently replaced by the default descending order, with the arrow flipped and the grid re-listed and scrolled to the top. The selection is also cleared, since the re-list is not a keep. Nothing the user asked for changed.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:531`; `D:/GitHub/photo-gallery/crates/client/src/controller/listing.rs:57`; `D:/GitHub/photo-gallery/crates/client/src/sort.rs:27`; `C:/Qt/6.12.0/Src/qtdeclarative/src/quicktemplates/qquickcombobox.cpp:374`; `C:/Qt/6.12.0/Src/qtdeclarative/src/quicktemplates/qquickcombobox.cpp:433`

### UI-059 [low/focus] Arrow keys on the closed sort box lose focus after one step — CONFIRMED

- Verdict: CONFIRMED
- Note: TopBar.qml:111 passes every activated to sortActivated, and Main.qml:531-532 ends it with grid.takeFocus(). Qt emits activated when a closed ComboBox steps by keyboard (qquickcombobox.cpp:692-693, incrementCurrentIndex at :696), so focus leaves the box after one step.
- Confidence: high
- Trigger: Tab to the sort box with the popup closed and press Down (or Up, Home, End, or type a letter). Qt emits activated for keyboard navigation of a closed ComboBox, and the handler then moves focus to the grid.
- Consequence: The second Down press goes to the grid and moves its selection instead of the sort. Each step also relists the library. The keyboard user cannot step through the sort box without tabbing back.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/TopBar.qml:111`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:532`; `C:/Qt/6.12.0/Src/qtdeclarative/src/quicktemplates/qquickcombobox.cpp:696`

### UI-060 [low/wrong-state] Border-depth fallback turns the list into Name descending — CONFIRMED

- Verdict: CONFIRMED
- Note: without_borders (features.rs:27-33) changes a Borders sort to Name and keeps the direction, and listing::update calls it on BordersTurnedOff (listing.rs:70-75). The default for Borders is descending, so the list becomes Name descending.
- Confidence: medium
- Trigger: Sort by border depth (its default is descending, largest first), then turn border detection off in the Library panel.
- Consequence: The list is silently reversed to Z to A under By name with the descending arrow, an order the user never chose. The sort box and arrow are consistent with the list, so this is a surprise rather than a misleading display.
- Sites: `D:/GitHub/photo-gallery/crates/client/src/features.rs:27`; `D:/GitHub/photo-gallery/crates/client/src/features.rs:30`; `D:/GitHub/photo-gallery/crates/client/src/sort.rs:27`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:709`

## U18: Apply a saved view

Entry: `crates/desktop/qml/ViewsMenu.qml:41 onTriggered -> viewChosen (ViewsMenu.qml:22) -> TopBar viewChosen -> Main.qml:538 win.applyView`

### UI-061 [medium/wrong-state] Applied view whose folder is missing from the tree highlights All folders — CONFIRMED

- Verdict: CONFIRMED
- Note: applyView takes Math.max(row, 0) (Main.qml:329), and revealListing gives -2 for a folder missing from the tree (folders.rs:246). The list keeps the view's folder and TopBar.qml:74 shows its name, while the tree highlights All.
- Confidence: high
- Trigger: A saved view names folder X, and X is no longer in the tree (its files were removed from the library or the folder is empty). The user picks the view from the Views menu. folders.rs:246 returns -2 for a folder the tree does not have. Main.qml:329 maps it to row 0.
- Consequence: The grid lists folder X (or shows its empty message) and the top bar labels it X, but the tree highlights All folders. The tree and the listing disagree until the next tree read. The -2 to 0 mapping is correct for treeLoaded, where the controller lists all folders, but not for applyView, which keeps the view's folder.
- Sites: `crates/desktop/qml/Main.qml:329`; `crates/desktop/src/folders.rs:246`; `crates/desktop/qml/TopBar.qml:74`

### UI-062 [low/missing-feedback] A failed read of the chosen view is silently a no-op — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.rs:379-381 returns false on a failed view read with no notice. Main.qml:324-325 then returns at once. views_outcome (shell.rs:115-117) is used for save and delete only.
- Confidence: high
- Trigger: The catalog read behind views().view(name) fails (a database error) when a view is chosen from the menu.
- Consequence: shell.apply_view returns false with no notice, and Main.applyView returns at once. The user clicks a view and nothing happens, with no message. Saving and deleting views report failures through views_outcome, so this path is the odd one out.
- Sites: `crates/desktop/src/shell.rs:379`; `crates/desktop/qml/Main.qml:324`; `crates/desktop/src/shell.rs:115`

### UI-063 [low/focus] A refused view apply leaves keyboard focus off the grid — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:324-325 return before grid.takeFocus() (line 332), so focus stays off the grid after a refused apply. Where Qt puts focus after the menu closes was not checked.
- Confidence: medium
- Trigger: Borders are off and the chosen view uses borders (or the view is missing). shell.apply_view returns false, and Main.qml returns at line 325 before grid.takeFocus() at line 332.
- Consequence: After the menu closes, focus goes back to the Views button (Qt restores the last active item of the popup). The grid's arrow keys and shortcuts stop working until the user clicks the grid. Before the menu was opened, the grid had focus.
- Sites: `crates/desktop/qml/Main.qml:324`; `crates/desktop/qml/Main.qml:332`; `crates/desktop/src/shell.rs:383`

### UI-064 [low/double-fire] A pending query pause re-lists the applied view and clears the selection — CONFIRMED

- Verdict: CONFIRMED
- Note: No deadline is cleared on ViewApplied: the only drain is flush (controller.rs:512). A pending ApplyQuery therefore fires listing_now (controller.rs:549-551), which opens the list with keep=false.
- Confidence: medium
- Trigger: The user types in the filter bar's name field and picks a saved view within 300 ms, so the ApplyQuery pause is still armed when ViewApplied replaces the listing.
- Consequence: About 300 ms after the view appears, the pause fires listing_now, which sends OpenList with keep=false. The grid scrolls back to the top and the selection is cleared, although nothing changed. Rare, since it needs typing and a menu pick within the pause.
- Sites: `crates/client/src/controller.rs:301`; `crates/client/src/controller.rs:529`; `crates/client/src/controller.rs:549`

### UI-065 [low/stale-result] A view applied before the start is over can be replaced by all folders — CONFIRMED

- Verdict: CONFIRMED
- Note: start.rs:59-64 has no started check, so TreeLoaded{false} emits OpenAll whenever it arrives. Main.qml:292 sends it for a view whose folder is missing from the tree (folders.rs:246 gives -2). This needs a view applied before the tree arrives, a narrow window.
- Confidence: medium
- Trigger: A view whose folder is not in the tree is applied before the start gate has its tree (before TreeLoaded). Main.qml:292 then sends shell.treeLoaded(false), and the gate emits OpenAll with no started check.
- Consequence: The user's applied view is replaced by the All folders listing without a notice, and the filter bar keeps the view's query. The window is only usable while the start runs for a brief time, so the window is narrow.
- Sites: `crates/client/src/controller/start.rs:59`; `crates/client/src/controller/start.rs:63`; `crates/client/src/controller.rs:357`; `crates/desktop/qml/Main.qml:292`

## U19: Save the current view (dialog)

Entry: `crates/desktop/qml/ViewsMenu.qml:49 onTriggered -> saveViewRequested -> TopBar.qml:54 -> Main.qml:540 askSaveView (Main.qml:389)`

### UI-066 [medium/missing-feedback] Saving with an empty or blank name closes the dialog and saves nothing, silently — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.rs:318-320 returns on an empty name with no notice, and the dialog closes on Dialog.Save (SaveViewDialog.qml:38, :44). No toast or inbox entry is posted.
- Confidence: high
- Trigger: The user opens Views > Save current view…, leaves the name empty (or types only spaces), and presses Save or Enter.
- Consequence: The dialog closes as if the view were saved, but no entry appears in the Views menu and no notice is shown. The user believes the view exists when it does not.
- Sites: `crates/desktop/qml/SaveViewDialog.qml:44`; `crates/desktop/qml/SaveViewDialog.qml:38`; `crates/desktop/src/shell.rs:318`

### UI-067 [medium/stale-state] Save dialog's filter summary still shows border filters that the saved view drops — PARTIAL

- Verdict: PARTIAL
- Note: BordersTurnedOff changes the listing (listing.rs:70-75), and showListing refreshes only the counts and the sort box (Main.qml:317-320). It does not call filterBar.reloadQuery (only applyView and the view-by-name path do, Main.qml:326, 341), so the bar's fields keep the border filters. Whether the Save dialog's summary reads those fields was not traced.
- Confidence: high
- Trigger: The current query uses a border facet or a Review choice. The user unticks Detect borders in the Library panel (listing drops the border filters), then opens Views > Save current view and reads the summary before confirming.
- Consequence: The summary line shows the filter bar's old description, including the border filters. The saved view is stored without them. The dialog therefore misstates what the user is saving, and the filter bar's own count (Filters button, N of M) also stays stale.
- Sites: `crates/desktop/qml/Main.qml:709`; `crates/desktop/src/shell.rs:373`; `crates/client/src/controller/listing.rs:70`; `crates/desktop/src/filters.rs:64`; `crates/desktop/qml/FilterBar.qml:44`

## U20: Delete a saved view

Entry: `crates/desktop/qml/ViewsMenu.qml:60 onTriggered -> viewDeleteRequested -> TopBar.qml:53 -> Main.qml:539 shell.deleteView`

### UI-068 [medium/stays-open] Views menu stays open after deleting a view — CONFIRMED

- Verdict: CONFIRMED
- Note: qquickmenu.cpp:1131 calls dismiss only from onItemTriggered, which is connected in insertItem (:621) and disconnected in removeItem (:720). The QML onTriggered (ViewsMenu.qml:60) runs viewDeleteRequested, which removes the item from deleteMenu during its own emission (:62-63). The disconnect then stops dismiss from being called. The order depends on QML connecting before the menu does, which I did not check.
- Confidence: medium
- Trigger: User opens Views, goes to Delete view, and clicks a view name. The delete runs, and the triggered item is removed from deleteMenu during its own triggered emission, which disconnects QQuickMenu's onItemTriggered before it can call dismiss().
- Consequence: The Views menu and its Delete submenu remain open after the action is done, showing the remaining views. The ViewsMenu panel stays marked open to Shell (Main.qml:545 panelOpened), so modal-panel rules still apply until the user clicks outside or presses Escape. Repeated clicks can delete a second view by mistake.
- Sites: `crates/desktop/qml/ViewsMenu.qml:60`; `crates/desktop/qml/ViewsMenu.qml:63`; `crates/desktop/qml/Main.qml:545`

### UI-069 [low/missing-confirmation] Saved view deleted immediately with no confirmation or undo — CONFIRMED

- Verdict: CONFIRMED
- Note: ViewsMenu.qml:60 calls viewDeleteRequested on a single trigger, and Main.qml:539 forwards it to shell.deleteView (shell.rs:326-329) with no confirmation. The other destructive dialogs, such as RemoveFolderDialog, do confirm.
- Confidence: high
- Trigger: A single click on a name in Views > Delete view. Only one submenu level away from Save current view.
- Consequence: The view's stored folder, query, sort and name are permanently removed by one misclick. Other destructive actions in this window (RemoveFolderDialog, DiscardRemovalDialog, CutBordersDialog) confirm first, so this path is inconsistent.
- Sites: `crates/desktop/qml/ViewsMenu.qml:60`; `crates/desktop/qml/Main.qml:539`

### UI-070 [low/wrong-state-on-read-failure] Failed list read after a delete empties the menu while views still exist — CONFIRMED

- Verdict: CONFIRMED
- Note: view_names uses unwrap_or_default (shell.rs:121). delete_view then sets saved_views from it (shell.rs:328), so a failed list read gives an empty Delete submenu. The ViewOp::List notice is posted (shell.rs:116).
- Confidence: medium
- Trigger: The delete succeeds, or fails, and then the catalog read in view_names() fails (for example a locked or unreadable catalog). unwrap_or_default() returns an empty list.
- Consequence: The Views menu lists no views and the Delete submenu is disabled, even though the catalog still holds them. The 'couldn't be listed' notice does say a read failed, so the user is told, but the menu state is wrong until the next successful read.
- Sites: `crates/desktop/src/shell.rs:121`; `crates/desktop/src/shell.rs:328`

## U21: Filter bar open and close, facet counts

Entry: `crates/desktop/qml/TopBar.qml:103 onClicked -> filtersToggled -> Main.qml:518 shell.toggleFilters (shell_bridge.rs:130)`

### UI-071 [medium/missing-feedback] Start with --filters shows an empty filter bar — CONFIRMED

- Verdict: CONFIRMED
- Note: The bar's controls are created only once `opened` is set (FilterBar.qml:61-62), and that happens only on a visibleChanged to true. A bar that starts visible from --filters (controller.rs:125) may never see that change, so its controls are never made. Confirming the Qt visibleChanged behaviour at creation was not done.
- Confidence: high
- Trigger: The app starts with the --filters option. The bar is open at the start, but its controls are never created.
- Consequence: The Filters button shows the open arrow over an empty strip with no chips, ranges, duplicates, or name field. The user must click Filters closed and open again to see any control. Only the command line reaches this state.
- Sites: `crates/desktop/qml/FilterBar.qml:61`; `crates/desktop/qml/Main.qml:584`; `crates/desktop/src/controller.rs:125`

### UI-072 [medium/wrong-message] A failed facet count shows zero counts as real data — CONFIRMED

- Verdict: CONFIRMED
- Note: filters.rs:165-166 applies unwrap_or_default on a failed facet read. apply_facets then sets total and matching from zeros (filters.rs:178-182), and the chips read 0 through the same default.
- Confidence: high
- Trigger: The catalog read for the filter counts fails (the connection cannot be opened, the query errors, or the read panics) while the bar is open or a filter is active.
- Consequence: Every chip shows a count of 0, the total becomes 0, and the label reads like N of 0 files. This looks like a true result. The only signal is the FacetsFailed toast in the inbox, and the bar keeps the zeros until a later count succeeds.
- Sites: `crates/desktop/src/filters.rs:165`; `crates/desktop/src/filters.rs:166`; `crates/desktop/src/filters.rs:181`

### UI-073 [low/missing-feedback] Chip counts show zeros or the previous folder's counts until the count returns — CONFIRMED

- Verdict: CONFIRMED
- Note: facet_choices.rs:41 maps a count that has not arrived to 0 (shown.unwrap_or(0)), so the chips show 0 until the first count. After a folder change the previous rows stay until the new count is delivered.
- Confidence: high
- Trigger: The bar is opened for the first time (no count has run yet), or the folder or query changes while the bar is open, and the new count is still being read off the GUI thread.
- Consequence: The chips show 0 and the extension row is empty on first open. After a folder change they show the previous folder's counts, with no pending indicator. The numbers look current until the count arrives. A large catalog can take noticeable time.
- Sites: `crates/desktop/src/controller.rs:28`; `crates/desktop/src/facet_choices.rs:41`; `crates/desktop/qml/FilterBar.qml:143`

### UI-074 [low/focus] Clicking Filters leaves keyboard focus on the Filters button, not the grid — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:518 calls shell.toggleFilters with no grid.takeFocus, unlike the sort and direction handlers (Main.qml:532, :536). Whether a Qt Button takes StrongFocus on Windows was not checked.
- Confidence: medium
- Trigger: The user clicks the Filters button to open or close the bar with the mouse, then uses the arrow keys, Enter or F.
- Consequence: Qt's Button takes StrongFocus on Windows, so the click leaves focus on the Filters button. Grid keys then do not reach the grid until the user clicks the grid. Sort and the direction button move focus back to the grid, so this handler is the inconsistent one.
- Sites: `crates/desktop/qml/Main.qml:518`; `crates/desktop/qml/Main.qml:532`; `crates/desktop/qml/TopBar.qml:103`

## U22: Filter facet toggles

Entry: `crates/desktop/qml/FilterBar.qml:157 (types), :170 (ratings), :182 (extensions), :194 (orientations), :237 (status) onClicked -> filters.toggle; :249 onClicked (borders, bordersBlocked when borders are off)`

### UI-075 [medium/wrong-message] A failed count shows as zero: N of 0 and every chip reads (0) — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-072)
- Note: Duplicate of UI-072 from the top bar's side: the same unwrap_or_default at filters.rs:166. TopBar.qml:84-85 then reads "N of 0", and the chips read (0).
- Confidence: high
- Trigger: A Facets read fails (the catalog connection fails, a lock or I/O error, or a panic) while the filter bar is open or a filter is active; the failure is delivered through apply_facets with the default Facets.
- Consequence: The top bar reads 'N of 0' (TopBar.qml:84-85) and every type, extension and border chip shows (0) (FilterBar.qml:156, 181, 248). The user is shown a wrong count as if it were the real one. The only signal is the 'filters' counts couldn't be read' notice in the inbox (text.ftl:298-299), and the zeros stay until a later successful count.
- Sites: `crates/desktop/src/filters.rs:163`; `crates/desktop/src/filters.rs:166`; `crates/desktop/src/filters.rs:181`

### UI-076 [low/unneeded-relist] An unparsed bound keystroke re-lists and clears the selection — CONFIRMED

- Verdict: CONFIRMED
- Note: edit() (filters.rs:78-82) never compares, and set_query calls edited() unconditionally (filters.rs:84-88). That arms the 300 ms pause (controller.rs:301-303), which lists from the top with keep=false. The citation filters.rs:270 is out of range (the file has 191 lines). The mechanism is at filters.rs:133-136.
- Confidence: high
- Trigger: With a min or max field open (no value), the user types text that does not parse ('.', '-', 'lots', or a partial date). The query does not change, but set_bound still calls set_query, which re-arms the 300 ms pause, which lists the folder.
- Consequence: After the pause the grid reloads from the top: the selection is cleared (selection_after with keep false) and the view scrolls to the beginning, although the filter did not change. The user loses their place while typing.
- Sites: `crates/desktop/qml/FilterBar.qml:101`; `crates/desktop/src/filters.rs:136`; `crates/desktop/src/filters.rs:97`; `crates/desktop/src/filters.rs:270`

### UI-077 [low/focus] Keyboard toggle of a chip loses focus and focus is not returned to the grid — CONFIRMED

- Verdict: CONFIRMED
- Note: facet_choices.rs:62-64 resets the whole model when the rows change, which destroys the focused chip delegate. Main.qml:317-320 (showListing) takes no focus action, and nothing in the chip path calls grid.takeFocus.
- Confidence: medium
- Trigger: Tab to a filter chip and press Space or Enter. The toggle makes the facet's rows change, the Repeater resets, and the focused chip is destroyed. showListing does not move focus anywhere.
- Consequence: The keyboard focus falls to the window root, so the grid's arrow and other keys have no receiver until the user clicks the grid. Other controls (sort, descending, applyView) call grid.takeFocus(); this path does not.
- Sites: `crates/desktop/src/facet_choices.rs:58`; `crates/desktop/src/facet_choices.rs:62`; `crates/desktop/qml/Main.qml:317`; `crates/desktop/qml/Main.qml:532`

### UI-078 [low/stale-result] Total reads N of 0 until the first count arrives after start or a view applies — CONFIRMED

- Verdict: CONFIRMED
- Note: total and matching start at 0 (filters.rs:38-39) and change only when a count is applied. updateFacets counts only when the bar is open or a filter is active (Main.qml:146-149), so the 'N of 0' label shows until that count arrives. The interval is brief.
- Confidence: high
- Trigger: The app starts with a filter in the session query (or a saved view with a filter is applied) while the bar is closed. Filters.total and matching start at 0 and are set only when a count is delivered, which is requested only on listingChanged.
- Consequence: The top bar shows 'N of 0' (or the previous folder's total after applyView) for the time between the list showing and the count arriving, which can be visible on a large catalog.
- Sites: `crates/desktop/src/filters.rs:38`; `crates/desktop/qml/TopBar.qml:84`; `crates/desktop/qml/Main.qml:146`

## U23: Filter bounds and name field

Entry: `crates/desktop/qml/FilterBar.qml:101 onTextEdited -> filters.setBound (filters.rs:133); FilterBar.qml:308 onTextEdited -> filters.editName (filters.rs:95)`

### UI-079 [low/double-fire] Bound edit that keeps the same value re-lists and clears selection — CONFIRMED

- Verdict: CONFIRMED
- Note: set_bound and edit (filters.rs:136, 78-82) do not compare values, and set_query calls edited() each time (filters.rs:84-88). The QueryEdited pause therefore re-lists with keep=false after the decimal, even when the parsed value is unchanged.
- Confidence: high
- Trigger: The user types a decimal separator or trailing zero into a numeric bound whose parsed value does not change, for example typing "2.5" in Megapixels min: the "2" becomes "2." (parses to 2.0, unchanged). Each such keystroke still edits the query, and after the 300 ms pause the list is re-listed from the top.
- Consequence: The grid resets to the top of the list and drops the selection and anchor (keep=false, Changed::All, selection_after returns no current item) while the user is only typing a decimal. This happens on every decimal typed into a bound, and the redundant list is re-read from the catalog.
- Sites: `crates/desktop/src/filters.rs:136`; `crates/desktop/src/filters.rs:97`; `crates/desktop/src/filters.rs:104`; `crates/client/src/controller.rs:549`; `crates/client/src/controller.rs:550`; `crates/client/src/grid.rs:442`; `crates/client/src/grid.rs:443`

### UI-080 [low/wrong-message] Failed facet count shows zero totals as if the folder were empty — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-072)
- Note: Duplicate of UI-072: the same unwrap_or_default on a failed facet read (filters.rs:165-166), then apply_facets publishes the zeros (filters.rs:178-182). The cited lines are right; the duplicate is in the count, not a separate defect.
- Confidence: high
- Trigger: The reader fails to produce the facets (catalog read error) while the filter bar counts a folder with a filter active. This happens in count_facets, after listingChanged or the bar opening.
- Consequence: apply_facets sets total and matching to 0 and publishes all chip counts as 0, so the top bar reads like "N of 0 files" and every chip reads 0. The FacetsFailed toast is posted, but the numbers on screen are shown as real results.
- Sites: `crates/desktop/src/filters.rs:162`; `crates/desktop/src/filters.rs:166`; `crates/desktop/src/filters.rs:178`; `crates/desktop/src/filters.rs:181`; `crates/desktop/qml/TopBar.qml:85`; `crates/catalog/src/browse/read/facets.rs:20`

### UI-081 [low/missing-feedback] Unparseable bound text silently drops the bound and widens the list — CONFIRMED

- Verdict: CONFIRMED
- Note: set_bound passes parse_number's None to set_number (filters.rs:136-137), so the bound is open and the edit re-lists. The red text (FilterBar.qml:100) is the only sign. The documented behaviour at filters_bridge.rs:74 was not read.
- Confidence: medium
- Trigger: The user types text that does not parse into a numeric or date bound, for example "5x" or "2024-0" mid-way through a date. set_bound stores None for that end, so the bound is open and the list re-lists after the pause with that limit removed.
- Consequence: The list silently widens (files outside the typed bound now appear) and the counts change. The only signal is the red text colour on the field (FilterBar.qml:100). Nothing says the bound was ignored. This is the documented behaviour (filters_bridge.rs:74), so the gap is the missing feedback, not the open end.
- Sites: `crates/desktop/src/filters.rs:136`; `crates/desktop/src/filters.rs:137`; `crates/desktop/qml/FilterBar.qml:94`; `crates/desktop/qml/FilterBar.qml:100`

## U24: Duplicates and border-marked choosers

Entry: `crates/desktop/qml/FilterBar.qml:228 onActivated -> filters.chooseDuplicates (filters.rs:102); FilterBar.qml:257 onActivated -> filters.chooseBorderMarked (filters.rs:109)`

### UI-082 [medium/stale-result] Border detection turned off leaves the review chooser and filter bar showing a review the list no longer applies — CONFIRMED

- Verdict: CONFIRMED
- Note: Stronger than UI-067: the review choice (border_marked) is dropped by BordersTurnedOff, and nothing reruns the filter sync or reloadQuery (Main.qml:709 and showListing :317-320). The filter count and the Clear state (filters.rs:70-73) therefore stay on the removed choice.
- Confidence: high
- Trigger: A review choice is set (Not marked or Marked not a border), then in the Library panel the user unticks Detect borders. Main.qml:709 calls shell.bordersTurnedOff(), the listing drops border_marked (client listing.rs BordersTurnedOff, features::without_borders) and the grid re-lists unfiltered, but no Filters sync or reloadQuery runs.
- Consequence: The review box still shows the removed choice, the TopBar filter badge and count (filterActive) still count it, Clear stays enabled, and the save-view dialog's description still says the borders review is in force. The user sees a filter as applied while the list is unfiltered by it, and a saved view from that dialog would not match its description. Sort is re-synced on the same event (showListing, Main.qml:318-319), so only the filter bar is stale.
- Sites: `crates/desktop/qml/Main.qml:709`; `crates/desktop/src/shell.rs:374`; `crates/desktop/src/filters.rs:70`; `crates/desktop/src/filters.rs:174`; `crates/desktop/qml/FilterBar.qml:134`; `crates/desktop/qml/Main.qml:512`

### UI-083 [low/focus] Mouse-picked duplicates or review choice leaves focus on the combo, so Up/Down keep changing the filter instead of moving in the grid — CONFIRMED

- Verdict: CONFIRMED
- Note: FilterBar.qml:228 and :261 send the combo's activated to chooseDuplicates or chooseBorderMarked, and neither takes focus back. The keyboard step on a closed ComboBox emits activated (qquickcombobox.cpp:692-696, as in UI-059), so each arrow changes the filter.
- Confidence: high
- Trigger: The user clicks the Duplicates or Review combo and picks an item (or Tabs there). The ComboBox has StrongFocus and keeps focus. FilterBar never calls grid.takeFocus(). Pressing Up or Down then steps the focused closed combo, which emits activated and runs chooseDuplicates or chooseBorderMarked again, re-listing the grid.
- Consequence: A keyboard user who changes a duplicates or review filter cannot move through the grid with the arrow keys. Each arrow press silently changes the filter and re-lists. Sort and the descending toggle both call grid.takeFocus() after their change (Main.qml:531-536), so the filter bar is inconsistent with them.
- Sites: `crates/desktop/qml/FilterBar.qml:228`; `crates/desktop/qml/FilterBar.qml:261`; `crates/desktop/qml/Main.qml:532`; `crates/desktop/qml/Main.qml:531`

### UI-084 [low/wrong-message] A failed count shows zeros as real counts, with the total reading 'of 0' — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-072)
- Note: Duplicate of UI-072: the same failure path (filters.rs:165-166, 178-182), with the TopBar reading "N of 0" (TopBar.qml:85).
- Confidence: medium
- Trigger: The facet count read fails (for example a catalog read error) while the filter bar is open or a filter is active. count_facets posts FacetsFailed, then unwraps to Facets::default() and apply_facets publishes it as valid.
- Consequence: The choice chips show 0 for every type, extension and borders value, and the bar's total is 0, so the TopBar reads 'N of 0' next to the real file count. The error toast says the counts could not be read, but the numbers are presented as results.
- Sites: `crates/desktop/src/filters.rs:162`; `crates/desktop/src/filters.rs:163`; `crates/desktop/src/filters.rs:166`; `crates/desktop/src/filters.rs:178`; `crates/desktop/qml/TopBar.qml:85`

### UI-085 [low/missing-feedback] Duplicates chooser answers from partial data when duplicate detection is off, with no notice (the borders chooser refuses and says why) — CONFIRMED

- Verdict: CONFIRMED
- Note: choose_duplicates (filters.rs:102-107) has no feature check, and the Hashes pass returns early when duplicates is off (passes.rs:160-161). The query's answer for duplicates was not traced (crates/query lib.rs:289 was not read).
- Confidence: medium
- Trigger: Library panel: Find duplicates in the background is unticked (prefs detect_duplicates false). The user picks Has duplicates or No duplicates. chooseDuplicates has no feature check, and the pairing and hash passes stop (passes.rs:160-161, 190-191), so new images are not paired and the query (crates/query/src/lib.rs:289-292) gives a partial answer.
- Consequence: Has duplicates lists only the pairs found before detection went off, with no caveat. The borders chooser, by contrast, refuses with a popup when borders are off (FilterBar.qml:249, Main.qml:577). The tooltip states the behaviour, but the chooser gives no indication when it is in effect.
- Sites: `crates/desktop/qml/FilterBar.qml:225`; `crates/desktop/src/filters.rs:102`; `crates/desktop/qml/Main.qml:711`; `crates/library/src/passes.rs:160`

## U25: Clear all filters

Entry: `crates/desktop/qml/FilterBar.qml:319 onClicked -> filters.clear (filters_bridge.rs:86, filters.rs:146)`

### UI-086 [low/focus] Clear leaves no control focused, so grid keys stop — CONFIRMED

- Verdict: CONFIRMED
- Note: FilterBar.qml:317 disables the Clear button when filters.active reaches 0. Qt clears focus in the scope when a focused item is disabled (qquickitem.cpp:6835-6838), and nothing calls takeFocus afterwards (Main.qml:574 only calls queryEdited).
- Confidence: high
- Trigger: With a filter set and the filter bar open, the user clicks Clear (or Tabs to it and presses Space). The press gives the Button focus (qquickabstractbutton.cpp:1196-1197). Clear's sync() sets filters.active to 0 in the same call, so Button.enabled goes false. Qt's QQuickItem::setEnabled passes the focus scope to setEffectiveEnableRecur, which calls clearFocusInScope when a focused item is disabled (qquickitem.cpp:6816-6843). Nothing takes focus afterwards: Main.qml:574 only calls shell.queryEdited().
- Consequence: After Clear, no item in the window has active focus. Arrow keys and the grid's own keys, which GalleryGrid.onKeyPressed sends to shell.keyPressed, do nothing until the user clicks a cell. applyView, showAsked, sortBy and descendingToggled all return focus with grid.takeFocus(); Clear does not, so its focus behaviour differs from them.
- Sites: `crates/desktop/qml/FilterBar.qml:317`; `crates/desktop/qml/FilterBar.qml:319`; `crates/desktop/src/filters.rs:148`; `crates/desktop/qml/Main.qml:574`; `crates/desktop/qml/Main.qml:532`

### UI-087 [low/stale-result] Clear shows pre-clear counts and an early empty-state text until the list is in — CONFIRMED

- Verdict: CONFIRMED
- Note: clear() (filters.rs:146-151) does not touch the facet generation, so a count started before the clear is still accepted by apply_facets (filters.rs:175). sync publishes the old facets with the cleared query (filters.rs:66, 70-73). The state corrects when the list arrives.
- Confidence: medium
- Trigger: A filter is set and a count is in flight or already shown. The user clicks Clear. Within the 300 ms ApplyQuery pause and the count's read, the bar shows the new query with the old facets' counts (filters.rs:66 publishes the pre-clear facets with the cleared query). clear() does not bump the count generation, so a count started before the Clear that is still in flight is accepted (filters.rs:175 check passes) and replaces total and matching with old-query values. The grid's empty-state text is picked from `filtering`, which turns false at once.
- Consequence: For about a third of a second up to the list's arrival, the choice chips and the counts describe the previous query while the bar shows the cleared one. If the previous list was empty because of the filter, the grid says "No images" rather than "Nothing matches". All of it corrects itself when the new list and its counts arrive, and the window is not left in a wrong state.
- Sites: `crates/desktop/src/filters.rs:66`; `crates/desktop/src/filters.rs:175`; `crates/desktop/src/filters.rs:156`; `crates/desktop/qml/Main.qml:629`; `crates/desktop/qml/GalleryGrid.qml:98`

### UI-088 [low/wrong-message] Clear button beside the name field clears every filter — CONFIRMED

- Verdict: CONFIRMED
- Note: FilterBar.qml:315-320 binds Clear to filters.clear(), which resets the whole query (filters.rs:146-151). The button has no ToolTip, while the save-crops and cut buttons do.
- Confidence: medium
- Trigger: The user sees "Name [contains…] [Clear]" and clicks Clear meaning to drop the name text. The click calls filters.clear(), which resets every facet (Query::default()), and the chip choices, sizes, dates, duplicates and review are lost with it. There is no tooltip, and no confirmation.
- Consequence: The user loses all filter choices in one click. The label and placement say only the name is cleared, so the result does not match what the button seems to promise. Other filter-bar buttons carry tooltips (the save-crops and cut buttons, FilterBar.qml:278-280, 290-292); this one has none.
- Sites: `crates/desktop/qml/FilterBar.qml:315`; `crates/desktop/qml/FilterBar.qml:316`; `crates/desktop/qml/FilterBar.qml:319`; `crates/client/locales/en/ui.ftl:45`

## U26: Open the library panel

Entry: `crates/desktop/qml/TopBar.qml:135 onClicked -> libraryRequested -> Main.qml:541 libraryOpen = true (Main.qml:56 starts libraryLoader)`

### UI-089 [medium/focus] Dialog close hands focus to the grid behind the open library panel — CONFIRMED

- Verdict: CONFIRMED
- Note: removeDialog and discardDialog call grid.takeFocus on close (Main.qml:223, :237), and the library panel stays open. The panel's overlay hands keys to the grid while it is open (ModalOverlay.qml:20-22). shell.key_pressed checks only the viewer (shell.rs:153), so the grid acts and Escape no longer reaches the panel.
- Confidence: high
- Trigger: The user clicks Remove (or Discard) in the library panel, then Cancel or Remove. Both dialogs call grid.takeFocus() on close (Main.qml:223, 237), but the library panel is still open. The user then presses a digit, R, N or Enter, or Escape.
- Consequence: Keys reach the grid under the dimmed panel: digit keys rate the selected photo, R turns it (and writes it when write-back is on), N marks a border, Enter opens the viewer under the panel. Escape no longer closes the panel, which is the only keyboard exit. None of these keys is blocked by the open modal panel, so the user sees changes they did not make to the visible list.
- Sites: `crates/desktop/qml/Main.qml:223`; `crates/desktop/qml/Main.qml:237`; `crates/desktop/qml/GalleryGrid.qml:73`; `crates/desktop/src/shell.rs:153`; `crates/desktop/qml/ModalOverlay.qml:22`

### UI-090 [low/missing-feedback] Empty-library message is centred on the list's content item and half clipped — CONFIRMED

- Verdict: CONFIRMED
- Note: The Label is a child of the WheelListView (LibraryPanel.qml:74-79), so Qt places it in the flickable's content item. centerIn parent then centres it on a zero-height content item, and clip is true (:59). The top half is clipped. Not checked on screen.
- Confidence: medium
- Trigger: The library is empty (no folders), so the panel opens with hasRoots false, which is the first-run case. The Label is a child of WheelListView, so Qt reparents it to the contentItem. centerIn: parent centres it at y=0 of a zero-height content item, and clip: true cuts the top half.
- Consequence: The 'No folders yet.' text is shown half cut off at the top of the list on first use. Logic is otherwise correct, and the text is still readable in its lower half.
- Sites: `crates/desktop/qml/LibraryPanel.qml:59`; `crates/desktop/qml/LibraryPanel.qml:75`; `crates/desktop/qml/LibraryPanel.qml:76`

### UI-091 [low/wrong-message] Folder-list-changed notice is hardcoded English, not catalogued — CONFIRMED

- Verdict: CONFIRMED
- Note: library.rs:282 posts the hardcoded English string "the folder list changed: try again" as the notice's error, with no catalogued text.
- Confidence: high
- Trigger: The user clicks Remove on a row, and the catalog's id/path no longer match the cache (Begun::Changed or NotFound).
- Consequence: The notice body is the raw English sentence 'the folder list changed: try again' under the catalogued title, so it bypasses the localized text table and is not worded like the rest of the notices.
- Sites: `crates/desktop/src/library.rs:282`

### UI-092 [medium/wrong-message] Thumbnail-sweep failure after a removal is reported as 'folder couldn't be removed' — CONFIRMED

- Verdict: CONFIRMED
- Note: coordinator.rs:791-793 posts FolderNotRemoved with a format! body, the English text that goes under the title "couldn't be removed" (text.ftl:280). The removal itself succeeded.
- Confidence: medium
- Trigger: A folder's files are removed by the library's removal job, and sweep_orphans fails afterwards (I/O error on the thumbnail cache). The folder is already gone from the library and its files are deleted.
- Consequence: The inbox shows the title 'The folder couldn't be removed from the library' with the English body 'the thumbnails of removed files weren't dropped: ...', so the user is told a successful removal failed. The body is also uncatalogued.
- Sites: `crates/library/src/coordinator.rs:791`; `crates/library/src/coordinator.rs:792`; `crates/client/locales/en/text.ftl:280`

### UI-093 [low/stale-result] Confirmed removal acts on the row index, not on the folder the dialog named — CONFIRMED

- Verdict: CONFIRMED
- Note: askRemoveFolder records both the row and the folder name (Main.qml:215-216), but confirm calls remove_folder(row) (Main.qml:222), which reads root_ids and root_paths by index (library.rs:266-272). A roots reload changes what the index names. It is latent: nothing reloads roots while the dialog is open.
- Confidence: medium
- Trigger: The user clicks Remove on a row, and the library's roots change before Remove is confirmed (a scan pass reloads roots and the row now maps to a different folder). The dialog still shows the old folder name.
- Consequence: Confirming removes whichever folder now sits at that row, with its files, ratings and thumbnails, and the dialog named a different folder. In-app paths currently do not change roots while the modal is open, so this is latent.
- Sites: `crates/desktop/qml/Main.qml:216`; `crates/desktop/qml/Main.qml:222`; `crates/desktop/src/library.rs:269`

### UI-094 [low/wrong-message] Unreadable catalog shows 'No folders yet' and hides pending removals — CONFIRMED

- Verdict: CONFIRMED
- Note: load_roots applies an empty list on error (library.rs:89-91), and load_removals does the same for pending removals (library.rs:108-109). The panel then shows the empty state, and the notice is posted.
- Confidence: medium
- Trigger: The catalog read for roots (or pending removals) fails when the panel's data is loaded or on a scan pass.
- Consequence: The inbox gets the FoldersUnreadable notice, but the panel's list is emptied, so it shows 'No folders yet.', disables Rescan, and hides the 'Being removed' rows. The folders are still in the library, so the panel says something false.
- Sites: `crates/desktop/src/library.rs:89`; `crates/desktop/src/library.rs:108`

### UI-095 [low/missing-feedback] Rescan does nothing silently when the library is not running — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-010)
- Note: Duplicate of UI-010: app library.rs:431 does nothing when running() is None, and the desktop rescan posts nothing on success (desktop library.rs:288-292).
- Confidence: medium
- Trigger: The library failed to start (LibraryUnavailable posted), and the user clicks Rescan all, which is enabled whenever folders exist.
- Consequence: No scan starts and the panel gives no response. The only notice is the one posted at start.
- Sites: `crates/app/src/library.rs:431`; `crates/desktop/src/library.rs:289`

## U27: Add a folder through the picker

Entry: `crates/desktop/qml/LibraryPanel.qml:153 onClicked addRequested -> Main.qml:699 addPicker.pick(0)`

### UI-096 [medium/unhandled-close] Added folder's scan request is lost when the app quits before it runs — CONFIRMED

- Verdict: CONFIRMED
- Note: add_folder queues the scan only when the job runs (app library.rs:377), and the queue is in memory (coordinator.rs:277-282). I found no scan requested at start (the grep of library.start shows none), so existing roots are not re-queued on the next launch.
- Confidence: high
- Trigger: The user adds a folder (a large one, so its walk takes a while) and closes the window or quits before the scan finishes. Or the job is stopped at exit with the scan still queued.
- Consequence: The folder is recorded in the catalog and listed in the library panel and tree, but no file of it is scanned. The scan request lives only in the in-memory queue (Coordinator reg.scans), and nothing on start re-queues existing roots. The folder stays empty after the next launch until the user presses Rescan all. The user sees a folder with no photos and no error.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\library.rs:377`; `D:\GitHub\photo-gallery\crates\library\src\coordinator.rs:280`; `D:\GitHub\photo-gallery\crates\library\src\runner.rs:95`; `D:\GitHub\photo-gallery\crates\library\src\passes.rs:63`

### UI-097 [low/missing-feedback] The picker's failed message is never shown: addPicker has no onFailed handler — CONFIRMED

- Verdict: CONFIRMED
- Note: addPicker (Main.qml:157-161) has no onFailed handler, while savePicker has one (YandexPanel.qml:125). picker.rs:94-96 posts PickerFailed and emits failed with the rendered message, which is then dropped for the library panel.
- Confidence: medium
- Trigger: The system folder dialog fails (Windows: a CoCreateInstance, SetFolder or Show error), or the in-app dialog cannot list a folder (other systems).
- Consequence: picker.rs builds a catalogued folder-dialog-failed or folder-unreadable message and emits failed(), but Main.qml ignores it. The LibraryPanel shows nothing. The only feedback is the PickerFailed toast on the status bar, which the dimmed LibraryPanel overlay sits over. The rendered message is computed and then discarded.
- Sites: `D:\GitHub\photo-gallery\crates\desktop\qml\Main.qml:159`; `D:\GitHub\photo-gallery\crates\desktop\qml\Main.qml:160`; `D:\GitHub\photo-gallery\crates\desktop\src\picker.rs:96`; `D:\GitHub\photo-gallery\crates\desktop\qml\YandexPanel.qml:125`

### UI-098 [low/wrong-message] GUI add refuses a folder with a kept removal where the CLI finishes the removal — CONFIRMED

- Verdict: CONFIRMED
- Note: roots.rs:73-76 refuses an add while a removal marker overlaps the folder. The GUI calls add_folder, which does not finish the removal (app library.rs:370-380, desktop library.rs:252-262). The CLI calls add_folder_finishing (cli lib.rs:303). The refusal text was not read.
- Confidence: medium
- Trigger: The user removes a library folder while its writes are unfinished (the panel lists it under the kept removals), then adds the same folder again from the picker.
- Consequence: The add is refused with the notice "The folder couldn't be added: ... is still being removed from the library: try again when it is gone". The GUI never finishes the removal, unlike the CLI, so retrying does nothing until the writes finish or the user discards the removal. The message does not say this.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\library.rs:372`; `D:\GitHub\photo-gallery\crates\cli\src\lib.rs:303`; `D:\GitHub\photo-gallery\crates\catalog\src\roots.rs:75`; `D:\GitHub\photo-gallery\crates\desktop\src\library.rs:262`

### UI-099 [low/focus] Focus is not handed back to the library panel after the in-app folder dialog closes — CONFIRMED

- Verdict: CONFIRMED
- Note: addDialog has onVisibleChanged (Main.qml:172) and no onClosed, so the in-app folder dialog's close never calls grid.takeFocus. The other dialogs do (Main.qml:208, :223, :237). Whether focus returns to the panel after the dialog hides in Qt was not checked.
- Confidence: low
- Trigger: On a system without the native dialog, the user clicks Choose or Choose here, or presses Escape or clicks the backdrop, in the in-app folder dialog while the library panel is open.
- Consequence: Only addDialog closes, and its hidden signal is not connected, so the keyboard focus is not returned to LibraryPanel. The other dialogs in Main.qml return focus to the grid on close. Keys such as Escape may then fail to close the library panel until the user clicks it. Not verified against Qt's focus-on-hide behaviour.
- Sites: `D:\GitHub\photo-gallery\crates\desktop\qml\Main.qml:172`; `D:\GitHub\photo-gallery\crates\desktop\qml\Main.qml:208`; `D:\GitHub\photo-gallery\crates\desktop\qml\ModalOverlay.qml:21`

## U28: Remove a folder (with confirmation)

Entry: `crates/desktop/qml/LibraryPanel.qml:71 onClicked removeRequested -> Main.qml:700 askRemoveFolder (Main.qml:214-218)`

### UI-100 [medium/focus] Focus returns to the grid behind the still-open library panel — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-089)
- Note: Duplicate of UI-089: removeDialog.onClosed calls grid.takeFocus (Main.qml:223) with the library panel still open.
- Confidence: high
- Trigger: The user opens Remove on a folder in the library panel, then confirms or cancels the confirmation dialog (any close: Remove, Cancel, Escape, outside click), and the library panel stays open. Focus is then moved to the grid by onClosed.
- Consequence: The library panel remains visible but the keyboard is on the grid behind it. Escape does not close the panel (the panel's Keys.onEscapePressed is on a sibling). Grid keys still act: digits rate the current photo, arrows move the selection, Enter/Space open the viewer, all behind the dimmed panel, because shell.key_pressed gates only on the viewer being open. A keyboard user can only reach the panel's Close by clicking.
- Sites: `crates/desktop/qml/Main.qml:223`; `crates/desktop/qml/GalleryGrid.qml:55`; `crates/desktop/qml/ModalOverlay.qml:22`; `crates/desktop/src/shell.rs:153`

### UI-101 [high/stale-result] Confirmed removal can target a different folder than the one named — CONFIRMED

- Verdict: CONFIRMED
- Note: askRemoveFolder records the row and the name (Main.qml:215-216). remove_folder reads root_ids and root_paths by index (library.rs:266-272) and calls begin_removal with that path. The catalog's expected-path check is then satisfied by the row's current folder, not the one the dialog named (app library.rs:389). Latent, as the finding says.
- Confidence: medium
- Trigger: The dialog opens for row N and names folder X. Before Remove is confirmed, the folder list changes and is re-read: a folder is removed from the library by another process (the CLI 'remove' command calls begin_removal on the same catalog) and an in-app scan finishes (Changes::Scanned -> on_pass -> load_roots). Row N now names folder Y. Confirm sends only the row.
- Consequence: The folder at row N at confirm time is removed, with its images, ratings and analyses, though the dialog showed X. The user confirmed a removal of a different folder. If the row is past the end of the list, nothing happens and no notice is shown. The catalog's expected-folder check is defeated because the Rust side passes the path it just read from the same row.
- Sites: `crates/desktop/qml/Main.qml:216`; `crates/desktop/qml/Main.qml:222`; `crates/desktop/src/library.rs:268`; `crates/desktop/src/library.rs:271`; `crates/app/src/library.rs:389`

### UI-102 [low/wrong-message] List-changed notice body is hard-coded English, not from the text catalog — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-091)
- Note: Duplicate of UI-091: the hardcoded English body at library.rs:282. The NotFound case does not say the folder was removed elsewhere.
- Confidence: high
- Trigger: Remove is confirmed for a folder whose id is no longer in the catalog (Begun::NotFound), or the catalog reports Begun::Changed.
- Consequence: The notice title is catalogued and localised, but the body is a fixed English sentence in lower case, so other locales see English. The wording 'the folder list changed' is also wrong for NotFound, where the folder was removed elsewhere.
- Sites: `crates/desktop/src/library.rs:282`

### UI-103 [low/wrong-message] Failure after the removal is committed shows 'not removed' and leaves the panel stale — CONFIRMED

- Verdict: CONFIRMED
- Note: With no running job, app library.rs:389-400 commits begin_removal and then calls removals::finish on the Qt thread. A failure returns Err, and the desktop Err branch posts FolderNotRemoved and does not reload roots (desktop library.rs:284).
- Confidence: high
- Trigger: The library job is not running (library start failed, so LibraryUnavailable is shown), and Remove is confirmed. remove_folder commits begin_removal, then runs removals::finish synchronously on the Qt thread. finish fails, or only a thumbnail forget fails after the deletion has completed.
- Consequence: The desktop returns Err, so it posts 'couldn't be removed' and does not call load_roots. The panel keeps listing a folder whose marker is already committed, and the user is told it was not removed when it has left the library. The synchronous path also blocks the UI for the whole deletion with no progress or busy state.
- Sites: `crates/app/src/library.rs:396`; `crates/desktop/src/library.rs:284`; `crates/library/src/removals.rs:40`

### UI-104 [low/stale-result] Grid keeps listing the removed folder's images until the removal job ends — PARTIAL

- Verdict: PARTIAL
- Note: The removal's final publish is Scanned (coordinator.rs:847), so the grid refreshes only then. Whether the Begun::Started reload still lists the removed folder's files depends on the listing query, which I did not trace.
- Confidence: medium
- Trigger: The user confirms the removal of a folder with many files. The removal runs as a background job (hide, delete in batches) that can take a long time.
- Consequence: The reload at Begun::Started re-lists the grid while the removed folder's files are still visible. Hiding and deletion then happen with no grid refresh until the job's final Scanned. The user sees images the dialog said were being deleted, and the tree no longer shows their folder.
- Sites: `crates/desktop/src/library.rs:276`; `crates/desktop/src/library.rs:277`; `crates/desktop/qml/Main.qml:129`; `crates/library/src/coordinator.rs:847`

## U29: Discard a pending removal

Entry: `crates/desktop/qml/LibraryPanel.qml:103 onClicked discardRequested -> Main.qml:701 askDiscardRemoval (Main.qml:229)`

### UI-105 [medium/wrong-message] Discard reports failure and keeps the row after the discard committed — CONFIRMED

- Verdict: CONFIRMED
- Note: discard_removal commits the catalog delete (removal.rs:259), then sweep_orphans can fail (app library.rs:423). The desktop Err branch posts FolderNotRemoved and does not reload (desktop library.rs:142-144). Discard on the same row then finds nothing pending (desktop library.rs:139-141) and returns silently.
- Confidence: high
- Trigger: User confirms Discard, and the removal is committed in the catalog, but the thumbnail sweep that runs after the commit fails (thumbnail store unreadable or busy). This is a failure after the commit, so the removal is gone from the catalog.
- Consequence: The user sees 'The folder couldn't be removed from the library' for an action that did happen. The kept row stays listed because the Err branch never reloads roots. Clicking Discard again finds no pending removal and silently does nothing, so the list and the catalog disagree until a later Scanned event.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\library.rs:423`; `D:\GitHub\photo-gallery\crates\catalog\src\removal.rs:259`; `D:\GitHub\photo-gallery\crates\desktop\src\library.rs:143`; `D:\GitHub\photo-gallery\crates\desktop\src\library.rs:144`

### UI-106 [low/wrong-message] Discard failures are worded as 'folder couldn't be removed from the library' — CONFIRMED

- Verdict: CONFIRMED
- Note: The desktop posts FolderNotRemoved for every discard failure (desktop library.rs:134, :144), so the title is "couldn't be removed from the library" (text.ftl:280), while the folder had already left the library.
- Confidence: high
- Trigger: A discard fails in any way: the pending-removals read fails, the removal is refused because a library folder overlaps it, or the discard itself errors.
- Consequence: The user is told the folder could not be removed from the library. The folder already left the library when the removal began, and the failure is about discarding the changes. The message misdescribes the situation.
- Sites: `D:\GitHub\photo-gallery\crates\desktop\src\library.rs:134`; `D:\GitHub\photo-gallery\crates\desktop\src\library.rs:144`; `D:\GitHub\photo-gallery\crates\client\locales\en\text.ftl:280`

### UI-107 [medium/wrong-message] Discard shows success while a temp copy is left on disk — CONFIRMED

- Verdict: CONFIRMED
- Note: discard_removal logs a failed temp unlink as a warning and carries on (app library.rs:420-422), then returns success. The dialog promises the temp copies are deleted (ui.ftl:74). The user gets no notice.
- Confidence: medium
- Trigger: A .pg-write temp of a kept write cannot be unlinked during Discard (a sharing violation from another program, such as antivirus or Explorer, or any other non-NotFound error). The rows are deleted and the removal is committed.
- Consequence: The dialog promised 'their temporary copies are deleted'. The row disappears and no error is shown. The temp, which holds the discarded change, stays in the user's photo folder, and the only record is a log line the user never sees.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\library.rs:420`; `D:\GitHub\photo-gallery\crates\app\src\library.rs:421`; `D:\GitHub\photo-gallery\crates\client\locales\en\ui.ftl:74`; `D:\GitHub\photo-gallery\crates\catalog\src\removal.rs:65`

### UI-108 [low/focus] Closing the discard dialog moves focus to the grid behind the still-open library panel — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-089)
- Note: Duplicate of UI-089 for the discard dialog: discardDialog.onClosed calls grid.takeFocus (Main.qml:237) with the library panel still open.
- Confidence: medium
- Trigger: The user opens Discard from the library panel, then presses Cancel or Discard. The dialog closes and the library panel stays open. The user then presses Escape.
- Consequence: Focus lands on the grid behind the dimmed panel. The panel's ModalOverlay Escape handler stops receiving keys, so Escape does not close the library panel, and the user must click the backdrop or Close. The same pattern exists for RemoveFolderDialog at Main.qml:223.
- Sites: `D:\GitHub\photo-gallery\crates\desktop\qml\Main.qml:237`; `D:\GitHub\photo-gallery\crates\desktop\qml\ModalOverlay.qml:22`; `D:\GitHub\photo-gallery\crates\desktop\qml\Main.qml:697`

### UI-109 [low/wrong-target] Discard finds its removal by lossy display text, so two removals can match — CONFIRMED

- Verdict: CONFIRMED
- Note: library.rs:139 finds the pending removal by comparing rules.display(p) with the folder text, and display is lossy (rules.rs:207-211, documented as lossy). Two removals with the same display text match the first. Latent, as the finding says.
- Confidence: medium
- Trigger: Two pending removals whose paths display identically, because display() is lossy for non-UTF-8 or unpaired-surrogate names. The user clicks Discard on the second row.
- Consequence: find() takes the first match, so the first removal is discarded. The changes of the folder the user did not choose are dropped, and the second stays listed. This is latent: it needs two such paths.
- Sites: `D:\GitHub\photo-gallery\crates\desktop\src\library.rs:139`; `D:\GitHub\photo-gallery\crates\model\src\rules.rs:207`

## U30: Rescan all folders

Entry: `crates/desktop/qml/LibraryPanel.qml:157 onClicked rescanRequested -> Main.qml:702 library.rescan() (library.rs:288)`

### UI-110 [low/double-fire] Second Rescan during a walk re-walks that folder — CONFIRMED

- Verdict: CONFIRMED
- Note: A preempted walk is pushed back to the front of its lane queue (passes.rs:100). A second rescan adds the same path to the lane's queue when it is not already there (coordinator.rs:509-511), and the lane's own queue does not hold the path while it is being walked. The folder is therefore queued twice and walked twice.
- Confidence: high
- Trigger: The user clicks Rescan all again while a scan is walking a library folder. Nothing in the panel shows the job is busy, so a repeat click is likely. The second request is queued, the lane's walk is preempted, and once the walk is resumed the same folder sits in the queue twice.
- Consequence: The folder is walked fully twice in a row, with no change in result. On a large library this costs hours of disk reads, and the lane restarts its progress. The user sees the scan restart from the beginning.
- Sites: `crates/library/src/coordinator.rs:275`; `crates/library/src/coordinator.rs:509`; `crates/library/src/coordinator.rs:513`; `crates/library/src/passes.rs:63`; `crates/library/src/passes.rs:100`; `crates/desktop/qml/LibraryPanel.qml:154`

### UI-111 [low/missing-feedback] Rescan all does nothing silently when the library never started — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-010)
- Note: Duplicate of UI-010 and UI-095: app library.rs:431 does nothing when the job is not running, and the desktop rescan reports nothing on success (desktop library.rs:288-292).
- Confidence: high
- Trigger: The library's start failed at launch (its threads could not start), so the library is never running. The user clicks Rescan all in the library panel. The button is enabled because the folders exist.
- Consequence: Nothing is scanned, and the click gives no message. The only sign is the earlier LibraryUnavailable notice in the inbox, which the user may not connect to this click.
- Sites: `crates/app/src/library.rs:428`; `crates/desktop/src/library.rs:288`; `crates/desktop/qml/Main.qml:702`

### UI-112 [low/missing-feedback] Rescan silently switches the listing to All when the listed folder has no present files left — CONFIRMED

- Verdict: CONFIRMED
- Note: The tree counts only present files (tree.rs:11, 15: missing = 0). After a rescan marks the listed folder's files missing, the folder is not in the tree, so folders.rs:246 gives -2 and Main.qml:292 sends treeLoaded(false). The gate then emits OpenAll with no started check (start.rs:59-63).
- Confidence: medium
- Trigger: The user lists folder F, then F's files are deleted or moved on disk, and Rescan all marks them missing. After the scan the tree no longer has F, because the tree counts only present files. Main.treeLoaded then sends shell.treeLoaded(false).
- Consequence: The start gate sends OpenAll before its started check, so the grid silently changes to All folders with no message. The user loses the folder view they had. The Main.qml comment documents the rule, but nothing tells the user. A failed tree read after a scan has the same effect, with the tree replaced by an empty one.
- Sites: `crates/client/src/controller/start.rs:59`; `crates/desktop/src/folders.rs:246`; `crates/desktop/qml/Main.qml:292`; `crates/catalog/src/browse/read/tree.rs:15`; `crates/desktop/src/folders.rs:181`

## U31: Library settings toggles

Entry: `crates/desktop/qml/LibraryPanel.qml:127 onToggled (write-back), :134 (detect borders), :141 (detect duplicates), :148 (thumbnail cache); signals at LibraryPanel.qml:35-38`

### UI-113 [medium/stale-result] Filter bar keeps the border review and filter count after Detect borders is turned off — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-082)
- Note: Duplicate of UI-082 with a stronger trigger: the review and border-facet choices drop with BordersTurnedOff (listing.rs:70-75), and Filters::sync runs only from set_query, reload and clear (filters.rs:64, 84-93, 146-151).
- Confidence: high
- Trigger: With detection on, the user sets the filter bar's Review box to Marked, not a border (or selects a border chip). They open the Library panel from the top bar, untick Detect borders, and close the panel. Main.qml:709 calls shell.bordersTurnedOff(), and listing.rs:70-75 drops the border review and facets from the listing, so the grid lists all files.
- Consequence: The Review box still reads Marked, not a border, the top bar shows Filters (1) highlighted with the N of M label, and the save-view dialog header prints the stale filter text. The user sees a filter as applied when the grid is unfiltered, and a saved view names a filter it does not contain. The stale values persist until the user edits, clears or reloads the filter bar, because Filters::sync (filters.rs:64) is called only from set_query, reload and clear.
- Sites: `crates/desktop/qml/Main.qml:709`; `crates/desktop/src/shell.rs:375`; `crates/desktop/src/filters.rs:70`; `crates/desktop/qml/FilterBar.qml:22`; `crates/desktop/qml/FilterBar.qml:134`; `crates/desktop/qml/Main.qml:317`; `crates/desktop/qml/Main.qml:793`

### UI-114 [medium/stale-state] Viewer still draws border overlay and frame while border detection is off — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-021)
- Note: Duplicate of UI-021: Main.qml:649 passes win.showBorders without bordersOn, while the grid gate is at Main.qml:627. Viewer.qml:437 and :456 draw the frame and the overlay from viewer.showBorders.
- Confidence: high
- Trigger: With detection on, the user presses B in the viewer, which saves show_borders as true (Command::SaveShowBorders). They close the viewer, open the Library panel, untick Detect borders, and reopen any image in the viewer that has borders found earlier.
- Consequence: The grid hides borders (Main.qml:627 gates on bordersOn), but the viewer shows the magenta frame and the border overlay, because Main.qml:649 passes the ungated win.showBorders and Viewer.qml has no bordersOn input. Pressing B then shows Border detection is off and cannot hide the overlay, so the viewer displays a border state the rest of the app says is disabled.
- Sites: `crates/desktop/qml/Main.qml:627`; `crates/desktop/qml/Main.qml:649`; `crates/desktop/qml/Viewer.qml:437`; `crates/desktop/qml/Viewer.qml:456`; `crates/desktop/src/gallery/items.rs:122`

## U32: Scan lanes: pause a lane, open its failed count

Entry: `crates/desktop/qml/LaneRow.qml:117 onClicked -> pauseRequested (LaneRow.qml:39) and LaneRow.qml:82 onTapped -> failedRequested (LaneRow.qml:41)`

### UI-115 [medium/pause-lost] Lane pause silently dropped when the drive plan re-routes — CONFIRMED

- Verdict: CONFIRMED
- Note: set_lane_paused changes only the lane's control (coordinator.rs:233-237). A lane retired by a re-plan (coordinator.rs:478) takes its pause with it, and spawn_lane creates the replacement with Inner::new, which has no pause state (coordinator.rs:558-561). The plan's keep and retire logic (coordinator.rs:465-478) was read only in part.
- Confidence: high
- Trigger: The user pauses a lane (Pause on its row, which then shows Resume). Within about 15 s, one of its drive's probes overruns its 5 s deadline (the root becomes Place::Unknown) or the drive is briefly unplugged. plan() then collapses the roots into one group, or gives the root no group, and assign_lanes retires the paused lane, typically the higher-numbered one. Its replacement lane is created unpaused.
- Consequence: The paused row disappears with no message and its drive resumes reading and importing. The user's Pause is gone without any notice, and the lane never reports that its pause was dropped. Set-lane-paused returns false and the bool is ignored, so no failure is reported either.
- Sites: `D:/GitHub/photo-gallery/crates/library/src/coordinator.rs:233`; `D:/GitHub/photo-gallery/crates/library/src/coordinator.rs:561`; `D:/GitHub/photo-gallery/crates/library/src/coordinator.rs:478`; `D:/GitHub/photo-gallery/crates/library/src/lanes.rs:35`; `D:/GitHub/photo-gallery/crates/library/src/coordinator.rs:70`; `D:/GitHub/photo-gallery/crates/desktop/src/lane_model.rs:100`

### UI-116 [low/wrong-target] Failed link can open the inbox with no entry expanded — PARTIAL

- Verdict: PARTIAL
- Note: The link is enabled by the pass's bit in notices.rs:173 and LaneRow.qml:37, and the lookup at Main.qml:833-834 then finds no row. Whether unreadable files really leave no ledger row was not traced in the runner.
- Confidence: high
- Trigger: A pass whose only failures are unreadable files, which the runner says but does not record in any ledger. The UnreadableFiles row sets the pass's bit in Notices.passes, so LaneRow's link is enabled. The pass has no ledger entry, so the lookup behind expandRow finds no row. The user clicks the lane's N failed.
- Consequence: The inbox opens with nothing expanded and no message. The link promises the pass's failed-file ledger, and that ledger is not listed. The user has to find the unreadable-files row themselves.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:173`; `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:525`; `D:/GitHub/photo-gallery/crates/desktop/qml/LaneRow.qml:37`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:833`; `D:/GitHub/photo-gallery/crates/desktop/qml/InboxPanel.qml:134`

### UI-117 [low/no-keyboard-path] Failed-count link has no keyboard activation — CONFIRMED

- Verdict: CONFIRMED
- Note: LaneRow.qml:70-83: the failed count is a Label with only a TapHandler (objectName laneFailedTap) and an Accessible.Link role. It takes no focus and handles no key, so the keyboard cannot reach the expanded failed pass. Ctrl+Shift+N (shell.rs:226-229) opens the inbox without the expand.
- Confidence: high
- Trigger: A keyboard or screen-reader user reaches the status bar's lane row. The failed count is a link only through a pointer TapHandler, and it cannot take focus or handle a key, even though its accessible role is Link.
- Consequence: The keyboard path reaches the inbox (Ctrl+Shift+N) but not the expanded failed-files entry the mouse path reaches. The user must find the ledger by hand, so the two paths do not reach the same result.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/LaneRow.qml:70`; `D:/GitHub/photo-gallery/crates/desktop/qml/LaneRow.qml:79`; `D:/GitHub/photo-gallery/crates/desktop/qml/LaneRow.qml:81`; `D:/GitHub/photo-gallery/crates/desktop/src/shell.rs:226`

## U33: Lane progress polling

Entry: `crates/desktop/qml/Main.qml:412-417 Timer (250 ms, running while laneRows.settling) and Main.qml:132 onProgressed`

### UI-118 [low/focus] Failed-files link cannot be reached or activated from the keyboard — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-117)
- Note: Duplicate of UI-117 for the activation half. The citation StatusBar.qml:564 is out of range (the view shows "line out of range"), and the mechanism is at LaneRow.qml:70-83.
- Confidence: high
- Trigger: A lane with a failed count that the inbox lists (failedLinked true). The user tries to open the failed files with Tab or a screen reader's activate action, not the mouse.
- Consequence: The failed count is a Label with only a TapHandler. It is not focusable (Label's default is NoFocus), and it declares Accessible.role Link with no press action, so the link cannot be activated by keyboard or assistive tech. The mouse path opens the inbox with that pass's ledger expanded. The keyboard path can only open the inbox (GlobalShortcuts Inbox), which does not expand the failed pass.
- Sites: `crates/desktop/qml/LaneRow.qml:70`; `crates/desktop/qml/LaneRow.qml:79`; `crates/desktop/qml/StatusBar.qml:564`

### UI-119 [low/stale-result] A held-back count change is shown only at the next file or phase end — CONFIRMED

- Verdict: CONFIRMED
- Note: control.rs:416-424 throttles the progress notice by NOTICE_INTERVAL, and the desktop timer that refreshes the lane rows runs only while laneRows.settling is set (Main.qml:412-417). The comment at control.rs:413-416 that calls it intentional was not read.
- Confidence: medium
- Trigger: A lane's files complete in a burst within 100 ms of the last notice, then one file takes a long time (for example a large RAW decode or a slow import). The bar and count are read on each progress pass and on the settling timer, and the timer runs only while a row is held.
- Consequence: The row keeps showing the count from before the last file in the burst (for example 39 of 40 while 40 are done) until the slow file finishes or the phase ends. The row's bar and count can trail by up to one file's duration. The library's comment documents this as intentional (control.rs:413-416). The desktop side does not poll while a lane is working, so nothing corrects it sooner.
- Sites: `crates/jobs/src/control.rs:420`; `crates/desktop/qml/Main.qml:415`

## U34: Folder picker dialog navigation

Entry: `crates/desktop/qml/FolderPickerDialog.qml:51 onClicked goUp (-> picker.rs:132 up), :78 onClicked openRow (-> picker.rs:125 enter), :84 and :97 onClicked (choose actions), :105 onClicked dismissed (-> cancel)`

### UI-120 [medium/double-fire] Double-click on a folder name enters a second, unintended subfolder — CONFIRMED

- Verdict: CONFIRMED
- Note: FolderPickerDialog.qml:78 opens on click, and picker.rs:181 replaces the names when the folder is shown. The second click of a double-click then lands on the new list's row at the same position. Qt sends a click per press, which I did not check against the ItemDelegate.
- Confidence: medium
- Trigger: In the in-app folder dialog (non-Windows), the user double-clicks a folder name. The first click enters that folder, which replaces the list model (picker.rs:181). The second click of the double-click then lands on a row of the new listing at the same screen position and enters it too.
- Consequence: The dialog shows a folder two levels below the one the user meant to open. If the user then chooses Choose here, the wrong folder is added or used as the crop or Yandex save folder. The user has to click Up to recover.
- Sites: `crates/desktop/qml/FolderPickerDialog.qml:78`; `crates/desktop/src/picker.rs:181`

### UI-121 [low/focus] Keyboard focus is lost after Enter opens a folder — CONFIRMED

- Verdict: CONFIRMED
- Note: overlay.names replaces the model (FolderPickerDialog.qml:66), which destroys the focused ItemDelegate delegate (:74) that held focus.
- Confidence: low
- Trigger: A keyboard user tabs to a folder row's open button and presses Enter. The list model is replaced by show_folder, which destroys the focused delegate.
- Consequence: Focus is not moved to a sensible place in the new listing. The keyboard user has to Tab again from the top, and Escape may not reach the dialog until focus is restored. Mouse use is not affected.
- Sites: `crates/desktop/qml/FolderPickerDialog.qml:66`; `crates/desktop/qml/FolderPickerDialog.qml:74`; `crates/desktop/src/picker.rs:181`

### UI-122 [low/focus] Folder dialog does not return focus to the panel that opened it — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-099)
- Note: Duplicate of UI-099 for the other dialogs: addDialog (Main.qml:162-173), cropDialog (Main.qml:179) and saveDialog (YandexPanel.qml:127) have no onClosed, so none gives focus back to the panel that opened it (ModalOverlay.qml:16 says the owner gives the keys back).
- Confidence: low
- Trigger: The user opens Add folder from the library panel, then chooses a folder or cancels. The library panel stays open, but the dialog's hidden signal is not connected in the three owners, so no focus is given back. Other dialogs in Main do this with onClosed: grid.takeFocus().
- Consequence: After the dialog closes, keys may not reach the still-open library panel, so Escape may not close it until the user clicks inside it. The same applies to the Yandex save dialog, where the Yandex panel stays open.
- Sites: `crates/desktop/qml/Main.qml:162`; `crates/desktop/qml/Main.qml:179`; `crates/desktop/qml/YandexPanel.qml:127`; `crates/desktop/qml/ModalOverlay.qml:16`; `crates/desktop/qml/Main.qml:208`

### UI-123 [low/missing-feedback] In-app folder dialog drops its own unreadable-folder message for add and crop — CONFIRMED

- Verdict: CONFIRMED
- Note: picker.rs:160-166 posts PickerFailed and emits failed with FolderUnreadable. addPicker (Main.qml:157-161) and cropPicker (Main.qml:174-178) have no onFailed, so the in-app dialog's message is dropped, while savePicker shows it (YandexPanel.qml:125).
- Confidence: medium
- Trigger: On a non-Windows build, the user clicks a folder row (or Up) whose folder cannot be read, for example a permission error. The Add folder or Save cropped copies dialog is the one open.
- Consequence: The dialog stays on the previous folder and nothing in it changes, with no in-dialog explanation. The message the picker emits ('Couldn't read the folder') is dropped. The only feedback is the inbox notice 'The folder dialog failed', so a Yandex-save user sees the in-dialog status but an add or crop user does not.
- Sites: `crates/desktop/qml/Main.qml:157`; `crates/desktop/qml/Main.qml:174`; `crates/desktop/qml/YandexPanel.qml:125`; `crates/desktop/src/picker.rs:164`; `crates/desktop/src/picker.rs:172`

## U35: Save cropped copies (dialog)

Entry: `crates/desktop/qml/FilterBar.qml:32 saveCropsRequested (emitted from one of the onClicked handlers at FilterBar.qml:273 or :285, not traced in detail) -> Main.qml:579 askSaveCrops (Main.qml:193) -> cropPicker.pick(1)`

### UI-124 [medium/unhandled-close] Quit mid-pass drops unfinished copies with no notice — CONFIRMED

- Verdict: CONFIRMED
- Note: A stopped crop returns the remaining ids (coordinator.rs:628-629) and they go back to the in-memory queue (coordinator.rs:703), which does not survive exit. Stats.remaining is set at crop lib.rs:149. The Cropped notice is posted only when nothing remains (coordinator.rs:630-631), so a partial pass says nothing.
- Confidence: high
- Trigger: The user clicks Save cropped copies on a list with many bordered files and quits the app (or the window closes into shutdown) before the crop pass finishes. The crop control is stopped with the root; the pipeline stops at the next file and returns Ok with remaining ids.
- Consequence: The copies that were not written are lost. The job's remaining list goes back to an in-memory queue that dies with the process. No Cropped notice is posted, because it is posted only when nothing remains, so nothing in the inbox says the pass was partial. The user sees neither the copies nor an explanation.
- Sites: `crates/library/src/coordinator.rs:628`; `crates/library/src/coordinator.rs:703`; `crates/library/src/coordinator.rs:106`; `crates/crop/src/lib.rs:149`

### UI-125 [low/data-loss] Failed tagging deletes a copy saved earlier to the same path — CONFIRMED

- Verdict: CONFIRMED
- Note: publish renames the new copy over the old one (crop lib.rs:382, 401). When tag_copy then fails, lib.rs:125-128 removes that output, so the earlier copy is gone with no record. This is the same mechanism as SEC-078 in the security review.
- Confidence: high
- Trigger: The user saves cropped copies twice into the same folder. The second run writes a TIFF or compressed-BMP copy (metadata false) over the earlier copy, then ExifTool tag_copy fails.
- Consequence: publish has already renamed the new untagged bytes over the earlier good copy, and the failure path then deletes that output. The previous copy is gone, and the notice reports the file as failed. The original is untouched, but a copy from an earlier successful run is lost.
- Sites: `crates/crop/src/lib.rs:125`; `crates/crop/src/lib.rs:128`; `crates/crop/src/lib.rs:382`

### UI-126 [low/missing-feedback] Crop folder picker's failure message is never shown — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-123)
- Note: Duplicate of UI-123: cropPicker (Main.qml:174-178) has no onFailed, so the failed(purpose, message) from picker.rs:94-96 is dropped for the crop dialog.
- Confidence: high
- Trigger: Save cropped copies is clicked and the folder dialog fails: the in-app dialog cannot list the folder (for example access denied), or the Windows dialog returns an error.
- Consequence: The picker's failed(purpose, message) signal carries the rendered message (FolderUnreadable or FolderDialogFailed), but Main.qml never connects onFailed for cropPicker or addPicker. The in-app dialog silently keeps its old folder or does not open. The only feedback is the inbox PickerFailed notice, and whether that is shown as a toast was not traced.
- Sites: `crates/desktop/qml/Main.qml:174`; `crates/desktop/src/picker.rs:95`; `crates/desktop/src/picker.rs:164`; `crates/desktop/qml/YandexPanel.qml:125`

### UI-127 [low/missing-feedback] Folder chosen, but nothing is saved and nothing says so — CONFIRMED

- Verdict: CONFIRMED
- Note: saveCroppedCopiesPicked returns n and queues nothing when borders_found is empty (edits.rs:169-174). In the not-started case, app library.rs:349-352 does nothing when the library is not running. Main.qml:176 drops the return value, so the user gets no message in either case.
- Confidence: medium
- Trigger: (a) The user chooses a folder while no file in the list has Borders::Found any more, so borders_found() is empty; or (b) the library has not started yet (the crop button is usable in the short window after the list shows but before startHooks calls library.start()).
- Consequence: saveCroppedCopiesPicked returns 0 and QML ignores the return value, so no job is queued and no message appears. In case (b) the save_crops call is a silent no-op, and the user sees nothing happen after choosing a folder. The nothing-to-crop popup is shown only at click time.
- Sites: `crates/desktop/src/gallery/edits.rs:171`; `crates/desktop/qml/Main.qml:176`; `crates/app/src/library.rs:349`

### UI-128 [low/wrong-message] "Had nothing to crop" also counts changed and missing files — PARTIAL

- Verdict: PARTIAL
- Note: The wording is confirmed: the notice says "{skipped} had nothing to crop" (text.ftl:209), and crop lib.rs:137 counts every Skipped as skipped. Whether Written::Skipped covers files that changed or went missing (crop lib.rs:119) was not traced.
- Confidence: high
- Trigger: Save cropped copies runs over a list where some bordered files changed on disk since their borders were found (size or mtime differ), or were removed from the library.
- Consequence: The completion notice says these files "had nothing to crop". They did have borders and were not copied because they changed or went missing, so the wording misstates why they were not saved.
- Sites: `crates/client/locales/en/text.ftl:209`; `crates/crop/src/lib.rs:119`; `crates/crop/src/lib.rs:137`

### UI-129 [low/focus] Focus is not returned to the grid after the crop folder dialog closes — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-099)
- Note: Duplicate of UI-099 and UI-122 for the crop dialog: cropDialog (Main.qml:179-184) has no onClosed, and the other dialogs (Main.qml:208) call grid.takeFocus.
- Confidence: low
- Trigger: The user picks or cancels a folder in the in-app crop folder dialog (non-Windows, or the app's own picker) opened from the filter bar's Save cropped copies button.
- Consequence: Keyboard focus is not handed back to the grid, unlike the cut, remove and discard dialogs, which call grid.takeFocus() when closed. Arrow keys and shortcuts may stop reaching the grid until the user clicks it. This was not confirmed against Qt's focus-on-hide behaviour.
- Sites: `crates/desktop/qml/Main.qml:179`; `crates/desktop/qml/Main.qml:208`; `crates/desktop/qml/ModalOverlay.qml:21`

## U36: Cut borders from originals (dialog)

Entry: `crates/desktop/qml/FilterBar.qml:39 cutBordersRequested (handler at FilterBar.qml:273 or :285) -> Main.qml:580 askCutBorders (Main.qml:198) -> cutDialog.open with count from gallery_bridge.rs:172 cut_borders_count`

### UI-130 [medium/wrong-message] Thumbnail-forget error reported as a failed cut — CONFIRMED

- Verdict: CONFIRMED
- Note: cut_done posts a CutFailed for each forget error (passes.rs:48-50). The BordersCut summary is posted separately when the cut finishes (coordinator.rs:656-665), so the user sees a failed-cut entry beside a success for the same originals.
- Confidence: high
- Trigger: A cut batch rewrites the originals, but the thumbnail cache refuses to forget one or more cut files (store error during forget).
- Consequence: The inbox shows Cutting the borders failed once per failing file, next to Borders cut from the originals for the same job. The user is told the cut failed when the originals were rewritten, and the thumbnail error is not named as such.
- Sites: `crates/library/src/passes.rs:48`; `crates/library/src/passes.rs:49`

### UI-131 [medium/missing-feedback] Cut button and dialog stay live and show stale count while cut runs — PARTIAL

- Verdict: PARTIAL
- Note: cut_done publishes Imported at the end of each pass result (passes.rs:51, coordinator.rs:656). Whether pg_crop::cut returns once per batch or once per chunk was not traced, so "once at the end of the whole batch" is not settled.
- Confidence: high
- Trigger: The user confirms a cut of many files, then reopens Cut borders while the job is still running. Grid entries are only re-listed by Imported, which cut_done publishes once at the end of the whole batch.
- Consequence: The filter bar still shows the old bordered count and the grid still shows uncut thumbnails until the batch ends. The reopened dialog reports the full old N, though fewer remain. Confirming queues a second job; catalog crop_source skips already-cut files, so nothing is double-written, but the second notice says they had nothing to cut. The button has no busy or disabled state, and the only progress is the status bar crop row.
- Sites: `crates/desktop/qml/FilterBar.qml:287`; `crates/library/src/coordinator.rs:656`; `crates/crop/src/lib.rs:259`; `crates/library/src/passes.rs:51`; `crates/library/src/shared.rs:188`

### UI-132 [medium/unhandled-close] App quit mid-cut loses the summary and the queued remainder — CONFIRMED

- Verdict: CONFIRMED
- Note: A stopped cut returns Done::Stopped(Job::Cut(remaining)) (coordinator.rs:667), which goes back to the in-memory queue (coordinator.rs:703-705). The BordersCut summary is posted only when nothing remains (coordinator.rs:657-664), so an interrupted cut is silent. Same pattern as UI-124.
- Confidence: high
- Trigger: The user confirms a cut of many originals and closes the app before it finishes. Lanes::stop asks the crop thread to stop.
- Consequence: Originals already rewritten get no Borders cut summary: no saved or failed count and no failure list in the inbox. The rest of the confirmed cut is dropped silently, because the queue lives only in memory and is not resumed at the next start. The user is never told the cut was interrupted.
- Sites: `crates/library/src/coordinator.rs:657`; `crates/library/src/coordinator.rs:667`; `crates/library/src/coordinator.rs:108`; `crates/library/src/coordinator.rs:350`

### UI-133 [low/stale-result] Dialog count is captured once and the confirm result is ignored — CONFIRMED

- Verdict: CONFIRMED
- Note: askCutBorders captures the count once (Main.qml:199), and cutBordersConfirmed recomputes the set (edits.rs:184-191). QML ignores its return value (Main.qml:207), so a different count is neither shown nor reported.
- Confidence: medium
- Trigger: The grid is re-listed while the cut dialog is open (for example a cut or import finishing in the background, which publishes Imported). The question shows the old count, and cutBordersConfirmed recomputes the set at confirm time.
- Consequence: The question says N files but the job cuts M, with no message. If M is 0 at confirm, the dialog closes and nothing happens, and nothing is said, because the return value is discarded.
- Sites: `crates/desktop/qml/Main.qml:199`; `crates/desktop/qml/Main.qml:207`; `crates/desktop/src/gallery/edits.rs:187`; `crates/desktop/qml/CutBordersDialog.qml:42`

### UI-134 [low/stale-result] Incomplete write leaves the thumbnail and grid entry stale — CONFIRMED

- Verdict: CONFIRMED
- Note: An Incomplete replace returns an error (crop lib.rs:315), so the file is not pushed to stats.cut (crop lib.rs:257). cut_done then forgets only the files in s.cut (coordinator.rs:656), and this file's thumbnail stays stale.
- Confidence: medium
- Trigger: A write into an original fails part-way (for example disk full during the copy). replace returns Incomplete, so the file is absent from stats.cut.
- Consequence: The notice correctly says the file may be incomplete and names the temp file. But cut_done never forgets that file's thumbnail, so the grid keeps the pre-cut thumbnail and still counts the file as bordered until the next full re-list.
- Sites: `crates/crop/src/lib.rs:315`; `crates/crop/src/lib.rs:257`; `crates/library/src/coordinator.rs:656`

## U37: Popups for refused border actions

Entry: `crates/desktop/qml/Main.qml:527 (sort refused), :577 (FilterBar bordersBlocked), :578 (nothingToCrop), :664 and :668 (viewer B and N) -> featurePopup.show; Main.qml:751-752 Connections on shell.bordersRefused / optionsRefused (shell_bridge.rs:75, :80)`

### UI-135 [medium/wrong-state] Viewer keeps border overlay after detection is turned off — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-021)
- Note: Duplicate of UI-021 and UI-114: Main.qml:649 passes win.showBorders without the bordersOn gate, and Viewer.qml:437 and :456 draw from it. The B refusal is at Main.qml:664-665.
- Confidence: high
- Trigger: Borders are shown (B on) in the session, then the user turns off Detect borders in the Library panel and opens an image that has stored borders. Shell show_borders stays true because bordersTurnedOff only changes the listing.
- Consequence: The viewer keeps drawing the border overlay and the magenta frame even though the grid hides them and the window says border detection is off. Pressing B in the viewer is refused with the 'Border detection is off. Turn it on in the Library panel.' popup, so the user cannot hide the overlay without re-enabling detection. The message gives the wrong advice for the user's intent.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:649`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:627`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:437`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:456`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:664`

### UI-136 [medium/stale-result] Filter bar keeps border review and filter count after detection is turned off — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-067)
- Note: Duplicate of UI-067, UI-082 and UI-113: BordersTurnedOff drops the review and facets (listing.rs:70-75), and the filter bar's sync runs only from set_query, reload and clear (filters.rs:64, 84-93).
- Confidence: high
- Trigger: A border review choice (Marked not a border, which needs detection on) is in the query. The user then turns off Detect borders in the Library panel. The listing drops the border review and facets (listing.rs:70-74), but the filter bar is never resynced.
- Consequence: The review box keeps showing 'Marked not a border' and the Filters button and Clear state keep counting a filter the list no longer applies. The user sees a filter as active that is not in force. The stale state persists until the next filter edit, which is when sync runs.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/filters.rs:70`; `D:/GitHub/photo-gallery/crates/desktop/src/shell.rs:375`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:709`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:318`; `D:/GitHub/photo-gallery/crates/desktop/qml/FilterBar.qml:256`

### UI-137 [medium/stale-result] Viewer N button stays checked after a refused mark — CONFIRMED

- Verdict: CONFIRMED
- Note: RatingBar.qml:48-59 is a checkable ToolButton whose checked is bound to bar.borderMarked. The click toggles checked, the handler only asks (Main.qml:666-669 shows the popup), and nothing resets the binding. The button stays checked while the image is unmarked.
- Confidence: medium
- Trigger: Border detection is off, the viewer shows an image with stored borders (N button visible), and the user clicks the struck-out N button with the mouse.
- Consequence: The button toggles its checked state on the click, so it shows 'marked as not a border' while the image is not marked. The popup says detection is off, but the button still shows the toggled state. The keyboard N path does not show this, so the mouse and key paths differ.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/RatingBar.qml:50`; `D:/GitHub/photo-gallery/crates/desktop/qml/RatingBar.qml:51`; `D:/GitHub/photo-gallery/crates/desktop/qml/RatingBar.qml:59`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:666`

### UI-138 [low/missing-feedback] --borders option silently dropped when detection is off — CONFIRMED

- Verdict: CONFIRMED
- Note: controller.rs:126 drops --borders when features.borders is false, and the start refusals (controller.rs:139-142) cover only the query, sort and view, not show_borders. So nothing is reported.
- Confidence: medium
- Trigger: The app is started with --borders=1 (or --borders with a true value) while Detect borders is off in prefs.
- Consequence: The borders stay hidden with no message. Start popups are shown for the query, sort and saved view options, but not for this one, although the code comments say the options' refusals are said.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/controller.rs:126`; `D:/GitHub/photo-gallery/crates/desktop/src/controller.rs:142`

## U38: Rate a photo in the viewer (star bar, keys)

Entry: `crates/desktop/qml/RatingBar.qml:83 onClicked -> rateAsked (RatingBar.qml:22) -> Hud.qml:41 rateAsked -> Viewer.qml:507 onRateAsked; keys Viewer.qml:267-276 (Rate action, Viewer.qml:276)`

### UI-139 [high/focus] Space after clicking a star rates the next photo instead of stepping — CONFIRMED

- Verdict: CONFIRMED
- Note: Qt's ToolButton accepts Space as a press key (qplatformtheme.cpp:700) and takes StrongFocus (qquickabstractbutton.cpp:109). rate() (Viewer.qml:183-188) does not take focus, and the star's onClicked (RatingBar.qml:83) runs the rate. The next Space goes to the focused button before the viewer's key handler (Viewer.qml:267).
- Confidence: high
- Trigger: The user clicks any star in the viewer's rating bar. The mouse press gives the star ToolButton keyboard focus, and rate() never takes focus back. The user then presses Space to skip to the next photo. The focused button accepts Space (Windows ButtonPressKeys = Space, Select) and fires onClicked, which rates the photo now shown with the clicked star and moves on. Each further Space repeats this on the next photo.
- Consequence: Photos the user meant to skip are rated with the last-clicked star, silently overwriting their existing ratings in the catalog and in the files. The hint line says Space steps forward, so the user sees a wrong result as correct.
- Sites: `crates/desktop/qml/RatingBar.qml:83`; `crates/desktop/qml/Viewer.qml:183`; `crates/desktop/qml/Viewer.qml:161`; `crates/desktop/qml/Viewer.qml:275`; `crates/desktop/qml/RatingBar.qml:70`

### UI-140 [high/double-fire] Holding a rating key or Up/Down rates and skips every photo it passes — CONFIRMED

- Verdict: CONFIRMED
- Note: Viewer.qml:267-276 handles every press without checking isAutoRepeat, and keymap viewer_of (keymap.rs) takes no repeat flag. Each repeat calls rate, which writes and steps (Viewer.qml:186-187).
- Confidence: high
- Trigger: In the viewer, the user holds 0-5, Up or Down for about a second. The OS auto-repeat sends repeated KeyPress events (isAutoRepeat true). Keys.onPressed does not check isAutoRepeat, so each repeat calls rate(arg), which writes a rating and steps to the next photo.
- Consequence: Photos the user never looked at are given the held rating, overwriting their earlier ratings, which are written into the files. The viewer races through the list, and the user sees the wrong rating on photos as if chosen.
- Sites: `crates/desktop/qml/Viewer.qml:267`; `crates/desktop/qml/Viewer.qml:276`; `crates/client/src/keys.rs:129`; `crates/desktop/qml/Viewer.qml:187`

### UI-141 [medium/double-fire] Double-clicking a star rates two photos — CONFIRMED

- Verdict: CONFIRMED
- Note: rate() calls rated and then step(1) synchronously (Viewer.qml:186-187), so the viewer's index moves at once and the bar is over the next photo. The second click of a double-click is a click on that star, which rates it. No guard exists in these lines.
- Confidence: high
- Trigger: The user double-clicks one star in the bar. The first click rates photo A with that star and steps to B synchronously (the viewer's index moves at once). The second click lands on the same bar position over B and rates B with the same star. No guard or pause exists.
- Consequence: Photo B gets a rating the user did not see and did not choose, overwriting its existing rating in the catalog and the file.
- Sites: `crates/desktop/qml/RatingBar.qml:83`; `crates/desktop/qml/Viewer.qml:186`; `crates/desktop/qml/Viewer.qml:187`; `crates/desktop/qml/Main.qml:651`

### UI-142 [medium/wrong-message] Rating popup shows success before the write, and still shows it when the write fails — CONFIRMED

- Verdict: CONFIRMED
- Note: Viewer.qml:521 shows the popup on rated, before the write. The write runs on the worker (edits.rs:108-112). On failure the model is not updated (rating_written is not called) and the error is posted by edit_outcome (app.rs:36-38).
- Confidence: medium
- Trigger: Any rating (a star click or a key) where the catalog or file write fails on the writes' thread (edits.rs:109 returns Err, for example a locked or unwritable file).
- Consequence: The popup confirms the new star over the next photo for about half a second, before the write has run. On failure the item keeps its old rating (rating_written is never called) and the only sign is the inbox notice 'A rating couldn't be saved', which shows after the viewer has already moved on. The user sees success, then an error elsewhere.
- Sites: `crates/desktop/qml/Viewer.qml:521`; `crates/desktop/qml/RatingPopup.qml:16`; `crates/desktop/src/gallery/edits.rs:110`; `crates/desktop/src/app.rs:37`

## U39: Turn a photo in the viewer

Entry: `crates/desktop/qml/Viewer.qml:277-278 keys TurnForward and TurnBack -> turn(+1/-1) (Viewer.qml:190); RatingBar.qml:66 and :91 turnAsked -> Hud.qml:42 -> Viewer.qml:508`

### UI-143 [high/shadowed-key] Focused RatingBar button takes Space from the viewer — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-139)
- Note: Duplicate of UI-139 for the turn buttons (RatingBar.qml:66, :91): both are ToolButtons that take focus on click and accept Space, and Viewer.turn (Viewer.qml:190-193) does not take focus back either.
- Confidence: high
- Trigger: Click a RatingBar ToolButton with the mouse (a star, ⟲ or ⟳), then press Space while the viewer is open. The click gives the button keyboard focus and it keeps it, since the viewer does not reclaim focus after a bar action. Space is a button press key, so the button consumes it and the viewer's Space (step forward) never runs.
- Consequence: After ⟳ or ⟲, Space turns the same photo again instead of stepping on. After a star click, Space re-clicks that star on the next photo: Viewer.rate rates the photo now shown (written to its file through write-back) and steps past it, so a photo the user never looked at is rated and skipped. Enter, Escape, arrows and R still reach the viewer, so the keys behave inconsistently.
- Sites: `crates/desktop/qml/RatingBar.qml:83`; `crates/desktop/qml/RatingBar.qml:66`; `crates/desktop/qml/RatingBar.qml:91`; `crates/desktop/qml/Viewer.qml:275`; `crates/desktop/qml/YandexPreview.qml:152`; `C:/Qt/6.12.0/Src/qtdeclarative/src/quicktemplates/qquickabstractbutton.cpp:109`; `C:/Qt/6.12.0/Src/qtbase/src/gui/kernel/qplatformtheme.cpp:700`

### UI-144 [low/missing-feedback] Turn shows nothing until the catalog write lock is free — CONFIRMED

- Verdict: CONFIRMED
- Note: Viewer.turn sends turned and resets the view at once (Viewer.qml:190-193). The rotation is queued on the worker (edits.rs:27-34) and its result is applied only after turn_queued. The 5 s wait is the catalog's busy timeout, which I did not read.
- Confidence: high
- Trigger: Press R or click ⟲/⟳ while another connection holds the catalog write lock (an import flush, a scan, or the writer's own transaction). The queue write waits up to 5 s on the window-writes thread, and the picture and the 'turning…' label change only after that write succeeds.
- Consequence: For up to 5 s the photo does not change and no 'turning…' label appears, although the zoom has already reset, so the press looks ignored and may be repeated. If the lock stays busy past the timeout, the turn is dropped; only an inbox notice ('A turn couldn't be saved') says so.
- Sites: `crates/desktop/src/gallery/edits.rs:31`; `crates/desktop/qml/Viewer.qml:190`; `crates/app/src/library.rs:438`; `crates/catalog/src/core.rs:319`

### UI-145 [low/modifier-ignored] Ctrl+R and Ctrl+Shift+R turn the photo — CONFIRMED

- Verdict: CONFIRMED
- Note: keys.rs:115 (grid R) and :132 (viewer R) do not check mods.ctrl, so Ctrl+R and Ctrl+Shift+R turn the photo. Only F checks !ctrl (keys.rs:116).
- Confidence: high
- Trigger: Press Ctrl+R (a common reload habit) with a photo open in the viewer, or with a grid item selected.
- Consequence: The photo is rotated, and the rotation is queued for write-back into the file, though no turn was asked for. The F key does check !ctrl, so the map is inconsistent.
- Sites: `crates/client/src/keys.rs:132`; `crates/client/src/keys.rs:115`; `crates/client/src/keys.rs:116`

### UI-146 [low/stale-result] A dropped turn leaves the grid showing it as pending — PARTIAL

- Verdict: PARTIAL
- Note: writeback lib.rs:435-439 drops the job and counts it as a lost edit. The missing Change::Turned on that path was not checked, so the pending mark may or may not stay.
- Confidence: medium
- Trigger: A turn is queued for a file that is missing or no longer listed when the writer picks the job, so the job takes the drop_gone_write path.
- Consequence: The writer deletes the queue row and posts 'write-dropped', but it publishes no Change::Turned, so the grid entry keeps pending_turns. The HUD shows 'turning…' and the image is requested turned until the list is read again or another turn is written. The inbox notice is the only sign.
- Sites: `crates/writeback/src/lib.rs:435`; `crates/writeback/src/lib.rs:189`; `crates/desktop/src/library.rs:231`; `crates/client/src/grid.rs:398`

### UI-147 [low/stale-result] A turn can be counted twice if a refresh read lands between the catalog write and turn_queued — CONFIRMED

- Verdict: CONFIRMED
- Note: edits.rs:44 adds the turn to pending_turns from the model thread, and turned_request reads that state. refresh_turned applies only when the revision is unchanged (grid.rs:405-408). The interleaving with turn_queued and whether edit() bumps the revision were not fully traced. Low severity, as the finding says.
- Confidence: low
- Trigger: Press R while a FilesChanged refresh is reading the same pending file. The read is taken after the turn is committed to the catalog, and its result is applied before turn_queued runs on the model thread.
- Consequence: The entry's pending_turns counts the new turn twice, so the HUD and the image show an extra quarter turn. It self-corrects when the writer writes the queued turns and FilesChanged re-reads the file. The window is very narrow unless the worker thread is preempted between the commit and the queue call.
- Sites: `crates/desktop/src/gallery/edits.rs:30`; `crates/desktop/src/gallery/edits.rs:44`; `crates/client/src/grid.rs:405`

## U40: Mark or unmark a border (viewer N key, HUD)

Entry: `crates/desktop/qml/Viewer.qml:284 MarkBorder key -> Viewer.qml:114 borderMarkToggled; RatingBar.qml:59 borderMarkAsked -> Hud.qml:43 -> Viewer.qml:509`

### UI-148 [medium/double-fire] Two quick border-mark toggles collapse into one while the first write is pending — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-036)
- Note: Duplicate of UI-036 extended to the viewer: Viewer.qml:284 sends !borderMarked from the viewer's own viewed value, which is also stale until the write lands. Both presses ask for marked = true, and edits.rs:137 drops the second.
- Confidence: high
- Trigger: Press N (grid or viewer) on an unmarked photo with borders found, then press N again before the first write lands. Writes wait for the catalog write lock during an import (worker.rs header), so the window can last seconds and the overlay gives no feedback in that time. The grid toggle reads the stored mark, which is unchanged until the write. The viewer sends the value from its stale viewedBorderMarked. Both send marked=true, and set_border_marked drops the second request as equal to the queued target.
- Consequence: An even number of presses inside the window leaves the photo marked when the user meant to unmark it. The result disagrees with the number of presses, and the user sees no change until the write lands, so nothing tells them to correct it.
- Sites: `crates/desktop/src/gallery/items.rs:131`; `crates/desktop/src/shell.rs:504`; `crates/desktop/qml/Viewer.qml:284`; `crates/desktop/src/gallery/edits.rs:137`; `crates/desktop/src/gallery/edits.rs:129`

### UI-149 [medium/stale-state] Border-mark button stays checked after a failed write and across photos — CONFIRMED

- Verdict: CONFIRMED
- Note: The checkable button toggles checked on click (RatingBar.qml:48-59). border_mark_done returns early when the write failed (edits.rs:151-153), so borderMarked does not change and the binding at RatingBar.qml:52 does not re-run. The button stays checked.
- Confidence: high
- Trigger: In the viewer, click the border-mark button on a photo with borders found while the mark write fails (the catalog write errors). Qt toggles the checkable button's checked before clicked fires. borderMarked never changes, so the checked: bar.borderMarked binding does not re-run.
- Consequence: The button shows checked and struck through while the photo is not marked and the overlay is not grey. The EditFailed notice reports the failure, but the control contradicts it. The wrong state carries over when the user moves to another unmarked photo, because the bound value does not change.
- Sites: `crates/desktop/qml/RatingBar.qml:52`; `crates/desktop/qml/RatingBar.qml:59`; `crates/desktop/src/gallery/edits.rs:151`; `crates/desktop/src/gallery/edits.rs:152`

### UI-150 [low/missing-feedback] N on a photo with no borders found does nothing and says nothing — CONFIRMED

- Verdict: CONFIRMED
- Note: keys.rs:118 binds N for any selected item. set_border_marked returns early for an item without borders Found or Marked (edits.rs:134-136), so the key does nothing, and no message is posted.
- Confidence: high
- Trigger: Select a photo with no borders found (the common case) in the grid and press N. The key is bound for any selected item, and the grid hint lists N as not a border.
- Consequence: Nothing changes and no message appears, so the user cannot tell whether the key was received or the photo was refused. The viewer does not bind N unless borders are found, so the two paths differ.
- Sites: `crates/client/src/keys.rs:118`; `crates/desktop/src/gallery/edits.rs:134`

### UI-151 [low/inconsistent-gate] Viewer keeps drawing border overlays after border detection is turned off — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-021)
- Note: Duplicate of UI-021, UI-114 and UI-153: Main.qml:649 passes win.showBorders with no bordersOn gate, and Viewer.qml:456 draws from it.
- Confidence: medium
- Trigger: With borders shown (B) and detection on, turn detection off in the Library panel while the window runs. The grid hides borders (Main.qml:627). The viewer does not.
- Consequence: The viewer keeps showing the border overlay and the border-mark button while detection is off. Pressing B in the viewer only shows the detection-off popup and cannot hide the overlay.
- Sites: `crates/desktop/qml/Main.qml:649`; `crates/desktop/qml/Main.qml:627`; `crates/desktop/qml/Viewer.qml:456`; `crates/desktop/qml/Main.qml:709`

### UI-152 [low/modifier-ignored] Ctrl+N marks or unmarks a border in the grid and the viewer — CONFIRMED

- Verdict: CONFIRMED
- Note: keys.rs:118 (grid N) and keys.rs:138 (viewer N, guarded only by shown.has_borders) do not check mods.ctrl, so Ctrl+N marks a border in both places.
- Confidence: medium
- Trigger: Press Ctrl+N with a photo selected in the grid, or on a photo with borders found in the viewer. The key map matches Letter('N') without checking modifiers, so Ctrl is ignored.
- Consequence: A stray Ctrl+N silently changes a border mark. It is reversible by pressing N again, but no notice says so. Letter('F') already requires !ctrl, so the key map is not uniform.
- Sites: `crates/client/src/keys.rs:118`; `crates/client/src/keys.rs:138`

## U41: Show or hide borders (viewer B key)

Entry: `crates/desktop/qml/Viewer.qml:283 Borders key -> bordersToggled (Viewer.qml:113) -> Main.qml:662-665 shell.toggleBorders (shell_bridge.rs:134), or featurePopup when borders are off`

### UI-153 [medium/inconsistent-gate] Viewer draws borders while border detection is off — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-021)
- Note: Duplicate of UI-021 and UI-114. bordersTurnedOff (shell.rs:373-376) and the listing change (listing.rs:70-75) never clear the viewer's show flag at Main.qml:649. The --borders hook path is also real (Main.qml:478).
- Confidence: high
- Trigger: Borders are shown (B pressed while detection is on). The user unticks Detect borders in the Library panel (Main.qml:706-710 -> shell.bordersTurnedOff). The show flag is never cleared (shell.rs:373-376 and listing.rs:70-75 only change the listing). The user then opens a picture whose stored borders exist. Alternatively, the --borders test hook (Main.qml:478) turns the flag on with detection off.
- Consequence: The viewer keeps drawing the magenta frame (Viewer.qml:437) and the border overlay with its cut (Viewer.qml:449-467), even though border detection is off and the popup says so. The grid hides the same borders (Main.qml:627). Pressing B only shows 'Border detection is off. Turn it on in the Library panel.' (ui-borders-disabled, ftl:85) and cannot hide the overlay. The only way out is to re-enable detection. The text the user sees contradicts what is drawn.
- Sites: `crates/desktop/qml/Main.qml:649`; `crates/desktop/qml/Main.qml:627`; `crates/desktop/qml/Viewer.qml:437`; `crates/desktop/qml/Viewer.qml:456`; `crates/desktop/qml/Main.qml:709`; `crates/desktop/src/shell.rs:373`; `crates/client/src/controller/listing.rs:70`; `crates/desktop/qml/Main.qml:478`

### UI-154 [low/misleading-hint] Grid key hint advertises B and N with border detection off — CONFIRMED

- Verdict: CONFIRMED
- Note: keymap.rs:153-155 calls keys::grid_hint, which always lists Borders and NotBorder (keys.rs:288-291) with no feature check. The status bar shows it with nothing selected (Main.qml:468).
- Confidence: medium
- Trigger: Border detection is off and no item is selected. The status bar shows the grid key hint, which lists B borders and N not a border regardless of the feature.
- Consequence: The hint offers keys that do nothing except open the 'Border detection is off' popup. N also needs a selected item, so it cannot be used from that hint line at all.
- Sites: `crates/desktop/qml/Main.qml:468`; `crates/desktop/src/keymap.rs:153`; `crates/client/src/keys.rs:288`

### UI-155 [low/double-fire] Held B key toggles borders on every auto-repeat — CONFIRMED

- Verdict: CONFIRMED
- Note: Same root as UI-140: Viewer.qml:267 handles every press, so B repeats toggle the viewer's borders (Viewer.qml:283, bordersToggled). The grid's B has no auto-repeat check either (shell.rs:161). The result depends on the number of repeats.
- Confidence: high
- Trigger: B is held down in the viewer or the grid, so the key auto-repeats. Each repeat is a separate key press.
- Consequence: The borders flicker, each repeat queues a preferences write, and the final state depends on how many repeats arrived (parity), not on a single user toggle.
- Sites: `crates/desktop/qml/Viewer.qml:267`; `crates/desktop/qml/GalleryGrid.qml:73`; `crates/desktop/src/shell.rs:161`

## U42: Step through pictures in the viewer (wheel, keys, swipe)

Entry: `crates/desktop/qml/Viewer.qml:302-313 WheelHandler (notches), :274-275 Keys StepBack/StepForward, :346 touch swipe -> viewer.step (Viewer.qml:170)`

### UI-156 [high/shadowed-key] Space clicks a focused viewer button instead of stepping — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-139)
- Note: Duplicate of UI-139 and UI-143 for the viewer side: a focused rating or turn button takes Space (qplatformtheme.cpp:700), and Viewer.qml:275 steps only if the viewer has the key. The video play and mute buttons (VideoTransport.qml:57) have the same effect.
- Confidence: high
- Trigger: In the viewer, the user clicks a rating star (or the turn, play or mute button). Qt gives that ToolButton active focus on press, and the viewer does not take focus back when it steps. The user then presses Space on the next picture: the focused button accepts Space (ButtonPressKeys), so the viewer's Space-to-step never runs. The star clicked again rates the newly shown photo with that star count and steps on, so that photo is passed over unseen. The turn buttons rotate the photo instead of stepping, and play and mute toggle playback.
- Consequence: The next photo gets a rating it was never shown with, overwriting any rating it had, and the triage moves on past it without the user seeing it. The hint line says Space steps forward, so the result looks correct to the user while it is wrong.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/RatingBar.qml:83`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:275`; `D:/GitHub/photo-gallery/crates/client/src/keys.rs:128`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:183`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:161`; `D:/GitHub/photo-gallery/crates/desktop/qml/RatingBar.qml:66`; `D:/GitHub/photo-gallery/crates/desktop/qml/VideoTransport.qml:57`

### UI-157 [medium/shadowed-key] Left and Right are taken by a focused transport slider — CONFIRMED

- Verdict: CONFIRMED
- Note: Qt's Slider handles Left and Right when it has focus (qquickslider.cpp:761-775), so the transport slider takes the keys that Viewer.qml:274-275 would use. Viewer.qml:156 focuses the viewer only when it becomes visible, so nothing takes focus back after a click on the slider. Whether a click gives the slider focus was not checked.
- Confidence: high
- Trigger: A video is open. The user clicks the volume slider (or the seek slider, which is enabled when the video is seekable). Focus stays on the slider, which accepts Left and Right to change its value. The user presses Right or Left to move to the next or previous item: the slider changes the volume (or seeks) and the viewer does not step. Focus stays there until the viewer closes, because nothing refocuses the viewer.
- Consequence: Stepping by keyboard stops working through videos until the viewer is closed and reopened. The volume or the playback position changes unexpectedly, and the hint line still says Left and Right step.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/VideoTransport.qml:83`; `D:/GitHub/photo-gallery/crates/desktop/qml/VideoTransport.qml:62`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:274`; `D:/GitHub/photo-gallery/crates/client/src/keys.rs:127`

### UI-158 [medium/wrong-message] Viewer hint says wheel zoom, but the wheel steps pictures — PARTIAL

- Verdict: PARTIAL
- Note: The hint lists WheelZoom for stills (keys.rs:304; ui.ftl:231), and the wheel handler calls viewer.step(-n) (Viewer.qml:306-311). I did not read the branch above it at Viewer.qml:300-312 that decides whether a notch zooms when the image is not zoomed.
- Confidence: high
- Trigger: A still image is open. The hint line at the top right reads wheel zoom. The user scrolls expecting zoom. Each notch moves to the previous or next picture and resets the view, losing the zoom and pan.
- Consequence: The hint misleads the user about what the wheel does. Users skip pictures and lose their view position, and cannot find the zoom that the hint promised. The zoom keys are pinch, + and -, and double-click 1:1.
- Sites: `D:/GitHub/photo-gallery/crates/client/locales/en/ui.ftl:231`; `D:/GitHub/photo-gallery/crates/client/src/keys.rs:304`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:310`

## U43: Zoom, pan, fit and 1:1 in the viewer

Entry: `crates/desktop/qml/Viewer.qml:318-320 PinchHandler -> setZoom; :327-329 DragHandler pan; :349-351 touch pan; :354-362 TapHandler double-tap -> toggleOneToOne (Viewer.qml:221); keys Viewer.qml:279-281 (RealSize, ZoomIn, ZoomOut)`

### UI-159 [high/shadowed-key] Space after clicking a rating-bar button rates or turns the next picture — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-139)
- Note: Duplicate of UI-139 for the viewer's rating bar and find-similar button. Viewer.qml:156 sets focus only on becoming visible, and rate() (Viewer.qml:183-188) does not take focus back.
- Confidence: high
- Trigger: Open a photo in the viewer, click a star (or the turn or find-similar button) in the rating bar. The viewer rates the photo and steps to the next one, and focus stays on that button. The user then presses Space, which the hint and keymap define as step forward.
- Consequence: Space is consumed by the focused ToolButton, which fires a click on the picture now shown. The next picture is rated with the same star value (overwriting its existing rating, which is written back to the file when write-back is on), or it is rotated, and the viewer does not step. The user sees a changed rating or rotation on a photo they did not act on, and no step happens.
- Sites: `crates/desktop/qml/RatingBar.qml:83`; `crates/desktop/qml/Viewer.qml:156`; `crates/desktop/qml/Viewer.qml:267`; `crates/desktop/qml/Viewer.qml:161`

### UI-160 [medium/wrong-message] Viewer hint says 'wheel zoom' but the wheel steps to the next picture — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-158)
- Note: Duplicate of UI-158, with the same citations (ui.ftl:231, keys.rs:304, Viewer.qml:310). It is a restatement of the same hint, and it is partial for the same reason.
- Confidence: high
- Trigger: Open a still image in the viewer. The hint line reads 'wheel zoom'. The user scrolls the wheel expecting to zoom.
- Consequence: Each wheel notch moves to the previous or next photo and resets the zoom and pan. The on-screen instruction teaches a gesture that does not work, so the user cannot zoom with the wheel and may lose a zoomed view they were inspecting.
- Sites: `crates/client/locales/en/ui.ftl:231`; `crates/client/src/keys.rs:304`; `crates/desktop/qml/Viewer.qml:310`; `crates/client/src/text.rs:798`

### UI-161 [low/double-fire] Key auto-repeat fires the viewer's toggles and turns repeatedly — CONFIRMED

- Verdict: CONFIRMED
- Note: Viewer.qml:267 has no isAutoRepeat check, so Z (Viewer.qml:279) flips settings.realSize on each repeat. set_real_size_saved writes the prefs each time (settings.rs:41-45). R repeats turn the photo (Viewer.qml:277).
- Confidence: high
- Trigger: Hold Z (or L, H, K, M, N, B, or R) in the viewer past the OS key-repeat delay. Each repeated KeyPress reaches Keys.onPressed, which never checks e.isAutoRepeat.
- Consequence: Z flips the persisted Real Size preference (which also governs future images and runs) on every repeat and writes the prefs file each time, so the final mode depends on how many repeats occurred. R queues a quarter turn per repeat, which is written into the original file when write-back is on. The HUD shows the final state, but the result is parity-dependent and unintended.
- Sites: `crates/desktop/qml/Viewer.qml:267`; `crates/desktop/qml/Viewer.qml:279`; `crates/desktop/src/settings.rs:41`

### UI-162 [low/stale-state] Gesture start captures the view after the handler's threshold, so the view jumps at the start of a drag or pinch — PARTIAL

- Verdict: PARTIAL
- Note: The gesture start stores the base offset or scale at activation (Viewer.qml:318, 327, 339), so the handler's threshold could show as a jump. I did not check Qt's source for whether the drag or pinch translation is measured from the press point.
- Confidence: medium
- Trigger: Pan a zoomed picture with the mouse or touch, or start a pinch zoom. The base offset or scale is captured at activation, but Qt measures the translation or scale from the press point, which includes the drag or pinch threshold distance.
- Consequence: The picture shifts by roughly the threshold distance (a few pixels for a drag, a few percent for a pinch) at the first move of each gesture, then tracks the pointer. This is a small visual jump, not a data problem.
- Sites: `crates/desktop/qml/Viewer.qml:318`; `crates/desktop/qml/Viewer.qml:327`; `crates/desktop/qml/Viewer.qml:329`; `crates/desktop/qml/Viewer.qml:339`

### UI-163 [low/stale-state] A tap just before closing flips the overlay state that the next opening shows — PARTIAL

- Verdict: PARTIAL
- Note: zenTimer toggles zen when it fires (Viewer.qml:364-368), and the tap restarts it (Viewer.qml:360). show() does not reset zen (Viewer.qml:158-162). The close() body was not read, so whether it stops the timer is not settled.
- Confidence: high
- Trigger: Single-click the picture, then close the viewer (Esc or Enter) within the double-click interval. zenTimer is not stopped on close, so it fires while the viewer is hidden and toggles zen.
- Consequence: On the next opening the HUD (counter, hint, rating bar, close button) is hidden, or shown when it should not be, until the user taps again. The state is not reset on show or close.
- Sites: `crates/desktop/qml/Viewer.qml:360`; `crates/desktop/qml/Viewer.qml:367`; `crates/desktop/qml/Viewer.qml:165`; `crates/desktop/qml/Viewer.qml:67`

## U44: Viewer re-render: fit size and region tiles

Entry: `crates/desktop/qml/Viewer.qml:130-131 onFitWidthChanged/onFitHeightChanged -> fitSettle (Viewer.qml:146-150) -> settleFit (Viewer.qml:140); Viewer.qml:384 ImageLayers onRegionDue`

### UI-164 [medium/missing-feedback] Zoomed region at the right or bottom edge fails on a float overshoot of the crop — PARTIAL

- Verdict: PARTIAL
- Note: The clamp at image.rs:134 (w.min(1.0 - x)) does not keep x*iw + w*iw within iw in floating point, and fast_image_resize rejects a crop past the edge (crop_box.rs:116). The 2% rate is from a Python re-run of the arithmetic, which the finding itself says is not a build or test. The rate was not measured in Rust.
- Confidence: high
- Trigger: The viewer is zoomed between fit and 1:1 and the view is panned or zoomed until the visible part reaches the right or bottom edge. Then x + w (or y + h) is exactly 1 in real arithmetic, and the rounded products x*iw and w*iw can sum to slightly more than iw. A Python re-run of this arithmetic (not a build or test) gives about 2 percent overshoot for edge-touching regions with widths 1000-8000 px and six-place fractions.
- Consequence: fast_image_resize rejects the crop (SizeIsOutOfImageBoundaries), so that region's Rust render fails. The QML region Image goes to Error and draws nothing, so the view shows the soft fit stretched to the zoomed size, with no message. Only a warning in the log records it. The next pan usually renders correctly, so the user sees an intermittent blur at the edges.
- Sites: `D:/GitHub/photo-gallery/crates/client/src/image.rs:134`; `D:/GitHub/photo-gallery/crates/render/src/display.rs:120`; `D:/GitHub/photo-gallery/crates/codec/src/resize.rs:89`; `C:/Users/Torres/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/fast_image_resize-6.1.0/src/crop_box.rs:116`

### UI-165 [medium/stale-result] A key change (file edited or re-read) leaves the zoomed region showing the old pixels — PARTIAL

- Verdict: PARTIAL
- Note: onKeyChanged calls settleFit (Viewer.qml:91), and the region is dropped only on viewReset (Viewer.qml:523). Whether settleFit clears the region layer was not read. The finding also says it was not verified by a run.
- Confidence: medium
- Trigger: The open file changes on disk (mtime changes, or its rotation or pending turns change) while the viewer shows it zoomed between fit and 1:1, and the gallery refreshes the viewed item (refresh_viewed on a revision bump). Viewer.key changes, but the view is not reset.
- Consequence: The fit is re-rendered from the new file, but the region placed over it still shows the previous version's pixels, as though they were current, until the user pans or zooms. A zoomed, changed file can look unchanged. (Depends on the list refresh delivering the new mtime while the viewer is open; not verified by a run.)
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:91`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:521`; `D:/GitHub/photo-gallery/crates/desktop/qml/ImageLayers.qml:121`; `D:/GitHub/photo-gallery/crates/desktop/qml/ImageLayers.qml:96`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/items.rs:45`; `D:/GitHub/photo-gallery/crates/desktop/src/gallery/model.rs:302`

### UI-166 [low/missing-feedback] A failed region, or an offline original seen while zoomed, shows nothing and logs nothing — CONFIRMED

- Verdict: CONFIRMED
- Note: Hud.qml:95 shows "original unavailable" only from fitFailed (Viewer.qml:498). full_image logs only Undecodable (images.rs:317-319), and providers.rs:125-128 skips logging for Original and Undecodable. An Original error or a failed region is therefore invisible to the user and the log.
- Confidence: high
- Trigger: While zoomed, the original goes offline or is removed, or a region request fails for any reason (the file is rewritten past the 10 s wait, or the crop fails). The fit that is already on screen stays Ready.
- Consequence: The Hud reads only the fit's status, so no original unavailable line appears. Region failures are not in the Hud at all. For an Original error, full_image logs nothing (only Undecodable is logged), and providers skips logging Original, so the failure is invisible to both user and log. The view silently keeps the soft fit.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:498`; `D:/GitHub/photo-gallery/crates/desktop/qml/ImageLayers.qml:92`; `D:/GitHub/photo-gallery/crates/app/src/images.rs:317`; `D:/GitHub/photo-gallery/crates/desktop/src/providers.rs:125`

### UI-167 [low/wrong-message] Any fit failure is labelled original unavailable, including undecodable files and rewrite timeouts — CONFIRMED

- Verdict: CONFIRMED
- Note: Hud.qml:95 uses fitFailed, which is true for any fit error, so an Undecodable or Rewriting failure (images.rs:189) shows the same "original unavailable" text.
- Confidence: medium
- Trigger: A fit request fails with Error::Undecodable (a file that does not decode) or with Error::Rewriting (the 10 s write wait ran out), while the original is present.
- Consequence: The Hud says the original is unavailable for a file that is there, which misleads the user about the cause. The Rust side logs these, but the user does not see it.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Hud.qml:95`; `D:/GitHub/photo-gallery/crates/client/locales/en/ui.ftl:60`; `D:/GitHub/photo-gallery/crates/app/src/images.rs:189`

### UI-168 [low/repeated-failure] Each region request re-decodes an undecodable file and logs a warning — CONFIRMED

- Verdict: CONFIRMED
- Note: images.rs:322-326 stores only successful decodes, so a failing file decodes again on each request, and images.rs:317-319 logs a warning on each one. The request rate of 20 per second is from the finding and was not checked.
- Confidence: medium
- Trigger: An undecodable still whose catalog size is known (so it is zoomable) is zoomed between fit and 1:1, and the view is panned. Every throttled region request (up to 20 per second) runs.
- Consequence: A failed decode is never kept (only successful decodes are stored in last_decodes), so each request repeats the decode and writes a viewer warning. This costs CPU on the two Full-pool threads, which the fit requests share, and floods the log.
- Sites: `D:/GitHub/photo-gallery/crates/app/src/images.rs:314`; `D:/GitHub/photo-gallery/crates/app/src/images.rs:322`; `D:/GitHub/photo-gallery/crates/desktop/qml/Viewer.qml:384`

## U45: Loupe (10x under the cursor)

Entry: `crates/desktop/qml/Viewer.qml:282 Loupe key -> loupe toggle (Viewer.qml:66 loupe property); Viewer.qml:472-484 Loupe with cursor from HoverHandler (Viewer.qml:469)`

### UI-169 [medium/wrong-message] Loupe label shows for videos and animations where loupe does nothing — PARTIAL

- Verdict: PARTIAL
- Note: Hud.qml:93 shows the loupe label from viewer.loupe (Viewer.qml:497), and Viewer.qml:282 toggles it for any kind. The hint lists Loupe for stills only (keys.rs:304). The Loupe.qml 'requested' definition, which the finding says gates the overlay, was not read.
- Confidence: high
- Trigger: The user presses L while a video or animated GIF is shown. The loupe flag turns on (Viewer.qml:282), and the Hud label reads 'loupe 10x' (Hud.qml:93, fed by Viewer.qml:497). Nothing appears, because Loupe.qml:30 requires requested = viewer.loupe && viewer.still. The hint line does not list L for these kinds (keys.rs:303-304).
- Consequence: The overlay reports the loupe as active on a video where nothing shows and the key is not in the hint line. The user sees a state that does not match what is on screen, until they move to a still.
- Sites: `crates/desktop/qml/Viewer.qml:282`; `crates/desktop/qml/Viewer.qml:497`; `crates/desktop/qml/Hud.qml:93`; `crates/client/src/keys.rs:136`

### UI-170 [low/double-fire] Holding L toggles the loupe on every auto-repeat — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-161)
- Note: Duplicate of UI-161 for L: Viewer.qml:267 has no isAutoRepeat check, and Loupe toggles on each press (Viewer.qml:282).
- Confidence: high
- Trigger: The user holds L down. Qt sends auto-repeat key presses after the initial delay, and Keys.onPressed handles each one, so loupe = !loupe flips on every repeat, at the OS repeat rate.
- Consequence: The loupe flickers while the key is held, and its final state depends on how many repeats fired (on or off, at random). The other toggles on this handler (Z, B, H, M) share the same flaw.
- Sites: `crates/desktop/qml/Viewer.qml:267`; `crates/desktop/qml/Viewer.qml:282`

### UI-171 [low/missing-feedback] L with the pointer off the picture shows nothing, but the label says loupe is on — CONFIRMED

- Verdict: CONFIRMED
- Note: Loupe.qml:30 requires hovered, which comes from the HoverHandler in the viewport (Viewer.qml:469). A keyboard L with the pointer off the picture sets the flag and the label without showing anything.
- Confidence: high
- Trigger: The user uses the keyboard only, or parks the pointer on the frame or outside the picture, then presses L. The loupe needs hovered (Loupe.qml:30, from the HoverHandler at Viewer.qml:469), so nothing shows until the pointer enters the picture. Meanwhile the Hud label reads 'loupe 10x' (Hud.qml:93).
- Consequence: The keyboard path does not reach the same result as the mouse path: the loupe appears only after the pointer moves over the image, with no hint that it needs the pointer there.
- Sites: `crates/desktop/qml/Loupe.qml:30`; `crates/desktop/qml/Viewer.qml:469`; `crates/desktop/qml/Hud.qml:93`

### UI-172 [low/missing-feedback] Full-size decode failure leaves the loupe silently absent — PARTIAL

- Verdict: PARTIAL
- Note: A failed full decode would leave the full layer in Error and the loupe absent, and Hud shows only the fit status (Viewer.qml:498). fullReady (ImageLayers.qml:33) was not read, so whether it stays false is not settled.
- Confidence: medium
- Trigger: A still whose fit decodes (the scaled decode, display.rs:67) but whose full decode fails (MinSize::Full, display.rs:71; for example a truncated file or a memory failure on a very large original). The user presses L. full.status goes to Error, so fullReady stays false (ImageLayers.qml:33) and the loupe never appears.
- Consequence: The loupe silently does nothing and the label still says 'loupe 10x'. The only original-unavailable message is tied to the fit (Viewer.qml:498), so the user gets no explanation. A later 1:1 view also shows nothing, because the region and the full both fail.
- Sites: `crates/desktop/qml/ImageLayers.qml:121`; `crates/desktop/qml/Viewer.qml:498`; `crates/desktop/src/providers.rs:118`; `crates/render/src/display.rs:71`

## U46: Close the viewer

Entry: `crates/desktop/qml/Viewer.qml:273 Close key -> close (Viewer.qml:165); crates/desktop/qml/Hud.qml:76 onClicked -> closeAsked -> Viewer.qml:506`

### UI-173 [low/double-fire] Held Enter closes the viewer, then auto-repeat reopens it — CONFIRMED

- Verdict: CONFIRMED
- Note: Viewer.qml:273 closes on Enter. The viewer's visibility change hands focus to the grid (Main.qml:655-659), and Enter maps to Grid::Open (keys.rs:113), so the repeats reopen it. Nothing filters isAutoRepeat.
- Confidence: high
- Trigger: The user holds Enter (or Escape-free Enter) while the viewer is open. Enter closes the viewer on the first press. After the OS repeat delay, the repeated KeyPress reaches the grid, which has taken focus (Main.qml:657), and Enter maps to Grid::Open, so the viewer reopens on the same photo. Repeats keep toggling it while the key is held, and the viewer is open or closed when Enter is released depending on timing. The reverse also happens when Enter opened the viewer from the grid: its repeats close it again.
- Consequence: The viewer flickers open and closed while Enter is held. The state it ends in depends on when the key is released. No data is lost, and the photo and selection stay the same. Nothing filters isAutoRepeat, so this is reachable with an ordinary held key.
- Sites: `crates/desktop/qml/Viewer.qml:273`; `crates/client/src/keys.rs:126`; `crates/desktop/qml/GalleryGrid.qml:73`; `crates/client/src/keys.rs:110`; `crates/desktop/src/shell.rs:151`

### UI-174 [low/unhandled-close] Pending zen timer from a tap fires after the viewer is closed — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-163)
- Note: Duplicate of UI-163: zenTimer (Viewer.qml:364-368) is not stopped by close() (Viewer.qml:165-168), which only emits closed and moved(-1). A tap before close restarts the timer, and it fires after the viewer is hidden.
- Confidence: high
- Trigger: The user single-taps the picture, then closes the viewer with Escape or Enter within the mouse double-click interval (about 500 ms). The TapHandler's zenTimer is still running, close() does not stop it, and it fires while the viewer is closed.
- Consequence: The zen flag (overlays hidden) flips while the viewer is closed. The next time the viewer opens, the HUD is in the opposite state to the one the user left it in, and the user has to tap again to correct it.
- Sites: `crates/desktop/qml/Viewer.qml:165`; `crates/desktop/qml/Viewer.qml:360`; `crates/desktop/qml/Viewer.qml:367`

## U47: Video and animation transport

Entry: `crates/desktop/qml/VideoTransport.qml:57 playAsked, :79 muteAsked, :68 onMoved seekAsked, :87 onMoved volumeAsked; keys Viewer.qml:286 PlayPause, :287 Mute`

### UI-175 [medium/stale-focus] Keyboard focus stays on hidden transport; arrows change hidden controls — CONFIRMED

- Verdict: CONFIRMED
- Note: Qt's QQuickItem::setVisible documentation (qquickitem.cpp:6641-6645) says a hidden item keeps keyboard focus, and QQuickItemPrivate::setVisible (qquickitem.cpp:6669-6681) does not clear it. So the slider or button keeps focus once the transport hides, and the viewer's Keys handler does not see the arrows.
- Confidence: high
- Trigger: Open a video in the viewer, click the seek or volume bar (or the play or mute button), then move to a photo or animation with the mouse wheel (or press H to hide overlays), and press Left or Right.
- Consequence: Left and Right no longer step through the pictures. They seek the hidden video or change the session volume (which carries to later videos). Focus is not returned to the viewer until the viewer is reopened or a panel closes, so arrow navigation looks broken.
- Sites: `crates/desktop/qml/VideoTransport.qml:39`; `crates/desktop/qml/VideoTransport.qml:62`; `crates/desktop/qml/MediaView.qml:130`; `crates/desktop/qml/Viewer.qml:156`; `crates/desktop/qml/Viewer.qml:267`

### UI-176 [low/shadowed-key] Focused transport controls consume Space and arrows while visible — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-156)
- Note: Duplicate of UI-156 and UI-157 for the transport: the play and mute ToolButtons (VideoTransport.qml:54-60, 76-82) take focus on click and accept Space, and the seek and volume Sliders (VideoTransport.qml:62, 83) take Left and Right. Viewer.qml:275 steps only when the viewer has the key.
- Confidence: high
- Trigger: Click the transport play or mute button, then press Space: playback (or mute) toggles instead of stepping to the next picture. After clicking the seek bar, Left and Right seek the video instead of stepping.
- Consequence: Space does not step. Space toggles the video or mute state each press, and the arrow keys act on the slider. The user sees playback change where they expected navigation, and focus never goes back to the viewer.
- Sites: `crates/desktop/qml/VideoTransport.qml:54`; `crates/desktop/qml/VideoTransport.qml:62`; `crates/desktop/qml/Viewer.qml:267`

### UI-177 [low/double-fire] Held K or M toggles on every auto-repeat — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-161)
- Note: Duplicate of UI-161 and UI-170 for K and M: Viewer.qml:267 has no isAutoRepeat check, and Viewer.qml:286 (togglePlay) and :287 (muted = !muted) flip on each press.
- Confidence: high
- Trigger: Press and hold K (play/pause) or M (mute) on a video past the key repeat delay.
- Consequence: Play/pause or mute flickers on each repeat, and the final state depends on the release timing rather than the single press.
- Sites: `crates/desktop/qml/Viewer.qml:267`; `crates/desktop/qml/Viewer.qml:286`; `crates/desktop/qml/Viewer.qml:287`

### UI-178 [low/missing-feedback] Mute button gives no visible change at volume 0 — CONFIRMED

- Verdict: CONFIRMED
- Note: VideoTransport.qml:77 shows the muted icon when muted or volume is 0, and onClicked (VideoTransport.qml:79) only calls muteAsked. At volume 0 the icon does not change, and unmuting leaves it on the muted icon.
- Confidence: high
- Trigger: Drag the volume slider to 0, then click the mute button. The icon stays the muted icon (it is shown for volume 0 whatever the mute flag), and a second click also changes nothing on screen.
- Consequence: The mute toggle appears broken at volume 0. After unmuting at volume 0 the icon still says muted, which is wrong until the volume is raised.
- Sites: `crates/desktop/qml/VideoTransport.qml:77`; `crates/desktop/qml/VideoTransport.qml:79`

### UI-179 [low/missing-feedback] Video catalog-lookup failure is silent — CONFIRMED

- Verdict: CONFIRMED
- Note: video_source.rs:36-38 on Err logs a warning and calls player.clear(), which sets no playerError. The HUD shows a player error only when playerError is set (Hud.qml:96), so the failure is silent apart from the log.
- Confidence: high
- Trigger: Open a video whose catalog lookup fails (the file is removed from the catalog while the viewer is open, or the catalog read errors).
- Consequence: The poster stays, the HUD shows nothing, and nothing indicates the video will not play. A failed device open does show Could not open source device.
- Sites: `crates/desktop/src/video_source.rs:36`; `crates/desktop/src/video_source.rs:38`

### UI-180 [low/missing-feedback] Animation that cannot be read or decoded fails silently — PARTIAL

- Verdict: PARTIAL
- Note: Citation error: animation.rs:76 and :112 are both past end of file (LINE OUT OF RANGE). The HUD part holds: Hud.qml:96 shows playerError only for Video kind, so an animation failure gets no message. The animation decode path that would set an error was not read, so the failure mechanism is not verified.
- Confidence: medium
- Trigger: Open an animated GIF, APNG or WebP whose file cannot be read or whose frames fail to decode (for example a corrupt file, or one removed after listing while its thumbnail is still cached). Press K.
- Consequence: The still poster stays, the HUD shows no message, and K does nothing visible. Only video reports playerError into the HUD.
- Sites: `crates/desktop/src/animation.rs:76`; `crates/desktop/src/animation.rs:112`; `crates/desktop/qml/Hud.qml:96`

### UI-181 [low/wrong-message] HUD shows untranslated Qt English error text — CONFIRMED

- Verdict: CONFIRMED
- Note: MediaView.qml:101 forwards errorString unchanged to playerFailed. Hud.qml:96 wraps it in ui-player-error (ui.ftl:61, "— { $text }"), so the prefix is translated and the body is Qt's English. The string is at qffmpegmediadataholder.cpp:210 ("Could not open source device.").
- Confidence: high
- Trigger: Any playback error from the device or the FFmpeg backend, for example an unopenable file.
- Consequence: The HUD shows the catalogued prefix followed by Qt's English string (for example Could not open source device.). The text is not catalogued, is not translated, and is not pseudo-localized.
- Sites: `crates/desktop/qml/MediaView.qml:101`; `crates/desktop/qml/Hud.qml:96`; `crates/client/locales/en/ui.ftl:61`; `C:/Qt/6.12.0/Src/qtmultimedia/src/plugins/multimedia/ffmpeg/playbackengine/qffmpegmediadataholder.cpp:210`

## U48: Find similar from the viewer

Entry: `crates/desktop/qml/Hud.qml:44 findSimilarAsked (RatingBar.qml:45 same signal) -> Viewer.qml:510 onFindSimilarAsked -> Viewer.qml:116 findSimilar -> Main.qml:672 win.findSimilar (Main.qml:377)`

### UI-182 [high/stale-result] Find similar for a second photo shows the first photo's results — CONFIRMED

- Verdict: CONFIRMED
- Note: yandex.rs:141 returns on Ok(false), which app/yandex.rs:425-426 gives while busy, so search_photo neither resets the model nor sets status. openPanel (YandexPanel.qml:42-47) has already shown the panel, so it shows A's busy state or A's results under a request for B.
- Confidence: high
- Trigger: The user runs find-similar on photo A, closes the Yandex panel while A's search or save is still running (busy), goes to the viewer on photo B and clicks '≈'. Search::start refuses because busy is set, and search_photo returns at Ok(false) without touching the model or status.
- Consequence: The panel opens with A's progress (busy spinner and status) or, once A finishes, A's results, under a find-similar request for B. The panel does not name the photo, so the user sees results for another picture as if they answered B. Nothing says the request was refused.
- Sites: `crates/desktop/src/yandex.rs:141`; `crates/desktop/src/yandex.rs:147`; `crates/app/src/yandex.rs:425`; `crates/desktop/qml/YandexPanel.qml:44`

### UI-183 [medium/stale-result] Unreadable-photo error sits over the previous photo's results — CONFIRMED

- Verdict: CONFIRMED
- Note: yandex.rs:120-124 sets only the status on UnknownFile (catalogs.rs:49), and the earlier results stay as they were. openPanel (YandexPanel.qml:31-33) has already shown them. The "raw catalog id" wording depends on how Error::UnknownFile renders, which was not read.
- Confidence: medium
- Trigger: The viewer asks for find-similar on a file the catalog cannot resolve (catalogs.file_path returns UnknownFile, or the catalog read fails) while results from an earlier search are still in the model.
- Consequence: openPanel has already shown the panel, and the error is set as the status, but the earlier photo's results and busy state are left as they were. The user sees an error about this photo next to another photo's results, and the error text shows a raw catalog id ('unknown file N').
- Sites: `crates/desktop/src/yandex.rs:124`; `crates/app/src/catalogs.rs:49`; `crates/desktop/qml/YandexPanel.qml:33`

### UI-184 [low/double-fire] Double-clicking the find-similar button opens and immediately closes the panel — CONFIRMED

- Verdict: CONFIRMED
- Note: RatingBar.qml:45 opens the panel on the first click (openPanel makes it visible, YandexPanel.qml:42-47). The panel's full-window MouseArea (YandexPanel.qml:105-107) then receives the second click and calls close(). The stacking of the panel over the viewer was not read, but the comment at YandexPanel.qml:103-104 says no click reaches the viewer below.
- Confidence: medium
- Trigger: The user double-clicks '≈' in the viewer. The first click opens the panel and starts the search. The second click lands on the panel's full-window background MouseArea, which closes the panel.
- Consequence: The panel flashes open and closes. The search still runs in the background, and reopening shows its progress, so no result is lost, but the user sees a surprising close with no message.
- Sites: `crates/desktop/qml/RatingBar.qml:45`; `crates/desktop/qml/YandexPanel.qml:107`

### UI-185 [low/missing-keyboard-path] Find similar has no keyboard path in the viewer — CONFIRMED

- Verdict: CONFIRMED
- Note: keys.rs viewer() (keys.rs:124-...) has no find-similar arm, and ViewerAction (Viewer.qml:265) has no find-similar entry. The only keyboard route is the command-line --find-similar hook (YandexPanel.qml:36-40).
- Confidence: high
- Trigger: A keyboard-only user opens a still image in the viewer and wants a find-similar search. No key is bound to it (keys.rs viewer() has no such arm; the ViewerAction enum lacks it).
- Consequence: Only the mouse button reaches the search, and it is hidden in zen mode (H). The command line's --find-similar is the only other route. The gap may be intentional, but the flow has no keyboard equivalent.
- Sites: `crates/client/src/keys.rs:124`; `crates/desktop/qml/Viewer.qml:490`; `crates/desktop/qml/RatingBar.qml:41`

### UI-186 [low/missing-feedback] Dead results are dropped from the list with no note — CONFIRMED

- Verdict: CONFIRMED
- Note: yandex.rs:372-374 calls drop_dead without setting any status, and the status comes only from poll.status (yandex.rs:375-376), which carries the search's note and not the download failures.
- Confidence: medium
- Trigger: A search finds several results whose files cannot be fetched or decoded. The fetchers mark them dead, and poll() reports them, so drop_dead removes the rows.
- Consequence: The 'Other sizes (N)' and 'Similar images (N)' counts drop with no status line saying that some results could not be loaded. The user cannot tell a short list from a failed one.
- Sites: `crates/desktop/src/yandex.rs:372`; `crates/desktop/src/yandex.rs:241`

### UI-187 [low/stale-result] Previous search's fetchers keep running during a new search — CONFIRMED

- Verdict: CONFIRMED
- Note: app/yandex.rs:450-454 replaces the slot with the new Images and stops the previous one only at that point, so the old fetchers run until the new search's worker reaches it. The "no wrong result shown" part is consistent with the model being cleared (yandex.rs:147).
- Confidence: medium
- Trigger: A search finishes and its fetchers still have queued results. The user starts a new search before those fetches finish. The old Images stay in the slot until the new search's worker completes, and are only stopped then.
- Consequence: The old search's files keep downloading while the new search runs, competing with the new search's uploads and requests. No wrong result is shown, because the panel's model is cleared and the old ids are never reused.
- Sites: `crates/app/src/yandex.rs:450`; `crates/app/src/yandex/images.rs:96`

## U49: Yandex search results load

Entry: `crates/desktop/qml/YandexPanel.qml:31-34 show -> yandex.searchFile; command line: Main.qml:382-385 findSimilarFromCommandLine -> YandexPanel.qml:37-40 showCommandLine -> yandex.searchCommandLine (yandex_bridge.rs:79, yandex.rs:129)`

### UI-188 [medium/missing-feedback] Refused find-similar shows another search's state with no message — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-182)
- Note: Duplicate of UI-182: the same Ok(false) return at yandex.rs:141 while busy (app/yandex.rs:425-426), with the panel opened by YandexPanel.qml:33 and no status change.
- Confidence: high
- Trigger: A search (or a save) for photo A is still running after the user closed the panel. The user opens photo B in the viewer and clicks the find-similar button. search_photo gets Ok(false) from Search::start and returns without changing the status, so the panel opens showing A's progress or A's finished results.
- Consequence: The user sees progress or results from a different photo as the answer for B, with no message that the request for B was dropped. The panel does not show the photo's name, so nothing tells the two apart. B is not searched at all.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:141`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:33`; `D:/GitHub/photo-gallery/crates/app/src/yandex.rs:425`

### UI-189 [medium/stale-result] Failed lookup or start keeps the previous photo's results and picks on screen — PARTIAL

- Verdict: PARTIAL
- Note: The error-status half duplicates UI-183 (yandex.rs:123-124 sets only the status). The not_started path (yandex.rs:142) was not read, and the claim that Save picked names the earlier photo was not traced (where the save takes its name was not read).
- Confidence: high
- Trigger: After a search for an earlier photo has shown results, the viewed file is no longer in the catalog (its folder was removed while the viewer was open), so catalogs().file_path fails. The same happens if the search thread cannot start (not_started).
- Consequence: The panel shows the earlier photo's results under an error about the current photo. 'Save picked' stays enabled and saves the earlier photo's picks, named after the earlier photo, while the user is looking at a different photo.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:124`; `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:142`

### UI-190 [medium/wrong-message] 'No results.' shown when every found result failed to load — CONFIRMED

- Verdict: CONFIRMED
- Note: YandexPanel.qml:255 shows "No results" when not busy, both counts are 0 and status is "". Status is set at search completion (app/yandex.rs:462) and maps to empty when no note is given (text.rs:415), before any download fails. So it stays empty and the rows dropped by drop_dead (yandex.rs:372-374) leave the empty-result label.
- Confidence: high
- Trigger: Yandex returns results, but every result file fails to download or decode (for example the image hosts are unreachable). Each result is settled dead, and drop_dead removes all rows. The search ended with no note, so the status stays empty.
- Consequence: The panel shows 'No results.' as if Yandex found nothing. The user is not told that the results were found and failed to download.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:261`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:255`; `D:/GitHub/photo-gallery/crates/app/src/yandex.rs:462`; `D:/GitHub/photo-gallery/crates/client/src/text.rs:415`

### UI-191 [low/missing-feedback] Results whose files fail are removed from the panel with no note — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-186)
- Note: Duplicate of UI-186 at the source: images.rs:177-179 marks the result dead and drop_dead removes the tile (yandex.rs:241). No status is set for the removal.
- Confidence: high
- Trigger: Some result files fail to download or decode, for example a dead host or a non-image response. Their tiles are removed after the thumbnail attempt, and the group counts drop.
- Consequence: The panel shows fewer results than Yandex returned, and the status line says nothing about the missing ones, so the user cannot tell that results were lost.
- Sites: `D:/GitHub/photo-gallery/crates/app/src/yandex/images.rs:178`; `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:241`

### UI-192 [low/stuck-busy] Failed change-pump thread leaves the panel busy with its results never delivered — CONFIRMED

- Verdict: CONFIRMED
- Note: yandex.rs:176-179 leaves pump false when the thread fails, and the comment at yandex.rs:174-175 says the panel is then not told the search's changes. search_photo still sets busy (yandex.rs:145). How the panel recovers on a later open was not traced.
- Confidence: high
- Trigger: The OS refuses to start the 'yandex changes' thread in ensure_pump (a resource limit).
- Consequence: search_photo still starts the search and sets busy. The spinner and the Uploading line stay on screen, and the results are never polled, so they are lost. The panel recovers only on a later open, once the search has ended.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:138`; `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:176`; `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:145`

### UI-193 [low/focus] No keyboard path to pick or open an individual result — CONFIRMED

- Verdict: CONFIRMED
- Note: ResultTile.qml:97-113 is a MouseArea only, with no focus or key handling. YandexPanel.qml:97-102 accepts only Close, so no tile can be picked or opened from the keyboard.
- Confidence: medium
- Trigger: A keyboard-only user has opened the Yandex panel and tries to pick a result or open one at full size.
- Consequence: Only the mouse can pick a single result (a click, or Shift-click for a range) or open it (a double-click). Tab reaches the group Select all buttons and Close, but no tile can take focus, so the keyboard path does not reach the mouse path's results.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/ResultTile.qml:100`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:98`

## U50: Pick Yandex results, shift-range pick

Entry: `crates/desktop/qml/ResultTile.qml:100 onClicked -> toggled (ResultTile.qml:34) -> YandexPanel.qml:223, :245 yandex.togglePick; ResultTile.qml:35 rangeTo -> YandexPanel.qml:224, :246 yandex.pickTo`

### UI-194 [medium/missing-feedback] Find-similar for another photo is silently refused while a search or save runs, and the panel shows the old photo's results — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-188)
- Note: Duplicate of UI-188 for the save case: search_photo returns on Ok(false) (yandex.rs:141) while the app is busy (app/yandex.rs:425-426), and openPanel (YandexPanel.qml:42-47) has already shown the panel.
- Confidence: high
- Trigger: A save (or a search) is running. The user closes the Yandex panel, goes to the viewer, and presses find-similar on photo B. search_photo gets Ok(false) from Search::start because busy is set, so nothing starts and nothing is said.
- Consequence: The panel opens showing the previous photo's results and picks, with the status line still describing the other job. The user sees photo A's similar images as if they were for photo B, and no notice says B's search was never started. The status line is then overwritten by the save's outcome, so nothing explains the mismatch.
- Sites: `crates/desktop/src/yandex.rs:139`; `crates/desktop/src/yandex.rs:141`; `crates/app/src/yandex.rs:425`; `crates/desktop/qml/YandexPanel.qml:42`

### UI-195 [low/shadowed-key] Double-click's undo of the first click's toggle is keyed on the second click's Shift state only — PARTIAL

- Verdict: PARTIAL
- Note: ResultTile.qml:101-104 toggles on each onClicked, and onDoubleClicked (ResultTile.qml:108-112) undoes the toggle only when Shift is off. Whether MouseArea emits onClicked for the second press of a double-click (the comment at ResultTile.qml:106 says it does not) was not checked in qquickmousearea.cpp. The consequence depends on that.
- Confidence: high
- Trigger: A double-click where Shift is held on only one of the two clicks. The first click's effect is not undone correctly. Shift on the first click only: the first click range-fills, and the second (plain) click toggles the tile again. Shift on the second click only: the first click's plain toggle is never undone, because the doubleClicked handler skips toggled when Shift is set.
- Consequence: Opening a result to look at it (double-click) leaves the pick changed, or flips it, in a way the comment at ResultTile.qml:106-107 says cannot happen. The count and badge are updated, so the wrong state is visible but may go unnoticed.
- Sites: `crates/desktop/qml/ResultTile.qml:101`; `crates/desktop/qml/ResultTile.qml:109`; `crates/desktop/qml/ResultTile.qml:106`

### UI-196 [low/missing-keyboard] The result list has no keyboard path to pick, and no keyboard path to open the full-size preview — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-193)
- Note: Duplicate of UI-193 for the key path: YandexPanel.qml:97-102 handles only Close, and YandexPreview's Pick (YandexPreview.qml:107) is reachable only through a mouse double-click on a tile.
- Confidence: high
- Trigger: A keyboard-only user opens the Yandex panel and presses Space, arrow keys or Enter on a result. The panel's Keys handler accepts only the Close action (Escape). ResultTile has no focus or key handling, so the only route to togglePick or the preview is a mouse click or double-click.
- Consequence: A keyboard user cannot pick any result to save, and cannot reach the preview where Space does pick. The mouse and keyboard paths do not reach the same result.
- Sites: `crates/desktop/qml/YandexPanel.qml:97`; `crates/desktop/qml/YandexPanel.qml:98`; `crates/desktop/qml/YandexPreview.qml:107`

### UI-197 [low/unhandled-close] Quitting while a save runs cuts it short with no record of what was saved — CONFIRMED

- Verdict: CONFIRMED
- Note: app.rs:156-158 logs a warning when yandex.stop() returns false and then carries on with shutdown. Nothing reaches the user or the inbox, so an unfinished save is not reported.
- Confidence: medium
- Trigger: The user picks several results, starts Save, and quits the app while the files are still being written or fetched.
- Consequence: Yandex::stop waits at most 2 s, then the app shuts down while the save worker is still running. The remaining picked files are never written, and no 'saved N of M' status is shown. Files already written are complete, because write_new renames a temp file into place. This is a documented trade-off (a slow request is left to end with the process), but the user is not told the save was incomplete.
- Sites: `crates/app/src/app.rs:156`; `crates/app/src/yandex.rs:498`; `crates/app/src/yandex/images.rs:194`

## U51: Select all of a Yandex group

Entry: `crates/desktop/qml/YandexPanel.qml:81 onClicked -> yandex.setPicked(first, count, picked)`

### UI-198 [medium/wrong-message] Second photo's search is silently dropped while busy; old results stay shown — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-182)
- Note: Duplicate of UI-182 and UI-188 for the viewer's show path: YandexPanel.qml:31-34 opens the panel and calls searchFile, and search_photo returns on Ok(false) (yandex.rs:141) while busy (app/yandex.rs:425-426) without changing status or results.
- Confidence: high
- Trigger: A save or a search is running, the user closes the panel, and opens Find similar on a different photo. show(fileId) calls openPanel() then searchFile, and start() returns Ok(false) because busy is set, so search_photo returns without a status or a change to the results.
- Consequence: The panel shows the previous photo's results and picks under the Find similar title, with the running job's status and no notice that the new photo was not searched. Select all, the picks and Save then act on the old photo's results as if they were the new photo's.
- Sites: `crates/desktop/qml/YandexPanel.qml:31`; `crates/desktop/src/yandex.rs:140`; `crates/app/src/yandex.rs:425`

### UI-199 [low/missing-busy-guard] Group Select all/none stays enabled during a save and changes the Save count — CONFIRMED

- Verdict: CONFIRMED
- Note: YandexPanel.qml:81 calls setPicked with no busy check, and yandex.rs:437-441 applies it unconditionally. Only the Save button is disabled while busy (YandexPanel.qml:165). The snapshot behaviour of the running save was not read.
- Confidence: medium
- Trigger: A save is running (busy true, Save button disabled). The user clicks a group's Select all or Select none, which calls setPicked with no busy check.
- Consequence: The picks change while the save runs on its snapshot, so the Save label shows a count that does not match the files being written. The change is not applied to the save in progress, and the panel gives no notice.
- Sites: `crates/desktop/qml/YandexPanel.qml:81`; `crates/desktop/qml/YandexPanel.qml:165`; `crates/desktop/src/yandex.rs:437`

### UI-200 [low/stale-state] Picks are discarded on close and reopen because each open starts a new search — CONFIRMED

- Verdict: CONFIRMED
- Note: A new open runs search_photo, which calls show(Results::default()) (yandex.rs:147), so the picks reset. close() only hides the panel (YandexPanel.qml:49-52) and gives no message.
- Confidence: medium
- Trigger: The user picks results (with Select all or single clicks), closes the panel without saving, and opens Find similar again for the same photo. show() runs a new search, and search_photo resets the results and picks to none.
- Consequence: The picks are lost without a message. The user re-picks, and each reopen sends the photo to Yandex again.
- Sites: `crates/desktop/qml/YandexPanel.qml:49`; `crates/desktop/src/yandex.rs:147`

## U52: Yandex full-size preview

Entry: `crates/desktop/qml/ResultTile.qml:36 previewRequested via double-click (ResultTile.qml:108) -> YandexPanel.qml:225, :247 preview.open (YandexPreview.qml:51)`

### UI-201 [low/double-fire] Held Space auto-repeats the pick toggle in the preview — CONFIRMED

- Verdict: CONFIRMED
- Note: YandexPreview.qml:99-110 handles every press, and keys.rs:191 maps Space to Pick. There is no isAutoRepeat check, so a held Space toggles the pick on each repeat.
- Confidence: high
- Trigger: The user holds Space while the Yandex preview is open. Qt delivers auto-repeat key presses to Keys.onPressed, and the handler has no isAutoRepeat guard, so each repeat calls pickRequested again and togglePick flips the pick back and forth.
- Consequence: The pick state changes several times while the user holds the key, and the final state depends on how long the key was held. The badge and the Picked button show the true state, so nothing is shown wrongly, but the user can pick or unpick results unintentionally.
- Sites: `crates/desktop/qml/YandexPreview.qml:99`; `crates/desktop/qml/YandexPreview.qml:107`; `crates/client/src/keys.rs:191`

### UI-202 [low/unintended-pick] Shift+double-click applies a range pick and keeps it — CONFIRMED

- Verdict: CONFIRMED
- Note: ResultTile.qml:100-104 runs rangeTo on the first Shift click, and that is not undone. onDoubleClicked (ResultTile.qml:108-112) skips the toggle-back when Shift is held, so the range pick stays, which contradicts the comment at ResultTile.qml:106-107.
- Confidence: high
- Trigger: The user Shift+double-clicks a result. The first Shift click runs rangeTo, which sets every result between the anchor and the tile to the anchor's state. The second press is a Shift double-click, which skips the toggle-back and only opens the preview.
- Consequence: The range pick is applied and stays, although the comment says looking at a result does not change the picks. Earlier picks in the range can be overwritten without the user meaning to change them.
- Sites: `crates/desktop/qml/ResultTile.qml:100`; `crates/desktop/qml/ResultTile.qml:108`; `crates/desktop/qml/ResultTile.qml:106`

### UI-203 [low/keyboard-parity] The 1:1 zoom has no keyboard path — CONFIRMED

- Verdict: CONFIRMED
- Note: YandexPreview.qml:155-160 sets NoFocus on the 1:1 button, and the panel key map (keys.rs:186-193) has no zoom or real-size action. Only a mouse click or a tap reaches 1:1.
- Confidence: high
- Trigger: A keyboard user opens a result in the preview and wants to switch to 1:1. The 1:1 button has focusPolicy NoFocus and no key is mapped to zoom, so only a mouse click or a tap on the picture reaches 1:1.
- Consequence: Keyboard-only users cannot reach the 1:1 view that the mouse path provides. The viewer has a real-size key (Z), but the Yandex preview has none.
- Sites: `crates/desktop/qml/YandexPreview.qml:155`; `crates/desktop/qml/YandexPreview.qml:158`; `crates/client/src/keys.rs:186`

### UI-204 [low/missing-feedback] Preview closes silently when its picture turns out dead — CONFIRMED

- Verdict: CONFIRMED
- Note: images.rs:187-189 publishes a change when a result settles dead. yandex.rs:260 sets the preview item to -1 when its picture is gone, and YandexPanel.qml:288-289 closes the preview. The status line is not changed on this path. The short "Can't load or decode it" label was not traced.
- Confidence: high
- Trigger: The user double-clicks a result whose file is still being fetched in the background (prefetch). The fetch or its decode fails, so Images::settle marks the result Dead and publishes a change. The poll then calls drop_dead, which sets the preview's item to -1 because the shown picture is gone.
- Consequence: The preview closes and the tile vanishes from the grid. The status line does not change and no reason is shown. The 'Can't load or decode it' label appears only until the next poll.
- Sites: `crates/app/src/yandex/images.rs:187`; `crates/desktop/src/yandex.rs:260`; `crates/desktop/qml/YandexPanel.qml:288`

## U53: Save picked Yandex results (dialog)

Entry: `crates/desktop/qml/YandexPanel.qml:164-170 Save button onClicked -> savePicker.pick(2)`

### UI-205 [medium/stale-result] Find-similar during a save shows the previous results, with no message — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-198)
- Note: Duplicate of UI-198 with the viewer's entry point: Main.qml:377-379 sets yandexLoader and calls similarAsked, then search_photo returns on Ok(false) (yandex.rs:141) while busy (app/yandex.rs:425-426), and the panel shows the previous results (YandexPanel.qml:42-47).
- Confidence: high
- Trigger: A save is running (busy). The user closes the Yandex panel (Close or Escape; the save keeps going), goes to the viewer on a different photo, and asks for find-similar. The viewer's request reaches search_photo, which returns Ok(false) and returns silently.
- Consequence: The panel reopens with the previous search's results (the other photo's) under the find-similar title, while the status line shows the save's progress. The requested photo is never searched and nothing says so. If the photo's catalog lookup fails (search_file), the error status is set but the stale results stay on screen the same way.
- Sites: `crates/desktop/src/yandex_bridge.rs:141`; `crates/app/src/yandex.rs:425`; `crates/desktop/qml/YandexPanel.qml:44`; `crates/desktop/qml/Main.qml:379`

### UI-206 [low/wrong-message] Picks that die while the folder dialog is open are dropped silently, and if all die the save says Pick at least one image — PARTIAL

- Verdict: PARTIAL
- Note: Citation error: yandex_bridge.rs:449, 453 and 455 are past the end of the file (LINE OUT OF RANGE). The save path could not be checked at those lines. The general claim that a pick dying while the folder dialog is open is dropped is consistent with drop_dead (yandex.rs:241) and the poll (yandex.rs:372-374), but the empty-set path into "Pick at least one image" was not read.
- Confidence: medium
- Trigger: A pick's file fails to fetch while the Save folder dialog is open. poll runs during the dialog (Qt events run on the Qt thread during the dialog's own loop) and drop_dead removes the row and its pick. If that was the only pick, download_picked reads an empty set after take has already consumed the folder.
- Consequence: The user is told 'Pick at least one image to save.' although they picked one. The chosen folder is discarded and must be picked again. With several picks the rest are saved and the dead one vanishes from the grid with no message.
- Sites: `crates/desktop/src/yandex_bridge.rs:449`; `crates/desktop/src/yandex_bridge.rs:453`; `crates/desktop/src/yandex_bridge.rs:455`

### UI-207 [low/missing-feedback] download_picked returns silently when the save is refused, after the folder is consumed — PARTIAL

- Verdict: PARTIAL
- Note: Citation error: yandex_bridge.rs:464 is past the end of the file (LINE OUT OF RANGE). The app side is confirmed: save returns Ok(false) silently when busy or closed (app/yandex.rs:539-541), so the refusal is not reported. The bridge side that discards the folder was not read.
- Confidence: medium
- Trigger: search().save returns Ok(false): busy or closed. The UI disables Save while busy and the viewer cannot be reached during the dialog, so this is latent; it is reachable only when the app is stopping (closed).
- Consequence: The chosen folder is discarded and the status line is left as it was, so the user sees nothing happen after choosing a folder.
- Sites: `crates/desktop/src/yandex_bridge.rs:464`; `crates/app/src/yandex.rs:539`

### UI-208 [low/wrong-message] Failed save with nothing written reads Saved 0, 3 failed — CONFIRMED

- Verdict: CONFIRMED
- Note: text.rs:346-350 renders YandexSavedFailed with written = 0 when nothing was written, and text.ftl:157 reads "Saved { $written }, { $failed } failed". The string starts with "Saved 0".
- Confidence: high
- Trigger: Every file of the save fails (for example the folder is read-only or was removed after it was chosen).
- Consequence: The status line begins with 'Saved' for a save that wrote nothing. The failure is reported, but the wording suggests partial success.
- Sites: `crates/client/src/text.rs:346`; `crates/client/locales/en/text.ftl:157`

### UI-209 [low/missing-feedback] In-app folder dialog's Up or open failure is shown only in the dimmed status line behind it — CONFIRMED

- Verdict: CONFIRMED
- Note: picker.rs:170-176 calls report_unlisted on an unlistable folder, which emits failed (picker.rs:160-166). The save picker forwards it to showFailure (YandexPanel.qml:125), so the message goes to the panel's status line. The scrim (ModalOverlay.qml:26, #b0000000) dims that line under the dialog. The Windows-only gate (picker.rs:16) was not read.
- Confidence: high
- Trigger: Off Windows only (NATIVE is cfg!(windows), platform picker.rs:16). The user clicks Up or opens a folder that can't be listed.
- Consequence: The dialog stays on the old folder with no message of its own. The error goes to the panel's status line, which sits under the dialog's dimmed scrim, so it is hard to see while the dialog is open.
- Sites: `crates/desktop/src/picker.rs:174`; `crates/desktop/qml/YandexPanel.qml:125`; `crates/desktop/qml/ModalOverlay.qml:26`; `crates/desktop/qml/FolderPickerDialog.qml:51`

## U54: Close the Yandex panel and its preview

Entry: `crates/desktop/qml/YandexPanel.qml:171 Close button and :107 background onClicked -> close (YandexPanel.qml:49); :97-102 Keys (Escape); YandexPreview.qml:161 Back button and :51-57 open/close`

### UI-210 [high/stale-result] Refused Find similar leaves the previous photo's results on screen — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-182)
- Note: Duplicate of UI-182 and UI-188 for the close-then-reopen case: YandexPanel.qml:31-34 opens the panel, yandex.rs:141 returns on Ok(false) while busy (app/yandex.rs:425-426), and the panel keeps A's results and state.
- Confidence: high
- Trigger: Find similar on photo A, then close the panel (Escape or Close) while A's search is still uploading or working. Open photo B in the viewer and click Find similar before A's search ends. The app refuses B's search because one is running, and the panel opens showing A's busy state.
- Consequence: The panel shows A's spinner, status and, once A's search finishes, A's similar results, with no message that B's search never started. The user reads A's results as B's. Nothing in the panel names the photo it is showing.
- Sites: `crates/desktop/qml/YandexPanel.qml:33`; `crates/desktop/src/yandex.rs:141`; `crates/app/src/yandex.rs:425`

### UI-211 [low/focus] Hidden save-folder dialog keeps focus, so Escape no longer closes the Yandex panel — CONFIRMED

- Verdict: CONFIRMED
- Note: The same Qt rule (qquickitem.cpp:6641-6645, 6669-6681) means the hidden overlay keeps activeFocus, so Escape reaches ModalOverlay's Keys.onEscapePressed (ModalOverlay.qml:22) and not YandexPanel's handler.
- Confidence: medium
- Trigger: On a platform where FolderPicker.native is false (not Windows): Save picked opens the in-app folder dialog, then Choose, Cancel, Escape or a backdrop click hides it. The hidden ModalOverlay still holds keyboard focus, and Qt keeps delivering keys to it.
- Consequence: The hidden overlay consumes Escape (dismissed calls cancel again) and never reaches YandexPanel's Keys handler, so Escape does not close the panel and the arrows and Space do not reach the preview. Only a click that takes focus recovers the keys. Mouse use still works. On Windows this path does not occur because the native dialog is used.
- Sites: `crates/desktop/qml/ModalOverlay.qml:21`; `crates/desktop/qml/ModalOverlay.qml:22`; `crates/desktop/qml/YandexPanel.qml:127`; `crates/desktop/qml/FolderPickerDialog.qml:85`; `crates/platform/src/picker.rs:16`

### UI-212 [low/stale-result] A failed file lookup keeps the previous search's results beside the new error — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-183)
- Note: Duplicate of UI-183 and UI-189: yandex.rs:123-124 sets only the status on a failed lookup, and the earlier results stay in the panel (YandexPanel.qml:31-34).
- Confidence: medium
- Trigger: The viewer's photo id is no longer in the catalog when Find similar is clicked, while results from an earlier search are still in the panel. file_path fails in search_file.
- Consequence: The panel shows the error for the requested photo under the old photo's results, which look like they belong to it. Latent: the viewer normally shows a file the catalog has.
- Sites: `crates/desktop/src/yandex.rs:124`; `crates/desktop/qml/YandexPanel.qml:33`

## U55: Yandex thumbnails settle

Entry: `crates/desktop/qml/ResultTile.qml:55 onStatusChanged -> thumbSettled (ResultTile.qml:38) -> YandexPanel.qml:222, :244 yandex.thumbSettled`

### UI-213 [medium/unhandled-close] Closing the window waits for an in-flight Yandex download before the process exits — CONFIRMED

- Verdict: CONFIRMED
- Note: lib.rs:221-224 runs app::stop and then drop(engine) before app::shutdown. providers.cpp:131-135 calls waitForDone on the pool, which blocks while a fetch is running. Yandex::stop cannot cut a request short (app/yandex.rs:100-102), so the process waits for the download to end. The 15-minute body timeout was not read.
- Confidence: high
- Trigger: The user closes the app while a Yandex result's file is being downloaded for a grid thumbnail (the turn row) or the full-size preview (the original). The provider job is blocked in Images::get -> Fetcher download.
- Consequence: The window disappears but the process stays alive until the download finishes or its body timeout (15 minutes) fires. That looks like a hang. Catalog connections and the WAL merge in app::shutdown wait too, since they run after drop(engine). The stop comment in yandex.rs says a slow request is left to end with the process, which is not what happens.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/lib.rs:223`; `D:/GitHub/photo-gallery/crates/desktop/cpp/providers.cpp:131`; `D:/GitHub/photo-gallery/crates/desktop/cpp/providers.cpp:135`; `D:/GitHub/photo-gallery/crates/app/src/yandex.rs:100`; `D:/GitHub/photo-gallery/crates/app/src/yandex/images.rs:194`

### UI-214 [low/missing-feedback] Results whose files fail are removed silently; the status line never says so — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-186)
- Note: Duplicate of UI-186 and UI-191 with the status-line point: yandex.rs:372-374 drops dead results without a status, and YandexPanel.qml:255 shows "No results" when every result is gone and the status is empty.
- Confidence: high
- Trigger: After a search finishes, a result's file cannot be fetched or decoded (a dead link). The result is taken off the grid, and a picked one loses its pick. The trigger is a network or host failure, not a user action.
- Consequence: Group counts and the Save count drop with no message. The status line is empty after a clean search. If every result dies, the panel shows 'No results' although the search found some. A partial result is not reported as partial.
- Sites: `D:/GitHub/photo-gallery/crates/app/src/yandex/images.rs:178`; `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:372`; `D:/GitHub/photo-gallery/crates/app/src/yandex.rs:462`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:255`

### UI-215 [low/wrong-state] The --yandex-preview hook opens the preview after a failed or empty search — CONFIRMED

- Verdict: CONFIRMED
- Note: YandexPanel.qml:58-63 opens the preview on any busy change to false, with no check that results exist. app/yandex.rs:464-468 clears busy after a failed search as well. Test hook only, as the finding says.
- Confidence: high
- Trigger: Run with --yandex-preview=N and a search that fails or finds nothing. busy goes false with no results.
- Consequence: The preview opens at index N with no result behind it. Its label reads a similar-image count of 0, and the picture is empty. This is test-hook only; a normal user does not reach it.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:59`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:60`; `D:/GitHub/photo-gallery/crates/app/src/yandex.rs:464`

## U56: Mascot click

Entry: `crates/desktop/qml/Mascot.qml:38 onClicked (mouse.button right -> previous, else next) -> mascot.step (Mascot.qml:19)`

### UI-216 [low/blank-flash] Mascot blanks on each click until the new PNG decodes — PARTIAL

- Verdict: PARTIAL
- Note: Mascot.qml:27 sets asynchronous, and the source changes on each click (the number changes, Mascot.qml:30). Whether retainWhileLoading defaults to false was not read in qquickimage.cpp, and the click handler (Mascot.qml:34-) was not read.
- Confidence: medium
- Trigger: Click the mascot to show a picture whose PNG is not in Qt's pixmap cache (first time shown, or after the cache has released it). Image is asynchronous with retainWhileLoading at its default false.
- Consequence: For the decode time (a few frames at most for a qrc PNG of about 120-150 KB) the 200 px mascot area is empty after each click, then the new picture appears. This is a cosmetic flicker and does not lose data or state.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/Mascot.qml:27`; `D:/GitHub/photo-gallery/crates/desktop/qml/Mascot.qml:30`

## U57: Status bar toast

Entry: `crates/desktop/qml/StatusBar.qml:107-111 Timer -> toastExpired (StatusBar.qml:39) -> Main.qml:562 inboxModel.dismissToast`

### UI-217 [medium/wrong-message] Toast shows the oldest of a batch of new notices, not the newest — PARTIAL

- Verdict: PARTIAL
- Note: notices.rs:359 shows toasts.last(), and inbox.rs:1060-1071 publishes the toasts in listing order. The claim that the listing is newest first was not checked, so whether last() is the oldest depends on that sort.
- Confidence: high
- Trigger: Two notices that turn unread in the same writer batch (for example a ledger turning unread and a kept event posted while the writer is busy with a scan's batch). refresh publishes the Toast events in listing order, which is newest first, and on_pass shows toasts.last().
- Consequence: The status bar toast names the older notice as the newest one, contrary to the documented 'the newest notice, as a toast'. The user reads the wrong notice's title. The newer one is still in the inbox, so nothing is lost, but the toast misleads.
- Sites: `D:/GitHub/photo-gallery/crates/app/src/inbox.rs:1060`; `D:/GitHub/photo-gallery/crates/app/src/inbox.rs:1071`; `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:359`

### UI-218 [low/timer-not-restarted] A second info toast is cut short by the first toast's timer — CONFIRMED

- Verdict: CONFIRMED
- Note: StatusBar.qml:107-112 binds running to toast length, level 0 and not held. A text change does not change running, so the timer keeps its first deadline and the second toast can be cut short.
- Confidence: high
- Trigger: An info-level toast is shown, then another info-level notice arrives within 6 s of the first (the toast text changes, so toastLevel and the running binding stay the same).
- Consequence: The Timer keeps the first toast's deadline, so the new toast disappears early (for example 0.5 s after it appears when it arrives 5.5 s into the first). The inbox's unread badge still shows it, so the notice is kept, but the toast does not get the 6 s the inbox header promises.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/StatusBar.qml:110`; `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:429`

### UI-219 [low/stale-toast] A warning toast about an already-read notice survives closing the inbox — CONFIRMED

- Verdict: CONFIRMED
- Note: StatusBar.qml:110 runs the toast timer only for level 0, so a warning stays until it is dismissed. Main.qml:66 dismisses the toast only on open, and Main.qml:70 closes the inbox with setViewing and no dismissal. notices.rs:466 sets marking on close, which covers the read state.
- Confidence: medium
- Trigger: The inbox is open in an active window when a warning-level state notice is posted. The writer toasts it (news ignores viewing), mark_shown_read marks it read at once, and the toast stays up because only opening the inbox dismisses it. The user then closes the inbox.
- Consequence: A warning stays in the status bar, after the inbox was closed, for a notice the user has already seen and that is already read. It stays until replaced or the inbox is opened again. The toast does not match the read state of the inbox.
- Sites: `D:/GitHub/photo-gallery/crates/app/src/inbox.rs:1038`; `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:466`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:66`

### UI-220 [low/missing-feedback] A failed spawn of the toast follower leaves the toast silently dead — CONFIRMED

- Verdict: CONFIRMED
- Note: notices.rs:344-346 logs a warning when the follower thread fails to start and does nothing else, so no toast or listing update reaches the status bar.
- Confidence: medium
- Trigger: The 'inbox view' thread cannot be started (resource exhaustion) when the window starts.
- Consequence: No toast, and no listing update, ever reaches the status bar for the rest of the session. The only signal is a warn log line; the status bar shows nothing and the unread badge stays at its start value.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:344`; `D:/GitHub/photo-gallery/crates/desktop/src/notices.rs:345`

## U58: Open the inbox (status bell, shortcut)

Entry: `crates/desktop/qml/StatusBar.qml:105 TapHandler and :118 onClicked -> inboxRequested (StatusBar.qml:40) -> Main.qml:563 shell.globalAction(Inbox); shortcut GlobalShortcuts.qml:24-30`

### UI-221 [medium/double-fire] Failed mark is re-sent on every listing with no bound — CONFIRMED

- Verdict: CONFIRMED
- Note: inbox.rs:923 sets force when a mark write fails. inbox.rs:903-906 then refreshes and publishes Changed (inbox.rs:1074). notices.rs:400-406 re-marks the shown rows while marking is set, and notices.rs:452 sends them to the writer again. No backoff or bound appears on this path.
- Confidence: high
- Trigger: The inbox is open in an active window and a mark of a shown entry fails in the writer with a non-busy database error (for example SQLITE_LOCKED, a full disk, or a read-only catalog). show() re-marks the rows still unread in the listing that follows.
- Consequence: The writer logs 'the inbox couldn't be changed', force-refreshes, and publishes Changed. The window re-marks the same rows, which fails again. The cycle has no pause and no bound: the writer and the Qt thread keep working, the warning log grows without limit, and the rows repaint on every cycle. It stops only when the inbox closes or the window deactivates.
- Sites: `crates/desktop/src/notices.rs:357`; `crates/desktop/src/notices.rs:404`; `crates/desktop/src/notices.rs:452`; `crates/app/src/inbox.rs:903`; `crates/app/src/inbox.rs:923`; `crates/app/src/inbox.rs:1074`

### UI-222 [medium/wrong-message] Bell count includes unread entries the inbox cannot list or mark — CONFIRMED

- Verdict: CONFIRMED
- Note: catalog/inbox.rs:211 lists only LIMIT 200 rows (LISTED, inbox.rs:697), but the unread count comes from the unlimited query at catalog/inbox.rs:254, counted at app inbox.rs:1051. The mark on open covers only the listed rows (notices.rs:446-452), so the badge can stay on after the listed rows are read.
- Confidence: high
- Trigger: More than 200 non-dismissed entries exist (LISTED = 200). The inbox lists the newest 200 and marks them read on open. The unread count is still computed over all entries.
- Consequence: After the open marks everything shown read, the bell still reads 'Inbox (N)' and stays highlighted, where N is the count of unlisted unread entries. Those entries have no row, so they cannot be read, marked or dismissed from the window until Clear all brings them into the list. The badge disagrees with what the user can see.
- Sites: `crates/app/src/inbox.rs:697`; `crates/app/src/inbox.rs:1051`; `crates/catalog/src/inbox.rs:211`; `crates/catalog/src/inbox.rs:254`; `crates/desktop/src/notices.rs:400`; `crates/desktop/src/notices.rs:450`

### UI-223 [low/stale-toast] Warning toast raised while the inbox is open outlives the inbox — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-219)
- Note: Duplicate of UI-219: Main.qml:66 dismisses the toast only on open, and notices.rs:359-362 sets a warning toast in the same pass that marks the entry read.
- Confidence: medium
- Trigger: A warning or error notice is published while the inbox is open in an active window. show() marks its entry read first, then on_pass sets the warning toast. Closing the inbox does not clear it, because dismissToast runs only on open.
- Consequence: After the inbox closes, a warning toast stays in the status bar for a notice the user has already seen and that is marked read. It stays until the user opens the inbox (which clears it), or until another notice replaces it. The toast rule says a warning stays until dismissed or replaced, so this is a stale notice, not a wrong one.
- Sites: `crates/desktop/src/notices.rs:359`; `crates/desktop/src/notices.rs:446`; `crates/desktop/qml/Main.qml:66`; `crates/desktop/qml/Main.qml:70`

### UI-224 [low/missing-feedback] A new info toast inherits the previous toast's timer — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-218)
- Note: Duplicate of UI-218: StatusBar.qml:107-112 keeps the running timer, so a second info toast inherits the first one's remaining time. The claim that a transient info notice is never listed in the inbox was not traced.
- Confidence: high
- Trigger: An info toast is up and its 6 s timer is running. A second info notice arrives within those 6 s. The timer's running binding stays true, so it is not restarted.
- Consequence: The second info toast vanishes after the remainder of the first one's 6 s, possibly within a second, and the user can miss it. A transient info notice is logged and toasted but never listed in the inbox, so nothing else shows it.
- Sites: `crates/desktop/qml/StatusBar.qml:109`; `crates/desktop/qml/StatusBar.qml:110`; `crates/desktop/src/notices.rs:430`

## U59: Inbox select and details

Entry: `crates/desktop/qml/InboxPanel.qml:331-337 TapHandler -> panel.select (InboxPanel.qml:101); keys InboxPanel.qml:213-241 (Up, Down, Home, End, PageUp, PageDown, Enter or Space for details, Delete, Ctrl+C, R)`

### UI-225 [medium/missing-feedback] Copy of a ledger entry copies only its title line when the read fails, with no message — CONFIRMED

- Verdict: CONFIRMED
- Note: notices.rs:512-516 maps a read error to None, and copy_words with None writes only the notice line (notices.rs:183-185). The error is logged (notices.rs:514), and nothing reaches the user.
- Confidence: high
- Trigger: Select a ledger entry (for example a failed thumbnails pass) and press Ctrl+C or click Copy while the reader's read of its failed files fails (for example 'database is locked').
- Consequence: The clipboard receives only the one-line notice. The failed-file list the user asked for is silently missing, and no toast or message says so. The user pastes it believing the copy is complete.
- Sites: `crates/desktop/src/notices.rs:513`; `crates/desktop/src/notices.rs:518`

### UI-226 [medium/stale-result] A ledger Copy still reading overwrites a later Copy's clipboard text — CONFIRMED

- Verdict: CONFIRMED
- Note: A kept notice is copied at once (notices.rs:506), while the ledger copy is queued after its read (notices.rs:520). The ledger write can land after a later copy, and nothing checks the order. Whether the reader is busy during a listing was not traced, but the ordering does not depend on it.
- Confidence: high
- Trigger: Press Copy on a ledger entry. Before its read is delivered (it can wait behind a running listing), select a kept notice and press Copy. The notice is written to the clipboard at once, then the ledger's queued write runs last and replaces it.
- Consequence: The clipboard ends up holding the older ledger text rather than the notice the user last copied. Pasting gives the wrong content with no sign of the mismatch.
- Sites: `crates/desktop/src/notices.rs:506`; `crates/desktop/src/notices.rs:520`

### UI-227 [low/missing-feedback] A failed Reveal of a failed file does nothing visible — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.rs:353 toasts only for Ok(None) (FileNoLongerInLibrary). shell.rs:354 logs the Err case and does nothing else, so a failed read leaves the inbox unchanged with no message.
- Confidence: high
- Trigger: Click Reveal on a file row while the catalog read that finds its folder fails. Only the Ok(None) case (file no longer in library) toasts.
- Consequence: The inbox stays open and nothing changes. The user sees no message and cannot tell why Reveal did nothing. The failure is logged only.
- Sites: `crates/desktop/src/shell.rs:354`

### UI-228 [low/missing-feedback] A failed Retry or Dismiss write is only logged, so the user gets no explanation — PARTIAL

- Verdict: PARTIAL
- Note: inbox.rs:922-927 forces a refresh, adds to self.lost and logs a warning, which matches the finding. Whether self.lost is shown anywhere in the UI was not read, so "only logged" is not settled.
- Confidence: high
- Trigger: Click Retry, or dismiss an entry, when the inbox writer's write fails with a non-busy catalog error.
- Consequence: A failed Retry leaves the entry as it was with no message. A failed Dismiss makes the row reappear on the forced re-read with no explanation. The user cannot tell that the action failed.
- Sites: `crates/app/src/inbox.rs:922`

### UI-229 [low/focus] A footer button click takes keyboard focus off the list, so the list's keys stop working — CONFIRMED

- Verdict: CONFIRMED
- Note: InboxPanel.qml:481-501: the footer buttons take focus on click (qquickabstractbutton.cpp:109, read for UI-143). Keys then go to the focused button, not the list. Disabling a focused button drops its focus (the UI-086 mechanism, qquickitem.cpp:6835-6838). Escape reaches the panel's own handler.
- Confidence: high
- Trigger: Click Copy, then press Down, Delete or R: the focused Copy button receives the keys. Or click Dismiss (or Clear all): the button becomes disabled while focused, so Qt hands focus to the enclosing focus scope.
- Consequence: Up, Down, Home, End, PageUp, PageDown, Delete, R, and Enter or Space (which then re-presses the focused Copy button) no longer reach the entry list. Escape still closes the panel. The user must click the list or press Tab to recover.
- Sites: `crates/desktop/qml/InboxPanel.qml:481`; `crates/desktop/qml/InboxPanel.qml:490`; `crates/desktop/qml/InboxPanel.qml:497`

## U60: Copy an inbox entry

Entry: `crates/desktop/qml/InboxPanel.qml:485 onClicked copyRequested; Ctrl+C at InboxPanel.qml:233-235`

### UI-230 [medium/stale-result] Late ledger copy overwrites a newer kept-notice copy on the clipboard — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-226)
- Note: Duplicate of UI-226: the kept-notice copy (notices.rs:506) and the late ledger copy (notices.rs:520) are the same race.
- Confidence: high
- Trigger: The user Copies a ledger entry (Ctrl+C or the Copy button), and before its read is delivered copies a kept notice row. The kept-notice copy writes the clipboard at once (notices.rs:506). The reader is single-threaded and may be busy with a listing or another inbox read, so the ledger read finishes later. Its queued write (notices.rs:520) then overwrites the clipboard with the ledger's text.
- Consequence: The user's last copy is the ledger's text, not the notice they chose. Pasting gives the wrong entry, and nothing signals that the clipboard changed.
- Sites: `crates/desktop/src/notices.rs:506`; `crates/desktop/src/notices.rs:520`

### UI-231 [medium/missing-feedback] Copy of a ledger silently truncates at 1000 items with no note — CONFIRMED

- Verdict: CONFIRMED
- Note: notices.rs:142 caps the read at COPIED_FILES = 1000, and notices.rs:512 keeps only failed.items. The counts are dropped, so no "newest N of M" line is added to the copy, unlike the panel's ui-inbox-more.
- Confidence: high
- Trigger: Copy a ledger entry (a pass's failed files, or the scans' folders) that holds more than 1000 rows. The read is limited to COPIED_FILES and the counts are dropped, so the clipboard gets 1000 lines and nothing says the rest are missing.
- Consequence: The user takes the copied text for the full list when it is partial. The inbox panel itself says "The newest N of M files" (ui.ftl ui-inbox-more), but the copy has no equivalent line.
- Sites: `crates/desktop/src/notices.rs:142`; `crates/desktop/src/notices.rs:512`; `crates/desktop/src/notices.rs:182`

### UI-232 [medium/wrong-message] A failed ledger read copies the headline alone and reports success — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-225)
- Note: Duplicate of UI-225: the same Err arm (notices.rs:513-516) gives None, and copy_words writes only the headline (notices.rs:183-185). Nothing reports the failure.
- Confidence: high
- Trigger: Copy a ledger entry while its catalog read fails: the connection can't open, the read errors, or it panics. The reader delivers the error, the Err arm logs it, and copy_words runs with no items.
- Consequence: The clipboard gets only the notice headline, and the success is silent. The user has no way to tell that the failed files or folders were left out.
- Sites: `crates/desktop/src/notices.rs:513`; `crates/desktop/src/notices.rs:514`; `crates/desktop/src/notices.rs:518`

### UI-233 [low/focus] After a mouse Copy, the list's keys stop responding because focus stays on the button — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-229)
- Note: Duplicate of UI-229: InboxPanel.qml:481-486 is a Button, which takes focus on press (qquickabstractbutton.cpp:1197-1198), and the list's Ctrl+C handler (InboxPanel.qml:233) gets no key while the button has focus.
- Confidence: high
- Trigger: Click the Copy button (inboxCopy) in the inbox, then press Ctrl+C, Up or Down. Pressing a Button gives it active focus (Qt's AbstractButton is StrongFocus and forceActiveFocus on press). The button ignores these keys, so they go up to the overlay, which handles only Escape.
- Consequence: Keyboard navigation and Ctrl+C do nothing until the user clicks a row or Tabs back. The copy itself still works.
- Sites: `crates/desktop/qml/InboxPanel.qml:481`; `crates/desktop/qml/InboxPanel.qml:485`; `crates/desktop/qml/ModalOverlay.qml:22`; `C:\Qt\6.12.0\Src\qtdeclarative\src\quicktemplates\qquickabstractbutton.cpp:107`

### UI-234 [low/shadowed-key] Clicking the details text takes focus and Ctrl+C no longer copies the entry — CONFIRMED

- Verdict: CONFIRMED
- Note: InboxPanel.qml:351-355 is a readOnly TextArea, which takes focus. qquicktextcontrol.cpp:833-836 accepts QKeySequence::Copy and copies the selection only, so the list handler at InboxPanel.qml:233 never runs.
- Confidence: medium
- Trigger: The details of a kept notice are open, and the user clicks in the details text (inboxDetails). The readOnly TextArea takes focus, and Ctrl+C goes to it. The text control handles QKeySequence::Copy by copying only the selection, so the list's Ctrl+C handler never runs. With no selection, nothing is copied. Delete and the arrow keys are shadowed the same way.
- Consequence: Ctrl+C copies just the selected words, or nothing, where the user expects the entry. The keyboard path to Copy no longer matches the mouse path. Nothing reports the difference.
- Sites: `crates/desktop/qml/InboxPanel.qml:351`; `crates/desktop/qml/InboxPanel.qml:233`; `C:\Qt\6.12.0\Src\qtdeclarative\src\quick\items\qquicktextcontrol.cpp:833`

### UI-235 [low/missing-feedback] Copy gives no confirmation, and a ledger copy can land after the user has pasted — CONFIRMED

- Verdict: CONFIRMED
- Note: InboxPanel.qml:481-485 leaves the button enabled and shows no busy state. The clipboard write is queued only after the read (notices.rs:510-520), so the write can land after a paste made in between.
- Confidence: high
- Trigger: Click Copy or press Ctrl+C on a ledger entry while the reader is busy. Nothing changes on screen (no busy state, no toast, the button stays enabled). The clipboard is written only when the read is delivered, so a paste made in the meantime gets the previous clipboard contents.
- Consequence: The user gets no sign that the copy is pending or has finished, and a quick paste returns the old clipboard.
- Sites: `crates/desktop/qml/InboxPanel.qml:481`; `crates/desktop/src/notices.rs:510`

### UI-236 [low/invisible-characters] Clipboard text for a kept notice carries bidi isolate controls — CONFIRMED

- Verdict: CONFIRMED
- Note: text/notice.rs:21-24 wraps a value in U+2068 and U+2069 through isolated(). copy_words takes notice_line unchanged (notices.rs:183, text.rs:325-329), and nothing strips the characters. Which notice kinds call isolated() was not checked.
- Confidence: medium
- Trigger: Copy a kept notice whose title or body names a path (for example a folder already added, or a scan failure). Its title and body come from args(), which wraps paths in U+2068 and U+2069 through isolated(). copy_words takes notice_line unchanged, so the clipboard gets those invisible characters. Failed-file lines written by the same copy are not wrapped, so the convention is mixed.
- Consequence: Pasting the copied text into another program can carry invisible characters around the path, which can break a paste into a path field or a shell. The copy stays readable.
- Sites: `crates/client/src/text/notice.rs:21`; `crates/client/src/text.rs:325`; `crates/desktop/src/notices.rs:183`

## U61: Dismiss an inbox entry, clear all

Entry: `crates/desktop/qml/InboxPanel.qml:491 onClicked dismissSelected (-> dismissRequested, InboxPanel.qml:52); InboxPanel.qml:498-501 Clear onClicked -> unselect and clearRequested (InboxPanel.qml:54)`

### UI-237 [medium/stale-state] Clear all leaves the entries past 200 that show at once after it — CONFIRMED

- Verdict: CONFIRMED
- Note: notices.rs:489 takes the ids of the rows the window holds, and the writer clears those. catalog/inbox.rs:211 lists only the first 200 rows afterwards, so the rest appear after the clear. The unread count covers all entries (app inbox.rs:1051). The 1000-event retention was not read.
- Confidence: high
- Trigger: The inbox holds more than LISTED (200) kept entries (events are kept up to 1000) and the user clicks Clear all. The window names only the 200 rows it holds (notices.rs:489), the writer dismisses those, then its refresh (app/inbox.rs:903-905) lists what is left.
- Consequence: The panel empties and then, within a moment, shows the remaining entries (up to the rest of the inbox), and the status bar's unread count is set to count them. Clear all looks like it failed for those entries. The label 'Clear all' (ui.ftl:99) and the 'Clear all dismisses the entries displayed' contract are accurate only for the first 200.
- Sites: `crates/desktop/src/notices.rs:489`; `crates/catalog/src/inbox.rs:211`; `crates/catalog/src/inbox.rs:254`; `crates/app/src/inbox.rs:1051`; `crates/app/src/inbox.rs:1057`; `crates/app/src/inbox.rs:903`

### UI-238 [medium/focus] Mouse Dismiss or Clear all drops keyboard focus from the panel — CONFIRMED

- Verdict: CONFIRMED
- Note: InboxPanel.qml:490 disables Dismiss when selected is -1, and :497 disables Clear when list.count is 0. Both change in the same handler, and a button that is disabled while focused loses focus (qquickitem.cpp:6835-6838). The row tap restores focus (InboxPanel.qml:333), but these two paths do not.
- Confidence: medium
- Trigger: With the panel open the user clicks Dismiss (or Clear all). A Qt Controls button takes focus on press, and the same handler then disables it: Dismiss through enabled: panel.selected >= 0 after unselect(), Clear through enabled: list.count > 0 after the reset.
- Consequence: Qt Quick clears activeFocus within the item's focus scope when the focused item becomes disabled, and gives focus to no other item (QQuickItemPrivate::setEffectiveEnableRecur calls clearFocusInScope). The list then stops getting Up, Down, Home, End, Delete and R, and Escape stops closing the panel, until the user clicks a row or Close. The row tap path restores focus explicitly (InboxPanel.qml:333); the Dismiss and Clear paths do not.
- Sites: `crates/desktop/qml/InboxPanel.qml:490`; `crates/desktop/qml/InboxPanel.qml:497`; `crates/desktop/qml/InboxPanel.qml:333`; `crates/desktop/qml/ModalOverlay.qml:22`; `C:/Qt/6.12.0/Src/qtdeclarative/src/quicktemplates/qquickabstractbutton.cpp:1197`; `C:/Qt/6.12.0/Src/qtdeclarative/src/quick/items/qquickitem.cpp:6835`

### UI-239 [low/stale-state] A dismissed ledger that comes back reopens and reselects itself while the panel is open — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:68 clears inboxExpandPass only when the inbox closes. Main.qml:833-834 drives expandRow from the passes bit, and InboxPanel.qml:145 runs expand() on each change. expand() resets expanded when expandRow goes to -1 (InboxPanel.qml:134-136), so when the ledger returns it moves the selection and opens its details (InboxPanel.qml:138-142).
- Confidence: high
- Trigger: The inbox is opened from the status bar's 'N failed' link for a ledger (inboxExpandPass set). The user dismisses that ledger while the panel stays open. Later, while the panel is still open, a new failure of that pass makes the ledger listed again.
- Consequence: expandRow goes -1 on the dismissal (its passes bit clears) and expanded is reset, so when the ledger returns expand() selects its row, scrolls to it and opens its details, taking the selection away from the entry the user is on. inboxExpandPass is only reset when the inbox closes, so the request outlives the dismissal it was made for.
- Sites: `crates/desktop/qml/Main.qml:68`; `crates/desktop/qml/Main.qml:833`; `crates/desktop/qml/InboxPanel.qml:134`; `crates/desktop/qml/InboxPanel.qml:138`; `crates/desktop/qml/InboxPanel.qml:145`

### UI-240 [low/stale-result] A listing computed before a dismissal can put the dismissed entry back until the next listing — CONFIRMED

- Verdict: CONFIRMED
- Note: notices.rs:357 applies a pending listing whenever the Qt thread runs it, and notices.rs:476-484 removes the row at once on dismiss. The order is not checked, so an older listing can add the entry and reset the count (notices.rs:400) until the next refresh. The window in which this happens is short.
- Confidence: medium
- Trigger: The writer publishes a listing that still contains entry E (for example after a thumbnail failure is written). Its pending pass is queued to the Qt thread. The user clicks Dismiss on E before that pass runs. The pass then applies the older listing.
- Consequence: show() inserts E again (diff against the rows already removed by the optimistic dismissal) and resets the unread count to the older listing's value, so E and its badge count reappear for the short time until the writer's next listing. That listing does come, because a successful dismissal triggers a refresh. The window's optimistic state is not re-checked against pending results.
- Sites: `crates/desktop/src/notices.rs:357`; `crates/desktop/src/notices.rs:476`; `crates/desktop/src/notices.rs:400`; `crates/desktop/src/notices.rs:484`; `crates/app/src/inbox.rs:903`

### UI-241 [low/missing-feedback] A dismissal the catalog refuses reappears with no message — CONFIRMED

- Verdict: CONFIRMED
- Note: inbox.rs:922-929 sets force, adds to lost and logs a warning. The window removed the entry optimistically (notices.rs:476-484), and the forced refresh brings it back. Nothing in the UI reports the failure.
- Confidence: high
- Trigger: A dismissal or Clear all reaches the writer and the catalog write fails with a non-busy error (for example an I/O or disk error). Busy writes are retried, so this is the rare path.
- Consequence: The window had removed the entry optimistically. The writer then forces a refresh (out.force) and the entry, or the whole cleared set, reappears, with only a log line. The panel and status bar say nothing about the failed dismissal, so it looks as if the dismissal was undone for no reason.
- Sites: `crates/app/src/inbox.rs:923`; `crates/app/src/inbox.rs:929`; `crates/desktop/src/notices.rs:478`; `crates/desktop/src/notices.rs:484`

### UI-242 [low/unhandled-close] A dismissal is dropped at shutdown if the catalog stays busy past the 5 s drain — CONFIRMED

- Verdict: CONFIRMED
- Note: inbox.rs:932-943 handles Abandoned by logging the count of notices and changes that were not written and then breaking. Nothing is shown to the user, and the dismissal is not replayed.
- Confidence: high
- Trigger: The user dismisses an entry and then quits while another connection holds the catalog's write lock for longer than the writer's shutdown drain (5 s), for example during a scan.
- Consequence: Inbox::close abandons the queued Dismiss, logging only that the user's changes weren't written. The entry is listed again on the next launch, and the user is told nothing. The window closes without a warning.
- Sites: `crates/app/src/inbox.rs:889`; `crates/app/src/inbox.rs:932`; `crates/app/src/inbox.rs:939`; `crates/app/src/inbox.rs:547`

## U62: Retry an inbox entry

Entry: `crates/desktop/qml/InboxPanel.qml:473 onClicked retrySelected (InboxPanel.qml:125) -> retryRequested (InboxPanel.qml:53); key R at InboxPanel.qml:236`

### UI-243 [medium/focus] Hidden Retry button keeps focus, so list keys stop working — CONFIRMED

- Verdict: CONFIRMED
- Note: qquickitem.cpp:6641-6645 says a hidden item keeps keyboard focus, and setVisible (qquickitem.cpp:6669-6681) does not clear it. So the hidden Retry keeps focus and the list does not get the keys. Dismiss and Copy use enabled, which does clear focus (qquickitem.cpp:6835-6838).
- Confidence: medium
- Trigger: The user clicks the Retry button with the mouse. The click gives the button keyboard focus (AbstractButton is StrongFocus and forceActiveFocus on press, qquickabstractbutton.cpp:1197). The retry succeeds, the entry's row is removed, unselect() sets selectedCanRetry false (InboxPanel.qml:92, 164), and the button becomes invisible (line 471).
- Consequence: Qt keeps activeFocus on an item made invisible (setVisible does not clear it, qquickitem.cpp:6669-6681; the docs at 6641-6643 say a hidden item retains focus). Key presses then go to the hidden button and its ancestors, not the list, so Up, Down, Home, End, Enter, Delete, Ctrl+C and R stop working until the user clicks or tabs into the list. The panel stays open and the mouse still works. Dismiss and Copy use enabled, which clears focus (qquickitem.cpp:6835-6839), so they do not have this problem.
- Sites: `D:\GitHub\photo-gallery\crates\desktop\qml\InboxPanel.qml:471`; `D:\GitHub\photo-gallery\crates\desktop\qml\InboxPanel.qml:490`; `D:\GitHub\photo-gallery\crates\desktop\qml\InboxPanel.qml:92`; `C:\Qt\6.12.0\Src\qtdeclarative\src\quick\items\qquickitem.cpp:6678`

### UI-244 [low/missing-feedback] A failed Retry shows nothing to the user — CONFIRMED

- Verdict: CONFIRMED
- Note: inbox.rs:922-929 forces a refresh and logs a warning, with no message to the user. The test at inbox.rs:1562-1564 ("a_failed_retry_wakes_nothing") shows the same path.
- Confidence: high
- Trigger: The user clicks Retry or presses R, and the thumbnails' ledger write fails (for example the catalog is locked past the busy retries, or any non-busy error).
- Consequence: The writer logs one warning, rereads the inbox and republishes it, so the entry and its Retry button stay as they were. There is no toast or message, so the click looks like it did nothing, and the user can't tell a failed retry from one still in progress.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\inbox.rs:922`; `D:\GitHub\photo-gallery\crates\app\src\inbox.rs:1563`; `D:\GitHub\photo-gallery\crates\desktop\src\notices.rs:549`

### UI-245 [low/unhandled-close] A Retry queued at close is dropped without a user-visible trace — CONFIRMED

- Verdict: CONFIRMED
- Note: inbox.rs:932-943 drops the rest of the queue, including Step Retry with its FnOnce, when the writer is Abandoned. The job is not woken, and only a warning is logged. The Retry variant is at inbox.rs:210.
- Confidence: medium
- Trigger: The user clicks Retry, then the app closes while the catalog is still busy after the 5 s shutdown drain (Inbox::close, inbox.rs:547-555), so the writer abandons the rest of its work.
- Consequence: The Retry's closure, which would have woken the job, is dropped unexecuted. Only a log line records that the user's change wasn't written. The thumbnail rows stay, and the user sees the same entry with Retry on the next launch.
- Sites: `D:\GitHub\photo-gallery\crates\app\src\inbox.rs:932`; `D:\GitHub\photo-gallery\crates\app\src\inbox.rs:210`

### UI-246 [low/stale-result] Thumbnail failures buffered before a Retry can be recorded again after it — PARTIAL

- Verdict: PARTIAL
- Note: thumbs lib.rs:329-330 writes the buffered failures on its own connection, and ledger.rs:209 deletes the rows of the pass. The finding says the ordering against Retry was not traced, and that remains open.
- Confidence: low
- Trigger: The user presses Retry while the thumbnails pass is running and has buffered failures that have not yet been flushed (up to 2 s, LEDGER_EVERY). The pass writes through its own connection, not the writer's queue, so its flush can land after the writer's Retry delete.
- Consequence: Those files' failures come back after the Retry as new, unread entries, with a toast. The user sees the Retry not clear them. This was not verified end to end: the pass's buffer contents and the ordering against the Retry were not traced in full.
- Sites: `D:\GitHub\photo-gallery\crates\thumbs\src\lib.rs:329`; `D:\GitHub\photo-gallery\crates\catalog\src\ledger.rs:209`

## U63: Inbox failed-file list

Entry: `crates/desktop/qml/InboxPanel.qml:75 requestFiles, from the ledger rows (InboxPanel.qml:64 show); Main.qml:838 onFilesRequested -> failedFilesModel.show(pass)`

### UI-247 [medium/stale-result] Open list not re-read when a non-newest failed file is cleared — CONFIRMED

- Verdict: CONFIRMED
- Note: InboxPanel.qml:78-82 returns early when the key and serial match, and onSerialChanged alone re-tells (InboxPanel.qml:276). clear_failures (ledger.rs:188-) removes a non-newest file without changing the newest serial, so the list is not re-read. The body of tell() was not read, so whether it calls requestFiles() is not settled, but requestFiles() itself would return early.
- Confidence: medium
- Trigger: A pass's ledger entry (for example the thumbnails) is open with its details showing. A file in the list that is not the newest gets its thumbnail made (Inbox::thumb_made, then clear_failures removes its row) with no new failure recorded, so the entry's newest serial does not change.
- Consequence: The list keeps showing the cleared file as failed, and the header line 'The newest N of M files' keeps the old total, until a new failure moves the newest serial or the user reselects the entry. The count text in the details updates through body, but the list does not, so the user sees a stale failed-files list.
- Sites: `crates/desktop/qml/InboxPanel.qml:78`; `crates/desktop/qml/InboxPanel.qml:276`; `crates/catalog/src/inbox.rs:231`; `crates/catalog/src/ledger.rs:188`; `crates/desktop/src/failed_files.rs:129`

### UI-248 [low/stale-result] A failed re-read shows the error above the previous rows and total — CONFIRMED

- Verdict: CONFIRMED
- Note: failed_files.rs:180-185 put_error sets only the error, so the rows of the earlier read stay, and InboxPanel.qml:382 shows the error above them.
- Confidence: high
- Trigger: An open ledger's newest serial moves, so the list is re-read for the same kind, and that read fails (for example the database is locked). The rows of the previous read are kept for the same kind.
- Consequence: The panel shows 'The list couldn't be read: ...' and directly beneath it the old file rows and the old 'The newest N of M files' line, with no sign that those rows are not current.
- Sites: `crates/desktop/src/failed_files.rs:164`; `crates/desktop/src/failed_files.rs:167`; `crates/desktop/qml/InboxPanel.qml:382`; `crates/desktop/qml/InboxPanel.qml:459`

### UI-249 [low/focus] Reveal of a failed file has no keyboard path — PARTIAL

- Verdict: PARTIAL
- Note: The key-binding half holds: no key in the list's Keys handler reveals a file. The "no keyboard path" half does not: qquickabstractbutton.cpp:105 sets activeFocusOnTab on every AbstractButton, so Tab can reach the Reveal button (InboxPanel.qml:444).
- Confidence: medium
- Trigger: A keyboard-only user opens a ledger entry's details and wants to reveal one of its failed files or failed folders.
- Consequence: Reveal is reachable only with the mouse. The file list is not in the Tab order, its Reveal buttons do not set activeFocusOnTab, and no key binding in the list's Keys handler reveals a row, so the keyboard path does not reach the same result as the mouse path.
- Sites: `crates/desktop/qml/InboxPanel.qml:396`; `crates/desktop/qml/InboxPanel.qml:444`; `crates/desktop/qml/InboxPanel.qml:213`

## U64: Reveal a file from the inbox

Entry: `crates/desktop/qml/InboxPanel.qml:448-453 Reveal onClicked (file, not folder) -> revealRequested (InboxPanel.qml:59) -> Main.qml:839 win.revealFile (Main.qml:351)`

### UI-250 [medium/missing-feedback] Reveal read error shows nothing to the user — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-227)
- Note: Duplicate of UI-227: shell.rs:354 logs the read error and posts nothing for the user.
- Confidence: high
- Trigger: The user clicks Reveal on a file in the inbox and the reader's read of present_file_folder fails (the catalog cannot be opened, a SQLite error, a panic). Reveal fails with an error rather than a missing file.
- Consequence: The inbox stays open, the grid does not change, and no toast or notice is posted. The only trace is a log line. The Reveal click looks like it did nothing. Other reads in this window report failures as notices through app::observe.
- Sites: `crates/desktop/src/shell.rs:354`

### UI-251 [medium/missing-feedback] File-gone toast is dimmed behind the open inbox — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.rs:353 posts FileNoLongerInLibrary as an info toast, and StatusBar sits under the inbox, which fills the window (Main.qml:819-822 anchors.fill: parent) with the ModalOverlay scrim (ModalOverlay.qml:26, #b0000000). The toast is dimmed as the finding says.
- Confidence: medium
- Trigger: The user clicks Reveal on a file the library no longer has (present_file_folder returns None). Reveal does not close the inbox, so the inbox stays up.
- Consequence: The notice is a transient info toast in the status bar. The status bar sits under the inbox's dim scrim (#b0000000), and the toast clears itself after 6 s. The panel does not change and no message is shown inside it, so the user sees a dimmed line at the bottom or nothing.
- Sites: `crates/desktop/src/shell.rs:353`; `crates/desktop/qml/ModalOverlay.qml:26`; `crates/desktop/qml/Main.qml:820`

### UI-252 [low/wrong-message] A failed listing read is reported as a missing file — PARTIAL

- Verdict: PARTIAL
- Note: model.rs:100-104 turns a listing error into ListingFailed and an empty list (unwrap_or_default), and applies it with the generation. The reveal branch that then posts FileNoLongerInLibrary (shell.rs:447-451 and the code after it) was not read, so the second notice is not verified.
- Confidence: high
- Trigger: The user clicks Reveal, the folder is found, but the grid's listing read for it fails (a catalog read error). open_listing's observe posts ListingFailed and the list is empty.
- Consequence: The empty list is applied and reports count 0 with the reveal's generation. The reveal then treats the file as absent and posts FileNoLongerInLibrary. The user gets two notices, one claiming the file is gone, though it is still in the library. The real cause is the listing failure.
- Sites: `crates/desktop/src/gallery/model.rs:100`; `crates/desktop/src/gallery/model.rs:104`; `crates/desktop/src/shell.rs:451`

### UI-253 [low/missing-feedback] Reveal shows no busy state while its folder is read — CONFIRMED

- Verdict: CONFIRMED
- Note: InboxPanel.qml:448-453 has no busy state, and the button stays enabled while the folder is read. Low severity. The notices.rs remark about the reader being busy was not read.
- Confidence: medium
- Trigger: The user clicks Reveal on a file while the reader is busy (for example behind a running grid listing, which notices.rs says is accepted). The read can take a noticeable time.
- Consequence: The Reveal button stays enabled with no label change or spinner, and the inbox stays open until the folder is listed. A second click resubmits the request, which supersedes the first safely, but the user has no sign the first click registered.
- Sites: `crates/desktop/qml/InboxPanel.qml:448`; `crates/desktop/qml/InboxPanel.qml:452`

### UI-254 [low/focus] File Reveal has no keyboard binding in the inbox — PARTIAL

- Verdict: PARTIAL
- Note: The list's Keys handler (InboxPanel.qml:213) has no reveal key, which holds. The claim that Tab may not reach the button is answered by qquickabstractbutton.cpp:105 (setActiveFocusOnTab(true) on every AbstractButton), so the button can be reached by Tab.
- Confidence: low
- Trigger: A keyboard-only user opens the inbox, selects a ledger entry, opens its details, and wants to reveal a failed file.
- Consequence: The panel's key map (Up/Down/Enter/Space/Delete/Ctrl+C/R) does not reveal a file. The only route is the row's Reveal Button, which is inside a list that disables activeFocusOnTab. Whether Tab reaches the button depends on Qt's default, which was not verified.
- Sites: `crates/desktop/qml/InboxPanel.qml:213`; `crates/desktop/qml/InboxPanel.qml:396`

## U65: Reveal a folder from the inbox

Entry: `crates/desktop/qml/InboxPanel.qml:448-453 Reveal onClicked (folder) -> folderRevealRequested (InboxPanel.qml:60) -> Main.qml:840 shell.revealFolder(row)`

### UI-255 [medium/stale-result] A pending file Reveal overrides a folder Reveal chosen after it — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.rs:346-351 dispatches Event::Reveal on delivery without checking whether a later reveal or folder change has happened. reveal_folder (shell.rs:363-365) changes the listing directly. reader.rs:492 supersedes only a Reveal with a newer Reveal, not a folder change.
- Confidence: medium
- Trigger: The user clicks Reveal on a file row (async reader read in flight), then, before that read is delivered, clicks Reveal on a folder row (or closes the inbox and clicks a tree folder). The folder reveal applies at once, then the file reveal's delivery arrives and lists the file's folder instead.
- Consequence: The grid and tree end up on the folder of the earlier file request, not the folder the user chose last. The user sees the wrong folder as the result of their latest action, and nothing says so.
- Sites: `crates/desktop/src/shell.rs:350`; `crates/desktop/src/shell.rs:346`; `crates/client/src/controller.rs:317`; `crates/app/src/reader.rs:492`

### UI-256 [low/stale-rows-under-error] A failed re-read of an expanded ledger shows the error above the previous rows — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-248)
- Note: Duplicate of UI-248 and UI-259. failed_files.rs:164-168 calls put_error only, and put_error (failed_files.rs:180-185) sets the error and leaves the rows. InboxPanel.qml:382 shows the error above them.
- Confidence: medium
- Trigger: A ledger entry is open and its newest failure changes, so it asks for the list again. The same-ledger re-read fails (the reader returns an error).
- Consequence: The inbox shows "couldn't be read" together with the old rows, which still have live Reveal buttons, and the total and more-label come from the earlier read. The user can take it for a current list.
- Sites: `crates/desktop/src/failed_files.rs:164`; `crates/desktop/src/failed_files.rs:133`; `crates/desktop/src/failed_files.rs:167`

### UI-257 [low/missing-feedback] Reveal of a refused library root lands on a folder with nothing in it, with no message — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.rs:363 takes the parent of the failed folder with split_file_name, or the folder itself when there is no parent. The refused-root row is written by roots.rs:241-245. A drive root therefore shows itself and a folder under a drive shows a parent outside the library, with no message.
- Confidence: medium
- Trigger: The failed folder is a library root itself (folder_refused writes the root as the failed path, e.g. a refused FAT or non-NTFS drive). Reveal then takes its parent: for a drive root, which has no parent, it shows the failed folder itself. For a folder under a drive, it shows a parent outside the library.
- Consequence: The inbox closes, the grid is empty, and nothing explains why. It looks like the folder is empty rather than refused.
- Sites: `crates/desktop/src/shell.rs:363`; `crates/catalog/src/roots.rs:241`

### UI-258 [low/wrong-state] Tree highlights All while the grid shows the revealed parent folder — CONFIRMED

- Verdict: CONFIRMED
- Note: folders.rs:246 returns -2 when the folder has no node and -1 when it has one without a row. Main.qml:343 then sets tree.currentIndex = Math.max(row, 0), which is 0, the All row.
- Confidence: medium
- Trigger: Reveal on a failed folder whose parent has no node in the folder tree (a node exists only for folders holding present files and their ancestors). revealListing returns -2.
- Consequence: Main.qml sets the tree's current row to All (row 0) by Math.max(row, 0), so the tree highlights All while the grid and top bar show the parent folder. The highlight and the listing disagree.
- Sites: `crates/desktop/qml/Main.qml:343`; `crates/desktop/src/folders.rs:246`

## U66: Show failed imports

Entry: `crates/desktop/qml/InboxPanel.qml:479 onClicked showFilesRequested (InboxPanel.qml:61) -> Main.qml:841-844 shell.showFailedImports (shell_bridge.rs:279) then win.showAsked`

### UI-259 [low/stale-result] Failed re-read keeps the previous rows under the error — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-248)
- Note: Duplicate of UI-248 for the import ledger: show() clears rows only on a kind change (failed_files.rs:133-137), so the same-kind error keeps the old rows (put_error, failed_files.rs:180-185).
- Confidence: high
- Trigger: The inbox's import ledger is expanded and its list is shown. A newer import failure changes the newest serial, so requestFiles asks for a new read of the same ledger (show(2)), and that read fails (the catalog returns an error). Earlier rows are kept because show() only clears on a kind change.
- Consequence: The list keeps the old rows and the old 'The newest N of M' count, under the red 'The list couldn't be read' label. The user can read the stale rows as the current failures, and Reveal acts on them.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/failed_files.rs:164`; `D:/GitHub/photo-gallery/crates/desktop/src/failed_files.rs:167`; `D:/GitHub/photo-gallery/crates/desktop/qml/InboxPanel.qml:395`; `D:/GitHub/photo-gallery/crates/desktop/qml/InboxPanel.qml:382`

### UI-260 [low/stale-result] A pending Reveal overrides a later Show failed files — CONFIRMED

- Verdict: CONFIRMED
- Note: Same mechanism as UI-255: shell.rs:346-351 applies a delivered Reveal unconditionally, and show_failed_imports (shell.rs:370) changes the listing directly.
- Confidence: medium
- Trigger: The user clicks Reveal on an inbox row. The inbox stays open until the folder read is delivered (revealShown closes it). Before that delivery, the user clicks Show the failed files. The failed-import listing is applied first and the inbox closes. Then the older RevealFolder read is delivered.
- Consequence: revealed() dispatches Event::Reveal unconditionally, so the listing becomes the revealed file's folder with the query cleared, and revealShown runs showAsked. The user ends up in that folder, not the failed-import list they last asked for. The window is short (the reader is single-slot and may queue the reveal behind other reads), so this is latent.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/shell.rs:350`; `D:/GitHub/photo-gallery/crates/desktop/src/shell.rs:351`; `D:/GitHub/photo-gallery/crates/desktop/src/shell.rs:370`

## U67: Inbox read state and unread count

Entry: `crates/desktop/qml/Main.qml:429-433 onActiveChanged -> inboxModel.poke (notices_bridge.rs:135) and setViewing(inboxOpen, active); Main.qml:70 setViewing on open`

### UI-261 [medium/timer-not-restarted] Info toast replaced mid-countdown expires early — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-218)
- Note: Duplicate of UI-218: StatusBar.qml:110 keeps the running timer through a text change, so the second info toast gets the first one's remaining time.
- Confidence: high
- Trigger: An info-level toast (level 0) is on screen with its 6 s countdown running. A second info toast arrives inside that window: a transient notice, or a kept Info notice that is news. on_pass sets the new text through set_toast_shown, and the Timer's running binding stays true, so the Timer is not restarted and the second toast is cleared when the first countdown ends.
- Consequence: The second toast can vanish after a fraction of a second instead of 6 s. A transient notice (feedback on what the user just did) is kept only as a toast, not in the inbox, so the user can miss it entirely. Kept notices remain in the inbox and the bell count.
- Sites: `crates/desktop/qml/StatusBar.qml:110`; `crates/desktop/qml/StatusBar.qml:111`; `crates/desktop/src/notices.rs:429`

### UI-262 [low/missed-retry] A failed listing read is never retried until the next write — CONFIRMED

- Verdict: CONFIRMED
- Note: inbox.rs:902-905 records self.version before refresh, and refresh returns on a failed read (inbox.rs:1023-1026) without publishing. The next poll sees the same version and skips the refresh until another write or commit.
- Confidence: high
- Trigger: A poke (window activation) or the 30 s POLL finds data_version moved, so refresh runs. The read of marks or entries fails (for example a transient SQLITE_BUSY or any read error). refresh logs and returns without publishing, but handle has already recorded the new version. Later polls and pokes see the same version and skip the refresh, so nothing is re-read until some write in this process or another commit happens. Latent: the catalog is in WAL mode, so reads rarely block.
- Consequence: The status bar's unread count and the inbox rows stay stale, possibly showing zero unread while entries are unread, with no message to the user, until the next write or external commit.
- Sites: `crates/app/src/inbox.rs:902`; `crates/app/src/inbox.rs:903`; `crates/app/src/inbox.rs:1025`; `crates/app/src/inbox.rs:862`

## U68: Close the inbox

Entry: `crates/desktop/qml/InboxPanel.qml:509 Close onClicked -> dismissed; ModalOverlay.qml:22 Keys.onEscapePressed -> dismissed; ModalOverlay.qml:28-31 background onClicked -> dismissed`

### UI-263 [medium/missing-feedback] First open of the session may not be reported as open to the controller — PARTIAL

- Verdict: PARTIAL
- Note: Main.qml:826 and ModalOverlay.qml:18-21 give the controller the visible change (Main.qml:848). Whether the first visible change is reported on first open was not checked at runtime, so this stays low confidence, as the finding says.
- Confidence: low
- Trigger: Launch the app, open the inbox for the first time in the session (bell, Ctrl+Shift+N, or the status bar's N failed link), then press Ctrl+Shift+N to close it. Escape and Close are not affected.
- Consequence: If the open is not reported, the controller's modal mirror (panels.rs) does not list Inbox, so switch_to (controller.rs:447-460) answers Ctrl+Shift+N with OpenPanel(Inbox) instead of ClosePanel. The inbox stays open, the key appears to do nothing, and a second press does nothing too. Ctrl+Shift+L can also open the log view over the inbox, because the sibling is not closed. After one close and reopen the mirror is correct. Confidence is low: the Qt visible-binding ordering that would cause this was not verified by running the app.
- Sites: `crates/desktop/qml/Main.qml:826`; `crates/desktop/qml/ModalOverlay.qml:18`; `crates/desktop/qml/Main.qml:848`; `crates/client/src/controller/panels.rs:127`

## U69: Log view (dev mode)

Entry: `crates/desktop/qml/StatusBar.qml:124 onClicked logRequested -> Main.qml:561 shell.globalAction(Log); shortcut GlobalShortcuts.qml:30-34; Main.qml:444-447 openPanel -> logModel.start (log_model_bridge.rs:73)`

### UI-264 [medium/stale-state] Following flag stays true during a wheel ease or scroll-bar drag, so new records snap the view back to the end — CONFIRMED

- Verdict: CONFIRMED
- Note: LogPanel.qml:112 positions the view at the end while following is true. WheelListView.qml:89-91 stops the ease when contentY differs from the target, and wheelScrolled (WheelListView.qml:97-99) is emitted only when the ease completes. So following stays true while records arrive.
- Confidence: high
- Trigger: Dev-mode log view open and following (the newest line at the bottom). The user wheels up one or more notches, or drags the scroll bar, and a record is logged before the ease completes (about 0.3 s per notch) or before the drag is released.
- Consequence: Each record calls positionViewAtEnd (LogPanel.qml:112) because following is still true. WheelListView's frame animation then stops at once, since contentY no longer matches `written` (WheelListView.qml:89-91), and wheelScrolled is never emitted, so following stays true. While the log is busy, scrolling up with the wheel or a scroll-bar drag is undone at each record, and the user cannot read older lines. The header of LogPanel.qml (:6-8) says the flag is set at the user's scroll, not from a stale atYEnd, but for the wheel and scroll bar it is set only after the gesture ends.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:112`; `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:114`; `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:118`; `D:/GitHub/photo-gallery/crates/desktop/qml/WheelListView.qml:89`; `D:/GitHub/photo-gallery/crates/desktop/qml/WheelListView.qml:97`

### UI-265 [low/missing-feedback] Keyboard can move the list but cannot select a row, and the current row is not shown — CONFIRMED

- Verdict: CONFIRMED
- Note: LogPanel.qml:45-48 forwards only the movement keys, and Enter and Space are not handled. The row is selected by TapHandler only (LogPanel.qml:162-165). The highlight (LogPanel.qml:132) follows selectedSeq, not the list's current row.
- Confidence: high
- Trigger: Log view open with focus on the list (its default). The user presses Up, Down, PageUp, Home or End, then Enter or Space, expecting to select a row and see its full message and detail.
- Consequence: The keys move the view, but there is no current-row highlight, and nothing handles Return or Space, so the row's message and detail (the TextArea at :175-186) cannot be reached by keyboard. Only a mouse tap selects. The keyboard path does not reach the same result as the mouse path. The summary's `Keys move and select lines` is not what the code does.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:116`; `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:132`; `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:163`

### UI-266 [low/wrong-message] Empty-state text 'Nothing logged yet' shows while the log files are still being read — CONFIRMED

- Verdict: CONFIRMED
- Note: LogPanel.qml:167-172 shows the empty label when list.count is 0. The LogModel's ready property (log_model_bridge.rs:25, set at log_model.rs:165) is not referenced in LogPanel.qml, which a grep confirms.
- Confidence: high
- Trigger: The user opens the log view for the first time (or after the window starts) while the log files (up to three rotated files of 5 MiB each) are still being read and parsed on the reader thread, before show_all runs.
- Consequence: For the read's duration the panel says 'Nothing logged yet' over an empty list, which is wrong: the records are on their way. The ready property that would tell it apart is never read by the panel.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:169`; `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:170`; `D:/GitHub/photo-gallery/crates/desktop/src/log_model_bridge.rs:25`; `D:/GitHub/photo-gallery/crates/desktop/src/log_model.rs:165`

### UI-267 [low/stale-state] Reopening the log view after scrolling away keeps the old position and no longer follows the newest line — CONFIRMED

- Verdict: CONFIRMED
- Note: following is set only in scrolled() and keyMoved() (LogPanel.qml:33-42), and LogPanel has no open handler that resets it. The loader is kept (Main.qml:809-813), so the view reopens at the old position with following still false.
- Confidence: medium
- Trigger: The user scrolls up (following becomes false), closes the log view, and while it is closed more records are logged. The user reopens it with Ctrl+Shift+L or the status bar button.
- Consequence: The panel is kept (the Loader is not reset), so it reopens at the old scroll position with following still false. The newest records are out of view and nothing indicates that the view is not at the end. The user must scroll down to see the newest lines.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:30`; `D:/GitHub/photo-gallery/crates/desktop/qml/LogPanel.qml:112`; `D:/GitHub/photo-gallery/crates/desktop/qml/Main.qml:809`

### UI-268 [low/missing-feedback] A failed reader thread leaves the view empty with no message and no retry — CONFIRMED

- Verdict: CONFIRMED
- Note: log_model.rs:111-115 sets started before the spawn, and log_model.rs:156-158 logs the failure and returns. start() then returns at once on later opens.
- Confidence: medium
- Trigger: std::thread::Builder::spawn fails when the log view is first opened (resource exhaustion). The started flag was already set.
- Consequence: The warning goes to the log, which this view does not show. The view stays empty under 'Nothing logged yet', and later opens do not retry, because start() returns at once once started is set. The user sees the empty message with the records still in the ring.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/log_model.rs:115`; `D:/GitHub/photo-gallery/crates/desktop/src/log_model.rs:156`

### UI-269 [low/wrong-message] Translator comment for the log strings is attached to the inbox key — CONFIRMED

- Verdict: CONFIRMED
- Note: ui.ftl:91 holds the log comment, and the next key is ui-inbox (ui.ftl:93). ui-log, ui-log-title and ui-log-empty (ui.ftl:116-118) have no comment of their own.
- Confidence: high
- Trigger: Anyone reading or translating crates/client/locales/en/ui.ftl.
- Consequence: The comment that describes ui-log, ui-log-title and ui-log-empty sits above ui-inbox, and ui-log has no comment of its own, so translators see the log strings described as the inbox's. No runtime text is wrong.
- Sites: `D:/GitHub/photo-gallery/crates/client/locales/en/ui.ftl:91`; `D:/GitHub/photo-gallery/crates/client/locales/en/ui.ftl:116`

## U70: Dismiss any modal panel (Escape or background click)

Entry: `crates/desktop/qml/ModalOverlay.qml:22 Keys.onEscapePressed and :28-31 MouseArea onClicked -> dismissed (ModalOverlay.qml:14); ModalOverlay.qml:21 onVisibleChanged sets focus or emits hidden (:16)`

### UI-270 [medium/focus] Library panel loses keyboard focus to the grid after Remove or Discard dialog closes — CONFIRMED

- Verdict: CONFIRMED
- Note: Main.qml:223 and 237 call grid.takeFocus() on close, but nothing clears libraryOpen. shell.rs:151-155 guards only the viewer, so the grid receives the keys while the library panel is still shown.
- Confidence: high
- Trigger: Open the library panel (TopBar Library button), click Remove (or Discard) on a folder, then press Escape or Cancel in the confirmation dialog. The dialog closes, but the library panel stays open (libraryOpen is never cleared). The user presses Escape again, or an arrow, Enter, Space, R, F, B or a digit key.
- Consequence: Escape does nothing, so the keyboard path to dismiss the library panel is gone and the user must click Close or the backdrop. While the panel is still visibly open, grid keys act on the photos behind it: arrows move the grid selection, Enter or Space opens the selected photo in the viewer under the panel, R turns the photo, digits 0-5 change its rating, F toggles the filter bar, and B and N change border state. None of this is visible through the dimmed panel, so the user can change a rating or a rotation without seeing it. Shell.key_pressed guards only the viewer, so nothing stops these keys.
- Sites: `crates/desktop/qml/Main.qml:223`; `crates/desktop/qml/Main.qml:237`; `crates/desktop/qml/GalleryGrid.qml:56`; `crates/desktop/src/shell.rs:153`

## U71: Preferences (settings) read and saved

Entry: `crates/desktop/qml/Main.qml:39 Settings {id: appSettings}; read at Main.qml:53, 690-693; written by LibraryPanel toggles (U31) and Viewer.qml:279 (settings.realSize) in the RealSize key`

### UI-271 [low/missing-feedback] Z toggles saved Real Size on videos and animations with no visible effect — PARTIAL

- Verdict: PARTIAL
- Note: keys.rs:133 maps Z to RealSize for any kind, Viewer.qml:279 writes the setting, Hud.qml:92 shows the label, and the hint lists it only for stills (keys.rs:304). The claim that the picture does not change depends on the viewport reset (client viewport.rs:170-176), which was not read.
- Confidence: high
- Trigger: Open a video or an animation (or a still whose fit is already 1:1) in the viewer and press Z. The key is accepted for every media kind.
- Consequence: keys.rs:133 maps Z to RealSize for any media kind, and Viewer.qml:279 writes settings.realSize. The viewport reset (client viewport.rs:170-176) keeps a non-zoomable view fitted, so the picture does not change. The HUD (Hud.qml:92) then shows '[real size]' for the video even though Real Size does nothing for it. The hint line (keys.rs:303-304) does not list Z for video. The persistent setting still changes silently, and the next still opened will open at 1:1.
- Sites: `crates/desktop/qml/Hud.qml:92`; `crates/client/src/keys.rs:133`; `crates/desktop/qml/Viewer.qml:279`

## U72: Controller deadlines and pauses

Entry: `crates/desktop/src/shell_bridge.rs:50 timeout (signal from the controller's deadlines), Main.qml:139-151 (query, layout and thumbnail pauses)`

### UI-272 [medium/stale-state] Restored selection at start is set after the grid is placed, so it is not scrolled into view — CONFIRMED

- Verdict: CONFIRMED
- Note: shell.rs:456-460 sets the current item in a queued gallery::post, while Main.qml:264-266 reads current right after shell.listShown. The restore therefore runs after the grid is placed. The Reveal path queues its select after the set (shell.rs:447-449).
- Confidence: high
- Trigger: Start with a remembered last file that is in the first list but below the visible rows (any library larger than one screen). The gate's RestoreSelection (controller start.rs 67-72 via controller.rs 358) runs inside gridLoaded's shell.listShown call.
- Consequence: The window opens at the top of the list (or the list's anchor) while the remembered file is selected off-screen. Its status-bar text shows, but no visible cell is highlighted, so the user sees the wrong place as the one they left. The Reveal path avoids this by queuing select_requested after its set (shell.rs 447-449).
- Sites: `crates/desktop/src/shell.rs:456`; `crates/desktop/qml/Main.qml:264`; `crates/desktop/src/shell.rs:431`

### UI-273 [low/lost-save] A selection made before the start is over is dropped from the session save — CONFIRMED

- Verdict: CONFIRMED
- Note: controller.rs:536 drops a SaveSession timer that fires before the start is over, and the start does not re-arm it (controller.rs:360-364 saves only when apply_view is set). A selection made in that window is not saved.
- Confidence: high
- Trigger: The user selects a file while the start is still pending (tree or first list not yet in) for longer than the 1 s SaveSession pause, and makes no further selection change before quitting.
- Consequence: The selection is never saved (the pause fires before started, is discarded, and no later event re-arms it, and flush saves only pending timers). The next start restores an older file than the one the user last chose.
- Sites: `crates/client/src/controller.rs:536`; `crates/client/src/controller.rs:364`; `crates/client/src/controller.rs:292`

### UI-274 [low/stale-state] Start-only --query and --sort are saved into the session — CONFIRMED

- Verdict: CONFIRMED
- Note: controller.rs:313-316 saves the session on FolderSpelled with no started check, so case (b) holds. Case (a) holds because the timer saves once started (controller.rs:532-534). The test a_start_with_its_options_saves_nothing (shell.rs:650-706) was not read.
- Confidence: high
- Trigger: A start with --query or --sort and either (a) a last file restored and found in the first list, which arms a save 1 s after the start; or (b) a folder the tree spells differently or a folder that is gone, which saves during the start.
- Consequence: The query or sort given for one run becomes the remembered session, so the next normal start shows it. This breaks the Start contract (controller.rs 34-36: the controller saves only what the user changes afterwards). The test a_start_with_its_options_saves_nothing (shell.rs 650-706) never sends CurrentChanged, so it misses case (a).
- Sites: `crates/client/src/controller.rs:532`; `crates/client/src/controller.rs:313`; `crates/client/src/controller.rs:495`

## U73: Command-line find similar

Entry: `crates/desktop/qml/TestHooks.qml similarRequested -> Main.qml:484 win.findSimilarFromCommandLine (Main.qml:382)`

### UI-275 [high/stale-result] Refused find-similar shows the running search's panel with no message — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-182)
- Note: Duplicate of UI-182 and UI-188 for the command-line entry: yandex.rs:141 returns on Ok(false) while busy (app/yandex.rs:425-426), and the panel reopens on A's state (YandexPanel.qml:36-40).
- Confidence: high
- Trigger: The command-line --find-similar=A.jpg search is still running (uploading, other sizes or similar pages can take tens of seconds). The user presses Escape to close the panel, then uses the viewer's Find similar on photo B. Search::start returns Ok(false) because busy, and search_photo returns silently.
- Consequence: The panel reopens showing photo A's progress (Uploading... or Similar) or, once A has finished, A's results, as the answer to the B request. No message says B's search was refused or never ran. The user can take A's results for B's.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:141`; `D:/GitHub/photo-gallery/crates/app/src/yandex.rs:425`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:32`

### UI-276 [low/stuck-busy] Thread failure in ensure_pump leaves the panel busy with no poll — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-192)
- Note: Duplicate of UI-192: yandex.rs:176-179 leaves pump false, and search_photo sets busy (yandex.rs:145) with no poll to clear it.
- Confidence: high
- Trigger: The change pump's thread cannot be spawned (system thread limit). ensure_pump logs and sets pump=false, but search_photo still starts the search and sets busy=true.
- Consequence: No poll ever runs, so the panel shows its spinner and Starting... forever, with the Save button disabled. The results are never shown. The ensure_pump comment says the next search retries, but this search gets no feedback.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:138`; `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:145`; `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:176`

### UI-277 [low/missing-check] --yandex-preview opens an out-of-range item when there is no result — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-215)
- Note: Duplicate of UI-215 for the hook: YandexPanel.qml:59-61 opens the preview with no check on result count, and YandexPreview.qml:51-55 sets the item as given.
- Confidence: high
- Trigger: Test hook: --find-similar=`<not a still image>` --yandex-preview=0 (or N past the result count). The search ends with no results, busy goes false, and onBusyChanged opens the preview with no check.
- Consequence: The preview opens on a blank picture. Its label names an image that does not exist (for example "Similar image 1 of 0"). This is test-hook only, and not user-facing.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:59`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPanel.qml:60`; `D:/GitHub/photo-gallery/crates/desktop/qml/YandexPreview.qml:51`

### UI-278 [low/stale-result] A failed viewer find-similar keeps the previous photo's results — CONFIRMED

- Verdict: CONFIRMED (duplicate of UI-183)
- Note: Duplicate of UI-183 and UI-212: yandex.rs:123-124 sets only the status, and the previous results stay in the panel.
- Confidence: medium
- Trigger: Not the command-line entry: the viewer's Find similar on a file the catalog no longer knows (Error::UnknownFile from catalogs.rs file_path) after an earlier search has results. search_file's error branch only sets the status.
- Consequence: The panel shows the earlier photo's results, with a "Can't read the photo" line for the new one. The results look like the new photo's.
- Sites: `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:121`; `D:/GitHub/photo-gallery/crates/desktop/src/yandex.rs:124`

## U74: Grid thumbnails load and cells display

Entry: `crates/desktop/qml/GridRow.qml:103 Component.onCompleted take and :106 onCountChanged take; GridRow.qml:78 take()`

### UI-279 [medium/stuck-error] A transient thumbnail failure leaves the error mark on a readable photo — PARTIAL

- Verdict: PARTIAL
- Note: GridCell.qml:68 binds the source to the thumbnail URL and :89 shows the error mark when thumb.status is Error. providers.rs:122-132 reports any error as failed(). Whether Qt's Image re-requests an unchanged source after an Error was not read in qquickimage.cpp, and that decides the finding.
- Confidence: medium
- Trigger: A photo's original cannot be read when its cell first loads (an offline or removable drive, or a file held by another process). The provider returns failed(), so the cell's thumb Image goes to Error and shows the red mark. The drive or file becomes readable again, but the row's thumbnail URL (the image key and the thumbnail height) does not change, so the Image never requests again. The mark stays until the cells are recreated, by a thumbnail-size step, a relayout, a list reset, or the row being given different files.
- Consequence: A photo that can now be read shows the failure mark, as if it could not be decoded, until the user changes the size or reloads the list.
- Sites: `crates/desktop/qml/GridCell.qml:68`; `crates/desktop/qml/GridCell.qml:89`; `crates/desktop/src/providers.rs:122`; `crates/desktop/src/providers.rs:132`; `crates/desktop/qml/GridRow.qml:84`

### UI-280 [medium/wrong-message] A failed grid listing shows the empty-folder message and clears the photos — CONFIRMED

- Verdict: CONFIRMED
- Note: model.rs:100-106 applies an empty list when the read fails (unwrap_or_default), and GalleryGrid.qml:96-99 shows "No images here" (or the filter or import text) whenever the count is 0 and not loading.
- Confidence: high
- Trigger: The grid's catalog read fails while a folder is open or being refreshed (the catalog cannot be opened, or the read errors). This happens during a refresh of the same folder after a scan, with the folder's photos already shown.
- Consequence: The grid replaces its rows with an empty list and shows 'No images here.' (or 'Nothing here matches the filters.' while a filter is on). That is a false statement about the folder, and the photos disappear until a later listing succeeds. The only other sign is the separate ListingFailed notice.
- Sites: `crates/desktop/src/gallery/model.rs:104`; `crates/desktop/src/app.rs:30`; `crates/desktop/src/gallery/model.rs:133`; `crates/desktop/qml/GalleryGrid.qml:96`; `crates/desktop/qml/GalleryGrid.qml:99`; `crates/client/locales/en/ui.ftl:53`

### UI-281 [low/visual-flicker] retainWhileLoading is undone by the fade, so a changed thumbnail flashes to the placeholder — CONFIRMED

- Verdict: CONFIRMED
- Note: GridCell.qml:70 sets retainWhileLoading. GridCell.qml:74 sets fade to 0 unless the status is Ready, so a new URL makes the status Loading and the retained image fades out through the Behavior at :75. GridCell.qml:58 then shows the blurred ThumbHash until the new image is Ready.
- Confidence: medium
- Trigger: A cell's thumbnail URL changes while the same file is shown: a thumbnail-height step (Ctrl+wheel calls shell.sizeStepped, which changes the thumb height and so every visible row's URL), a turn, or a rewrite (which changes the image key).
- Consequence: The cell's Image is set to Loading, so fade drops to 0 and the retained previous thumbnail fades out. The blurred ThumbHash shows until the new image is Ready, instead of the old thumbnail staying visible. Each size step flashes the whole visible grid.
- Sites: `crates/desktop/qml/GridCell.qml:70`; `crates/desktop/qml/GridCell.qml:74`; `crates/desktop/qml/GridCell.qml:81`; `crates/desktop/qml/GridCell.qml:58`
