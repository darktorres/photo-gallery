// Debug builds: QML edits show in the running app without a rebuild (src/live_reload.rs).
#pragma once

#include <QtCore/QString>
#include <QtCore/QUrl>
#include <QtQml/QQmlApplicationEngine>

#include "gpu_device.h"

namespace pg {

// Fetch the QML and JS files in `main`'s qrc: folder from `source_dir` instead. Call before the first load.
void read_qml_from_source(QQmlApplicationEngine& engine, const QUrl& main, const QString& source_dir);

// Load `main` again (in place of the window it made) whenever a file in `source_dir` changes; the new window is
// shown with show_windows. `gpu` outlives the engine.
void reload_on_change(QQmlApplicationEngine& engine, const QUrl& main, const QString& source_dir, GpuDevice& gpu);

}  // namespace pg
