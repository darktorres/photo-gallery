// The inbox, over a dimmed window: what the app told the user, newest first (Notices, src/notices.rs). A row is the
// notice's title, where it comes from, when it was last posted, and how many times; a click shows its details below,
// with Copy and Dismiss, which the panel asks of Main (its signals), as Clear all. Opening it marks every entry read. Made
// the first time it is opened (Main's inboxLoader), then kept. Escape or Close dismisses it; Tab stays within it.
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
    // The entries (Main's Notices).
    required property var notices
    // The row clicked: its details are shown below the list (-1: none).
    property int selected: -1
    property string selectedText: ""

    // The entry at `row` is to be copied, or dismissed; every entry is to be dismissed.
    signal copyRequested(int row)
    signal dismissRequested(int row)
    signal clearRequested()

    // A new listing moves the rows: the details shown are of a row that may be another now.
    Connections {
        target: panel.notices
        function onModelReset() { panel.selected = -1 }
    }

    Pane {
        anchors.centerIn: parent
        width: Math.min(parent.width - 80, 1000)
        height: Math.min(parent.height - 80, 700)
        padding: 16
        // A control is a focus scope too: the focus passes through it to the list.
        focus: true
        MouseArea { anchors.fill: parent }
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label { text: words.text("ui-inbox-title"); font.bold: true; font.pixelSize: 16 }
            WheelListView {
                id: list
                objectName: "inboxList"
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: panel.notices
                // The keys go to the list when the panel opens (ModalOverlay is a focus scope).
                focus: true
                activeFocusOnTab: true
                KeyNavigation.backtab: closeButton
                ScrollBar.vertical: ScrollBar {}
                delegate: Rectangle {
                    id: row
                    required property int index
                    required property string title
                    required property string body
                    required property string source
                    required property int level
                    required property real updated
                    required property int count
                    required property bool read
                    width: list.width
                    height: 30
                    color: panel.selected === row.index ? "#2f5f93" : (row.index % 2 ? "#242424" : "transparent")
                    Accessible.role: Accessible.ListItem
                    Accessible.name: row.title
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 10
                        Rectangle {
                            Layout.preferredWidth: 8
                            Layout.preferredHeight: 8
                            radius: 4
                            color: row.level === 2 ? "#e06060" : (row.level === 1 ? "#e0a050" : "#6a9fd8")
                        }
                        Label {
                            text: row.title
                            textFormat: Text.PlainText
                            font.bold: !row.read
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                        Label {
                            visible: row.count > 1
                            text: "×" + row.count
                            textFormat: Text.PlainText
                            color: "#aaaaaa"
                        }
                        Label {
                            text: row.source
                            textFormat: Text.PlainText
                            color: "#aaaaaa"
                            Layout.preferredWidth: 120
                            elide: Text.ElideRight
                        }
                        Label {
                            text: Qt.formatDateTime(new Date(row.updated * 1000), "yyyy-MM-dd hh:mm")
                            textFormat: Text.PlainText
                            color: "#888888"
                        }
                    }
                    TapHandler {
                        onTapped: {
                            panel.selected = row.index
                            panel.selectedText = row.body
                        }
                    }
                }
                Label {
                    anchors.centerIn: parent
                    visible: list.count === 0
                    text: words.text("ui-inbox-empty")
                    color: "#888888"
                }
            }
            // The row clicked: its details, selectable.
            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: 110
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
                Button {
                    objectName: "inboxCopy"
                    text: words.text("ui-inbox-copy")
                    enabled: panel.selected >= 0
                    onClicked: panel.copyRequested(panel.selected)
                }
                Button {
                    objectName: "inboxDismiss"
                    text: words.text("ui-inbox-dismiss")
                    enabled: panel.selected >= 0
                    onClicked: {
                        const row = panel.selected
                        panel.selected = -1
                        panel.dismissRequested(row)
                    }
                }
                Item { Layout.fillWidth: true }
                Button {
                    objectName: "inboxClear"
                    text: words.text("ui-inbox-clear")
                    enabled: list.count > 0
                    onClicked: {
                        panel.selected = -1
                        panel.clearRequested()
                    }
                }
                Button {
                    id: closeButton
                    objectName: "inboxClose"
                    text: words.text("ui-close")
                    activeFocusOnTab: true
                    KeyNavigation.tab: list
                    onClicked: panel.dismissed()
                }
            }
        }
    }
}
