// Full-window viewer (PLAN §7): fit to screen with an antialiased (Lanczos3) downscale made in Rust, 1:1 with
// panning, a 10× loupe to judge sharpness, and a bright frame around the image to reveal black bars.
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Window

Rectangle {
    id: viewer

    required property var gallery
    property int index: -1
    property bool actual: false   // Z: 1 image px = 1 screen px
    property bool loupe: false    // L: 10× loupe under the cursor
    property bool frame: false    // B: bright frame around the image
    readonly property double fid: index >= 0 ? gallery.fileId(index) : -1
    readonly property real dpr: Screen.devicePixelRatio

    signal closed(int index)

    color: "black"
    visible: index >= 0
    focus: visible

    function show(i) {
        index = i
        actual = false
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
            actual = false
        }
    }

    // The screen-sized image for item i, already fitted by Rust (shown 1:1 in device pixels).
    function fitSource(i) {
        if (i < 0 || i >= gallery.count)
            return ""
        return "image://full/" + gallery.fileId(i) + "/" + Math.round(width * dpr) + "x" + Math.round(height * dpr)
    }
    readonly property string fullSource: fid >= 0 ? "image://full/" + fid + "/full" : ""

    Keys.onPressed: (e) => {
        if (e.key === Qt.Key_Escape || e.key === Qt.Key_Return || e.key === Qt.Key_Enter) close()
        else if (e.key === Qt.Key_Left || e.key === Qt.Key_Backspace) step(-1)
        else if (e.key === Qt.Key_Right || e.key === Qt.Key_Space) step(1)
        else if (e.key === Qt.Key_Z) actual = !actual
        else if (e.key === Qt.Key_L) loupe = !loupe
        else if (e.key === Qt.Key_B) frame = !frame
        else if (e.key >= Qt.Key_0 && e.key <= Qt.Key_5) gallery.setRating(index, e.key - Qt.Key_0)
        else return
        e.accepted = true
    }

    MouseArea {
        anchors.fill: parent
        onDoubleClicked: viewer.close()
        onWheel: (w) => viewer.step(w.angleDelta.y > 0 ? -1 : 1)
    }

    Image {
        id: fit
        anchors.centerIn: parent
        visible: !viewer.actual
        source: viewer.visible ? viewer.fitSource(viewer.index) : ""
        width: implicitWidth / viewer.dpr
        height: implicitHeight / viewer.dpr
        asynchronous: true
        retainWhileLoading: true
        smooth: false
    }
    // Neighbours, preloaded so stepping is instant (same URLs, so the pixmap cache serves them).
    Image { visible: false; asynchronous: true; source: viewer.visible ? viewer.fitSource(viewer.index + 1) : "" }
    Image { visible: false; asynchronous: true; source: viewer.visible ? viewer.fitSource(viewer.index - 1) : "" }

    Flickable {
        id: flick
        anchors.fill: parent
        visible: viewer.actual
        clip: true
        contentWidth: Math.max(width, full.width)
        contentHeight: Math.max(height, full.height)
        boundsBehavior: Flickable.StopAtBounds
        Image {
            id: full
            source: viewer.visible && (viewer.actual || viewer.loupe) ? viewer.fullSource : ""
            width: implicitWidth / viewer.dpr
            height: implicitHeight / viewer.dpr
            x: Math.max(0, (flick.width - width) / 2)
            y: Math.max(0, (flick.height - height) / 2)
            asynchronous: true
            smooth: false
        }
        ScrollBar.horizontal: ScrollBar {}
        ScrollBar.vertical: ScrollBar {}
    }

    // Frame: a 1 px magenta line just outside the image, so black bars inside it stand out.
    Rectangle {
        readonly property Image target: viewer.actual ? full : fit
        visible: viewer.frame && target.status === Image.Ready
        // Bindings (not mapToItem, which doesn't update) so the frame follows panning in 1:1.
        x: (viewer.actual ? flick.x + full.x - flick.contentX : fit.x) - 1
        y: (viewer.actual ? flick.y + full.y - flick.contentY : fit.y) - 1
        width: target.width + 2
        height: target.height + 2
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
        visible: viewer.loupe && !viewer.actual && hover.hovered && full.status === Image.Ready
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

    Label {
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        anchors.margins: 10
        color: "#cccccc"
        style: Text.Outline
        styleColor: "black"
        text: (viewer.gallery.revision >= 0 && viewer.index >= 0 ? viewer.gallery.info(viewer.index) : "")
            + (viewer.actual ? "   [1:1]" : "") + (viewer.loupe ? "   [loupe 10×]" : "")
            + (fit.status === Image.Error ? "   — original unavailable" : "")
    }
    Label {
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 10
        color: "#777777"
        text: (viewer.index + 1) + " / " + viewer.gallery.count
              + "    ← → step · Z 1:1 · L loupe · B frame · 0–5 rate · Esc close"
    }
}
