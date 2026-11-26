//! What the viewer's details panel and the border overlay show of one file.

use crate::core::Catalog;
use crate::error::{Error, Result};
use crate::types::bad_value;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension};

use pg_model::details::Details;
use pg_model::detection;
use pg_model::ids::FileId;

impl Catalog {
    /// A file's details, or `None` when the file is not in the catalog.
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
                    r.get::<_, u64>(2)?,
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
            Some(d) => Ok(Some(Details { border_sides: border_sides_on(&snapshot, file_id)?, ..d })),
            None => Ok(None),
        }
    }
}

/// [`Catalog::border_sides`] on `conn`, which may be a snapshot's transaction. Here, with the details it is read for; the
/// border pass's writes and `Catalog::border_sides` share it.
pub(crate) fn border_sides_on(conn: &Connection, file_id: FileId) -> Result<detection::Sides> {
    let mut stmt = conn.prepare_cached(
        "SELECT pass, edge, kind, depth, color, straight, dev, drift, rejected FROM border_sides WHERE file_id = ?1
         ORDER BY pass, edge",
    )?;
    let mut rows = stmt.query([file_id.get()])?;
    let mut sides = detection::Sides::new();
    // A stored code that names no value: a damaged row, an error.
    let bad = |i: usize, column: &'static str, code: i64| bad_value(i, Type::Integer, column, &code.to_string());
    while let Some(r) = rows.next()? {
        let (edge, kind) = (r.get_ref(1)?.as_i64()?, r.get_ref(2)?.as_i64()?);
        let color: Option<i64> = r.get(4)?;
        let side = detection::Side {
            pass: r.get(0)?,
            // In edge order (top, bottom, left, right): the codes are in that order.
            edge: detection::Edge::from_code(edge).ok_or_else(|| bad(1, "border_sides.edge", edge))?,
            kind: detection::Kind::from_code(kind).ok_or_else(|| bad(2, "border_sides.kind", kind))?,
            depth: r.get(3)?,
            color: color.map(detection::Side::unpacked_color),
            straight: r.get(5)?,
            dev: r.get(6)?,
            drift: r.get(7)?,
            rejected: match r.get_ref(8)?.as_i64_or_null()? {
                Some(k) => Some(detection::Reason::from_code(k).ok_or_else(|| bad(8, "border_sides.rejected", k))?),
                None => None,
            },
        };
        sides.try_push(side).map_err(|_| {
            Error::Invalid(format!("file {}: more than {} border sides kept", file_id.get(), detection::MAX_SIDES))
        })?;
    }
    Ok(sides)
}
