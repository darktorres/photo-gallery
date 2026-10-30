// Full-window viewer (PLAN §7) and triage (ImageRater's): fit to the window with an antialiased (Lanczos3)
// downscale made in Rust; zoom from fit to 32× around the cursor (wheel, pinch, + −) and pan by dragging. Between
// fit and 1:1 the GPU scales the fitted image at once, and Rust renders the visible part with Lanczos after a
// pause; from 1:1 up the original is shown with nearest neighbour, to judge pixels. A 10× loupe, a bright frame
// around the image to reveal black bars. Rating a picture moves on to the next one.
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtQuick.Window
import pg.gallery

Rectangle {
    id: viewer

    required property var gallery
    required property var settings
    property int index: -1
    property bool loupe: false    // L: 10× loupe under the cursor
    property bool frame: false    // B: bright frame around the image
    property bool zen: false      // H or a tap: nothing over the picture
    readonly property real dpr: Screen.devicePixelRatio

    // The current item, re-read when the gallery changes it in place (a rating, a turn).
    readonly property int rev: gallery.revision
    readonly property string key: rev >= 0 && index >= 0 ? gallery.imageKey(index) : ""
    readonly property int turns: rev >= 0 && index >= 0 ? gallery.pendingTurns(index) : 0
    readonly property int rating: rev >= 0 && index >= 0 ? gallery.rating(index) : 0
    // Upright size in image pixels (pending turns included); 0 before the import, which leaves only the fit.
    readonly property int fw: rev >= 0 && index >= 0 ? gallery.fullWidth(index) : 0
    readonly property int fh: rev >= 0 && index >= 0 ? gallery.fullHeight(index) : 0
    readonly property bool zoomable: fw > 0 && fh > 0

    // The view: `zoom` device pixels per image pixel, the image's top-left corner at (ox, oy) in logical pixels.
    property real zoom: 1
    property real ox: 0
    property real oy: 0
    // At the fit zoom (follows window resizes).
    property bool fitted: true
    readonly property real fitZoom: zoomable ? Math.min(width * dpr / fw, height * dpr / fh, 1) : 1
    readonly property real dw: fw * zoom / dpr
    readonly property real dh: fh * zoom / dpr
    readonly property bool zoomed: zoomable && !fitted
    // 1:1 and beyond: the original, nearest neighbour.
    readonly property bool pixels: zoomable && zoom >= 0.9999

    signal closed(int index)

    color: "black"
    visible: index >= 0
    focus: visible

    function show(i) {
        index = i
        resetView()
        forceActiveFocus()
    }

    function close() {
        const i = index
        index = -1
        closed(i)
    }

    function step(d) {
        const n = Math.max(0, Math.min(gallery.count - 1, index + d))
        if (n !== index) {
            index = n
            resetView()
        }
    }

    // Rate, show the rating's symbol for a moment, and move on (ImageRater's triage).
    function rate(r) {
        if (index < 0)
            return
        gallery.setRating(index, r)
        popup.show(r)
        step(1)
    }

    function turn(d) {
        gallery.rotate(index, d)
        resetView()
    }

    // Keep the image on screen: centred along a side that fits, otherwise no gap at the edges.
    function clampView() {
        ox = dw <= width ? (width - dw) / 2 : Math.min(0, Math.max(width - dw, ox))
        oy = dh <= height ? (height - dh) / 2 : Math.min(0, Math.max(height - dh, oy))
        regionTimer.restart()
    }

    // Zoom to `z` keeping the image point under (px, py) where it is.
    function setZoom(z, px, py) {
        if (!zoomable)
            return
        z = Math.max(fitZoom, Math.min(32, z))
        const u = (px - ox) / dw, v = (py - oy) / dh
        zoom = z
        fitted = z <= fitZoom * 1.0001
        ox = px - u * dw
        oy = py - v * dh
        clampView()
    }

    // How a new image opens: fitted, or 1:1 in Real Size mode (Z).
    function resetView() {
        fitted = !(settings.realSize && zoomable && fitZoom < 1)
        zoom = fitted ? fitZoom : 1
        ox = (width - dw) / 2
        oy = (height - dh) / 2
        clampView()
        region.source = ""
    }

    // Double-click: fit ↔ 1:1 at the point clicked.
    function toggleOneToOne(px, py) {
        if (fitted)
            setZoom(1, px, py)
        else
            setZoom(fitZoom, px, py)
    }

    onWidthChanged: if (fitted) resetView(); else clampView()
    onHeightChanged: if (fitted) resetView(); else clampView()
    onFwChanged: if (fitted) resetView()
    onFhChanged: if (fitted) resetView()

    // The image for item i fitted to the window by Rust (shown 1:1 in device pixels).
    function fitSource(i) {
        if (i < 0 || i >= gallery.count)
            return ""
        return "image://full/" + gallery.imageKey(i) + "/" + Math.round(width * dpr) + "x" + Math.round(height * dpr)
    }

    Keys.onPressed: (e) => {
        const shift = e.modifiers & Qt.ShiftModifier
        if (e.key === Qt.Key_Escape || e.key === Qt.Key_Return || e.key === Qt.Key_Enter) close()
        else if (e.key === Qt.Key_Left || e.key === Qt.Key_Backspace) step(-1)
        else if (e.key === Qt.Key_Right || e.key === Qt.Key_Space) step(1)
        else if (e.key === Qt.Key_Up) rate(4)
        else if (e.key === Qt.Key_Down) rate(1)
        else if (e.key >= Qt.Key_0 && e.key <= Qt.Key_5) rate(e.key - Qt.Key_0)
        else if (e.key === Qt.Key_R) turn(shift ? 1 : -1)
        else if (e.key === Qt.Key_Z) {
            settings.realSize = !settings.realSize
            resetView()
        }
        else if (e.key === Qt.Key_Plus || e.key === Qt.Key_Equal) setZoom(zoom * 1.25, width / 2, height / 2)
        else if (e.key === Qt.Key_Minus) setZoom(zoom / 1.25, width / 2, height / 2)
        else if (e.key === Qt.Key_L) loupe = !loupe
        else if (e.key === Qt.Key_B) frame = !frame
        else if (e.key === Qt.Key_H) zen = !zen
        else return
        e.accepted = true
    }

    WheelHandler {
        id: wheel
        onWheel: (ev) => viewer.setZoom(viewer.zoom * Math.pow(1.25, ev.angleDelta.y / 120),
                                        wheel.point.position.x, wheel.point.position.y)
    }
    PinchHandler {
        id: pinch
        target: null
        property real start: 1
        onActiveChanged: if (active) start = viewer.zoom
        onActiveScaleChanged: viewer.setZoom(start * activeScale, centroid.position.x, centroid.position.y)
    }
    // Mouse: drag pans a zoomed image.
    DragHandler {
        id: mouseDrag
        target: null
        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad | PointerDevice.Stylus
        property point start
        onActiveChanged: if (active) start = Qt.point(viewer.ox, viewer.oy)
        onActiveTranslationChanged: if (active && viewer.zoomed) {
            viewer.ox = start.x + activeTranslation.x
            viewer.oy = start.y + activeTranslation.y
            viewer.clampView()
        }
    }
    // Touch: a zoomed image pans; a fitted one swipes (left/right step, up rates 4, down rates 1).
    DragHandler {
        target: null
        acceptedDevices: PointerDevice.TouchScreen
        property point start
        property bool panning
        onActiveChanged: {
            if (active) {
                start = Qt.point(viewer.ox, viewer.oy)
                panning = viewer.zoomed
                return
            }
            if (panning)
                return
            const t = activeTranslation
            if (Math.abs(t.x) > Math.abs(t.y) && Math.abs(t.x) > 60) viewer.step(t.x < 0 ? 1 : -1)
            else if (Math.abs(t.y) > 60) viewer.rate(t.y < 0 ? 4 : 1)
        }
        onActiveTranslationChanged: if (active && panning) {
            viewer.ox = start.x + activeTranslation.x
            viewer.oy = start.y + activeTranslation.y
            viewer.clampView()
        }
    }
    // A tap shows or hides the overlays (zen); a double tap toggles fit ↔ 1:1.
    TapHandler {
        onTapped: (ev, button) => {
            if (tapCount === 2) {
                zenTimer.stop()
                viewer.toggleOneToOne(point.position.x, point.position.y)
            } else {
                zenTimer.restart()
            }
        }
    }
    Timer {
        id: zenTimer
        interval: Qt.styleHints.mouseDoubleClickInterval
        onTriggered: viewer.zen = !viewer.zen
    }

    // Fitted by Rust. When zoomed, the GPU stretches it over the image's place until a sharper layer is ready.
    Image {
        id: fit
        source: viewer.visible ? viewer.fitSource(viewer.rev >= 0 ? viewer.index : -1) : ""
        width: viewer.zoomed ? viewer.dw : implicitWidth / viewer.dpr
        height: viewer.zoomed ? viewer.dh : implicitHeight / viewer.dpr
        x: viewer.zoomable ? viewer.ox + (viewer.dw - width) / 2 : (viewer.width - width) / 2
        y: viewer.zoomable ? viewer.oy + (viewer.dh - height) / 2 : (viewer.height - height) / 2
        asynchronous: true
        retainWhileLoading: true
        smooth: viewer.zoomed
    }
    // Between fit and 1:1: the visible part, rendered by Rust at the zoom (Lanczos3), after zooming and panning stop.
    // It is placed by the part of the image it shows, so it stays aligned while the view moves on.
    Image {
        id: region
        property var rect: [0, 0, 0, 0]
        property var next: [0, 0, 0, 0]
        onStatusChanged: if (status === Image.Ready) rect = next
        visible: status === Image.Ready && viewer.zoomed && !viewer.pixels
        x: viewer.ox + rect[0] * viewer.dw
        y: viewer.oy + rect[1] * viewer.dh
        width: rect[2] * viewer.dw
        height: rect[3] * viewer.dh
        asynchronous: true
        retainWhileLoading: true
        cache: false
        smooth: true
    }
    Timer {
        id: regionTimer
        interval: 150
        onTriggered: {
            if (!viewer.visible || !viewer.zoomed || viewer.pixels)
                return
            const x0 = Math.max(viewer.ox, 0), y0 = Math.max(viewer.oy, 0)
            const x1 = Math.min(viewer.ox + viewer.dw, viewer.width), y1 = Math.min(viewer.oy + viewer.dh, viewer.height)
            if (x1 <= x0 || y1 <= y0)
                return
            const r = [(x0 - viewer.ox) / viewer.dw, (y0 - viewer.oy) / viewer.dh,
                       (x1 - x0) / viewer.dw, (y1 - y0) / viewer.dh].map((v) => Number(v.toFixed(6)))
            region.next = r
            region.source = "image://full/" + viewer.key + "/region/" + r.join(",") + "/"
                + Math.round((x1 - x0) * viewer.dpr) + "x" + Math.round((y1 - y0) * viewer.dpr)
        }
    }
    // 1:1 and beyond (and the loupe's source): the original, nearest neighbour.
    Image {
        id: full
        source: viewer.visible && viewer.key.length && (viewer.pixels || viewer.loupe)
                ? "image://full/" + viewer.key + "/full" : ""
        visible: viewer.pixels && status === Image.Ready
        x: viewer.ox
        y: viewer.oy
        width: viewer.dw
        height: viewer.dh
        asynchronous: true
        smooth: false
    }

    // Neighbours, preloaded so stepping is instant (same URLs, so the pixmap cache serves them).
    Repeater {
        model: viewer.visible ? [-4, -3, -2, -1, 1, 2, 3, 4] : []
        Image {
            required property int modelData
            visible: false
            asynchronous: true
            source: viewer.fitSource(viewer.rev >= 0 ? viewer.index + modelData : -1)
        }
    }

    // Frame: a 1 px magenta line just outside the image, so black bars inside it stand out.
    Rectangle {
        visible: viewer.frame && fit.status === Image.Ready
        x: (viewer.zoomed ? viewer.ox : fit.x) - 1
        y: (viewer.zoomed ? viewer.oy : fit.y) - 1
        width: (viewer.zoomed ? viewer.dw : fit.width) + 2
        height: (viewer.zoomed ? viewer.dh : fit.height) + 2
        color: "transparent"
        border.color: "#ff00ff"
        border.width: 1
    }

    HoverHandler { id: hover }

    // Loupe: the full-resolution image at 10× (nearest neighbour), centred on the point under the cursor.
    Rectangle {
        id: loupeBox
        readonly property real u: (hover.point.position.x - fit.x) / Math.max(1, fit.width)
        readonly property real v: (hover.point.position.y - fit.y) / Math.max(1, fit.height)
        visible: viewer.loupe && !viewer.zoomed && hover.hovered && full.status === Image.Ready
                 && u >= 0 && u <= 1 && v >= 0 && v <= 1
        width: 400
        height: 400
        x: hover.point.position.x - width / 2
        y: hover.point.position.y - height / 2
        color: "black"
        clip: true
        Image {
            source: full.source
            width: full.implicitWidth * 10 / viewer.dpr
            height: full.implicitHeight * 10 / viewer.dpr
            x: loupeBox.width / 2 - loupeBox.u * width
            y: loupeBox.height / 2 - loupeBox.v * height
            asynchronous: true
            smooth: false
        }
        Rectangle {
            anchors.fill: parent
            color: "transparent"
            border.color: "white"
            border.width: 1
        }
    }

    // Everything over the picture; hidden in zen mode.
    Item {
        anchors.fill: parent
        visible: !viewer.zen

        Label {
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.margins: 10
            color: "#999999"
            style: Text.Outline
            styleColor: "black"
            text: (viewer.index + 1) + " / " + viewer.gallery.count
        }
        Label {
            anchors.right: closeButton.left
            anchors.verticalCenter: closeButton.verticalCenter
            anchors.rightMargin: 10
            color: "#999999"
            style: Text.Outline
            styleColor: "black"
            text: "← → step · 0–5 rate · ↑ 4 · ↓ 1 · R ⟲ · ⇧R ⟳ · wheel zoom · double-click 1:1 · Z real size"
                + " · L loupe · B frame · H hide · Esc close"
        }
        ToolButton {
            id: closeButton
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 4
            text: "✕"
            onClicked: viewer.close()
        }

        Label {
            anchors.left: parent.left
            anchors.right: bar.left
            anchors.verticalCenter: bar.verticalCenter
            anchors.margins: 10
            elide: Text.ElideMiddle
            color: "#cccccc"
            style: Text.Outline
            styleColor: "black"
            text: (viewer.rev >= 0 && viewer.index >= 0 ? viewer.gallery.info(viewer.index) : "")
                + (viewer.rating > 0 ? "   " + Ratings.symbol(viewer.rating) : "")
                + (viewer.zoomable ? "   " + Math.round(viewer.zoom * 100) + "%" : "")
                + (viewer.settings.realSize ? "   [real size]" : "")
                + (viewer.loupe ? "   [loupe 10×]" : "")
                + (viewer.turns !== 0 ? "   — turning…" : "")
                + (fit.status === Image.Error ? "   — original unavailable" : "")
        }

        // Rating bar: ImageRater's symbols, the current rating ringed; clicking one rates and moves on.
        Rectangle {
            id: bar
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 12
            width: barRow.implicitWidth + 16
            height: barRow.implicitHeight + 8
            radius: height / 2
            color: "#a0000000"
            RowLayout {
                id: barRow
                anchors.centerIn: parent
                spacing: 4
                ToolButton {
                    text: "⟲"
                    font.pixelSize: 22
                    ToolTip.visible: hovered
                    ToolTip.text: "Turn left (R)"
                    onClicked: viewer.turn(-1)
                }
                Repeater {
                    model: 6
                    ToolButton {
                        required property int index
                        text: Ratings.symbol(index)
                        font.pixelSize: 22
                        ToolTip.visible: hovered
                        ToolTip.text: index === 0 ? "Unrated (0)" : index + (index === 1 ? " star" : " stars") + " (" + index + ")"
                        background: Rectangle {
                            radius: width / 2
                            color: parent.hovered ? "#40ffffff" : "transparent"
                            border.color: "#ffd24a"
                            border.width: viewer.rating === parent.index ? 2 : 0
                        }
                        onClicked: viewer.rate(index)
                    }
                }
                ToolButton {
                    text: "⟳"
                    font.pixelSize: 22
                    ToolTip.visible: hovered
                    ToolTip.text: "Turn right (Shift+R)"
                    onClicked: viewer.turn(1)
                }
            }
        }
    }

    // The rating just given, for half a second (over the next picture).
    Label {
        id: popup
        anchors.centerIn: parent
        opacity: 0
        font.pixelSize: 120
        style: Text.Outline
        styleColor: "black"
        function show(r) {
            text = Ratings.symbol(r)
            fade.restart()
        }
        SequentialAnimation {
            id: fade
            PropertyAction { target: popup; property: "opacity"; value: 1 }
            PauseAnimation { duration: 350 }
            NumberAnimation { target: popup; property: "opacity"; to: 0; duration: 150 }
        }
    }
}
