// The --screenshot test hook. The window's frame as shown is read back (QQuickWindow::grabWindow), popups and menus
// included, at the screen's device pixel ratio. The delay counts from when the window says it is `ready` (Main.qml:
// the first list shown and the test hooks run), which a cold start can take a while to reach.

#include "screenshot.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QTimer>
#include <QtGui/QImage>
#include <QtQuick/QQuickWindow>

#include <cstdio>

namespace pg {

namespace {

QQuickWindow* window_of(QQmlApplicationEngine& engine) {
    for (QObject* root : engine.rootObjects())
        if (auto* window = qobject_cast<QQuickWindow*>(root))
            return window;
    return nullptr;
}

}  // namespace

void save_screenshot_later(QQmlApplicationEngine& engine, const QString& path, int delay_ms) {
    // Polled: the window's `ready` is a QML property, and this file has no moc for a slot to connect.
    auto* poll = new QTimer(&engine);
    poll->setInterval(50);
    QObject::connect(poll, &QTimer::timeout, &engine, [&engine, poll, path, delay_ms] {
        QQuickWindow* window = window_of(engine);
        const QVariant ready = window ? window->property("ready") : QVariant();
        // A window without the property is ready as it is.
        if (window && ready.isValid() && !ready.toBool())
            return;
        poll->stop();
        poll->deleteLater();
        QTimer::singleShot(delay_ms, &engine, [&engine, path] {
            if (QQuickWindow* window = window_of(engine))
                if (!window->grabWindow().save(path))
                    std::fprintf(stderr, "screenshot: can't save %s\n", qUtf8Printable(path));
            QCoreApplication::quit();
        });
    });
    poll->start();
}

}  // namespace pg
