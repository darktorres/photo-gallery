//! The folder tree's rows, read from the catalog: the tree itself is `pg_model::tree`'s (its nodes in key order,
//! built by its `TreeBuilder` from the folders' rows).

use crate::Catalog;
use crate::Result;
use pg_model::tree::{FolderTree, TreeBuilder};

/// The present files of each folder, counted from the `files_present` index alone (no sort, no table), and each
/// folder's path.
pub(super) const FOLDER_COUNTS: &str = "SELECT c.folder_id, (SELECT path FROM folders WHERE id = c.folder_id), c.n
    FROM (SELECT folder_id, count(*) AS n FROM files WHERE missing = 0 GROUP BY folder_id) c";

/// What the tree's strings take at most: every folder's row, path bytes and components (backslashes + 1), from the
/// `folders` table (a few thousand rows, read in a millisecond). The tree has the folders with present files: no
/// more rows, and no more path bytes than their paths (a key adds a byte per component, and lower case rarely
/// lengthens one). `char(92)` is `\`.
pub(super) const FOLDER_SIZES: &str = "SELECT count(*), coalesce(sum(length(CAST(path AS BLOB))), 0),
    coalesce(sum(length(path) - length(replace(path, char(92), '')) + 1), 0) FROM folders";

/// The merged tree of every folder holding present files, with their ancestors.
pub fn folder_tree(catalog: &Catalog) -> Result<FolderTree> {
    let conn = &catalog.conn;
    let (folders, bytes, components): (usize, usize, usize) =
        conn.query_row(FOLDER_SIZES, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let mut stmt = conn.prepare(FOLDER_COUNTS)?;
    let mut rows = stmt.query([])?;
    let mut builder = TreeBuilder::with_capacity(folders, bytes, components);
    while let Some(r) = rows.next()? {
        if let Some(path) = r.get_ref(1)?.as_str_or_null()? {
            builder.add(r.get(0)?, path, r.get(2)?);
        }
    }
    Ok(builder.build())
}
