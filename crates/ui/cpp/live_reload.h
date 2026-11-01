// Debug builds: QML edits show in the running app without a rebuild (src/live_reload.rs).
#pragma once

#include <QtCore/QString>
#include <QtCore/QUrl>
#include <QtQml/QQmlApplicationEngine>

#include "gpu_device.h"

namespace pg {

// Fetch the QML and JS files in `main`'s qrc: folder from `source_dir` instead, and load `main` again (in place of
// the window it made) whenever one of them changes; the new window is shown with show_windows. Call before the
// first load. `gpu` outlives the engine.
void enable_live_reload(QQmlApplicationEngine& engine, const QUrl& main, const QString& source_dir, GpuDevice& gpu);

}  // namespace pg
