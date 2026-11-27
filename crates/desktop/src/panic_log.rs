//! Panics in the app. A GUI program has no console, so the default hook's message goes nowhere: a panic on the
//! UI thread or in an image provider (where it aborts: it can't unwind into Qt) closes the window without a word.
//! The hook records the message and a backtrace in the log (`pg_log::panic`, `gallery.log` in the data folder). A panic
//! that ends the process also shows a box with where the log is. A panic that the background work catches is only
//! logged: its caller reports it, and the app goes on.
//!
//! Crashes that aren't panics (an access violation in Qt or libjpeg-turbo) still end without a trace: that needs
//! a minidump writer, which isn't here yet.

use std::backtrace::Backtrace;
use std::cell::Cell;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

use crate::alert;

thread_local! {
    /// Set while [`recovered`] runs its closure on this thread.
    static RECOVERING: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f`, catching a panic the caller recovers from: its message is logged, and no box is shown (a box would block the
/// thread, and the startup that waits for it, though the caller carries on without the panicked work).
pub(crate) fn recovered<T>(f: impl FnOnce() -> T) -> std::thread::Result<T> {
    let outer = RECOVERING.with(|c| c.replace(true));
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    RECOVERING.with(|c| c.set(outer));
    result
}

/// Record panics in the log at `log_path` (and on stderr, when there is one), and show a box for those that end the
/// process.
pub(crate) fn install(log_path: PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let caught = pg_app::panic_caught() || RECOVERING.with(Cell::get);
        let thread = std::thread::current();
        let message = format!("thread '{}': {info}", thread.name().unwrap_or("unnamed"));
        let written = pg_log::panic(&message, &Backtrace::force_capture().to_string());
        if !caught {
            let log_error = written.err().map(|e| e.to_string());
            let shown =
                pg_wording::internal_error(&info.to_string(), &log_path.display().to_string(), log_error.as_deref());
            alert::error_box(&shown);
        }
    }));
}
