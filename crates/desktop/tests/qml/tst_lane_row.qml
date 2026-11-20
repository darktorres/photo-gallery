// One lane's row (LaneRow.qml), alone: its Pause sends the state it is not showing; its bar is indeterminate only for a
// running lane with no known total (not when the lane is paused); a row with no work or no Pause keeps
// its cells, so the rest of the row stays where it is; and its Pause is named for its drive. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "LaneRow"
    visible: true
    when: windowShown
    width: 1000
    height: 200

    Component {
        id: rowC
        LaneRow {
            index: 0
            number: 1
            pausable: true
            label: "E:"
            status: "Importing 1 of 4"
            paused: false
            working: true
            progress: 0.25
            indeterminate: false
            failed: ""
            failedPass: -1
            inboxPasses: 0
        }
    }

    SignalSpy {
        id: pauseSpy
        signalName: "pauseRequested"
    }
    SignalSpy {
        id: failedSpy
        signalName: "failedRequested"
    }

    function rowOf(props: var): var {
        const row = createTemporaryObject(rowC, tc, props);
        verify(row !== null);
        pauseSpy.target = row;
        pauseSpy.clear();
        failedSpy.target = row;
        failedSpy.clear();
        waitForRendering(row);
        return row;
    }

    // The Pause sends the opposite of what it shows: a running lane is paused, a paused one resumed.
    function test_the_pause_sends_the_state_it_is_not_showing() {
        const running = rowOf({
            paused: false
        });
        mouseClick(findChild(running, "laneToggle"));
        compare(pauseSpy.count, 1);
        compare(pauseSpy.signalArguments[0][0], true, "a running lane is to be paused");
        const paused = rowOf({
            paused: true
        });
        mouseClick(findChild(paused, "laneToggle"));
        compare(pauseSpy.count, 1);
        compare(pauseSpy.signalArguments[0][0], false, "a paused lane is to be resumed");
    }

    // Indeterminate: a running lane whose total isn't known; never a paused lane.
    function test_the_bar_is_indeterminate_only_for_a_running_unknown_total() {
        const running = rowOf({
            indeterminate: true
        });
        verify(findChild(running, "laneBar").indeterminate);
        const paused = rowOf({
            indeterminate: true,
            paused: true
        });
        verify(!findChild(paused, "laneBar").indeterminate, "a paused lane's bar stops");
    }

    // A row with no work has no bar, and a row with no Pause has no button, but their cells keep their places: the status
    // text and the Pause beside it don't move when a part is hidden.
    function test_hidden_parts_keep_their_cells() {
        const row = rowOf({});
        const toggleX = findChild(row, "laneToggle").parent.mapToItem(row, 0, 0).x;
        const statusX = findChild(row, "laneStatus").mapToItem(row, 0, 0).x;
        verify(findChild(row, "laneBar").visible);
        row.working = false;
        waitForRendering(row);
        verify(!findChild(row, "laneBar").visible, "no work: no bar");
        compare(findChild(row, "laneToggle").parent.mapToItem(row, 0, 0).x, toggleX, "the Pause keeps its cell");
        compare(findChild(row, "laneStatus").mapToItem(row, 0, 0).x, statusX, "the status keeps its cell");
        row.pausable = false;
        waitForRendering(row);
        verify(!findChild(row, "laneToggle").visible, "no lane's own pause: no button");
        compare(findChild(row, "laneToggle").parent.mapToItem(row, 0, 0).x, toggleX, "the cell is still there");
    }

    // The button's name carries the drive, so each row's Pause says which drive it pauses; with no drive it is plain.
    // (The stub of Words answers a phrase with its id and value, and text with its id.)
    function test_the_pause_is_named_for_its_drive() {
        const named = rowOf({
            label: "E:"
        });
        compare(findChild(named, "laneToggle").Accessible.name, "ui-pause-lane:E:");
        const resumed = rowOf({
            label: "E:",
            paused: true
        });
        compare(findChild(resumed, "laneToggle").Accessible.name, "ui-resume-lane:E:");
        const plain = rowOf({
            label: ""
        });
        compare(findChild(plain, "laneToggle").Accessible.name, "ui-pause");
    }

    // The failed count is a link to its ledger only while the inbox lists that ledger (its bit of `inboxPasses`): then it
    // is underlined, a link to accessibility, and a click opens the ledger; otherwise it is plain text, and a click does
    // nothing.
    function test_the_failed_count_links_only_with_its_ledgers_bit() {
        const row = rowOf({
            failed: "2 failed",
            failedPass: 2,
            inboxPasses: 0
        });
        const failed = findChild(row, "laneFailed");
        compare(failed.text, "2 failed");
        verify(!failed.font.underline);
        compare(failed.Accessible.role, Accessible.StaticText);
        waitForRendering(row);
        mouseClick(failed);
        compare(failedSpy.count, 0, "nothing listed: the click does nothing");
        // The inbox lists the ledger now: the same words become the link.
        row.inboxPasses = 1 << 2;
        verify(failed.font.underline);
        compare(failed.Accessible.role, Accessible.Link);
        mouseClick(failed);
        compare(failedSpy.count, 1);
        compare(failedSpy.signalArguments[0][0], 2, "the click names the ledger");
    }

    // A row with no failures shows no count, and a crop's failures are listed under no ledger, so they never link.
    function test_no_failures_show_no_count_and_a_crop_never_links() {
        const none = rowOf({
            failed: "",
            failedPass: -1
        });
        compare(findChild(none, "laneFailed").text, "");
        const crop = rowOf({
            failed: "1 failed",
            failedPass: -1,
            inboxPasses: 0xFFFF
        });
        verify(!findChild(crop, "laneFailed").font.underline, "a ledger of -1 is no link");
        mouseClick(findChild(crop, "laneFailed"));
        compare(failedSpy.count, 0);
    }
}
