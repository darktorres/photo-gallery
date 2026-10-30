// Debug builds: QML edits show in the running app without a rebuild (src/live_reload.rs).
#pragma once

#include <QtCore/QString>
#include <QtCore/QUrl>
#include <QtQml/QQmlApplicationEngine>

namespace pg {

// Fetch the QML and JS files in `main`'s qrc: folder from `source_dir` instead, and load `main` again (in place of
// the window it made) whenever one of them changes. Call before the first load.
void enable_live_reload(QQmlApplicationEngine& engine, const QUrl& main, const QString& source_dir);

}  // namespace pg
