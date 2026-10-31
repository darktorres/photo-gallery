// The --screenshot test hook. The window's frame as shown is read back (QQuickWindow::grabWindow), popups and menus
// included, at the screen's device pixel ratio.

#include "screenshot.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QTimer>
#include <QtGui/QImage>
#include <QtQuick/QQuickWindow>

#include <cstdio>

namespace pg {

void save_screenshot_later(QQmlApplicationEngine& engine, const QString& path, int delay_ms) {
    QTimer::singleShot(delay_ms, &engine, [&engine, path] {
        for (QObject* root : engine.rootObjects()) {
            if (auto* window = qobject_cast<QQuickWindow*>(root)) {
                if (!window->grabWindow().save(path))
                    std::fprintf(stderr, "screenshot: can't save %s\n", qUtf8Printable(path));
                break;
            }
        }
        QCoreApplication::quit();
    });
}

}  // namespace pg
