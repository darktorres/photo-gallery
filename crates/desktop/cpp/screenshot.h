// The --screenshot test hook: the window handed to Rust as a PNG (src/screenshot.rs).
#pragma once

#include <QtQml/QQmlApplicationEngine>

namespace pg {

// Once the window has settled (screenshot.cpp), encode the window the engine shows as a PNG and hand it to Rust
// (screenshot_taken), then quit. `max_ms` is the most to wait for it to settle, after the call.
void save_screenshot_later(QQmlApplicationEngine& engine, int max_ms);

}  // namespace pg
