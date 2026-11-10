// The status bar at the bottom: the selected file's info and rating (or the key hints), the write-back status, the
// background job's latest message (a click dismisses it), its status, and its progress with pause, and in dev mode the log
// view's button. The library calls are signals: Main makes them. A name, a path or an error is plain text: one holding
// markup (`<b>`) is shown as written, never as rich text.
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
    // The job's latest message, whether it is a warning, its status, and whether it is running, paused, and how far.
    required property string message
    required property bool messageWarning
    required property string status
    required property bool busy
    required property bool paused
    required property real progress
    // Dev mode is on: the log view's button shows.
    required property bool devMode

    signal messageDismissed()
    signal pauseToggled()
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
            color: "#aaaaaa"
            text: bar.selection
            textFormat: Text.PlainText
        }
        // Ratings and turns on their way into the files, or why they aren't.
        Label {
            visible: bar.writeStatus.length > 0
            text: bar.writeStatus
            textFormat: Text.PlainText
            color: bar.writeFailed ? "#e0a050" : "#aaaaaa"
            elide: Text.ElideMiddle
            Layout.maximumWidth: 500
        }
        // The latest error or note from the background job; click to dismiss.
        Label {
            visible: bar.message.length > 0
            text: (bar.messageWarning ? "⚠ " : "") + bar.message
            textFormat: Text.PlainText
            color: bar.messageWarning ? "#e0a050" : "#aaaaaa"
            elide: Text.ElideMiddle
            Layout.maximumWidth: 500
            TapHandler { onTapped: bar.messageDismissed() }
        }
        Label {
            visible: bar.status.length > 0
            text: bar.status
            textFormat: Text.PlainText
            color: "#aaaaaa"
        }
        ProgressBar {
            visible: bar.busy
            indeterminate: bar.progress < 0
            value: Math.max(bar.progress, 0)
            Layout.preferredWidth: 140
        }
        Button {
            visible: bar.busy || bar.paused
            text: bar.paused ? words.text("ui-resume") : words.text("ui-pause")
            flat: true
            onClicked: bar.pauseToggled()
        }
        Button {
            visible: bar.devMode
            text: words.text("ui-log")
            flat: true
            onClicked: bar.logRequested()
        }
    }
}
