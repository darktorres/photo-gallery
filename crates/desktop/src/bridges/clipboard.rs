//! The system clipboard (cpp/clipboard.cpp): the inbox copies a notice's words there.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("pg-desktop/cpp/clipboard.h");
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        /// Put `text` on the clipboard. Call on the GUI thread.
        #[namespace = "pg"]
        fn copy_text(text: &QString);
    }
}
