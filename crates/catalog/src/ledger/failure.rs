//! A file a pass couldn't make its result of, and how its error is kept. A leaf below `core` and the writers:
//! `Catalog`'s methods and the `LedgerWriter` take it, and the compiler graph puts an inherent method in its type's
//! module, so a type they name has to be below `core` too.

use pg_model::ids::FileId;
use pg_model::turns::Turns;

/// The longest error text kept (bytes; cut on a character's boundary).
pub const ERROR_BYTES: usize = 1024;

/// A file a pass couldn't make its result of, as the pass read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub file_id: FileId,
    /// The file's state when the pass read it: the row is kept only while the catalog still has the file so.
    pub mtime: i64,
    pub size: i64,
    pub rotation: Turns,
    pub error: String,
    /// Whether the pass tries the file again by itself (a decode error, which a pass tries until the file changes), or
    /// not until it changes or the user retries it (a panic).
    pub retry: bool,
}

/// `error` as kept: on one line (a control character is a space), at most [`ERROR_BYTES`].
pub(crate) fn tidy(error: &str) -> String {
    let mut out: String = error.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if out.len() > ERROR_BYTES {
        let cut = out.char_indices().map(|(i, _)| i).take_while(|&i| i <= ERROR_BYTES - 3).last().unwrap_or(0);
        out.truncate(cut);
        out.push('…');
    }
    out
}
