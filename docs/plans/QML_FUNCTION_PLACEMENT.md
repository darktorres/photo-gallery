# QML function placement

Every `function` declaration under `crates/desktop/qml/` at commit `ec85b503`: 118 in total. Each is read in its body and given one verdict:

- **R (move to Rust):** the function decides something the app state depends on, such as which list is shown, what a command does, or what the start order is. It moves into the controller (`pg-client`) or a Rust facade, and QML only forwards the call.
- **D (delete or fold):** it only forwards a call or sets a value, so the caller can make the call directly.
- **Q (stays in QML):** it handles presentation: layout, focus, scroll position, animation, dialog routing, or the item it keeps for a view.
- **H (handler):** a signal, model or `Connections` handler. These stay in QML by nature. Some are noted as R-candidates when they keep selection state that Rust could hold instead.

The criterion is a proposal for F-48 (`REFACTOR_LEDGER.md`), not a decision: move what decides app state, keep what only shows or positions it.

Counts: R 26, D 9, Q 64, H 19. The total is 118. (The third pass moved five back to Q: `openPanel`, `closePanel`, `applyView`, `YandexPanel.show` and `showCommandLine` only forward to decisions Rust already makes.)

This is the second pass. The first pass kept some flow-starting functions in QML and treated some selection handlers as permanent. Both are corrected here: where the audit's A-017 policy says the controller decides, the function moves, and QML only renders. Handlers that exist only to keep selection on its row go away once Rust holds the selection by key.

## FilterBar.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 42 | countFacets | forwards to `Filters.countFacets` | Q | kept, not deleted. Tried removing it behind a `property alias` to `Filters`; `qmllint` cannot see members through an untyped alias and warns, so the typed forwarder stays |
| 46 | reloadQuery | forwards to `Filters.reload` | Q | kept, not deleted, for the same reason as `countFacets` |
| 97 | Bound.reload | sets a field's text from the bridge | Q | stays (view binding) |
| 109 | Bound onLoaded | handler | H | stays |
| 158 | refreshChoices | re-reads each facet's rows and the review box | Q | stays (view refresh) |
| 170 | Connections onEdited | handler | H | stays |
| 173 | Connections onLoaded | handler | H | stays |
| 176 | Connections onCounted | handler | H | stays |
| 406 | Bound.reload (second) | sets a field's text from the bridge | Q | stays |
| 413 | onLoaded (second) | handler | H | stays |

## GalleryGrid.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 41 | topItem | the first item of the topmost visible row | Q | stays (view geometry) |
| 53 | waitsForAbove | whether the row above still waits for pictures | Q | stays (view pooling) |
| 58 | positionAt | positions the view on a row | Q | stays |
| 62 | positionAtBeginning | positions the view at the top | Q | stays |
| 66 | takeFocus | gives the grid keyboard focus | Q | stays |

## GridRow.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 81 | recountPending | counts cells still loading a thumbnail | Q | stays (view state) |
| 92 | sameFiles | whether two rows show the same files | Q | stays (cell reuse) |
| 102 | take | reuses or rebuilds the row's cells | Q | stays |

## ImageLayers.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 45 | restartRegion | restarts the region timer | Q | stays |
| 55 | showRegion | shows the region Rust made | Q | stays (presents Rust's output) |
| 61 | dropRegion | drops the old region | Q | stays |

## InboxPanel.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 64 | show | stores the selected entry's details | Q | stays (selection display) |
| 75 | requestFiles | decides which ledger's failed-file list to ask for, once per entry and failure | R | Rust inbox: the controller decides which list is open |
| 86 | unselect | clears the selection | Q | stays |
| 101 | select | selects a row | Q | stays |
| 110 | moveTo | scrolls to a row and selects it | Q | stays |
| 119 | dismissSelected | dismisses the selected entry | R | a Rust command: dismiss the selected entry |
| 127 | retrySelected | retries the selected entry if it can be retried | R | a Rust command: the can-retry check moves with it |
| 135 | expand | opens the entry Rust asked to expand, once per opening and entry | Q | stays (view placement). Flagged: the once-per-opening rule is the only part that decides rather than places |
| 156 | Connections onRowsInserted | keeps the selection on its row as rows arrive | R | Rust holds the selection by entry key; this handler goes |
| 160 | Connections onRowsRemoved | keeps the selection on its row as rows leave | R | as above |
| 169 | Connections onRowsMoved | keeps the selection on its row as rows move | R | as above |
| 181 | Connections onModelReset | clears the selection on a model reset | H | stays |
| 268 | tell | a delegate tells the panel its details | Q | stays |
| 272 | wanted | whether this row is the one just selected | Q | stays |
| 277 | Connections onRowSelected | the delegate answers a selection | H | stays |

## LogPanel.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 33 | scrolled | follows the end when the user scrolls there | Q | stays |
| 40 | keyMoved | follows the end after a key move | Q | stays |
| 45 | keyPressed | notices keys that move the row | Q | stays |
| 56 | select | selects a record | Q | stays |
| 62 | unselect | clears the selection | Q | stays |
| 71 | Connections onRowsInserted | keeps the selection as records arrive | R | Rust holds the selection by record; this handler goes |
| 75 | Connections onRowsRemoved | keeps the selection as records leave | R | as above |
| 83 | Connections onModelReset | clears the selection on a reset | H | stays |

## Main.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 130 | Connections onFrameSwapped | the first frame is shown | H | stays |
| 154 | updateFacets | decides when the filter counts are needed (bar open or filtering) | R | the controller: it asks for counts when they are needed |
| 201 | askSaveCrops | opens the folder picker for the crop copies | R | the controller starts the crop save; QML shows the picker it asks for |
| 206 | askCutBorders | fetches the cut count and opens the dialog | R | the controller decides the cut and its count; QML shows the confirmation |
| 222 | askRemoveFolder | looks up the folder name for a row in `library.roots` | R | Rust provides the folder for the row; QML opens the dialog |
| 237 | askDiscardRemoval | opens the discard dialog | Q | stays |
| 249 | relayout | picks the anchor item, relays out, positions the grid | R | the anchor choice goes to Rust; QML positions |
| 265 | gridLoaded | the start's first step: placement, then `listShown` | R | the start gate (F-46) |
| 289 | treeLoaded | places the tree on the first load, then `treeLoaded` | R | the start gate (F-46) |
| 306 | currentRowOf | the tree row for the listed folder | R | the folder model returns the row |
| 314 | followListedRow | sets the tree row from a reveal | Q | stays (glue over the R above) |
| 319 | reloadAll | reloads the folder tree and the grid after a scan | R | one controller command, `reloadAll` |
| 327 | refreshGrid | calls `shell.reloadList` | D | done: removed. `library.onImported` and `reloadAll` call `shell.reloadList()` |
| 333 | showListing | syncs the facets and the sort box | Q | stays |
| 339 | applyView | applies a view and positions the tree and grid | Q | stays: `shell.apply_view` already decides; QML positions the tree and grid |
| 353 | showAsked | closes the inbox and viewer, shows the asked list | R | the controller decides the panels and the list; QML positions |
| 367 | revealFile | calls `shell.reveal` | D | done: removed. The inbox's reveal calls `shell.reveal(fileId)` |
| 372 | scrollTo | positions the grid on an item | Q | stays |
| 379 | select | selects an item in the window and scrolls to it | R | the controller owns the selection |
| 393 | findSimilar | loads the Yandex panel and starts its search | R | the controller starts the search; QML only creates the panel it needs |
| 398 | findSimilarFromCommandLine | as findSimilar, for the command line | R | as findSimilar |
| 405 | askSaveView | creates the save-view dialog on first ask | Q | stays |
| 452 | modalShown | maps the visibility to `panelOpened` or `panelClosed` | Q | kept, not deleted. 13 callers; deleting it means 13 ternaries. It is the one place that maps a dialog's visibility to the shell's rule |
| 460 | openPanel | opens a panel the shortcuts asked for | Q | stays: the decision is already Rust's (`panel_open_requested`); QML maps the kind to its visibility |
| 468 | closePanel | closes a panel the shortcuts asked for | Q | stays: already Rust's (`panel_close_requested`) |
| 476 | refocus | returns focus to the viewer or the grid | Q | stays |
| 766 | featurePopup.show | shows a notice for a few seconds | Q | stays |
| 774 | Connections onBordersRefused | handler | H | stays |
| 777 | Connections onOptionsRefused | handler | H | stays |
| 801 | Connections onSimilarSearchRequested | handler | H | stays |
| 804 | Connections onSimilarCommandLineRequested | handler | H | stays |
| 906 | startHooks | runs the test hooks, then the library, in order | R | the start gate (F-46) |

## Mascot.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 19 | step | shows the previous or next mascot | Q | stays |

## MediaView.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 46 | togglePlay | plays or pauses the shown video or animation | Q | stays |
| 57 | openVideo | opens the video device when shown | Q | stays (the device is Rust's) |
| 119 | Connections onVideoFrameChanged | the first frame is drawn | H | stays |

## RatingPopup.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 16 | show | shows a rating symbol and fades it | Q | stays |

## Ratings.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 14 | symbol | returns `words.ratingSymbol` | Q | kept, not deleted. `Ratings` is the one place the rating symbols come from; deleting it means a `Words` in each of 4 files |

## TestHooks.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 86 | run | interprets the command line's hook letters and asks for each action | R | the command line's hooks belong to the controller start (`command_line.rs`) |
| 146 | runAfterLibraryStart | the hooks that wait for the library | R | as `run` |

## TopBar.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 54 | openViewsMenu | opens the views menu (test hook) | Q | stays |

## VideoTransport.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 35 | time | formats a playback position | Q | kept, not deleted. It is the one place the playback time is formatted, with the rest of the words; 2 callers in the same file |

## Viewer.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 142 | settleFit | takes the fit size once a resize settles | Q | stays |
| 160 | show | moves to an item and resets the view | Q | stays |
| 167 | close | emits `closed` and clears the item | Q | stays |
| 172 | step | emits `stepRequested` | D | done: removed. Callers emit `stepRequested` directly, and the viewer test does too |
| 177 | stepTo | moves to an item and resets the view if it changed | Q | stays |
| 185 | rate | rates the item and moves to the next one | R | a Rust command: rate and advance together |
| 192 | turn | emits `turned` and resets the view | Q | stays |
| 197 | togglePlay | bumps the play request counter | Q | kept, not deleted. It owns the request counter; 3 callers would each need `playRequests++` |
| 202 | setZoom | zooms the viewport | Q | stays (gesture) |
| 210 | pan | pans the viewport | Q | stays (gesture) |
| 216 | resetView | resets the viewport | Q | stays |
| 223 | toggleOneToOne | toggles fit and 1:1 at a point | Q | stays (gesture) |
| 232 | pushWindow | re-fits the view when the window changes | Q | stays |
| 240 | pushMedia | re-fits the view when the image changes | Q | stays |
| 565 | Connections onRated | handler | H | stays |
| 568 | Connections onViewMoved | handler | H | stays |
| 571 | Connections onViewReset | handler | H | stays |

## ViewsMenu.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 27 | open | opens the menu (test hook) | Q | stays |

## WheelListView.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 28 | clampContentY | clamps a scroll position to the view | Q | stays |
| 38 | wheelBy | scrolls by one wheel event | Q | stays (the event arrives in QML) |
| 74 | Scroller moveTo | sets the scroll position | Q | stays |

## WheelStepper.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 17 | reset | drops a kept part of a notch | Q | stays (the wheel arrives in QML) |
| 22 | stepsFor | turns a wheel movement into steps | Q | stays |

## YandexPanel.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 31 | show | opens the panel and asks for the file's search | Q | stays: `yandex.searchFile` already starts the search; QML shows the panel |
| 37 | showCommandLine | as show, for the command line's photo | Q | stays: `yandex.searchCommandLine` already starts it |
| 42 | openPanel | shows the panel and takes focus | Q | stays |
| 49 | close | hides the panel | Q | stays |
| 58 | Connections onBusyChanged | handler (test hook) | H | stays |
| 306 | Connections onPreviewItemChanged | handler | H | stays |

## YandexPreview.qml

| Line | Function | What it does | Verdict | Where it goes |
|---|---|---|---|---|
| 48 | open | opens the preview on a result | Q | stays |
| 56 | close | closes the preview | Q | stays |
| 62 | step | moves to the next or previous result, wrapping | R | the result list is Rust's: the wrap-around moves there |

## What the R list needs first

- The start gate (F-46) is not built. `gridLoaded`, `treeLoaded`, `startHooks` and the test hooks' `run` all depend on it. Build the gate first, then move them.
- `relayout`'s anchor choice needs a Rust function that takes the grid width and returns the row to keep in view.
- `requestFiles`, `dismissSelected` and `retrySelected` need Rust inbox commands, which the inbox already has for the rows.
- Selection kept by entry key (the R-candidates in the inbox and log handlers) would remove three or four handlers each.

## Rust target for each R function

Status: **exists** means the Rust surface is already there and the QML only has to call it. **small change** means an existing Rust function gains a step. **new** means no Rust surface exists yet.

| Line | Function | Rust target | Status |
|---|---|---|---|
| 75 | InboxPanel.requestFiles | `failed_files.rs` `show(pass)` (`failed_files_bridge.rs`); the once-per-entry check moves into it | small change |
| 119 | InboxPanel.dismissSelected | `Notices::dismiss(row)` (`notices_bridge.rs`); the selected row comes from the keyed selection | exists, plus new selection |
| 127 | InboxPanel.retrySelected | `Notices::retry(row)` (`notices_bridge.rs`); the can-retry check moves to Rust | exists, plus small change |
| 156, 160, 169 | InboxPanel row handlers | keyed selection held in `notices.rs`, with a `selected` invokable | new |
| 71, 75 | LogPanel row handlers | keyed selection held in `log_model.rs`, with a `selected` invokable | new |
| 154 | Main.updateFacets | `Filters::count_facets` (`filters_bridge.rs`), called when Rust's filter state says the bar is open or filtering (`shell.toggle_filters` already sets it) | small change |
| 201 | Main.askSaveCrops | `gallery` save-crops and the picker's purpose `SaveCrops` (`picker_bridge.rs`) | exists |
| 206 | Main.askCutBorders | `GalleryModel::cut_borders_count` and `cut_borders_confirmed` (already Rust) | small change: the controller starts the request |
| 222 | Main.askRemoveFolder | `library_bridge.rs` `roots` (QStringList) for the name; `remove(row)` already exists | new invokable: folder at a row |
| 249 | Main.relayout | `Command::Relayout` (controller) and `galleryModel.relayout`; the anchor choice moves into the controller | small change |
| 265 | Main.gridLoaded | `StartGate` (controller `start`), `Shell::list_shown` and `list_positioned` (`shell_bridge.rs`), and `grid_width` | exists, plus small change |
| 289 | Main.treeLoaded | `Shell::tree_loaded(folder_known)` and the first-placement rule (`treeShown`) in `StartGate` | small change |
| 306 | Main.currentRowOf | `FolderModel::reveal_listing` returns the row (`folders_bridge.rs`); the -1 and 0 mapping moves into `folders.rs` | small change |
| 319 | Main.reloadAll | `Event::Reload` (controller), `Shell::reload_list`, `FolderModel::reload`; one new invokable `reload_all` runs them | new invokable |
| 353 | Main.showAsked | `Shell::show_failed_imports` already decides the list; Rust emits `panel_close_requested(Inbox)` and `set_viewer_open(false)` | small change |
| 379 | Main.select | `Command::Select(usize)` and `Shell::select_requested` (`shell_bridge.rs`) | exists |
| 393 | Main.findSimilar | a new `Command::FindSimilar(FileId)` routed to the existing `yandex.searchFile` | new command |
| 398 | Main.findSimilarFromCommandLine | the command line's `find_similar` flag (`command_line.rs`) read by the controller at start, routed to `yandex.searchCommandLine` | new command |
| 906 | Main.startHooks | `Command::Started` and `Shell::started` (`shell_bridge.rs`); the hooks run in the controller's start | new sequencing |
| 86 | TestHooks.run | the command line's hook flags (`command_line.rs`); the controller issues the requests in order | new sequencing |
| 146 | TestHooks.runAfterLibraryStart | as `run`, after the library starts | new sequencing |
| 185 | Viewer.rate | `Command::Rate` and `Shell::rate` (`shell_bridge.rs`); the controller adds `Command::Step { dx: 1 }` after a rate (`controller.rs`, the `Event::Rate` arm) | small change |
| 62 | YandexPreview.step | the result count and `preview_item_after` (`yandex.rs`); a new `step_preview(by)` invokable in `yandex_bridge.rs` | new invokable |

Rows marked **new** are not large: each is a small invokable or a command variant. The sequencing rows (`startHooks`, `run`, `runAfterLibraryStart`) are the biggest change, and they depend on the start gate.

## Implementation status

Validated on Windows: `just check`, `just lint` (Rust clippy with warnings as errors, boundaries, markdown, and the QML
lint with no warnings), `just qmltest` (146 passed), and `just test` across the workspace (all test binaries pass). Not
run: `just shots` (the screenshot comparisons) and `just check-linux`.

Done (Rust owns the decision, QML forwards or renders):

- Inbox selection held by key in `notices` (`select`, `unselect`, `setDetailsOpen`, `dismissSelected`, `retrySelected`, `filesRequested`). Replaces the inbox row handlers, `requestFiles`, `dismissSelected`, `retrySelected`, `expand`'s details write, and the delegate's `tell` and `wanted`.
- Log selection held by record number in `log_model` (`select`, `unselect`). Replaces the log's row handlers.
- Filter counts: `Filters::set_bar_open` and `count_if_needed`. Replaces `updateFacets`.
- `Main.select`: now `Shell::select` (`Event::Select`). Replaces the window's own select.
- Find similar: `Shell::find_similar` and `find_similar_from_command_line` (`Event::FindSimilar`, `Event::FindSimilarCommandLine`), with `similarRequested` and `similarCommandLineRequested` back to the window. Replaces `findSimilar` and `findSimilarFromCommandLine`.
- Viewer rating: `Event::RateInViewer` rates and shows the next picture in Rust. `Viewer.rate` only signals. Replaces the step after a rating. The viewer test is changed to match, and the move-on needs a Rust test.
- Preview step: `YandexSearch::step_preview` wraps the results. `YandexPreview.step` forwards it.

Deletions: `refreshGrid`, `revealFile` and `Viewer.step` removed. `countFacets` and `reloadQuery` kept as typed forwarders (the QML lint needs them). `modalShown`, `togglePlay`, `Ratings.symbol` and `VideoTransport.time` kept as the one place each job is done.

Reclassified to QML: `askSaveCrops` and `askCutBorders` open dialogs (the save and the cut are already Rust), `showAsked` closes panels and positions the tree, `applyView`, `openPanel`, `closePanel`, and `YandexPanel.show` and `showCommandLine` (already forwarding Rust decisions).

Also done in this pass:

- `relayout`: `GalleryModel::relayout` takes the view's top item and returns the item to keep in view (the current item, else the top). QML keeps the positioning, which is view geometry.
- `currentRowOf`: `FolderModel::current_row_of` (`folders.rs`, `folders_bridge.rs`) holds the -1/0 mapping. Replaces the window's function.
- `askRemoveFolder`: `Library::folder_at` (`library.rs`, `library_bridge.rs`) gives the folder named in the dialog. QML still opens the dialog.

Moved in the last passes (each decision in Rust, QML only scrolls or renders):

- Start placement: `FolderModel::tree_loaded` places the tree and emits `treePlaced`; `FolderModel::reveal_tree` does the same after a Reveal. `GalleryModel::place_list` picks the item to show and whether to contain it, and emits `placeRequested`; `gridLoaded` and `relayout` both use it.
- Test hooks: the order and the letter-to-action mapping are in `crates/desktop/src/test_hooks.rs`. The shell runs the selection, grid keys and similar search; `TestHooks.perform` does the viewer and panel actions, by the `Kind` it is given.
- Scan reload: `Shell::reload_all` emits `foldersReloadRequested` (the folder model reloads), then reloads the list.
- Modal panels: `Shell::set_panel_shown` replaces `modalShown`.
- Reveal and failed files: the controller emits `ClosePanel(Inbox)`, and `CloseViewer` when the viewer is open. `showAsked` keeps only the filter reset, the tree, and the focus.
- Inbox and log: the panels take their selection values as inputs from the window, and ask for each change through a signal. The window makes the model call. The failed-files request goes from the notices model to the failed-files model in `Main.qml`.

Left in QML:

- `countFacets` and `reloadQuery` stay as typed forwarders (the QML lint needs them).
- `Ratings.symbol`, `VideoTransport.time`, `Viewer.togglePlay` and `MediaView.togglePlay` each hold one job.

Not covered by a test yet: the window's selection rules that the QML tests no longer check (the selection follows its entry as rows move, and its details close when the entry goes, in `notices.rs`; the selection follows its record through a trim, in `log_model.rs`). They need a Rust test once the lookup is a pure function.
