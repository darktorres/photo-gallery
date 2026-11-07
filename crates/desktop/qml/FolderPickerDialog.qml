// The app's own folder dialog, for the systems where the folder picker is not the system's (`FolderPicker.native` is false).
// It shows the folders of the folder it is in, by name, and reports each choice; the listing and the choices are the
// picker's (Rust), so the path never crosses to QML. The owner sets `names` and `here` from its picker, opens the dialog
// when the picker asks (`openInApp`), and passes each choice on to the picker (its `done` then ends the choice).
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

ModalOverlay {
    id: overlay
    Words {
        id: words
    }
    // The folders listed (display names, in order) and the folder they are in (display text).
    property var names: null
    property string here: ""

    // The choices: the owner passes each on to its picker.
    signal goUp()
    signal openRow(int row)
    signal chooseRow(int row)
    signal chooseHere()
    // Closed without a choice (Cancel, Escape, or a click on the backdrop).
    signal cancelled()

    onDismissed: {
        overlay.open = false
        overlay.cancelled()
    }

    Pane {
        anchors.centerIn: parent
        width: 560
        height: 460
        padding: 16
        // Clicks inside the dialog don't reach the dimmed background.
        MouseArea { anchors.fill: parent }

        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label { text: words.text("ui-choose-folder-title"); font.bold: true; font.pixelSize: 16 }
            RowLayout {
                Layout.fillWidth: true
                Button {
                    text: words.text("ui-folder-up")
                    onClicked: overlay.goUp()
                }
                Label {
                    text: overlay.here
                    elide: Text.ElideMiddle
                    Layout.fillWidth: true
                }
            }
            WheelListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: overlay.names
                ScrollBar.vertical: ScrollBar {}
                delegate: RowLayout {
                    id: folderRow
                    required property string modelData
                    required property int index
                    width: ListView.view.width
                    // A click opens the folder; Choose takes it.
                    ItemDelegate {
                        Layout.fillWidth: true
                        text: folderRow.modelData
                        onClicked: overlay.openRow(folderRow.index)
                    }
                    Button {
                        text: words.text("ui-choose")
                        flat: true
                        onClicked: {
                            overlay.open = false
                            overlay.chooseRow(folderRow.index)
                        }
                    }
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button {
                    text: words.text("ui-choose-here")
                    onClicked: {
                        overlay.open = false
                        overlay.chooseHere()
                    }
                }
                Button {
                    text: words.text("ui-cancel")
                    onClicked: overlay.dismissed()
                }
            }
        }
    }
}
