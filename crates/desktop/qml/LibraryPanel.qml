// The library's folders over the window: added, removed and rescanned here; the background job imports them. Made the
// first time it is opened (Main's libraryLoader), then kept. Its actions are signals: Main makes the library calls.
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
    // The library's folders (library.roots), whether there are any, and whether the write-back setting is on.
    required property var roots
    required property bool hasRoots
    required property bool writeBack

    // Add a folder (Main asks the picker, then the library takes what it chose).
    signal addRequested()
    signal removeRequested(int row)
    signal rescanRequested()
    signal writeBackToggled(bool on)

    Pane {
        anchors.centerIn: parent
        width: 680
        height: 440
        padding: 16
        MouseArea { anchors.fill: parent }
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label { text: words.text("ui-library-folders"); font.bold: true; font.pixelSize: 16 }
            Label {
                text: words.text("ui-library-intro")
                color: "#999999"
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            WheelListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: panel.roots
                ScrollBar.vertical: ScrollBar {}
                delegate: RowLayout {
                    id: rootRow
                    required property string modelData
                    required property int index
                    width: ListView.view.width
                    Label { text: rootRow.modelData; elide: Text.ElideMiddle; Layout.fillWidth: true }
                    Button {
                        text: words.text("ui-remove")
                        flat: true
                        onClicked: panel.removeRequested(rootRow.index)
                    }
                }
                Label {
                    anchors.centerIn: parent
                    visible: !panel.hasRoots
                    text: words.text("ui-no-folders-yet")
                    color: "#888888"
                }
            }
            CheckBox {
                text: words.text("ui-write-files")
                checked: panel.writeBack
                onToggled: panel.writeBackToggled(checked)
                ToolTip.visible: hovered
                ToolTip.text: words.text("ui-write-files-tooltip")
            }
            RowLayout {
                Button { text: words.text("ui-add-folder"); onClicked: panel.addRequested() }
                Button {
                    text: words.text("ui-rescan-all")
                    enabled: panel.hasRoots
                    onClicked: panel.rescanRequested()
                }
                Item { Layout.fillWidth: true }
                Button { text: words.text("ui-close"); onClicked: panel.dismissed() }
            }
        }
    }
}
