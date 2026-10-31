// Reverse image search on Yandex for the photo in the viewer: its other sizes, and the images that look like it.
// Pick any of them and save the picks into a folder. The search and the saving run on background threads in the
// YandexSearch object; this panel polls it while it is busy. Over a dimmed window, like the library panel.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Dialogs
import QtQuick.Layouts
import pg.gallery

Rectangle {
    id: panel

    signal closed()

    property string photo: ""
    // The picks, by key ("o" + index, "s" + position): {url, name} for each one that is saved.
    property var picked: ({})
    readonly property int pickedCount: Object.keys(picked).length
    property var otherSizes: []
    property var similar: []

    function show(path) {
        photo = path
        picked = ({})
        otherSizes = []
        similar = []
        visible = true
        // The keys come here (Escape closes), not to the viewer's photo stepping behind.
        forceActiveFocus()
        yandex.search(path)
    }

    function close() {
        visible = false
        closed()
    }

    function toggle(key, item) {
        const next = Object.assign({}, picked)
        if (next[key] !== undefined)
            delete next[key]
        else
            next[key] = item
        picked = next
    }

    // A result: its picture with its size, outlined when picked. A click toggles the pick.
    component ResultTile: Rectangle {
        id: tile
        required property string key
        required property string fullUrl
        required property string thumbUrl
        required property string label
        required property string fileName
        readonly property bool isPicked: panel.picked[tile.key] !== undefined

        width: 176
        height: 176
        radius: 4
        color: "#202020"
        border.width: isPicked ? 3 : 1
        border.color: isPicked ? "#4a90e2" : "#444444"

        Image {
            anchors.fill: parent
            anchors.margins: 6
            source: tile.thumbUrl
            sourceSize.width: 320
            sourceSize.height: 320
            fillMode: Image.PreserveAspectFit
            asynchronous: true
        }
        Label {
            anchors.left: parent.left
            anchors.bottom: parent.bottom
            anchors.margins: 6
            text: tile.label
            color: "#dddddd"
            font.pixelSize: 11
        }
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: panel.toggle(tile.key, { url: tile.fullUrl, name: tile.fileName })
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

    YandexSearch {
        id: yandex
        onResultsChanged: {
            // Cleared when a search starts; nothing to read until it ends.
            if (yandex.results === "")
                return
            const data = JSON.parse(yandex.results)
            panel.otherSizes = data.otherSizes
            panel.similar = data.similar
        }
    }

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
        onAccepted: yandex.download(JSON.stringify(Object.keys(panel.picked).map(k => panel.picked[k])), selectedFolder)
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
                    text: "Save " + panel.pickedCount + (panel.pickedCount === 1 ? " image…" : " images…")
                    enabled: panel.pickedCount > 0 && !yandex.busy
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
                        text: "Other sizes of this photo (" + panel.otherSizes.length + ")"
                        font.bold: true
                        visible: panel.otherSizes.length > 0
                    }
                    Flow {
                        Layout.fillWidth: true
                        spacing: 8
                        Repeater {
                            model: panel.otherSizes
                            ResultTile {
                                required property var modelData
                                required property int index
                                key: "o" + index
                                fullUrl: modelData.url
                                thumbUrl: modelData.url
                                label: modelData.width + "×" + modelData.height
                                fileName: "size " + modelData.width + "x" + modelData.height
                            }
                        }
                    }

                    Label {
                        text: "Similar images (" + panel.similar.length + ")"
                        font.bold: true
                        visible: panel.similar.length > 0
                    }
                    Flow {
                        Layout.fillWidth: true
                        spacing: 8
                        Repeater {
                            model: panel.similar
                            ResultTile {
                                required property var modelData
                                key: "s" + modelData.pos
                                fullUrl: modelData.imageUrl
                                thumbUrl: modelData.thumbUrl || modelData.imageUrl || ""
                                label: modelData.width > 0 ? modelData.width + "×" + modelData.height : ""
                                fileName: "similar " + (modelData.pos + 1)
                            }
                        }
                    }

                    Label {
                        text: "No results."
                        color: "#888888"
                        visible: !yandex.busy && panel.otherSizes.length === 0 && panel.similar.length === 0
                            && yandex.status === ""
                    }
                }
            }
        }
    }
}
