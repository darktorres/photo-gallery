// The log view (LogPanel.qml): a row's click shows its whole message and detail, Escape and Close dismiss it, and Tab
// stays within it (the grid behind never takes the focus). It follows the newest record until the user scrolls away
// from the end, and again once they scroll back to it. The selection is the log model's (by the row's number, src/
// log_model.rs): the panel is given what it shows, and a test plays the window through answer(). Run by `just qmltest`.
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
        logRecords.append({
            time: "2026-11-10 12:00:00",
            level: 0,
            target: "session",
            message: message,
            detail: detail,
            seq: nextSeq
        });
        nextSeq++;
    }
    function fill(count: int) {
        logRecords.clear();
        nextSeq = 0;
        record("start", "gallery 0.1.0");
        logRecords.append({
            time: "2026-11-10 12:00:01",
            level: 2,
            target: "panic",
            message: "panicked at <b>x</b>",
            detail: "   0: main",
            seq: 1
        });
        nextSeq = 2;
        for (let i = 2; i < count; i++)
            record("record " + i, "");
    }

    // The window's answer to a selection: the log model holds it, and says what it shows (its row, the record's number
    // and its text, the message with its detail below). Here the records stand in for it.
    function answer(p, row) {
        const r = row >= 0 && row < logRecords.count ? logRecords.get(row) : null;
        p.selected = r ? row : -1;
        p.selectedSeq = r ? r.seq : -1;
        p.selectedText = r ? (r.detail !== "" ? r.message + "\n\n" + r.detail : r.message) : "";
    }

    Component {
        id: panelC
        LogPanel {
            id: log
            width: tc.width
            height: tc.height
            open: true
            records: logRecords
            recordsReady: true
            selected: -1
            selectedSeq: -1
            selectedText: ""
            onSelectRequested: row => tc.answer(log, row)
            onUnselectRequested: tc.answer(log, -1)
        }
    }

    SignalSpy {
        id: dismissedSpy
        signalName: "dismissed"
    }

    // A panel over `count` records (2 when not given).
    function panel(count) {
        fill(count || 2);
        const p = createTemporaryObject(panelC, tc);
        verify(p !== null);
        dismissedSpy.target = p;
        dismissedSpy.clear();
        return p;
    }

    function test_the_list_has_the_keys_when_it_opens() {
        const p = panel();
        const list = findChild(p, "logList");
        tryVerify(() => list.activeFocus);
        compare(list.count, 2);
        compare(p.selected, -1);
    }

    function test_escape_dismisses_it() {
        const p = panel();
        tryVerify(() => findChild(p, "logList").activeFocus);
        keyClick(Qt.Key_Escape);
        compare(dismissedSpy.count, 1);
    }

    function test_close_dismisses_it() {
        const p = panel();
        mouseClick(findChild(p, "logClose"));
        compare(dismissedSpy.count, 1);
    }

    function test_tab_wraps_within_the_panel() {
        const p = panel();
        const list = findChild(p, "logList");
        const close = findChild(p, "logClose");
        tryVerify(() => list.activeFocus);
        keyClick(Qt.Key_Tab);
        verify(close.activeFocus);
        keyClick(Qt.Key_Tab);
        verify(list.activeFocus);
        keyClick(Qt.Key_Backtab);
        verify(close.activeFocus);
    }

    function test_a_click_on_a_row_shows_it_whole() {
        const p = panel();
        const list = findChild(p, "logList");
        tryVerify(() => list.activeFocus);
        tryVerify(() => list.itemAtIndex(1) !== null);
        mouseClick(list.itemAtIndex(1));
        compare(p.selected, 1);
        compare(p.selectedSeq, 1);
        compare(p.selectedText, "panicked at <b>x</b>\n\n   0: main");
    }

    // The view follows the end as records come; a scroll up stops it (records keep coming, the view stays), and a scroll
    // back to the end follows again.
    function test_it_follows_the_end_until_the_user_scrolls_away() {
        const p = panel(200);
        const list = findChild(p, "logList");
        tryVerify(() => list.atYEnd);
        verify(p.following);
        record("one more", "");
        tryVerify(() => list.atYEnd, 2000, "a new record keeps the view at the end");
        // The wheel, up: away from the end.
        list.wheelBy(1200, 0, Qt.NoScrollPhase);
        tryVerify(() => !p.following, 2000);
        const at = list.contentY;
        record("while away", "");
        record("while away again", "");
        wait(50);
        verify(!list.atYEnd, "the view stays where the user put it");
        fuzzyCompare(list.contentY, at, 1);
        // Back to the end: following again.
        list.wheelBy(-120000, 0, Qt.NoScrollPhase);
        tryVerify(() => p.following, 2000);
        record("back", "");
        tryVerify(() => list.atYEnd, 2000);
    }

    // A key that moves the view away from the end stops it following, as a wheel does: when the next record comes the view
    // stays where the key put it. (A keyboard move from rest has no movementEnded, so the key itself has to say so.)
    function test_a_key_that_moves_the_view_away_stops_it_following() {
        const p = panel(200);
        const list = findChild(p, "logList");
        tryVerify(() => list.atYEnd);
        tryVerify(() => list.activeFocus);
        verify(p.following);
        // The current row is 0, above the view: Down moves to row 1, and the view moves with it.
        keyClick(Qt.Key_Down);
        tryVerify(() => !p.following, 2000, "a key that moves the view away stops it following");
        const at = list.contentY;
        record("while away", "");
        wait(50);
        verify(!list.atYEnd, "the view stays where the key put it");
        fuzzyCompare(list.contentY, at, 1);
    }
}
