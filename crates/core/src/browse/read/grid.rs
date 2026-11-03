//! The grid's entries: what a list of files shows of each (`Entry`, 56 bytes), a single file's entry, and a
//! folder's file list, filtered and sorted.

use anyhow::Result;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, params_from_iter};

use super::scope;
use crate::display;
use crate::query::{MediaType, Query, Sort};
use pg_model::detection;
use pg_model::media::{Ext, Kind};

/// A file as the grid holds it: what laying it out, drawing its thumbnail and badge, and naming its image take.
/// The rest is read when the file is shown (`details`). Small and without a `String`, so a list of 190,000 is
/// 10.6 MB and quick to build.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Entry {
    pub id: i64,
    /// With `rotation`, what makes an image URL current.
    pub mtime: i64,
    /// As analyzed, without `pending_turns`; 0 until the file is imported.
    pub width: u32,
    pub height: u32,
    /// Videos, once imported; `Entry::NO_DURATION` otherwise.
    pub duration_ms: u32,
    pub ext: Ext,
    /// Catalog rotation (quarter turns clockwise).
    pub rotation: u8,
    /// Quarter turns clockwise asked for but not yet written (`queue_rotation`): the grid shows them already.
    pub pending_turns: u8,
    pub rating: u8,
    /// More than one frame (known after the import).
    pub animated: bool,
    /// Orientation (1–8) of the analysis: what turns the stored pixels upright, the catalog rotation included.
    pub orientation: u8,
    /// Borders found: stored pixels cut from the left, top, right and bottom (`analysis.cut_*`), all 0 for none.
    pub cut: [u32; 4],
    pub borders: Borders,
}

const _: () = assert!(size_of::<Entry>() == 56, "Entry's size, in its doc");

/// Whether a file's borders were looked for, and what came of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Borders {
    /// Not looked for: not imported yet, a video, an animated or a tiny image.
    Unknown,
    None,
    Found,
    /// Found, and marked as not a border (the review).
    Marked,
}

impl Entry {
    pub const NO_DURATION: u32 = u32::MAX;

    pub fn kind(&self) -> Kind {
        self.ext.kind()
    }

    pub fn media_type(&self) -> MediaType {
        match self.kind() {
            Kind::Video => MediaType::Video,
            Kind::Image if self.animated => MediaType::Animated,
            Kind::Image => MediaType::Image,
        }
    }

    pub fn duration_ms(&self) -> Option<u32> {
        (self.duration_ms != Entry::NO_DURATION).then_some(self.duration_ms)
    }

    /// Width and height as shown, pending turns included; 0 before the import.
    pub fn shown_size(&self) -> (u32, u32) {
        if pg_model::orientation::turns_swap(self.pending_turns) {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }

    /// Width ÷ height as shown; 1 before the import.
    pub fn aspect(&self) -> f64 {
        let (w, h) = self.shown_size();
        if w > 0 && h > 0 { w as f64 / h as f64 } else { 1.0 }
    }

    /// What is left without the borders, as fractions of the image as shown (pending turns included): x, y, width,
    /// height. `None` without borders found (marked ones included).
    pub fn content(&self) -> Option<[f64; 4]> {
        if !matches!(self.borders, Borders::Found | Borders::Marked) {
            return None;
        }
        let [l, t, r, b] = self.cut.map(f64::from);
        let (w, h) = self.stored_size()?;
        Some(self.shown([l, t, w - r, h - b]))
    }

    /// What cropping the borders cuts from the left, top, right and bottom of the image as shown, in pixels. `None`
    /// without borders found (marked ones included).
    pub fn shown_cut(&self) -> Option<[u32; 4]> {
        let [x, y, w, h] = self.content()?;
        let (sw, sh) = self.shown_size();
        let px = |f: f64, side: u32| (f * f64::from(side)).round() as u32;
        Some([px(x, sw), px(y, sh), px(1.0 - x - w, sw), px(1.0 - y - h, sh)])
    }

    /// Where a border detector's side would cut, `depth` stored pixels from its edge of the stored image, as a line
    /// across the image as shown: x, y, width, height in fractions, one of them 0.
    pub fn side_line(&self, edge: detection::Edge, depth: u32) -> Option<[f64; 4]> {
        let (w, h) = self.stored_size()?;
        let d = f64::from(depth);
        Some(self.shown(match edge {
            detection::Edge::Top => [0.0, d, w, d],
            detection::Edge::Bottom => [0.0, h - d, w, h - d],
            detection::Edge::Left => [d, 0.0, d, h],
            detection::Edge::Right => [w - d, 0.0, w - d, h],
        }))
    }

    /// The stored image's size; the analysis' is upright, its transpose when the orientation turns it. `None`
    /// before the import.
    fn stored_size(&self) -> Option<(f64, f64)> {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        (w > 0.0 && h > 0.0).then_some(if pg_model::orientation::swaps_axes(self.orientation) {
            (h, w)
        } else {
            (w, h)
        })
    }

    /// A rectangle of the stored image in pixels (left, top, right, bottom) as fractions of the image as shown:
    /// x, y, width, height.
    fn shown(&self, rect: [f64; 4]) -> [f64; 4] {
        let Some((w, h)) = self.stored_size() else { return [0.0; 4] };
        let [l, t, r, b] = rect;
        let [x0, y0, x1, y1] = display::upright([l / w, t / h, r / w, b / h], self.orientation, self.pending_turns);
        [x0, y0, x1 - x0, y1 - y0]
    }
}

/// What an `Entry` is read from (`entry_from_row`), with `f` the file. Macros for the statements that are constants,
/// constants for those made at run time.
macro_rules! entry_columns {
    () => {
        "f.id, f.mtime, coalesce(a.width, 0), coalesce(a.height, 0), a.duration_ms, f.ext,
    f.rotation, coalesce(w.value, 0), coalesce(r.rating, 0), coalesce(a.frames, 1), coalesce(a.orientation, 1),
    a.cut_left, a.cut_top, a.cut_right, a.cut_bottom, m.file_id IS NOT NULL"
    };
}

const ENTRY_COLUMNS: &str = entry_columns!();

/// The column after `ENTRY_COLUMNS`: a sort key (`list_grid`).
const SORT_COLUMN: usize = 16;

/// What the entries and the filters read besides the file.
macro_rules! file_joins {
    () => {
        "LEFT JOIN analysis a ON a.file_id = f.id AND a.error IS NULL
    LEFT JOIN ratings r ON r.file_id = f.id
    LEFT JOIN file_writes w ON w.file_id = f.id AND w.kind = 'rotation'
    LEFT JOIN border_marks m ON m.file_id = f.id"
    };
}

pub(crate) const FILE_JOINS: &str = file_joins!();

fn entry_from_row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    let ext = r.get_ref(5)?.as_str()?;
    let sides: [Option<u32>; 4] = [r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?];
    let cut = sides.map(Option::unwrap_or_default);
    let cut_found = sides[0].map(|_| cut.iter().any(|&c| c > 0));
    Ok(Entry {
        id: r.get(0)?,
        mtime: r.get(1)?,
        width: r.get(2)?,
        height: r.get(3)?,
        duration_ms: r
            .get::<_, Option<i64>>(4)?
            .map_or(Entry::NO_DURATION, |ms| ms.clamp(0, u32::MAX as i64 - 1) as u32),
        // The scan records only files with these extensions.
        ext: Ext::of(ext).ok_or_else(|| rusqlite::Error::InvalidColumnType(5, format!("ext {ext}"), Type::Text))?,
        rotation: r.get(6)?,
        pending_turns: r.get(7)?,
        rating: r.get(8)?,
        animated: r.get::<_, i64>(9)? > 1,
        orientation: r.get(10)?,
        cut,
        borders: match cut_found {
            None => Borders::Unknown,
            Some(false) => Borders::None,
            Some(true) if r.get(15)? => Borders::Marked,
            Some(true) => Borders::Found,
        },
    })
}

/// Read an entry again (after its file was rated or turned), keeping its place in the list. `None` if it is gone.
pub fn entry(conn: &Connection, file_id: i64) -> Result<Option<Entry>> {
    Ok(conn
        .prepare_cached(concat!("SELECT ", entry_columns!(), " FROM files f ", file_joins!(), " WHERE f.id = ?1"))?
        .query_row([file_id], entry_from_row)
        .optional()?)
}

/// `list_grid`'s query: the entries in name order, then the sort key if it isn't the name.
pub(super) fn grid_sql(scope: &str, cond: &str, sort: Sort) -> String {
    let key = sort.key_sql();
    // Name order backwards for "by name, descending"; the other keys take their ties in name order.
    let dir = if key.is_none() && sort.descending { "DESC" } else { "ASC" };
    // `missing = 0` is written out, not bound: it's the `files_order` index's condition.
    let (comma, key) = key.map_or(("", ""), |k| (", ", k));
    format!(
        "SELECT {ENTRY_COLUMNS}{comma}{key} FROM files f {FILE_JOINS}
         WHERE f.missing = 0 AND {scope} AND {cond}
         ORDER BY f.order_key {dir}, f.id {dir}"
    )
}

/// The present files of a view: a merged folder (optionally with everything below it), filtered and ordered.
///
/// They are read in name order from the `files_order` index, so SQLite never sorts. Another order is a stable
/// sort here on that key alone, files without it last: files with the same key stay in name order.
///
/// The list is sized by a count of the scope's files, read from the `files_order` index alone in the same read
/// transaction (the background job writes meanwhile): exact without filters, an upper bound with them.
pub fn list_grid(conn: &Connection, merged: &str, recursive: bool, query: &Query, sort: Sort) -> Result<Vec<Entry>> {
    let (scope, mut values) = scope(merged, recursive);
    let tx = conn.unchecked_transaction()?;
    let bound: usize = tx.query_row(
        &format!("SELECT count(*) FROM files f WHERE f.missing = 0 AND {scope}"),
        params_from_iter(&values),
        |r| r.get(0),
    )?;
    let cond = query.condition();
    values.extend(cond.params);
    let mut stmt = tx.prepare(&grid_sql(&scope, &cond.sql, sort))?;
    let mut rows = stmt.query(params_from_iter(values))?;
    let entries = if sort.key_sql().is_none() {
        let mut entries = Vec::with_capacity(bound);
        while let Some(r) = rows.next()? {
            entries.push(entry_from_row(r)?);
        }
        // What the filters left out.
        if entries.len() < entries.capacity() / 4 * 3 {
            entries.shrink_to_fit();
        }
        entries
    } else {
        let mut keyed = Vec::with_capacity(bound);
        while let Some(r) = rows.next()? {
            keyed.push((r.get::<_, Option<i64>>(SORT_COLUMN)?, entry_from_row(r)?));
        }
        keyed.sort_by(|(a, _), (b, _)| match (a, b) {
            (Some(a), Some(b)) if sort.descending => b.cmp(a),
            (Some(a), Some(b)) => a.cmp(b),
            (a, b) => b.is_some().cmp(&a.is_some()),
        });
        // Collected in place, into the keyed list's allocation, which is a fifth larger than the entries need.
        let mut entries: Vec<Entry> = keyed.into_iter().map(|(_, e)| e).collect();
        entries.shrink_to_fit();
        entries
    };
    drop(rows);
    drop(stmt);
    tx.commit()?;
    Ok(entries)
}
