//! The GUI application, made from the arguments `main` read once at start. cxx-qt-lib's `QGuiApplication::new` reads
//! the process's arguments again and copies each one; this passes Qt the same arguments `command_line` reads
//! (cpp/gui_app.cpp).

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

// The static Qt's plugin registrations (cpp/static_qt.cpp), in the static build only (build.rs sets the cfg).
#[cfg(pg_static_qt)]
unsafe extern "C" {
    fn pg_static_qt_link();
}

/// The application, with `args` (`main`'s: the program's name first) as Qt's argc and argv. Call on the main thread,
/// before any other Qt object.
pub fn new(args: &[OsString]) -> cxx::UniquePtr<cxx_qt_lib::QGuiApplication> {
    keep_static_qt_plugins();
    ffi::new_gui_application(&argv_bytes(args))
}

/// Calls the static build's registration object, so the linker keeps the plugins it registers (a static Qt loads none
/// from disk). Does nothing in the shared build.
#[cfg(pg_static_qt)]
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

/// One argument's bytes. Unix: exactly as the system holds them, a raw byte included. Windows: its text, converted
/// lossily as cxx-qt-lib passes them; Qt reads its own arguments from the command line there, exactly (probed), and
/// the app reads its arguments from `command_line`, so the argv only has to be well formed.
#[cfg(unix)]
fn arg_bytes(arg: &OsString) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    arg.as_bytes().to_vec()
}

/// One argument's bytes: its text (lossy), as cxx-qt-lib passes them (see the Unix one).
#[cfg(windows)]
fn arg_bytes(arg: &OsString) -> Vec<u8> {
    arg.to_string_lossy().as_bytes().to_vec()
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
