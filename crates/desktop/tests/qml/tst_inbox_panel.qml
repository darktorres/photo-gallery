// The inbox (InboxPanel.qml): the list has the keys when it opens, a row's click shows its details, Copy, Dismiss and
// Clear all ask for their row (the panel's signals), and Escape and Close dismiss the panel. Run by `just qmltest`.
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
        ListElement { title: "A rating couldn't be saved"; body: "database is locked"; source: "Edits"; level: 2; updated: 1762790400; count: 3; read: false }
        ListElement { title: "Cropped copies saved"; body: "2 saved in <b>D:/out</b>"; source: "Cropping"; level: 0; updated: 1762790300; count: 1; read: true }
    }

    Component {
        id: panelC
        InboxPanel {
            width: tc.width
            height: tc.height
            open: true
            notices: entries
        }
    }

    SignalSpy { id: dismissedSpy; signalName: "dismissed" }
    SignalSpy { id: copySpy; signalName: "copyRequested" }
    SignalSpy { id: dismissSpy; signalName: "dismissRequested" }
    SignalSpy { id: clearSpy; signalName: "clearRequested" }

    function panel() {
        const p = createTemporaryObject(panelC, tc)
        verify(p !== null)
        for (const spy of [dismissedSpy, copySpy, dismissSpy, clearSpy]) {
            spy.target = p
            spy.clear()
        }
        return p
    }

    function test_the_list_has_the_keys_when_it_opens() {
        const p = panel()
        const list = findChild(p, "inboxList")
        tryVerify(() => list.activeFocus)
        compare(list.count, 2)
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
}