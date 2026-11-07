// Loupe: the full-resolution image at 10× (nearest neighbour), centred on the point under the cursor. Shown for a still
// (L), at the fit, while the cursor is over the image. The zoom comes from the Viewport facade.
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

    readonly property real u: (cursorX - fit.x) / Math.max(1, fit.width)
    readonly property real v: (cursorY - fit.y) / Math.max(1, fit.height)

    visible: requested && !viewport.zoomed && hovered
             && fullReady && u >= 0 && u <= 1 && v >= 0 && v <= 1
    width: 400
    height: 400
    x: cursorX - width / 2
    y: cursorY - height / 2
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
