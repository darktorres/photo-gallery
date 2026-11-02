//! Panics in the app. A GUI program has no console, so the default hook's message goes nowhere: a panic on the
//! UI thread or in an image provider (where it aborts: it can't unwind into Qt) closes the window without a word,
//! and one in the background job stops the import silently. The hook appends the message and a backtrace to
//! `gallery.log` beside the catalog folder (with `gallery.json`) and shows the message with where the log is.
//!
//! Crashes that aren't panics (an access violation in Qt or libjpeg-turbo) still end without a trace: that needs
//! a minidump writer, which isn't here yet.

use std::backtrace::Backtrace;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::PathBuf;

/// Log panics to `log` (and to stderr, when there is one) and tell the user.
pub fn install(log: PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let report = report(info);
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .and_then(|mut f| f.write_all(report.as_bytes()));
        let where_ = match written {
            Ok(()) => format!("The details are in {}.", log.display()),
            Err(e) => format!("Writing them to {} failed ({e}).", log.display()),
        };
        crate::message_box(&format!("Photo Gallery hit an internal error:\n\n{info}\n\n{where_}"));
    }));
}

fn report(info: &PanicHookInfo) -> String {
    let thread = std::thread::current();
    format!(
        "{} gallery {}, thread '{}': {info}\n{}\n\n",
        local_time(),
        env!("CARGO_PKG_VERSION"),
        thread.name().unwrap_or("unnamed"),
        Backtrace::force_capture()
    )
}

/// `2026-10-30 17:04:16`, local time.
fn local_time() -> String {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = SYSTEMTIME::default();
    // SAFETY: GetLocalTime only writes the SYSTEMTIME it is given.
    unsafe { GetLocalTime(&mut t) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}
