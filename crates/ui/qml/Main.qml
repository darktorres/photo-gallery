// Main window: merged folder tree on the left, justified thumbnail grid on the right, viewer on top. The
// library (folders the user added) is imported by a background job; its progress is in the bottom bar.
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
    title: "Photo Gallery" + (gallery.folder.length ? " — " + gallery.folder : "")
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

    Settings { id: settings }

    // The grid's row height in logical px: the thumbnail size, adjustable at any time (slider, Ctrl+wheel,
    // Ctrl +/-). The layout follows it at once; the thumbnails re-requested at the new size follow after a short
    // pause (thumbHeight), and meanwhile the old ones are scaled. Remembered across runs.
    property real rowHeight: settings.rowHeight
    property int thumbHeight: Math.round(rowHeight * Screen.devicePixelRatio)
    property int current: -1
    // What the grid shows: the open folder (gallery.folder), with or without subfolders, the files the filter bar
    // keeps (filterBar.query, a query::Query) in an order (a query::Sort). Remembered across runs, and in saved
    // views.
    property bool recursive: true
    property var sort: ({ key: "name", descending: false })
    property bool filtersOpen: false
    readonly property real gap: 4

    FolderModel { id: folders }
    GalleryModel { id: gallery }
    Library {
        id: library
        onScanned: win.reloadAll()
        onImported: win.refreshGrid()
        // Turned files changed in place: no renumbering, so the viewer can stay open.
        onFilesChanged: gallery.refreshTurned()
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
            settings.rowHeight = win.rowHeight
        }
    }
    // The selected file, saved once the selection rests.
    Timer {
        id: sessionTimer
        interval: 1000
        onTriggered: {
            const i = viewer.visible ? viewer.index : win.current
            settings.setSession("file", String(i >= 0 ? gallery.fileId(i) : -1))
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

    function setSize(h) { rowHeight = Math.max(60, Math.min(800, Math.round(h))) }

    function parseJson(text, fallback) {
        try {
            return JSON.parse(text)
        } catch (e) {
            return fallback
        }
    }

    // The counts next to the filter choices, and "N of M". Not needed (and not computed) while nothing filters
    // and the bar is closed.
    function updateFacets() {
        if (filtersOpen || filterBar.active > 0)
            filterBar.facets = JSON.parse(gallery.facets(gallery.folder, recursive, JSON.stringify(filterBar.query)))
    }

    // The filters changed: apply them after a short pause, so typing a number doesn't re-list at every key.
    Timer {
        id: applyTimer
        interval: 300
        onTriggered: win.openPath(gallery.folder)
    }
    onFiltersOpenChanged: updateFacets()

    // The first item of the topmost visible row, to keep the view anchored across relayouts.
    function topItem() {
        const row = grid.indexAt(10, grid.contentY + 1)
        const del = row >= 0 ? grid.itemAtIndex(row) : null
        return del ? del.firstItem : -1
    }

    function relayout() {
        const anchor = current >= 0 ? current : topItem()
        gallery.relayout(grid.width, rowHeight, gap)
        if (anchor >= 0)
            grid.positionViewAtIndex(gallery.rowOf(anchor), current >= 0 ? ListView.Contain : ListView.Beginning)
    }

    // The folder tree changed (a scan): reload it, keeping the open folder if it still exists.
    function reloadAll() {
        const path = gallery.folder
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
        const fid = keep >= 0 ? gallery.fileId(keep) : -1
        gallery.openFolder(gallery.folder, recursive, JSON.stringify(filterBar.query), JSON.stringify(sort))
        updateFacets()
        let i = fid >= 0 ? gallery.indexOf(fid) : -1
        // The selected file left the view (rated out of "Unrated"): select what took its place.
        if (i < 0 && hadCurrent && gallery.count > 0)
            i = Math.min(keep, gallery.count - 1)
        current = hadCurrent ? i : -1
        if (i >= 0)
            grid.positionViewAtIndex(gallery.rowOf(i), hadCurrent ? ListView.Contain : ListView.Beginning)
    }

    function openFolder(row) {
        openPath(folders.mergedPath(row))
    }

    // Show a merged folder (`""` = all) with the current subfolders, filters and sort, and remember it.
    function openPath(path) {
        current = -1
        const query = JSON.stringify(filterBar.query)
        gallery.openFolder(path, recursive, query, JSON.stringify(sort))
        updateFacets()
        grid.positionViewAtBeginning()
        sortBox.currentIndex = sortBox.indexOfValue(sort.key)
        settings.setSession("folder", path)
        settings.setSession("recursive", String(recursive))
        settings.setSession("query", query)
        settings.setSession("sort", JSON.stringify(sort))
    }

    // Open a folder by path, showing it in the tree (its parents expanded).
    function goTo(path) {
        const row = folders.reveal(path)
        tree.currentIndex = Math.max(row, 0)
        if (row >= 0)
            tree.positionViewAtIndex(row, ListView.Contain)
        openPath(row >= 0 ? path : "")
    }

    function applyView(name) {
        const v = settings.view(name)
        if (v.query === undefined)
            return
        recursive = v.recursive
        filterBar.load(parseJson(v.query, {}))
        sort = parseJson(v.sort, { key: "name", descending: false })
        goTo(v.folder)
        grid.forceActiveFocus()
    }

    function select(i) {
        if (i < 0)
            return
        current = i
        grid.positionViewAtIndex(gallery.rowOf(i), ListView.Contain)
    }

    function gridKey(e) {
        const c = current < 0 ? 0 : current
        const ctrl = e.modifiers & Qt.ControlModifier
        if (ctrl && (e.key === Qt.Key_Plus || e.key === Qt.Key_Equal)) setSize(rowHeight * 1.15)
        else if (ctrl && e.key === Qt.Key_Minus) setSize(rowHeight / 1.15)
        else if (e.key === Qt.Key_Left) select(gallery.neighbor(c, -1, 0))
        else if (e.key === Qt.Key_Right) select(gallery.neighbor(c, 1, 0))
        else if (e.key === Qt.Key_Up) select(gallery.neighbor(c, 0, -1))
        else if (e.key === Qt.Key_Down) select(gallery.neighbor(c, 0, 1))
        else if (e.key === Qt.Key_Home) select(0)
        else if (e.key === Qt.Key_End) select(gallery.count - 1)
        else if ((e.key === Qt.Key_Return || e.key === Qt.Key_Enter || e.key === Qt.Key_Space) && current >= 0)
            viewer.show(current)
        else if (e.key >= Qt.Key_0 && e.key <= Qt.Key_5 && current >= 0) gallery.setRating(current, e.key - Qt.Key_0)
        else if (e.key === Qt.Key_R && current >= 0) gallery.rotate(current, e.modifiers & Qt.ShiftModifier ? 1 : -1)
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
                text: gallery.folder.length ? gallery.folder : "All folders"
                font.bold: true
                elide: Text.ElideMiddle
                Layout.maximumWidth: 600
            }
            Label {
                color: "#999999"
                text: gallery.count.toLocaleString(Qt.locale(), "f", 0)
                    + (filterBar.active > 0 ? " of " + filterBar.facets.total.toLocaleString(Qt.locale(), "f", 0) : "")
                    + " files"
            }
            CheckBox {
                text: "Include subfolders"
                checked: win.recursive
                onToggled: {
                    win.recursive = checked
                    win.openPath(gallery.folder)
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
                textRole: "text"
                valueRole: "key"
                // query::SortKey
                model: [
                    { key: "name", text: "By name" },
                    { key: "size", text: "By file size" },
                    { key: "modified", text: "By date modified" },
                    { key: "taken", text: "By date taken" },
                    { key: "megapixels", text: "By megapixels" },
                    { key: "duration", text: "By duration" },
                ]
                onActivated: {
                    // Name reads A→Z; the others start with the largest, newest or longest.
                    win.sort = { key: currentValue, descending: currentValue !== "name" }
                    win.openPath(gallery.folder)
                    grid.forceActiveFocus()
                }
                Layout.preferredWidth: 160
            }
            Button {
                text: win.sort.descending ? "↓" : "↑"
                onClicked: {
                    win.sort = { key: win.sort.key, descending: !win.sort.descending }
                    win.openPath(gallery.folder)
                    grid.forceActiveFocus()
                }
                ToolTip.visible: hovered
                ToolTip.text: win.sort.descending ? "Descending (click for ascending)" : "Ascending (click for descending)"
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
                        model: settings.views
                        delegate: MenuItem {
                            required property string modelData
                            text: modelData
                            onTriggered: win.applyView(modelData)
                        }
                        onObjectAdded: (i, o) => viewsMenu.insertItem(i, o)
                        onObjectRemoved: (i, o) => viewsMenu.removeItem(o)
                    }
                    MenuSeparator {}
                    MenuItem {
                        text: "Save current view…"
                        onTriggered: saveViewDialog.open()
                    }
                    Menu {
                        id: deleteMenu
                        title: "Delete view"
                        enabled: settings.views.length > 0
                        Instantiator {
                            model: settings.views
                            delegate: MenuItem {
                                required property string modelData
                                text: modelData
                                onTriggered: settings.deleteView(modelData)
                            }
                            onObjectAdded: (i, o) => deleteMenu.insertItem(i, o)
                            onObjectRemoved: (i, o) => deleteMenu.removeItem(o)
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
                text: gallery.revision >= 0 && win.current >= 0
                    ? gallery.info(win.current) + "   " + (gallery.rating(win.current) > 0 ? Ratings.symbol(gallery.rating(win.current)) : "")
                    : "Click to select · double-click or Enter to view · 0–5 rate · R / Shift+R turn · arrows move"
                      + " · F filters · Ctrl+wheel or Ctrl +/− resize"
            }
            // Ratings and turns on their way into the files, or why they aren't.
            Label {
                visible: library.writeStatus.length > 0
                text: library.writeStatus
                color: library.writeStatus.indexOf(":") >= 0 ? "#e0a050" : "#aaaaaa"
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
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: topBar.bottom
        visible: win.filtersOpen
        onEdited: applyTimer.restart()
    }

    SplitView {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: filterBar.visible ? filterBar.bottom : topBar.bottom
        anchors.bottom: bottomBar.top

        ListView {
            id: tree
            SplitView.preferredWidth: 320
            SplitView.minimumWidth: 160
            clip: true
            model: folders
            currentIndex: 0
            ScrollBar.vertical: ScrollBar {}
            WheelScroller {}
            delegate: ItemDelegate {
                id: node
                required property int index
                required property string name
                required property int depth
                required property bool expandable
                required property bool expanded
                required property double total
                required property string drives
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
                    Label { text: node.drives; color: "#6fbf8f"; visible: node.drives.indexOf(" ") >= 0; font.pixelSize: 11 }
                    Label { text: node.total.toLocaleString(Qt.locale(), "f", 0); color: "#888888"; font.pixelSize: 11 }
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

            ListView {
                id: grid
                anchors.fill: parent
                anchors.margins: win.gap
                clip: true
                focus: true
                model: gallery
                spacing: win.gap
                cacheBuffer: 1500
                reuseItems: true
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar { policy: ScrollBar.AlwaysOn }
                onWidthChanged: layoutTimer.restart()
                Keys.onPressed: (e) => win.gridKey(e)

                WheelScroller {}
                WheelHandler {
                    acceptedModifiers: Qt.ControlModifier
                    onWheel: (ev) => win.setSize(win.rowHeight * (ev.angleDelta.y > 0 ? 1.1 : 1 / 1.1))
                }

                delegate: Item {
                    id: rowItem
                    required property int firstItem
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
                                // Changes when the file is rewritten or turned (gallery.imageKey).
                                readonly property string key: gallery.revision >= 0 ? gallery.imageKey(item) : ""
                                readonly property int rating: gallery.revision >= 0 ? gallery.rating(item) : 0
                                readonly property string badge: gallery.revision >= 0 ? gallery.badge(item) : ""
                                width: gallery.aspect(item) * rowItem.rowHeight
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
            }

            Label {
                anchors.centerIn: parent
                visible: gallery.count === 0
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
        gallery: gallery
        settings: settings
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
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: library.roots
                    ScrollBar.vertical: ScrollBar {}
                    WheelScroller {}
                    delegate: RowLayout {
                        required property string modelData
                        width: ListView.view.width
                        Label { text: modelData; elide: Text.ElideMiddle; Layout.fillWidth: true }
                        Button { text: "Remove"; flat: true; onClicked: library.removeFolder(modelData) }
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
                    checked: settings.writeBack
                    onToggled: settings.writeBack = checked
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
        onAccepted: if (viewName.text.trim().length)
            settings.saveView(viewName.text, gallery.folder, win.recursive, JSON.stringify(filterBar.query),
                JSON.stringify(win.sort))
        onClosed: grid.forceActiveFocus()
        ColumnLayout {
            Label {
                text: (gallery.folder.length ? gallery.folder : "All folders") + (win.recursive ? " and subfolders" : "")
                    + " · " + filterBar.describe() + " · " + sortBox.currentText + (win.sort.descending ? " ↓" : " ↑")
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
    //   --screenshot=<png>  save the window after 4 s (or --wait=<s>) and quit
    function arg(name) {
        for (const a of Qt.application.arguments)
            if (a.startsWith("--" + name + "="))
                return a.substring(name.length + 3)
        return ""
    }
    Timer {
        id: menuTimer
        interval: 500
        onTriggered: viewsButton.clicked()
    }
    Timer {
        id: shotTimer
        interval: (Number(win.arg("wait")) || 4) * 1000
        onTriggered: page.grabToImage((r) => {
            r.saveToFile(win.arg("screenshot"))
            Qt.quit()
        })
    }

    Component.onCompleted: {
        // Where the user was.
        recursive = settings.session("recursive", "true") === "true"
        filterBar.load(parseJson(settings.session("query", "{}"), {}))
        sort = parseJson(settings.session("sort", ""), { key: "name", descending: false })
        if (arg("query")) filterBar.load(parseJson(arg("query"), {}))
        if (arg("sort")) sort = parseJson(arg("sort"), sort)
        if (arg("filters")) filtersOpen = true
        goTo(settings.session("folder", ""))
        relayout()
        const last = gallery.indexOf(Number(settings.session("file", "-1")))
        if (last >= 0) select(last)

        if (arg("size")) setSize(Number(arg("size")))
        if (arg("select")) select(Number(arg("select")))
        if (arg("view")) {
            viewer.show(Number(arg("view")))
            const keys = arg("viewer-keys")
            for (let k = 0; k < keys.length; k++) {
                const c = keys[k]
                if (c === "L") viewer.loupe = true
                else if (c === "B") viewer.frame = true
                else if (c === "H") viewer.zen = true
                else if (c === "K") viewer.togglePlay()
                else if (c === "+") viewer.setZoom(viewer.zoom * 1.25, viewer.width / 2, viewer.height / 2)
                else if (c === "1") viewer.setZoom(1, viewer.width / 2, viewer.height / 2)
                else if (c === "<") viewer.turn(-1)
                else if (c === ">") viewer.turn(1)
                else if (c === "r" && k + 1 < keys.length) viewer.rate(Number(keys[++k]))
            }
            if (arg("zoom")) viewer.setZoom(Number(arg("zoom")), viewer.width / 2, viewer.height / 2)
        }
        if (arg("apply-view")) applyView(arg("apply-view"))
        if (arg("menu") === "views") menuTimer.start()
        if (arg("library")) libraryPanel.visible = true
        if (arg("screenshot")) shotTimer.start()
        library.start(thumbHeight)
        if (arg("add-folder")) library.addFolder("file:///" + arg("add-folder").replace(/\\/g, "/"))
    }
}
