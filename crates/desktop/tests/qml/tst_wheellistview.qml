// WheelListView (S-14.3): a notch moves the view by the system's lines of wheelLineHeight, a burst of notches travels
// all of them, and the view stops at its end. Run by `just qmltest`.
//
// The tests call wheelBy() with a wheel event's fields rather than sending mouseWheel(): QtTest's mouseWheel reaches a
// MouseArea's onWheel but not a WheelHandler (checked: a bare WheelHandler gets no event under the offscreen platform,
// and a plain ListView does not scroll on one), so the handler's own routing is not covered here.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "WheelListView"
    // Items created under a non-visible TestCase are not visible: the list would make no delegates.
    visible: true
    when: windowShown

    Component {
        id: listC
        WheelListView {
            width: 200
            height: 200
            model: 200
            delegate: Rectangle {
                required property int index
                width: 200
                height: 50
                color: index % 2 ? "red" : "blue"
            }
        }
    }

    // The distance one notch moves, in pixels: the system's wheel lines, each wheelLineHeight.
    function notch(list) {
        return Application.styleHints.wheelScrollLines * list.wheelLineHeight
    }

    // One notch as QWheelEvent reports it: a negative vertical angle delta (120 units) scrolls down.
    function wheelNotch(list) {
        list.wheelBy(-120, 0, Qt.NoScrollPhase)
    }

    function test_a_notch_moves_the_system_lines() {
        const list = createTemporaryObject(listC, tc)
        verify(list !== null)
        wait(20)
        compare(list.contentY, 0)
        wheelNotch(list)
        wait(1000)
        fuzzyCompare(list.contentY, notch(list), 1)
    }

    function test_a_burst_of_notches_travels_all_of_them() {
        const list = createTemporaryObject(listC, tc)
        verify(list !== null)
        wait(20)
        for (let i = 0; i < 10; i++)
            wheelNotch(list)
        wait(2000)
        fuzzyCompare(list.contentY, 10 * notch(list), 2)
    }

    // 400 notches are 400 × 72 px, far past the 9800 px of this list: the burst must stop at the end, not beyond it.
    function test_the_view_stops_at_its_end() {
        const list = createTemporaryObject(listC, tc)
        verify(list !== null)
        wait(20)
        for (let i = 0; i < 400; i++)
            wheelNotch(list)
        wait(3000)
        const bottom = list.contentHeight + list.bottomMargin - list.height
        verify(list.contentY <= bottom + 1, "scrolled past the last row: " + list.contentY + " > " + bottom)
        fuzzyCompare(list.contentY, bottom, 1)
    }
}
