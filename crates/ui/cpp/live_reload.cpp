// QML live reload for debug builds.
//
// The files keep their qrc: names: a URL interceptor only changes where the engine reads them from, so imports and
// types (the pg.gallery module, the Ratings singleton) resolve exactly as with the compiled-in copy. A change
// deletes the window and loads Main.qml again, where the old window was. The Rust objects the QML made are made
// again; what lives in Rust statics (the catalog, the background job) carries on.

#include "live_reload.h"

#include <QtCore/QDir>
#include <QtCore/QFileSystemWatcher>
#include <QtCore/QTimer>
#include <QtGui/QGuiApplication>
#include <QtQml/QQmlAbstractUrlInterceptor>
#include <QtQuick/QQuickWindow>

namespace pg {
namespace {

class SourceInterceptor : public QQmlAbstractUrlInterceptor {
public:
    // `prefix` is the qrc: path of the module's QML folder, `dir` the folder on disk; both end in '/'.
    SourceInterceptor(QString prefix, QString dir) : m_prefix(std::move(prefix)), m_dir(std::move(dir)) {}

    QUrl intercept(const QUrl& url, DataType type) override {
        if ((type != QmlFile && type != JavaScriptFile) || url.scheme() != u"qrc" || !url.path().startsWith(m_prefix))
            return url;
        return QUrl::fromLocalFile(m_dir + url.path().mid(m_prefix.size()));
    }

private:
    QString m_prefix;
    QString m_dir;
};

// Reloading deletes the window, so the app doesn't quit on its last window closing: it quits when the user closes
// the window instead.
void quit_on_close(QObject* root) {
    if (auto* window = qobject_cast<QWindow*>(root)) {
        QObject::connect(window, &QWindow::visibleChanged, qApp, [](bool visible) {
            if (!visible) QCoreApplication::quit();
        });
    }
}

}  // namespace

void enable_live_reload(QQmlApplicationEngine& engine, const QUrl& main, const QString& source_dir) {
    const QString dir = QDir(source_dir).absolutePath() + u'/';
    // Never removed: the engine uses it until it is destroyed, at exit.
    engine.addUrlInterceptor(new SourceInterceptor(main.adjusted(QUrl::RemoveFilename).path(), dir));

    QGuiApplication::setQuitOnLastWindowClosed(false);
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreated, &engine, [](QObject* root, const QUrl&) {
        if (root) quit_on_close(root);
    });

    auto* watcher = new QFileSystemWatcher(&engine);
    // An editor that saves by writing a new file and renaming it over the old one ends the watch on that file: the
    // folder's change brings it back.
    auto watch_files = [watcher, dir] {
        for (const QString& name : QDir(dir).entryList({"*.qml", "*.js"}, QDir::Files)) {
            if (!watcher->files().contains(dir + name)) watcher->addPath(dir + name);
        }
    };
    watcher->addPath(dir);
    watch_files();

    // One reload per save, however many change signals it makes.
    auto* settle = new QTimer(&engine);
    settle->setSingleShot(true);
    settle->setInterval(100);
    QObject::connect(watcher, &QFileSystemWatcher::fileChanged, settle, qOverload<>(&QTimer::start));
    QObject::connect(watcher, &QFileSystemWatcher::directoryChanged, settle, qOverload<>(&QTimer::start));

    QObject::connect(settle, &QTimer::timeout, &engine, [&engine, main, watch_files] {
        watch_files();
        QVariantMap geometry;
        bool maximized = false;
        for (QObject* root : engine.rootObjects()) {
            if (auto* window = qobject_cast<QWindow*>(root)) {
                QObject::disconnect(window, &QWindow::visibleChanged, qApp, nullptr);
                maximized = window->visibility() == QWindow::Maximized;
                if (!maximized) {
                    geometry = {{"x", window->x()}, {"y", window->y()}, {"width", window->width()},
                                {"height", window->height()}};
                }
            }
            delete root;
        }
        // Only once nothing made from them is left: the cached components' bindings stop working.
        engine.clearComponentCache();
        engine.setInitialProperties(geometry);
        engine.load(main);
        if (maximized) {
            for (QObject* root : engine.rootObjects()) {
                if (auto* window = qobject_cast<QWindow*>(root)) window->showMaximized();
            }
        }
    });
}

}  // namespace pg
