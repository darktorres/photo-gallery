// A result of the Yandex search, in YandexPanel's justified layout: its picture with its size, and a check badge when
// it is picked. The tile is a delegate of the search's results model (YandexSearch), so its roles are what it shows,
// its frame (where and how big it is in its group's block) included. A click toggles the pick, Shift-click gives every
// result from the last one clicked to this one that one's state (`rangeTo`), and a double-click shows it at full size
// (`previewRequested`).
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Item {
    id: tile

    // The result's index in the search (its row): the other sizes first, then the similar images.
    required property int index
    // The roles: whether it is picked, its picture's URL (`""` until the search has it), and its size in pixels
    // (0 when Yandex gives none).
    required property bool picked
    required property string thumb
    required property int imageWidth
    required property int imageHeight
    // The frame the layout gave it (YandexSearch's frame roles): its place in its group's block, and its size.
    required property real frameX
    required property real frameY
    required property real frameWidth
    required property real frameHeight

    signal toggled(int item)
    signal rangeTo(int item)
    signal previewRequested(int item)

    x: tile.frameX
    y: tile.frameY
    width: tile.frameWidth
    height: tile.frameHeight

    Image {
        anchors.fill: parent
        // The panel shows each group with its own Repeater over the whole model, so a tile of the other group is
        // hidden here and loads no picture.
        source: tile.visible ? tile.thumb : ""
        sourceSize.width: 320
        sourceSize.height: 320
        fillMode: Image.PreserveAspectFit
        asynchronous: true
    }
    // Picked: a check in the accent colour, in the top-right corner.
    Rectangle {
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 6
        width: 22
        height: 22
        radius: 11
        color: Theme.pickedBadge
        visible: tile.picked
        Label {
            anchors.centerIn: parent
            text: "✓"
            color: Theme.pickedText
            font.pixelSize: 13
            font.bold: true
        }
    }
    // The size, on a dark backing: the picture under it may be light or dark.
    Rectangle {
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        anchors.margins: 6
        width: caption.implicitWidth + 10
        height: caption.implicitHeight + 4
        radius: 3
        color: Theme.captionBacking
        visible: caption.text !== ""
        Label {
            id: caption
            textFormat: Text.PlainText
            anchors.centerIn: parent
            text: tile.imageWidth > 0 ? tile.imageWidth + "×" + tile.imageHeight : ""
            color: Theme.tileCaption
            font.pixelSize: 11
        }
    }
    MouseArea {
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onClicked: (mouse) => {
            if (mouse.modifiers & Qt.ShiftModifier)
                tile.rangeTo(tile.index)
            else
                tile.toggled(tile.index)
        }
        // The double-click's first click has toggled the pick already (MouseArea sends no click for the second):
        // toggled back, so looking at a result doesn't change what is picked.
        onDoubleClicked: (mouse) => {
            if (!(mouse.modifiers & Qt.ShiftModifier))
                tile.toggled(tile.index)
            tile.previewRequested(tile.index)
        }
    }
}
