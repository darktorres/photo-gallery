// Reverse image search on Yandex for the photo in the viewer: its other sizes, and the images that look like it.
// Pick any of them (a click, Shift-click for a range, or a group's Select all), look at one at full size
// (double-click), and save the picks into a folder. The search, the results, the picks and the saving are the
// YandexSearch object's (src/yandex.rs), which works on background threads; this panel polls it while it is busy.
// Over a dimmed window, like the library panel.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Dialogs
import QtQuick.Layouts
import QtQuick.Window
import pg.gallery

Rectangle {
    id: panel

    signal closed()

    function show(path: string) {
        preview.visible = false
        visible = true
        // The keys come here (Escape closes), not to the viewer's photo stepping behind.
        forceActiveFocus()
        yandex.search(path)
    }

    function close() {
        visible = false
        closed()
    }

    // For the test hook --yandex-preview: show result `item` at full size once the search has its results.
    property int previewWhenFound: -1
    Connections {
        target: yandex
        function onBusyChanged() {
            if (!yandex.busy && panel.previewWhenFound >= 0) {
                preview.open(panel.previewWhenFound)
                panel.previewWhenFound = -1
            }
        }
    }

    // A group's title with its count, and a button that picks all of it, or none once all are picked.
    component GroupHeader: RowLayout {
        id: header
        property string title
        property int first: 0
        property int count: 0
        readonly property int picked: yandex.revision >= 0 ? yandex.pickedIn(first, count) : 0

        visible: count > 0
        spacing: 12
        Label { text: header.title + " (" + header.count + ")"; font.bold: true }
        Button {
            text: header.picked === header.count ? "Select none" : "Select all"
            flat: true
            onClicked: yandex.setPicked(header.first, header.count, header.picked !== header.count)
        }
    }

    // A result (YandexSearch's item `first + index`): its picture with its size, outlined when picked. A click
    // toggles the pick, Shift-click gives every result from the last one clicked to this one that one's state, and a
    // double-click shows it at full size.
    component ResultTile: Rectangle {
        id: tile
        required property int index
        property int first: 0
        readonly property int item: first + index
        readonly property bool isPicked: yandex.revision >= 0 && yandex.isPicked(item)
        readonly property int pixelWidth: yandex.revision >= 0 ? yandex.width(item) : 0
        readonly property int pixelHeight: yandex.revision >= 0 ? yandex.height(item) : 0

        width: 176
        height: 176
        radius: 4
        color: "#202020"
        border.width: isPicked ? 3 : 1
        border.color: isPicked ? "#4a90e2" : "#444444"

        Image {
            anchors.fill: parent
            anchors.margins: 6
            source: yandex.revision >= 0 ? yandex.thumb(tile.item) : ""
            sourceSize.width: 320
            sourceSize.height: 320
            fillMode: Image.PreserveAspectFit
            asynchronous: true
        }
        Label {
            anchors.left: parent.left
            anchors.bottom: parent.bottom
            anchors.margins: 6
            text: tile.pixelWidth > 0 ? tile.pixelWidth + "×" + tile.pixelHeight : ""
            color: "#dddddd"
            font.pixelSize: 11
        }
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: (mouse) => {
                if (mouse.modifiers & Qt.ShiftModifier)
                    yandex.pickTo(tile.item)
                else
                    yandex.togglePick(tile.item)
            }
            // The double-click's first click has toggled the pick already (MouseArea sends no click for the second):
            // toggled back, so looking at a result doesn't change what is picked.
            onDoubleClicked: (mouse) => {
                if (!(mouse.modifiers & Qt.ShiftModifier))
                    yandex.togglePick(tile.item)
                preview.open(tile.item)
            }
        }
    }

    anchors.fill: parent
    visible: false
    color: "#b0000000"
    focus: visible
    Keys.onEscapePressed: close()
    // Clicks on the dimmed background close it. No click or wheel reaches the viewer below: the results' Flickable
    // passes the wheel on at its ends, and the viewer would zoom.
    MouseArea {
        anchors.fill: parent
        onClicked: panel.close()
        onWheel: (wheel) => wheel.accepted = true
    }

    YandexSearch { id: yandex }

    // While a search or a save runs, the object's state comes across on each tick.
    Timer {
        interval: 250
        repeat: true
        running: yandex.busy
        onTriggered: yandex.poll()
    }

    FolderDialog {
        id: saveDialog
        title: "Save the picked images into"
        onAccepted: yandex.download(selectedFolder)
    }

    Pane {
        anchors.centerIn: parent
        width: Math.min(parent.width - 64, 1040)
        height: Math.min(parent.height - 64, 760)
        padding: 16
        // Clicks inside the panel don't reach the dimmed background.
        MouseArea { anchors.fill: parent }

        ColumnLayout {
            anchors.fill: parent
            spacing: 10

            RowLayout {
                Layout.fillWidth: true
                spacing: 10
                Label { text: "Find similar on Yandex"; font.bold: true; font.pixelSize: 16 }
                BusyIndicator {
                    running: yandex.busy
                    visible: running
                    implicitWidth: 28
                    implicitHeight: 28
                }
                Item { Layout.fillWidth: true }
                Button {
                    text: "Save " + yandex.pickedCount + (yandex.pickedCount === 1 ? " image…" : " images…")
                    enabled: yandex.pickedCount > 0 && !yandex.busy
                    onClicked: {
                        // The folder saved into last, or the photo's.
                        saveDialog.currentFolder = yandex.startFolder()
                        saveDialog.open()
                    }
                }
                Button { text: "Close"; onClicked: panel.close() }
            }

            // The status in full: wrapped, so nothing is cut off, and selectable so it can be copied.
            RowLayout {
                Layout.fillWidth: true
                spacing: 10
                visible: yandex.status !== ""

                TextEdit {
                    id: statusText
                    Layout.fillWidth: true
                    text: yandex.status
                    color: "#cccccc"
                    font.pixelSize: 13
                    wrapMode: TextEdit.Wrap
                    readOnly: true
                    selectByMouse: true
                }
                Button {
                    text: "Copy"
                    onClicked: {
                        statusText.selectAll()
                        statusText.copy()
                    }
                }
            }

            Flickable {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: sections.height
                ScrollBar.vertical: ScrollBar {}

                ColumnLayout {
                    id: sections
                    width: parent.width
                    spacing: 12

                    GroupHeader {
                        title: "Other sizes of this photo"
                        count: yandex.otherCount
                    }
                    Flow {
                        Layout.fillWidth: true
                        spacing: 8
                        Repeater {
                            model: yandex.otherCount
                            ResultTile {}
                        }
                    }

                    GroupHeader {
                        title: "Similar images"
                        first: yandex.otherCount
                        count: yandex.similarCount
                    }
                    Flow {
                        Layout.fillWidth: true
                        spacing: 8
                        Repeater {
                            model: yandex.similarCount
                            ResultTile { first: yandex.otherCount }
                        }
                    }

                    Label {
                        text: "No results."
                        color: "#888888"
                        visible: !yandex.busy && yandex.otherCount === 0 && yandex.similarCount === 0
                            && yandex.status === ""
                    }
                }
            }
        }
    }

    // One result at full size (a tile's double-click), loaded from its URL: fitted to the window, enlarged when it is
    // smaller, or 1:1 (one file pixel per screen pixel) by a click on it or the button. ←/→ step through all the results, Space
    // picks or unpicks, Escape goes back to the list.
    Rectangle {
        id: preview
        property int item: -1
        property bool actual: false
        readonly property int total: yandex.otherCount + yandex.similarCount
        readonly property bool isPicked: yandex.revision >= 0 && item >= 0 && yandex.isPicked(item)

        function open(i: int) {
            item = i
            actual = false
            visible = true
            forceActiveFocus()
        }

        function close() {
            visible = false
            item = -1
            panel.forceActiveFocus()
        }

        function step(by: int) {
            if (total > 0) {
                item = (item + by + total) % total
                actual = false
            }
        }

        // 1:1 opens on the image's middle; fitted, the content is the view, scrolled to its start. Done once the
        // content has its new size, on the next event-loop pass.
        Timer {
            id: centre
            interval: 0
            onTriggered: {
                previewView.contentX = Math.max(0, (previewView.contentWidth - previewView.width) / 2)
                previewView.contentY = Math.max(0, (previewView.contentHeight - previewView.height) / 2)
            }
        }
        onActualChanged: centre.restart()

        anchors.fill: parent
        visible: false
        color: "#141414"
        Keys.onEscapePressed: close()
        Keys.onLeftPressed: step(-1)
        Keys.onRightPressed: step(1)
        Keys.onSpacePressed: yandex.togglePick(item)
        // Nothing reaches the list below.
        MouseArea {
            anchors.fill: parent
            onWheel: (wheel) => wheel.accepted = true
        }

        RowLayout {
            id: previewBar
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 12
            spacing: 10

            Label {
                text: preview.item < 0 ? ""
                    : preview.item < yandex.otherCount
                        ? "Other size " + (preview.item + 1) + " of " + yandex.otherCount
                        : "Similar image " + (preview.item - yandex.otherCount + 1) + " of " + yandex.similarCount
                font.bold: true
            }
            // The file's own size once it is loaded; Yandex's until then.
            Label {
                text: full.status === Image.Ready ? full.sourceSize.width + "×" + full.sourceSize.height
                    : yandex.revision >= 0 && preview.item >= 0 && yandex.width(preview.item) > 0
                        ? yandex.width(preview.item) + "×" + yandex.height(preview.item) : ""
                color: "#cccccc"
            }
            Label {
                text: full.status === Image.Loading ? "Loading… " + Math.round(full.progress * 100) + "%"
                    : full.status === Image.Error ? "Can't load or decode it; saving may still work." : ""
                color: full.status === Image.Error ? "#e57373" : "#999999"
                Layout.fillWidth: true
                elide: Text.ElideRight
            }
            // No focus for the buttons: the arrow keys and Space stay the preview's.
            Button {
                text: preview.isPicked ? "Picked ✓" : "Pick"
                highlighted: preview.isPicked
                focusPolicy: Qt.NoFocus
                onClicked: yandex.togglePick(preview.item)
            }
            Button {
                text: preview.actual ? "Fit" : "1:1"
                enabled: full.zoomable
                focusPolicy: Qt.NoFocus
                onClicked: preview.actual = !preview.actual
            }
            Button { text: "Back"; focusPolicy: Qt.NoFocus; onClicked: preview.close() }
        }

        Flickable {
            id: previewView
            anchors.fill: parent
            anchors.topMargin: previewBar.height + 24
            anchors.leftMargin: 12
            anchors.rightMargin: 12
            anchors.bottomMargin: 12
            clip: true
            contentWidth: Math.max(width, full.width)
            contentHeight: Math.max(height, full.height)
            ScrollBar.vertical: ScrollBar {}
            ScrollBar.horizontal: ScrollBar {}

            Image {
                id: full
                // Its size in logical pixels at 1:1, and the scale that fits it to the view, up or down.
                readonly property real naturalWidth: sourceSize.width / Screen.devicePixelRatio
                readonly property real naturalHeight: sourceSize.height / Screen.devicePixelRatio
                readonly property real fit: naturalWidth > 0
                    ? Math.min(previewView.width / naturalWidth, previewView.height / naturalHeight) : 1
                // 1:1 differs from the fit: it is worth a button and a click.
                readonly property bool zoomable: Math.abs(fit - 1) > 0.01
                readonly property real zoom: preview.actual ? 1 : fit

                source: yandex.revision >= 0 && preview.visible && preview.item >= 0 ? yandex.url(preview.item) : ""
                asynchronous: true
                smooth: true
                mipmap: true
                width: naturalWidth * zoom
                height: naturalHeight * zoom
                x: Math.max(0, (previewView.width - width) / 2)
                y: Math.max(0, (previewView.height - height) / 2)

                TapHandler {
                    onTapped: if (full.zoomable) preview.actual = !preview.actual
                }
            }
        }
        BusyIndicator {
            anchors.centerIn: previewView
            running: full.status === Image.Loading
            visible: running
        }
    }
}
