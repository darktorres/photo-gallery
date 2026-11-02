// The GUI application, made from the command line's arguments (src/gui_app.rs).
#pragma once

#include "cxx-qt-lib/qguiapplication.h"
#include "rust/cxx.h"

#include <cstdint>
#include <memory>

namespace pg {

// A QGuiApplication whose argc and argv are these arguments: NUL-terminated, one after another.
std::unique_ptr<QGuiApplication> new_gui_application(rust::Slice<const std::uint8_t> args);

}  // namespace pg
