//! A file a pass couldn't make its result of, and how its error is kept. A leaf below `core` and the writers (the rule is
//! in the crate's docs).

use pg_model::ids::FileId;
use pg_model::turns::Turns;

use crate::error::Result;

/// The longest error text kept (bytes; cut on a character's boundary).
pub const ERROR_BYTES: usize = 1024;

/// A file a pass couldn't make its result of, as the pass read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// The file the failure is about.
    pub file_id: FileId,
    /// The file's state when the pass read it: the row is kept only while the catalog still has the file so.
    pub mtime: i64,
    /// The file's size when the pass read it.
    pub size: u64,
    /// The file's stored turns (`files.turns`) when the pass read it.
    pub turns: Turns,
    /// The error the pass recorded, as text.
    pub error: String,
    /// Whether the pass tries the file again by itself. `false` is kept until the file changes or the user retries it:
    /// a decode error and a panic are both recorded that way (the thumbnails' writer does so), and no failure is
    /// recorded with `true` today.
    pub retry: bool,
}

/// `error` as kept: on one line (a control character is a space), at most [`ERROR_BYTES`].
pub(crate) fn kept_error(error: &str) -> String {
    pg_model::notice::tidy(error, ERROR_BYTES)
}

/// The thumbnails' panicked files are listed again for their pass when the thumbnailer's version changes
/// (`lifecycle::set_thumbs_version` takes this as its `relist`).
pub(crate) fn relist_thumbs_panics(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    relist_failures(tx, "thumbs", |tx| Ok(tx.execute(failures_relisted!(" AND retry = 0"), ["thumbs"])?))
}

/// A retry of the failed files of one pass, in the transaction `tx`: the pass's next run lists them again (`seq` moves
/// on, `list` makes the listing), and their rows go, so what still fails is recorded anew.
pub(crate) fn relist_failures(
    tx: &rusqlite::Transaction<'_>,
    pass_code: &str,
    list: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<usize>,
) -> Result<()> {
    tx.execute_batch(next_seq!())?;
    // A failure a pass found before this retry is not recorded after it: the pass's buffered ones are refused by the count.
    tx.execute_batch("UPDATE meta SET value = value + 1 WHERE key = 'failure_retries';")?;
    list(tx)?;
    tx.execute("DELETE FROM file_failures WHERE pass = ?1", [pass_code])?;
    Ok(())
}
