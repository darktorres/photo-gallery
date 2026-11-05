//! The folder tree's rows, read from the catalog: the tree itself is `pg_model::tree`'s (its nodes in key order,
//! built by its `TreeBuilder` from the folders' rows).

use crate::Catalog;
use crate::Result;
use pg_model::tree::{FolderTree, TreeBuilder};

/// The present files of each folder, counted from the `files_present` index alone (no sort, no table), and each
/// folder's path.
pub(super) const FOLDER_COUNTS: &str = "SELECT c.folder_id, (SELECT path FROM folders WHERE id = c.folder_id), c.n
    FROM (SELECT folder_id, count(*) AS n FROM files WHERE missing = 0 GROUP BY folder_id) c";

/// What the tree's strings take at most: every folder's row, path bytes and components (separators + 1), from the
/// `folders` table (a few thousand rows, read in a millisecond). The tree has the folders with present files: no
/// more rows, and no more path bytes than their paths (a key adds a byte per component, and lower case rarely
/// lengthens one). The separator is `char(92)`, the byte `\`, which a stored path has as a byte in each component
/// boundary; the replace reads the path as text, which is only a size hint.
pub(super) const FOLDER_SIZES: &str = "SELECT count(*), coalesce(sum(length(path)), 0),
    coalesce(sum(length(path) - length(CAST(replace(CAST(path AS TEXT), char(92), '') AS BLOB))), 0) + count(*)
    FROM folders";

/// The merged tree of every folder holding present files, with their ancestors.
///
/// The tree is built from each folder's display text, so a name with an unpaired surrogate is shown with U+FFFD
/// here. That is the tree's last lossy spot, until it is keyed by the rules' anchors (S-17.4).
pub fn folder_tree(catalog: &Catalog) -> Result<FolderTree> {
    let conn = &catalog.conn;
    let (folders, bytes, components): (usize, usize, usize) =
        conn.query_row(FOLDER_SIZES, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let rules = catalog.rules();
    let mut stmt = conn.prepare(FOLDER_COUNTS)?;
    let mut rows = stmt.query([])?;
    let mut builder = TreeBuilder::with_capacity(folders, bytes, components);
    while let Some(r) = rows.next()? {
        if let Some(stored) = r.get::<_, Option<Vec<u8>>>(1)? {
            let path = rules.parse_bytes(&stored).ok_or_else(|| anyhow::anyhow!("a stored folder path has a NUL"))?;
            builder.add(r.get(0)?, &rules.display(&path), r.get(2)?);
        }
    }
    Ok(builder.build())
}
