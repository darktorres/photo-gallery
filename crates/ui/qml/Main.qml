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
    visible: true
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
    property int current: -1
    property bool filtersOpen: false
    readonly property real gap: 4
    // query::SortKey names, and the sort box's names for them.
    readonly property list<string> sortKeys: ["name", "size", "modified", "taken", "megapixels", "duration"]
    readonly property list<string> sortNames: ["By name", "By file size", "By date modified", "By date taken",
        "By megapixels", "By duration"]
    // The test hook's rating keys (--viewer-keys): "r3" rates 3.
    readonly property list<string> ratingKeys: ["r0", "r1", "r2", "r3", "r4", "r5"]

    FolderModel { id: folders }
    GalleryModel { id: galleryModel }
    // Where the grid looks, with or without subfolders, in what order; with the filter bar's query, a view.
    // Remembered across runs, and in saved views (src/view.rs).
    View { id: view }
    CommandLine { id: commandLine }
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
    // A refresh that waits for the viewer to close (it would renumber the items under it).
    property bool refreshPending: false

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
        onTriggered: {
            appSettings.lastFile = galleryModel.fileId(viewer.visible ? viewer.index : win.current)
        }
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

    // The first item of the topmost visible row, to keep the view anchored across relayouts.
    function topItem(): int {
        const row = grid.indexAt(10, grid.contentY + 1)
        const del = row >= 0 ? grid.itemAtIndex(row) as GridRow : null
        return del ? del.firstItem : -1
    }

    function relayout() {
        const anchor = current >= 0 ? current : topItem()
        galleryModel.relayout(grid.width, rowHeight, gap)
        if (anchor >= 0)
            grid.positionViewAtIndex(galleryModel.rowOf(anchor), current >= 0 ? ListView.Contain : ListView.Beginning)
    }

    // The folder tree changed (a scan): reload it, keeping the open folder if it still exists.
    function reloadAll() {
        const path = view.folder
        folders.reload()
        const row = folders.rowOf(path)
        tree.currentIndex = Math.max(row, 0)
        if (row < 0)
            openFolder(0)
        else
            refreshGrid()
    }

    // Re-read the open folder (new files, new sizes), keeping the selection or the top of the view in place.
    function refreshGrid() {
        if (viewer.visible) {
            refreshPending = true
            return
        }
        const hadCurrent = current >= 0
        const keep = hadCurrent ? current : topItem()
        let i = galleryModel.openFolder(view.folder, view.recursive, filterBar.query, view.sortJson(), keep)
        updateFacets()
        // The selected file left the view (rated out of "Unrated"): select what took its place.
        if (i < 0 && hadCurrent && galleryModel.count > 0)
            i = Math.min(keep, galleryModel.count - 1)
        current = hadCurrent ? i : -1
        if (i >= 0)
            grid.positionViewAtIndex(galleryModel.rowOf(i), hadCurrent ? ListView.Contain : ListView.Beginning)
    }

    function openFolder(row: int) {
        openPath(folders.mergedPath(row))
    }

    // Show a merged folder (`""` = all) with the current subfolders, filters and sort, and remember it.
    function openPath(path: string) {
        current = -1
        view.folder = path
        galleryModel.openFolder(path, view.recursive, filterBar.query, view.sortJson(), -1)
        updateFacets()
        grid.positionViewAtBeginning()
        sortBox.currentIndex = sortKeys.indexOf(view.sortKey)
        view.saveSession(filterBar.query)
    }

    // Open a folder by path, showing it in the tree (its parents expanded).
    function goTo(path: string) {
        const row = folders.reveal(path)
        tree.currentIndex = Math.max(row, 0)
        if (row >= 0)
            tree.positionViewAtIndex(row, ListView.Contain)
        openPath(row >= 0 ? path : "")
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
        if (i < 0)
            return
        current = i
        grid.positionViewAtIndex(galleryModel.rowOf(i), ListView.Contain)
    }

    function gridKey(e: KeyEvent) {
        const c = current < 0 ? 0 : current
        const ctrl = e.modifiers & Qt.ControlModifier
        if (ctrl && (e.key === Qt.Key_Plus || e.key === Qt.Key_Equal)) setSize(rowHeight * 1.15)
        else if (ctrl && e.key === Qt.Key_Minus) setSize(rowHeight / 1.15)
        else if (e.key === Qt.Key_Left) select(galleryModel.neighbor(c, -1, 0))
        else if (e.key === Qt.Key_Right) select(galleryModel.neighbor(c, 1, 0))
        else if (e.key === Qt.Key_Up) select(galleryModel.neighbor(c, 0, -1))
        else if (e.key === Qt.Key_Down) select(galleryModel.neighbor(c, 0, 1))
        else if (e.key === Qt.Key_Home) select(0)
        else if (e.key === Qt.Key_End) select(galleryModel.count - 1)
        else if ((e.key === Qt.Key_Return || e.key === Qt.Key_Enter || e.key === Qt.Key_Space) && current >= 0)
            viewer.show(current)
        else if (e.key >= Qt.Key_0 && e.key <= Qt.Key_5 && current >= 0) galleryModel.setRating(current, e.key - Qt.Key_0)
        else if (e.key === Qt.Key_R && current >= 0) galleryModel.rotate(current, e.modifiers & Qt.ShiftModifier ? 1 : -1)
        else if (e.key === Qt.Key_F && !ctrl) filtersOpen = !filtersOpen
        else return
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
                text: Qt.locale().toString(galleryModel.count)
                    + (filterBar.active > 0 ? " of " + Qt.locale().toString(filterBar.total) : "")
                    + " files"
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
                text: "Filters" + (filterBar.active > 0 ? " (" + filterBar.active + ")" : "") + (win.filtersOpen ? " ▴" : " ▾")
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
                ToolTip.text: view.descending ? "Descending (click for ascending)" : "Ascending (click for descending)"
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
                        onTriggered: saveViewDialog.open()
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
                onClicked: libraryPanel.visible = true
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
                text: galleryModel.revision >= 0 && win.current >= 0
                    ? galleryModel.info(win.current) + "   " + (galleryModel.rating(win.current) > 0 ? Ratings.symbol(galleryModel.rating(win.current)) : "")
                    : "Click to select · double-click or Enter to view · 0–5 rate · R / Shift+R turn · arrows move"
                      + " · F filters · Ctrl+wheel or Ctrl +/− resize"
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
                text: "⚠ " + library.message
                color: "#e0a050"
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
                clip: true
                focus: true
                model: galleryModel
                spacing: win.gap
                cacheBuffer: 1500
                reuseItems: true
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar { policy: ScrollBar.AlwaysOn }
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
                    required final property int firstItem
                    required property int itemCount
                    required property real rowHeight
                    width: grid.width
                    height: rowHeight

                    Row {
                        spacing: win.gap
                        Repeater {
                            model: rowItem.itemCount
                            delegate: Item {
                                id: cell
                                required property int index
                                readonly property int item: rowItem.firstItem + index
                                // Changes when the file is rewritten or turned (galleryModel.imageKey).
                                readonly property string key: galleryModel.revision >= 0 ? galleryModel.imageKey(item) : ""
                                readonly property int rating: galleryModel.revision >= 0 ? galleryModel.rating(item) : 0
                                readonly property string badge: galleryModel.revision >= 0 ? galleryModel.badge(item) : ""
                                width: galleryModel.aspect(item) * rowItem.rowHeight
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
                                Rectangle {
                                    anchors.fill: parent
                                    color: "transparent"
                                    border.color: "#4aa3ff"
                                    border.width: 3
                                    visible: cell.item === win.current
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
                visible: galleryModel.count === 0
                text: library.roots.length === 0 ? "No folders in the library yet.\nAdd one with Library… (top right)."
                    : filterBar.active > 0 ? "Nothing here matches the filters."
                    : library.busy ? "Nothing here yet: the library is being imported." : "No images here."
                horizontalAlignment: Text.AlignHCenter
                color: "#888888"
            }
        }
    }

    Viewer {
        id: viewer
        anchors.fill: parent
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
        settings: appSettings
        onRated: (i, r) => galleryModel.setRating(i, r)
        onTurned: (i, d) => galleryModel.rotate(i, d)
        onIndexChanged: sessionTimer.restart()
        onFindSimilar: (path) => yandexPanel.show(path)
        onClosed: (i) => {
            win.select(i)
            if (win.refreshPending) {
                win.refreshPending = false
                win.refreshGrid()
            }
            grid.forceActiveFocus()
        }
    }

    // The library folders, over a dimmed window (an item in the page, like the viewer, so window grabs include it).
    Rectangle {
        id: libraryPanel
        anchors.fill: parent
        visible: false
        color: "#b0000000"
        focus: visible
        onVisibleChanged: if (!visible) grid.forceActiveFocus()
        Keys.onEscapePressed: visible = false
        // Clicks outside the panel close it; no click or wheel reaches the grid below.
        MouseArea {
            anchors.fill: parent
            onClicked: libraryPanel.visible = false
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
                    text: "Images in these folders and their subfolders are imported in the background. Removing a "
                        + "folder only hides its images: adding it back restores them, ratings included."
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
                        Button { text: "Remove"; flat: true; onClicked: library.removeFolder(rootRow.modelData) }
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
                    ToolTip.text: "Ratings are always kept in the catalog. With this on, they also go into the image "
                        + "files (XMP), so other programs see them; turns rotate the files losslessly."
                }
                RowLayout {
                    Button { text: "Add folder…"; onClicked: folderDialog.open() }
                    Button {
                        text: "Rescan all"
                        enabled: library.roots.length > 0
                        onClicked: library.rescan()
                    }
                    Item { Layout.fillWidth: true }
                    Button { text: "Close"; onClicked: libraryPanel.visible = false }
                }
            }
        }
    }

    // Reverse image search for the viewer's photo (over the viewer, so it comes after it).
    YandexPanel {
        id: yandexPanel
        // Back to the viewer it was opened from, so its arrow keys work again; the grid only if the viewer is gone.
        onClosed: viewer.visible ? viewer.forceActiveFocus() : grid.forceActiveFocus()
    }

    FolderDialog {
        id: folderDialog
        title: "Add a folder to the library"
        onAccepted: library.addFolder(selectedFolder)
    }

    Dialog {
        id: saveViewDialog
        title: "Save the current view"
        anchors.centerIn: parent
        modal: true
        standardButtons: Dialog.Save | Dialog.Cancel
        onAboutToShow: {
            viewName.text = ""
            viewName.forceActiveFocus()
        }
        onAccepted: view.saveView(viewName.text, filterBar.query)
        onClosed: grid.forceActiveFocus()
        ColumnLayout {
            Label {
                text: (view.folder.length ? view.folder : "All folders") + (view.recursive ? " and subfolders" : "")
                    + " · " + filterBar.description + " · " + sortBox.currentText + (view.descending ? " ↓" : " ↑")
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

    // Test hooks, for checking the UI without a person at the screen:
    //   --size=<px>  row height   --select=<n>  select item n   --view=<n>  open item n in the viewer
    //   --viewer-keys=<keys>  e.g. "LB" (loupe, frame); also H (zen), K (play/pause), + (zoom in), 1 (1:1), < > (turn left,
    //                         right), digits after "r" rate
    //   --zoom=<factor>  zoom the viewer to this many device px per image px
    //   --menu=views  open the views menu   --apply-view=<name>  open a saved view
    //   --query=<json>  filter by a query::Query   --sort=<json>  order by a query::Sort   --filters=1  open the bar
    //   --library=1  open the library folders
    //   --add-folder=<path>  add a folder to the library (as "Add folder…" does)
    //   --screenshot=<png>  save the window after 4 s (or --wait=<s>) and quit (src/screenshot.rs)
    Timer {
        id: menuTimer
        interval: 500
        onTriggered: viewsButton.clicked()
    }
    // A property, read afresh at each key: a list held in a variable over calls isn't compiled.
    readonly property list<string> viewerKeys: commandLine.keys("viewer-keys")

    Component.onCompleted: {
        // Where the user was: the folder, subfolders and sort are the view's (it read them at its start), the
        // filters the bar's.
        filterBar.query = commandLine.has("query") ? commandLine.text("query") : view.sessionQuery()
        if (commandLine.has("sort")) view.setSortJson(commandLine.text("sort"))
        if (commandLine.has("filters")) filtersOpen = true
        goTo(view.folder)
        relayout()
        const last = galleryModel.indexOf(appSettings.lastFile)
        if (last >= 0) select(last)

        if (commandLine.has("size")) setSize(commandLine.number("size", rowHeight))
        if (commandLine.has("select")) select(commandLine.number("select", -1))
        if (commandLine.has("view")) {
            viewer.show(commandLine.number("view", -1))
            for (let k = 0; k < viewerKeys.length; k++) {
                const c = viewerKeys[k]
                if (c === "L") viewer.loupe = true
                else if (c === "B") viewer.frame = true
                else if (c === "H") viewer.zen = true
                else if (c === "K") viewer.togglePlay()
                else if (c === "+") viewer.setZoom(viewer.zoom * 1.25, viewer.width / 2, viewer.height / 2)
                else if (c === "1") viewer.setZoom(1, viewer.width / 2, viewer.height / 2)
                else if (c === "<") viewer.turn(-1)
                else if (c === ">") viewer.turn(1)
                else if (ratingKeys.indexOf(c) >= 0) viewer.rate(ratingKeys.indexOf(c))
            }
            if (commandLine.has("zoom")) viewer.setZoom(commandLine.number("zoom", 1), viewer.width / 2, viewer.height / 2)
        }
        if (commandLine.has("apply-view")) applyView(commandLine.text("apply-view"))
        if (commandLine.text("menu") === "views") menuTimer.start()
        if (commandLine.has("library")) libraryPanel.visible = true
        library.start(thumbHeight)
        if (commandLine.has("add-folder")) library.addFolder(commandLine.url("add-folder"))
    }
}
