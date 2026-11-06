//! The GUI application, made from the arguments `main` read once at start. cxx-qt-lib's `QGuiApplication::new` reads
//! the process's arguments again and copies each one; this passes Qt the same bytes `command_line` reads
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

/// The application, with `args` (`main`'s: the program's name first) as Qt's argc and argv. Call on the main thread,
/// before any other Qt object.
pub fn new(args: &[std::ffi::OsString]) -> cxx::UniquePtr<cxx_qt_lib::QGuiApplication> {
    let mut bytes = Vec::new();
    for arg in args {
        // Windows: WTF-8, converted lossily, as cxx-qt-lib passes them.
        bytes.extend_from_slice(arg.to_string_lossy().as_bytes());
        bytes.push(0);
    }
    ffi::new_gui_application(&bytes)
}
