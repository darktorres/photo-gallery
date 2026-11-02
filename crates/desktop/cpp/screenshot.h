// The --screenshot test hook: the window saved as an image (src/screenshot.rs).
#pragma once

#include <QtCore/QString>
#include <QtQml/QQmlApplicationEngine>

namespace pg {

// `delay_ms` after the call, save the window the engine shows to `path` (its format by its extension), then quit.
void save_screenshot_later(QQmlApplicationEngine& engine, const QString& path, int delay_ms);

}  // namespace pg
