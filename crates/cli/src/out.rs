//! The command line's output. Every line it prints goes through one of these two functions: its reports to stdout, its
//! progress and errors to stderr. Each writes one line and returns the write's result, so a closed pipe (`gallery scan
//! | head`) is the command's error rather than a panic.

use std::fmt::Display;
use std::io::Write;

/// Writes `text` and a newline to stdout, as one line.
pub fn stdout_line(text: impl Display) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{text}")
}

/// Writes `text` and a newline to stderr, as one line.
pub fn stderr_line(text: impl Display) -> std::io::Result<()> {
    let mut err = std::io::stderr().lock();
    writeln!(err, "{text}")
}
