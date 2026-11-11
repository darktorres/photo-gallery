//! A file's border sides as detection found them (`border_sides`), and the user's mark that they are not a border
//! (`set_border_mark`, the review). Methods of `Catalog`; the details read the sides in their snapshot
//! (`border_sides_on`).

use rusqlite::Connection;
use rusqlite::types::Type;

use pg_model::detection;
use pg_model::ids::FileId;

use crate::core::Catalog;
use crate::error::Result;

impl Catalog {
    /// The sides detection found in a file, as the catalog keeps them. The keys are read in place. A file with more
    /// than detection finds (`detection::MAX_SIDES`) can only be in a damaged catalog: an error.
    pub fn border_sides(&self, file_id: FileId) -> Result<detection::Sides> {
        border_sides_on(&self.conn, file_id)
    }

    /// Mark a file's borders as not a border, or take the mark back (the review).
    pub fn set_border_mark(&self, file_id: FileId, marked: bool) -> Result<()> {
        if marked {
            self.conn
                .prepare_cached("INSERT OR REPLACE INTO border_marks(file_id, marked) VALUES (?1, unixepoch())")?
                .execute([file_id.get()])?;
        } else {
            self.conn.prepare_cached("DELETE FROM border_marks WHERE file_id = ?1")?.execute([file_id.get()])?;
        }
        Ok(())
    }
}

/// [`Catalog::border_sides`] on `conn`, which may be a snapshot's transaction.
pub(crate) fn border_sides_on(conn: &Connection, file_id: FileId) -> Result<detection::Sides> {
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
