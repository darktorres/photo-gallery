//! The folder tree's rows, read from the catalog: the tree itself is `pg_model::tree`'s (its nodes in key order,
//! built by its `TreeBuilder` from the folders' rows).

use crate::Catalog;
use crate::error::Result;
use pg_model::tree::{FolderTree, TreeBuilder};

/// The present files of each folder, counted from the `files_present` index alone (no sort, no table), and each
/// folder's path.
pub(super) const FOLDER_COUNTS: &str = "SELECT c.folder_id, (SELECT path FROM folders WHERE id = c.folder_id), c.n
    FROM (SELECT folder_id, count(*) AS n FROM files WHERE missing = 0 GROUP BY folder_id) c";

/// What the tree's strings take at most: every folder's row, path bytes and components (separators + 1), from the
/// `folders` table (a few thousand rows, read in a millisecond). The tree has the folders with present files: no
/// more rows, and no more path bytes than their paths (a key adds a byte per component, and lower case rarely
/// lengthens one). The separator is bound (`?1`, the rules' separator as text): a stored path has it as a byte at
/// each component boundary; the replace reads the path as text, which is only a size hint.
pub(super) const FOLDER_SIZES: &str = "SELECT count(*), coalesce(sum(length(path)), 0),
    coalesce(sum(length(path) - length(CAST(replace(CAST(path AS TEXT), ?1, '') AS BLOB))), 0) + count(*)
    FROM folders";

impl Catalog {
    /// The merged tree of every folder holding present files, with their ancestors, built under the catalog's rules.
    pub fn folder_tree(&self) -> Result<FolderTree> {
        let conn = &self.conn;
        let rules = self.rules();
        let (folders, bytes, components): (usize, usize, usize) =
            conn.query_row(FOLDER_SIZES, [rules.sep().to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let mut stmt = conn.prepare(FOLDER_COUNTS)?;
        let mut rows = stmt.query([])?;
        let mut builder = TreeBuilder::with_capacity(rules, folders, bytes, components);
        while let Some(r) = rows.next()? {
            if let Some(stored) = r.get::<_, Option<Vec<u8>>>(1)? {
                let path = rules.parse_bytes(&stored).ok_or_else(|| anyhow::anyhow!("a stored folder path has a NUL"))?;
                builder.add(r.get(0)?, &path, r.get(2)?);
            }
        }
        Ok(builder.build())
    }
}