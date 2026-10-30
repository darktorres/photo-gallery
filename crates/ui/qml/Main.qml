// Main window: merged folder tree on the left, justified thumbnail grid on the right, viewer on top.
import QtQuick
import QtQuick.Controls
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
        Label {
            anchors.fill: parent
            anchors.leftMargin: 8
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideMiddle
            color: "#aaaaaa"
            text: gallery.revision >= 0 && win.current >= 0 ? gallery.info(win.current)
                : "Click to select · double-click or Enter to view · 0–5 rate · arrows move · Ctrl+wheel or Ctrl +/− resize"
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
                text: "No images here.\nImport with pg.exe (scan, then import), then pick a folder."
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
            grid.forceActiveFocus()
        }
    }
    }

    // Test hooks, for checking the UI without a person at the screen:
    //   --size=<px>  row height   --select=<n>  select item n   --view=<n>  open item n in the viewer
    //   --viewer-keys=<keys>  e.g. "LB" (loupe, frame)   --screenshot=<png>  save the window after 4 s and quit
    function arg(name) {
        for (const a of Qt.application.arguments)
            if (a.startsWith("--" + name + "="))
                return a.substring(name.length + 3)
        return ""
    }
    Timer {
        id: shotTimer
        interval: 4000
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
        if (arg("screenshot")) shotTimer.start()
    }
}
