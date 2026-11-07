//! The `--screenshot=<png>` test hook (with `--wait=<s>`, the most to wait for the window to settle, 10 by default):
//! save the window once it has settled (ready, no image loading, no QML timer pending, no animation, no library job or
//! write pending, no frame for a while), then
//! quit (cpp/screenshot.cpp). The reason it was taken goes to `<png>.settle.txt`. For checking the UI without a person
//! at the screen, with the QML's own hooks (Main.qml).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("pg-desktop/cpp/screenshot.h");
        /// Once the window the engine shows has settled (at most `max_ms` after this call), save it to `path`, then quit.
        #[namespace = "pg"]
        fn save_screenshot_later(engine: Pin<&mut QQmlApplicationEngine>, path: &QString, max_ms: i32);
    }
}

use core::pin::Pin;

use cxx_qt_lib::{QQmlApplicationEngine, QString};

use crate::command_line;

/// Schedule the screenshot if the command line asks for one.
pub fn schedule(engine: Pin<&mut QQmlApplicationEngine>) {
    let Some(path) = command_line::value("screenshot") else { return };
    let wait: f64 = command_line::value("wait").and_then(|w| w.trim().parse().ok()).unwrap_or(10.0);
    ffi::save_screenshot_later(engine, &QString::from(path), millis(wait));
}

/// Seconds as whole milliseconds, within an int's range (NaN is 0).
fn millis(seconds: f64) -> i32 {
    pg_model::num::f64_to_i32((seconds * 1000.0).clamp(f64::from(i32::MIN), f64::from(i32::MAX)))
}
