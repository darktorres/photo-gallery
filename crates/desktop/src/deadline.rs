//! The window's one deadline timer (S-12.18): the glue to cpp/deadline.cpp. The shell arms it to the controller's next deadline
//! (`controller::deadlines`), on the GUI thread. The timer is a child of the shell's QObject, and when it fires it queues the
//! shell's `timeout` invokable, so the pauses run as an ordinary event and never inside another call of the shell.

use std::pin::Pin;

use cxx_qt::QObject;
use cxx_qt::casting::Upcast;

use crate::deadline_bridge::ffi;

/// The shell's QObject, which the timer is a child of. The pointer lives as long as the shell, and only the shell's methods
/// (on the GUI thread) use it.
pub(crate) fn owner<S: Upcast<QObject>>(shell: Pin<&mut S>) -> *mut QObject {
    let mut base: Pin<&mut QObject> = shell.upcast_pin();
    // SAFETY: the pinned base is not moved out of; the pointer is only handed to the C++ timer, which the shell owns.
    core::ptr::from_mut(unsafe { base.as_mut().get_unchecked_mut() })
}

/// The monotonic clock the controller's deadlines are on, in milliseconds.
pub(crate) fn now_ms() -> u64 {
    // A plain read of the QElapsedTimer clock, on the GUI thread.
    let ms = ffi::deadline_now_ms();
    u64::try_from(ms).unwrap_or(0)
}

/// Fire the shell's timer in `ms` milliseconds, replacing any pending arm.
pub(crate) fn arm(owner: *mut QObject, ms: u64) {
    // SAFETY: `owner` is the shell's QObject (see `owner`), alive while the shell's methods run.
    unsafe { ffi::deadline_arm(owner, i64::try_from(ms).unwrap_or(i64::MAX)) }
}

/// Cancel the shell's pending arm, if any.
pub(crate) fn disarm(owner: *mut QObject) {
    // SAFETY: as for `arm`.
    unsafe { ffi::deadline_disarm(owner) }
}
