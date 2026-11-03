//! What the viewer's details panel and the border overlay show of one file.

use anyhow::Result;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension};

use crate::detection;
use crate::format::Format;
use crate::media::Kind;
use crate::metadata;

/// What a file's grid entry leaves out, for the file shown (the selection, the viewer's).
#[derive(Clone, PartialEq, Debug)]
pub struct Details {
    pub path: String,
    pub size: i64,
    /// Whether the file can hold its rating too (`false`: kept in the catalog only). Unknown before the import.
    pub rating_in_file: bool,
    /// Every side the border detector found, accepted and rejected (`borders::detect`).
    pub border_sides: detection::Sides,
}

pub fn details(conn: &Connection, file_id: i64) -> Result<Option<Details>> {
    let found = conn
        .prepare_cached(
            "SELECT fo.path || '\\' || f.name, f.size, f.kind, a.format FROM files f
             JOIN folders fo ON fo.id = f.folder_id LEFT JOIN analysis a ON a.file_id = f.id AND a.error IS NULL
             WHERE f.id = ?1",
        )?
        .query_row([file_id], |r| {
            let format: Option<String> = r.get(3)?;
            Ok(Details {
                path: r.get(0)?,
                size: r.get(1)?,
                // Videos keep theirs in the catalog (rewriting a video to add XMP would copy GBs).
                rating_in_file: r.get_ref(2)?.as_str()? == Kind::Image.key()
                    && format.as_deref().and_then(Format::from_name).is_none_or(metadata::can_hold_rating),
                border_sides: detection::Sides::new(),
            })
        })
        .optional()?;
    match found {
        Some(d) => Ok(Some(Details { border_sides: border_sides(conn, file_id)?, ..d })),
        None => Ok(None),
    }
}

/// The sides the border detector found in a file, in the order found.
/// The sides detection found in a file, as the catalog keeps them. The keys are read in place. A file with more
/// than detection finds (`detection::MAX_SIDES`) can only be in a damaged catalog: an error.
pub fn border_sides(conn: &Connection, file_id: i64) -> Result<detection::Sides> {
    let mut stmt = conn.prepare_cached(
        "SELECT pass, edge, kind, depth, color, straight, dev, drift, rejected FROM border_sides WHERE file_id = ?1
         ORDER BY pass, CASE edge WHEN 'top' THEN 0 WHEN 'bottom' THEN 1 WHEN 'left' THEN 2 ELSE 3 END",
    )?;
    let mut rows = stmt.query([file_id])?;
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
            color: color.map(|c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]),
            straight: r.get(5)?,
            dev: r.get(6)?,
            drift: r.get(7)?,
            rejected: match r.get_ref(8)?.as_str_or_null()? {
                Some(k) => Some(detection::Reason::from_key(k).ok_or_else(|| unknown(8, k))?),
                None => None,
            },
        };
        sides
            .try_push(side)
            .map_err(|_| anyhow::anyhow!("file {file_id}: more than {} border sides kept", detection::MAX_SIDES))?;
    }
    Ok(sides)
}
