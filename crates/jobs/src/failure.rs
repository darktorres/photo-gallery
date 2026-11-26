//! The failures a pass reports: a file or folder it couldn't process, and why (`pg_model::notice::FileFailure`, the one
//! shape of a failed file). A pass keeps the first few for its stats and counts the rest.
//!
//! A job's failure is a [`Failed`]: its [`Class`] (what the pass does about it) and its text, with its source kept so the
//! chain can be walked. A library turns its own error into a `Failed` where it crosses into a pass, and that is the one
//! place its class is decided.

use std::error::Error;

use pg_model::job::Cause;
use pg_model::name::Name;
use pg_model::notice::FileFailure;
use pg_model::rules::{Path, PathRules};
use pg_platform::file::ReadFailed;

/// What a failure is, for what a pass does about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// A panic a job caught on the file: a bug, which would come back at every pass, so it is recorded (its file
    /// is tried again once the file changes, or the user asks).
    Panic,
    /// The file couldn't be read this time (a locked file, an offline drive, a file gone): the next pass tries it again,
    /// and nothing is recorded.
    Read,
    /// The bytes were read and don't decode: the file's fault, recorded, and tried again once the file changes.
    Data,
    /// Anything else: tried again by the next pass, and nothing is recorded.
    Other,
}

/// A job's failure on one item, or a pass's own: its class, its text (the error's chain, as [`pg_model::error::chain`]
/// writes it), and its source, which [`Error::source`] returns.
#[derive(Debug)]
pub struct Failed {
    class: Class,
    text: String,
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
}

/// A panic that a job caught (`panics::guard`): the message it carried. Its text is that message alone; the words that say
/// it was a panic are the app's (`pg_wording`), which tells it apart from other errors by its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panicked(pub String);

impl std::fmt::Display for Panicked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Panicked {}

impl Failed {
    /// A failure of `class` on `error`: its text is `error`'s chain, and `error` is its source.
    pub fn new<E>(class: Class, error: E) -> Failed
    where
        E: Error + Send + Sync + 'static,
    {
        Failed { class, text: pg_model::error::chain(&error), source: Some(Box::new(error)) }
    }

    /// A panic a job caught, with its message: the text, and a [`Panicked`] source.
    pub fn panic(message: String) -> Failed {
        Failed { class: Class::Panic, text: message.clone(), source: Some(Box::new(Panicked(message))) }
    }

    /// The failure's class.
    pub fn class(&self) -> Class {
        self.class
    }

    /// The failure's text: the error's chain, or a caught panic's message.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether a job caught a panic here.
    pub fn is_panic(&self) -> bool {
        self.class == Class::Panic
    }

    /// Whether the read of the file failed (the class [`Class::Read`]).
    pub fn is_read(&self) -> bool {
        self.class == Class::Read
    }

    /// Whether the file changed while it was read whole ([`ChangedWhileRead`], a [`Class::Read`]): its bytes are not the
    /// file's, so nothing made of them is stored, and the file stays due (section 7).
    pub fn is_changed(&self) -> bool {
        self.source.as_deref().is_some_and(|source| source.is::<ChangedWhileRead>())
    }

    /// The same failure with `what` before its text (`what: text`), its class and its source kept.
    pub fn context(self, what: &str) -> Failed {
        Failed { class: self.class, text: format!("{what}: {}", self.text), source: self.source }
    }

    /// A pass's own failure, with no cause under it: the class [`Class::Other`], and `text` as both its text and its
    /// source's.
    pub fn other(text: String) -> Failed {
        Failed::new(Class::Other, Note(text))
    }
}

/// A file read whole whose bytes were not as many as its length when it was opened: it changed while it was read (a copy
/// still in progress, a program writing it), so the bytes are part of one version or of two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangedWhileRead {
    /// The file's length when it was opened.
    pub opened: u64,
    /// The bytes read from it.
    pub read: u64,
}

impl std::fmt::Display for ChangedWhileRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the file changed while it was read ({} bytes when opened, {} read)", self.opened, self.read)
    }
}

impl Error for ChangedWhileRead {}

/// A failure's text, as an error of its own (the source of [`Failed::other`]).
#[derive(Debug)]
struct Note(String);

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for Note {}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl Error for Failed {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_deref().map(|source| -> &(dyn Error + 'static) { source })
    }
}

/// An I/O error is a failed read: a file that could not be read or gone, with the read's context in the text.
impl From<std::io::Error> for Failed {
    fn from(error: std::io::Error) -> Failed {
        Failed::new(Class::Read, ReadFailed::from(error))
    }
}

/// The failures a pass keeps for its stats. Past this many, only the count grows: a folder that fails on every file
/// doesn't fill the stats.
pub const FAILURES_KEPT: usize = 20;

/// Keeps `failure` if there is room, as a pass's stats do.
pub fn keep_file_failure(kept: &mut Vec<FileFailure>, failure: FileFailure) {
    if kept.len() < FAILURES_KEPT {
        kept.push(failure);
    }
}

/// Keeps a failure of a plain error (not a caught panic) if there is room, as a pass's stats do.
pub fn keep_failure(kept: &mut Vec<FileFailure>, path: Path, error: impl std::fmt::Display) {
    if kept.len() < FAILURES_KEPT {
        keep_file_failure(kept, FileFailure { path, error: error.to_string(), panicked: false });
    }
}

/// The failure of the file at `path` on `failed`: a caught panic's message, or the error's text.
pub fn file_failure(path: Path, failed: &Failed) -> FileFailure {
    let (error, panicked) = match Cause::from(failed) {
        Cause::Error(text) => (text, false),
        Cause::Panicked(message) => (message, true),
    };
    FileFailure { path, error, panicked }
}

/// The path of a file from the bytes it is stored as (`WorkList::path`, the rules' join), for a failure to name it. The
/// stored bytes parse as the rules' path; bytes that don't (none is stored, as no name holds a NUL) are kept whole as
/// one name under an empty anchor, so the failure still names the file.
pub fn stored_path(rules: PathRules, bytes: &[u8]) -> Path {
    rules.parse_bytes(bytes).unwrap_or_else(|| Path { anchor: Name::from_raw(&[]), parts: vec![Name::from_raw(bytes)] })
}

/// The cause of a job's failure, for the app to word: a panic if the failure is one ([`crate::panics::guard`]), else its
/// text. The cause is the model's (`pg_model::job::Cause`); this crate builds it from a failure.
impl From<&Failed> for Cause {
    fn from(failed: &Failed) -> Cause {
        if failed.is_panic() {
            Cause::Panicked(failed.text().to_string())
        } else {
            Cause::Error(failed.text().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_io_error_is_a_read_and_keeps_its_text_and_source() {
        let failed = Failed::from(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
        assert_eq!(failed.class(), Class::Read);
        assert!(failed.is_read());
        assert!(failed.source().is_some());
        assert!(failed.text().contains("gone"), "{}", failed.text());
    }

    #[test]
    fn a_panic_is_its_message_and_its_class() {
        let failed = Failed::panic("the file fails".into());
        assert_eq!((failed.class(), failed.text()), (Class::Panic, "the file fails"));
        assert!(failed.is_panic());
        assert_eq!(Cause::from(&failed), Cause::Panicked("the file fails".into()));
        let source = failed.source().expect("a panic's source");
        assert_eq!(source.to_string(), "the file fails");
    }

    #[test]
    fn a_failure_of_any_class_is_said_by_its_text() {
        let failed = Failed::new(Class::Data, ReadFailed::from(std::io::Error::other("cut short")));
        assert_eq!(Cause::from(&failed), Cause::Error(failed.text().to_string()));
        assert_eq!(failed.class(), Class::Data);
    }
}
