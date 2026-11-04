// The top bar: the open folder and its file count, the subfolders and filters, the sort, the saved views and the size
// slider. It shows the state and reports what the user asks for (signals); Main makes the calls on the Rust objects.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts

ToolBar {
    id: bar
    // The open folder ("" for all folders), whether subfolders are in, the sort's direction, and the saved views.
    required property string folder
    required property bool recursive
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

    // The sort box's names (the keys are Main's sortKeys, in the same order).
    readonly property list<string> sortNames: ["By name", "By file size", "By date modified", "By date taken",
        "By megapixels", "By duration", "By border depth"]
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
        viewsButton.clicked()
    }

    anchors.left: parent.left
    anchors.right: parent.right

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 12
        Label {
            text: bar.folder.length ? bar.folder : "All folders"
            font.bold: true
            elide: Text.ElideMiddle
            Layout.maximumWidth: 600
        }
        Label {
            color: "#999999"
            text: bar.loading && bar.fileCount === 0 ? "Loading…"
                : Qt.locale().toString(bar.fileCount)
                  + (bar.filterActive > 0 ? " of " + Qt.locale().toString(bar.filterTotal) : "")
                  + " files"
        }
        BusyIndicator {
            running: bar.loading
            visible: running
            Layout.preferredWidth: 20
            Layout.preferredHeight: 20
        }
        CheckBox {
            text: "Include subfolders"
            checked: bar.recursive
            onToggled: bar.recursiveToggled(checked)
        }
        Button {
            text: "Filters" + (bar.filterActive > 0 ? " (" + bar.filterActive + ")" : "")
                + (bar.filtersOpen ? " ▴" : " ▾")
            highlighted: bar.filterActive > 0
            onClicked: bar.filtersToggled()
            ToolTip.visible: hovered
            ToolTip.text: "Filter by type, extension, rating, size, date, duplicates… (F)"
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
                ? "Descending (click for ascending)"
                : "Ascending (click for descending)"
            Layout.preferredWidth: 32
        }
        // Saved views (ImageRater's bookmarks): a folder with its subfolders, filters and sort, by name.
        Button {
            id: viewsButton
            text: "Views ▾"
            onClicked: viewsMenu.popup(viewsButton, 0, viewsButton.height)
            Menu {
                id: viewsMenu
                Instantiator {
                    model: bar.savedViews
                    delegate: MenuItem {
                        required property string modelData
                        text: modelData
                        onTriggered: bar.viewChosen(modelData)
                    }
                    onObjectAdded: (i, o) => viewsMenu.insertItem(i, o as MenuItem)
                    onObjectRemoved: (i, o) => viewsMenu.removeItem(o as MenuItem)
                }
                MenuSeparator {}
                MenuItem {
                    text: "Save current view…"
                    onTriggered: bar.saveViewRequested()
                }
                Menu {
                    id: deleteMenu
                    title: "Delete view"
                    enabled: bar.hasSavedViews
                    Instantiator {
                        model: bar.savedViews
                        delegate: MenuItem {
                            required property string modelData
                            text: modelData
                            onTriggered: bar.viewDeleteRequested(modelData)
                        }
                        onObjectAdded: (i, o) => deleteMenu.insertItem(i, o as MenuItem)
                        onObjectRemoved: (i, o) => deleteMenu.removeItem(o as MenuItem)
                    }
                }
            }
        }
        Item { Layout.fillWidth: true }
        Button {
            text: "Library…"
            onClicked: bar.libraryRequested()
        }
        Label { text: "Size" }
        Slider {
            from: 60
            to: 800
            value: bar.rowHeight
            onMoved: bar.sizeChosen(value)
            Layout.preferredWidth: 220
        }
        Label { text: Math.round(bar.rowHeight) + " px"; Layout.preferredWidth: 50 }
    }
}
