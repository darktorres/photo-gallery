// The log view (dev mode), over a dimmed window: the records of gallery.log and this run's, oldest first, following the
// new ones (LogModel, src/log_model.rs). A row is one line (the time, the level, what it is about, the message, elided);
// a click shows the row's whole message and its detail below. Made the first time it is opened (Main's logLoader), then
// kept. Escape or Close dismisses it; Tab stays within it.
//
// The view follows the newest record (`following`) until the user scrolls away from the end, and again once they scroll
// back to it: the flag is set at the user's scroll (a wheel, the scroll bar, a drag) or at a key that moves the row, never
// from `atYEnd` after an insert, which is stale. The selection is
// the row's number (the `seq` role), which it keeps as the oldest rows are trimmed: it is found again after a trim, and
// let go when its own row was trimmed.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

ModalOverlay {
    id: panel
    Words {
        id: words
    }
    // The records (Main's LogModel).
    required property var records
    // The row clicked: the log model holds the selection (by its number), and the window gives the panel what it shows:
    // its row (-1: none), its number, and its message and detail, shown below the list.
    required property int selected
    required property real selectedSeq
    required property string selectedText
    // The view stays at the newest record as records come.
    property bool following: true

    // The user scrolled (a drag, a flick, the wheel, the scroll bar): following again only at the end.
    function scrolled() {
        following = list.atYEnd;
    }

    // A key that moves the current row (the list's own keyboard navigation) is the user scrolling, as a wheel or a scroll
    // bar is. The view moves with the row, but a keyboard move from rest ends with no movementEnded (Qt's setContentY
    // sets no moving flag), so the flag is read from the row: following only while it is the newest one.
    function keyMoved() {
        following = list.currentIndex === list.count - 1;
    }

    // The keys that move the row, read a turn later: the view has moved to the row by then.
    function keyPressed(key: int) {
        if ([Qt.Key_Up, Qt.Key_Down, Qt.Key_PageUp, Qt.Key_PageDown, Qt.Key_Home, Qt.Key_End].includes(key))
            keyMovedLater.restart();
    }

    Timer {
        id: keyMovedLater
        interval: 0
        onTriggered: panel.keyMoved()
    }

    // Select the record at `row`, or none: the log model keeps the selection as the rows move, so the window makes the call.
    signal selectRequested(int row)
    signal unselectRequested

    function select(row: int) {
        panel.selectRequested(row);
    }

    function unselect() {
        panel.unselectRequested();
    }

    Pane {
        anchors.centerIn: parent
        width: Math.min(parent.width - 80, 1200)
        height: Math.min(parent.height - 80, 760)
        padding: 16
        // A control is a focus scope too: the focus passes through it to the list.
        focus: true
        MouseArea {
            anchors.fill: parent
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label {
                text: words.text("ui-log")
                font.bold: true
                font.pixelSize: 16
            }
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
                onCountChanged: if (panel.following)
                    positionViewAtEnd()
                onMovementEnded: panel.scrolled()
                onWheelScrolled: panel.scrolled()
                // The keys that move the current row (read a turn later, see `keyPressed`).
                Keys.onPressed: event => panel.keyPressed(event.key)
                ScrollBar.vertical: ScrollBar {
                    onPressedChanged: if (!pressed)
                        panel.scrolled()
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
                    color: row.chosen ? Theme.chosenRow : (row.index % 2 ? "#242424" : "transparent")
                    Accessible.role: Accessible.ListItem
                    Accessible.name: row.message
                    Accessible.selected: row.chosen
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 6
                        anchors.rightMargin: 6
                        spacing: 10
                        Label {
                            text: row.time
                            textFormat: Text.PlainText
                            color: Theme.hintText
                            font.family: "monospace"
                        }
                        Label {
                            text: row.level === 2 ? words.text("ui-log-level-error") : (row.level === 1 ? words.text("ui-log-level-warning") : words.text("ui-log-level-info"))
                            textFormat: Text.PlainText
                            color: row.level === 2 ? Theme.invalidText : (row.level === 1 ? Theme.statusWarning : Theme.hintText)
                            font.bold: true
                        }
                        Label {
                            text: row.target
                            textFormat: Text.PlainText
                            color: Theme.statusText
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
                        onTapped: panel.select(row.index)
                    }
                }
                Label {
                    anchors.centerIn: parent
                    visible: list.count === 0
                    text: words.text("ui-log-empty")
                    color: Theme.hintText
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
                Item {
                    Layout.fillWidth: true
                }
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
