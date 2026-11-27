// A stub of the GalleryModel (crates/desktop/src/gallery.rs) for the QML tests: the viewed item's values, as the
// viewer reads them. The real one is the Rust side of the window.
import QtQuick

QtObject {
    // The media kinds, as the real enum (gallery.rs MediaKind) names them.
    enum MediaKind {
        Image,
        Animated,
        Video
    }

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
    // Why the viewed item's fit failed (the window's FitError codes: 0 none).
    property int viewedFitError: 0
    // The list shown was asked for and could not be read (its rows are kept).
    property bool listingFailed: false
    // A rating was written for an item (the real one is emitted once the catalog and the file hold it).
    signal ratingWritten(int rating)
    // The full-size image's URL (the real one comes from the Rust grammar: pg_client::image).
    property string viewedFullUrl: ""

    // The cut-borders calls of Main (askCutBorders, the dialog's confirmed): the real ones are the Rust invokables.
    function cutBordersCount() {
        return 0;
    }
    function cutBordersConfirmed() {
        return 0;
    }
}
