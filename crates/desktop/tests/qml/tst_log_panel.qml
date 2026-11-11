// The log view (LogPanel.qml): a row's click shows its whole message and detail, Escape and Close dismiss it, and Tab
// stays within it (the grid behind never takes the focus). It follows the newest record until the user scrolls away
// from the end, and again once they scroll back to it; the selection is the row's number (`seq`), found again when the
// oldest rows are trimmed and let go when its own row is. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "LogPanel"
    // Items created under a non-visible TestCase are not visible, so no key reaches them (see tst_wheellistview).
    visible: true
    when: windowShown
    width: 1000
    height: 800

    // The model's roles, as LogModel has them (src/log_model.rs): `seq` is the row's own number, which trims keep.
    ListModel {
        id: logRecords
    }
    property int nextSeq: 0
    function record(message: string, detail: string) {
        logRecords.append({ time: "2026-11-10 12:00:00", level: 0, target: "session", message: message, detail: detail, seq: nextSeq })
        nextSeq++
    }
    function fill(count: int) {
        logRecords.clear()
        nextSeq = 0
        record("start", "gallery 0.1.0")
        logRecords.append({ time: "2026-11-10 12:00:01", level: 2, target: "panic", message: "panicked at <b>x</b>", detail: "   0: main", seq: 1 })
        nextSeq = 2
        for (let i = 2; i < count; i++)
            record("record " + i, "")
    }

    Component {
        id: panelC
        LogPanel {
            width: tc.width
            height: tc.height
            open: true
            records: logRecords
        }
    }

    SignalSpy { id: dismissedSpy; signalName: "dismissed" }

    // A panel over `count` records (2 when not given).
    function panel(count) {
        fill(count || 2)
        const p = createTemporaryObject(panelC, tc)
        verify(p !== null)
        dismissedSpy.target = p
        dismissedSpy.clear()
        return p
    }

    function test_the_list_has_the_keys_when_it_opens() {
        const p = panel()
        const list = findChild(p, "logList")
        tryVerify(() => list.activeFocus)
        compare(list.count, 2)
    }

    function test_escape_dismisses_it() {
        const p = panel()
        tryVerify(() => findChild(p, "logList").activeFocus)
        keyClick(Qt.Key_Escape)
        compare(dismissedSpy.count, 1)
    }

    function test_close_dismisses_it() {
        const p = panel()
        mouseClick(findChild(p, "logClose"))
        compare(dismissedSpy.count, 1)
    }

    function test_tab_wraps_within_the_panel() {
        const p = panel()
        const list = findChild(p, "logList")
        const close = findChild(p, "logClose")
        tryVerify(() => list.activeFocus)
        keyClick(Qt.Key_Tab)
        verify(close.activeFocus)
        keyClick(Qt.Key_Tab)
        verify(list.activeFocus)
        keyClick(Qt.Key_Backtab)
        verify(close.activeFocus)
    }

    function test_a_click_on_a_row_shows_it_whole() {
        const p = panel()
        const list = findChild(p, "logList")
        tryVerify(() => list.activeFocus)
        tryVerify(() => list.itemAtIndex(1) !== null)
        mouseClick(list.itemAtIndex(1))
        compare(p.selected, 1)
        compare(p.selectedSeq, 1)
        compare(p.selectedText, "panicked at <b>x</b>\n\n   0: main")
    }

    // The view follows the end as records come; a scroll up stops it (records keep coming, the view stays), and a scroll
    // back to the end follows again.
    function test_it_follows_the_end_until_the_user_scrolls_away() {
        const p = panel(200)
        const list = findChild(p, "logList")
        tryVerify(() => list.atYEnd)
        verify(p.following)
        record("one more", "")
        tryVerify(() => list.atYEnd, 2000, "a new record keeps the view at the end")
        // The wheel, up: away from the end.
        list.wheelBy(1200, 0, Qt.NoScrollPhase)
        tryVerify(() => !p.following, 2000)
        const at = list.contentY
        record("while away", "")
        record("while away again", "")
        wait(50)
        verify(!list.atYEnd, "the view stays where the user put it")
        fuzzyCompare(list.contentY, at, 1)
        // Back to the end: following again.
        list.wheelBy(-120000, 0, Qt.NoScrollPhase)
        tryVerify(() => p.following, 2000)
        record("back", "")
        tryVerify(() => list.atYEnd, 2000)
    }

    // The selection is its row's number: rows trimmed before it move it, and it is let go when its own row goes.
    function test_the_selection_follows_its_row_through_a_trim() {
        const p = panel(5)
        const list = findChild(p, "logList")
        tryVerify(() => list.itemAtIndex(2) !== null)
        mouseClick(list.itemAtIndex(2))
        compare(p.selected, 2)
        compare(p.selectedSeq, 2)
        // The ring trims the oldest record, and a new one comes after.
        logRecords.remove(0)
        record("new", "")
        compare(p.selected, 1)
        compare(p.selectedSeq, 2)
        // The view lays the changed rows out at its next polish; until then its delegates are those from before.
        list.forceLayout()
        verify(list.itemAtIndex(1).chosen)
        verify(!list.itemAtIndex(0).chosen)
        // The selected row itself is trimmed: nothing is selected.
        logRecords.remove(0, 2)
        compare(p.selected, -1)
        compare(p.selectedText, "")
    }
}
