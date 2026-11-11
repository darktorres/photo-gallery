// The log view (dev mode), over a dimmed window: the records of gallery.log and this run's, oldest first, following the
// new ones (LogModel, src/log_model.rs). A row is one line (the time, the level, what it is about, the message, elided);
// a click shows the row's whole message and its detail below. Made the first time it is opened (Main's logLoader), then
// kept. Escape or Close dismisses it; Tab stays within it.
//
// The view follows the newest record (`following`) until the user scrolls away from the end, and again once they scroll
// back to it: the flag is set at the user's scroll only, since `atYEnd` read after an insert is stale. The selection is
// the row's number (the `seq` role), which it keeps as the oldest rows are trimmed: it is found again after a trim, and
// let go when its own row was trimmed.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

ModalOverlay {
    Words {
        id: words
    }
    id: panel
    // The records (Main's LogModel).
    required property var records
    // The row clicked: its message and detail are shown below the list (-1: none), and its number (`seq`).
    property int selected: -1
    property real selectedSeq: -1
    property string selectedText: ""
    // The view stays at the newest record as records come.
    property bool following: true

    // The user scrolled (a drag, a flick, the wheel, the scroll bar): following again only at the end.
    function scrolled() {
        following = list.atYEnd
    }

    function select(row: int, seq: real, text: string) {
        selected = row
        selectedSeq = seq
        selectedText = text
    }

    function unselect() {
        selected = -1
        selectedSeq = -1
        selectedText = ""
    }

    // The rows move under the selection: records appended after it, the oldest trimmed before it (or with it).
    Connections {
        target: panel.records
        function onRowsInserted(parent: var, first: int, last: int) {
            if (panel.selected >= first)
                panel.selected += last - first + 1
        }
        function onRowsRemoved(parent: var, first: int, last: int) {
            if (panel.selected < 0)
                return
            if (panel.selected > last)
                panel.selected -= last - first + 1
            else if (panel.selected >= first)
                panel.unselect()
        }
        function onModelReset() {
            panel.unselect()
        }
    }

    Pane {
        anchors.centerIn: parent
        width: Math.min(parent.width - 80, 1200)
        height: Math.min(parent.height - 80, 760)
        padding: 16
        // A control is a focus scope too: the focus passes through it to the list.
        focus: true
        MouseArea { anchors.fill: parent }
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label { text: words.text("ui-log-title"); font.bold: true; font.pixelSize: 16 }
            WheelListView {
                id: list
                objectName: "logList"
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: panel.records
                // The keys go to the list when the panel opens (ModalOverlay is a focus scope).
                focus: true
                activeFocusOnTab: true
                KeyNavigation.backtab: closeButton
                // The newest record is at the bottom, and the view stays there as records come while it follows them.
                onCountChanged: if (panel.following) positionViewAtEnd()
                onMovementEnded: panel.scrolled()
                onWheelScrolled: panel.scrolled()
                ScrollBar.vertical: ScrollBar {
                    onPressedChanged: if (!pressed) panel.scrolled()
                }
                delegate: Rectangle {
                    id: row
                    required property int index
                    required property string time
                    required property int level
                    required property string target
                    required property string message
                    required property string detail
                    required property real seq
                    readonly property bool chosen: row.seq === panel.selectedSeq
                    width: list.width
                    height: 22
                    color: row.chosen ? "#2f5f93" : (row.index % 2 ? "#242424" : "transparent")
                    Accessible.role: Accessible.ListItem
                    Accessible.name: row.message
                    Accessible.selected: row.chosen
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 6
                        anchors.rightMargin: 6
                        spacing: 10
                        Label { text: row.time; textFormat: Text.PlainText; color: "#888888"; font.family: "monospace" }
                        Label {
                            text: row.level === 2 ? "E" : (row.level === 1 ? "W" : "I")
                            textFormat: Text.PlainText
                            color: row.level === 2 ? "#e06060" : (row.level === 1 ? "#e0a050" : "#888888")
                            font.bold: true
                        }
                        Label {
                            text: row.target
                            textFormat: Text.PlainText
                            color: "#aaaaaa"
                            elide: Text.ElideRight
                            Layout.preferredWidth: 90
                        }
                        Label {
                            text: row.message
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }
                    TapHandler {
                        onTapped: panel.select(row.index, row.seq,
                                               row.detail.length > 0 ? row.message + "\n\n" + row.detail : row.message)
                    }
                }
                Label {
                    anchors.centerIn: parent
                    visible: list.count === 0
                    text: words.text("ui-log-empty")
                    color: "#888888"
                }
            }
            // The row clicked, whole: its message and its detail (a backtrace), selectable to copy.
            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: 160
                visible: panel.selected >= 0
                TextArea {
                    text: panel.selectedText
                    textFormat: TextEdit.PlainText
                    readOnly: true
                    wrapMode: TextEdit.Wrap
                    activeFocusOnTab: false
                }
            }
            RowLayout {
                Item { Layout.fillWidth: true }
                Button {
                    id: closeButton
                    objectName: "logClose"
                    text: words.text("ui-close")
                    activeFocusOnTab: true
                    KeyNavigation.tab: list
                    onClicked: panel.dismissed()
                }
            }
        }
    }
}
