// A result of the Yandex search, in YandexPanel's grid: its picture with its size, outlined when picked. The tile is a
// delegate of the search's results model (YandexSearch), so its roles are what it shows. A click toggles the pick,
// Shift-click gives every result from the last one clicked to this one that one's state (`rangeTo`), and a
// double-click shows it at full size (`previewRequested`).
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Rectangle {
    id: tile

    // The result's index in the search (its row): the other sizes first, then the similar images.
    required property int index
    // The roles: whether it is picked, its picture's URL (`""` until the search has it), and its size in pixels
    // (0 when Yandex gives none).
    required property bool picked
    required property string thumb
    required property int imageWidth
    required property int imageHeight

    signal toggled(int item)
    signal rangeTo(int item)
    signal previewRequested(int item)

    width: 176
    height: 176
    radius: 4
    color: Theme.tileFill
    border.width: tile.picked ? 3 : 1
    border.color: tile.picked ? Theme.pickedOutline : Theme.tileOutline

    Image {
        anchors.fill: parent
        anchors.margins: 6
        // The panel shows each group with its own Repeater over the whole model, so a tile of the other group is
        // hidden here and loads no picture.
        source: tile.visible ? tile.thumb : ""
        sourceSize.width: 320
        sourceSize.height: 320
        fillMode: Image.PreserveAspectFit
        asynchronous: true
    }
    Label {
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        anchors.margins: 6
        text: tile.imageWidth > 0 ? tile.imageWidth + "×" + tile.imageHeight : ""
        color: Theme.tileCaption
        font.pixelSize: 11
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
