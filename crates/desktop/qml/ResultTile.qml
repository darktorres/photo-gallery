// A result of the Yandex search, in YandexPanel's grid: its picture with its size, outlined when picked. The panel
// owns the search and passes the tile what it shows. A click toggles the pick, Shift-click gives every result from
// the last one clicked to this one that one's state (`rangeTo`), and a double-click shows it at full size
// (`previewRequested`).
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Rectangle {
    id: tile

    // The result's index in the search: the other sizes first, then the similar images.
    required property int item
    required property bool picked
    // The picture's URL (`""` until the search has it), and the size as text (`""` when unknown).
    required property string thumb
    required property string size

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
        source: tile.thumb
        sourceSize.width: 320
        sourceSize.height: 320
        fillMode: Image.PreserveAspectFit
        asynchronous: true
    }
    Label {
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        anchors.margins: 6
        text: tile.size
        color: Theme.tileCaption
        font.pixelSize: 11
    }
    MouseArea {
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onClicked: (mouse) => {
            if (mouse.modifiers & Qt.ShiftModifier)
                tile.rangeTo(tile.item)
            else
                tile.toggled(tile.item)
        }
        // The double-click's first click has toggled the pick already (MouseArea sends no click for the second):
        // toggled back, so looking at a result doesn't change what is picked.
        onDoubleClicked: (mouse) => {
            if (!(mouse.modifiers & Qt.ShiftModifier))
                tile.toggled(tile.item)
            tile.previewRequested(tile.item)
        }
    }
}
