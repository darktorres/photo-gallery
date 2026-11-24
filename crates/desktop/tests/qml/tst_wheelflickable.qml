// WheelFlickable: a notch moves the view by the system's lines of wheelLineHeight, a burst of notches travels all of
// them, and the view stops at its end. Run by `just qmltest`. The same checks as tst_wheellistview.qml, on a Flickable.
//
// The tests call wheelBy() with a wheel event's fields rather than sending mouseWheel(): QtTest's mouseWheel does not
// reach a WheelHandler, so the handler's own routing is not covered here.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "WheelFlickable"
    // Items created under a non-visible TestCase are not visible: the view would not lay out its content.
    visible: true
    when: windowShown

    Component {
        id: flickC
        WheelFlickable {
            width: 200
            height: 200
            contentWidth: width
            contentHeight: 200 * 50
        }
    }

    // The distance one notch moves, in pixels: the system's wheel lines, each wheelLineHeight.
    function notch(flick) {
        return Application.styleHints.wheelScrollLines * flick.wheelLineHeight;
    }

    // One notch as QWheelEvent reports it: a negative vertical angle delta (120 units) scrolls down.
    function wheelNotch(flick) {
        flick.wheelBy(-120, 0, Qt.NoScrollPhase);
    }

    function test_a_notch_moves_the_system_lines() {
        const flick = createTemporaryObject(flickC, tc);
        verify(flick !== null);
        wait(20);
        compare(flick.contentY, 0);
        wheelNotch(flick);
        wait(1000);
        fuzzyCompare(flick.contentY, notch(flick), 1);
    }

    function test_a_burst_of_notches_travels_all_of_them() {
        const flick = createTemporaryObject(flickC, tc);
        verify(flick !== null);
        wait(20);
        for (let i = 0; i < 10; i++)
            wheelNotch(flick);
        wait(2000);
        fuzzyCompare(flick.contentY, 10 * notch(flick), 2);
    }

    // 400 notches are far past the 9800 px of this content: the burst must stop at the end, not beyond it.
    function test_the_view_stops_at_its_end() {
        const flick = createTemporaryObject(flickC, tc);
        verify(flick !== null);
        wait(20);
        for (let i = 0; i < 400; i++)
            wheelNotch(flick);
        wait(3000);
        const bottom = flick.contentHeight + flick.bottomMargin - flick.height;
        verify(flick.contentY <= bottom + 1, "scrolled past the end: " + flick.contentY + " > " + bottom);
        fuzzyCompare(flick.contentY, bottom, 1);
    }
}
