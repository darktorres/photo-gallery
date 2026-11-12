// A stub of the GalleryModel (crates/desktop/src/gallery.rs) for the QML tests: the viewed item's values, as the
// viewer reads them. The real one is the Rust side of the window.
import QtQuick

QtObject {
    // The media kinds, as the real enum (gallery.rs MediaKind) names them.
    enum MediaKind { Image, Animated, Video }

    property string viewedKey: ""
    property var viewedNeighbourKeys: []
    property int viewedTurns: 0
    property int viewedRating: 0
    property int viewedFullWidth: 0
    property int viewedFullHeight: 0
    property int viewedKind: GalleryModel.Image
    property int viewedPlayerTurns: 0
    property int viewedFileId: 0
    property string viewedInfo: ""
    property rect viewedBorderBox: Qt.rect(0, 0, 0, 0)
    property real viewedRejectedTop: 0
    property real viewedRejectedBottom: 0
    property real viewedRejectedLeft: 0
    property real viewedRejectedRight: 0
    property bool viewedBorderMarked: false
    // The full-size image's URL (the real one comes from the Rust grammar: pg_client::image).
    property string viewedFullUrl: ""
}
