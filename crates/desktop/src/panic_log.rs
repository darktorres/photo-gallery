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
        if !caught {
            let log_error = written.err().map(|e| e.to_string());
            let message = info.to_string();
            let shown = pg_client::text::internal_error(&message, &log.display().to_string(), log_error.as_deref());
            crate::message_box(&shown);
        }
    }));
}

/// Append a line to `gallery.log`, for a problem a GUI program can't show on stderr. Before `install`, or when the log
/// can't be written, the line goes to stderr instead: a message is never dropped, and a terminal still shows it.
#[expect(clippy::print_stderr, reason = "the fallback: a line the log can't take still reaches a terminal")]
pub fn note(line: &str) {
    let text = format!("{} {line}\n", local_time());
    match LOG.get().map(|log| append(log, &text)) {
        Some(Ok(())) => {}
        Some(Err(_)) | None => eprint!("{text}"),
    }
}

/// `name` in the folder that holds `gallery.log` (what the app keeps beside its log); `None` before `install`.
pub fn beside_log(name: &str) -> Option<PathBuf> {
    LOG.get().and_then(|log| log.parent()).map(|folder| folder.join(name))
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
