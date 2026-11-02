// The window icon of every window: the title bar and the taskbar (src/app_icon.rs).
#pragma once

namespace pg {

// Set the application's window icon from the PNG in the QML module's resources. Call after the QGuiApplication exists.
void set_app_icon();

}  // namespace pg
