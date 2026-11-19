// The status bar (StatusBar.qml): the pass's "N failed" is a link to the inbox only while the inbox lists something of
// the pass (Notices' `passes` bit, `failedLinked`): then it is underlined, a link to accessibility, and a click asks for
// the failed files; otherwise it is plain text, a click does nothing, and its tooltip says it isn't recorded yet. An info
// toast goes by itself unless the screenshots hold it. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "StatusBar"
    visible: true
    when: windowShown
    width: 1000
    height: 200

    Component {
        id: barC
        StatusBar {
            selection: ""
            writeStatus: ""
            writeFailed: false
            toast: ""
            toastLevel: 0
            unread: 0
            status: "Importing 1 of 4"
            stacked: false
            indeterminate: false
            failed: "2 failed"
            failedLinked: false
            busy: true
            paused: false
            progress: 0.25
            devMode: false
        }
    }

    // The lanes' rows, as the LaneModel's roles: a list of them is set per test.
    Component {
        id: laneListC
        ListModel {}
    }

    SignalSpy { id: failedSpy; signalName: "failedRequested" }
    SignalSpy { id: pauseSpy; signalName: "lanePauseRequested" }

    // A bar with the given rows, stacked or not, and the rows' pause clicks counted.
    function stackedBar(rows: var, stacked: bool): var {
        const model = createTemporaryObject(laneListC, tc)
        verify(model !== null)
        for (const row of rows)
            model.append(row)
        const b = createTemporaryObject(barC, tc, { stacked: stacked, laneModel: model, busy: true })
        verify(b !== null)
        pauseSpy.target = b
        pauseSpy.clear()
        waitForRendering(b)
        return b
    }

    function rowOf(b, at): var {
        const column = findChild(b, "laneRepeater")
        return column.itemAt(at)
    }

    function bar(linked: bool): var {
        const b = createTemporaryObject(barC, tc, { failedLinked: linked })
        verify(b !== null)
        failedSpy.target = b
        failedSpy.clear()
        return b
    }

    function test_the_failed_count_links_only_with_the_passs_bit() {
        const b = bar(false)
        const failed = findChild(b, "statusFailed")
        verify(failed.visible)
        verify(!failed.font.underline)
        compare(failed.Accessible.role, Accessible.StaticText)
        waitForRendering(b)
        mouseClick(failed)
        compare(failedSpy.count, 0, "nothing listed: the click does nothing")
        // The inbox lists the pass's ledger now: the same words become the link.
        b.failedLinked = true
        verify(failed.font.underline)
        compare(failed.Accessible.role, Accessible.Link)
        mouseClick(failed)
        compare(failedSpy.count, 1)
        // And back, when the entry goes.
        b.failedLinked = false
        mouseClick(failed)
        compare(failedSpy.count, 1)
    }

    function test_the_link_starts_live_when_the_bit_is_set() {
        const b = bar(true)
        const failed = findChild(b, "statusFailed")
        verify(failed.font.underline)
        waitForRendering(b)
        mouseClick(failed)
        compare(failedSpy.count, 1)
    }

    function test_no_failures_show_no_count() {
        const b = bar(true)
        b.failed = ""
        verify(!findChild(b, "statusFailed").visible)
    }

    // An info toast's timer runs, unless the toast is held (the screenshots); a warning's never runs.
    function test_an_info_toast_goes_by_itself_unless_held() {
        const b = bar(false)
        const timer = findChild(b, "toastTimer")
        verify(!timer.running)
        b.toast = "Cropped copies saved"
        verify(timer.running)
        b.toastHeld = true
        verify(!timer.running)
        b.toastHeld = false
        b.toastLevel = 1
        verify(!timer.running)
    }

    // Stacked: a row for each lane, each with its own bar and its own Pause, one under the other, and no shared bar. The
    // Pause names its lane by its number.
    function test_stacked_rows_each_have_their_own_bar_and_pause_one_under_another() {
        const b = stackedBar([
            { number: 1, pausable: true, label: "E:", status: "Importing 1 of 0", paused: false, working: true,
              progress: 0, indeterminate: true },
            { number: 2, pausable: true, label: "D:", status: "Paused · Importing 2 of 0", paused: true, working: true,
              progress: 0, indeterminate: true },
        ], true)
        const first = rowOf(b, 0)
        const second = rowOf(b, 1)
        verify(first !== null && second !== null)
        verify(first.y < second.y, "the second row is below the first")
        compare(first.x, second.x, "the rows share a left edge")
        const bar1 = findChild(first, "laneBar")
        const bar2 = findChild(second, "laneBar")
        compare(bar1.mapToItem(b, 0, 0).x, bar2.mapToItem(b, 0, 0).x, "the bars share a column")
        verify(bar1.indeterminate, "a running lane with no known total is indeterminate")
        verify(!bar2.indeterminate, "a paused lane's bar is not, however its total")
        verify(!findChild(b, "sharedBar").visible, "stacked: the shared bar is not shown")
        verify(findChild(b, "laneColumn").visible)
        mouseClick(findChild(first, "laneToggle"))
        compare(pauseSpy.count, 1)
        compare(pauseSpy.signalArguments[0][0], 1, "the Pause names its lane by its number")
        compare(pauseSpy.signalArguments[0][1], true, "the lane was running: it is to be paused")
    }

    // A lane paused and idle keeps its Resume with no bar, and the crop has no Pause: the cells keep their place, so
    // the rows still line up.
    function test_an_idle_paused_lane_and_the_crop_keep_their_cells() {
        const b = stackedBar([
            { number: -1, pausable: false, label: "", status: "Saving crops 1 of 3", paused: false, working: true,
              progress: 0.33, indeterminate: false },
            { number: 4, pausable: true, label: "F:", status: "Paused", paused: true, working: false, progress: 0,
              indeterminate: false },
        ], true)
        const crop = rowOf(b, 0)
        const idle = rowOf(b, 1)
        verify(!findChild(crop, "laneToggle").visible, "the crop has no Pause")
        verify(!findChild(idle, "laneBar").visible, "an idle lane has no bar")
        verify(findChild(idle, "laneToggle").visible, "a paused lane keeps its Resume")
        compare(findChild(crop, "laneToggle").parent.mapToItem(b, 0, 0).x,
                findChild(idle, "laneToggle").parent.mapToItem(b, 0, 0).x,
                "the Pause cells keep their place whichever row has one")
    }

    // One lane is the status line and the shared bar, as it always was: no rows.
    function test_one_lane_keeps_the_status_line_and_the_shared_bar() {
        const b = stackedBar([
            { number: 1, pausable: true, label: "E:", status: "Importing 1 of 4", paused: false, working: true,
              progress: 0.25, indeterminate: false },
        ], false)
        verify(!findChild(b, "laneColumn").visible, "one lane is not stacked")
        verify(findChild(b, "sharedBar").visible, "the shared bar shows the one lane")
    }
}
