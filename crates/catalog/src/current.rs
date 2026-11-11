//! Whether a file is still as a pass listed it: the one rule the passes' writers and `Catalog::is_current` keep
//! what was read from a file by. A leaf, below `core` and the writers.

use rusqlite::{Statement, params};

use pg_model::ids::FileId;

use crate::error::Result;
use crate::types::Listed;

/// Whether file `?1` is still as listed (`?2` its size, `?3` its mtime, `?4` its rotation), with no write of the app's
/// into it in progress: the one rule for keeping what a pass read from a file (`is_current`).
pub(crate) const IS_CURRENT: &str = concat!(
    "SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1 AND size = ?2 AND mtime = ?3 AND rotation = ?4) AND ",
    not_being_written!()
);

/// [`IS_CURRENT`], prepared as `stmt`, for file `id` as `listed`.
pub(crate) fn query_current(stmt: &mut Statement<'_>, id: FileId, listed: &Listed) -> Result<bool> {
    Ok(stmt.query_row(params![id.get(), listed.size, listed.mtime, listed.rotation.get()], |r| r.get(0))?)
}
