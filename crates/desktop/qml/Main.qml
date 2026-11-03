// Main window: merged folder tree on the left, justified thumbnail grid on the right, viewer on top. The
// library (folders the user added) is imported by a background job; its progress is in the bottom bar.
pragma ComponentBehavior: Bound

import QtQml.Models
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Dialogs
import QtQuick.Layouts
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
    // query::SortKey names, and the sort box's names for them.
    readonly property list<string> sortKeys: ["name", "size", "modified", "taken", "megapixels", "duration",
        "borders"]
    readonly property list<string> sortNames: ["By name", "By file size", "By date modified", "By date taken",
        "By megapixels", "By duration", "By border depth"]
    // The test hook's rating keys (--viewer-keys): "r3" rates 3.
    readonly property list<string> ratingKeys: ["r0", "r1", "r2", "r3", "r4", "r5"]

    // Both read the catalog off the GUI thread: what they were asked for arrives later (onLoaded).
    FolderModel {
        id: folders
        onLoaded: win.treeLoaded()
    }
    GalleryModel {
        id: galleryModel
        // Without a selection, a refreshed list keeps the top row's file in view. The rows change (all of them for
        // another list), then gridLoaded moves the view to where it belongs.
        onAboutToLoad: {
            galleryModel.anchor = win.current >= 0 ? -1 : win.topItem()
            grid.settling = true
        }
        onLoaded: (index) => win.gridLoaded(index)
    }
    // Where the grid looks, with or without subfolders, in what order; with the filter bar's query, a view.
    // Remembered across runs, and in saved views (src/view.rs).
    View { id: view }
    CommandLine { id: commandLine }
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
    Timer {
        interval: 500
        running: true
        repeat: true
        onTriggered: library.poll()
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
            appSettings.lastFile = galleryModel.fileId(viewer.visible ? viewer.index : win.current)
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

    // The first item of the topmost visible row, to keep the view anchored across relayouts.
    function topItem(): int {
        const row = grid.indexAt(10, grid.contentY + 1)
        const del = row >= 0 ? grid.itemAtIndex(row) as GridRow : null
        return del ? del.firstItem : -1
    }

    function relayout() {
        const anchor = current >= 0 ? current : topItem()
        grid.settling = true
        galleryModel.relayout(grid.width, rowHeight, gap)
        if (loadedIndex > -2) {
            gridLoaded(loadedIndex)
            return
        }
        if (anchor >= 0)
            grid.positionViewAtIndex(galleryModel.rowOf(anchor), current >= 0 ? ListView.Contain : ListView.Beginning)
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
            grid.positionViewAtIndex(galleryModel.rowOf(current), ListView.Contain)
        else if (index >= 0)
            grid.positionViewAtIndex(galleryModel.rowOf(index), ListView.Beginning)
        else
            grid.positionViewAtBeginning()
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
        const row = first ? folders.reveal(view.folder) : folders.rowOf(view.folder)
        if (row >= 0) {
            tree.currentIndex = row
            if (first)
                tree.positionViewAtIndex(row, ListView.Contain)
            // Spelled as the tree spells it (paths match in any case): the list already shows it.
            const spelled = folders.mergedPath(row)
            if (spelled !== view.folder) {
                view.folder = spelled
                view.saveSession(filterBar.query)
            }
        } else if (folders.contains(view.folder)) {
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
        folders.reload()
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
        openPath(folders.mergedPath(row))
    }

    // Show a merged folder (`""` = all) with the current subfolders, filters and sort, and remember it.
    function openPath(path: string) {
        view.folder = path
        galleryModel.openFolder(path, view.recursive, filterBar.query, view.sortJson(), false)
        updateFacets()
        sortBox.currentIndex = sortKeys.indexOf(view.sortKey)
        view.saveSession(filterBar.query)
    }

    // Open a folder by path, showing it in the tree (its parents expanded). Before the tree is in, it is shown
    // when it arrives (treeLoaded).
    function goTo(path: string) {
        const row = folders.reveal(path)
        tree.currentIndex = Math.max(row, 0)
        if (row >= 0)
            tree.positionViewAtIndex(row, ListView.Contain)
        openPath(row >= 0 ? folders.mergedPath(row) : folders.ready ? "" : path)
    }

    function applyView(name: string) {
        const query = view.applyView(name)
        if (query === "")
            return
        filterBar.query = query
        goTo(view.folder)
        grid.forceActiveFocus()
    }

    function select(i: int) {
        if (i < 0 || i >= galleryModel.count)
            return
        current = i
        grid.positionViewAtIndex(galleryModel.rowOf(i), ListView.Contain)
    }

    // To the panel and dialog made when first needed (their Loaders, below): what they are asked to do comes as a
    // signal once they are made (a Loader not loading asynchronously makes its item at once).
    signal similarAsked(path: string)
    signal saveViewAsked()

    // The test hook --yandex-preview: the result the Yandex panel shows at full size once its search has results. Set
    // before the panel is made (findSimilar), which takes it as its own property.
    property int yandexPreviewHook: -1

    // The viewer's "find similar": search Yandex for the photo's file.
    function findSimilar(path: string) {
        yandexLoader.active = true
        similarAsked(path)
    }

    function askSaveView() {
        saveViewLoader.active = true
        saveViewAsked()
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
        case Main.Main.GridAction.Open: viewer.show(current); break
        case Main.Main.GridAction.Rate: galleryModel.setRating(current, arg); break
        case Main.Main.GridAction.TurnForward: galleryModel.rotate(current, 1); break
        case Main.Main.GridAction.TurnBack: galleryModel.rotate(current, -1); break
        case Main.Main.GridAction.Filters: filtersOpen = !filtersOpen; break
        case Main.Main.GridAction.Borders: toggleBorders(); break
        case Main.Main.GridAction.MarkBorder: galleryModel.setBorderMarked(current, !galleryModel.borderMarked(current)); break
        }
        e.accepted = true
    }

    Item {
        id: page
        anchors.fill: parent

        ToolBar {
            id: topBar
            anchors.left: parent.left
            anchors.right: parent.right
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                spacing: 12
                Label {
                    text: view.folder.length ? view.folder : "All folders"
                    font.bold: true
                    elide: Text.ElideMiddle
                    Layout.maximumWidth: 600
                }
                Label {
                    color: "#999999"
                    text: galleryModel.loading && galleryModel.count === 0 ? "Loading…"
                        : Qt.locale().toString(galleryModel.count)
                          + (filterBar.active > 0 ? " of " + Qt.locale().toString(filterBar.total) : "")
                          + " files"
                }
                BusyIndicator {
                    running: galleryModel.loading
                    visible: running
                    Layout.preferredWidth: 20
                    Layout.preferredHeight: 20
                }
                CheckBox {
                    text: "Include subfolders"
                    checked: view.recursive
                    onToggled: {
                        view.recursive = checked
                        win.openPath(view.folder)
                    }
                }
                Button {
                    text: "Filters" + (filterBar.active > 0 ? " (" + filterBar.active + ")" : "")
                        + (win.filtersOpen ? " ▴" : " ▾")
                    highlighted: filterBar.active > 0
                    onClicked: win.filtersOpen = !win.filtersOpen
                    ToolTip.visible: hovered
                    ToolTip.text: "Filter by type, extension, rating, size, date, duplicates… (F)"
                }
                ComboBox {
                    id: sortBox
                    model: win.sortNames
                    onActivated: (i) => {
                        // Name reads A→Z; the others start with the largest, newest or longest.
                        view.sortKey = win.sortKeys[i]
                        view.descending = i !== 0
                        win.openPath(view.folder)
                        grid.forceActiveFocus()
                    }
                    Layout.preferredWidth: 160
                }
                Button {
                    text: view.descending ? "↓" : "↑"
                    onClicked: {
                        view.descending = !view.descending
                        win.openPath(view.folder)
                        grid.forceActiveFocus()
                    }
                    ToolTip.visible: hovered
                    ToolTip.text: view.descending
                        ? "Descending (click for ascending)"
                        : "Ascending (click for descending)"
                    Layout.preferredWidth: 32
                }
                // Saved views (ImageRater's bookmarks): a folder with its subfolders, filters and sort, by name.
                Button {
                    id: viewsButton
                    text: "Views ▾"
                    onClicked: viewsMenu.popup(viewsButton, 0, viewsButton.height)
                    Menu {
                        id: viewsMenu
                        Instantiator {
                            model: view.savedViews
                            delegate: MenuItem {
                                required property string modelData
                                text: modelData
                                onTriggered: win.applyView(modelData)
                            }
                            onObjectAdded: (i, o) => viewsMenu.insertItem(i, o as MenuItem)
                            onObjectRemoved: (i, o) => viewsMenu.removeItem(o as MenuItem)
                        }
                        MenuSeparator {}
                        MenuItem {
                            text: "Save current view…"
                            onTriggered: win.askSaveView()
                        }
                        Menu {
                            id: deleteMenu
                            title: "Delete view"
                            enabled: view.savedViews.length > 0
                            Instantiator {
                                model: view.savedViews
                                delegate: MenuItem {
                                    required property string modelData
                                    text: modelData
                                    onTriggered: view.deleteView(modelData)
                                }
                                onObjectAdded: (i, o) => deleteMenu.insertItem(i, o as MenuItem)
                                onObjectRemoved: (i, o) => deleteMenu.removeItem(o as MenuItem)
                            }
                        }
                    }
                }
                Item { Layout.fillWidth: true }
                Button {
                    text: "Library…"
                    onClicked: win.libraryOpen = true
                }
                Label { text: "Size" }
                Slider {
                    from: 60
                    to: 800
                    value: win.rowHeight
                    onMoved: win.setSize(value)
                    Layout.preferredWidth: 220
                }
                Label { text: Math.round(win.rowHeight) + " px"; Layout.preferredWidth: 50 }
            }
        }

        ToolBar {
            id: bottomBar
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                spacing: 12
                Label {
                    Layout.fillWidth: true
                    elide: Text.ElideMiddle
                    color: "#aaaaaa"
                    // `revision >= 0` is true for any loaded model: it makes this binding depend on `revision`, so it re-evaluates
                    // after a change. The model's roles replace these guards at the QML step (docs/REVIEW.md, R-63).
                    text: galleryModel.revision >= 0 && win.current >= 0
                        ? galleryModel.info(win.current) + "   "
                          + (galleryModel.rating(win.current) > 0
                              ? Ratings.symbol(galleryModel.rating(win.current)) : "")
                        : keymap.gridHint()
                }
                // Ratings and turns on their way into the files, or why they aren't.
                Label {
                    visible: library.writeStatus.length > 0
                    text: library.writeStatus
                    color: library.writeFailed ? "#e0a050" : "#aaaaaa"
                    elide: Text.ElideMiddle
                    Layout.maximumWidth: 500
                }
                // The latest error or note from the background job; click to dismiss.
                Label {
                    visible: library.message.length > 0
                    text: (library.messageWarning ? "⚠ " : "") + library.message
                    color: library.messageWarning ? "#e0a050" : "#aaaaaa"
                    elide: Text.ElideMiddle
                    Layout.maximumWidth: 500
                    TapHandler { onTapped: library.message = "" }
                }
                Label {
                    visible: library.status.length > 0
                    text: library.status
                    color: "#aaaaaa"
                }
                ProgressBar {
                    visible: library.busy
                    indeterminate: library.progress < 0
                    value: Math.max(library.progress, 0)
                    Layout.preferredWidth: 140
                }
                Button {
                    visible: library.busy || library.paused
                    text: library.paused ? "Resume" : "Pause"
                    flat: true
                    onClicked: library.togglePause()
                }
            }
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

            WheelListView {
                id: tree
                SplitView.preferredWidth: 320
                SplitView.minimumWidth: 160
                clip: true
                model: folders
                currentIndex: 0
                ScrollBar.vertical: ScrollBar {}
                delegate: ItemDelegate {
                    id: node
                    required property int index
                    required property string name
                    required property int depth
                    required property bool expandable
                    required property bool expanded
                    required property double total
                    required property string drives
                    required property bool merged
                    width: ListView.view.width
                    height: 26
                    rightPadding: 18  // clear of the scrollbar
                    highlighted: ListView.isCurrentItem
                    contentItem: RowLayout {
                        spacing: 4
                        Item { Layout.preferredWidth: node.depth * 14 }
                        Label {
                            text: node.expandable ? (node.expanded ? "▾" : "▸") : ""
                            Layout.preferredWidth: 12
                            TapHandler { onTapped: folders.toggle(node.index) }
                        }
                        Label { text: node.name; elide: Text.ElideRight; Layout.fillWidth: true }
                        // Folders present on more than one drive are merged into this one node.
                        Label { text: node.drives; color: "#6fbf8f"; visible: node.merged; font.pixelSize: 11 }
                        Label { text: Qt.locale().toString(node.total, "f", 0); color: "#888888"; font.pixelSize: 11 }
                    }
                    onClicked: {
                        tree.currentIndex = index
                        win.openFolder(index)
                        grid.forceActiveFocus()
                    }
                    onDoubleClicked: folders.toggle(index)
                }
            }

            Item {
                SplitView.fillWidth: true

                WheelListView {
                    id: grid
                    anchors.fill: parent
                    anchors.margins: win.gap
                    // The scrollbar's lane: on the right of the grid, not over the rows (its 16 px, and a gap).
                    anchors.rightMargin: win.gap + 16
                    clip: true
                    focus: true
                    model: galleryModel
                    spacing: win.gap
                    cacheBuffer: 1500
                    reuseItems: true
                    // The view is about to be moved: rows are made, reused or changed without their cells until it is
                    // where it is going. Another list is a model reset, which takes the view to the top and fills it
                    // there (and 1500 px beyond) before it can be moved to the list's selection; a list read again or a
                    // relayout changes the rows in view before the view moves to its selection or its anchor.
                    property bool settling: false
                    boundsBehavior: Flickable.StopAtBounds
                    onWidthChanged: layoutTimer.restart()
                    Keys.onPressed: (e) => win.gridKey(e)

                    WheelHandler {
                        acceptedModifiers: Qt.ControlModifier
                        onWheel: (ev) => win.setSize(win.rowHeight * (ev.angleDelta.y > 0 ? 1.1 : 1 / 1.1))
                    }

                    // A row of the grid: a component with a name, so topItem() can read its firstItem typed (and final,
                    // so the QML compiler can trust it through the cast).
                    component GridRow: Item {
                        id: rowItem
                        // The row's items: (first, count), one property (galleryModel's ROLE_SPAN), so that a row
                        // changed in place or reused sees its new items at once, not its first item before its count.
                        required property point span
                        readonly final property int firstItem: span.x
                        required property real rowHeight
                        // New whenever the row's items change in place: rated, turned, marked, rewritten, or another
                        // list (galleryModel's ROLE_VERSION, signalled with a new span too when both change).
                        required property int version
                        width: grid.width
                        height: rowHeight

                        // What the cells show, made to match the row once it is in use (not in the view's pool) and the
                        // view is where it is going (grid.settling): its span's items, at its version. A reuse sets the
                        // span and the version one after the other, before the row is in use. A span change makes the
                        // cells read their items, so a version signalled with it is not read again. Either way the
                        // cells read their items (and ask for their images) once.
                        property int cellsFirst: 0
                        property int cellsCount: 0
                        // Bumped to read the same items again.
                        property int cellsVersion: 0
                        property int shownVersion: -1
                        property bool pooled: false
                        readonly property bool live: !pooled && !grid.settling
                        function update() {
                            if (!live)
                                return
                            const first = span.x
                            const count = span.y
                            if (cellsFirst !== first || cellsCount !== count) {
                                // The Repeater (an integer model) makes all its cells anew when its count changes: they
                                // are made once, for the new items.
                                if (cellsCount !== count)
                                    cellsCount = 0
                                cellsFirst = first
                                cellsCount = count
                            } else if (shownVersion !== version) {
                                cellsVersion++
                            }
                            shownVersion = version
                        }
                        Component.onCompleted: update()
                        onLiveChanged: update()
                        onSpanChanged: update()
                        onVersionChanged: update()
                        ListView.onPooled: pooled = true
                        ListView.onReused: pooled = false

                        Row {
                            spacing: win.gap
                            Repeater {
                                model: rowItem.cellsCount
                                delegate: Item {
                                    id: cell
                                    required property int index
                                    readonly property int item: rowItem.cellsFirst + index
                                    // Changes when the file is rewritten or turned (galleryModel.imageKey).
                                    readonly property string key:
                                        rowItem.cellsVersion >= 0 ? galleryModel.imageKey(item) : ""
                                    readonly property int rating:
                                        rowItem.cellsVersion >= 0 ? galleryModel.rating(item) : 0
                                    readonly property string badge:
                                        rowItem.cellsVersion >= 0 ? galleryModel.badge(item) : ""
                                    // What is left without the borders (an empty rectangle: none found).
                                    readonly property rect content:
                                        rowItem.cellsVersion >= 0 ? galleryModel.borderBox(item)
                                        : Qt.rect(0, 0, 0, 0)
                                    readonly property bool bordersMarked:
                                        rowItem.cellsVersion >= 0 && galleryModel.borderMarked(item)
                                    // The image's aspect as shown; the tile's may be narrower (clamped panoramas), and
                                    // the thumbnail is then cropped to it.
                                    readonly property real imageAspect:
                                        rowItem.cellsVersion >= 0 && galleryModel.fullHeight(item) > 0
                                            ? galleryModel.fullWidth(item) / galleryModel.fullHeight(item) : 1
                                    // A turn changes the item's shape.
                                    width: rowItem.cellsVersion >= 0 ? galleryModel.aspect(item) * rowItem.rowHeight : 0
                                    height: rowItem.rowHeight

                                    // ThumbHash placeholder (from the import) until the thumbnail arrives.
                                    Image {
                                        anchors.fill: parent
                                        source: "image://thumbhash/" + cell.key
                                        asynchronous: true
                                        smooth: true
                                        visible: thumb.status !== Image.Ready
                                    }
                                    Image {
                                        id: thumb
                                        anchors.fill: parent
                                        // Files not imported yet have no size: their cell is square until they are.
                                        fillMode: Image.PreserveAspectCrop
                                        clip: true
                                        source: "image://thumb/" + cell.key + "/" + win.thumbHeight
                                        asynchronous: true
                                        retainWhileLoading: true
                                        smooth: true
                                        mipmap: true
                                        opacity: status === Image.Ready ? 1 : 0
                                        Behavior on opacity { NumberAnimation { duration: 120 } }
                                    }
                                    // B: the borders over the thumbnail, placed as PreserveAspectCrop paints it.
                                    Item {
                                        anchors.fill: parent
                                        clip: true
                                        visible: win.showBorders && cell.content.width > 0
                                        BorderOverlay {
                                            readonly property bool wider: cell.imageAspect > cell.width / cell.height
                                            width: wider ? cell.height * cell.imageAspect : cell.width
                                            height: wider ? cell.height : cell.width / cell.imageAspect
                                            x: (cell.width - width) / 2
                                            y: (cell.height - height) / 2
                                            kept: cell.content
                                            marked: cell.bordersMarked
                                            lineWidth: 1
                                        }
                                    }
                                    Rectangle {
                                        anchors.fill: parent
                                        color: "transparent"
                                        border.color: "#4aa3ff"
                                        border.width: 3
                                        visible: cell.item === win.current
                                    }
                                    // Borders found (grey: marked as not a border).
                                    Label {
                                        anchors.left: parent.left
                                        anchors.top: parent.top
                                        anchors.margins: 4
                                        visible: cell.content.width > 0
                                        text: "▭"
                                        font.pixelSize: 12
                                        font.bold: true
                                        font.strikeout: cell.bordersMarked
                                        color: cell.bordersMarked ? "#a0a0a0" : "#ff4ad8"
                                        padding: 1
                                        leftPadding: 4
                                        rightPadding: 4
                                        background: Rectangle { color: "#a0000000"; radius: 3 }
                                    }
                                    Text {
                                        anchors.left: parent.left
                                        anchors.bottom: parent.bottom
                                        anchors.margins: 4
                                        visible: cell.rating > 0
                                        text: Ratings.symbol(cell.rating)
                                        style: Text.Outline
                                        styleColor: "black"
                                        font.pixelSize: 16
                                    }
                                    // Animated images (and, later, videos) say so.
                                    Label {
                                        anchors.right: parent.right
                                        anchors.top: parent.top
                                        anchors.margins: 4
                                        visible: cell.badge.length > 0
                                        text: cell.badge
                                        font.pixelSize: 11
                                        font.bold: true
                                        color: "white"
                                        padding: 2
                                        leftPadding: 4
                                        rightPadding: 4
                                        background: Rectangle { color: "#a0000000"; radius: 3 }
                                    }
                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: {
                                            win.current = cell.item
                                            grid.forceActiveFocus()
                                        }
                                        onDoubleClicked: viewer.show(cell.item)
                                    }
                                }
                            }
                        }
                    }
                    delegate: GridRow {}
                }

                Label {
                    anchors.centerIn: parent
                    visible: galleryModel.count === 0 && !galleryModel.loading
                    text: library.roots.length === 0
                        ? "No folders in the library yet.\nAdd one with Library… (top right)."
                        : filterBar.active > 0 ? "Nothing here matches the filters."
                        : library.busy ? "Nothing here yet: the library is being imported." : "No images here."
                    horizontalAlignment: Text.AlignHCenter
                    color: "#888888"
                }

                // The grid's scrollbar, in its own lane to the right of the grid. It is a standalone bar, not the view's
                // attached one, which lies over the view's own right edge. Qt's style makes the handle a 6 px nub, as
                // long as it is wide at least: on a big library it is all but impossible to grab, so this one is 12 px
                // wide and at least 48 px long, and brightens as it is hovered and dragged.
                ScrollBar {
                    id: gridBar
                    orientation: Qt.Vertical
                    policy: ScrollBar.AlwaysOn
                    anchors.top: grid.top
                    anchors.bottom: grid.bottom
                    anchors.right: parent.right
                    anchors.rightMargin: win.gap
                    size: grid.visibleArea.heightRatio
                    minimumSize: height > 0 ? Math.min(1, 48 / height) : 0
                    // The view follows the bar while it is not dragged; a drag moves the view.
                    Binding on position {
                        value: grid.visibleArea.yPosition
                        when: !gridBar.pressed
                    }
                    onPositionChanged: if (pressed) grid.contentY = grid.originY + position * grid.contentHeight
                    contentItem: Rectangle {
                        implicitWidth: 12
                        radius: width / 2
                        color: gridBar.pressed ? "#a0a0a0" : gridBar.hovered ? "#808080" : "#5a5a5a"
                    }
                }

                // While the scrollbar is dragged: about where in the library the grid is, in files.
                Rectangle {
                    visible: gridBar.pressed && galleryModel.count > 0
                    width: readout.implicitWidth + 16
                    height: readout.implicitHeight + 8
                    x: parent.width - width - 24
                    y: Math.max(0, Math.min(parent.height - height,
                                            gridBar.y + gridBar.position * gridBar.height - height / 2))
                    radius: 4
                    color: "#2a2a2a"
                    border.color: "#555555"
                    Label {
                        id: readout
                        anchors.centerIn: parent
                        color: "#dddddd"
                        font.pixelSize: 12
                        text: {
                            const f = gridBar.size < 1 ? Math.min(1, gridBar.position / (1 - gridBar.size)) : 0
                            return "file ≈ " + Qt.locale().toString(Math.round(f * (galleryModel.count - 1)) + 1)
                                + " of " + Qt.locale().toString(galleryModel.count)
                        }
                    }
                }
            }
        }

        Viewer {
            id: viewer
            anchors.fill: parent
            index: galleryModel.viewing
            count: galleryModel.count
            // The shown item, read again when the gallery changes it in place (a rating, a turn). Out of range (-1:
            // closed), the gallery gives empty values.
            key: galleryModel.revision >= 0 ? galleryModel.imageKey(viewer.index) : ""
            neighbourKeys: galleryModel.revision >= 0 && viewer.visible ? galleryModel.keysAround(viewer.index, 4) : []
            turns: galleryModel.revision >= 0 ? galleryModel.pendingTurns(viewer.index) : 0
            rating: galleryModel.revision >= 0 ? galleryModel.rating(viewer.index) : 0
            fw: galleryModel.revision >= 0 ? galleryModel.fullWidth(viewer.index) : 0
            fh: galleryModel.revision >= 0 ? galleryModel.fullHeight(viewer.index) : 0
            kind: galleryModel.revision >= 0 ? galleryModel.mediaType(viewer.index) : ""
            playerTurns: galleryModel.revision >= 0 ? galleryModel.playerTurns(viewer.index) : 0
            fileUrl: galleryModel.revision >= 0 ? galleryModel.fileUrl(viewer.index) : Qt.url("")
            filePath: galleryModel.revision >= 0 ? galleryModel.filePath(viewer.index) : ""
            info: galleryModel.revision >= 0 ? galleryModel.info(viewer.index) : ""
            borderBox: galleryModel.revision >= 0 ? galleryModel.borderBox(viewer.index) : Qt.rect(0, 0, 0, 0)
            rejectedTop: galleryModel.revision >= 0 && viewer.visible
                ? galleryModel.borderRejected(viewer.index, "top") : -1
            rejectedBottom: galleryModel.revision >= 0 && viewer.visible
                ? galleryModel.borderRejected(viewer.index, "bottom") : -1
            rejectedLeft: galleryModel.revision >= 0 && viewer.visible
                ? galleryModel.borderRejected(viewer.index, "left") : -1
            rejectedRight: galleryModel.revision >= 0 && viewer.visible
                ? galleryModel.borderRejected(viewer.index, "right") : -1
            borderMarked: galleryModel.revision >= 0 && galleryModel.borderMarked(viewer.index)
            showBorders: win.showBorders
            settings: appSettings
            onMoved: (i) => galleryModel.viewing = i
            // Closed: the keys go back to the grid.
            onVisibleChanged: if (!visible) grid.forceActiveFocus()
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
                Rectangle {
                    id: libraryPanel
                    visible: win.libraryOpen
                    color: "#b0000000"
                    // The Loader is a focus scope: the keys are taken here when it opens, and go back to the grid.
                    Component.onCompleted: forceActiveFocus()
                    onVisibleChanged: visible ? forceActiveFocus() : grid.forceActiveFocus()
                    Keys.onEscapePressed: win.libraryOpen = false
                    // Clicks outside the panel close it; no click or wheel reaches the grid below.
                    MouseArea {
                        anchors.fill: parent
                        onClicked: win.libraryOpen = false
                        onWheel: (wheel) => wheel.accepted = true
                    }

                    Pane {
                        anchors.centerIn: parent
                        width: 680
                        height: 440
                        padding: 16
                        MouseArea { anchors.fill: parent }
                        ColumnLayout {
                            anchors.fill: parent
                            spacing: 10
                            Label { text: "Library folders"; font.bold: true; font.pixelSize: 16 }
                            Label {
                                text: "Images in these folders and their subfolders are imported in the background. "
                                    + "Removing a folder only hides its images: adding it back restores them, ratings "
                                    + "included."
                                color: "#999999"
                                wrapMode: Text.Wrap
                                Layout.fillWidth: true
                            }
                            WheelListView {
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                clip: true
                                model: library.roots
                                ScrollBar.vertical: ScrollBar {}
                                delegate: RowLayout {
                                    id: rootRow
                                    required property string modelData
                                    width: ListView.view.width
                                    Label { text: rootRow.modelData; elide: Text.ElideMiddle; Layout.fillWidth: true }
                                    Button {
                                        text: "Remove"
                                        flat: true
                                        onClicked: library.removeFolder(rootRow.modelData)
                                    }
                                }
                                Label {
                                    anchors.centerIn: parent
                                    visible: library.roots.length === 0
                                    text: "No folders yet."
                                    color: "#888888"
                                }
                            }
                            CheckBox {
                                text: "Write ratings and turns into the files too (where the format allows)"
                                checked: appSettings.writeBack
                                onToggled: appSettings.writeBack = checked
                                ToolTip.visible: hovered
                                ToolTip.text: "Ratings are always kept in the catalog. With this on, they also go into "
                                    + "the image files (XMP), so other programs see them; turns rotate the files "
                                    + "losslessly."
                            }
                            RowLayout {
                                Button { text: "Add folder…"; onClicked: folderDialog.open() }
                                Button {
                                    text: "Rescan all"
                                    enabled: library.roots.length > 0
                                    onClicked: library.rescan()
                                }
                                Item { Layout.fillWidth: true }
                                Button { text: "Close"; onClicked: win.libraryOpen = false }
                            }
                        }
                    }

                    FolderDialog {
                        id: folderDialog
                        title: "Add a folder to the library"
                        onAccepted: library.addFolder(selectedFolder)
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
                    previewWhenFound: win.yandexPreviewHook
                    // Back to the viewer it was opened from, so its arrow keys work again; the grid only if the viewer
                    // is gone.
                    onClosed: viewer.visible ? viewer.forceActiveFocus() : grid.forceActiveFocus()
                    Connections {
                        target: win
                        function onSimilarAsked(path: string) { yandexPanel.show(path) }
                    }
                }
            }
        }

        // Made the first time it is asked for (saveViewAsked), then kept.
        Loader {
            id: saveViewLoader
            active: false
            sourceComponent: Component {
                Dialog {
                    id: saveViewDialog
                    // A Loader doesn't parent a popup to an item: centred over the page.
                    parent: page
                    anchors.centerIn: parent
                    title: "Save the current view"
                    modal: true
                    standardButtons: Dialog.Save | Dialog.Cancel
                    onAboutToShow: {
                        viewName.text = ""
                        viewName.forceActiveFocus()
                    }
                    onAccepted: view.saveView(viewName.text, filterBar.query)
                    onClosed: grid.forceActiveFocus()
                    Connections {
                        target: win
                        function onSaveViewAsked() { saveViewDialog.open() }
                    }
                    ColumnLayout {
                        Label {
                            text: (view.folder.length ? view.folder : "All folders")
                                + (view.recursive ? " and subfolders" : "") + " · " + filterBar.description + " · "
                                + sortBox.currentText + (view.descending ? " ↓" : " ↑")
                            color: "#999999"
                        }
                        TextField {
                            id: viewName
                            placeholderText: "Name (one already saved is replaced)"
                            Layout.preferredWidth: 400
                            onAccepted: saveViewDialog.accept()
                        }
                    }
                }
            }
        }
    }

    // Test hooks, for checking the UI without a person at the screen:
    //   --size=<px>  row height   --select=<n>  select item n   --view=<n>  open item n in the viewer
    //   --viewer-keys=<keys>  e.g. "LB" (loupe, borders); also H (zen), K (play/pause), + (zoom in), 1 (1:1), < > (turn
    //                         left, right), N (not a border), digits after "r" rate
    //   --zoom=<factor>  zoom the viewer to this many device px per image px
    //   --menu=views  open the views menu   --apply-view=<name>  open a saved view
    //   --query=<json>  filter by a query::Query   --sort=<json>  order by a query::Sort   --filters=1  open the bar
    //   --library=1  open the library folders   --save-view=1  open the save-view dialog
    //   --find-similar=<path>  search Yandex for the file (it is uploaded: a path that isn't a still image only shows
    //                          the panel with its error)
    //   --yandex-preview=<n>  with --find-similar: show result n at full size once the results are in
    //   --add-folder=<path>  add a folder to the library (as "Add folder…" does)
    //   --borders  show the borders (as B does) from the start
    //   --save-crops=<folder>  save cropped copies of the listed files with borders found
    //                        (GalleryModel.saveCroppedCopies)
    //   --screenshot=<png>  save the window 4 s (or --wait=<s>) after it is ready, and quit (src/screenshot.rs)
    // --query, --sort and --apply-view shape the first list; the others act on it (started()).
    Timer {
        id: menuTimer
        interval: 500
        onTriggered: viewsButton.clicked()
    }
    // A property, read afresh at each key: a list held in a variable over calls isn't compiled.
    readonly property list<string> viewerKeys: commandLine.keys("viewer-keys")

    // The window is made (it shows right after): ask for where the user was. Nothing here waits for the catalog;
    // the tree and the files arrive later, in either order (treeLoaded, gridLoaded), and the second of them
    // finishes the start (started).
    Component.onCompleted: {
        // The folder, subfolders and sort are the view's (it read them at its start), the filters the bar's.
        filterBar.query = commandLine.has("query") ? commandLine.text("query") : view.sessionQuery()
        if (commandLine.has("sort")) view.setSortJson(commandLine.text("sort"))
        if (commandLine.has("apply-view")) {
            const query = view.applyView(commandLine.text("apply-view"))
            if (query !== "")
                filterBar.query = query
        }
        if (commandLine.has("filters")) filtersOpen = true
        if (commandLine.has("borders")) showBorders = true
        sortBox.currentIndex = sortKeys.indexOf(view.sortKey)
        relayout()
        folders.reload()
        // As it was left: whether the folder still exists is known once the tree is in (treeLoaded).
        galleryModel.openFolder(view.folder, view.recursive, filterBar.query, view.sortJson(), true)
        updateFacets()
    }

    // The first list is shown (with the file the user was on selected, gridLoaded): run the test hooks, then start
    // the background job (after the first reads, which it would compete with for the disk).
    function started() {
        ready = true
        stages.ready()
        if (commandLine.has("apply-view")) view.saveSession(filterBar.query)

        if (commandLine.has("size")) setSize(commandLine.number("size", rowHeight))
        if (commandLine.has("select")) select(commandLine.number("select", -1))
        if (commandLine.has("view")) {
            viewer.show(commandLine.number("view", -1))
            for (let k = 0; k < viewerKeys.length; k++) {
                const c = viewerKeys[k]
                if (c === "L") viewer.loupe = true
                else if (c === "B") showBorders = true
                else if (c === "N") galleryModel.setBorderMarked(viewer.index, !galleryModel.borderMarked(viewer.index))
                else if (c === "H") viewer.zen = true
                else if (c === "K") viewer.togglePlay()
                else if (c === "+") viewer.setZoom(viewer.zoom * 1.25, viewer.width / 2, viewer.height / 2)
                else if (c === "1") viewer.setZoom(1, viewer.width / 2, viewer.height / 2)
                else if (c === "<") viewer.turn(-1)
                else if (c === ">") viewer.turn(1)
                else if (ratingKeys.indexOf(c) >= 0) viewer.rate(ratingKeys.indexOf(c))
            }
            if (commandLine.has("zoom"))
                viewer.setZoom(commandLine.number("zoom", 1), viewer.width / 2, viewer.height / 2)
        }
        if (commandLine.text("menu") === "views") menuTimer.start()
        if (commandLine.has("library")) libraryOpen = true
        if (commandLine.has("save-view")) askSaveView()
        if (commandLine.has("find-similar")) {
            if (commandLine.has("yandex-preview")) yandexPreviewHook = commandLine.number("yandex-preview", 0)
            findSimilar(commandLine.text("find-similar"))
        }
        library.start(thumbHeight)
        if (commandLine.has("add-folder")) library.addFolder(commandLine.url("add-folder"))
        if (commandLine.has("save-crops")) galleryModel.saveCroppedCopies(commandLine.url("save-crops"))
    }
}
