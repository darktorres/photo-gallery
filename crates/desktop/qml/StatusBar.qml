// The status bar at the bottom: the selected file's info and rating (or the key hints), the write-back status, the
// inbox's toast (a click opens the inbox; an info goes by itself), the background job's status and its progress with
// pause, the inbox's button with its unread count, and in dev mode the log view's button. The library calls are signals: Main makes them. A name, a path or an error is plain text: one holding
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
    // The inbox's toast (a notice's title; empty: none) and its level (0 info, 1 warning, 2 error), and how many of its
    // entries are unread.
    required property string toast
    required property int toastLevel
    required property int unread
    // The job's status, and whether it is running, paused, and how far.
    required property string status
    required property bool busy
    required property bool paused
    required property real progress
    // Dev mode is on: the log view's button shows.
    required property bool devMode

    // The toast went by itself (an info, after a while).
    signal toastExpired()
    signal inboxRequested()
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
        // The newest notice, as a toast: a warning or an error stays until it is dismissed or replaced; a click opens
        // the inbox.
        Label {
            visible: bar.toast.length > 0
            text: (bar.toastLevel > 0 ? "⚠ " : "") + bar.toast
            textFormat: Text.PlainText
            color: bar.toastLevel > 0 ? "#e0a050" : "#aaaaaa"
            elide: Text.ElideMiddle
            Layout.maximumWidth: 500
            TapHandler { onTapped: bar.inboxRequested() }
        }
        Timer {
            interval: 6000
            running: bar.toast.length > 0 && bar.toastLevel === 0
            onTriggered: bar.toastExpired()
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
