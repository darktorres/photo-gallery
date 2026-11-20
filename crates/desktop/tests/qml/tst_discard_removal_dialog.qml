// DiscardRemovalDialog (the confirmation before a removal kept for an unfinished write is given up): it names the folder,
// and only its "Discard changes" button confirms. Cancel has the focus when it opens, so Enter does not confirm; Escape and
// Cancel close it with no choice. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "DiscardRemovalDialog"
    // Items created under a non-visible TestCase are not visible, so no key or click reaches them (see
    // tst_folder_picker_dialog).
    visible: true
    when: windowShown
    width: 800
    height: 600

    Component {
        id: dialogC
        DiscardRemovalDialog {}
    }

    // A signal spy, made per test by its target and signal name.
    Component {
        id: spyC
        SignalSpy {}
    }

    // The dialog, for `folder` (the path the panel shows), open and with its focus where it opens.
    function shown(folder) {
        const d = createTemporaryObject(dialogC, tc);
        verify(d !== null);
        d.folder = folder;
        d.open();
        tryVerify(() => d.opened);
        return d;
    }

    function test_the_folder_is_named() {
        const d = shown("E:\\Archive");
        compare(d.folderText, "E:\\Archive");
    }

    function test_cancel_has_the_focus_when_it_opens() {
        const d = shown("E:\\Archive");
        tryVerify(() => d.cancelButton.activeFocus);
        verify(!d.discardButton.activeFocus);
    }

    function test_enter_does_not_confirm() {
        const d = shown("E:\\Archive");
        const confirmed = createTemporaryObject(spyC, tc, {
            target: d,
            signalName: "confirmed"
        });
        tryVerify(() => d.cancelButton.activeFocus);
        keyClick(Qt.Key_Return);
        keyClick(Qt.Key_Enter);
        compare(confirmed.count, 0);
    }

    function test_cancel_closes_it_without_a_choice() {
        const d = shown("E:\\Archive");
        const confirmed = createTemporaryObject(spyC, tc, {
            target: d,
            signalName: "confirmed"
        });
        waitForRendering(d.cancelButton);
        mouseClick(d.cancelButton);
        tryVerify(() => !d.opened);
        compare(confirmed.count, 0);
    }

    function test_escape_closes_it_without_a_choice() {
        const d = shown("E:\\Archive");
        const confirmed = createTemporaryObject(spyC, tc, {
            target: d,
            signalName: "confirmed"
        });
        tryVerify(() => d.cancelButton.activeFocus);
        keyClick(Qt.Key_Escape);
        tryVerify(() => !d.opened);
        compare(confirmed.count, 0);
    }

    function test_discard_confirms_exactly_once() {
        const d = shown("E:\\Archive");
        const confirmed = createTemporaryObject(spyC, tc, {
            target: d,
            signalName: "confirmed"
        });
        waitForRendering(d.discardButton);
        mouseClick(d.discardButton);
        tryVerify(() => !d.opened);
        compare(confirmed.count, 1);
    }
}
