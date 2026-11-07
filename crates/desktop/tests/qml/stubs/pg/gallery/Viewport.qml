// A stub of the Viewport facade (pg_client's View behind it): the zoom and the pan as the viewer asks them, with the
// window's and the media's sizes taken as given. The real one clamps the view to the window.
import QtQuick

QtObject {
    property real zoom: 1
    property real ox: 0
    property real oy: 0
    property bool fitted: true
    property real fitZoom: 1
    property real minZoom: 1
    property real dw: 0
    property real dh: 0
    property bool zoomed: false
    property bool pixels: false

    function setStill(still) { return true }
    function setMedia(w, h, realSize) { return true }
    function setWindow(w, h, dpr, realSize) { return true }
    function setZoom(z, px, py) { zoom = z; zoomed = z > fitZoom }
    function pan(x, y) { ox = x; oy = y }
    function resetView(realSize) { zoom = fitZoom; zoomed = false }
    function toggleOneToOne(px, py) { zoom = zoomed ? fitZoom : 1; zoomed = !zoomed }
}
