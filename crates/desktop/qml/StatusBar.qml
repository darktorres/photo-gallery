// The status bar at the bottom: the selected file's info and rating (or the key hints), the write-back status, the
// inbox's toast (a click opens the inbox; an info goes by itself), the background job's status, its "N failed" (a link
// that opens the inbox at the pass's failed files, only while the inbox lists something of that pass: else plain text,
// with a tooltip saying it isn't recorded yet) and its progress with pause, the inbox's button with its unread count,
// and in dev mode the log view's button. The library calls are signals: Main makes them. A name, a path or an error is
// plain text: one holding markup (`<b>`) is shown as written, never as rich text.
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
            objectName: "toastTimer"
            interval: 6000
            running: bar.toast.length > 0 && bar.toastLevel === 0 && !bar.toastHeld
            onTriggered: bar.toastExpired()
        }
        Label {
            visible: bar.status.length > 0
            text: bar.status
            textFormat: Text.PlainText
            color: "#aaaaaa"
        }
        // A link to the pass's failed files in the inbox while it lists them; until then the count alone.
        Label {
            objectName: "statusFailed"
            visible: bar.failed.length > 0
            text: bar.failed
            textFormat: Text.PlainText
            color: "#e0a050"
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
