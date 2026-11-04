// The --screenshot test hook: the window saved as an image (src/screenshot.rs).
#pragma once

#include <QtCore/QString>
#include <QtQml/QQmlApplicationEngine>

namespace pg {

// Once the window has settled (screenshot.cpp), save the window the engine shows to `path` (its format by its
// extension) and quit. `max_ms` is the most to wait for it to settle, after the call.
void save_screenshot_later(QQmlApplicationEngine& engine, const QString& path, int max_ms);

}  // namespace pg
