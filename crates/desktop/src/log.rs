//! The app's log: `gallery.log` in the data folder, for a problem a GUI program can't show on stderr (it has none).
//! `note` appends a line to it; the panic hook (`panic_log`) appends its reports with `append`. Before `init`, or when the
//! log can't be written, a line goes to stderr instead: a message is never dropped, and a terminal still shows it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The log `init` was given, for [`note`].
static LOG: OnceLock<PathBuf> = OnceLock::new();

/// Sets the log file that [`note`] appends to. Called once, at startup, before anything is noted.
pub(crate) fn init(log: PathBuf) {
    let _ = LOG.set(log);
}

/// Append a line to the log, for a problem a GUI program can't show on stderr. Before `init`, or when the log can't be
/// written, the line goes to stderr instead.
#[expect(clippy::print_stderr, reason = "the fallback: a line the log can't take still reaches a terminal")]
pub(crate) fn note(line: &str) {
    let text = format!("{} {line}\n", local_time());
    match LOG.get().map(|log| append(log, &text)) {
        Some(Ok(())) => {}
        Some(Err(_)) | None => eprint!("{text}"),
    }
}

/// Appends `text` to the file at `log`, creating it when it is missing.
pub(crate) fn append(log: &Path, text: &str) -> std::io::Result<()> {
    std::fs::OpenOptions::new().create(true).append(true).open(log)?.write_all(text.as_bytes())
}

/// `2026-10-30 17:04:16`, local time.
pub(crate) fn local_time() -> String {
    let t = pg_platform::clock::local_time();
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}
