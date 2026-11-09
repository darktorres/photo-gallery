//! Panics in the app. A GUI program has no console, so the default hook's message goes nowhere: a panic on the
//! UI thread or in an image provider (where it aborts: it can't unwind into Qt) closes the window without a word.
//! The hook appends the message and a backtrace to `gallery.log` in the data folder (`pg_library::layout`, with `gallery.json`), through
//! `log`. A panic that ends the process also shows a box with where the log is. A panic that the background work catches
//! is only logged: its caller reports it in the status line, and the app goes on.
//!
//! Crashes that aren't panics (an access violation in Qt or libjpeg-turbo) still end without a trace: that needs
//! a minidump writer, which isn't here yet.

use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::path::PathBuf;

use crate::{alert, log};

/// Log panics to `gallery.log` at `log_path` (and to stderr, when there is one), and show a box for those that end the
/// process.
pub fn install(log_path: PathBuf) {
    log::init(log_path.clone());
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let caught = pg_app::reexport::panic_caught();
        let report = report(info);
        let written = log::append(&log_path, &report);
        if !caught {
            let log_error = written.err().map(|e| e.to_string());
            let message = info.to_string();
            let shown =
                pg_client::text::internal_error(&message, &log_path.display().to_string(), log_error.as_deref());
            alert::error_box(&shown);
        }
    }));
}

fn report(info: &PanicHookInfo) -> String {
    let thread = std::thread::current();
    format!(
        "{} gallery {}, thread '{}': {info}\n{}\n\n",
        log::local_time(),
        env!("CARGO_PKG_VERSION"),
        thread.name().unwrap_or("unnamed"),
        Backtrace::force_capture()
    )
}
