// The viewer's gesture rules (S-14.3): stepping stops at the ends of the list, rating moves on, turning and closing
// signal the window, and the keys reach those calls. The Rust types the viewer needs (pg.gallery) are stubs
// (tests/qml/stubs), so these tests check Viewer.qml, not the decoder or the catalog. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "Viewer"
    // Items created under a non-visible TestCase are not visible, so no key or wheel reaches them (see tst_wheellistview).
    visible: true
    when: windowShown

    Settings { id: settingsStub }
    GalleryModel { id: galleryStub }
    Shell { id: shellStub }

    Component {
        id: viewerC
        Viewer {
            width: 800
            height: 600
            dpr: Screen.devicePixelRatio
            gallery: galleryStub
            onStepRequested: (d) => stepTo(shellStub.viewerStep(index, d))
            settings: settingsStub
            fullUrl: ""
            showBorders: false
        }
    }

    // The viewer's signals, spied on each test's viewer.
    SignalSpy { id: movedSpy; signalName: "moved" }
    SignalSpy { id: ratedSpy; signalName: "rated" }
    SignalSpy { id: turnedSpy; signalName: "turned" }
    SignalSpy { id: closedSpy; signalName: "closed" }
    SignalSpy { id: similarSpy; signalName: "findSimilar" }

    // A viewer showing `index` of `count` items, with its signals spied on.
    function viewer(index, count) {
        shellStub.count = count
        const v = createTemporaryObject(viewerC, tc, { index: index, count: count })
        verify(v !== null)
        movedSpy.target = v
        ratedSpy.target = v
        turnedSpy.target = v
        closedSpy.target = v
        movedSpy.clear()
        ratedSpy.clear()
        turnedSpy.clear()
        closedSpy.clear()
        return v
    }

    function test_stepping_moves_within_the_list() {
        const v = viewer(1, 3)
        v.step(1)
        compare(movedSpy.count, 1)
        compare(movedSpy.signalArguments[0][0], 2)
        v.step(-1)
        compare(movedSpy.count, 2)
        compare(movedSpy.signalArguments[1][0], 0)
    }

    function test_stepping_stops_at_the_ends() {
        const first = viewer(0, 3)
        first.step(-1)
        compare(movedSpy.count, 0)
        const last = viewer(2, 3)
        last.step(1)
        compare(movedSpy.count, 0)
    }

    function test_a_rating_is_signalled_and_moves_on() {
        const v = viewer(0, 3)
        v.rate(4)
        compare(ratedSpy.count, 1)
        compare(ratedSpy.signalArguments[0][0], 0)
        compare(ratedSpy.signalArguments[0][1], 4)
        compare(movedSpy.count, 1)
        compare(movedSpy.signalArguments[0][0], 1)
    }

    function test_turning_asks_the_window_for_the_quarter_turns() {
        const v = viewer(1, 3)
        v.turn(1)
        compare(turnedSpy.count, 1)
        compare(turnedSpy.signalArguments[0][0], 1)
        compare(turnedSpy.signalArguments[0][1], 1)
        v.turn(-1)
        compare(turnedSpy.signalArguments[1][1], -1)
    }

    function test_closing_signals_the_item_then_moves_off() {
        const v = viewer(1, 3)
        v.close()
        compare(closedSpy.count, 1)
        compare(closedSpy.signalArguments[0][0], 1)
        compare(movedSpy.count, 1)
        compare(movedSpy.signalArguments[0][0], -1)
    }

    function test_keys_reach_the_viewer_actions() {
        const v = viewer(1, 3)
        v.forceActiveFocus()
        keyClick(Qt.Key_Right)
        compare(movedSpy.signalArguments[movedSpy.count - 1][0], 2)
        keyClick(Qt.Key_Left)
        compare(movedSpy.signalArguments[movedSpy.count - 1][0], 0)
    }

    // The players and the Yandex search go by the file's id, which the gallery gives the viewer (no path text).
    function test_the_viewer_takes_its_file_id_from_the_gallery() {
        galleryStub.viewedFileId = 42
        const v = viewer(0, 3)
        compare(v.fileId, 42)
        galleryStub.viewedFileId = 0
    }

    // The bar's "find similar" asks for the shown file by its id (Main.qml hands it to the Yandex panel), not a path.
    function test_find_similar_carries_the_shown_file_id() {
        galleryStub.viewedFileId = 42
        const v = viewer(-1, 3)
        v.index = 0
        const hud = findChild(v, "hud")
        verify(hud !== null)
        similarSpy.target = v
        similarSpy.clear()
        hud.findSimilarAsked()
        compare(similarSpy.count, 1)
        compare(similarSpy.signalArguments[0][0], 42)
        galleryStub.viewedFileId = 0
    }

    // A resize renders the fit again at the window's new size (C-15), once the size has settled, not at each step of
    // the drag. Opened from closed, as the window does it, so the content is made.
    function test_a_resize_renders_the_fit_again_once_it_settles() {
        galleryStub.viewedKey = "k"
        const v = viewer(-1, 3)
        v.index = 0
        const fit = findChild(v, "fitImage")
        verify(fit !== null)
        const opened = "image://full/k/fit/" + Math.round(800 * v.dpr) + "x" + Math.round(600 * v.dpr)
        compare(String(fit.source), opened)
        v.width = 500
        v.width = 400
        compare(String(fit.source), opened)
        const resized = "image://full/k/fit/" + Math.round(400 * v.dpr) + "x" + Math.round(600 * v.dpr)
        tryVerify(() => String(fit.source) === resized, 2000)
        galleryStub.viewedKey = ""
    }

    // A pixel-ratio change (the window moved to a screen with another ratio) hands the ratio to the view, and the fit
    // is rendered at the new device size once it has settled (C-15, L-43).
    function test_a_pixel_ratio_change_renders_the_fit_at_the_new_size() {
        galleryStub.viewedKey = "k"
        const v = viewer(-1, 3)
        v.index = 0
        const fit = findChild(v, "fitImage")
        verify(fit !== null)
        const dpr = v.dpr * 2
        v.dpr = dpr
        tryCompare(v, "fitW", Math.round(800 * dpr), 2000)
        tryCompare(v, "fitH", Math.round(600 * dpr), 2000)
        const resized = "image://full/k/fit/" + Math.round(800 * dpr) + "x" + Math.round(600 * dpr)
        tryVerify(() => String(fit.source) === resized, 2000)
        galleryStub.viewedKey = ""
    }
}
