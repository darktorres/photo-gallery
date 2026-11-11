// A stub of the Viewport facade (pg_client's View behind it): the zoom and the pan as the viewer asks them, with the
// window's and the media's sizes taken as given. The real one clamps the view to the window.
import QtQuick

QtObject {
    property real zoom: 1
    property real ox: 0
    property real oy: 0
    property real dw: 0
    property real dh: 0
    property bool zoomed: false
    property bool pixels: false
    // The window's size in device pixels, as setWindow gives it (the real one rounds the same way).
    property int fitWidth: 0
    property int fitHeight: 0
    // The stub's own fit zoom, which its zoom functions compare with: not a property of the real Viewport, which keeps
    // its fit zoom to itself.
    readonly property real stubFitZoom: 1

    function setStill(still) { return true }
    function setMedia(w, h, realSize) { return true }
    function setWindow(w, h, dpr, realSize) {
        fitWidth = Math.round(w * dpr)
        fitHeight = Math.round(h * dpr)
        return true
    }
    function setZoom(z, px, py) { zoom = z; zoomed = z > stubFitZoom }
    function pan(x, y) { ox = x; oy = y }
    function resetView(realSize) { zoom = stubFitZoom; zoomed = false }
    function toggleOneToOne(px, py) { zoom = zoomed ? stubFitZoom : 1; zoomed = !zoomed }
    // The fitted image's URL, with the size it is rendered at (the real one: pg_client::image); "" for no key.
    function fitSource(key, w, h) { return key.length ? "image://full/" + key + "/fit/" + w + "x" + h : "" }
}
