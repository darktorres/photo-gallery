// FolderPickerDialog (the app's own folder dialog, on the systems without the system's): it shows the folders and the
// folder it is in that its owner gives it, and reports each choice as its signal, closing on a choice or a cancel. The
// choices themselves are the picker's (Rust); these tests check only what the dialog shows and reports. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "FolderPickerDialog"
    // Items created under a non-visible TestCase are not visible: the list would make no delegates.
    visible: true
    when: windowShown

    Component {
        id: dialogC
        FolderPickerDialog {
            width: 800
            height: 700
            open: true
            names: ["Alpha", "Beta"]
            here: "C:\\Pics"
        }
    }

    // A signal spy, made per test by its target and signal name.
    Component {
        id: spyC
        SignalSpy {}
    }

    // The first item under `item` with this objectName, or null.
    function find(item, name) {
        if (!item)
            return null
        if (item.objectName === name)
            return item
        const kids = (item.children || []).concat(item.contentItem ? [item.contentItem] : [])
        for (const kid of kids) {
            const found = find(kid, name)
            if (found)
                return found
        }
        return null
    }

    // The dialog as the test needs it: shown, laid out, with its listing made.
    function shown() {
        const d = createTemporaryObject(dialogC, tc)
        waitForRendering(d)
        return d
    }

    function test_the_folder_it_is_in_and_its_folders_are_shown() {
        const d = shown()
        compare(find(d, "here").text, "C:\\Pics")
        const list = find(d, "folders")
        compare(list.count, 2)
        compare(find(list.itemAtIndex(0), "open").text, "Alpha")
        compare(find(list.itemAtIndex(1), "open").text, "Beta")
    }

    function test_choosing_here_reports_it_and_closes() {
        const d = shown()
        const chosen = createTemporaryObject(spyC, tc, { target: d, signalName: "chooseHere" })
        mouseClick(find(d, "chooseHere"))
        compare(chosen.count, 1)
        compare(d.open, false)
    }

    function test_a_folder_opens_by_its_row_and_stays_open() {
        const d = shown()
        const opened = createTemporaryObject(spyC, tc, { target: d, signalName: "openRow" })
        mouseClick(find(find(d, "folders").itemAtIndex(1), "open"))
        compare(opened.count, 1)
        compare(opened.signalArguments[0][0], 1, "the row that was clicked")
        compare(d.open, true, "opening a folder does not close the dialog")
    }

    function test_choose_on_a_row_reports_that_row_and_closes() {
        const d = shown()
        const chosen = createTemporaryObject(spyC, tc, { target: d, signalName: "chooseRow" })
        mouseClick(find(find(d, "folders").itemAtIndex(0), "choose"))
        compare(chosen.count, 1)
        compare(chosen.signalArguments[0][0], 0)
        compare(d.open, false)
    }

    function test_up_goes_up_and_cancel_closes_without_a_choice() {
        const d = shown()
        const up = createTemporaryObject(spyC, tc, { target: d, signalName: "goUp" })
        const cancelled = createTemporaryObject(spyC, tc, { target: d, signalName: "cancelled" })
        const chosen = createTemporaryObject(spyC, tc, { target: d, signalName: "chooseHere" })
        mouseClick(find(d, "up"))
        compare(up.count, 1)
        compare(d.open, true, "going up does not close the dialog")
        mouseClick(find(d, "cancel"))
        compare(cancelled.count, 1)
        compare(chosen.count, 0, "a cancel is no choice")
        compare(d.open, false)
    }

    function test_dismissing_the_dialog_is_a_cancel() {
        const d = shown()
        const cancelled = createTemporaryObject(spyC, tc, { target: d, signalName: "cancelled" })
        d.dismissed()
        compare(cancelled.count, 1)
        compare(d.open, false)
    }
}
