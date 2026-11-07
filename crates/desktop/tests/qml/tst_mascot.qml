// Mascot: a left click shows the next mascot, a right click the previous one, both wrapping around the 40. Run by
// `just qmltest`. The mascots' qrc: files aren't staged for the tests, so the image itself doesn't load here; the
// numbers and the source's file name are what's checked.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "Mascot"
    // Items created under a non-visible TestCase are not visible, and a click needs a shown window.
    visible: true
    when: windowShown

    Component {
        id: mascotC
        Mascot {
            width: 300
        }
    }

    function clickAt(item, button) {
        mouseClick(item, item.width / 2, item.height / 2, button)
    }

    function test_starts_on_a_mascot_from_1_to_40() {
        const m = createTemporaryObject(mascotC, tc)
        verify(m !== null)
        verify(m.number >= 1 && m.number <= m.count)
    }

    function test_left_click_shows_the_next_one() {
        const m = createTemporaryObject(mascotC, tc)
        verify(m !== null)
        wait(20)
        m.number = 5
        clickAt(m, Qt.LeftButton)
        compare(m.number, 6)
        verify(m.source.toString().endsWith("/resources/mascots/06.png"), m.source)
    }

    function test_right_click_shows_the_previous_one() {
        const m = createTemporaryObject(mascotC, tc)
        verify(m !== null)
        wait(20)
        m.number = 5
        clickAt(m, Qt.RightButton)
        compare(m.number, 4)
        verify(m.source.toString().endsWith("/resources/mascots/04.png"), m.source)
    }

    function test_left_click_after_the_last_wraps_to_the_first() {
        const m = createTemporaryObject(mascotC, tc)
        verify(m !== null)
        wait(20)
        m.number = m.count
        clickAt(m, Qt.LeftButton)
        compare(m.number, 1)
        verify(m.source.toString().endsWith("/resources/mascots/01.png"), m.source)
    }

    function test_a_pinned_mascot_starts_on_its_number() {
        const m = createTemporaryObject(mascotC, tc, { pinned: 7 })
        verify(m !== null)
        compare(m.number, 7)
        verify(m.source.toString().endsWith("/resources/mascots/07.png"), m.source)
    }

    function test_a_pin_out_of_range_is_random_as_without_one() {
        const m = createTemporaryObject(mascotC, tc, { pinned: 99 })
        verify(m !== null)
        verify(m.number >= 1 && m.number <= m.count)
    }

    function test_right_click_before_the_first_wraps_to_the_last() {
        const m = createTemporaryObject(mascotC, tc)
        verify(m !== null)
        wait(20)
        m.number = 1
        clickAt(m, Qt.RightButton)
        compare(m.number, m.count)
        verify(m.source.toString().endsWith("/resources/mascots/40.png"), m.source)
    }
}
