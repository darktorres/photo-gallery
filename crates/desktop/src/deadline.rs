//! The window's one deadline timer (S-12.18): the glue to cpp/deadline.cpp. The shell arms it to the core's next deadline
//! (`controller::deadlines`), on the GUI thread. When it fires, `deadline_fired` queues the shell's timeout handler, so the
//! pauses run as an ordinary event and never inside another call of the shell.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("pg-desktop/cpp/deadline.h");
        #[namespace = "pg"]
        fn deadline_arm(ms: i64);
        #[namespace = "pg"]
        fn deadline_disarm();
        #[namespace = "pg"]
        fn deadline_now_ms() -> i64;
    }

    extern "Rust" {
        #[namespace = "pg"]
        fn deadline_fired();
    }
}

use std::cell::RefCell;

use cxx_qt::CxxQtThread;

use crate::shell::qobject::Shell;

thread_local! {
    /// The shell's thread handle, kept by the shell on each change (`remember`). Only the GUI thread touches it.
    static SHELL: RefCell<Option<CxxQtThread<Shell>>> = const { RefCell::new(None) };
}

/// Keep the shell's thread handle, so the timer can reach the shell's timeout handler.
pub(crate) fn remember(thread: CxxQtThread<Shell>) {
    SHELL.with(|shell| *shell.borrow_mut() = Some(thread));
}

/// The monotonic clock the core's deadlines are on, in milliseconds.
pub(crate) fn now_ms() -> u64 {
    // A plain read of the QElapsedTimer clock, on the GUI thread.
    let ms = ffi::deadline_now_ms();
    u64::try_from(ms).unwrap_or(0)
}

/// Fire the timer in `ms` milliseconds, replacing any pending arm.
pub(crate) fn arm(ms: u64) {
    // Called on the GUI thread, where the QTimer lives; the C++ side only restarts it.
    ffi::deadline_arm(i64::try_from(ms).unwrap_or(i64::MAX));
}

/// Cancel the pending arm, if any.
pub(crate) fn disarm() {
    ffi::deadline_disarm();
}

/// The timer fired (on the GUI thread): queue the shell's timeout, which runs the pauses that are due.
fn deadline_fired() {
    SHELL.with(|shell| {
        if let Some(thread) = shell.borrow().as_ref() {
            let _ = thread.queue(|shell| shell.timeout());
        }
    });
}
