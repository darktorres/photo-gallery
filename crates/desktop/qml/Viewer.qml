// Full-window viewer (PLAN §7) and triage (ImageRater's): fit to the window, up or down, with an antialiased
// (Lanczos3) resample made in Rust; zoom from fit to 32× around the cursor (pinch, + −) and pan by dragging; the wheel
// steps to the previous or next picture. Between
// fit and 1:1 the GPU scales the fitted image at once, and Rust renders the visible part with Lanczos after a
// pause; from 1:1 up the original is shown with nearest neighbour, to judge pixels. A 10× loupe; borders (B): a
// bright frame around the image to reveal black bars, and over it what cropping the borders found would cut, with
// N to mark them as not a border. Rating a picture moves on to the next one. Videos play with Qt Multimedia
// on our FFmpeg; animated GIF, APNG and WebP with our AnimationPlayer (src/animation.rs), into the same
// VideoOutput. Both are fitted to the window, enlarged when they are smaller; the poster or first frame shows until
// the first frame played.
//
// The gallery's items are the window's (Main.qml): it gives the one shown, by the properties below, and does the
// ratings, turns and border marks asked for (rated(), turned(), borderMarkToggled()). The QML compiler compiles a
// call into the gallery model to C++ only where the model is named by its id, in the window.
// (Reading and writing a final property through another
// object compiles: Settings.realSize.)
//
// What it shows is made when it is first opened (the content Loader), and the players with the first video or
// animation (the media Loader, inside it): until then the window has neither, nor Qt Multimedia's audio output,
// whose device probe runs on threads of its own. Both stay once made. The root keeps the state and the API; it
// reaches what the Loaders made only through its signals (viewMoved, viewReset, playToggled, rated), and they
// report back through its properties (videoStarted, playerError).
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
    // Its borders: what is left without them, as fractions of the image as shown (empty: none found); where a
    // rejected candidate would have cut at each edge (a fraction from that edge, -1: none); whether they are marked as
    // not a border. Whether to show them (B, the window's).
    required property rect borderBox
    required property real rejectedTop
    required property real rejectedBottom
    required property real rejectedLeft
    required property real rejectedRight
    required property bool borderMarked
    required property bool showBorders
    // Real Size (Z): open images at 1:1 instead of fitted.
    required property Settings settings

    property bool loupe: false    // L: 10× loupe under the cursor
    property bool zen: false      // H or a tap: nothing over the picture
    readonly property real dpr: Screen.devicePixelRatio

    readonly property bool still: kind === "image"
    readonly property bool hasBorders: still && borderBox.width > 0
    readonly property bool zoomable: fw > 0 && fh > 0 && still
    readonly property bool playsVideo: visible && kind === "video"
    readonly property bool playsAnimation: visible && kind === "animated"
    readonly property real playScale: fw > 0 && fh > 0 ? Math.min(width / fw, height / fh) : 0
    // Sound, for the session.
    property bool muted: false
    property real volume: 1
    // Made once needed, then kept: the content at the first show, the players at the first video or animation.
    property bool opened: false
    property bool mediaOpened: false
    onVisibleChanged: if (visible) opened = true
    onPlaysVideoChanged: if (playsVideo) mediaOpened = true
    onPlaysAnimationChanged: if (playsAnimation) mediaOpened = true
    // From the players: a frame of the shown video or animation was drawn (the poster gives way); the player's error.
    property bool videoStarted: false
    property string playerError: ""

    // To what the Loaders made: the view moved (render the visible part again, after a pause), a new image opened
    // (drop the old part), play or pause asked for.
    signal viewMoved()
    signal viewReset()
    signal playToggled()

    // Show another item (-1: close), rate the shown one, turn it by quarter turns (1 = clockwise): the window does it.
    signal moved(int index)
    signal rated(int index, int rating)
    signal turned(int index, int turns)
    // B: show or hide the borders (the window keeps the setting); N: mark the shown item's as not a border, or not.
    signal bordersToggled()
    signal borderMarkToggled(int index, bool marked)
    // The bar's "find similar" button: the photo's file, for the Yandex panel (Main.qml).
    signal findSimilar(string path)

    // The view (Rust: pg_client's View, behind the Viewport object): `zoom` device pixels per image pixel, the image's
    // top-left corner at (ox, oy) in logical pixels, fitted to the window or not. The numbers are the Viewport's; this
    // file makes the gestures and the signals.
    readonly property real zoom: viewport.zoom
    readonly property real ox: viewport.ox
    readonly property real oy: viewport.oy
    // At the fit zoom (follows window resizes): above 1 for an image smaller than the window, which is enlarged to it.
    readonly property bool fitted: viewport.fitted
    readonly property real fitZoom: viewport.fitZoom
    // The smallest zoom: the fit, or 1:1 when the fit enlarges the image (1:1 is then the furthest zoom out).
    readonly property real minZoom: viewport.minZoom
    readonly property real dw: viewport.dw
    readonly property real dh: viewport.dh
    readonly property bool zoomed: viewport.zoomed
    // 1:1 and beyond, away from the fit: the original, nearest neighbour.
    readonly property bool pixels: viewport.pixels

    Viewport {
        id: viewport
    }

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

    // Rate, show the rating's symbol for a moment (the content, on `rated`), and move on (ImageRater's triage).
    function rate(r: int) {
        if (index < 0)
            return
        rated(index, r)
        step(1)
    }

    function turn(d: int) {
        turned(index, d)
        resetView()
    }

    function togglePlay() {
        playToggled()
    }

    // A playback position: m:ss, or h:mm:ss from an hour.
    function time(ms: real): string {
        return words.timecode(Math.floor(ms / 1000))
    }

    // Zoom to `z` keeping the image point under (px, py) where it is (the view limits it).
    function setZoom(z: real, px: real, py: real) {
        if (!zoomable)
            return
        viewport.setZoom(z, px, py)
        viewMoved()
    }

    // Pan the image's corner (a drag or a swipe), kept on screen.
    function pan(x: real, y: real) {
        viewport.pan(x, y)
        viewMoved()
    }

    // How a new image opens: fitted, or 1:1 in Real Size mode (Z), which is the fit when the two are the same.
    function resetView() {
        viewport.resetView(settings.realSize)
        viewMoved()
        viewReset()
    }

    // Double-click: fit ↔ 1:1 at the point clicked.
    function toggleOneToOne(px: real, py: real) {
        if (!zoomable)
            return
        viewport.toggleOneToOne(px, py)
        viewMoved()
    }

    // The window's size changed: a fitted view follows it (reopened fitted), another is kept and clamped.
    function pushWindow() {
        const reset = viewport.setWindow(width, height, dpr, settings.realSize)
        viewMoved()
        if (reset)
            viewReset()
    }

    // The image's size changed: a fitted view opens it again, fitted (or 1:1 in Real Size mode).
    function pushMedia() {
        if (viewport.setMedia(fw, fh, settings.realSize)) {
            viewMoved()
            viewReset()
        }
    }

    onWidthChanged: pushWindow()
    onHeightChanged: pushWindow()
    onFwChanged: pushMedia()
    onFhChanged: pushMedia()
    onStillChanged: viewport.setStill(still)

    // The view starts from the values the properties have when the viewer is made, without the signals.
    Component.onCompleted: {
        viewport.setStill(still)
        viewport.setMedia(fw, fh, settings.realSize)
        viewport.setWindow(width, height, dpr, settings.realSize)
    }

    // An image (by its key) fitted to the window by Rust (shown 1:1 in device pixels).
    function fitSource(imageKey: string): string {
        return viewport.fitSource(imageKey)
    }

    // The key map (pg-client's keys, through the KeyMap facade) names the viewer's action for a key; this carries it out.
    KeyMap {
        id: keymap
    }

    Words {
        id: words
    }

    // The viewer's actions, as the codes keymap.viewerAction returns (the Rust side lists them in the same order).
    enum ViewerAction { Close, StepBack, StepForward, Rate, TurnForward, TurnBack, RealSize, ZoomIn, ZoomOut, Loupe, Borders, MarkBorder, Hide, PlayPause, Mute }

    Keys.onPressed: (e) => {
        const action = keymap.viewerAction(e.key, e.modifiers, kind, hasBorders)
        if (action < 0)
            return
        const arg = keymap.viewerArg(e.key, e.modifiers, kind, hasBorders)
        switch (action) {
        case Viewer.ViewerAction.Close: close(); break
        case Viewer.ViewerAction.StepBack: step(-1); break
        case Viewer.ViewerAction.StepForward: step(1); break
        case Viewer.ViewerAction.Rate: rate(arg); break
        case Viewer.ViewerAction.TurnForward: turn(1); break
        case Viewer.ViewerAction.TurnBack: turn(-1); break
        case Viewer.ViewerAction.RealSize: settings.realSize = !settings.realSize; resetView(); break
        case Viewer.ViewerAction.ZoomIn: setZoom(zoom * 1.25, width / 2, height / 2); break
        case Viewer.ViewerAction.ZoomOut: setZoom(zoom / 1.25, width / 2, height / 2); break
        case Viewer.ViewerAction.Loupe: loupe = !loupe; break
        case Viewer.ViewerAction.Borders: bordersToggled(); break
        case Viewer.ViewerAction.MarkBorder: borderMarkToggled(index, !borderMarked); break
        case Viewer.ViewerAction.Hide: zen = !zen; break
        case Viewer.ViewerAction.PlayPause: togglePlay(); break
        case Viewer.ViewerAction.Mute: muted = !muted; break
        }
        e.accepted = true
    }

    // What it shows: made at the first show (`opened`), then kept.
    Loader {
        anchors.fill: parent
        active: viewer.opened
        sourceComponent: Component {
            Item {
                id: content

                // The wheel steps through the pictures: a notch (120 units) is one step, up for the previous. A
                // touchpad's small deltas add up until they make a notch.
                WheelHandler {
                    id: wheelStep
                    property real notches: 0
                    onWheel: (ev) => {
                        notches += ev.angleDelta.y / 120
                        const n = Math.trunc(notches)
                        if (n !== 0) {
                            notches -= n
                            viewer.step(-n)
                        }
                    }
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
                    onActiveTranslationChanged: if (active && viewer.zoomed)
                        viewer.pan(start.x + activeTranslation.x, start.y + activeTranslation.y)
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
                        viewer.pan(start.x + activeTranslation.x, start.y + activeTranslation.y)
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

                // Fitted by Rust. When zoomed, the GPU stretches it over the image's place until a sharper layer is
                // ready.
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
                    visible: viewer.still || !viewer.videoStarted
                }
                // Between fit and 1:1: the visible part, rendered by Rust at the zoom (Lanczos3), after zooming and
                // panning stop.
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
                        // The part on screen (Rust works it out and renders it); "" when none shows.
                        const source = viewport.regionSource(viewer.key)
                        if (source === "")
                            return
                        // Before the source: a cached region is ready at once.
                        region.next = Qt.rect(viewport.regionX(), viewport.regionY(), viewport.regionW(), viewport.regionH())
                        region.source = source

                    }
                }
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

                // Videos and animations: made with the first one shown (`mediaOpened`), then kept.
                Loader {
                    anchors.fill: parent
                    active: viewer.mediaOpened
                    sourceComponent: Component {
                        Item {
                            // Fitted (up or down), turned by the catalog's turns.
                            Item {
                                visible: !viewer.still && viewer.playScale > 0
                                width: viewer.fw * viewer.playScale
                                height: viewer.fh * viewer.playScale
                                anchors.centerIn: parent
                                Item {
                                    anchors.centerIn: parent
                                    width: viewport.swapsAxes(viewer.playerTurns) ? parent.height : parent.width
                                    height: viewport.swapsAxes(viewer.playerTurns) ? parent.width : parent.height
                                    rotation: viewer.playerTurns * 90
                                    VideoOutput {
                                        id: videoOut
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
                                    viewer.videoStarted = false
                                    if (viewer.playsVideo)
                                        play()
                                }
                                onErrorChanged: viewer.playerError = errorString
                                // Made for a video already shown: its source was set before the handler above.
                                Component.onCompleted: if (viewer.playsVideo) play()
                            }
                            AnimationPlayer {
                                id: animPlayer
                                source: viewer.playsAnimation ? viewer.filePath : ""
                                onSourceChanged: viewer.videoStarted = false
                                Component.onCompleted: setSink(videoOut.videoSink)
                            }
                            // The first frame: the poster stays until then.
                            Connections {
                                target: videoOut.videoSink
                                function onVideoFrameChanged() {
                                    viewer.videoStarted = true
                                }
                            }
                            Connections {
                                target: viewer
                                function onPlayToggled() {
                                    if (viewer.kind === "animated")
                                        animPlayer.playing = !animPlayer.playing
                                    else if (player.playbackState === MediaPlayer.PlayingState)
                                        player.pause()
                                    else
                                        player.play()
                                }
                            }

                            // Video transport: play/pause, position (drag to seek), sound; just above the rating bar,
                            // which is not a sibling to anchor to (both are 12 px plus their height from the bottom).
                            Rectangle {
                                visible: viewer.kind === "video" && !viewer.zen
                                anchors.horizontalCenter: parent.horizontalCenter
                                anchors.bottom: parent.bottom
                                anchors.bottomMargin: 12 + bar.height + 8
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
                        }
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
                    visible: viewer.showBorders && fit.status === Image.Ready
                    x: (viewer.zoomed ? viewer.ox : fit.x) - 1
                    y: (viewer.zoomed ? viewer.oy : fit.y) - 1
                    width: (viewer.zoomed ? viewer.dw : fit.width) + 2
                    height: (viewer.zoomed ? viewer.dh : fit.height) + 2
                    color: "transparent"
                    border.color: "#ff00ff"
                    border.width: 1
                }

                // The borders found, over the image wherever it is (fitted or zoomed). Hovering over the cut hides
                // them, to see what is there.
                BorderOverlay {
                    id: borders
                    // The cursor's place in the image, as fractions.
                    readonly property real u: (hover.point.position.x - x) / Math.max(1, width)
                    readonly property real v: (hover.point.position.y - y) / Math.max(1, height)
                    readonly property bool peek: hover.hovered && u >= 0 && u <= 1 && v >= 0 && v <= 1
                        && !(u >= kept.x && u < kept.x + kept.width && v >= kept.y && v < kept.y + kept.height)
                    visible: viewer.showBorders && viewer.hasBorders && fit.status === Image.Ready && !peek
                    x: viewer.zoomed ? viewer.ox : fit.x
                    y: viewer.zoomed ? viewer.oy : fit.y
                    width: viewer.zoomed ? viewer.dw : fit.width
                    height: viewer.zoomed ? viewer.dh : fit.height
                    kept: viewer.borderBox
                    rejectedTop: viewer.rejectedTop
                    rejectedBottom: viewer.rejectedBottom
                    rejectedLeft: viewer.rejectedLeft
                    rejectedRight: viewer.rejectedRight
                    marked: viewer.borderMarked
                }

                HoverHandler { id: hover }

                // Loupe: the full-resolution image at 10× (nearest neighbour), centred on the point under the cursor.
                Rectangle {
                    id: loupeBox
                    readonly property real u: (hover.point.position.x - fit.x) / Math.max(1, fit.width)
                    readonly property real v: (hover.point.position.y - fit.y) / Math.max(1, fit.height)
                    visible: viewer.loupe && viewer.still && !viewer.zoomed && hover.hovered
                             && full.status === Image.Ready && u >= 0 && u <= 1 && v >= 0 && v <= 1
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
                        text: keymap.viewerHint(viewer.kind, viewer.hasBorders)
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
                            + (viewer.kind === "video" && viewer.playerError !== "" ? "   — " + viewer.playerError : "")
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
                            // Borders found: marked as not a border while checked.
                            ToolButton {
                                text: "▭"
                                visible: viewer.hasBorders
                                checkable: true
                                checked: viewer.borderMarked
                                font.pixelSize: 22
                                font.strikeout: viewer.borderMarked
                                ToolTip.visible: hovered
                                ToolTip.text: viewer.borderMarked
                                    ? "Marked not a border: unmark (N)"
                                    : "Not a border (N)"
                                onClicked: viewer.borderMarkToggled(viewer.index, !viewer.borderMarked)
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
                                    ToolTip.text: index === 0 ? "Unrated (0)"
                                        : index + (index === 1 ? " star" : " stars") + " (" + index + ")"
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

                // What the root asks of it.
                Connections {
                    target: viewer
                    function onRated(i: int, r: int) { popup.show(r) }
                    function onViewMoved() { regionTimer.restart() }
                    function onViewReset() { region.source = "" }
                }
            }
        }
    }
}
