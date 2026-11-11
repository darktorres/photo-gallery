// One thumbnail of a grid row: the ThumbHash placeholder until the thumbnail arrives, the borders (B), the rating, the
// badge of an animated image, and the selection frame. Its values come from the row (GridRow, from the model's cell
// roles, by index): it calls no method of the model.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Item {
    id: cell
    // The item this cell shows, and its values (see GalleryModel's ROLE_BADGES to ROLE_CONTENT_HEIGHT): the badge, the
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
    // The row's version: the values are read again when it changes (a version of -1 reads nothing).
    required property int version
    required property real rowHeight
    // The image URLs, made by the model (the grammar is Rust's: pg_client::image): the thumbnail at the thumbnail height,
    // and the placeholder (ROLE_THUMB_URLS, ROLE_THUMBHASH_URLS). Their image key (GalleryModel's image_key) changes
    // when the file is rewritten or turned, so the cell shows the new image.
    required property string itemThumbUrl
    required property string itemThumbhashUrl
    required property bool showBorders
    required property int current

    signal cellClicked(int itemIndex)
    signal cellDoubleClicked(int itemIndex)

    readonly property int rating: cell.version >= 0 ? cell.itemRating : 0
    readonly property string badge: cell.version >= 0 ? cell.itemBadge : ""
    // What is left without the borders (an empty rectangle: none found).
    readonly property rect content: cell.version >= 0
        ? Qt.rect(cell.itemContentX, cell.itemContentY, cell.itemContentWidth, cell.itemContentHeight)
        : Qt.rect(0, 0, 0, 0)
    readonly property bool bordersMarked: cell.version >= 0 && cell.itemBordersMarked
    // The image's aspect as shown; the tile's may be narrower (clamped panoramas), and the thumbnail is then cropped to it.
    readonly property real imageAspect: cell.version >= 0 ? cell.itemImageAspect : 1
    // A turn changes the item's shape.
    width: cell.version >= 0 ? cell.itemAspect * cell.rowHeight : 0
    height: cell.rowHeight

    // ThumbHash placeholder (from the import) until the thumbnail is opaque. It stays under the thumbnail while the
    // thumbnail fades in: hiding it as soon as the thumbnail is ready would show the empty frame behind it for the fade.
    Image {
        anchors.fill: parent
        // Cropped as the thumbnail is, so the two line up when they swap.
        fillMode: Image.PreserveAspectCrop
        source: cell.itemThumbhashUrl
        asynchronous: true
        smooth: true
        visible: thumb.opacity < 1
    }
    Image {
        id: thumb
        anchors.fill: parent
        // Files not imported yet have no size: their cell is square until they are.
        fillMode: Image.PreserveAspectCrop
        clip: true
        source: cell.itemThumbUrl
        asynchronous: true
        retainWhileLoading: true
        smooth: true
        mipmap: true
        opacity: status === Image.Ready ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }
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
        border.color: "#4aa3ff"
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
        font.pixelSize: 12
        font.bold: true
        font.strikeout: cell.bordersMarked
        color: cell.bordersMarked ? "#a0a0a0" : "#ff4ad8"
        padding: 1
        leftPadding: 4
        rightPadding: 4
        background: Rectangle { color: "#a0000000"; radius: 3 }
    }
    Text {
        textFormat: Text.PlainText
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        anchors.margins: 4
        visible: cell.rating > 0
        text: Ratings.symbol(cell.rating)
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
        background: Rectangle { color: "#a0000000"; radius: 3 }
    }
    MouseArea {
        anchors.fill: parent
        onClicked: cell.cellClicked(cell.item)
        onDoubleClicked: cell.cellDoubleClicked(cell.item)
    }
}
