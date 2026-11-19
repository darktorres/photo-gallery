// The confirmation before a folder leaves the library for good (the library panel's Remove). Its images are deleted from
// the catalog with their ratings, analyses and thumbnails, so this asks first; the files on disk are not touched. `folder`
// is the folder as the panel shows it, and `row` the panel's row of it (Main sets both). Cancel has the focus when the
// dialog opens, so Enter does not confirm; Escape, a click outside and Cancel close it without a choice. Only "Remove"
// accepts, and `confirmed()` follows that accept, once.
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
    // The folder shown, and its row in the library panel.
    property string folder: ""
    property int row: -1

    // The user chose "Remove": the library removes the folder (Main connects it to the library's removeFolder).
    signal confirmed()

    // The question's text, the folder's and the two buttons, for the tests.
    property alias questionText: question.text
    property alias folderText: folderLabel.text
    property alias cancelButton: cancelBtn
    property alias removeButton: removeBtn

    title: words.text("ui-remove-folder-title")
    modal: true
    width: 420
    // Qt gives the focus to the first enabled button when the dialog shows; Cancel takes it once the dialog is open.
    onOpened: cancelBtn.forceActiveFocus()
    onAccepted: dialog.confirmed()

    Label {
        id: folderLabel
        width: dialog.availableWidth
        elide: Text.ElideMiddle
        textFormat: Text.PlainText
        font.bold: true
        text: dialog.folder
    }

    Label {
        id: question
        width: dialog.availableWidth
        wrapMode: Text.WordWrap
        textFormat: Text.PlainText
        text: words.text("ui-remove-folder-question")
    }

    footer: DialogButtonBox {
        Button {
            id: cancelBtn
            text: words.text("ui-cancel")
            DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
        }
        Button {
            id: removeBtn
            text: words.text("ui-remove")
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
    }
}
