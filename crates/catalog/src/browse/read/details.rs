//! What the viewer's details panel and the border overlay show of one file.

use crate::core::Catalog;
use crate::error::Result;
use rusqlite::OptionalExtension;

use pg_model::details::Details;
use pg_model::detection;
use pg_model::ids::FileId;

impl Catalog {
    pub fn details(&self, file_id: FileId) -> Result<Option<Details>> {
        // One read snapshot: the file's row and its border sides are one moment's (a re-analysis may write meanwhile).
        let snapshot = self.conn.unchecked_transaction()?;
        let row = snapshot
            .prepare_cached(
                "SELECT fo.path, f.name, f.size, f.kind, a.format, a.error IS NOT NULL FROM files f
                 JOIN folders fo ON fo.id = f.folder_id LEFT JOIN analysis a ON a.file_id = f.id
                 WHERE f.id = ?1",
            )?
            .query_row([file_id.get()], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, i64>(2)?,
                    crate::types::kind_at(r, 3)?,
                    crate::types::format_at(r, 4)?,
                    r.get::<_, bool>(5)?,
                ))
            })
            .optional()?;
        let found = match row {
            Some((folder, name, size, kind, format, failed)) => {
                Some(Details {
                    path: self.file_path_of(&folder, &name)?,
                    size,
                    // The rule the rating's write-back follows (`Catalog::set_rating`): videos keep theirs in the catalog
                    // (rewriting a video to add XMP would copy GBs), and so does a file the import couldn't read.
                    rating_in_file: !failed && pg_model::format::rating_in_file_possible(kind, format),
                    border_sides: detection::Sides::new(),
                })
            }
            None => None,
        };
        match found {
            Some(d) => Ok(Some(Details { border_sides: crate::borders::border_sides_on(&snapshot, file_id)?, ..d })),
            None => Ok(None),
        }
    }
}
