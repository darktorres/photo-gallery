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
    Words {
        id: words
    }
    id: win
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
        highlight: "#2f5f93"
        highlightedText: "#ffffff"
        mid: "#3a3a3a"
        dark: "#111111"
        light: "#4a4a4a"
        placeholderText: "#777777"
    }

    Settings { id: appSettings }

    // The grid's row height in logical px: the thumbnail size, adjustable at any time (slider, Ctrl+wheel,
    // Ctrl +/-). The layout follows it at once; the thumbnails re-requested at the new size follow after a short
    // pause (thumbHeight), and meanwhile the old ones are scaled. Remembered across runs.
    // The size, the filter bar's and the borders' flags are Shell's: read here, changed by its invokables.
    readonly property real rowHeight: shell.rowHeight
    readonly property int thumbHeight: shell.thumbHeight
    // The selected item (-1: none): the gallery's, so it stays with its file when the list is refreshed.
    property alias current: galleryModel.current
    readonly property bool filtersOpen: shell.filtersOpen
    // B: the grid and the viewer show what cropping the borders would cut. Remembered across runs (the test hook
    // sets it for its run only).
    readonly property bool showBorders: shell.showBorders
    // Remembered across runs whenever it changes, from the keys or the menu alike (test runs use their own prefs file).
    onShowBordersChanged: appSettings.showBorders = showBorders
    // The library panel is shown; made the first time (libraryLoader).
    property bool libraryOpen: false
    onLibraryOpenChanged: if (libraryOpen) libraryLoader.active = true
    // The log view is shown (dev mode); made the first time (logLoader).
    property bool logOpen: false
    onLogOpenChanged: if (logOpen) logLoader.active = true
    // The inbox is shown; made the first time (inboxLoader). Opening it marks its entries read.
    property bool inboxOpen: false
    onInboxOpenChanged: {
        if (inboxOpen) {
            inboxLoader.active = true
            inboxModel.markAllRead()
            inboxModel.dismissToast()
        } else {
            inboxExpandRow = -1
        }
    }
    // The inbox's row to open with it (the status bar's "N failed": the pass's ledger; -1: none).
    property int inboxExpandRow: -1
    // The start is over (Shell.started, the test hooks have run): the --screenshot hook waits for it.
    property bool ready: false
    // A list that arrived before the grid had a width: positioned once it is laid out (-2: none).
    property int loadedIndex: -2
    readonly property real gap: 4

    // Both read the catalog off the GUI thread: what they were asked for arrives later (onLoaded).
    FolderModel {
        id: folderModel
        onLoaded: win.treeLoaded()
    }
    GalleryModel {
        id: galleryModel
        // The thumbnails' height (the screen's scale, from the shell): the model's thumbnail URLs are made for it.
        thumbHeight: win.thumbHeight
        // Without a selection, a refreshed list keeps the top row's file in view. The rows change (all of them for
        // another list), then gridLoaded moves the view to where it belongs.
        onAboutToLoad: {
            galleryModel.anchor = win.current >= 0 ? -1 : grid.topItem()
            grid.settling = true
        }
        onLoaded: (index) => win.gridLoaded(index)
    }
    // The start's stages, for measuring it (src/trace.rs).
    Stages { id: stages }
    // The window's policy is the controller's (pg-client, held by Shell in src/shell.rs): the start, what the first list
    // selects, the listing (where the grid looks, with or without subfolders, in what order, with the filter bar's
    // query), the keys, the size and the pauses. Its commands come back as these signals; what is here is how each is
    // shown. The shell emits them after its call (Shell.later), so a handler may call the shell back.
    Shell {
        id: shell
        onListingChanged: win.showListing()
        onStarted: {
            win.ready = true
            win.startHooks()
        }
        onRelayoutDue: win.relayout()
        onSessionDue: if (win.ready) appSettings.lastFile = galleryModel.fileId(viewerItem.visible ? viewerItem.index : win.current)
        onSaveRowHeight: (height) => appSettings.rowHeight = height
        onSelectRequested: (i) => win.scrollTo(i)
        onViewerRequested: (i) => viewerItem.show(i)
        onPanelOpenRequested: (kind) => win.openPanel(kind)
        onPanelCloseRequested: (kind) => win.closePanel(kind)
    }
    // The core, for the screenshot hook's settle check: its pending pauses are asked (cpp/screenshot.cpp).
    readonly property var shellCore: shell
    Connections {
        id: firstFrame
        target: win
        function onFrameSwapped() {
            firstFrame.enabled = false
            stages.firstFrame()
        }
    }
    Library {
        id: library
        onScanned: win.reloadAll()
        onImported: win.refreshGrid()
        // Turned files changed in place: no renumbering, so the viewer can stay open (the controller reads them again).
        onFilesChanged: shell.filesChanged()
    }
    onThumbHeightChanged: library.setThumbHeight(thumbHeight)
    // The tree has been shown once: from then on a new tree (a scan) leaves its folders as the user left them.
    property bool treeShown: false

    // The pauses (the thumbnails, the saved selection, the layout, the list after a query) are the controller's
    // (Shell.currentChanged, gridResized, queryEdited); their commands come back as Shell's signals above.
    onCurrentChanged: shell.currentChanged(current)
    onClosing: shell.flush()

    // The counts next to the filter choices, and "N of M". Not needed (and not computed) while nothing filters
    // and the bar is closed.
    function updateFacets() {
        if (filtersOpen || filterBar.active > 0)
            filterBar.countFacets()
    }

    // The filters changed: the list is shown after a short pause (Shell.queryEdited; the controller's pause opens the list), so typing a number
    // doesn't re-list at every key.
    onFiltersOpenChanged: updateFacets()

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
        onOpenRow: (row) => addPicker.enter(row)
        onChooseRow: (row) => addPicker.choose(row)
        onChooseHere: addPicker.chooseHere()
        onCancelled: addPicker.cancel()
        onVisibleChanged: win.modalShown(PanelKind.FolderPicker, visible)
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
        onOpenRow: (row) => cropPicker.enter(row)
        onChooseRow: (row) => cropPicker.choose(row)
        onChooseHere: cropPicker.chooseHere()
        onCancelled: cropPicker.cancel()
        onVisibleChanged: win.modalShown(PanelKind.FolderPicker, visible)
    }

    // Where the filter bar's "Save cropped copies" puts them: the folder is asked for, then they are made.
    function askSaveCrops() {
        cropPicker.pick(1)
    }

    function relayout() {
        const anchor = current >= 0 ? current : grid.topItem()
        grid.settling = true
        galleryModel.relayout(grid.gridWidth, rowHeight, gap)
        if (loadedIndex > -2) {
            gridLoaded(loadedIndex)
            return
        }
        if (anchor >= 0)
            grid.positionAt(galleryModel.rowOf(anchor), current >= 0 ? ListView.Contain : ListView.Beginning)
        grid.settling = false
    }

    // A list asked for is shown: keep the selection in view, else the file that was at the top (`index`), else
    // start at the top. The gate hears of the list first (Shell.listShown: the file the last session left is selected
    // before the start), then of its placement (Shell.listPositioned, which can finish the start).
    function gridLoaded(index: int) {
        // Laid out only once the grid has a width (relayout() comes back here). The grid has no rows meanwhile.
        if (galleryModel.count > 0 && galleryModel.rowOf(0) < 0) {
            loadedIndex = index
            return
        }
        loadedIndex = -2
        shell.listShown(galleryModel.indexOf(appSettings.lastFile), galleryModel.count)
        if (current >= 0)
            grid.positionAt(galleryModel.rowOf(current), ListView.Contain)
        else if (index >= 0)
            grid.positionAt(galleryModel.rowOf(index), ListView.Beginning)
        else
            grid.positionAtBeginning()
        grid.settling = false
        shell.listPositioned()
    }

    // The folder tree is in. At the start, the open folder is shown in it (its parents expanded); after a scan it
    // is only found again, the tree left as the user left it. The folder listed is the view's (the saved one, or the
    // applied view's): when the tree doesn't have it, the controller lists all folders.
    // The folder model works out the listed folder's row (-1: under a folder the user collapsed; -2: not in the tree).
    // The shell keeps the folder as the tree spells it and tells the controller whether the folder is known.
    function treeLoaded() {
        // The view is placed once, on the first tree; the model opens the folders above the listed one then (treeLoaded).
        const first = !treeShown
        treeShown = true
        const row = folderModel.treeLoaded()
        if (row >= 0) {
            tree.currentIndex = row
            if (first)
                tree.positionViewAtIndex(row, ListView.Contain)
        } else {
            tree.currentIndex = row === -1 ? -1 : 0
        }
        shell.treeLoaded(row !== -2)
    }

    // The subfolders were switched: for the empty folder that moves the listing between "All" (with them) and "Drive
    // roots" (without), so the tree's current row follows the listing (-1: under a folder the user collapsed).
    function followListedRow() {
        const row = folderModel.revealListing(false)
        tree.currentIndex = row === -1 ? -1 : Math.max(row, 0)
    }

    // The folder tree changed (a scan): read it again, and the open folder's files.
    function reloadAll() {
        folderModel.reload()
        refreshGrid()
    }

    // Re-read the open folder (new files, new sizes), keeping the selection or the top of the view in place. While
    // the viewer is open the new list waits for it to close (it could take away the pictures being rated, out of
    // "Unrated", and the viewer's place with them).
    function refreshGrid() {
        shell.reloadList()
    }

    // The list the controller asked for is loading (the shell announces it: listingChanged). The facets and the sort box
    // follow it; the session is saved by the controller.
    function showListing() {
        updateFacets()
        topBar.sortIndex = shell.sortIndex()
    }

    // A saved view's listing is the shell's (it has announced it); the filter bar shows its query, the tree its folder.
    function applyView(name: string) {
        if (!shell.applyView(name))
            return
        filterBar.reloadQuery()
        // The applied view's folder is shown in the tree (its parents expanded), when the tree has it.
        const row = folderModel.revealListing(true)
        tree.currentIndex = Math.max(row, 0)
        if (row >= 0)
            tree.positionViewAtIndex(row, ListView.Contain)
        grid.takeFocus()
    }

    // The inbox asked for a list of its own (a failed file's folder, the failed imports), which the shell has announced:
    // the inbox and the viewer close, the filter bar shows the query, the tree the folder.
    function showAsked() {
        inboxOpen = false
        if (viewerItem.visible)
            viewerItem.close()
        filterBar.reloadQuery()
        const row = folderModel.revealListing(true)
        tree.currentIndex = Math.max(row, 0)
        if (row >= 0)
            tree.positionViewAtIndex(row, ListView.Contain)
        grid.takeFocus()
    }

    // An inbox entry's Reveal: the file in its folder, selected once shown; a file the library no longer has toasts.
    function revealFile(fileId: real) {
        if (shell.reveal(fileId))
            showAsked()
    }

    // Show item i in the grid: its row, brought into view. The selection is the model's (the shell sets it).
    function scrollTo(i: int) {
        const row = galleryModel.rowOf(i)
        if (row >= 0)
            grid.positionAt(row, ListView.Contain)
    }

    // A selection the window makes itself (the test hooks, the viewer's close): set it, then show it.
    function select(i: int) {
        if (i < 0 || i >= galleryModel.count)
            return
        current = i
        scrollTo(i)
    }

    // To the panel made when first needed (Yandex's Loader, below): what it is asked to do comes as a signal once it
    // is made (a Loader not loading asynchronously makes its item at once).
    signal similarAsked(fileId: int)
    // The command line's `--find-similar`: the photo is the path it names, not a catalog file.
    signal similarCommandLineAsked()

    // The viewer's "find similar": search Yandex for the photo's file (by its id).
    function findSimilar(fileId: int) {
        yandexLoader.active = true
        similarAsked(fileId)
    }

    function findSimilarFromCommandLine() {
        yandexLoader.active = true
        similarCommandLineAsked()
    }

    // The save-view dialog (saveViewLoader) opens each time this rises: made at the first ask, then kept.
    property int saveViewRequests: 0
    function askSaveView() {
        saveViewLoader.active = true
        saveViewRequests++
    }

    // The key map (pg-client's keys, through the KeyMap facade) names the grid's action for a key; this carries it out.
    KeyMap {
        id: keymap
    }
    // The window's shortcuts (the inbox, the log view, dev mode): matched before any item's keys, decided by the controller.
    GlobalShortcuts {
        onActivated: (code) => shell.globalAction(code)
    }
    // The log's records, for the log view; read when it first opens (LogModel.start).
    LogModel {
        id: logModel
    }
    // The inbox: its entries, the bell's count and the toast. Followed from the start, so the opening's notices toast.
    Notices {
        id: inboxModel
        Component.onCompleted: start()
    }
    // The failed files of the inbox's ledger entry shown.
    FailedFiles {
        id: failedFilesModel
    }
    // What another process changed in the inbox shows when the window comes back.
    onActiveChanged: if (active) inboxModel.poke()

    // A modal panel opened or closed: the shell mirrors which are open, for the shortcuts' rule.
    function modalShown(kind: int, shown: bool) {
        if (shown)
            shell.panelOpened(kind)
        else
            shell.panelClosed(kind)
    }

    // A shortcut asked for a panel (Shell.panelOpenRequested, panelCloseRequested).
    function openPanel(kind: int) {
        if (kind === PanelKind.Log) {
            logModel.start()
            win.logOpen = true
        } else if (kind === PanelKind.Inbox) {
            win.inboxOpen = true
        }
    }
    function closePanel(kind: int) {
        if (kind === PanelKind.Log)
            win.logOpen = false
        else if (kind === PanelKind.Inbox)
            win.inboxOpen = false
    }

    // Back to where the keys were before a panel: the viewer when it is open, else the grid.
    function refocus() {
        if (viewerItem.visible)
            viewerItem.forceActiveFocus()
        else
            grid.takeFocus()
    }

    // The status bar's selection: the item's info and rating, or the key hints with none selected.
    readonly property string selectionInfo: win.current >= 0 ? galleryModel.selectionInfo : keymap.gridHint()

    // The test hooks (command-line options): their values are read in TestHooks, and what they do is run here.
    TestHooks {
        id: testHooks
        viewer: viewerItem
        rowHeight: win.rowHeight
        onSizeRequested: (h) => shell.sizeChosen(h)
        onGridKeyRequested: (key) => shell.keyPressed(key, 0)
        onSelectRequested: (i) => win.select(i)
        onBorderMarkToggled: (i) => shell.toggleBorderMark(i)
        onBordersRequested: shell.setShowBorders(true)
        onViewsMenuRequested: topBar.openViewsMenu()
        onLibraryPanelRequested: {
            win.libraryOpen = true
        }
        onSaveViewDialogRequested: win.askSaveView()
        onSimilarRequested: win.findSimilarFromCommandLine()
        onAddFolderRequested: addPicker.chooseCommandLine(0)
        onSaveCropsRequested: cropPicker.chooseCommandLine(1)
        onViewerShowRequested: (index) => viewerItem.show(index)
        onViewerPlayRequested: viewerItem.togglePlay()
        onViewerSetZoomRequested: (z, px, py) => viewerItem.setZoom(z, px, py)
        onViewerTurnRequested: (turns) => viewerItem.turn(turns)
        onViewerRateRequested: (rating) => viewerItem.rate(rating)
        onLogPanelRequested: shell.globalAction(GlobalShortcuts.Log)
        onInboxPanelRequested: shell.globalAction(GlobalShortcuts.Inbox)
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
            onFiltersToggled: shell.toggleFilters()
            onRecursiveToggled: (on) => {
                shell.setRecursive(on)
                win.followListedRow()
            }
            onSortActivated: (i) => {
                shell.sortBy(i)
                grid.takeFocus()
            }
            onDescendingToggled: {
                shell.toggleDescending()
                grid.takeFocus()
            }
            onViewChosen: (name) => win.applyView(name)
            onViewDeleteRequested: (name) => shell.deleteView(name)
            onSaveViewRequested: win.askSaveView()
            onLibraryRequested: {
                win.libraryOpen = true
            }
            onSizeChosen: (h) => shell.sizeChosen(h)
            onViewsMenuOpenChanged: win.modalShown(PanelKind.ViewsMenu, viewsMenuOpen)
        }

        StatusBar {
            id: bottomBar
            selection: win.selectionInfo
            writeStatus: library.writeStatus
            writeFailed: library.writeFailed
            toast: inboxModel.toast
            toastLevel: inboxModel.toastLevel
            unread: inboxModel.unread
            status: library.status
            failed: library.failed
            busy: library.busy
            paused: library.paused
            progress: library.progress
            devMode: shell.devMode
            onLogRequested: shell.globalAction(GlobalShortcuts.Log)
            onToastExpired: inboxModel.dismissToast()
            onInboxRequested: shell.globalAction(GlobalShortcuts.Inbox)
            onFailedRequested: {
                win.inboxExpandRow = inboxModel.rowOfPass(library.failedPass)
                if (!win.inboxOpen)
                    shell.globalAction(GlobalShortcuts.Inbox)
            }
            onPauseToggled: library.togglePause()
        }

        FilterBar {
            id: filterBar
            onEdited: shell.queryEdited()
            croppable: galleryModel.croppable
            onSaveCropsRequested: win.askSaveCrops()
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
                    onToggleRequested: (row) => folderModel.toggle(row)
                    onFolderOpened: (row) => {
                        shell.openFolderRow(row)
                        grid.takeFocus()
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
                showBorders: win.showBorders
                noFolders: library.roots.length === 0
                filtering: filterBar.active > 0
                importing: library.busy
                onKeyPressed: (e) => e.accepted = shell.keyPressed(e.key, e.modifiers)
                onCellClicked: (i) => {
                    win.current = i
                }
                onCellDoubleClicked: (i) => viewerItem.show(i)
                onSizeStepped: (factor) => shell.sizeStepped(factor)
                onGridResized: (width) => shell.gridResized(width)
            }
        }

        Viewer {
            id: viewerItem
            anchors.fill: parent
            gallery: galleryModel
            fullUrl: galleryModel.viewedFullUrl
            index: galleryModel.viewing
            count: galleryModel.count
            showBorders: win.showBorders
            settings: appSettings
            onMoved: (i) => galleryModel.viewing = i
            onStepRequested: (d) => stepTo(shell.viewerStep(index, d))
            // Opened or closed: the core knows (its keys are the viewer's while it is open); closed, the keys go back to
            // the grid.
            onVisibleChanged: {
                shell.setViewerOpen(visible)
                if (!visible)
                    grid.takeFocus()
            }
            onRated: (i, r) => shell.rate(i, r)
            onTurned: (i, d) => shell.turn(i, d)
            onBordersToggled: shell.toggleBorders()
            onBorderMarkToggled: (i, m) => shell.setBorderMark(i, m)
            // The file the viewer is on is the one a session remembers (Shell.sessionDue).
            onIndexChanged: if (index >= 0) shell.currentChanged(index)
            onFindSimilar: (fileId) => win.findSimilar(fileId)
            onClosed: (i) => win.select(i)
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
                    hasRoots: library.roots.length > 0
                    writeBack: appSettings.writeBack
                    onDismissed: {
                        win.libraryOpen = false
                    }
                    onHidden: grid.takeFocus()
                    onVisibleChanged: win.modalShown(PanelKind.Library, visible)
                    onAddRequested: addPicker.pick(0)
                    onRemoveRequested: (row) => library.removeFolder(row)
                    onRescanRequested: library.rescan()
                    onWriteBackToggled: (on) => {
                        appSettings.writeBack = on
                    }
                }
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
                    gap: win.gap
                    previewWhenFound: testHooks.yandexPreview
                    // Back to the viewer it was opened from, so its arrow keys work again; the grid only if the viewer
                    // is gone.
                    onClosed: win.refocus()
                    onVisibleChanged: win.modalShown(PanelKind.Yandex, visible)
                    onPreviewOpenChanged: win.modalShown(PanelKind.YandexPreview, previewOpen)
                    Connections {
                        target: win
                        function onSimilarAsked(fileId: int) { yandexPanel.show(fileId) }
                        function onSimilarCommandLineAsked() { yandexPanel.showCommandLine() }
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
                    onSaved: (name) => shell.saveView(name)
                    onClosed: grid.takeFocus()
                    onVisibleChanged: win.modalShown(PanelKind.SaveViewDialog, visible)
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
                    onDismissed: win.logOpen = false
                    onHidden: win.refocus()
                    onVisibleChanged: win.modalShown(PanelKind.Log, visible)
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
                    failedFiles: failedFilesModel
                    failedTotal: failedFilesModel.total
                    expandRow: win.inboxExpandRow
                    onCopyRequested: (row) => inboxModel.copy(row)
                    onDismissRequested: (row) => inboxModel.dismiss(row)
                    onRetryRequested: (row) => inboxModel.retry(row)
                    onFilesRequested: (pass) => failedFilesModel.show(pass)
                    onRevealRequested: (fileId) => win.revealFile(fileId)
                    onShowFilesRequested: {
                        shell.showFailedImports()
                        win.showAsked()
                    }
                    onClearRequested: inboxModel.dismissAll()
                    onDismissed: win.inboxOpen = false
                    onHidden: win.refocus()
                    onVisibleChanged: win.modalShown(PanelKind.Inbox, visible)
                }
            }
        }
    }

    // The window is made (it shows right after): ask for where the user was. Nothing here waits for the catalog;
    // the tree and the files arrive later, in either order (treeLoaded, gridLoaded), and the second of them
    // finishes the start (started).
    Component.onCompleted: {
        // The grid model takes the controller's model commands from here on.
        galleryModel.registerModel()
        // The folder, subfolders, sort and query are the listing's and the filter bar's as they start (the controller's
        // start: the session's, then the test hooks' --sort, --query and --apply-view).
        // The scale, the size and the borders as the user left them: the controller holds them from here on (the
        // thumbnails are sized from the scale before the library starts, in startHooks).
        shell.setScale(Screen.devicePixelRatio)
        shell.sizeChosen(appSettings.rowHeight)
        shell.setShowBorders(appSettings.showBorders)
        // Dev mode as the user left it, or on for this run (--dev=1, PG_DEV=1): the shell reads both (not saved here).
        shell.seedDevMode()
        if (testHooks.hasFilters) shell.setFiltersOpen(true)
        if (testHooks.hasBorders) shell.setShowBorders(true)
        topBar.sortIndex = shell.sortIndex()
        relayout()
        folderModel.reload()
        // As it was left: whether the folder still exists is known once the tree is in (treeLoaded).
        shell.reloadList()
    }

    // The gate let the start through (Shell.started): run the test hooks, then start the background job (after the
    // first reads, which it would compete with for the disk).
    function startHooks() {
        stages.ready()
        if (testHooks.hasApplyView) shell.rememberListing()
        testHooks.run()
        library.start(thumbHeight)
        testHooks.runAfterLibraryStart()
    }
}
