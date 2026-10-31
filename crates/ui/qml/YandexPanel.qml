// Reverse image search on Yandex for the photo in the viewer: its other sizes, and the images that look like it.
// Pick any of them and save the picks into a folder. The search, the results, the picks and the saving are the
// YandexSearch object's (src/yandex.rs), which works on background threads; this panel polls it while it is busy.
// Over a dimmed window, like the library panel.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Dialogs
import QtQuick.Layouts
import pg.gallery

Rectangle {
    id: panel

    signal closed()

    function show(path: string) {
        visible = true
        // The keys come here (Escape closes), not to the viewer's photo stepping behind.
        forceActiveFocus()
        yandex.search(path)
    }

    function close() {
        visible = false
        closed()
    }

    // A result (YandexSearch's item `first + index`): its picture with its size, outlined when picked. A click
    // toggles the pick.
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
            onClicked: yandex.togglePick(tile.item)
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
                    onClicked: saveDialog.open()
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

                    Label {
                        text: "Other sizes of this photo (" + yandex.otherCount + ")"
                        font.bold: true
                        visible: yandex.otherCount > 0
                    }
                    Flow {
                        Layout.fillWidth: true
                        spacing: 8
                        Repeater {
                            model: yandex.otherCount
                            ResultTile {}
                        }
                    }

                    Label {
                        text: "Similar images (" + yandex.similarCount + ")"
                        font.bold: true
                        visible: yandex.similarCount > 0
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
}
