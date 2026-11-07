// The merged folder tree on the left of the window: a folder on several drives is one node. A click shows the folder (Main
// opens it), the arrow or a double click expands or collapses it (Main calls the FolderModel).
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

WheelListView {
    id: tree
    // The folders: the tree's rows.
    required property FolderModel folders

    // Row `row` was clicked: it is the current row here; Main opens its folder.
    signal folderOpened(int row)
    // Row `row` was asked to expand or collapse.
    signal toggleRequested(int row)

    clip: true
    model: folders
    currentIndex: 0
    ScrollBar.vertical: ScrollBar {}
    delegate: ItemDelegate {
        id: node
        required property int index
        required property string name
        required property int depth
        required property bool expandable
        required property bool expanded
        required property double total
        required property string drives
        required property bool merged
        width: ListView.view.width
        height: 26
        rightPadding: 18  // clear of the scrollbar
        highlighted: ListView.isCurrentItem
        contentItem: RowLayout {
            spacing: 4
            Item { Layout.preferredWidth: node.depth * 14 }
            Label {
                text: node.expandable ? (node.expanded ? "▾" : "▸") : ""
                Layout.preferredWidth: 12
                TapHandler { onTapped: tree.toggleRequested(node.index) }
            }
            Label { text: node.name; elide: Text.ElideRight; Layout.fillWidth: true }
            // Folders present on more than one drive are merged into this one node.
            Label { text: node.drives; color: "#6fbf8f"; visible: node.merged; font.pixelSize: 11 }
            Label { text: Qt.locale().toString(node.total, "f", 0); color: "#888888"; font.pixelSize: 11 }
        }
        onClicked: {
            tree.currentIndex = node.index
            tree.folderOpened(node.index)
        }
        onDoubleClicked: tree.toggleRequested(node.index)
    }
}
