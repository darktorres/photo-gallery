// The picture's layers, under the video and the overlays: the fitted image (fit), the visible part rendered at the zoom
// between fit and 1:1 (region), and the original, nearest neighbour, at 1:1 and beyond and for the loupe (full). The
// viewer preloads the neighbours, so stepping is instant (same URLs, so the pixmap cache serves them). The view's numbers
// come from the Viewport facade (`viewport`, its numbers only); the URLs and the region are made by the viewer, which
// has the Viewport and the gallery by their ids (Viewer.qml).
pragma ComponentBehavior: Bound

import QtQuick
import pg.gallery

Item {
    id: layers

    anchors.fill: parent

    required property Viewport viewport
    // The URLs, made by the viewer: the fitted image and the original.
    required property string fitSource
    required property string fullUrl
    // The viewer is shown (its index is 0 or more), and the shown item's key.
    required property bool showing
    required property string key
    required property bool still
    required property bool zoomable
    required property bool loupe
    // A video or animation has drawn its first frame: the poster gives way.
    required property bool videoStarted
    required property real dpr

    readonly property alias fitImage: fit
    // The parts' values over the picture (the Loupe, the frame, the Hud), read from the images.
    readonly property int fitStatus: fit.status
    readonly property bool fullReady: full.status === Image.Ready
    readonly property url fullSource: full.source
    readonly property real fullWidth: full.implicitWidth
    readonly property real fullHeight: full.implicitHeight

    // The view wants the visible part rendered (showRegion), now.
    signal regionDue

    // The view moved: render the visible part at once, and again at most every 50 ms while it keeps moving (a move in
    // between is rendered when the window is over). A new image opened: drop the old part (dropRegion).
    property bool regionPending: false

    function restartRegion() {
        if (regionTimer.running) {
            regionPending = true;
            return;
        }
        layers.regionDue();
        regionTimer.start();
    }

    // The region as Rust made it: its URL and the part of the image it shows (fractions), placed when it is ready.
    function showRegion(source: string, next: rect) {
        // Before the source: a cached region is ready at once.
        region.next = next;
        region.source = source;
    }

    function dropRegion() {
        region.held = false;
        region.source = "";
    }

    // Fitted by Rust. When zoomed, the GPU stretches it over the image's place until a sharper layer is ready.
    Image {
        id: fit
        objectName: "fitImage"
        source: layers.fitSource
        width: layers.viewport.zoomed ? layers.viewport.dw : implicitWidth / layers.dpr
        height: layers.viewport.zoomed ? layers.viewport.dh : implicitHeight / layers.dpr
        x: layers.zoomable ? layers.viewport.ox + (layers.viewport.dw - width) / 2 : (layers.width - width) / 2
        y: layers.zoomable ? layers.viewport.oy + (layers.viewport.dh - height) / 2 : (layers.height - height) / 2
        asynchronous: true
        retainWhileLoading: true
        smooth: layers.viewport.zoomed
        visible: layers.still || !layers.videoStarted
    }

    // Between fit and 1:1: the visible part, rendered by Rust at the zoom (Lanczos3) as the view moves. It is placed by
    // the part of the image it shows, so it stays aligned while the view moves on. Once shown it stays until the next
    // part is ready (retainWhileLoading keeps the pixels), so the fit is never stretched in its place again; at 1:1 and
    // beyond the original takes over when it has loaded.
    Image {
        id: region
        // The part of the image shown and the one being rendered, as fractions of the image.
        property rect shown: Qt.rect(0, 0, 0, 0)
        property rect next: Qt.rect(0, 0, 0, 0)
        // A part has been shown for the picture (cleared by dropRegion).
        property bool held: false
        onStatusChanged: if (status === Image.Ready) {
            shown = next;
            held = true;
        }
        visible: held && layers.viewport.zoomed && !(layers.viewport.pixels && layers.fullReady)
        x: layers.viewport.ox + shown.x * layers.viewport.dw
        y: layers.viewport.oy + shown.y * layers.viewport.dh
        width: shown.width * layers.viewport.dw
        height: shown.height * layers.viewport.dh
        asynchronous: true
        retainWhileLoading: true
        cache: false
        smooth: true
    }

    // Throttles the region's renders while the view moves: the first move renders at once, later moves at most every 50 ms.
    Timer {
        id: regionTimer
        interval: 50
        onTriggered: if (layers.regionPending) {
            layers.regionPending = false;
            layers.regionDue();
            start();
        }
    }

    // 1:1 and beyond (and the loupe's source): the original, nearest neighbour.
    Image {
        id: full
        source: layers.showing && layers.still && layers.key.length && (layers.viewport.pixels || layers.loupe) ? layers.fullUrl : ""
        visible: layers.viewport.pixels && status === Image.Ready
        x: layers.viewport.ox
        y: layers.viewport.oy
        width: layers.viewport.dw
        height: layers.viewport.dh
        asynchronous: true
        smooth: false
    }
}
