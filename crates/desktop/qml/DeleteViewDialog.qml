// The confirmation before a saved view is deleted (the views menu's Delete view). The view is only its name, folder, query
// and sort, so nothing in the library changes, but it is gone for good: this asks first. `view` is the view's name (Main
// sets it). Cancel has the focus when the dialog opens, so Enter does not confirm; Escape, a click outside and Cancel
// close it without a choice. Only "Delete" accepts, and `confirmed()` follows that accept, once.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Dialog {
    id: dialog
    Words {
        id: words
    }
    // Centred on the window: Main declares it under the window, where a dialog has no parent of its own to centre on.
    parent: Overlay.overlay
    anchors.centerIn: parent
    // The saved view's name.
    property string view: ""

    // The user chose "Delete": the view is removed (Main connects it to the shell's deleteView).
    signal confirmed

    // The question's text, the view's name and the two buttons, for the tests.
    property alias questionText: question.text
    property alias viewText: viewLabel.text
    property alias cancelButton: cancelBtn
    property alias deleteButton: deleteBtn
    property alias viewLabelItem: viewLabel
    property alias questionLabel: question

    title: words.text("ui-delete-view-title")
    modal: true
    width: 420
    // Qt gives the focus to the first enabled button when the dialog shows; Cancel takes it once the dialog is open.
    onOpened: cancelBtn.forceActiveFocus()
    onAccepted: dialog.confirmed()

    // A column, so the question sits under the view's name: the dialog sizes to its content.
    Column {
        width: dialog.availableWidth
        spacing: 8

        Label {
            id: viewLabel
            width: parent.width
            elide: Text.ElideMiddle
            textFormat: Text.PlainText
            font.bold: true
            text: dialog.view
        }

        Label {
            id: question
            width: parent.width
            wrapMode: Text.WordWrap
            textFormat: Text.PlainText
            text: words.text("ui-delete-view-question")
        }
    }

    footer: DialogButtonBox {
        Button {
            id: cancelBtn
            text: words.text("ui-cancel")
            DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
        }
        Button {
            id: deleteBtn
            text: words.text("ui-delete")
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
    }
}
