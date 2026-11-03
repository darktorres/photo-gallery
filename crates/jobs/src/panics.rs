//! Panics on a job's thread. A panic a job catches is reported by the job (its status line, the log), not as a box
//! on a GUI program: the process's panic hook asks [`caught`] before it shows one.

use std::any::Any;
use std::cell::Cell;
use std::panic::UnwindSafe;

thread_local! {
    /// Set while [`catch`] runs its closure on this thread.
    static CAUGHT: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f`, catching a panic (see the module doc). The caller must report the error it returns.
pub fn catch<T>(f: impl FnOnce() -> T + UnwindSafe) -> std::thread::Result<T> {
    CAUGHT.with(|c| c.set(true));
    let result = std::panic::catch_unwind(f);
    CAUGHT.with(|c| c.set(false));
    result
}

/// Whether a panic starting on this thread is inside [`catch`].
pub fn caught() -> bool {
    CAUGHT.with(Cell::get)
}

/// The message a panic carried, for the report.
pub fn message(payload: &(dyn Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s
    } else {
        "no message"
    }
}
