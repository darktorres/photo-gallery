// The status bar at the bottom: the selected file's info and rating (or the key hints), the write-back status, the
// inbox's toast (a click opens the inbox; an info goes by itself), the inbox's button with its unread count, and in dev
// mode the log view's button. The background job is on its lanes' rows (LaneModel), above this line: each working or
// paused lane, and the crop, has a row of its own with its status, its failed count, its bar and its own Pause. The
// library calls are signals: Main makes them. A name, a path or an error is plain text: one holding markup (`<b>`) is
// shown as written, never as rich text.
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
    // The rows of the lanes (LaneModel, src/lane_model.rs).
    property var laneModel: null
    // The ledgers the inbox lists (Notices' `passes`): a row's failed count is a link only when its ledger has its bit.
    required property int inboxPasses
    // Dev mode is on: the log view's button shows.
    required property bool devMode
    // An info toast stays up instead of going by itself (the screenshots: no timer runs while the window settles).
    property bool toastHeld: false

    // The toast went by itself (an info, after a while).
    signal toastExpired()
    signal inboxRequested()
    // A row's Pause was pressed: the lane numbered `number` is to be paused (`paused`) or resumed.
    signal lanePauseRequested(int number, bool paused)
    // A row's failed count was pressed: the ledger `pass` (a LedgerKind code) opens in the inbox.
    signal laneFailedRequested(int pass)
    signal logRequested()

    anchors.left: parent.left
    anchors.right: parent.right
    anchors.bottom: parent.bottom

    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        // A line for each lane that works or is paused (and the crop), above the status bar's line.
        ColumnLayout {
            objectName: "laneColumn"
            visible: laneRepeater.count > 0
            Layout.alignment: Qt.AlignRight
            Layout.leftMargin: 8
            Layout.rightMargin: 8
            spacing: 2
            Repeater {
                id: laneRepeater
                objectName: "laneRepeater"
                model: bar.laneModel
                delegate: LaneRow {
                    id: lane
                    inboxPasses: bar.inboxPasses
                    onPauseRequested: (paused) => bar.lanePauseRequested(lane.number, paused)
                    onFailedRequested: (pass) => bar.laneFailedRequested(pass)
                }
            }
        }
        RowLayout {
            objectName: "statusRow"
            Layout.fillWidth: true
            Layout.leftMargin: 8
            Layout.rightMargin: 8
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
}
