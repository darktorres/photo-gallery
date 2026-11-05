// Full-window viewer (PLAN §7) and triage (ImageRater's): fit to the window, up or down, with an antialiased
// (Lanczos3) resample made in Rust; zoom from fit to 32× around the cursor (pinch, + −) and pan by dragging; the wheel
// steps to the previous or next picture. Between
// fit and 1:1 the GPU scales the fitted image at once, and Rust renders the visible part with Lanczos after a
// pause; from 1:1 up the original is shown with nearest neighbour, to judge pixels. A 10× loupe; borders (B): a
// bright frame around the image to reveal black bars, and over it what cropping the borders found would cut, with
// N to mark them as not a border. Rating a picture moves on to the next one.
//
// This file is the composition root. What it shows is made when it is first opened (the content Loader); the
// video and animation player (MediaView) with the first one shown (the media Loader, inside it): until then the
// window has neither, nor Qt Multimedia's audio output, whose device probe runs on threads of its own. Both stay
// once made. The parts: ImageLayers (the picture, from Rust), MediaView with VideoTransport (the players), Loupe, the
// frame and borders (here), Hud (the labels and the RatingBar) and RatingPopup. The zoom and pan numbers come from
// the Viewport facade (`viewportItem`); the gallery's items are the window's (Main.qml), which gives the one shown by the
// properties below and does the ratings, turns and border marks asked for (rated(), turned(), borderMarkToggled()).
// The root keeps the state and the API; the parts report through signals, and the root reads what the media made
// through its properties (videoStarted, playerError).
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Window
import pg.gallery

Rectangle {
    id: viewer

    // The item shown (-1: none, and the viewer is closed), of `count`: the gallery's (it follows the file when the
    // list is refreshed). Showing another asks the window (`moved`).
    final required property int index
    required property int count
    // The gallery the item comes from. What the viewer shows of the item is the gallery's viewed* properties, which
    // it sets again when it changes the item in place (a rating, a turn): read here, not asked for.
    required property GalleryModel gallery
    // The shown item, from the gallery. Its image's key (image://full/<key>/…); the keys of the items around it,
    // preloaded.
    readonly property string key: gallery.viewedKey
    readonly property list<string> neighbourKeys: gallery.viewedNeighbourKeys
    // Quarter turns asked for but not yet in the file; its rating.
    readonly property int turns: gallery.viewedTurns
    readonly property int rating: gallery.viewedRating
    // Upright size in image pixels (pending turns included); 0 before the import, which leaves only the fit.
    readonly property int fw: gallery.viewedFullWidth
    readonly property int fh: gallery.viewedFullHeight
    // image, animated or video (query::MediaType). Only still images zoom.
    readonly property int kind: gallery.viewedKind
    // Videos and animations: Qt's players show the file as stored, so the catalog's turns are added here.
    readonly property int playerTurns: gallery.viewedPlayerTurns
    // The file, for the players; a line about it for the bar.
    readonly property url fileUrl: gallery.viewedFileUrl
    readonly property string filePath: gallery.viewedFilePath
    readonly property string info: gallery.viewedInfo
    // Its borders: what is left without them, as fractions of the image as shown (empty: none found); where a
    // rejected candidate would have cut at each edge (a fraction from that edge, -1: none); whether they are marked as
    // not a border. Whether to show them (B, the window's).
    readonly property rect borderBox: gallery.viewedBorderBox
    readonly property real rejectedTop: gallery.viewedRejectedTop
    readonly property real rejectedBottom: gallery.viewedRejectedBottom
    readonly property real rejectedLeft: gallery.viewedRejectedLeft
    readonly property real rejectedRight: gallery.viewedRejectedRight
    readonly property bool borderMarked: gallery.viewedBorderMarked
    required property bool showBorders
    // Real Size (Z): open images at 1:1 instead of fitted.
    required property Settings settings
    // The full-size image's URL (GalleryModel.viewedFullUrl, given by Main): the loupe's and the 1:1 view's.
    required property string fullUrl

    final property bool loupe: false    // L: 10× loupe under the cursor
    final property bool zen: false      // H or a tap: nothing over the picture
    readonly property real dpr: Screen.devicePixelRatio

    readonly property bool still: kind === GalleryModel.Image
    readonly property bool hasBorders: still && borderBox.width > 0
    readonly property bool zoomable: fw > 0 && fh > 0 && still
    readonly property bool playsVideo: visible && kind === GalleryModel.Video
    readonly property bool playsAnimation: visible && kind === GalleryModel.Animated
    // Sound, for the session.
    property bool muted: false
    property real volume: 1
    // Made once needed, then kept: the content at the first show, the players at the first video or animation.
    property bool opened: false
    property bool mediaOpened: false
    onVisibleChanged: if (visible) opened = true
    onPlaysVideoChanged: if (playsVideo) mediaOpened = true
    onPlaysAnimationChanged: if (playsAnimation) mediaOpened = true
    // From the media: a frame of the shown video or animation was drawn (the poster gives way); the player's error.
    property bool videoStarted: false
    property string playerError: ""

    // To what the parts made: the view moved (render the visible part again, after a pause), a new image opened
    // (drop the old part), play or pause asked for.
    signal viewMoved()
    signal viewReset()
    // Play or pause asked: each change makes MediaView toggle its player (its togglePlay).
    property int playRequests: 0

    // Show another item (-1: close), rate the shown one, turn it by quarter turns (1 = clockwise): the window does it.
    signal moved(int index)
    // A step by d through the list: the window answers with where it lands (the core clamps it, Shell.viewerStep),
    // which comes back to `stepTo`.
    signal stepRequested(int d)
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
    final readonly property real zoom: viewportItem.zoom
    readonly property real ox: viewportItem.ox
    readonly property real oy: viewportItem.oy
    // At the fit zoom (follows window resizes): above 1 for an image smaller than the window, which is enlarged to it.
    readonly property bool fitted: viewportItem.fitted
    readonly property real fitZoom: viewportItem.fitZoom
    // The smallest zoom: the fit, or 1:1 when the fit enlarges the image (1:1 is then the furthest zoom out).
    readonly property real minZoom: viewportItem.minZoom
    readonly property real dw: viewportItem.dw
    readonly property real dh: viewportItem.dh
    readonly property bool zoomed: viewportItem.zoomed
    // 1:1 and beyond, away from the fit: the original, nearest neighbour.
    readonly property bool pixels: viewportItem.pixels

    Viewport {
        id: viewportItem
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
        stepRequested(d)
    }

    // The item the step lands on (`n`): the viewer moves there, unless the step stays on the item it is on.
    function stepTo(n: int) {
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
        playRequests++
    }

    // Zoom to `z` keeping the image point under (px, py) where it is (the view limits it).
    function setZoom(z: real, px: real, py: real) {
        if (!zoomable)
            return
        viewportItem.setZoom(z, px, py)
        viewMoved()
    }

    // Pan the image's corner (a drag or a swipe), kept on screen.
    function pan(x: real, y: real) {
        viewportItem.pan(x, y)
        viewMoved()
    }

    // How a new image opens: fitted, or 1:1 in Real Size mode (Z), which is the fit when the two are the same.
    function resetView() {
        viewportItem.resetView(settings.realSize)
        viewMoved()
        viewReset()
    }

    // Double-click: fit ↔ 1:1 at the point clicked.
    function toggleOneToOne(px: real, py: real) {
        if (!zoomable)
            return
        viewportItem.toggleOneToOne(px, py)
        viewMoved()
    }

    // The window's size changed: a fitted view follows it (reopened fitted), another is kept and clamped.
    function pushWindow() {
        const reset = viewportItem.setWindow(width, height, dpr, settings.realSize)
        viewMoved()
        if (reset)
            viewReset()
    }

    // The image's size changed: a fitted view opens it again, fitted (or 1:1 in Real Size mode).
    function pushMedia() {
        if (viewportItem.setMedia(fw, fh, settings.realSize)) {
            viewMoved()
            viewReset()
        }
    }

    onWidthChanged: pushWindow()
    onHeightChanged: pushWindow()
    onFwChanged: pushMedia()
    onFhChanged: pushMedia()
    onStillChanged: viewportItem.setStill(still)

    // The view starts from the values the properties have when the viewer is made, without the signals.
    Component.onCompleted: {
        viewportItem.setStill(still)
        viewportItem.setMedia(fw, fh, settings.realSize)
        viewportItem.setWindow(width, height, dpr, settings.realSize)
    }

    // The key map (pg-client's keys, through the KeyMap facade) names the viewer's action for a key; this carries it out.
    KeyMap {
        id: keymap
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

                // The picture: fitted, the region between fit and 1:1, the original at 1:1 and for the loupe.
                ImageLayers {
                    id: layers
                    viewport: viewportItem
                    fitSource: viewer.visible ? viewportItem.fitSource(viewer.key) : ""
                    fullUrl: viewer.fullUrl
                    showing: viewer.visible
                    key: viewer.key
                    still: viewer.still
                    zoomable: viewer.zoomable
                    loupe: viewer.loupe
                    videoStarted: viewer.videoStarted
                    dpr: viewer.dpr
                    // The part on screen, after a pause: Rust works it out and renders it; "" when none shows.
                    onRegionDue: {
                        if (!viewer.visible || !viewportItem.zoomed || viewportItem.pixels)
                            return
                        const source = viewportItem.regionSource(viewer.key)
                        if (source === "")
                            return
                        layers.showRegion(source, Qt.rect(viewportItem.regionX(), viewportItem.regionY(),
                                                          viewportItem.regionW(), viewportItem.regionH()))
                    }
                }

                // Neighbours, preloaded so stepping is instant (same URLs, so the pixmap cache serves them).
                Repeater {
                    model: viewer.neighbourKeys
                    Image {
                        required property string modelData
                        visible: false
                        asynchronous: true
                        source: viewportItem.fitSource(modelData)
                    }
                }

                // Videos and animations: made with the first one shown (`mediaOpened`), then kept.
                Loader {
                    id: mediaLoader
                    anchors.fill: parent
                    active: viewer.mediaOpened
                    sourceComponent: MediaView {
                        swapsAxes: viewportItem.swapsAxes(viewer.playerTurns)
                        playRequests: viewer.playRequests
                        kind: viewer.kind
                        still: viewer.still
                        zen: viewer.zen
                        playsVideo: viewer.playsVideo
                        playsAnimation: viewer.playsAnimation
                        fw: viewer.fw
                        fh: viewer.fh
                        playerTurns: viewer.playerTurns
                        fileUrl: viewer.fileUrl
                        filePath: viewer.filePath
                        muted: viewer.muted
                        volume: viewer.volume
                        barHeight: hud.barHeight
                        onFrameDrawn: viewer.videoStarted = true
                        onSourceSwitched: viewer.videoStarted = false
                        onPlayerFailed: (text) => viewer.playerError = text
                        onPlayAsked: viewer.togglePlay()
                        onMuteAsked: viewer.muted = !viewer.muted
                        onVolumeAsked: (value) => viewer.volume = value
                    }
                }

                // Frame: a 1 px magenta line just outside the image, so black bars inside it stand out.
                Rectangle {
                    visible: viewer.showBorders && layers.fitStatus === Image.Ready
                    x: (viewportItem.zoomed ? viewportItem.ox : layers.fitImage.x) - 1
                    y: (viewportItem.zoomed ? viewportItem.oy : layers.fitImage.y) - 1
                    width: (viewportItem.zoomed ? viewportItem.dw : layers.fitImage.width) + 2
                    height: (viewportItem.zoomed ? viewportItem.dh : layers.fitImage.height) + 2
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
                    visible: viewer.showBorders && viewer.hasBorders && layers.fitStatus === Image.Ready && !peek
                    x: viewportItem.zoomed ? viewportItem.ox : layers.fitImage.x
                    y: viewportItem.zoomed ? viewportItem.oy : layers.fitImage.y
                    width: viewportItem.zoomed ? viewportItem.dw : layers.fitImage.width
                    height: viewportItem.zoomed ? viewportItem.dh : layers.fitImage.height
                    kept: viewer.borderBox
                    rejectedTop: viewer.rejectedTop
                    rejectedBottom: viewer.rejectedBottom
                    rejectedLeft: viewer.rejectedLeft
                    rejectedRight: viewer.rejectedRight
                    marked: viewer.borderMarked
                }

                HoverHandler { id: hover }

                // Loupe: the full-resolution image at 10× (nearest neighbour), centred on the point under the cursor.
                Loupe {
                    viewport: viewportItem
                    fit: layers.fitImage
                    fullSource: layers.fullSource
                    fullReady: layers.fullReady
                    fullWidth: layers.fullWidth
                    fullHeight: layers.fullHeight
                    cursorX: hover.point.position.x
                    cursorY: hover.point.position.y
                    hovered: hover.hovered
                    requested: viewer.loupe && viewer.still
                    dpr: viewer.dpr
                }

                // Everything over the picture; hidden in zen mode.
                Hud {
                    id: hud
                    visible: !viewer.zen
                    index: viewer.index
                    count: viewer.count
                    kind: viewer.kind
                    hasBorders: viewer.hasBorders
                    still: viewer.still
                    zoomable: viewer.zoomable
                    loupe: viewer.loupe
                    fitFailed: layers.fitStatus === Image.Error
                    turns: viewer.turns
                    rating: viewer.rating
                    borderMarked: viewer.borderMarked
                    info: viewer.info
                    playerError: viewer.playerError
                    filePath: viewer.filePath
                    settings: viewer.settings
                    viewport: viewportItem
                    onCloseAsked: viewer.close()
                    onRateAsked: (r) => viewer.rate(r)
                    onTurnAsked: (d) => viewer.turn(d)
                    onBorderMarkAsked: (marked) => viewer.borderMarkToggled(viewer.index, marked)
                    onFindSimilarAsked: viewer.findSimilar(viewer.filePath)
                }

                // The rating just given, for half a second (over the next picture).
                RatingPopup {
                    id: ratingPopup
                }

                // What the root asks of it.
                Connections {
                    target: viewer
                    function onRated(i: int, r: int) { ratingPopup.show(r) }
                    function onViewMoved() { layers.restartRegion() }
                    function onViewReset() { layers.dropRegion() }
                }
            }
        }
    }
}
