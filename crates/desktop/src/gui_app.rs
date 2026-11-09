//! The GUI application, made from the arguments `main` read once at start. cxx-qt-lib's `QGuiApplication::new` reads
//! the process's arguments again and copies each one; this passes Qt the same arguments `command_line` reads
//! (cpp/gui_app.cpp).

use pg_platform::names::arg_bytes;

#[cxx_qt::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qguiapplication.h");
        type QGuiApplication = cxx_qt_lib::QGuiApplication;

        include!("pg-desktop/cpp/gui_app.h");

        /// A QGuiApplication whose argc and argv are these arguments: NUL-terminated, one after another.
        #[namespace = "pg"]
        fn new_gui_application(args: &[u8]) -> UniquePtr<QGuiApplication>;
    }
}

use std::ffi::OsString;

/// The application, with `args` (`main`'s: the program's name first) as Qt's argc and argv. Call on the main thread,
/// before any other Qt object.
pub fn new(args: &[OsString]) -> cxx::UniquePtr<cxx_qt_lib::QGuiApplication> {
    ffi::new_gui_application(&argv_bytes(args))
}

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
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    // A raw byte in an argument reaches Qt's argv as it is, the program's name first.
    #[test]
    fn a_raw_byte_argument_reaches_qts_argv_as_it_is() {
        let args = [OsString::from("gallery"), OsString::from_vec(b"--screenshot=/a/\xFF.png".to_vec())];
        assert_eq!(argv_bytes(&args), b"gallery\0--screenshot=/a/\xFF.png\0");
    }
}
