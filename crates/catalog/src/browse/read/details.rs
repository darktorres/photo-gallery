//! What the viewer's details panel and the border overlay show of one file.

use crate::Catalog;
use crate::error::Result;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension};

use pg_model::details::Details;
use pg_model::detection;
use pg_model::ids::FileId;

pub fn details(catalog: &Catalog, file_id: FileId) -> Result<Option<Details>> {
    // One read snapshot: the file's row and its border sides are one moment's (a re-analysis may write meanwhile).
    let snapshot = catalog.conn.unchecked_transaction()?;
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
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })
        .optional()?;
    let found = match row {
        Some((folder, name, size, kind, format, failed)) => {
            Some(Details {
                path: catalog.file_path_of(&folder, &name)?,
                size,
                // The rule the rating's write-back follows (`writes::set_rating`): videos keep theirs in the catalog
                // (rewriting a video to add XMP would copy GBs), and so does a file the import couldn't read.
                rating_in_file: !failed && crate::writes::rating_in_file_possible(&kind, format.as_deref()),
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

/// The sides detection found in a file, as the catalog keeps them. The keys are read in place. A file with more
/// than detection finds (`detection::MAX_SIDES`) can only be in a damaged catalog: an error.
pub fn border_sides(catalog: &Catalog, file_id: FileId) -> Result<detection::Sides> {
    border_sides_on(&catalog.conn, file_id)
}

/// [`border_sides`] on `conn`, which may be a snapshot's transaction.
fn border_sides_on(conn: &Connection, file_id: FileId) -> Result<detection::Sides> {
    let mut stmt = conn.prepare_cached(
        "SELECT pass, edge, kind, depth, color, straight, dev, drift, rejected FROM border_sides WHERE file_id = ?1
         ORDER BY pass, CASE edge WHEN 'top' THEN 0 WHEN 'bottom' THEN 1 WHEN 'left' THEN 2 ELSE 3 END",
    )?;
    let mut rows = stmt.query([file_id.get()])?;
    let mut sides = detection::Sides::new();
    while let Some(r) = rows.next()? {
        let unknown = |i: usize, v: &str| rusqlite::Error::InvalidColumnType(i, v.to_string(), Type::Text);
        let (edge, kind) = (r.get_ref(1)?.as_str()?, r.get_ref(2)?.as_str()?);
        let color: Option<i64> = r.get(4)?;
        let side = detection::Side {
            pass: r.get(0)?,
            edge: detection::Edge::from_key(edge).ok_or_else(|| unknown(1, edge))?,
            kind: detection::Kind::from_key(kind).ok_or_else(|| unknown(2, kind))?,
            depth: r.get(3)?,
            color: color.map(|c| {
                let [.., r, g, b] = c.to_be_bytes();
                [r, g, b]
            }),
            straight: r.get(5)?,
            dev: r.get(6)?,
            drift: r.get(7)?,
            rejected: match r.get_ref(8)?.as_str_or_null()? {
                Some(k) => Some(detection::Reason::from_key(k).ok_or_else(|| unknown(8, k))?),
                None => None,
            },
        };
        sides.try_push(side).map_err(|_| {
            anyhow::anyhow!("file {}: more than {} border sides kept", file_id.get(), detection::MAX_SIDES)
        })?;
    }
    Ok(sides)
}
