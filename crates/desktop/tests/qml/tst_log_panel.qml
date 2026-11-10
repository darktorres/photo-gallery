// The log view (LogPanel.qml): a row's click shows its whole message and detail, Escape and Close dismiss it, and Tab
// stays within it (the grid behind never takes the focus). Run by `just qmltest`.
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

    // The model's roles, as LogModel has them (src/log_model.rs).
    ListModel {
        id: logRecords
        ListElement { time: "2026-11-10 12:00:00"; level: 0; target: "session"; message: "start"; detail: "gallery 0.1.0" }
        ListElement { time: "2026-11-10 12:00:01"; level: 2; target: "panic"; message: "panicked at <b>x</b>"; detail: "   0: main" }
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

    function panel() {
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
        compare(p.selectedText, "panicked at <b>x</b>\n\n   0: main")
    }
}
