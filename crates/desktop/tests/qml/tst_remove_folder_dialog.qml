// RemoveFolderDialog (the confirmation before a folder leaves the library for good): it names the folder, and only its
// "Remove" button confirms. Cancel has the focus when it opens, so Enter does not confirm; Escape and Cancel close it with
// no choice. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "RemoveFolderDialog"
    // Items created under a non-visible TestCase are not visible, so no key or click reaches them (see
    // tst_folder_picker_dialog).
    visible: true
    when: windowShown
    width: 800
    height: 600

    Component {
        id: dialogC
        RemoveFolderDialog {}
    }

    // A signal spy, made per test by its target and signal name.
    Component {
        id: spyC
        SignalSpy {}
    }

    // The dialog, for `folder` (the path the panel shows), open and with its focus where it opens.
    function shown(folder) {
        const d = createTemporaryObject(dialogC, tc)
        verify(d !== null)
        d.folder = folder
        d.open()
        tryVerify(() => d.opened)
        return d
    }

    function test_the_folder_is_named() {
        const d = shown("D:\\Photos\\Trips")
        compare(d.folderText, "D:\\Photos\\Trips")
    }

    // The question is laid out under the folder's name, not over it, and inside the dialog's height.
    function test_the_question_sits_under_the_folder() {
        const d = shown("D:\\Photos\\Trips")
        verify(d.questionLabel.y >= d.folderLabelItem.y + d.folderLabelItem.height)
    }

    function test_cancel_has_the_focus_when_it_opens() {
        const d = shown("D:\\Photos")
        tryVerify(() => d.cancelButton.activeFocus)
        verify(!d.removeButton.activeFocus)
    }

    function test_enter_does_not_confirm() {
        const d = shown("D:\\Photos")
        const confirmed = createTemporaryObject(spyC, tc, { target: d, signalName: "confirmed" })
        tryVerify(() => d.cancelButton.activeFocus)
        keyClick(Qt.Key_Return)
        keyClick(Qt.Key_Enter)
        compare(confirmed.count, 0)
    }

    function test_cancel_closes_it_without_a_choice() {
        const d = shown("D:\\Photos")
        const confirmed = createTemporaryObject(spyC, tc, { target: d, signalName: "confirmed" })
        waitForRendering(d.cancelButton)
        mouseClick(d.cancelButton)
        tryVerify(() => !d.opened)
        compare(confirmed.count, 0)
    }

    function test_escape_closes_it_without_a_choice() {
        const d = shown("D:\\Photos")
        const confirmed = createTemporaryObject(spyC, tc, { target: d, signalName: "confirmed" })
        tryVerify(() => d.cancelButton.activeFocus)
        keyClick(Qt.Key_Escape)
        tryVerify(() => !d.opened)
        compare(confirmed.count, 0)
    }

    function test_remove_confirms_exactly_once() {
        const d = shown("D:\\Photos")
        const confirmed = createTemporaryObject(spyC, tc, { target: d, signalName: "confirmed" })
        waitForRendering(d.removeButton)
        mouseClick(d.removeButton)
        tryVerify(() => !d.opened)
        compare(confirmed.count, 1)
    }
}
