//! The GUI application, made from the arguments `main` read once at start. cxx-qt-lib's `QGuiApplication::new` reads
//! the process's arguments again and copies each one; this passes Qt the same arguments `command_line` reads
//! (cpp/gui_app.cpp).

use crate::gui_app_bridge::ffi;
use pg_platform::args::arg_bytes;

use std::ffi::OsString;

// The static Qt's plugin registrations (cpp/static_qt.cpp), in the static build only (build.rs sets the cfg).
#[cfg(pg_static_qt)]
#[expect(unsafe_code, reason = "the static build's registration object, declared by cpp/static_qt.cpp")]
unsafe extern "C" {
    fn pg_static_qt_link();
}

/// The application, with `args` (`main`'s: the program's name first) as Qt's argc and argv. Call on the main thread,
/// before any other Qt object.
pub(crate) fn new(args: &[OsString]) -> cxx::UniquePtr<cxx_qt_lib::QGuiApplication> {
    keep_static_qt_plugins();
    ffi::new_gui_application(&argv_bytes(args))
}

/// Calls the static build's registration object, so the linker keeps the plugins it registers (a static Qt loads none
/// from disk). Does nothing in the shared build.
#[cfg(pg_static_qt)]
#[expect(unsafe_code, reason = "calls the static build's registration object, which is an empty function")]
fn keep_static_qt_plugins() {
    // SAFETY: pg_static_qt_link is an empty function in cpp/static_qt.cpp, linked into this binary by build.rs.
    unsafe { pg_static_qt_link() }
}

#[cfg(not(pg_static_qt))]
fn keep_static_qt_plugins() {}

/// The arguments as Qt's argv holds them: each one NUL-terminated, in order.
fn argv_bytes(args: &[OsString]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for arg in args {
        bytes.extend_from_slice(&arg_bytes(arg));
        bytes.push(0);
    }
    bytes
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    // A raw byte in an argument reaches Qt's argv as it is, the program's name first.
    #[test]
    fn a_raw_byte_argument_reaches_qts_argv_as_it_is() {
        let mut odd = pg_platform::testing::odd_name("--screenshot=/a/");
        odd.push(".png");
        let args = [OsString::from("gallery"), odd];
        assert_eq!(argv_bytes(&args), b"gallery\0--screenshot=/a/\xFF.png\0");
    }
}
