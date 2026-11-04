// Main window: the composition of the page (top bar, folder tree and grid, status bar, filters, viewer) and the panels made
// when first needed. It owns the Rust objects (the gallery, the folders, the view, the library) and makes their calls; the
// components report what the user asks for through signals. The start choreography (started, gridLoaded, treeLoaded) is
// here; the test hooks are in TestHooks.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Dialogs
import QtQuick.Window
import pg.gallery

ApplicationWindow {
    id: win
    width: 1600
    height: 1000
    // Shown after the load, once it can be handed the app's D3D11 device (src/gpu_device.rs).
    visible: false
    title: "Photo Gallery" + (view.folder.length ? " — " + view.folder : "")
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
    property real rowHeight: appSettings.rowHeight
    property int thumbHeight: Math.round(rowHeight * Screen.devicePixelRatio)
    // The selected item (-1: none): the gallery's, so it stays with its file when the list is refreshed.
    property alias current: galleryModel.current
    property bool filtersOpen: false
    // B: the grid and the viewer show what cropping the borders would cut. Remembered across runs (the test hook
    // sets it for its run only).
    property bool showBorders: appSettings.showBorders
    function toggleBorders() {
        showBorders = !showBorders
        appSettings.showBorders = showBorders
    }
    // The library panel is shown; made the first time (libraryLoader).
    property bool libraryOpen: false
    onLibraryOpenChanged: if (libraryOpen) libraryLoader.active = true
    // The first list is shown and the test hooks have run (the --screenshot hook waits for it).
    property bool ready: false
    // A list that arrived before the grid had a width: positioned once it is laid out (-2: none).
    property int loadedIndex: -2
    readonly property real gap: 4
    // query::SortKey names, in the order of the sort box (TopBar).
    readonly property list<string> sortKeys: ["name", "size", "modified", "taken", "megapixels", "duration",
        "borders"]

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
            galleryModel.anchor = win.current >= 0 ? -1 : grid.topItem()
            grid.settling = true
        }
        onLoaded: (index) => win.gridLoaded(index)
    }
    // Where the grid looks, with or without subfolders, in what order; with the filter bar's query, a view.
    // Remembered across runs, and in saved views (src/view.rs).
    View { id: view }
    // The start's stages, for measuring it (src/trace.rs).
    Stages { id: stages }
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
        // Turned files changed in place: no renumbering, so the viewer can stay open.
        onFilesChanged: galleryModel.refreshTurned()
    }
    onThumbHeightChanged: library.setThumbHeight(thumbHeight)
    // The tree has been shown once: from then on a new tree (a scan) leaves its folders as the user left them.
    property bool treeShown: false

    Timer {
        id: thumbTimer
        interval: 300
        onTriggered: {
            win.thumbHeight = Math.round(win.rowHeight * Screen.devicePixelRatio)
            appSettings.rowHeight = win.rowHeight
        }
    }
    // The selected file, saved once the selection rests.
    Timer {
        id: sessionTimer
        interval: 1000
        // Not before the start selected it: a key pressed on the empty grid would save no file.
        onTriggered: if (win.ready)
            appSettings.lastFile = galleryModel.fileId(viewerItem.visible ? viewerItem.index : win.current)
    }
    onCurrentChanged: sessionTimer.restart()
    Timer {
        id: layoutTimer
        interval: 50
        onTriggered: win.relayout()
    }
    onRowHeightChanged: {
        relayout()
        thumbTimer.restart()
    }

    function setSize(h: real) { rowHeight = Math.max(60, Math.min(800, Math.round(h))) }

    // The counts next to the filter choices, and "N of M". Not needed (and not computed) while nothing filters
    // and the bar is closed.
    function updateFacets() {
        if (filtersOpen || filterBar.active > 0)
            filterBar.countFacets(view.folder, view.recursive)
    }

    // The filters changed: apply them after a short pause, so typing a number doesn't re-list at every key.
    Timer {
        id: applyTimer
        interval: 300
        onTriggered: win.openPath(view.folder)
    }
    onFiltersOpenChanged: updateFacets()

    // Where the filter bar's "Save cropped copies" puts them: made when asked for, and opened as it is made.
    Loader {
        id: cropDialog
        active: false
        sourceComponent: Component {
            FolderDialog {
                title: "Save cropped copies into"
                onAccepted: galleryModel.saveCroppedCopies(selectedFolder)
                Component.onCompleted: open()
            }
        }
    }
    function askSaveCrops() {
        cropDialog.active = false
        cropDialog.active = true
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
    // start at the top. Until the start is over, the selection is the file the user was on, if the list has it.
    // The first list with the tree in finishes the start.
    function gridLoaded(index: int) {
        // Laid out only once the grid has a width (relayout() comes back here). The grid has no rows meanwhile.
        if (galleryModel.count > 0 && galleryModel.rowOf(0) < 0) {
            loadedIndex = index
            return
        }
        loadedIndex = -2
        if (!ready) {
            const last = galleryModel.indexOf(appSettings.lastFile)
            if (last >= 0)
                current = last
        }
        if (current >= 0)
            grid.positionAt(galleryModel.rowOf(current), ListView.Contain)
        else if (index >= 0)
            grid.positionAt(galleryModel.rowOf(index), ListView.Beginning)
        else
            grid.positionAtBeginning()
        grid.settling = false
        // The tree says whether the folder is still there: without it this may be a list of a folder gone.
        if (!ready && treeShown)
            started()
    }

    // The folder tree is in. At the start, the open folder is shown in it (its parents expanded); after a scan it
    // is only found again, the tree left as the user left it. A folder gone opens all folders.
    function treeLoaded() {
        const first = !treeShown
        treeShown = true
        // At the start, the saved folder if the tree still has it (folderModel.restore; "" when it is gone, or for all
        // folders). The list was asked for the saved one: when the answer differs, the list is asked again for it,
        // and the tree shows it; the list's loading finishes the start.
        if (first) {
            const restored = folderModel.restore()
            if (restored !== view.folder) {
                const at = folderModel.reveal(restored)
                tree.currentIndex = Math.max(at, 0)
                if (at >= 0)
                    tree.positionViewAtIndex(at, ListView.Contain)
                openPath(restored)
                return
            }
        }
        const row = first ? folderModel.reveal(view.folder) : folderModel.rowOf(view.folder)
        if (row >= 0) {
            tree.currentIndex = row
            if (first)
                tree.positionViewAtIndex(row, ListView.Contain)
            // Spelled as the tree spells it (paths match in any case): the list already shows it.
            const spelled = folderModel.mergedPath(row)
            if (spelled !== view.folder) {
                view.folder = spelled
                view.saveSession(filterBar.query)
            }
        } else if (folderModel.contains(view.folder)) {
            // Under a folder the user collapsed.
            tree.currentIndex = -1
        } else {
            tree.currentIndex = 0
            if (view.folder !== "") {
                openPath("")
                return
            }
        }
        // The list came first (and isn't waiting for the grid's width): the start finishes now.
        if (!ready && !galleryModel.loading && loadedIndex === -2)
            started()
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
        galleryModel.openFolder(view.folder, view.recursive, filterBar.query, view.sortJson(), true)
        updateFacets()
    }

    function openFolder(row: int) {
        openPath(folderModel.mergedPath(row))
    }

    // Show a merged folder (`""` = all) with the current subfolders, filters and sort, and remember it.
    function openPath(path: string) {
        view.folder = path
        galleryModel.openFolder(path, view.recursive, filterBar.query, view.sortJson(), false)
        updateFacets()
        topBar.sortIndex = sortKeys.indexOf(view.sortKey)
        view.saveSession(filterBar.query)
    }

    // Open a folder by path, showing it in the tree (its parents expanded). Before the tree is in, it is shown
    // when it arrives (treeLoaded).
    function goTo(path: string) {
        const row = folderModel.reveal(path)
        tree.currentIndex = Math.max(row, 0)
        if (row >= 0)
            tree.positionViewAtIndex(row, ListView.Contain)
        openPath(row >= 0 ? folderModel.mergedPath(row) : folderModel.ready ? "" : path)
    }

    function applyView(name: string) {
        const query = view.applyView(name)
        if (query === "")
            return
        filterBar.query = query
        goTo(view.folder)
        grid.takeFocus()
    }

    function select(i: int) {
        if (i < 0 || i >= galleryModel.count)
            return
        current = i
        grid.positionAt(galleryModel.rowOf(i), ListView.Contain)
    }

    // To the panel made when first needed (Yandex's Loader, below): what it is asked to do comes as a signal once it
    // is made (a Loader not loading asynchronously makes its item at once).
    signal similarAsked(path: string)

    // The viewer's "find similar": search Yandex for the photo's file.
    function findSimilar(path: string) {
        yandexLoader.active = true
        similarAsked(path)
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

    // The grid's actions, as the codes keymap.gridAction returns (the Rust side lists them in the same order).
    enum GridAction { Grow, Shrink, Left, Right, Up, Down, First, Last, Open, Rate, TurnForward, TurnBack, Filters, Borders, MarkBorder }

    function gridKey(e: KeyEvent) {
        const action = keymap.gridAction(e.key, e.modifiers, current >= 0)
        if (action < 0)
            return
        const arg = keymap.gridArg(e.key, e.modifiers, current >= 0)
        const c = current < 0 ? 0 : current
        switch (action) {
        case Main.Main.GridAction.Grow: setSize(rowHeight * 1.15); break
        case Main.Main.GridAction.Shrink: setSize(rowHeight / 1.15); break
        case Main.Main.GridAction.Left: select(galleryModel.neighbor(c, -1, 0)); break
        case Main.Main.GridAction.Right: select(galleryModel.neighbor(c, 1, 0)); break
        case Main.Main.GridAction.Up: select(galleryModel.neighbor(c, 0, -1)); break
        case Main.Main.GridAction.Down: select(galleryModel.neighbor(c, 0, 1)); break
        case Main.Main.GridAction.First: select(0); break
        case Main.Main.GridAction.Last: select(galleryModel.count - 1); break
        case Main.Main.GridAction.Open: viewerItem.show(current); break
        case Main.Main.GridAction.Rate: galleryModel.setRating(current, arg); break
        case Main.Main.GridAction.TurnForward: galleryModel.rotate(current, 1); break
        case Main.Main.GridAction.TurnBack: galleryModel.rotate(current, -1); break
        case Main.Main.GridAction.Filters: filtersOpen = !filtersOpen; break
        case Main.Main.GridAction.Borders: toggleBorders(); break
        case Main.Main.GridAction.MarkBorder: galleryModel.setBorderMarked(current, !galleryModel.borderMarked(current)); break
        }
        e.accepted = true
    }

    // The status bar's selection: the item's info and rating, or the key hints with none selected.
    readonly property string selectionInfo: galleryModel.revision >= 0 && win.current >= 0
        ? galleryModel.info(win.current) + "   "
          + (galleryModel.rating(win.current) > 0
              ? Ratings.symbol(galleryModel.rating(win.current)) : "")
        : keymap.gridHint()

    // The test hooks (command-line options): their values are read in TestHooks, and what they do is run here.
    TestHooks {
        id: testHooks
        viewer: viewerItem
        rowHeight: win.rowHeight
        onSizeRequested: (h) => win.setSize(h)
        onSelectRequested: (i) => win.select(i)
        onBorderMarkToggled: (i) => {
            galleryModel.setBorderMarked(i, !galleryModel.borderMarked(i))
        }
        onBordersRequested: {
            win.showBorders = true
        }
        onViewsMenuRequested: topBar.openViewsMenu()
        onLibraryPanelRequested: {
            win.libraryOpen = true
        }
        onSaveViewDialogRequested: win.askSaveView()
        onSimilarRequested: (path) => win.findSimilar(path)
        onAddFolderRequested: (folder) => library.addFolder(folder)
        onSaveCropsRequested: (folder) => galleryModel.saveCroppedCopies(folder)
        onViewerShowRequested: (index) => viewerItem.show(index)
        onViewerPlayRequested: viewerItem.togglePlay()
        onViewerSetZoomRequested: (z, px, py) => viewerItem.setZoom(z, px, py)
        onViewerTurnRequested: (turns) => viewerItem.turn(turns)
        onViewerRateRequested: (rating) => viewerItem.rate(rating)
    }

    Item {
        id: page
        anchors.fill: parent

        TopBar {
            id: topBar
            folder: view.folder
            recursive: view.recursive
            descending: view.descending
            savedViews: view.savedViews
            hasSavedViews: view.savedViews.length > 0
            loading: galleryModel.loading
            fileCount: galleryModel.count
            filterActive: filterBar.active
            filterTotal: filterBar.total
            filtersOpen: win.filtersOpen
            rowHeight: win.rowHeight
            onFiltersToggled: {
                win.filtersOpen = !win.filtersOpen
            }
            onRecursiveToggled: (on) => {
                view.recursive = on
                win.openPath(view.folder)
            }
            onSortActivated: (i) => {
                // Name reads A→Z; the others start with the largest, newest or longest.
                view.sortKey = win.sortKeys[i]
                view.descending = i !== 0
                win.openPath(view.folder)
                grid.takeFocus()
            }
            onDescendingToggled: {
                view.descending = !view.descending
                win.openPath(view.folder)
                grid.takeFocus()
            }
            onViewChosen: (name) => win.applyView(name)
            onViewDeleteRequested: (name) => view.deleteView(name)
            onSaveViewRequested: win.askSaveView()
            onLibraryRequested: {
                win.libraryOpen = true
            }
            onSizeChosen: (h) => win.setSize(h)
        }

        StatusBar {
            id: bottomBar
            selection: win.selectionInfo
            writeStatus: library.writeStatus
            writeFailed: library.writeFailed
            message: library.message
            messageWarning: library.messageWarning
            status: library.status
            busy: library.busy
            paused: library.paused
            progress: library.progress
            onMessageDismissed: {
                library.message = ""
            }
            onPauseToggled: library.togglePause()
        }

        FilterBar {
            id: filterBar
            onEdited: applyTimer.restart()
            croppable: galleryModel.revision >= 0 && galleryModel.count >= 0 ? galleryModel.borderedCount() : 0
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

            FolderTree {
                id: tree
                SplitView.preferredWidth: 320
                SplitView.minimumWidth: 160
                folders: folderModel
                onToggleRequested: (row) => folderModel.toggle(row)
                onFolderOpened: (row) => {
                    win.openFolder(row)
                    grid.takeFocus()
                }
            }

            GalleryGrid {
                id: grid
                SplitView.fillWidth: true
                gallery: galleryModel
                gap: win.gap
                current: win.current
                thumbHeight: win.thumbHeight
                showBorders: win.showBorders
                noFolders: library.roots.length === 0
                filtering: filterBar.active > 0
                importing: library.busy
                onKeyPressed: (e) => win.gridKey(e)
                onCellClicked: (i) => {
                    win.current = i
                }
                onCellDoubleClicked: (i) => viewerItem.show(i)
                onSizeStepped: (factor) => win.setSize(win.rowHeight * factor)
                onGridResized: layoutTimer.restart()
            }
        }

        Viewer {
            id: viewerItem
            anchors.fill: parent
            fullUrl: galleryModel.fullUrl(viewerItem.key, "full")
            index: galleryModel.viewing
            count: galleryModel.count
            // The shown item, read again when the gallery changes it in place (a rating, a turn). Out of range (-1:
            // closed), the gallery gives empty values.
            key: galleryModel.revision >= 0 ? galleryModel.imageKey(viewerItem.index) : ""
            neighbourKeys: galleryModel.revision >= 0 && viewerItem.visible ? galleryModel.keysAround(viewerItem.index, 4) : []
            turns: galleryModel.revision >= 0 ? galleryModel.pendingTurns(viewerItem.index) : 0
            rating: galleryModel.revision >= 0 ? galleryModel.rating(viewerItem.index) : 0
            fw: galleryModel.revision >= 0 ? galleryModel.fullWidth(viewerItem.index) : 0
            fh: galleryModel.revision >= 0 ? galleryModel.fullHeight(viewerItem.index) : 0
            kind: galleryModel.revision >= 0 ? galleryModel.mediaType(viewerItem.index) : ""
            playerTurns: galleryModel.revision >= 0 ? galleryModel.playerTurns(viewerItem.index) : 0
            fileUrl: galleryModel.revision >= 0 ? galleryModel.fileUrl(viewerItem.index) : Qt.url("")
            filePath: galleryModel.revision >= 0 ? galleryModel.filePath(viewerItem.index) : ""
            info: galleryModel.revision >= 0 ? galleryModel.info(viewerItem.index) : ""
            borderBox: galleryModel.revision >= 0 ? galleryModel.borderBox(viewerItem.index) : Qt.rect(0, 0, 0, 0)
            rejectedTop: galleryModel.revision >= 0 && viewerItem.visible
                ? galleryModel.borderRejected(viewerItem.index, "top") : -1
            rejectedBottom: galleryModel.revision >= 0 && viewerItem.visible
                ? galleryModel.borderRejected(viewerItem.index, "bottom") : -1
            rejectedLeft: galleryModel.revision >= 0 && viewerItem.visible
                ? galleryModel.borderRejected(viewerItem.index, "left") : -1
            rejectedRight: galleryModel.revision >= 0 && viewerItem.visible
                ? galleryModel.borderRejected(viewerItem.index, "right") : -1
            borderMarked: galleryModel.revision >= 0 && galleryModel.borderMarked(viewerItem.index)
            showBorders: win.showBorders
            settings: appSettings
            onMoved: (i) => galleryModel.viewing = i
            // Closed: the keys go back to the grid.
            onVisibleChanged: if (!visible) grid.takeFocus()
            onRated: (i, r) => galleryModel.setRating(i, r)
            onTurned: (i, d) => galleryModel.rotate(i, d)
            onBordersToggled: win.toggleBorders()
            onBorderMarkToggled: (i, m) => galleryModel.setBorderMarked(i, m)
            onIndexChanged: sessionTimer.restart()
            onFindSimilar: (path) => win.findSimilar(path)
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
                    onFolderPicked: (folder) => library.addFolder(folder)
                    onRemoveRequested: (path) => library.removeFolder(path)
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
                    previewWhenFound: testHooks.yandexPreview
                    // Back to the viewer it was opened from, so its arrow keys work again; the grid only if the viewer
                    // is gone.
                    onClosed: viewerItem.visible ? viewerItem.forceActiveFocus() : grid.takeFocus()
                    Connections {
                        target: win
                        function onSimilarAsked(path: string) { yandexPanel.show(path) }
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
                    folder: view.folder
                    recursive: view.recursive
                    descending: view.descending
                    filters: filterBar.description
                    sortName: topBar.sortText
                    onSaved: (name) => view.saveView(name, filterBar.query)
                    onClosed: grid.takeFocus()
                }
            }
        }
    }

    // The window is made (it shows right after): ask for where the user was. Nothing here waits for the catalog;
    // the tree and the files arrive later, in either order (treeLoaded, gridLoaded), and the second of them
    // finishes the start (started).
    Component.onCompleted: {
        // The folder, subfolders and sort are the view's (it read them at its start), the filters the bar's.
        filterBar.query = testHooks.hasQuery ? testHooks.queryJson : view.sessionQuery()
        if (testHooks.hasSort) view.setSortJson(testHooks.sortJson)
        if (testHooks.hasApplyView) {
            const query = view.applyView(testHooks.applyViewName)
            if (query !== "")
                filterBar.query = query
        }
        if (testHooks.hasFilters) filtersOpen = true
        if (testHooks.hasBorders) showBorders = true
        topBar.sortIndex = sortKeys.indexOf(view.sortKey)
        relayout()
        folderModel.reload()
        // As it was left: whether the folder still exists is known once the tree is in (treeLoaded).
        galleryModel.openFolder(view.folder, view.recursive, filterBar.query, view.sortJson(), true)
        updateFacets()
    }

    // The first list is shown (with the file the user was on selected, gridLoaded): run the test hooks, then start
    // the background job (after the first reads, which it would compete with for the disk).
    function started() {
        ready = true
        stages.ready()
        if (testHooks.hasApplyView) view.saveSession(filterBar.query)
        testHooks.run()
        library.start(thumbHeight)
        testHooks.runAfterLibraryStart()
    }
}
