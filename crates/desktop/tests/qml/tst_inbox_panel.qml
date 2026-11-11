// The inbox (InboxPanel.qml): the list has the keys when it opens, and they drive it (Up, Down, Home, End, Page Up and
// Page Down move; Enter and Space open and close the details; Delete dismisses; Ctrl+C copies; R retries); a row's click
// shows its details; Copy, Dismiss and Clear all ask for their row (the panel's signals); a ledger's row lists its files
// with Reveal (loading until they are read), and offers Retry; the selection and the open details stay with their row
// when a notice comes in above it; a ledger's entry to expand opens when the panel opens and when it appears later;
// Tab wraps within the panel; Escape and Close dismiss it. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "InboxPanel"
    // Items created under a non-visible TestCase are not visible, so no key reaches them (see tst_wheellistview).
    visible: true
    when: windowShown
    width: 1000
    height: 800

    // The model's roles, as Notices has them (src/notices.rs).
    ListModel {
        id: entries
    }
    // The rows each test starts from.
    function fill() {
        entries.clear()
        entries.append({ title: "A rating couldn't be saved", body: "database is locked", source: "Edits", level: 2, updated: 1762790400, count: 3, read: false, pass: -1, canRetry: false, canShowFiles: false, key: "row:1:1", sinceDismissed: "", serial: 0 })
        entries.append({ title: "Cropped copies saved", body: "2 saved in <b>D:/out</b>", source: "Cropping", level: 0, updated: 1762790300, count: 1, read: true, pass: -1, canRetry: false, canShowFiles: false, key: "row:2:2", sinceDismissed: "", serial: 0 })
        entries.append({ title: "Thumbnails couldn't be made", body: "1 new, 1 earlier", source: "Thumbnails", level: 1, updated: 1762790200, count: 2, read: false, pass: 0, canRetry: true, canShowFiles: false, key: "ledger:thumbs", sinceDismissed: "1 since you dismissed it", serial: 40 })
    }
    // The failed files, as FailedFiles has them (src/failed_files.rs).
    ListModel {
        id: files
        ListElement { path: "D:/photos/a.jpg"; error: "not a JPEG"; fileId: 7; firstSeen: 1762790200; isNew: true; retried: true; root: ""; reason: ""; isFolder: false }
    }

    Component {
        id: panelC
        InboxPanel {
            width: tc.width
            height: tc.height
            open: true
            notices: entries
            failedFiles: files
            failedTotal: 2
            failedLoading: false
            failedError: ""
        }
    }

    SignalSpy { id: dismissedSpy; signalName: "dismissed" }
    SignalSpy { id: copySpy; signalName: "copyRequested" }
    SignalSpy { id: dismissSpy; signalName: "dismissRequested" }
    SignalSpy { id: clearSpy; signalName: "clearRequested" }
    SignalSpy { id: retrySpy; signalName: "retryRequested" }
    SignalSpy { id: revealSpy; signalName: "revealRequested" }
    SignalSpy { id: filesSpy; signalName: "filesRequested" }

    function panel() {
        fill()
        const p = createTemporaryObject(panelC, tc)
        verify(p !== null)
        for (const spy of [dismissedSpy, copySpy, dismissSpy, clearSpy, retrySpy, revealSpy, filesSpy]) {
            spy.target = p
            spy.clear()
        }
        tryVerify(() => findChild(p, "inboxList").activeFocus)
        return p
    }

    function test_the_list_has_the_keys_when_it_opens() {
        const p = panel()
        const list = findChild(p, "inboxList")
        compare(list.count, 3)
        compare(p.selected, -1, "nothing is selected until the user moves")
    }

    function test_escape_and_close_dismiss_it() {
        const p = panel()
        keyClick(Qt.Key_Escape)
        compare(dismissedSpy.count, 1)
        mouseClick(findChild(p, "inboxClose"))
        compare(dismissedSpy.count, 2)
    }

    function test_tab_wraps_within_the_panel() {
        const p = panel()
        const list = findChild(p, "inboxList")
        const close = findChild(p, "inboxClose")
        keyClick(Qt.Key_Backtab)
        verify(close.activeFocus)
        keyClick(Qt.Key_Tab)
        verify(list.activeFocus)
        // Forward from the list, through the panel's buttons, and round to the list again: never out of the panel.
        for (let i = 0; i < 8 && !close.activeFocus; i++)
            keyClick(Qt.Key_Tab)
        verify(close.activeFocus)
        keyClick(Qt.Key_Tab)
        verify(list.activeFocus)
    }

    function test_a_click_shows_the_details_and_dismiss_asks_the_model() {
        const p = panel()
        const list = findChild(p, "inboxList")
        tryVerify(() => list.itemAtIndex(1) !== null)
        mouseClick(list.itemAtIndex(1))
        compare(p.selected, 1)
        verify(p.detailsOpen)
        tryCompare(p, "selectedKey", "row:2:2")
        tryCompare(p, "selectedText", "2 saved in <b>D:/out</b>")
        verify(findChild(p, "inboxDetails").visible)
        mouseClick(findChild(p, "inboxCopy"))
        compare(copySpy.count, 1)
        compare(copySpy.signalArguments[0][0], 1)
        mouseClick(findChild(p, "inboxDismiss"))
        compare(dismissSpy.count, 1)
        compare(dismissSpy.signalArguments[0][0], 1)
        compare(p.selected, -1)
        mouseClick(findChild(p, "inboxClear"))
        compare(clearSpy.count, 1)
    }

    // The keys move the selection and stop at the ends; Enter and Space open and close the details.
    function test_the_keys_move_the_selection_and_open_the_details() {
        const p = panel()
        keyClick(Qt.Key_Down)
        compare(p.selected, 0)
        tryCompare(p, "selectedKey", "row:1:1")
        keyClick(Qt.Key_Down)
        keyClick(Qt.Key_Down)
        keyClick(Qt.Key_Down)
        compare(p.selected, 2, "the last row stays the last")
        keyClick(Qt.Key_Home)
        compare(p.selected, 0)
        keyClick(Qt.Key_End)
        compare(p.selected, 2)
        keyClick(Qt.Key_Up)
        compare(p.selected, 1)
        keyClick(Qt.Key_PageUp)
        compare(p.selected, 0)
        keyClick(Qt.Key_PageDown)
        compare(p.selected, 2)
        verify(!p.detailsOpen)
        keyClick(Qt.Key_Return)
        verify(p.detailsOpen)
        keyClick(Qt.Key_Space)
        verify(!p.detailsOpen)
        keyClick(Qt.Key_Enter)
        verify(p.detailsOpen)
    }

    // Delete dismisses the selected row, Ctrl+C copies it, R retries it when its entry can be retried.
    function test_the_keys_dismiss_copy_and_retry() {
        const p = panel()
        keyClick(Qt.Key_R)
        compare(retrySpy.count, 0, "nothing selected: nothing to retry")
        keyClick(Qt.Key_End)
        keyClick(Qt.Key_C, Qt.ControlModifier)
        compare(copySpy.count, 1)
        compare(copySpy.signalArguments[0][0], 2)
        keyClick(Qt.Key_R)
        compare(retrySpy.count, 1)
        compare(retrySpy.signalArguments[0][0], 2)
        keyClick(Qt.Key_Home)
        keyClick(Qt.Key_R)
        compare(retrySpy.count, 1, "a row that can't be retried")
        keyClick(Qt.Key_Delete)
        compare(dismissSpy.count, 1)
        compare(dismissSpy.signalArguments[0][0], 0)
    }

    function test_a_ledger_lists_its_files_with_reveal_and_offers_retry() {
        const p = panel()
        const list = findChild(p, "inboxList")
        tryVerify(() => list.itemAtIndex(2) !== null)
        verify(!findChild(p, "inboxRetry").visible)
        mouseClick(list.itemAtIndex(2))
        tryCompare(p, "selectedPass", 0)
        compare(filesSpy.count, 1)
        compare(filesSpy.signalArguments[0][0], 0)
        verify(findChild(p, "inboxLedger").visible)
        verify(findChild(p, "inboxRetry").visible)
        verify(!findChild(p, "inboxShowFiles").visible)
        // The files' list is laid out once it shows.
        tryVerify(() => findChild(p, "inboxReveal0") !== null && findChild(p, "inboxReveal0").width > 0)
        waitForRendering(p)
        mouseClick(findChild(p, "inboxReveal0"))
        compare(revealSpy.count, 1)
        compare(revealSpy.signalArguments[0][0], 7)
        mouseClick(findChild(p, "inboxRetry"))
        compare(retrySpy.count, 1)
        compare(retrySpy.signalArguments[0][0], 2)
        // The entry stays (the next listing says what still fails), and its list is not asked for again until its
        // newest failure moves.
        compare(p.selected, 2)
        compare(filesSpy.count, 1)
        entries.setProperty(2, "serial", 41)
        compare(filesSpy.count, 2, "a newer failure: the list is read again")
        compare(filesSpy.signalArguments[1][0], 0)
        // Closing the details lets the list go (the keys are the list's again: the click gave them to the button).
        list.forceActiveFocus()
        keyClick(Qt.Key_Space)
        compare(filesSpy.signalArguments[filesSpy.count - 1][0], -1)
    }

    // "Loading" shows until the ledger's list is delivered, and an error when its read fails.
    function test_a_ledgers_list_says_it_is_loading_and_why_it_failed() {
        const p = panel()
        const list = findChild(p, "inboxList")
        tryVerify(() => list.itemAtIndex(2) !== null)
        mouseClick(list.itemAtIndex(2))
        verify(!findChild(p, "inboxLoading").visible)
        p.failedLoading = true
        verify(findChild(p, "inboxLoading").visible)
        p.failedLoading = false
        p.failedError = "database is locked"
        const error = findChild(p, "inboxFilesError")
        verify(error.visible)
        // The read's error, worded (the stub of Words answers with the id).
        compare(error.text, "ui-inbox-unreadable")
    }

    // A notice that comes in above the selected row keeps the selection on its entry, with its details open.
    function test_the_selection_stays_with_its_entry_across_an_update() {
        const p = panel()
        const list = findChild(p, "inboxList")
        tryVerify(() => list.itemAtIndex(1) !== null)
        mouseClick(list.itemAtIndex(1))
        tryCompare(p, "selectedKey", "row:2:2")
        entries.insert(0, { title: "New", body: "new", source: "App", level: 0, updated: 1762790500, count: 1, read: false, pass: -1, canRetry: false, canShowFiles: false, key: "row:3:3", sinceDismissed: "", serial: 0 })
        compare(p.selected, 2)
        tryCompare(p, "selectedKey", "row:2:2")
        verify(p.detailsOpen)
        tryCompare(p, "selectedText", "2 saved in <b>D:/out</b>")
        // A row moved above it keeps it too, and its entry changed in place shows the change.
        entries.move(3, 0, 1)
        tryCompare(p, "selectedKey", "row:2:2")
        entries.setProperty(p.selected, "body", "3 saved")
        tryCompare(p, "selectedText", "3 saved")
    }

    function test_an_expanded_row_is_shown_when_it_opens() {
        const p = panel()
        p.expandRow = 2
        compare(p.selected, 2)
        verify(p.detailsOpen)
        // The row tells its pass once its delegate is made.
        tryCompare(p, "selectedPass", 0)
    }

    // Once opened, the entry's row moving (a notice coming in above it) doesn't take the selection back to it.
    function test_an_expanded_row_that_moves_leaves_the_selection_alone() {
        const p = panel()
        p.expandRow = 2
        compare(p.selected, 2)
        keyClick(Qt.Key_Home)
        tryCompare(p, "selectedKey", "row:1:1")
        entries.insert(0, { title: "New", body: "new", source: "App", level: 0, updated: 1762790500, count: 1, read: false, pass: -1, canRetry: false, canShowFiles: false, key: "row:3:3", sinceDismissed: "", serial: 0 })
        p.expandRow = 3
        compare(p.selected, 1)
        tryCompare(p, "selectedKey", "row:1:1")
        // The entry gone and back: it opens again.
        p.expandRow = -1
        p.expandRow = 3
        compare(p.selected, 3)
    }

    // The entry to expand is not listed yet: nothing opens; when it appears (Main finds its row then), it opens.
    function test_an_entry_to_expand_opens_when_it_appears() {
        const p = panel()
        p.expandRow = -1
        compare(p.selected, -1)
        entries.append({ title: "Files couldn't be imported", body: "1 new, 0 earlier", source: "Import", level: 1, updated: 1762790100, count: 1, read: false, pass: 2, canRetry: false, canShowFiles: true, key: "ledger:import", sinceDismissed: "", serial: 9 })
        p.expandRow = 3
        compare(p.selected, 3)
        tryCompare(p, "selectedPass", 2)
        verify(findChild(p, "inboxShowFiles").visible)
        compare(filesSpy.signalArguments[filesSpy.count - 1][0], 2)
    }
}
