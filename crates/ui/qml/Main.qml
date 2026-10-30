// Main window: merged folder tree on the left, justified thumbnail grid on the right, viewer on top. The
// library (folders the user added) is imported by a background job; its progress is in the bottom bar.
import QtQuick
import QtQuick.Controls
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

    // The grid's row height in logical px: the thumbnail size, adjustable at any time (slider, Ctrl+wheel,
    // Ctrl +/-). The layout follows it at once; the thumbnails re-requested at the new size follow after a short
    // pause (thumbHeight), and meanwhile the old ones are scaled.
    property real rowHeight: 220
    property int thumbHeight: Math.round(rowHeight * Screen.devicePixelRatio)
    property int current: -1
    property bool recursive: true
    readonly property real gap: 4

    FolderModel { id: folders }
    GalleryModel { id: gallery }
    Library {
        id: library
        onScanned: win.reloadAll()
        onImported: win.refreshGrid()
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
        onTriggered: win.thumbHeight = Math.round(win.rowHeight * Screen.devicePixelRatio)
    }
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
        gallery.openFolder(gallery.folder, recursive)
        const i = fid >= 0 ? gallery.indexOf(fid) : -1
        current = hadCurrent ? i : -1
        if (i >= 0)
            grid.positionViewAtIndex(gallery.rowOf(i), hadCurrent ? ListView.Contain : ListView.Beginning)
    }

    function openFolder(row) {
        current = -1
        gallery.openFolder(folders.mergedPath(row), recursive)
        grid.positionViewAtBeginning()
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
            Label { text: gallery.count.toLocaleString(Qt.locale(), "f", 0) + " images"; color: "#999999" }
            CheckBox {
                text: "Include subfolders"
                checked: win.recursive
                onToggled: {
                    win.recursive = checked
                    gallery.openFolder(gallery.folder, checked)
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
                text: gallery.revision >= 0 && win.current >= 0 ? gallery.info(win.current)
                    : "Click to select · double-click or Enter to view · 0–5 rate · arrows move · Ctrl+wheel or Ctrl +/− resize"
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

    SplitView {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: topBar.bottom
        anchors.bottom: bottomBar.top

        ListView {
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
                                readonly property double fid: gallery.fileId(item)
                                width: gallery.aspect(item) * rowItem.rowHeight
                                height: rowItem.rowHeight

                                // ThumbHash placeholder (from the import) until the thumbnail arrives.
                                Image {
                                    anchors.fill: parent
                                    source: "image://thumbhash/" + cell.fid
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
                                    source: "image://thumb/" + cell.fid + "/" + win.thumbHeight
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
                                    text: "★".repeat(gallery.revision >= 0 ? gallery.rating(cell.item) : 0)
                                    color: "#ffd24a"
                                    style: Text.Outline
                                    styleColor: "black"
                                    font.pixelSize: 14
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
        // Clicks outside the panel close it; none reach the grid below.
        MouseArea { anchors.fill: parent; onClicked: libraryPanel.visible = false }

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

    FolderDialog {
        id: folderDialog
        title: "Add a folder to the library"
        onAccepted: library.addFolder(selectedFolder)
    }
    }

    // Test hooks, for checking the UI without a person at the screen:
    //   --size=<px>  row height   --select=<n>  select item n   --view=<n>  open item n in the viewer
    //   --viewer-keys=<keys>  e.g. "LB" (loupe, frame)   --library=1  open the library folders
    //   --add-folder=<path>  add a folder to the library (as "Add folder…" does)
    //   --screenshot=<png>  save the window after 4 s (or --wait=<s>) and quit
    function arg(name) {
        for (const a of Qt.application.arguments)
            if (a.startsWith("--" + name + "="))
                return a.substring(name.length + 3)
        return ""
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
        openFolder(0)
        relayout()
        if (arg("size")) setSize(Number(arg("size")))
        if (arg("select")) select(Number(arg("select")))
        if (arg("view")) {
            viewer.show(Number(arg("view")))
            for (const k of arg("viewer-keys"))
                viewer[{ L: "loupe", B: "frame", Z: "actual" }[k]] = true
        }
        if (arg("library")) libraryPanel.visible = true
        if (arg("screenshot")) shotTimer.start()
        library.start(thumbHeight)
        if (arg("add-folder")) library.addFolder("file:///" + arg("add-folder").replace(/\\/g, "/"))
    }
}
