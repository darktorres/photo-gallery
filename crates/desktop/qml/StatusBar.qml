// The status bar at the bottom: the selected file's info and rating (or the key hints), the write-back status, the
// inbox's toast (a click opens the inbox; an info goes by itself), the background job's status, its "N failed" (a link
// that opens the inbox at the pass's failed files, only while the inbox lists something of that pass: else plain text,
// with a tooltip saying it isn't recorded yet) and its progress with pause, the inbox's button with its unread count,
// and in dev mode the log view's button. The library calls are signals: Main makes them. A name, a path or an error is
// plain text: one holding markup (`<b>`) is shown as written, never as rich text.
//
// The job's rows (LaneModel): while one lane works, the status line and the shared bar show it, as they always have. When
// the rows stack (`stacked`: two or more rows, or a lane paused by itself), each row is a LaneRow with its own bar and
// its own Pause, one under the other, and the status line and the shared bar are not shown.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

ToolBar {
    Words {
        id: words
    }
    id: bar
    // The selection's text (Main's selectionInfo).
    required property string selection
    // The library's write-back status, and whether it failed.
    required property string writeStatus
    required property bool writeFailed
    // The inbox's toast (a notice's title; empty: none) and its level (0 info, 1 warning, 2 error), and how many of its
    // entries are unread.
    required property string toast
    required property int toastLevel
    required property int unread
    // The job's status line (the library's one progress), and whether it is running, paused, and how far.
    required property string status
    // The rows of the lanes (LaneModel, src/lane_model.rs), and whether they stack.
    property var laneModel: null
    required property bool stacked
    // The pass's total isn't known: the shared bar is indeterminate while it runs.
    required property bool indeterminate
    // The pass's failures ("N failed"; empty: none), and whether the inbox lists something of the pass (Notices'
    // `passes` has its bit): only then is it a link.
    required property string failed
    required property bool failedLinked
    required property bool busy
    required property bool paused
    required property real progress
    // Dev mode is on: the log view's button shows.
    required property bool devMode
    // An info toast stays up instead of going by itself (the screenshots: no timer runs while the window settles).
    property bool toastHeld: false

    // The toast went by itself (an info, after a while).
    signal toastExpired()
    signal inboxRequested()
    signal failedRequested()
    signal pauseToggled()
    // A row's Pause was pressed: the lane numbered `number` is to be paused (`paused`) or resumed.
    signal lanePauseRequested(int number, bool paused)
    signal logRequested()

    anchors.left: parent.left
    anchors.right: parent.right
    anchors.bottom: parent.bottom

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 12
        Label {
            Layout.fillWidth: true
            elide: Text.ElideMiddle
            color: Theme.statusText
            text: bar.selection
            textFormat: Text.PlainText
        }
        // Ratings and turns on their way into the files, or why they aren't.
        Label {
            visible: bar.writeStatus.length > 0
            text: bar.writeStatus
            textFormat: Text.PlainText
            color: bar.writeFailed ? Theme.statusWarning : Theme.statusText
            elide: Text.ElideMiddle
            Layout.maximumWidth: 500
        }
        // The newest notice, as a toast: a warning or an error stays until it is dismissed or replaced; a click opens
        // the inbox.
        Label {
            visible: bar.toast.length > 0
            text: (bar.toastLevel > 0 ? "⚠ " : "") + bar.toast
            textFormat: Text.PlainText
            color: bar.toastLevel > 0 ? Theme.statusWarning : Theme.statusText
            elide: Text.ElideMiddle
            Layout.maximumWidth: 500
            TapHandler { onTapped: bar.inboxRequested() }
        }
        Timer {
            objectName: "toastTimer"
            interval: 6000
            running: bar.toast.length > 0 && bar.toastLevel === 0 && !bar.toastHeld
            onTriggered: bar.toastExpired()
        }
        Label {
            visible: bar.status.length > 0 && !bar.stacked
            text: bar.status
            textFormat: Text.PlainText
            color: Theme.statusText
        }
        // Stacked: a row for each lane that works or is paused (and the crop), each with its own bar and its own Pause.
        ColumnLayout {
            objectName: "laneColumn"
            visible: bar.stacked
            spacing: 2
            Repeater {
                id: laneRepeater
                objectName: "laneRepeater"
                model: bar.laneModel
                delegate: LaneRow {
                    id: lane
                    globalPaused: bar.paused
                    onPauseRequested: (paused) => bar.lanePauseRequested(lane.number, paused)
                }
            }
        }
        // A link to the pass's failed files in the inbox while it lists them; until then the count alone.
        Label {
            objectName: "statusFailed"
            visible: bar.failed.length > 0
            text: bar.failed
            textFormat: Text.PlainText
            color: Theme.statusWarning
            font.underline: bar.failedLinked
            Accessible.role: bar.failedLinked ? Accessible.Link : Accessible.StaticText
            Accessible.name: bar.failedLinked ? words.text("ui-failed-link") : bar.failed
            Accessible.description: bar.failedLinked ? "" : words.text("ui-failed-not-recorded")
            ToolTip.visible: failedHover.hovered && !bar.failedLinked
            ToolTip.text: words.text("ui-failed-not-recorded")
            HoverHandler {
                id: failedHover
                cursorShape: bar.failedLinked ? Qt.PointingHandCursor : Qt.ArrowCursor
            }
            TapHandler {
                objectName: "statusFailedTap"
                enabled: bar.failedLinked
                onTapped: bar.failedRequested()
            }
        }
        // One line's bar: the shared one while the rows don't stack (a row has its own bar when they do).
        ProgressBar {
            objectName: "sharedBar"
            visible: bar.busy && !bar.stacked
            indeterminate: bar.indeterminate && !bar.paused
            value: bar.progress
            Layout.preferredWidth: 140
        }
        Button {
            visible: bar.busy || bar.paused
            text: bar.paused ? words.text("ui-resume") : words.text("ui-pause")
            flat: true
            onClicked: bar.pauseToggled()
        }
        Button {
            text: words.phrase("ui-inbox", bar.unread, "" + bar.unread, "")
            Accessible.name: text
            flat: true
            highlighted: bar.unread > 0
            onClicked: bar.inboxRequested()
        }
        Button {
            visible: bar.devMode
            text: words.text("ui-log")
            flat: true
            onClicked: bar.logRequested()
        }
    }
}
