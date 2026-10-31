// Full-window viewer (PLAN §7) and triage (ImageRater's): fit to the window with an antialiased (Lanczos3)
// downscale made in Rust; zoom from fit to 32× around the cursor (wheel, pinch, + −) and pan by dragging. Between
// fit and 1:1 the GPU scales the fitted image at once, and Rust renders the visible part with Lanczos after a
// pause; from 1:1 up the original is shown with nearest neighbour, to judge pixels. A 10× loupe, a bright frame
// around the image to reveal black bars. Rating a picture moves on to the next one. Videos play with Qt Multimedia
// on our FFmpeg; animated GIF, APNG and WebP with our AnimationPlayer (src/animation.rs), into the same
// VideoOutput. Both are fitted and never enlarged; the poster or first frame shows until the first frame played.
//
// The gallery's items are the window's (Main.qml): it gives the one shown, by the properties below, and does the
// ratings and turns asked for (rated(), turned()). The QML compiler compiles a call into the gallery model to C++
// only where the model is named by its id, in the window. (Reading and writing a final property through another
// object compiles: Settings.realSize.)
pragma ComponentBehavior: Bound

import QtMultimedia
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtQuick.Window
import pg.gallery

Rectangle {
    id: viewer

    // The item shown (-1: none, and the viewer is closed), of `count`: the gallery's (it follows the file when the
    // list is refreshed). Showing another asks the window (`moved`).
    required property int index
    required property int count
    // The shown item, from the gallery (which changes them in place, at a rating or a turn).
    // Its image's key (image://full/<key>/…); the keys of the items around it, preloaded.
    required property string key
    required property list<string> neighbourKeys
    // Quarter turns asked for but not yet in the file; its rating.
    required property int turns
    required property int rating
    // Upright size in image pixels (pending turns included); 0 before the import, which leaves only the fit.
    required property int fw
    required property int fh
    // image, animated or video (query::MediaType). Only still images zoom.
    required property string kind
    // Videos and animations: Qt's players show the file as stored, so the catalog's turns are added here.
    required property int playerTurns
    // The file, for the players; a line about it for the bar.
    required property url fileUrl
    required property string filePath
    required property string info
    // Real Size (Z): open images at 1:1 instead of fitted.
    required property Settings settings

    property bool loupe: false    // L: 10× loupe under the cursor
    property bool frame: false    // B: bright frame around the image
    property bool zen: false      // H or a tap: nothing over the picture
    readonly property real dpr: Screen.devicePixelRatio

    readonly property bool still: kind === "image"
    readonly property bool zoomable: fw > 0 && fh > 0 && still
    readonly property bool playsVideo: visible && kind === "video"
    readonly property bool playsAnimation: visible && kind === "animated"
    readonly property real playScale: fw > 0 && fh > 0 ? Math.min(width / fw, height / fh, 1 / dpr) : 0
    // Sound, for the session.
    property bool muted: false
    property real volume: 1

    // Show another item (-1: close), rate the shown one, turn it by quarter turns (1 = clockwise): the window does it.
    signal moved(int index)
    signal rated(int index, int rating)
    signal turned(int index, int turns)
    // The bar's "find similar" button: the photo's file, for the Yandex panel (Main.qml).
    signal findSimilar(string path)

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

    function show(i: int) {
        moved(i)
        resetView()
        forceActiveFocus()
    }

    // The item it was on first, while the list is still the one it showed: closing may show a list that waited.
    function close() {
        closed(index)
        moved(-1)
    }

    function step(d: int) {
        const n = Math.max(0, Math.min(count - 1, index + d))
        if (n !== index) {
            moved(n)
            resetView()
        }
    }

    // Rate, show the rating's symbol for a moment, and move on (ImageRater's triage).
    function rate(r: int) {
        if (index < 0)
            return
        rated(index, r)
        popup.show(r)
        step(1)
    }

    function turn(d: int) {
        turned(index, d)
        resetView()
    }

    function togglePlay() {
        if (kind === "animated")
            animPlayer.playing = !animPlayer.playing
        else if (player.playbackState === MediaPlayer.PlayingState)
            player.pause()
        else
            player.play()
    }

    // A playback position: m:ss, or h:mm:ss from an hour.
    function time(ms: real): string {
        const s = Math.floor(ms / 1000)
        return s >= 3600 ? Math.floor(s / 3600) + ":" + twoDigits(Math.floor(s / 60) % 60) + ":" + twoDigits(s % 60)
                         : Math.floor(s / 60) + ":" + twoDigits(s % 60)
    }
    function twoDigits(n: int): string { return (n < 10 ? "0" : "") + n }

    // Keep the image on screen: centred along a side that fits, otherwise no gap at the edges.
    function clampView() {
        ox = dw <= width ? (width - dw) / 2 : Math.min(0, Math.max(width - dw, ox))
        oy = dh <= height ? (height - dh) / 2 : Math.min(0, Math.max(height - dh, oy))
        regionTimer.restart()
    }

    // Zoom to `z` keeping the image point under (px, py) where it is.
    function setZoom(z: real, px: real, py: real) {
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
    function toggleOneToOne(px: real, py: real) {
        if (fitted)
            setZoom(1, px, py)
        else
            setZoom(fitZoom, px, py)
    }

    onWidthChanged: if (fitted) resetView(); else clampView()
    onHeightChanged: if (fitted) resetView(); else clampView()
    onFwChanged: if (fitted) resetView()
    onFhChanged: if (fitted) resetView()

    // An image (by its key) fitted to the window by Rust (shown 1:1 in device pixels).
    function fitSource(imageKey: string): string {
        if (imageKey === "")
            return ""
        return "image://full/" + imageKey + "/" + Math.round(width * dpr) + "x" + Math.round(height * dpr)
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
        else if (e.key === Qt.Key_K && !still) togglePlay()
        else if (e.key === Qt.Key_M && kind === "video") muted = !muted
        else return
        e.accepted = true
    }

    WheelHandler {
        id: wheelZoom
        onWheel: (ev) => viewer.setZoom(viewer.zoom * Math.pow(1.25, ev.angleDelta.y / 120),
                                        wheelZoom.point.position.x, wheelZoom.point.position.y)
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
        interval: Application.styleHints.mouseDoubleClickInterval
        onTriggered: viewer.zen = !viewer.zen
    }

    // Fitted by Rust. When zoomed, the GPU stretches it over the image's place until a sharper layer is ready.
    Image {
        id: fit
        source: viewer.visible ? viewer.fitSource(viewer.key) : ""
        width: viewer.zoomed ? viewer.dw : implicitWidth / viewer.dpr
        height: viewer.zoomed ? viewer.dh : implicitHeight / viewer.dpr
        x: viewer.zoomable ? viewer.ox + (viewer.dw - width) / 2 : (viewer.width - width) / 2
        y: viewer.zoomable ? viewer.oy + (viewer.dh - height) / 2 : (viewer.height - height) / 2
        asynchronous: true
        retainWhileLoading: true
        smooth: viewer.zoomed
        visible: viewer.still || !videoOut.started
    }
    // Between fit and 1:1: the visible part, rendered by Rust at the zoom (Lanczos3), after zooming and panning stop.
    // It is placed by the part of the image it shows, so it stays aligned while the view moves on.
    Image {
        id: region
        // The part of the image shown and the one being rendered, as fractions of the image.
        property rect shown: Qt.rect(0, 0, 0, 0)
        property rect next: Qt.rect(0, 0, 0, 0)
        onStatusChanged: if (status === Image.Ready) shown = next
        visible: status === Image.Ready && viewer.zoomed && !viewer.pixels
        x: viewer.ox + shown.x * viewer.dw
        y: viewer.oy + shown.y * viewer.dh
        width: shown.width * viewer.dw
        height: shown.height * viewer.dh
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
            const rx = viewer.sixPlaces((x0 - viewer.ox) / viewer.dw), ry = viewer.sixPlaces((y0 - viewer.oy) / viewer.dh)
            const rw = viewer.sixPlaces((x1 - x0) / viewer.dw), rh = viewer.sixPlaces((y1 - y0) / viewer.dh)
            const source = "image://full/" + viewer.key + "/region/" + rx + "," + ry + "," + rw + "," + rh
                + "/" + Math.round((x1 - x0) * viewer.dpr) + "x" + Math.round((y1 - y0) * viewer.dpr)
            // Before the source: a cached region is ready at once.
            region.next = Qt.rect(rx, ry, rw, rh)
            region.source = source
        }
    }
    // A fraction of the image, to six places: the region's URL names it, so a pixel of panning doesn't make a new one.
    function sixPlaces(v: real): real { return Math.round(v * 1e6) / 1e6 }
    // 1:1 and beyond (and the loupe's source): the original, nearest neighbour.
    Image {
        id: full
        source: viewer.visible && viewer.still && viewer.key.length && (viewer.pixels || viewer.loupe)
                ? "image://full/" + viewer.key + "/full" : ""
        visible: viewer.pixels && status === Image.Ready
        x: viewer.ox
        y: viewer.oy
        width: viewer.dw
        height: viewer.dh
        asynchronous: true
        smooth: false
    }

    // Videos and animations, fitted (never enlarged), turned by the catalog's turns.
    Item {
        id: playArea
        visible: !viewer.still && viewer.playScale > 0
        width: viewer.fw * viewer.playScale
        height: viewer.fh * viewer.playScale
        anchors.centerIn: parent
        Item {
            anchors.centerIn: parent
            width: viewer.playerTurns % 2 ? parent.height : parent.width
            height: viewer.playerTurns % 2 ? parent.width : parent.height
            rotation: viewer.playerTurns * 90
            VideoOutput {
                id: videoOut
                // Set by the first frame, so the poster stays until then.
                property bool started: false
                anchors.fill: parent
                fillMode: VideoOutput.PreserveAspectFit
            }
        }
    }
    MediaPlayer {
        id: player
        source: viewer.playsVideo ? viewer.fileUrl : Qt.url("")
        videoOutput: videoOut
        audioOutput: AudioOutput {
            muted: viewer.muted
            volume: viewer.volume
        }
        loops: MediaPlayer.Infinite
        onSourceChanged: {
            videoOut.started = false
            if (viewer.playsVideo)
                play()
        }
    }
    AnimationPlayer {
        id: animPlayer
        source: viewer.playsAnimation ? viewer.filePath : ""
        onSourceChanged: videoOut.started = false
        Component.onCompleted: setSink(videoOut.videoSink)
    }
    Connections {
        target: videoOut.videoSink
        function onVideoFrameChanged() {
            videoOut.started = true
        }
    }

    // Neighbours, preloaded so stepping is instant (same URLs, so the pixmap cache serves them).
    Repeater {
        model: viewer.neighbourKeys
        Image {
            required property string modelData
            visible: false
            asynchronous: true
            source: viewer.fitSource(modelData)
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
        visible: viewer.loupe && viewer.still && !viewer.zoomed && hover.hovered && full.status === Image.Ready
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
            text: (viewer.index + 1) + " / " + viewer.count
        }
        Label {
            anchors.right: closeButton.left
            anchors.verticalCenter: closeButton.verticalCenter
            anchors.rightMargin: 10
            color: "#999999"
            style: Text.Outline
            styleColor: "black"
            text: viewer.kind === "video"
                ? "← → step · K play/pause · M mute · 0–5 rate · ↑ 4 · ↓ 1 · R ⟲ · ⇧R ⟳ · H hide · Esc close"
                : viewer.kind === "animated"
                ? "← → step · K play/pause · 0–5 rate · ↑ 4 · ↓ 1 · R ⟲ · ⇧R ⟳ · H hide · Esc close"
                : "← → step · 0–5 rate · ↑ 4 · ↓ 1 · R ⟲ · ⇧R ⟳ · wheel zoom · double-click 1:1 · Z real size"
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
            text: viewer.info
                + (viewer.rating > 0 ? "   " + Ratings.symbol(viewer.rating) : "")
                + (viewer.zoomable ? "   " + Math.round(viewer.zoom * 100) + "%" : "")
                + (viewer.settings.realSize ? "   [real size]" : "")
                + (viewer.loupe ? "   [loupe 10×]" : "")
                + (viewer.turns !== 0 ? "   — turning…" : "")
                + (fit.status === Image.Error ? "   — original unavailable" : "")
                + (viewer.kind === "video" && player.errorString !== "" ? "   — " + player.errorString : "")
        }

        // Video transport: play/pause, position (drag to seek), sound.
        Rectangle {
            id: transport
            visible: viewer.kind === "video"
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: bar.top
            anchors.bottomMargin: 8
            width: Math.min(parent.width - 40, 900)
            height: transportRow.implicitHeight + 8
            radius: 8
            color: "#a0000000"
            RowLayout {
                id: transportRow
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                spacing: 8
                ToolButton {
                    text: player.playbackState === MediaPlayer.PlayingState ? "⏸" : "▶"
                    font.pixelSize: 18
                    onClicked: viewer.togglePlay()
                    ToolTip.visible: hovered
                    ToolTip.text: "Play / pause (K)"
                }
                Label { text: viewer.time(player.position); color: "#cccccc" }
                Slider {
                    id: seek
                    Layout.fillWidth: true
                    from: 0
                    to: Math.max(1, player.duration)
                    enabled: player.seekable
                    onMoved: player.position = value
                    // Follows the playback except while dragged.
                    Binding on value {
                        when: !seek.pressed
                        value: player.position
                    }
                }
                Label { text: viewer.time(player.duration); color: "#cccccc" }
                ToolButton {
                    text: viewer.muted || viewer.volume === 0 ? "🔇" : "🔊"
                    font.pixelSize: 16
                    onClicked: viewer.muted = !viewer.muted
                    ToolTip.visible: hovered
                    ToolTip.text: "Mute (M)"
                }
                Slider {
                    from: 0
                    to: 1
                    value: viewer.volume
                    onMoved: viewer.volume = value
                    Layout.preferredWidth: 100
                }
            }
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
                    text: "≈"
                    visible: viewer.still
                    font.pixelSize: 22
                    ToolTip.visible: hovered
                    ToolTip.text: "Find similar images on Yandex"
                    onClicked: viewer.findSimilar(viewer.filePath)
                }
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
                        id: star
                        required property int index
                        text: Ratings.symbol(index)
                        font.pixelSize: 22
                        ToolTip.visible: hovered
                        ToolTip.text: index === 0 ? "Unrated (0)" : index + (index === 1 ? " star" : " stars") + " (" + index + ")"
                        background: Rectangle {
                            radius: width / 2
                            color: star.hovered ? "#40ffffff" : "transparent"
                            border.color: "#ffd24a"
                            border.width: viewer.rating === star.index ? 2 : 0
                        }
                        onClicked: viewer.rate(star.index)
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
        function show(r: int) {
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
