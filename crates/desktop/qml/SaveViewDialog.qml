// Names the current view (its folder, subfolders, filters and sort) and saves it under that name. Made the first time it
// is asked for (Main's saveViewLoader), then kept: each rise of `requests` opens it. Saving is a signal; Main saves.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts

Dialog {
    id: dialog
    // The item the dialog is centred over (Main's page): a Loader doesn't parent a popup to an item.
    required property Item host
    // Opens the dialog each time it rises (Main counts the asks).
    required property int requests
    // The view being saved, as the dialog's line describes it.
    required property string folder
    required property bool recursive
    required property bool descending
    required property string filters
    required property string sortName

    // The name typed, when the dialog is accepted.
    signal saved(string name)

    parent: host
    anchors.centerIn: parent
    title: "Save the current view"
    modal: true
    standardButtons: Dialog.Save | Dialog.Cancel
    onRequestsChanged: dialog.open()
    onAboutToShow: {
        viewName.text = ""
        viewName.forceActiveFocus()
    }
    onAccepted: dialog.saved(viewName.text)

    ColumnLayout {
        Label {
            text: (dialog.folder.length ? dialog.folder : "All folders")
                + (dialog.recursive ? " and subfolders" : "") + " · " + dialog.filters + " · "
                + dialog.sortName + (dialog.descending ? " ↓" : " ↑")
            color: "#999999"
        }
        TextField {
            id: viewName
            placeholderText: "Name (one already saved is replaced)"
            Layout.preferredWidth: 400
            onAccepted: dialog.accept()
        }
    }
}
