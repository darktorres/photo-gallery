// The confirmation before a removal kept for its unfinished writes is given up (the library panel's Discard). The changes
// not yet written into the folder's files are dropped and their temporary copies deleted, and the originals they were being
// written into may be left partly written, so this asks first. `folder` is the folder as the panel shows it (Main sets it).
// Cancel has the focus when the dialog opens, so Enter does not confirm; Escape, a click outside and Cancel close it without
// a choice. Only "Discard changes" accepts, and `confirmed()` follows that accept, once.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Dialog {
    id: dialog
    Words {
        id: words
    }
    // The folder shown.
    property string folder: ""

    // The user chose "Discard changes": the library gives the removal up (Main connects it to the library's discardRemoval).
    signal confirmed()

    // The question's text, the folder's and the two buttons, for the tests.
    property alias questionText: question.text
    property alias folderText: folderLabel.text
    property alias cancelButton: cancelBtn
    property alias discardButton: discardBtn

    title: words.text("ui-removal-discard-title")
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
        text: words.text("ui-removal-discard-question")
    }

    footer: DialogButtonBox {
        Button {
            id: cancelBtn
            text: words.text("ui-cancel")
            DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
        }
        Button {
            id: discardBtn
            text: words.text("ui-removal-discard-confirm")
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
    }
}
