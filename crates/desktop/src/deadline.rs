//! The window's one deadline timer: the glue to cpp/deadline.cpp. The shell arms it to the controller's next deadline
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

#[cfg(test)]
mod tests {
    /// The C++ side reaches the shell by name: cpp/deadline.cpp invokes `timeout`, and cpp/screenshot.cpp reads the window's
    /// `shellCore` and `ready` and invokes `pending`. Each name is checked against its Rust or QML definition here, so a
    /// rename on either side fails the tests rather than failing silently at run time.
    #[test]
    fn the_names_the_cpp_side_reaches_by_string_exist() {
        let deadline = include_str!("../cpp/deadline.cpp");
        let screenshot = include_str!("../cpp/screenshot.cpp");
        let shell_bridge = include_str!("shell_bridge.rs");
        let main = include_str!("../qml/Main.qml");
        assert!(deadline.contains("\"timeout\""));
        assert!(shell_bridge.contains("fn timeout(self: Pin<&mut Shell>);"));
        assert!(screenshot.contains("\"pending\""));
        assert!(shell_bridge.contains("fn pending(self: &Shell) -> bool;"));
        assert!(screenshot.contains("\"shellCore\""));
        assert!(main.contains("readonly property var shellCore: shell"));
        assert!(screenshot.contains("\"ready\""));
        assert!(main.contains("property bool ready: false"));
    }
}
