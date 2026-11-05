// The viewer's gesture rules (S-14.3): stepping stops at the ends of the list, rating moves on, turning and closing
// signal the window, and the keys reach those calls. The Rust types the viewer needs (pg.gallery) are stubs
// (tests/qml/stubs), so these tests check Viewer.qml, not the decoder or the catalog. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery
import "../../qml"

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
}
