// The status bar (StatusBar.qml): its line of the selection, the write-back status, the toast and the inbox's button. The
// job is on the lane rows (LaneRow, tst_lane_row.qml), above it: each row has its own bar, its own Pause and its own
// failed count, and the rows carry the job's text, so the status bar has no line of its own for it. An info toast goes by
// itself unless the screenshots hold it. Run by `just qmltest`.
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
            inboxPasses: 0
            devMode: false
        }
    }

    // The lanes' rows, as the LaneModel's roles: a list of them is set per test.
    Component {
        id: laneListC
        ListModel {}
    }

    SignalSpy { id: pauseSpy; signalName: "lanePauseRequested" }

    // A bar with the given rows, and the rows' pause clicks counted.
    function laneBar(rows: var): var {
        const model = createTemporaryObject(laneListC, tc)
        verify(model !== null)
        for (const row of rows)
            model.append(row)
        const b = createTemporaryObject(barC, tc, { laneModel: model })
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

    // An info toast's timer runs, unless the toast is held (the screenshots); a warning's never runs.
    function test_an_info_toast_goes_by_itself_unless_held() {
        const b = createTemporaryObject(barC, tc)
        verify(b !== null)
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

    // Each lane has a row, with its own bar and its own Pause, one under another, and the rows are aligned to the right
    // edge of the status row beneath them. There is no shared bar and no line of the job in the status bar: the rows carry
    // it. The Pause names its lane by its number.
    function test_each_lane_has_its_own_row_bar_and_pause_one_under_another() {
        const b = laneBar([
            { number: 1, pausable: true, label: "E:", status: "Importing 1 of 0", failed: "", failedPass: -1,
              paused: false, working: true, progress: 0, indeterminate: true },
            { number: 2, pausable: true, label: "D:", status: "Paused · Importing 2 of 0", failed: "", failedPass: -1,
              paused: true, working: true, progress: 0, indeterminate: true },
        ])
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
        verify(findChild(b, "sharedBar") === null, "no shared bar: each row has its own")
        verify(findChild(b, "statusLine") === null, "the job's line is on the rows, not in the status bar")
        verify(findChild(b, "laneColumn").visible)
        const statusRow = findChild(b, "statusRow")
        verify(first.mapToItem(b, 0, 0).y < statusRow.mapToItem(b, 0, 0).y, "the lanes are on lines above the status row")
        const column = findChild(b, "laneColumn")
        compare(column.mapToItem(b, column.width, 0).x, statusRow.mapToItem(b, statusRow.width, 0).x,
                "the lanes are aligned to the right, with the status row's right edge")
        mouseClick(findChild(first, "laneToggle"))
        compare(pauseSpy.count, 1)
        compare(pauseSpy.signalArguments[0][0], 1, "the Pause names its lane by its number")
        compare(pauseSpy.signalArguments[0][1], true, "the lane was running: it is to be paused")
    }

    // A lane paused and idle keeps its Resume with no bar, and the crop has no Pause: the cells keep their place, so
    // the rows still line up.
    function test_an_idle_paused_lane_and_the_crop_keep_their_cells() {
        const b = laneBar([
            { number: -1, pausable: false, label: "", status: "Saving crops 1 of 3", failed: "", failedPass: -1,
              paused: false, working: true, progress: 0.33, indeterminate: false },
            { number: 4, pausable: true, label: "F:", status: "Paused", failed: "", failedPass: -1,
              paused: true, working: false, progress: 0, indeterminate: false },
        ])
        const crop = rowOf(b, 0)
        const idle = rowOf(b, 1)
        verify(!findChild(crop, "laneToggle").visible, "the crop has no Pause")
        verify(!findChild(idle, "laneBar").visible, "an idle lane has no bar")
        verify(findChild(idle, "laneToggle").visible, "a paused lane keeps its Resume")
        compare(findChild(crop, "laneToggle").parent.mapToItem(b, 0, 0).x,
                findChild(idle, "laneToggle").parent.mapToItem(b, 0, 0).x,
                "the Pause cells keep their place whichever row has one")
    }

    // One working lane has its own row too, as any number of lanes do. With no rows there is no column.
    function test_one_working_lane_has_its_own_row() {
        const b = laneBar([
            { number: 1, pausable: true, label: "E:", status: "Importing 1 of 4", failed: "", failedPass: -1,
              paused: false, working: true, progress: 0.25, indeterminate: false },
        ])
        verify(findChild(b, "laneColumn").visible, "one lane has its row")
        const empty = laneBar([])
        verify(!findChild(empty, "laneColumn").visible, "no rows, no column")
    }
}
