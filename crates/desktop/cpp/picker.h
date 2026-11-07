// The folder dialog's owner: the window the user works in (src/picker.rs).
#pragma once

#include <cstddef>

namespace pg {

// The native handle of the app's window: its HWND on Windows, 0 when no window is shown. The folder dialog is owned by it,
// so it stays on top of the app while it is open.
std::size_t main_window_handle();

}  // namespace pg
