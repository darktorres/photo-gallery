// The system clipboard (src/clipboard.rs): what the inbox's "Copy" puts there.
#pragma once

#include <QtCore/QString>

namespace pg {

// Put `text` on the clipboard, as plain text. Call on the GUI thread, after the QGuiApplication exists.
void copy_text(const QString& text);

}  // namespace pg
