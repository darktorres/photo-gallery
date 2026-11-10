// The system clipboard. QML has no clipboard of its own (Qt Quick's text fields copy only their selection), so the
// inbox's "Copy" comes here.

#include "clipboard.h"

#include <QtGui/QClipboard>
#include <QtGui/QGuiApplication>

namespace pg {

void copy_text(const QString& text) {
    if (QClipboard* clipboard = QGuiApplication::clipboard()) {
        clipboard->setText(text);
    }
}

}  // namespace pg
