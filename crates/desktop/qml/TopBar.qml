// The top bar: the open folder and its file count, the subfolders and filters, the sort, the saved views and the size
// slider. It shows the state and reports what the user asks for (signals); Main makes the calls on the Rust objects.
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
    // The open folder ("" for all folders), whether subfolders are in, the sort's direction, and the saved views.
    required property string folder
    required property bool recursive
    // What the empty folder is: 0 all folders, 1 the drive roots' own files, 2 a folder (the shell's folderKind).
    required property int folderKind
    required property bool descending
    required property var savedViews
    required property bool hasSavedViews
    // The gallery: whether a list is loading, the files shown, the filter counts, and whether the filter bar is open.
    required property bool loading
    required property int fileCount
    required property int filterActive
    required property int filterTotal
    required property bool filtersOpen
    // The grid's row height, which the size slider shows.
    required property real rowHeight

    // The sort box's names (its keys are pg_client::sort::KEYS, in the same order).
    readonly property list<string> sortNames: [words.text("ui-sort-name"), words.text("ui-sort-file-size"),
        words.text("ui-sort-modified"), words.text("ui-sort-taken"), words.text("ui-sort-megapixels"),
        words.text("ui-sort-duration"), words.text("ui-sort-border-depth")]
    // The sort box's index, and its text (the save-view dialog says it).
    property alias sortIndex: sortBox.currentIndex
    readonly property string sortText: sortBox.currentText

    signal filtersToggled()
    signal recursiveToggled(bool on)
    signal sortActivated(int index)
    signal descendingToggled()
    signal viewChosen(string name)
    signal viewDeleteRequested(string name)
    signal saveViewRequested()
    signal libraryRequested()
    signal sizeChosen(real size)

    // Opens the saved views menu (the test hook --menu=views).
    function openViewsMenu() {
        viewsMenu.open()
    }

    anchors.left: parent.left
    anchors.right: parent.right

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 12
        Label {
            // The empty folder's name comes from Rust (folderKind), so it matches the tree's row.
            text: bar.folder.length ? bar.folder
                : bar.folderKind === 1 ? words.text("ui-drive-roots") : words.text("ui-all-folders")
            font.bold: true
            elide: Text.ElideMiddle
            Layout.maximumWidth: 600
        }
        Label {
            color: "#999999"
            text: bar.loading && bar.fileCount === 0 ? words.text("ui-loading")
                : bar.filterActive > 0
                  ? words.phrase("ui-files-of", 0, Qt.locale().toString(bar.fileCount), Qt.locale().toString(bar.filterTotal))
                  : words.phrase("ui-files", 0, Qt.locale().toString(bar.fileCount), "")
        }
        BusyIndicator {
            running: bar.loading
            visible: running
            Layout.preferredWidth: 20
            Layout.preferredHeight: 20
        }
        CheckBox {
            text: words.text("ui-include-subfolders")
            checked: bar.recursive
            onToggled: bar.recursiveToggled(checked)
        }
        Button {
            text: (bar.filterActive > 0 ? words.phrase("ui-filters-count", 0, ("" + (bar.filterActive)), "") : words.text("ui-filters"))
                + (bar.filtersOpen ? " ▴" : " ▾")
            highlighted: bar.filterActive > 0
            onClicked: bar.filtersToggled()
            ToolTip.visible: hovered
            ToolTip.text: words.text("ui-filters-tooltip")
        }
        ComboBox {
            id: sortBox
            model: bar.sortNames
            // Name reads A→Z; the others start with the largest, newest or longest.
            onActivated: (i) => bar.sortActivated(i)
            Layout.preferredWidth: 160
        }
        Button {
            text: bar.descending ? "↓" : "↑"
            onClicked: bar.descendingToggled()
            ToolTip.visible: hovered
            ToolTip.text: bar.descending
                ? words.text("ui-sort-descending")
                : words.text("ui-sort-ascending")
            Layout.preferredWidth: 32
        }
        // Saved views (ImageRater's bookmarks): a folder with its subfolders, filters and sort, by name.
        ViewsMenu {
            id: viewsMenu
            savedViews: bar.savedViews
            hasSavedViews: bar.hasSavedViews
            onViewChosen: (name) => bar.viewChosen(name)
            onViewDeleteRequested: (name) => bar.viewDeleteRequested(name)
            onSaveViewRequested: bar.saveViewRequested()
        }
        Item { Layout.fillWidth: true }
        Button {
            text: words.text("ui-library")
            onClicked: bar.libraryRequested()
        }
        Label { text: words.text("ui-size") }
        Slider {
            from: 60
            to: 800
            value: bar.rowHeight
            onMoved: bar.sizeChosen(value)
            Layout.preferredWidth: 220
        }
        // The row height in pixels, typed in directly; the slider and the keys set it too, and this box follows them.
        SpinBox {
            id: sizeBox
            from: 60
            to: 800
            editable: true
            onValueModified: bar.sizeChosen(value)
            Layout.preferredWidth: 110
            // Follows the row height whatever set it (a typed value would otherwise break a plain binding).
            Binding on value {
                value: Math.round(bar.rowHeight)
                restoreMode: Binding.RestoreNone
            }
        }
        Label { text: words.text("ui-px") }
    }
}
