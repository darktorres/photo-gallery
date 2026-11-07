//! Message boxes, for the failures the user has to see when there is no console to print to.

/// An error message box with `caption` and `text`, from any thread (it runs its own message loop). Elsewhere there
/// is no window, so nothing is shown: the caller has logged the error.
#[cfg(windows)]
pub fn error_box(caption: &str, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(text), wide(caption));
    // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_ICONERROR) };
}

/// Elsewhere, see above.
#[cfg(not(windows))]
pub fn error_box(_caption: &str, _text: &str) {}
