//! Whether a file is still as a pass listed it: the one rule the passes' writers and `Catalog::is_current` keep
//! what was read from a file by. A leaf, below `core` and the writers.

use rusqlite::{Statement, params};

use pg_model::ids::FileId;

use crate::error::Result;
use crate::types::Listed;

/// Whether file `?1` is still as listed (`?2` its size, `?3` its mtime, `?4` its rotation), not hidden (a file being
/// removed from the library is not written), with no write of the app's into it in progress: the one rule for keeping
/// what a pass read from a file (`is_current`).
pub(crate) const IS_CURRENT: &str = concat!(
    "SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1 AND size = ?2 AND mtime = ?3 AND turns = ?4 AND missing = 0) AND ",
    not_being_written!()
);

/// The same rule over a batch of files (`Catalog::currents`): the files `IN rarray(?1)` that have no write in progress,
/// with their listed values (`id, size, mtime, rotation`). A file that is absent from the result is not current; the
/// caller compares the values, which are what [`IS_CURRENT`] compares in SQL.
pub(crate) const CURRENT_ROWS: &str = concat!(
    "SELECT id, size, mtime, turns FROM files WHERE id IN rarray(?1) AND missing = 0 AND ",
    not_being_written!(files)
);

/// The files one [`CURRENT_ROWS`] read checks, at most.
pub(crate) const CURRENT_BATCH: usize = 200;

/// [`IS_CURRENT`], prepared as `stmt`, for file `id` as `listed`.
pub(crate) fn query_current(stmt: &mut Statement<'_>, id: FileId, listed: &Listed) -> Result<bool> {
    Ok(stmt.query_row(params![id.get(), listed.size, listed.mtime, listed.turns.get()], |r| r.get(0))?)
}
