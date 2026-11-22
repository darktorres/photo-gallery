// Main window: the composition of the page (top bar, folder tree and grid, status bar, filters, viewer) and the panels made
// when first needed. It owns the Rust objects (the gallery, the folders, the library, the shell) and makes their calls; the
// components report what the user asks for through signals. The window's policy, the start and the listing included, is in
// Shell (the controller in pg-client); what is here is what it asks for, the positions and the test hooks (TestHooks).
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Window
import pg.gallery

ApplicationWindow {
    id: win
    Words {
        id: words
    }
    width: 1600
    height: 1000
    // Shown after the load, once it can be handed the app's D3D11 device (src/gpu_device.rs).
    visible: false
    title: shell.folder.length ? words.phrase("ui-window-title", 0, shell.folder, "") : words.text("ui-app-name")
    color: "#181818"
    palette {
        window: "#202020"
        windowText: "#dddddd"
        base: "#1c1c1c"
        alternateBase: "#242424"
        text: "#dddddd"
        button: "#2d2d2d"
        buttonText: "#dddddd"
        highlight: Theme.chosenRow
        highlightedText: "#ffffff"
        mid: "#3a3a3a"
        dark: "#111111"
        light: "#4a4a4a"
        placeholderText: "#777777"
    }

    Settings {
        id: appSettings
    }

    // The grid's row height in logical px: the thumbnail size, adjustable at any time (slider, Ctrl+wheel,
    // Ctrl +/-). The layout follows it at once; the thumbnails re-requested at the new size follow after a short
    // pause, and meanwhile the old ones are scaled. Remembered across runs, by the shell.
    // The size, the filter bar's and the borders' flags are Shell's: read here, changed by its invokables.
    readonly property real rowHeight: shell.rowHeight
    // The selected item (-1: none): the gallery's, so it stays with its file when the list is refreshed.
    property alias current: galleryModel.current
    readonly property bool filtersOpen: shell.filtersOpen
    // B: the grid and the viewer show what cropping the borders would cut. The window starts with the prefs' (or
    // --borders), and the controller saves the user's toggles.
    readonly property bool showBorders: shell.showBorders
    // Border detection is on (the Library panel's choice): the borders controls act only then.
    readonly property bool bordersOn: appSettings.detectBorders
    // The library panel is shown; made the first time (libraryLoader).
    property bool libraryOpen: false
    onLibraryOpenChanged: if (libraryOpen)
        libraryLoader.active = true
    // The log view is shown (dev mode); made the first time (logLoader).
    property bool logOpen: false
    onLogOpenChanged: if (logOpen)
        logLoader.active = true
    // The inbox is shown; made the first time (inboxLoader). While it is open the inbox reads its ledgers' counts, and
    // while it is open in the active window what it shows is marked read (Notices.setViewing).
    property bool inboxOpen: false
    onInboxOpenChanged: {
        if (inboxOpen) {
            inboxLoader.active = true;
            inboxModel.dismissToast();
        } else {
            inboxExpandPass = -1;
        }
        inboxModel.setViewing(inboxOpen, win.active);
    }
    // The ledger whose entry opens with the inbox, and again when it appears (a lane row's "N failed": its pass's
    // ledger; --inbox-expand; -1: none): a `pass` code, a bit of Notices.passes.
    property int inboxExpandPass: -1
    // The start is over (Shell.started, the test hooks have run): the --screenshot hook waits for it.
    property bool ready: false
    // A list that arrived before the grid had a width: positioned once it is laid out (`loadPending` says there is one;
    // `loadedIndex` is its index, -1 for none).
    property bool loadPending: false
    property int loadedIndex: -1
    readonly property real gap: 4

    // Both read the catalog off the GUI thread: what they were asked for arrives later (onLoaded).
    FolderModel {
        id: folderModel
        onLoaded: win.treeLoaded()
    }
    GalleryModel {
        id: galleryModel
        // Without a selection, a refreshed list keeps the top row's file in view. The rows change (all of them for
        // another list), then gridLoaded moves the view to where it belongs.
        onAboutToLoad: {
            galleryModel.anchor = win.current >= 0 ? -1 : grid.topItem();
            grid.settling = true;
        }
        onLoaded: index => win.gridLoaded(index)
    }
    // The start's stages, for measuring it (src/trace.rs).
    Stages {
        id: stages
    }
    // The window's policy is the controller's (pg-client, held by Shell in src/shell.rs): the start, what the first list
    // selects, the listing (where the grid looks, with or without subfolders, in what order, with the filter bar's
    // query), the keys, the size and the pauses. Its commands come back as these signals; what is here is how each is
    // shown. The shell emits them after its call (Shell.later), so a handler may call the shell back.
    Shell {
        id: shell
        onListingChanged: win.showListing()
        onStarted: {
            win.ready = true;
            win.startHooks();
        }
        onRelayoutDue: win.relayout()
        onSelectRequested: i => win.scrollTo(i)
        // A test hook the window performs (the shell ran the rest, in order: TestHooks.perform).
        onTestStep: (kind, index, value, text) => testHooks.perform(kind, index, value, text)
        // The library was scanned: the tree is read again, then the open folder's files (Shell.reloadAll).
        onFoldersReloadRequested: folderModel.reload()
        // The controller asked for a similar-search panel: it is made (its loader) and the panel searches.
        onSimilarRequested: fileId => {
            yandexLoader.active = true;
            win.similarSearchRequested(fileId);
        }
        onSimilarCommandLineRequested: {
            yandexLoader.active = true;
            win.similarCommandLineRequested();
        }
        onViewerRequested: i => viewerItem.show(i)
        onPanelOpenRequested: kind => win.openPanel(kind)
        onPanelCloseRequested: kind => win.closePanel(kind)
        // A listing that shows a reveal or the failed files takes the viewer away (the controller's CloseViewer).
        onViewerCloseRequested: {
            if (viewerItem.visible)
                viewerItem.close();
        }
        // A Reveal's folder is read and listed (Shell.reveal, Shell.revealFolder).
        onRevealShown: win.showAsked()
    }
    // The core, for the screenshot hook's settle check: its pending pauses are asked (cpp/screenshot.cpp).
    readonly property var shellCore: shell
    Connections {
        id: firstFrame
        target: win
        function onFrameSwapped() {
            firstFrame.enabled = false;
            stages.firstFrame();
        }
    }
    Library {
        id: library
        onScanned: shell.reloadAll()
        onImported: shell.reloadList()
        // The job's progress was read again: the status bar's rows follow it (LaneModel).
        onProgressed: laneRows.refresh()
        // Turned files changed in place: no renumbering, so the viewer can stay open (the controller reads them again).
        onFilesChanged: shell.filesChanged()
    }
    // Where the view and the tree go is the model's choice; these only scroll them to it (GalleryModel.placeRequested,
    // FolderModel.treePlaced).
    Connections {
        target: galleryModel
        function onPlaceRequested(item: int, contain: bool) {
            if (item >= 0)
                grid.positionAt(galleryModel.rowOf(item), contain ? ListView.Contain : ListView.Beginning);
            else
                grid.positionAtBeginning();
        }
    }
    Connections {
        target: folderModel
        function onTreePlaced(row: int, reveal: bool) {
            tree.currentIndex = row;
            if (reveal)
                tree.positionViewAtIndex(row, ListView.Contain);
        }
    }

    // The pauses (the thumbnails, the saved selection, the layout, the list after a query) are the controller's
    // (Shell.currentChanged, gridResized, queryEdited); their commands come back as Shell's signals above.
    onCurrentChanged: shell.currentChanged(current)
    onClosing: shell.flush()

    // The filters changed: the list is shown after a short pause (Shell.queryEdited; the controller's pause opens the list), so typing a number
    // doesn't re-list at every key. The counts next to the filter choices are the filter bar's model's to keep: it counts
    // them while the bar is open or a filter is set.
    onFiltersOpenChanged: filterBar.setBarOpen(filtersOpen)

    // The folder pickers: the system's dialog where it is used, else the app's own (FolderPickerDialog). Each picker is
    // told when its folder is chosen (`done`), and then its action takes the folder.
    FolderPicker {
        id: addPicker
        onDone: library.addPicked()
        onOpenInApp: addDialog.open = true
    }
    FolderPickerDialog {
        id: addDialog
        z: 100
        names: addPicker.names
        here: addPicker.here
        onGoUp: addPicker.up()
        onOpenRow: row => addPicker.enter(row)
        onChooseRow: row => addPicker.choose(row)
        onChooseHere: addPicker.chooseHere()
        onCancelled: addPicker.cancel()
        onVisibleChanged: shell.setPanelShown(PanelKind.FolderPicker, visible)
    }
    FolderPicker {
        id: cropPicker
        onDone: galleryModel.saveCroppedCopiesPicked()
        onOpenInApp: cropDialog.open = true
    }
    FolderPickerDialog {
        id: cropDialog
        z: 100
        names: cropPicker.names
        here: cropPicker.here
        onGoUp: cropPicker.up()
        onOpenRow: row => cropPicker.enter(row)
        onChooseRow: row => cropPicker.choose(row)
        onChooseHere: cropPicker.chooseHere()
        onCancelled: cropPicker.cancel()
        onVisibleChanged: shell.setPanelShown(PanelKind.FolderPicker, visible)
    }

    // Where the filter bar's "Save cropped copies" puts them: the folder is asked for, then they are made.
    function askSaveCrops() {
        cropPicker.pick(PickerPurpose.SaveCrops);
    }

    // Where the filter bar's "Cut borders from originals" starts: the count is asked for, then the dialog confirms.
    function askCutBorders() {
        cutDialog.count = galleryModel.cutBordersCount();
        cutDialog.open();
    }

    // The cut dialog's confirmation is the only way the originals are rewritten. It reports as the save-view dialog to the
    // shell: PanelKind has no kind of its own, and any modal dialog counts the same for the shortcuts.
    CutBordersDialog {
        id: cutDialog
        onConfirmed: galleryModel.cutBordersConfirmed()
        onClosed: win.refocus()
        onVisibleChanged: shell.setPanelShown(PanelKind.SaveViewDialog, visible)
    }

    // Where the library panel's Remove starts: the folder is named, then the dialog confirms. The folder leaves the library
    // for good (its images and their ratings), so nothing is removed before the user's confirmation.
    function askRemoveFolder(row: int) {
        removeDialog.row = row;
        removeDialog.folder = library.folderAt(row);
        removeDialog.open();
    }

    RemoveFolderDialog {
        id: removeDialog
        onConfirmed: library.removeFolder(removeDialog.row)
        onClosed: win.refocus()
        onVisibleChanged: shell.setPanelShown(PanelKind.SaveViewDialog, visible)
    }

    // Where the library panel's Discard starts: the removal is named, then the dialog confirms. Its changes are given up
    // for good, so nothing is dropped before the user's confirmation.
    function askDiscardRemoval(folder: string) {
        discardDialog.folder = folder;
        discardDialog.open();
    }

    DiscardRemovalDialog {
        id: discardDialog
        onConfirmed: library.discardRemoval(discardDialog.folder)
        onClosed: win.refocus()
        onVisibleChanged: shell.setPanelShown(PanelKind.SaveViewDialog, visible)
    }

    // The item to keep in view is the model's choice (the current item, else the top of the view, read before the change).
    function relayout() {
        const top = grid.topItem();
        grid.settling = true;
        const anchor = galleryModel.relayout(grid.gridWidth, rowHeight, gap, top);
        if (loadPending) {
            gridLoaded(loadedIndex);
            return;
        }
        if (anchor >= 0)
            galleryModel.placeList(anchor);
        grid.settling = false;
    }

    // A list asked for is shown: keep the selection in view, else the file that was at the top (`index`), else
    // start at the top. The gate hears of the list first (Shell.listShown: the file the last session left is selected
    // before the start), then of its placement (Shell.listPositioned, which can finish the start).
    function gridLoaded(index: int) {
        // Laid out only once the grid has a width (relayout() comes back here). The grid has no rows meanwhile.
        if (galleryModel.count > 0 && galleryModel.rowOf(0) < 0) {
            loadedIndex = index;
            loadPending = true;
            return;
        }
        loadPending = false;
        shell.listShown(galleryModel.count);
        // Where the view goes is the model's (placeRequested).
        galleryModel.placeList(index);
        grid.settling = false;
        shell.listPositioned();
    }

    // The folder tree is in. At the start, the open folder is shown in it (its parents expanded); after a scan it
    // is only found again, the tree left as the user left it. The folder listed is the view's (the saved one, or the
    // applied view's): when the tree doesn't have it, the controller lists all folders.
    // The folder model works out the listed folder's row (-1: under a folder the user collapsed, or not in the tree).
    // The shell keeps the folder as the tree spells it and tells the controller whether the folder is known.
    function treeLoaded() {
        // The model places the tree (treePlaced), then the shell hears whether the listed folder is known.
        folderModel.treeLoaded();
        shell.treeLoaded(folderModel.listedKnown());
    }

    // The subfolders were switched: for the empty folder that moves the listing between "All" (with them) and "Drive
    // roots" (without), so the tree's current row follows the listing (currentRowOf).
    function followListedRow() {
        tree.currentIndex = folderModel.currentRowOf(folderModel.revealListing(false));
    }

    // The list the controller asked for is loading (the shell announces it: listingChanged). The facets and the sort box
    // follow it; the session is saved by the controller.
    function showListing() {
        filterBar.countIfNeeded();
        topBar.sortIndex = shell.sortIndex();
    }

    // A saved view's listing is the shell's (it has announced it); the filter bar shows its query, the tree its folder.
    function applyView(name: string) {
        if (!shell.applyView(name))
            return;
        filterBar.reloadQuery();
        // The applied view's folder is shown in the tree (its parents expanded), when the tree has it.
        const row = folderModel.revealListing(true);
        tree.currentIndex = Math.max(row, 0);
        if (row >= 0)
            tree.positionViewAtIndex(row, ListView.Contain);
        grid.takeFocus();
    }

    // The inbox asked for a list of its own (a failed file's folder, the failed imports), which the shell has announced:
    // the inbox and the viewer close, the filter bar shows the query, the tree the folder.
    // The listing that shows a reveal or the failed files: the inbox and the viewer are closed by the controller
    // (ClosePanel, CloseViewer); here the filters, the tree and the focus follow the listing.
    function showAsked() {
        filterBar.reloadQuery();
        folderModel.revealTree();
        grid.takeFocus();
    }

    // Show item i in the grid: its row, brought into view. The selection is the model's (the shell sets it).
    function scrollTo(i: int) {
        const row = galleryModel.rowOf(i);
        if (row >= 0)
            grid.positionAt(row, ListView.Contain);
    }


    // To the panel made when first needed (Yandex's Loader, below): what it is asked to do comes as a signal once it
    // is made (a Loader not loading asynchronously makes its item at once).
    signal similarSearchRequested(fileId: real)
    // The command line's `--find-similar`: the photo is the path it names, not a catalog file.
    signal similarCommandLineRequested


    // The save-view dialog (saveViewLoader) opens each time this rises: made at the first ask, then kept.
    property int saveViewRequests: 0
    function askSaveView() {
        saveViewLoader.active = true;
        saveViewRequests++;
    }

    // The key map (pg-client's keys, through the KeyMap facade) names the grid's action for a key; this carries it out.
    KeyMap {
        id: keymap
    }
    // The window's shortcuts (the inbox, the log view, dev mode): matched before any item's keys, decided by the controller.
    GlobalShortcuts {
        onActivated: code => shell.globalAction(code)
    }
    // The log's records, for the log view; read when it first opens (LogModel.start).
    LogModel {
        id: logModel
    }
    // The lanes the status bar shows, one row each (LaneModel). Refreshed as the job's progress is read; while a row only
    // waits out its hold, the timer asks again, so the row goes when it should.
    LaneModel {
        id: laneRows
        Component.onCompleted: refresh()
    }
    Timer {
        interval: 250
        repeat: true
        running: laneRows.settling
        onTriggered: laneRows.refresh()
    }
    // The inbox: its entries, the bell's count and the toast. Followed from the start, so the opening's notices toast.
    Notices {
        id: inboxModel
        Component.onCompleted: start()
    }
    // A ledger's entry is open: its failed files are to be read (-1: none), once per entry and newest failure (Notices).
    Connections {
        target: inboxModel
        function onFilesRequested(pass: int) {
            failedFilesModel.show(pass);
        }
    }
    // The failed files of the inbox's ledger entry shown.
    FailedFiles {
        id: failedFilesModel
    }
    // What another process changed in the inbox shows when the window comes back; what the open inbox shows is read
    // once the window is active again.
    onActiveChanged: {
        if (active)
            inboxModel.poke();
        inboxModel.setViewing(inboxOpen, active);
    }

    // A shortcut asked for a panel (Shell.panelOpenRequested, panelCloseRequested).
    function openPanel(kind: int) {
        if (kind === PanelKind.Log) {
            logModel.start();
            win.logOpen = true;
        } else if (kind === PanelKind.Inbox) {
            win.inboxOpen = true;
        }
    }
    function closePanel(kind: int) {
        if (kind === PanelKind.Log)
            win.logOpen = false;
        else if (kind === PanelKind.Inbox)
            win.inboxOpen = false;
    }

    // Back to where the keys were before a panel: the viewer when it is open, else the grid.
    function refocus() {
        if (viewerItem.visible)
            viewerItem.forceActiveFocus();
        else
            grid.takeFocus();
    }

    // The status bar's selection: the item's info and rating, or the key hints with none selected.
    readonly property string selectionInfo: win.current >= 0 ? galleryModel.selectionInfo : keymap.gridHint()

    // The test hooks (command-line options): the shell runs them in order (Shell.runTestHooks), and hands the window
    // each one it performs (Shell.testStep, TestHooks.perform).
    TestHooks {
        id: testHooks
        viewer: viewerItem
        onBorderMarkToggled: i => shell.toggleBorderMark(i)
        // The user's B, as the test hook asks for it: the toggle, so the controller saves it as the user's key does.
        onBordersRequested: if (!win.showBorders)
            shell.toggleBorders()
        onViewsMenuRequested: topBar.openViewsMenu()
        onLibraryPanelRequested: {
            win.libraryOpen = true;
        }
        onSaveViewDialogRequested: win.askSaveView()
        onAddFolderRequested: addPicker.chooseCommandLine(0)
        onSaveCropsRequested: cropPicker.chooseCommandLine(1)
        onViewerShowRequested: index => viewerItem.show(index)
        onViewerPlayRequested: viewerItem.togglePlay()
        onViewerSetZoomRequested: (z, px, py) => viewerItem.setZoom(z, px, py)
        onViewerTurnRequested: turns => viewerItem.turn(turns)
        onViewerRateRequested: rating => viewerItem.rate(rating)
        onLogPanelRequested: shell.globalAction(GlobalShortcuts.Log)
        onInboxPanelRequested: shell.globalAction(GlobalShortcuts.Inbox)
        onInboxExpandRequested: code => win.inboxExpandPass = inboxModel.passOfCode(code)
    }

    Item {
        id: page
        anchors.fill: parent

        TopBar {
            id: topBar
            folder: shell.folder
            recursive: shell.recursive
            folderKind: shell.folderKind
            descending: shell.descending
            savedViews: shell.savedViews
            hasSavedViews: shell.savedViews.length > 0
            // A list held under the viewer is loaded: it is not loading to the user until it shows.
            loading: galleryModel.loading && !galleryModel.holding
            fileCount: galleryModel.count
            filterActive: filterBar.active
            filterTotal: filterBar.total
            filtersOpen: win.filtersOpen
            rowHeight: win.rowHeight
            minRowHeight: shell.minRowHeight
            maxRowHeight: shell.maxRowHeight
            onFiltersToggled: shell.toggleFilters()
            onRecursiveToggled: on => {
                shell.setRecursive(on);
                win.followListedRow();
            }
            onSortActivated: i => {
                // The last sort is by border depth: with border detection off it is not applied, and the box shows the
                // sort that is in force again.
                if (i === topBar.sortNames.length - 1 && !win.bordersOn) {
                    featurePopup.show(words.text("ui-borders-disabled"));
                    topBar.sortIndex = shell.sortIndex();
                    return;
                }
                shell.sortBy(i);
                grid.takeFocus();
            }
            onDescendingToggled: {
                shell.toggleDescending();
                grid.takeFocus();
            }
            onViewChosen: name => win.applyView(name)
            onViewDeleteRequested: name => shell.deleteView(name)
            onSaveViewRequested: win.askSaveView()
            onLibraryRequested: {
                win.libraryOpen = true;
            }
            onSizeChosen: h => shell.sizeChosen(h)
            onViewsMenuOpenChanged: shell.setPanelShown(PanelKind.ViewsMenu, viewsMenuOpen)
        }

        StatusBar {
            id: bottomBar
            selection: win.selectionInfo
            writeStatus: library.writeStatus
            writeFailed: library.writeFailed
            toast: inboxModel.toast
            toastLevel: inboxModel.toastLevel
            unread: inboxModel.unread
            laneModel: laneRows
            inboxPasses: inboxModel.passes
            devMode: shell.devMode
            // The screenshots: an info toast stays up while the window settles (no timer runs).
            toastHeld: testHooks.screenshot
            onLogRequested: shell.globalAction(GlobalShortcuts.Log)
            onToastExpired: inboxModel.dismissToast()
            onInboxRequested: shell.globalAction(GlobalShortcuts.Inbox)
            onLaneFailedRequested: pass => {
                win.inboxExpandPass = pass;
                if (!win.inboxOpen)
                    shell.globalAction(GlobalShortcuts.Inbox);
            }
            onLanePauseRequested: (number, paused) => library.setLanePaused(number, paused)
        }

        FilterBar {
            id: filterBar
            onEdited: shell.queryEdited()
            croppable: galleryModel.croppable
            bordersOn: win.bordersOn
            onBordersBlocked: featurePopup.show(words.text("ui-borders-disabled"))
            onNothingToCrop: featurePopup.show(words.text("ui-nothing-to-crop"))
            onSaveCropsRequested: win.askSaveCrops()
            onCutBordersRequested: win.askCutBorders()
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: topBar.bottom
            visible: win.filtersOpen
        }

        SplitView {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: filterBar.visible ? filterBar.bottom : topBar.bottom
            anchors.bottom: bottomBar.top

            Item {
                SplitView.preferredWidth: 320
                SplitView.minimumWidth: 160

                FolderTree {
                    id: tree
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.bottom: mascot.top
                    folders: folderModel
                    onToggleRequested: row => folderModel.toggle(row)
                    onFolderOpened: row => {
                        shell.openFolderRow(row);
                        grid.takeFocus();
                    }
                }

                // Under the tree, at the bottom of the column.
                Mascot {
                    id: mascot
                    pinned: testHooks.mascot
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                }
            }

            GalleryGrid {
                id: grid
                SplitView.fillWidth: true
                gallery: galleryModel
                gap: win.gap
                current: win.current
                showBorders: win.showBorders && win.bordersOn
                noFolders: library.roots.length === 0
                filtering: filterBar.active > 0
                importing: library.busy
                onKeyPressed: e => e.accepted = shell.keyPressed(e.key, e.modifiers)
                onCellClicked: i => {
                    win.current = i;
                }
                onCellDoubleClicked: i => viewerItem.show(i)
                onSizeStepped: factor => shell.sizeStepped(factor)
                onGridResized: width => shell.gridResized(width)
            }
        }

        Viewer {
            id: viewerItem
            anchors.fill: parent
            dpr: win.devicePixelRatio
            gallery: galleryModel
            fullUrl: galleryModel.viewedFullUrl
            index: galleryModel.viewing
            count: galleryModel.count
            showBorders: win.showBorders
            settings: appSettings
            onMoved: i => galleryModel.viewing = i
            onStepRequested: d => stepTo(shell.viewerStep(index, d))
            // Opened or closed: the core knows (its keys are the viewer's while it is open); closed, the keys go back to
            // the grid.
            onVisibleChanged: {
                shell.setViewerOpen(visible);
                if (!visible)
                    grid.takeFocus();
            }
            onRated: (i, r) => shell.rateInViewer(i, r)
            onTurned: (i, d) => shell.turn(i, d)
            onBordersToggled: {
                if (win.bordersOn)
                    shell.toggleBorders();
                else
                    featurePopup.show(words.text("ui-borders-disabled"));
            }
            onBorderMarkToggled: (i, m) => {
                if (win.bordersOn)
                    shell.setBorderMark(i, m);
                else
                    featurePopup.show(words.text("ui-borders-disabled"));
            }
            // The file the viewer is on is the one a session remembers (the shell reads it, `gallery::shown_file`).
            onIndexChanged: if (index >= 0)
                shell.currentChanged(index)
            onFindSimilar: fileId => shell.findSimilar(fileId)
            onClosed: i => shell.select(i)
        }

        // The library folders, over a dimmed window (an item in the page, like the viewer, so window grabs include it).
        // Made the first time it is opened (libraryOpen), with its folder dialog, then kept.
        Loader {
            id: libraryLoader
            anchors.fill: parent
            active: false
            sourceComponent: Component {
                LibraryPanel {
                    open: win.libraryOpen
                    roots: library.roots
                    keptRemovals: library.keptRemovals
                    removingRemovals: library.removingRemovals
                    hasRemovals: library.keptRemovals.length + library.removingRemovals.length > 0
                    hasRoots: library.roots.length > 0
                    writeBack: appSettings.writeBack
                    detectBorders: appSettings.detectBorders
                    detectDuplicates: appSettings.detectDuplicates
                    thumbCache: appSettings.thumbCache
                    onDismissed: {
                        win.libraryOpen = false;
                    }
                    onHidden: grid.takeFocus()
                    onVisibleChanged: shell.setPanelShown(PanelKind.Library, visible)
                    onAddRequested: addPicker.pick(PickerPurpose.AddFolder)
                    onRemoveRequested: row => win.askRemoveFolder(row)
                    onDiscardRequested: folder => win.askDiscardRemoval(folder)
                    onRescanRequested: library.rescan()
                    onWriteBackToggled: on => {
                        appSettings.writeBack = on;
                    }
                    onDetectBordersToggled: on => {
                        appSettings.detectBorders = on;
                        // Off: the listing drops what borders give, as it would have had them never been used.
                        if (!on)
                            shell.bordersTurnedOff();
                    }
                    onDetectDuplicatesToggled: on => {
                        appSettings.detectDuplicates = on;
                    }
                    onThumbCacheToggled: on => {
                        appSettings.thumbCache = on;
                    }
                }
            }
        }

        // Border detection is off and a control that needs it was used: the text says why. Not modal: a press outside closes
        // it, and reaches the control under it.
        Popup {
            id: featurePopup
            property string message: ""
            modal: false
            focus: false
            padding: 12
            closePolicy: Popup.CloseOnPressOutside
            x: Math.round((win.width - width) / 2)
            y: win.height - height - 64
            Label {
                text: featurePopup.message
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                width: Math.min(360, win.width - 48)
            }
            Timer {
                id: featurePopupTimer
                interval: 4000
                onTriggered: featurePopup.close()
            }
            function show(text: string) {
                message = text;
                open();
                featurePopupTimer.restart();
            }
        }
        Connections {
            target: shell
            function onBordersRefused() {
                featurePopup.show(words.text("ui-borders-disabled"));
            }
            function onOptionsRefused() {
                featurePopup.show(words.text("ui-options-borders-disabled"));
            }
        }

        // Reverse image search for the viewer's photo (over the viewer, so it comes after it). Made with the first
        // search (findSimilar), then kept.
        Loader {
            id: yandexLoader
            anchors.fill: parent
            active: false
            sourceComponent: Component {
                YandexPanel {
                    id: yandexPanel
                    dpr: win.devicePixelRatio
                    gap: win.gap
                    previewWhenFound: testHooks.yandexPreview
                    // Back to the viewer it was opened from, so its arrow keys work again; the grid only if the viewer
                    // is gone.
                    onClosed: win.refocus()
                    onVisibleChanged: shell.setPanelShown(PanelKind.Yandex, visible)
                    onPreviewOpenChanged: shell.setPanelShown(PanelKind.YandexPreview, previewOpen)
                    Connections {
                        target: win
                        function onSimilarSearchRequested(fileId: real) {
                            yandexPanel.show(fileId);
                        }
                        function onSimilarCommandLineRequested() {
                            yandexPanel.showCommandLine();
                        }
                    }
                }
            }
        }

        // Made the first time it is asked for (askSaveView), then kept.
        Loader {
            id: saveViewLoader
            active: false
            sourceComponent: Component {
                SaveViewDialog {
                    host: page
                    requests: win.saveViewRequests
                    folder: shell.folder
                    recursive: shell.recursive
                    folderKind: shell.folderKind
                    descending: shell.descending
                    filters: filterBar.description
                    sortName: topBar.sortText
                    caret: !testHooks.screenshot
                    onSaved: name => shell.saveView(name)
                    onClosed: win.refocus()
                    onVisibleChanged: shell.setPanelShown(PanelKind.SaveViewDialog, visible)
                }
            }
        }

        // The log view (dev mode), over the rest. Made the first time it is opened (logOpen), then kept.
        Loader {
            id: logLoader
            anchors.fill: parent
            active: false
            sourceComponent: Component {
                LogPanel {
                    open: win.logOpen
                    records: logModel
                    selected: logModel.selectedRow
                    selectedSeq: logModel.selectedSeq
                    selectedText: logModel.selectedText
                    onSelectRequested: row => logModel.select(row)
                    onUnselectRequested: logModel.unselect()
                    onDismissed: win.logOpen = false
                    onHidden: win.refocus()
                    onVisibleChanged: shell.setPanelShown(PanelKind.Log, visible)
                }
            }
        }

        // The inbox, over the rest. Made the first time it is opened (inboxOpen), then kept.
        Loader {
            id: inboxLoader
            anchors.fill: parent
            active: false
            sourceComponent: Component {
                InboxPanel {
                    open: win.inboxOpen
                    notices: inboxModel
                    selected: inboxModel.selectedRow
                    selectedKey: inboxModel.selectedKey
                    selectedText: inboxModel.selectedText
                    selectedPass: inboxModel.selectedPass
                    selectedSerial: inboxModel.selectedSerial
                    selectedCanRetry: inboxModel.selectedCanRetry
                    selectedCanShowFiles: inboxModel.selectedCanShowFiles
                    detailsOpen: inboxModel.detailsOpen
                    failedFiles: failedFilesModel
                    failedTotal: failedFilesModel.total
                    failedLoading: failedFilesModel.loading
                    failedError: failedFilesModel.error
                    // The pass's ledger entry, once the inbox lists it (its bit of `passes`): this follows it appearing.
                    expandRow: win.inboxExpandPass >= 0 && (inboxModel.passes & (1 << win.inboxExpandPass)) !== 0 ? inboxModel.rowOfKey(inboxModel.passKey(win.inboxExpandPass)) : -1
                    onCopyRequested: row => inboxModel.copy(row)
                    onDismissRequested: row => inboxModel.dismiss(row)
                    onRetryRequested: row => inboxModel.retry(row)
                    onSelectRequested: row => inboxModel.select(row)
                    onUnselectRequested: inboxModel.unselect()
                    onDetailsOpenRequested: open => inboxModel.setDetailsOpen(open)
                    onDismissSelectedRequested: inboxModel.dismissSelected()
                    onRetrySelectedRequested: inboxModel.retrySelected()
                    // An inbox entry's Reveal: the file in its folder, selected once shown; a file the library no longer
                    // has toasts. The folder is read off the window's thread: the window follows when it is listed
                    // (Shell.revealShown).
                    onRevealRequested: fileId => shell.reveal(fileId)
                    onFolderRevealRequested: row => shell.revealFolder(row)
                    onShowFilesRequested: {
                        shell.showFailedImports();
                        win.showAsked();
                    }
                    onClearRequested: inboxModel.dismissAll()
                    onDismissed: win.inboxOpen = false
                    onHidden: win.refocus()
                    onVisibleChanged: shell.setPanelShown(PanelKind.Inbox, visible)
                }
            }
        }
    }

    // The window is made (it shows right after): ask for where the user was. Nothing here waits for the catalog;
    // the tree and the files arrive later, in either order (treeLoaded, gridLoaded), and the second of them
    // finishes the start (started).
    // A window moved to a screen with another pixel ratio: the thumbnails are made at the new size (the controller's scale).
    onDevicePixelRatioChanged: shell.setScale(win.devicePixelRatio)

    Component.onCompleted: {
        // The folder, subfolders, sort and query are the listing's and the filter bar's as they start (the controller's
        // start: the session's, then the test hooks' --sort, --query and --apply-view).
        // The start's size, borders, dev mode and filter bar are the controller's, from the prefs and the options. The
        // scale is the window's, and follows the window to another screen; the thumbnails are sized from it before the
        // library starts (startHooks).
        shell.setScale(win.devicePixelRatio);
        topBar.sortIndex = shell.sortIndex();
        relayout();
        folderModel.reload();
        // As it was left: whether the folder still exists is known once the tree is in (treeLoaded).
        shell.reloadList();
    }

    // The gate let the start through (Shell.started): run the test hooks, then start the background job (after the
    // first reads, which it would compete with for the disk).
    function startHooks() {
        stages.ready();
        shell.runTestHooks();
        library.start();
        shell.runLibraryHooks();
    }
}
