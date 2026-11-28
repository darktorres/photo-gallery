// A result of the Yandex search, in YandexPanel's justified layout: its picture with its size, and a check badge when
// it is picked. The tile is a delegate of the search's results model (YandexSearch), so its roles are what it shows,
// its frame (where and how big it is in its group's block) included. A click toggles the pick, Shift-click gives every
// result from the last one clicked to this one that one's state (`rangeTo`), and a double-click shows it at full size
// (`previewRequested`). The keyboard does the same: Tab reaches a tile, Space picks it (Shift+Space range-picks, as
// Shift-click does) and Enter shows it at full size.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtMultimedia
import pg.gallery

FocusScope {
    id: tile

    // The keys of a tile that has the focus: the pick and the open, as the mouse gives them.
    activeFocusOnTab: true
    Keys.onPressed: e => {
        if (e.key === Qt.Key_Space) {
            if ((e.modifiers & Qt.ShiftModifier) !== 0)
                tile.rangeTo(tile.index);
            else
                tile.toggled(tile.index);
        } else if (e.key === Qt.Key_Return || e.key === Qt.Key_Enter) {
            tile.previewRequested(tile.index);
        } else {
            return;
        }
        e.accepted = true;
    }

    // The result's index in the search (its row): the other sizes first, then the similar images.
    required property int index
    // The roles: whether it is picked, its picture's URL (`""` until the search has it), and its size in pixels: its
    // file's, as it decodes (0 until the file is in; Yandex's stated size is never shown).
    required property bool picked
    required property string thumb
    required property int imageWidth
    required property int imageHeight
    // The frame the layout gave it (YandexSearch's frame roles): its place in its group's block, and its size.
    required property real frameX
    required property real frameY
    required property real frameWidth
    required property real frameHeight
    // Whether it is one of the other sizes (the first group); the panel shows each group's tiles by it.
    required property bool isOther

    // The row whose picture may load now: the rows at or before it (YandexSearch.thumbTurn), in order.
    required property int loadTurn

    // The animation: the result's original URL ("" for a still), its file's turn and mirror (Orientation::parts), and
    // whether the panel lets tiles play now (the list is at rest, and the preview is closed).
    required property string animationUrl
    required property bool mirrored
    required property int turns
    required property bool mayPlay
    // The tile's block in the list's content (its group's top), and the list's scroll and height: a tile plays only while
    // it is on screen (or a row's height from it).
    required property real blockY
    required property real viewTop
    required property real viewHeight

    // The tile's top in the list's content, and whether it is on screen with a row's margin either side.
    readonly property real contentTop: tile.blockY + tile.frameY
    readonly property bool onScreen: tile.contentTop + tile.frameHeight >= tile.viewTop - tile.frameHeight
        && tile.contentTop <= tile.viewTop + tile.viewHeight + tile.frameHeight
    // The tile plays its animation: an animated result, shown, on screen, with the panel allowing it.
    readonly property bool playing: tile.visible && tile.animationUrl !== "" && tile.mayPlay && tile.onScreen

    signal toggled(int item)
    signal rangeTo(int item)
    signal previewRequested(int item)
    // The picture loaded or failed: the search moves its turn on from this row.
    signal thumbSettled(int row)

    x: tile.frameX
    y: tile.frameY
    width: tile.frameWidth
    height: tile.frameHeight

    Image {
        anchors.fill: parent
        // The panel shows each group with its own Repeater over the whole model, so a tile of the other group is
        // hidden here and loads no picture. The loader serves queued requests newest first, so the pictures are asked
        // for one row at a time, in order: a tile asks once its row is at the turn or before it.
        source: tile.visible && tile.index <= tile.loadTurn ? tile.thumb : ""
        sourceSize.width: 320
        sourceSize.height: 320
        fillMode: Image.PreserveAspectFit
        asynchronous: true
        onStatusChanged: {
            if (status === Image.Ready || status === Image.Error)
                tile.thumbSettled(tile.index);
        }
    }
    // An animated result plays over its still while the tile `playing` (the loader's player and output are made then and
    // dropped after, so only the tiles on screen hold a player). Over a white backing, as the still is flattened onto
    // white, once a frame is drawn: the still then gives way to the animation.
    Loader {
        id: animated
        anchors.fill: parent
        active: tile.playing
        sourceComponent: Component {
            Item {
                id: anim
                anchors.fill: parent
                property bool drawn: false
                Rectangle {
                    anchors.fill: parent
                    color: "white"
                    visible: anim.drawn
                }
                TurnedVideoOutput {
                    id: video
                    anchors.fill: parent
                    swapsAxes: tile.turns % 2 === 1
                    turns: tile.turns
                    mirrored: tile.mirrored
                    fillMode: VideoOutput.PreserveAspectCrop
                }
                ResultAnimationPlayer {
                    url: tile.animationUrl
                    height: Math.round(tile.frameHeight)
                    Component.onCompleted: setSink(video.videoSink)
                }
                Connections {
                    target: video.videoSink
                    function onVideoFrameChanged() {
                        anim.drawn = true;
                    }
                }
            }
        }
    }
    // The keyboard's place: a frame while the tile has the focus.
    Rectangle {
        anchors.fill: parent
        color: "transparent"
        border.color: Theme.pickedBadge
        border.width: 2
        visible: tile.activeFocus
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
            font.family: Theme.symbolFamily
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
    // A click acts once the double-click time has passed without a second click: a double click (which opens the preview)
    // must not also pick or range-pick on its first click, and its modifiers may differ from the second's.
    Timer {
        id: clickSettle
        property bool shift: false
        interval: Application.styleHints.mouseDoubleClickInterval
        onTriggered: {
            if (clickSettle.shift)
                tile.rangeTo(tile.index);
            else
                tile.toggled(tile.index);
        }
    }
    MouseArea {
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onClicked: mouse => {
            clickSettle.shift = (mouse.modifiers & Qt.ShiftModifier) !== 0;
            clickSettle.restart();
        }
        onDoubleClicked: mouse => {
            clickSettle.stop();
            tile.previewRequested(tile.index);
        }
    }
}
