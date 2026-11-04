// The library's folders over the window: added, removed and rescanned here; the background job imports them. Made the
// first time it is opened (Main's libraryLoader), then kept. Its actions are signals: Main makes the library calls.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Dialogs
import QtQuick.Layouts

ModalOverlay {
    id: panel
    // The library's folders (library.roots), whether there are any, and whether the write-back setting is on.
    required property var roots
    required property bool hasRoots
    required property bool writeBack

    signal folderPicked(url folder)
    signal removeRequested(string path)
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
            Label { text: "Library folders"; font.bold: true; font.pixelSize: 16 }
            Label {
                text: "Images in these folders and their subfolders are imported in the background. "
                    + "Removing a folder only hides its images: adding it back restores them, ratings "
                    + "included."
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
                    width: ListView.view.width
                    Label { text: rootRow.modelData; elide: Text.ElideMiddle; Layout.fillWidth: true }
                    Button {
                        text: "Remove"
                        flat: true
                        onClicked: panel.removeRequested(rootRow.modelData)
                    }
                }
                Label {
                    anchors.centerIn: parent
                    visible: !panel.hasRoots
                    text: "No folders yet."
                    color: "#888888"
                }
            }
            CheckBox {
                text: "Write ratings and turns into the files too (where the format allows)"
                checked: panel.writeBack
                onToggled: panel.writeBackToggled(checked)
                ToolTip.visible: hovered
                ToolTip.text: "Ratings are always kept in the catalog. With this on, they also go into "
                    + "the image files (XMP), so other programs see them; turns rotate the files "
                    + "losslessly."
            }
            RowLayout {
                Button { text: "Add folder…"; onClicked: folderDialog.open() }
                Button {
                    text: "Rescan all"
                    enabled: panel.hasRoots
                    onClicked: panel.rescanRequested()
                }
                Item { Layout.fillWidth: true }
                Button { text: "Close"; onClicked: panel.dismissed() }
            }
        }
    }

    FolderDialog {
        id: folderDialog
        title: "Add a folder to the library"
        onAccepted: panel.folderPicked(selectedFolder)
    }
}
