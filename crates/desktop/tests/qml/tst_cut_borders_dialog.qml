// CutBordersDialog (the confirmation before the borders come off the originals): it says how many files, and only its
// "Cut borders" button confirms. Cancel has the focus when it opens, so Enter does not confirm; Escape and Cancel close it
// with no choice. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "CutBordersDialog"
    // Items created under a non-visible TestCase are not visible, so no key or click reaches them (see
    // tst_folder_picker_dialog).
    visible: true
    when: windowShown
    width: 800
    height: 600

    Component {
        id: dialogC
        CutBordersDialog {}
    }

    // A signal spy, made per test by its target and signal name.
    Component {
        id: spyC
        SignalSpy {}
    }

    // The dialog, for `count` files, open and with its focus where it opens.
    function shown(count) {
        const d = createTemporaryObject(dialogC, tc)
        verify(d !== null)
        d.count = count
        d.open()
        tryVerify(() => d.opened)
        return d
    }

    function test_the_question_says_how_many_files() {
        const d = shown(12)
        compare(d.questionText, "ui-cut-borders-question 12")
    }

    function test_cancel_has_the_focus_when_it_opens() {
        const d = shown(12)
        tryVerify(() => d.cancelButton.activeFocus)
        verify(!d.cutButton.activeFocus)
    }

    function test_enter_does_not_confirm() {
        const d = shown(12)
        const confirmed = createTemporaryObject(spyC, tc, { target: d, signalName: "confirmed" })
        tryVerify(() => d.cancelButton.activeFocus)
        keyClick(Qt.Key_Return)
        keyClick(Qt.Key_Enter)
        compare(confirmed.count, 0)
    }

    function test_cancel_closes_it_without_a_choice() {
        const d = shown(12)
        const confirmed = createTemporaryObject(spyC, tc, { target: d, signalName: "confirmed" })
        waitForRendering(d.cancelButton)
        mouseClick(d.cancelButton)
        tryVerify(() => !d.opened)
        compare(confirmed.count, 0)
    }

    function test_escape_closes_it_without_a_choice() {
        const d = shown(12)
        const confirmed = createTemporaryObject(spyC, tc, { target: d, signalName: "confirmed" })
        tryVerify(() => d.cancelButton.activeFocus)
        keyClick(Qt.Key_Escape)
        tryVerify(() => !d.opened)
        compare(confirmed.count, 0)
    }

    function test_cut_borders_confirms_exactly_once() {
        const d = shown(12)
        const confirmed = createTemporaryObject(spyC, tc, { target: d, signalName: "confirmed" })
        waitForRendering(d.cutButton)
        mouseClick(d.cutButton)
        tryVerify(() => !d.opened)
        compare(confirmed.count, 1)
    }
}
