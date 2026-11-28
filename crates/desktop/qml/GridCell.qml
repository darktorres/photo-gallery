// One thumbnail of a grid row: the ThumbHash placeholder until the thumbnail arrives, the borders (B), the rating, the
// badge of an animated image, and the selection frame. Its values come from the row (GridRow, from the model's row, by
// index): it calls no method of the model.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtMultimedia
import pg.gallery

Item {
    id: cell
    // The item this cell shows, and its values (the row's value, see GalleryModel's row role): the badge, the
    // rating, whether the borders are marked, the aspect of the image as shown and of the tile, and the borders' box
    // (left as fractions of the image: x, y, width, height).
    required property int item
    required property string itemBadge
    required property int itemRating
    required property bool itemBordersMarked
    required property real itemImageAspect
    required property real itemAspect
    required property real itemContentX
    required property real itemContentY
    required property real itemContentWidth
    required property real itemContentHeight
    required property real rowHeight
    // The file the cell shows (its id), and its animation: whether it is one, the turns and mirror it is shown with, and
    // whether its row may play it now (GridRow.plays).
    required property real itemFileId
    required property bool itemAnimated
    required property int itemPlayerTurns
    required property bool itemPlayerMirrored
    required property bool itemPlays
    // The image URLs, made by the model (the grammar is Rust's: pg_client::image): the thumbnail at the thumbnail height,
    // and the placeholder (the row's thumbUrls and thumbhashUrls). Their image key (GalleryModel's image_key) changes
    // when the file is rewritten or turned, so the cell shows the new image.
    required property string itemThumbUrl
    required property string itemThumbhashUrl
    required property bool showBorders
    required property int current
    // Whether the cell's pictures may be asked for now (GridRow.loadStarted). Until they may, the images have no source, so
    // the loader's queue is not filled from the bottom of the grid up.
    property bool loadNow: true
    // The thumbnail's source while the cell loads it: nothing when the row is pooled. A named function, so the retry can bind
    // to it without a closure.
    function thumbSource(): string {
        return cell.loadNow ? cell.itemThumbUrl : "";
    }
    // True while a retry sets the thumbnail's source by hand (the binding below is off then).
    property bool retrying: false
    Binding {
        target: thumb
        property: "source"
        value: cell.thumbSource()
        when: !cell.retrying
    }

    signal cellClicked(int itemIndex)
    signal cellDoubleClicked(int itemIndex)

    readonly property int rating: cell.itemRating
    readonly property string badge: cell.itemBadge
    // What is left without the borders (an empty rectangle: none found).
    readonly property rect content: Qt.rect(cell.itemContentX, cell.itemContentY, cell.itemContentWidth, cell.itemContentHeight)
    readonly property bool bordersMarked: cell.itemBordersMarked
    // The image's aspect as shown; the tile's may be narrower (clamped panoramas), and the thumbnail is then cropped to it.
    readonly property real imageAspect: cell.itemImageAspect
    // The thumbnail is still to come: it has a URL, and is neither ready nor failed (GridRow counts these).
    final readonly property bool thumbPending: cell.itemThumbUrl !== "" && thumb.status !== Image.Ready && thumb.status !== Image.Error
    // A turn changes the item's shape.
    width: cell.itemAspect * cell.rowHeight
    height: cell.rowHeight

    // ThumbHash placeholder (made after the thumbnail) until the thumbnail is opaque. It stays under the thumbnail while the
    // thumbnail fades in: hiding it as soon as the thumbnail is ready would show the empty frame behind it for the fade.
    Image {
        anchors.fill: parent
        // Cropped as the thumbnail is, so the two line up when they swap.
        fillMode: Image.PreserveAspectCrop
        source: cell.loadNow ? cell.itemThumbhashUrl : ""
        asynchronous: true
        smooth: true
        visible: thumb.opacity < 1
    }
    Image {
        id: thumb
        // Found by tst_gridcell_fade.qml, which sets the fade.
        objectName: "thumb"
        anchors.fill: parent
        // Files not imported yet have no size: their cell is square until they are.
        fillMode: Image.PreserveAspectCrop
        clip: true
        asynchronous: true
        retainWhileLoading: true
        smooth: true
        mipmap: true
        // A thumbnail on show: ready, and not failed or cleared since. A thumbnail that is replaced keeps its place while the
        // new one loads (retainWhileLoading), so the fade stays at 1 for it: the old picture does not fade out.
        property bool shown: false
        // A failed thumbnail is asked for again (retryTimer), for as long as it fails; a ready one resets the count.
        property int retries: 0
        onStatusChanged: {
            if (status === Image.Ready) {
                shown = true;
                retries = 0;
                retryTimer.stop();
            } else if (status === Image.Error) {
                shown = false;
                retryTimer.restart();
            } else if (status === Image.Null) {
                shown = false;
                retryTimer.stop();
            }
        }
        // The fade in, from 0 to 1 once the thumbnail is ready.
        property real fade: shown ? 1 : 0
        Behavior on fade {
            NumberAnimation {
                duration: 120
                easing.type: Easing.OutCubic
            }
        }
        // The fade's last stretch shows as 1. Qt's software renderer (offscreen runs and the screenshots) ignores an
        // opacity within 1e-5 of the one it has (qFuzzyCompare in QSGSoftwareRenderableNode::setOpacity, Qt 6.12): a
        // frame of the fade at 0.999996 kept it there, under QML's final 1, and the thumbnail was drawn at alpha 254
        // (every channel from 128 one lower) in about one run in ten. From 0.999 the final 1 is far outside that
        // tolerance, and the step is invisible.
        opacity: fade < 0.999 ? fade : 1
    }
    // An animated image plays over its still while its row plays it (and only then: the player is made and dropped with
    // it). Over a white backing, as the still is flattened onto white, once a frame is drawn.
    Loader {
        id: animated
        anchors.fill: parent
        active: cell.itemAnimated && cell.itemPlays && cell.itemFileId > 0
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
                    swapsAxes: cell.itemPlayerTurns % 2 === 1
                    turns: cell.itemPlayerTurns
                    mirrored: cell.itemPlayerMirrored
                    fillMode: VideoOutput.PreserveAspectCrop
                }
                AnimationPlayer {
                    objectName: "cellAnimation"
                    fileId: cell.itemFileId
                    // The thumbnail's height: the frames are as sharp as the still, and small in memory.
                    height: Math.round(cell.rowHeight)
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
    // Asks again for a thumbnail that failed. Qt does not request an unchanged source again after an error (setSource
    // returns when the URL is the same), so the source is set again: a file that was unreadable for a moment (an offline
    // drive, a file another process holds) is read once it is. Qt does re-request on a device pixel ratio change by itself
    // (QQuickImageBase::itemChange calls load()). The wait is 1 s, doubling, capped at 15 s, and it goes on while the
    // thumbnail fails; a row that is pooled (off the view) has no source, so it stops there. A new URL starts over (the
    // Connections below).
    Timer {
        id: retryTimer
        repeat: false
        interval: Math.min(1000 * Math.pow(2, Math.min(thumb.retries, 4)), 15000)
        onTriggered: {
            thumb.retries++;
            // The binding below is off while the source is set by hand, so the same URL is asked for again.
            cell.retrying = true;
            thumb.source = "";
            thumb.source = cell.thumbSource();
            cell.retrying = false;
        }
    }
    Connections {
        target: cell
        function onItemThumbUrlChanged() {
            thumb.retries = 0;
            retryTimer.stop();
        }
    }
    // A thumbnail that won't load (it doesn't decode, or its original can't be read): an error mark in its place, a red
    // circle with a white cross, drawn with shapes so no font is needed. The placeholder under it is blank for such a file
    // (no ThumbHash, so a transparent image), so the cell would be empty.
    Item {
        objectName: "failed"
        anchors.centerIn: parent
        visible: thumb.status === Image.Error
        width: Math.round(Math.min(cell.width, cell.height) / 3)
        height: width
        Rectangle {
            anchors.fill: parent
            radius: width / 2
            color: Theme.errorText
        }
        Rectangle {
            anchors.centerIn: parent
            width: parent.width * 0.5
            height: Math.max(2, parent.width * 0.12)
            radius: height / 2
            color: "white"
            rotation: 45
        }
        Rectangle {
            anchors.centerIn: parent
            width: parent.width * 0.5
            height: Math.max(2, parent.width * 0.12)
            radius: height / 2
            color: "white"
            rotation: -45
        }
    }
    // B: the borders over the thumbnail, placed as PreserveAspectCrop paints it.
    Item {
        anchors.fill: parent
        clip: true
        visible: cell.showBorders && cell.content.width > 0
        BorderOverlay {
            readonly property bool wider: cell.imageAspect > cell.width / cell.height
            width: wider ? cell.height * cell.imageAspect : cell.width
            height: wider ? cell.height : cell.width / cell.imageAspect
            x: (cell.width - width) / 2
            y: (cell.height - height) / 2
            kept: cell.content
            marked: cell.bordersMarked
            lineWidth: 1
        }
    }
    Rectangle {
        anchors.fill: parent
        color: "transparent"
        border.color: Theme.selectionOutline
        border.width: 3
        visible: cell.item === cell.current
    }
    // Borders found (grey: marked as not a border).
    Label {
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.margins: 4
        visible: cell.content.width > 0
        text: "▭"
        font.family: Theme.symbolFamily
        font.pixelSize: 12
        font.bold: true
        font.strikeout: cell.bordersMarked
        color: cell.bordersMarked ? Theme.borderMarkedTint : Theme.borderTint
        padding: 1
        leftPadding: 4
        rightPadding: 4
        background: Rectangle {
            color: Theme.scrimBacking
            radius: 3
        }
    }
    Text {
        textFormat: Text.PlainText
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        anchors.margins: 4
        visible: cell.rating > 0
        // No text for no rating: a hidden text is laid out all the same, and ☆ (no rating) is no emoji. The ratings shown
        // are emoji, in the emoji font (Theme.emojiFamily).
        text: cell.rating > 0 ? Ratings.symbol(cell.rating) : ""
        font.family: Theme.emojiFamily
        style: Text.Outline
        styleColor: "black"
        font.pixelSize: 16
    }
    // Animated images (and, later, videos) say so.
    Label {
        textFormat: Text.PlainText
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
        background: Rectangle {
            color: Theme.scrimBacking
            radius: 3
        }
    }
    MouseArea {
        anchors.fill: parent
        onClicked: cell.cellClicked(cell.item)
        onDoubleClicked: cell.cellDoubleClicked(cell.item)
    }
}
