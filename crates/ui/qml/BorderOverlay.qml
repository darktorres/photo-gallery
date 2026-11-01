// What cropping an image's borders would cut, drawn over the image (as the eval viewer's Borders page did): the cut
// dimmed magenta, what is left outlined, and where rejected candidates would have cut as yellow lines (as
// tools/borders.py's report did). Grey for borders marked as not a border. It covers the image's painted area;
// what is left and the lines are fractions of it (GalleryModel.borderBox, borderRejected).
pragma ComponentBehavior: Bound

import QtQuick

Item {
    id: overlay

    // What is left: x, y, width, height as fractions of the image.
    required property rect kept
    // A rejected candidate's cut at each edge, as a fraction of the image from that edge (-1: none).
    property real rejectedTop: -1
    property real rejectedBottom: -1
    property real rejectedLeft: -1
    property real rejectedRight: -1
    property bool marked: false
    property real lineWidth: 2

    readonly property color tint: marked ? "#a0a0a0" : "#ff00c8"
    // What is left, in pixels.
    readonly property real keptLeft: kept.x * width
    readonly property real keptTop: kept.y * height
    readonly property real keptRight: (kept.x + kept.width) * width
    readonly property real keptBottom: (kept.y + kept.height) * height

    // The cut: above, below, then left and right of what is left.
    component Cut: Rectangle {
        color: overlay.tint
        opacity: 0.38
    }
    Cut { x: 0; y: 0; width: overlay.width; height: overlay.keptTop }
    Cut { x: 0; y: overlay.keptBottom; width: overlay.width; height: overlay.height - overlay.keptBottom }
    Cut { x: 0; y: overlay.keptTop; width: overlay.keptLeft; height: overlay.keptBottom - overlay.keptTop }
    Cut {
        x: overlay.keptRight
        y: overlay.keptTop
        width: overlay.width - overlay.keptRight
        height: overlay.keptBottom - overlay.keptTop
    }

    Rectangle {
        x: overlay.keptLeft
        y: overlay.keptTop
        width: overlay.keptRight - overlay.keptLeft
        height: overlay.keptBottom - overlay.keptTop
        color: "transparent"
        border.color: overlay.tint
        border.width: overlay.lineWidth
    }

    // Rejected candidates: across the image, where they would have cut.
    component Rejected: Rectangle {
        color: "#ffd200"
    }
    Rejected {
        visible: overlay.rejectedTop >= 0
        y: overlay.rejectedTop * overlay.height - overlay.lineWidth / 2
        width: overlay.width
        height: overlay.lineWidth
    }
    Rejected {
        visible: overlay.rejectedBottom >= 0
        y: (1 - overlay.rejectedBottom) * overlay.height - overlay.lineWidth / 2
        width: overlay.width
        height: overlay.lineWidth
    }
    Rejected {
        visible: overlay.rejectedLeft >= 0
        x: overlay.rejectedLeft * overlay.width - overlay.lineWidth / 2
        width: overlay.lineWidth
        height: overlay.height
    }
    Rejected {
        visible: overlay.rejectedRight >= 0
        x: (1 - overlay.rejectedRight) * overlay.width - overlay.lineWidth / 2
        width: overlay.lineWidth
        height: overlay.height
    }
}
