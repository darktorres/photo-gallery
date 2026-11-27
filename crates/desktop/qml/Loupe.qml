// Loupe: the full-resolution image at 10× (nearest neighbour), centred on the point under the cursor. Shown for a still
// (L), at the fit. With the pointer off the picture (or no pointer yet, the keys alone), it is centred on the place the
// pointer was last over the picture, else the picture's centre, so L shows it the same way with the keys. The zoom
// comes from the Viewport facade.
pragma ComponentBehavior: Bound

import QtQuick
import pg.gallery

Rectangle {
    id: loupeBox

    required property Viewport viewport
    // The fitted image (its place is the loupe's frame of reference) and the original (its source and size).
    required property Image fit
    // The original as ImageLayers has it: its source, whether it is ready, and its size.
    required property url fullSource
    required property bool fullReady
    required property real fullWidth
    required property real fullHeight
    // The cursor's place over the picture, and whether it is over it (the viewer's HoverHandler).
    required property real cursorX
    required property real cursorY
    required property bool hovered
    // L is on, and the shown item is a still.
    required property bool requested
    required property real dpr

    // The cursor over the fitted picture, as fractions of it.
    readonly property real cu: (cursorX - fit.x) / Math.max(1, fit.width)
    readonly property real cv: (cursorY - fit.y) / Math.max(1, fit.height)
    readonly property bool overPicture: cu >= 0 && cu <= 1 && cv >= 0 && cv <= 1
    // Where the loupe is centred: the pointer's place while it is over the viewer, else where the pointer was last over
    // the picture (the picture's centre before it has been there), so L with the keys alone shows it.
    property real lastX: fit.x + fit.width / 2
    property real lastY: fit.y + fit.height / 2
    Binding on lastX {
        value: loupeBox.cursorX
        when: loupeBox.hovered && loupeBox.overPicture
    }
    Binding on lastY {
        value: loupeBox.cursorY
        when: loupeBox.hovered && loupeBox.overPicture
    }
    readonly property real px: hovered ? cursorX : lastX
    readonly property real py: hovered ? cursorY : lastY
    readonly property real u: (px - fit.x) / Math.max(1, fit.width)
    readonly property real v: (py - fit.y) / Math.max(1, fit.height)

    visible: requested && !viewport.zoomed && fullReady && u >= 0 && u <= 1 && v >= 0 && v <= 1
    width: 400
    height: 400
    x: px - width / 2
    y: py - height / 2
    color: "black"
    clip: true

    Image {
        source: loupeBox.fullSource
        width: loupeBox.fullWidth * 10 / loupeBox.dpr
        height: loupeBox.fullHeight * 10 / loupeBox.dpr
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
