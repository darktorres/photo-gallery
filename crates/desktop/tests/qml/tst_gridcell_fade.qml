// GridCell's thumbnail fade: a faded-in thumbnail is drawn exactly as an opaque one, whatever frames the fade showed.
// Qt's software renderer, which offscreen runs and the screenshots use, ignores an opacity within 1e-5 of the one a
// node has (qFuzzyCompare in QSGSoftwareRenderableNode::setOpacity, Qt 6.12). A fade with a frame at 0.999996 then
// keeps that value under QML's final 1, and the raster engine draws it at alpha 254 ((int)(opacity * 256) is 255,
// which the blend makes 254): over black, every channel from 128 one lower. GridCell snaps its fade's last stretch to 1.
// The second test checks that Qt still does this: when it fails, the snap can go. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "GridCellFade"
    // Items created under a non-visible TestCase are not visible (see tst_wheellistview).
    visible: true
    when: windowShown
    width: 200
    height: 100

    // Black under the images: a thumbnail drawn at alpha 254 is then exactly one lower from 128 up.
    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    // The same stub thumbnail (one solid color, from the test host's provider) in each: at x 0, the one under test; at
    // x 100, an opaque one to compare with.
    readonly property string thumbUrl: "image://thumb/fade/100"

    Component {
        id: cellC
        GridCell {
            item: 0
            itemBadge: ""
            itemRating: 0
            itemBordersMarked: false
            itemImageAspect: 1
            itemAspect: 1
            itemContentX: 0
            itemContentY: 0
            itemContentWidth: 0
            itemContentHeight: 0
            version: 0
            rowHeight: 100
            itemThumbUrl: tc.thumbUrl
            itemThumbhashUrl: "image://thumbhash/fade"
            showBorders: false
            current: -1
        }
    }
    // A fade as GridCell had it: the opacity is the fade.
    Component {
        id: plainFadeC
        Image {
            property real fade: 0
            width: 100
            height: 100
            source: tc.thumbUrl
            opacity: fade
        }
    }
    Component {
        id: opaqueC
        Image {
            x: 100
            width: 100
            height: 100
            source: tc.thumbUrl
        }
    }

    // Each value in a frame of its own, as a fade shows them: the frames seen in the runs drawn at alpha 254 were one
    // well below 1, one within 1e-5 of it, and then 1. The wait is past GridCell's 120 ms Behavior.
    function showFrames(target, values) {
        for (const value of values) {
            target.fade = value
            wait(250)
        }
    }

    function channels(image, x, y) {
        const c = image.pixel(x, y)
        return [c.r, c.g, c.b].map(v => Math.round(v * 255))
    }

    // The thumbnail at x 0 and the opaque one at x 100, after the frames.
    function drawnAndOpaque(target) {
        const opaque = createTemporaryObject(opaqueC, tc)
        verify(opaque !== null)
        tryCompare(opaque, "status", Image.Ready)
        tryCompare(target, "status", Image.Ready)
        showFrames(target, [0, 0.98, 0.999996, 1])
        compare(target.opacity, 1)
        const image = grabImage(tc)
        return [channels(image, 50, 50), channels(image, 150, 50)]
    }

    function init() {
        // The renderer these tests are about: offscreen runs fall back to it on Windows and on Linux.
        compare(GraphicsInfo.api, GraphicsInfo.Software, "offscreen Qt Quick renders with the software renderer")
    }

    function test_a_faded_in_thumbnail_is_drawn_as_an_opaque_one() {
        const cell = createTemporaryObject(cellC, tc)
        verify(cell !== null)
        const thumb = findChild(cell, "thumb")
        verify(thumb !== null, "GridCell's thumbnail has the objectName thumb")
        const [drawn, opaque] = drawnAndOpaque(thumb)
        verify(opaque.some(v => v >= 128), "the stub color has a channel the bug would change: " + opaque)
        compare(drawn, opaque)
    }

    function test_qt_still_keeps_an_opacity_near_1() {
        const plain = createTemporaryObject(plainFadeC, tc)
        verify(plain !== null)
        const [drawn, opaque] = drawnAndOpaque(plain)
        compare(drawn, opaque.map(v => v >= 128 ? v - 1 : v),
                "Qt's software renderer no longer keeps an opacity within 1e-5 of 1: GridCell's snap to 1 can go")
    }
}
