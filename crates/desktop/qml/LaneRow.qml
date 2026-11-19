// One lane's row in the status bar, when the rows stack (StatusBar's column of them): its drive, its status, its bar while it
// works, and its own Pause. The row is the LaneModel's (src/lane_model.rs): each role is a required property here, and a
// change of one is a change of this row in place, so the bar and a press in flight are not rebuilt. Each part sits in a cell
// of its own width: a part that is hidden leaves its cell's width, so the rows line up whichever of them work.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

RowLayout {
    Words {
        id: words
    }
    id: laneRow
    required property int index
    // The lane's number (-1 for the crop row, which has no Pause of its own).
    required property int number
    required property bool pausable
    // The drive's label, empty when it can't be told and for the crop row.
    required property string label
    // The pass's line, without the label; the paused wording while the lane or the whole job is paused.
    required property string status
    // The lane's own pause (what its button shows and sets).
    required property bool paused
    // The lane is in a phase now: its bar shows.
    required property bool working
    required property real progress
    // The total is unknown.
    required property bool indeterminate
    // The whole job is paused: the bar stops.
    required property bool globalPaused
    // The Pause or Resume was pressed: the lane's own pause is to be `paused` (the opposite of what this row shows).
    signal pauseRequested(bool paused)

    spacing: 8

    Label {
        objectName: "laneLabel"
        Layout.preferredWidth: 110
        text: laneRow.label
        textFormat: Text.PlainText
        color: Theme.statusText
        elide: Text.ElideRight
    }
    Label {
        objectName: "laneStatus"
        Layout.preferredWidth: 320
        text: laneRow.status
        textFormat: Text.PlainText
        color: Theme.statusText
        elide: Text.ElideRight
    }
    Item {
        id: barCell
        Layout.preferredWidth: 140
        implicitHeight: laneBar.implicitHeight
        ProgressBar {
            id: laneBar
            objectName: "laneBar"
            anchors.verticalCenter: barCell.verticalCenter
            width: barCell.width
            visible: laneRow.working
            value: laneRow.progress
            indeterminate: laneRow.indeterminate && !laneRow.paused && !laneRow.globalPaused
        }
    }
    Item {
        id: toggleCell
        Layout.preferredWidth: 90
        implicitHeight: laneToggle.implicitHeight
        Button {
            id: laneToggle
            objectName: "laneToggle"
            anchors.verticalCenter: toggleCell.verticalCenter
            width: toggleCell.width
            visible: laneRow.pausable
            text: laneRow.paused ? words.text("ui-resume") : words.text("ui-pause")
            flat: true
            // The name carries the drive, so each row's button says which drive it pauses. Literal calls only: the ids of
            // the words are read from the QML's own calls (words.rs).
            Accessible.name: laneRow.label.length === 0
                ? (laneRow.paused ? words.text("ui-resume") : words.text("ui-pause"))
                : (laneRow.paused ? words.phrase("ui-resume-lane", 0, laneRow.label, "")
                                  : words.phrase("ui-pause-lane", 0, laneRow.label, ""))
            onClicked: laneRow.pauseRequested(!laneRow.paused)
        }
    }
}
