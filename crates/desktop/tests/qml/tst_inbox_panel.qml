// The inbox (InboxPanel.qml): the list has the keys when it opens, a row's click shows its details, Copy, Dismiss and
// Clear all ask for their row (the panel's signals), a ledger's row lists its files with Reveal and offers Retry, and
// Escape and Close dismiss the panel. Run by `just qmltest`.
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
        ListElement { title: "A rating couldn't be saved"; body: "database is locked"; source: "Edits"; level: 2; updated: 1762790400; count: 3; read: false; pass: -1; canRetry: false; canShowFiles: false }
        ListElement { title: "Cropped copies saved"; body: "2 saved in <b>D:/out</b>"; source: "Cropping"; level: 0; updated: 1762790300; count: 1; read: true; pass: -1; canRetry: false; canShowFiles: false }
        ListElement { title: "Thumbnails couldn't be made for 2 files"; body: "2 files"; source: "Thumbnails"; level: 1; updated: 1762790200; count: 2; read: false; pass: 0; canRetry: true; canShowFiles: false }
    }
    // The failed files, as FailedFiles has them (src/failed_files.rs).
    ListModel {
        id: files
        ListElement { path: "D:/photos/a.jpg"; error: "not a JPEG"; fileId: 7; firstSeen: 1762790200; isNew: true; retried: true }
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
        const p = createTemporaryObject(panelC, tc)
        verify(p !== null)
        for (const spy of [dismissedSpy, copySpy, dismissSpy, clearSpy, retrySpy, revealSpy, filesSpy]) {
            spy.target = p
            spy.clear()
        }
        return p
    }

    function test_the_list_has_the_keys_when_it_opens() {
        const p = panel()
        const list = findChild(p, "inboxList")
        tryVerify(() => list.activeFocus)
        compare(list.count, 3)
    }

    function test_escape_and_close_dismiss_it() {
        const p = panel()
        tryVerify(() => findChild(p, "inboxList").activeFocus)
        keyClick(Qt.Key_Escape)
        compare(dismissedSpy.count, 1)
        mouseClick(findChild(p, "inboxClose"))
        compare(dismissedSpy.count, 2)
    }

    function test_a_click_shows_the_details_and_dismiss_asks_the_model() {
        const p = panel()
        const list = findChild(p, "inboxList")
        tryVerify(() => list.itemAtIndex(1) !== null)
        mouseClick(list.itemAtIndex(1))
        compare(p.selected, 1)
        compare(p.selectedText, "2 saved in <b>D:/out</b>")
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

    function test_a_ledger_lists_its_files_with_reveal_and_offers_retry() {
        const p = panel()
        const list = findChild(p, "inboxList")
        tryVerify(() => list.itemAtIndex(2) !== null)
        verify(!findChild(p, "inboxRetry").visible)
        mouseClick(list.itemAtIndex(2))
        compare(p.selectedPass, 0)
        compare(filesSpy.signalArguments[filesSpy.count - 1][0], 0)
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
        compare(p.selected, -1)
    }

    function test_an_expanded_row_is_shown_when_it_opens() {
        const p = panel()
        p.expandRow = 2
        compare(p.selected, 2)
        // The row tells its pass once its delegate is made.
        tryCompare(p, "selectedPass", 0)
    }
}