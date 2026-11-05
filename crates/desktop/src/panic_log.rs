//! Panics in the app. A GUI program has no console, so the default hook's message goes nowhere: a panic on the
//! UI thread or in an image provider (where it aborts: it can't unwind into Qt) closes the window without a word.
//! The hook appends the message and a backtrace to `gallery.log` beside the catalog folder (with `gallery.json`).
//! A panic that ends the process also shows a box with where the log is. A panic that the background work catches is only
//! logged: its caller reports it in the status line, and the app goes on.
//!
//! Crashes that aren't panics (an access violation in Qt or libjpeg-turbo) still end without a trace: that needs
//! a minidump writer, which isn't here yet.

use std::backtrace::Backtrace;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The log `install` was given, for [`note`].
static LOG: OnceLock<PathBuf> = OnceLock::new();

/// Log panics to `log` (and to stderr, when there is one), and show a box for those that end the process.
pub fn install(log: PathBuf) {
    let _ = LOG.set(log.clone());
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let caught = pg_app::reexport::panic_caught();
        let report = report(info);
        let written = append(&log, &report);
        let where_ = match written {
            Ok(()) => format!("The details are in {}.", log.display()),
            Err(e) => format!("Writing them to {} failed ({e}).", log.display()),
        };
        if !caught {
            crate::message_box(&format!("Photo Gallery hit an internal error:\n\n{info}\n\n{where_}"));
        }
    }));
}

/// Append a line to `gallery.log`, for a problem a GUI program can't show on stderr. Does nothing before
/// `install`.
pub fn note(line: &str) {
    if let Some(log) = LOG.get() {
        let _ = append(log, &format!("{} {line}\n", local_time()));
    }
}

fn append(log: &Path, text: &str) -> std::io::Result<()> {
    std::fs::OpenOptions::new().create(true).append(true).open(log)?.write_all(text.as_bytes())
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
    let t = pg_platform::clock::local_time();
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}
