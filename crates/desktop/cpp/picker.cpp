// The folder dialog's owner (picker.h): the focused window when it is shown (a click in it opens the dialog), else the
// first top-level window that is.

#include "picker.h"

#include <QtGui/QGuiApplication>
#include <QtGui/QWindow>

namespace pg {

std::size_t main_window_handle() {
    QWindow* window = QGuiApplication::focusWindow();
    if (!window || !window->isVisible()) {
        window = nullptr;
        for (QWindow* top : QGuiApplication::topLevelWindows()) {
            if (top->isVisible()) {
                window = top;
                break;
            }
        }
    }
    return window ? static_cast<std::size_t>(window->winId()) : 0;
}

}  // namespace pg
