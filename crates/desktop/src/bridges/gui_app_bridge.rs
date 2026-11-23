//! The cxx-qt bridge of [`crate::gui_app`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qguiapplication.h");
        type QGuiApplication = cxx_qt_lib::QGuiApplication;

        include!("pg-desktop/cpp/gui_app.h");

        /// A QGuiApplication whose argc and argv are these arguments: NUL-terminated, one after another.
        #[namespace = "pg"]
        fn new_gui_application(args: &[u8]) -> UniquePtr<QGuiApplication>;
    }
}
