// The confirmation dialogs Main declares directly under its window (RemoveFolderDialog, DiscardRemovalDialog, CutBordersDialog)
// open centred on the window's content, as a dialog should. Declared here as Main declares them, under an ApplicationWindow,
// not under a test item: the test items would centre them by accident. Run by `just qmltest`.
import QtQuick
import QtQuick.Controls
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "DialogPlacement"
    visible: true
    when: windowShown
    width: 800
    height: 600

    ApplicationWindow {
        id: win
        width: 900
        height: 700
        visible: true

        RemoveFolderDialog {
            id: removeDialog
            folder: "D:/Photos"
        }
        DiscardRemovalDialog {
            id: discardDialog
            folder: "D:/Photos"
        }
        CutBordersDialog {
            id: cutDialog
            count: 3
        }
    }

    // The dialog's position is the centre of the window's content, to within a pixel.
    function verifyCentred(d) {
        const host = win.contentItem;
        verify(Math.abs(d.x - (host.width - d.width) / 2) < 1, "x is " + d.x + ", the window's centre is " + (host.width - d.width) / 2);
        verify(Math.abs(d.y - (host.height - d.height) / 2) < 1, "y is " + d.y + ", the window's centre is " + (host.height - d.height) / 2);
    }

    function test_remove_folder_dialog_opens_centred() {
        removeDialog.open();
        tryVerify(() => removeDialog.opened);
        verifyCentred(removeDialog);
        removeDialog.close();
    }

    function test_discard_removal_dialog_opens_centred() {
        discardDialog.open();
        tryVerify(() => discardDialog.opened);
        verifyCentred(discardDialog);
        discardDialog.close();
    }

    function test_cut_borders_dialog_opens_centred() {
        cutDialog.open();
        tryVerify(() => cutDialog.opened);
        verifyCentred(cutDialog);
        cutDialog.close();
    }
}
