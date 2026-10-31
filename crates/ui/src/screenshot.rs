//! The `--screenshot=<png>` test hook (with `--wait=<s>`, 4 by default): save the window once it has settled, then
//! quit (cpp/screenshot.cpp). For checking the UI without a person at the screen, with the QML's own hooks
//! (Main.qml).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("pg-ui/cpp/screenshot.h");
        /// `delay_ms` after the call, save the window the engine shows to `path`, then quit.
        #[namespace = "pg"]
        fn save_screenshot_later(engine: Pin<&mut QQmlApplicationEngine>, path: &QString, delay_ms: i32);
    }
}

use core::pin::Pin;

use cxx_qt_lib::{QQmlApplicationEngine, QString};

use crate::command_line;

/// Schedule the screenshot if the command line asks for one.
pub fn schedule(engine: Pin<&mut QQmlApplicationEngine>) {
    let Some(path) = command_line::value("screenshot") else { return };
    let wait: f64 = command_line::value("wait").and_then(|w| w.trim().parse().ok()).unwrap_or(4.0);
    ffi::save_screenshot_later(engine, &QString::from(&path), (wait * 1000.0) as i32);
}
