//! The error box the app shows, from any thread (it runs its own message loop). The one place that gives the window's
//! title: `fatal` in the root and the panic hook (`panic_log`) both show their messages through it.

/// Shows `message` in an error box titled with the app's name.
pub fn error_box(message: &str) {
    pg_platform::message::error_box("Photo Gallery", message);
}
