// The log view (dev mode), over a dimmed window: the records of gallery.log and this run's, oldest first, following the
// new ones (LogModel, src/log_model.rs). A row is one line (the time, the level, what it is about, the message, elided);
// a click shows the row's whole message and its detail below. Made the first time it is opened (Main's logLoader), then
// kept. Escape or Close dismisses it; Tab stays within it.
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
    // The row clicked: its message and detail are shown below the list (-1: none).
    property int selected: -1
    property string selectedText: ""


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
                // The newest record is at the bottom, and the view stays there as records come while it is.
                onCountChanged: if (atYEnd || count < 2) positionViewAtEnd()
                ScrollBar.vertical: ScrollBar {}
                delegate: Rectangle {
                    id: row
                    required property int index
                    required property string time
                    required property int level
                    required property string target
                    required property string message
                    required property string detail
                    width: list.width
                    height: 22
                    color: panel.selected === row.index ? "#2f5f93" : (row.index % 2 ? "#242424" : "transparent")
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
                        onTapped: {
                            panel.selected = row.index
                            panel.selectedText = row.detail.length > 0 ? row.message + "\n\n" + row.detail : row.message
                        }
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
