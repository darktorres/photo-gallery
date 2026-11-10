//! The cxx-qt bridge of [`crate::screenshot`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::screenshot::screenshot_taken;

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        include!("pg-desktop/cpp/screenshot.h");
        /// Once the window the engine shows has settled (at most `max_ms` after this call), hand it to
        /// `screenshot_taken`, then quit.
        #[namespace = "pg"]
        fn save_screenshot_later(engine: Pin<&mut QQmlApplicationEngine>, max_ms: i32);
    }

    extern "Rust" {
        #[namespace = "pg"]
        fn screenshot_taken(png: &[u8], settled: bool, has_window: bool, report: &str);
    }
}
