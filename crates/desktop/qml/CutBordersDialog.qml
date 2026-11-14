// The confirmation before the borders come off the originals (the filter bar's "Cut borders from originals"). The originals
// are rewritten in place, with no backup, so this asks first. `count` is how many files (Main sets it from
// GalleryModel.cutBordersCount). Cancel has the focus when the dialog opens, so Enter does not confirm; Escape, a click
// outside and Cancel close it without a choice. Only "Cut borders" accepts, and `confirmed()` follows that accept, once.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Dialog {
    id: dialog
    Words {
        id: words
    }
    // How many files the borders come off, in the question.
    property int count: 0

    // The user chose "Cut borders": the library cuts them (Main connects it to GalleryModel.cutBordersConfirmed).
    signal confirmed()

    // The question's text and the two buttons, for the tests.
    property alias questionText: question.text
    property alias cancelButton: cancelBtn
    property alias cutButton: cutBtn

    title: words.text("ui-cut-borders-title")
    modal: true
    width: 420
    // Qt gives the focus to the first enabled button when the dialog shows; Cancel takes it once the dialog is open.
    onOpened: cancelBtn.forceActiveFocus()
    onAccepted: dialog.confirmed()

    Label {
        id: question
        width: dialog.availableWidth
        wrapMode: Text.WordWrap
        textFormat: Text.PlainText
        text: words.phrase("ui-cut-borders-question", dialog.count, Qt.locale().toString(dialog.count), "")
    }

    footer: DialogButtonBox {
        Button {
            id: cancelBtn
            text: words.text("ui-cancel")
            DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
        }
        Button {
            id: cutBtn
            text: words.text("ui-cut-borders")
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
    }
}
