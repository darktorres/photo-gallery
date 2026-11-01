//! What the UI browses (PLAN §7): a folder tree where folders with the same path on different drives are one
//! node ("merged drives"), the files of a node filtered and sorted, ratings and turns (queued for the files),
//! saved views and where the user was.

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::types::{Type, Value};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::borders;
use crate::catalog::Catalog;
use crate::decode::Format;
use crate::media::{Ext, Kind};
use crate::metadata;
use crate::query::{BorderFacet, Facet, MediaType, Query, Sort};

/// A folder's path without its drive: `E:\pinterest\a` and `D:\pinterest\a` are both `pinterest\a`; a drive
/// root is the empty path. Other paths (UNC) are kept whole.
pub fn merged_path(path: &str) -> &str {
    match path.as_bytes() {
        [d, b':', ..] if d.is_ascii_alphabetic() => path[2..].trim_start_matches('\\'),
        _ => path,
    }
}

/// The components of a merged path, as the tree has them: `\\server\share\x` is `server`, `share`, `x`.
fn components(merged: &str) -> impl Iterator<Item = &str> {
    merged.split('\\').filter(|p| !p.is_empty())
}

/// The order key's part for a folder: each component of its merged path in lower case (`str::to_lowercase`, a
/// final Σ as ς), then 0x01. Bytes 0x01 and 0x02 in a name (NTFS allows them, Win32 doesn't) are escaped as
/// 0x02 0x02 and 0x02 0x03, which keeps the order. Compared as bytes, these keys order folders component by
/// component, as the tree does: `p`, `p\sub`, `p-x`; the same folder on every drive is one key.
pub fn folder_key(merged: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(merged.len() + 4);
    for part in components(merged) {
        for b in part.to_lowercase().bytes() {
            match b {
                0x01 => key.extend([0x02, 0x02]),
                0x02 => key.extend([0x02, 0x03]),
                _ => key.push(b),
            }
        }
        key.push(0x01);
    }
    key
}

/// The order key's part for a file name: 0x00, the name in lower case character by character, 0x00, the name
/// itself. Compared as bytes (UTF-8 keeps code-point order): case-insensitive (Unicode, unlike SQLite's NOCASE),
/// then exact, so the order is total. The leading 0x00 puts a folder's files before its subfolders.
pub fn name_key(name: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 * name.len() + 2);
    key.push(0x00);
    let mut buf = [0; 4];
    for c in name.chars().flat_map(char::to_lowercase) {
        key.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    key.push(0x00);
    key.extend_from_slice(name.as_bytes());
    key
}

/// `files.order_key`: where a file sorts by name, all drives merged: its folder's key, then its name's.
pub fn order_key(folder_path: &str, name: &str) -> Vec<u8> {
    let mut key = folder_key(merged_path(folder_path));
    key.extend(name_key(name));
    key
}

/// The files of a merged folder (optionally with everything below it), as a range of order keys: a condition
/// on `f` and the values it binds, which the `files_order` index answers.
fn scope(merged: &str, recursive: bool) -> (String, Vec<Value>) {
    let mut lo = folder_key(merged);
    let hi = if recursive {
        // Below the folder, nothing sorts at or after 0xFF (no UTF-8 byte is 0xFF).
        if lo.is_empty() { None } else { Some([lo.as_slice(), &[0xFF]].concat()) }
    } else {
        let hi = [lo.as_slice(), &[0x01]].concat();
        lo.push(0x00);
        Some(hi)
    };
    let mut values = vec![Value::Blob(lo)];
    let mut sql = "f.order_key >= ?".to_string();
    if let Some(hi) = hi {
        sql.push_str(" AND f.order_key < ?");
        values.push(Value::Blob(hi));
    }
    (sql, values)
}

pub struct FolderNode {
    pub name: String,
    pub merged_path: String,
    pub depth: usize,
    pub children: Vec<usize>,
    /// Present files directly in this folder, and in it plus everything below.
    pub direct: i64,
    pub total: i64,
    /// The drives this folder exists on, e.g. `D: E:`.
    pub drives: String,
}

pub struct FolderTree {
    pub nodes: Vec<FolderNode>,
    pub roots: Vec<usize>,
}

/// The merged tree of every folder holding present files, with their ancestors.
pub fn folder_tree(conn: &Connection) -> Result<FolderTree> {
    // Counted from the `files_present` index alone (no sort, no table), joined to the folders here.
    let counts: Vec<(i64, i64)> = conn
        .prepare("SELECT folder_id, count(*) FROM files WHERE missing = 0 GROUP BY folder_id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let paths: BTreeMap<i64, String> = conn
        .prepare("SELECT id, path FROM folders")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let folders = counts.iter().filter_map(|(id, n)| Some((paths.get(id)?, *n)));

    let mut tree = FolderTree { nodes: Vec::new(), roots: Vec::new() };
    let mut by_key: BTreeMap<String, usize> = BTreeMap::new();
    for (path, count) in folders {
        let drive = if merged_path(path).len() < path.len() { &path[..2] } else { "" };
        let mut parent: Option<usize> = None;
        let mut merged = String::new();
        let parts: Vec<&str> = components(merged_path(path)).collect();
        for (depth, part) in parts.iter().enumerate() {
            if !merged.is_empty() {
                merged.push('\\');
            }
            merged.push_str(part);
            let key = merged.to_lowercase();
            let id = *by_key.entry(key).or_insert_with(|| {
                tree.nodes.push(FolderNode {
                    name: part.to_string(),
                    merged_path: merged.clone(),
                    depth,
                    children: Vec::new(),
                    direct: 0,
                    total: 0,
                    drives: String::new(),
                });
                let id = tree.nodes.len() - 1;
                match parent {
                    Some(p) => tree.nodes[p].children.push(id),
                    None => tree.roots.push(id),
                }
                id
            });
            let node = &mut tree.nodes[id];
            node.total += count;
            if !drive.is_empty() && !node.drives.split(' ').any(|d| d.eq_ignore_ascii_case(drive)) {
                if !node.drives.is_empty() {
                    node.drives.push(' ');
                }
                node.drives.push_str(drive);
            }
            parent = Some(id);
        }
        if let Some(p) = parent {
            tree.nodes[p].direct += count;
        }
    }
    // The grid's order: by `folder_key`, component by component.
    let names: Vec<Vec<u8>> = tree.nodes.iter().map(|n| folder_key(&n.name)).collect();
    tree.roots.sort_by(|&a, &b| names[a].cmp(&names[b]));
    for node in &mut tree.nodes {
        node.children.sort_by(|&a, &b| names[a].cmp(&names[b]));
    }
    Ok(tree)
}

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
        if self.pending_turns % 2 == 1 { (self.height, self.width) } else { (self.width, self.height) }
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
    pub fn side_line(&self, edge: borders::Edge, depth: u32) -> Option<[f64; 4]> {
        let (w, h) = self.stored_size()?;
        let d = f64::from(depth);
        Some(self.shown(match edge {
            borders::Edge::Top => [0.0, d, w, d],
            borders::Edge::Bottom => [0.0, h - d, w, h - d],
            borders::Edge::Left => [d, 0.0, d, h],
            borders::Edge::Right => [w - d, 0.0, w - d, h],
        }))
    }

    /// The stored image's size; the analysis' is upright, its transpose when the orientation turns it. `None`
    /// before the import.
    fn stored_size(&self) -> Option<(f64, f64)> {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        (w > 0.0 && h > 0.0).then_some(if self.orientation >= 5 { (h, w) } else { (w, h) })
    }

    /// A rectangle of the stored image in pixels (left, top, right, bottom) as fractions of the image as shown:
    /// x, y, width, height.
    fn shown(&self, rect: [f64; 4]) -> [f64; 4] {
        let Some((w, h)) = self.stored_size() else { return [0.0; 4] };
        let [l, t, r, b] = rect;
        let [x0, y0, x1, y1] = borders::upright([l / w, t / h, r / w, b / h], self.orientation, self.pending_turns);
        [x0, y0, x1 - x0, y1 - y0]
    }
}

/// What an `Entry` is read from (`entry_from_row`), with `f` the file.
const ENTRY_COLUMNS: &str = "f.id, f.mtime, coalesce(a.width, 0), coalesce(a.height, 0), a.duration_ms, f.ext,
    f.rotation, coalesce(w.value, 0), coalesce(r.rating, 0), coalesce(a.frames, 1), coalesce(a.orientation, 1),
    a.cut_left, a.cut_top, a.cut_right, a.cut_bottom, m.file_id IS NOT NULL";
/// The column after `ENTRY_COLUMNS`: a sort key (`list_grid`).
const SORT_COLUMN: usize = 16;
/// What the entries and the filters read besides the file.
const FILE_JOINS: &str = "LEFT JOIN analysis a ON a.file_id = f.id AND a.error IS NULL
    LEFT JOIN ratings r ON r.file_id = f.id
    LEFT JOIN file_writes w ON w.file_id = f.id AND w.kind = 'rotation'
    LEFT JOIN border_marks m ON m.file_id = f.id";

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
        .prepare_cached(&format!("SELECT {ENTRY_COLUMNS} FROM files f {FILE_JOINS} WHERE f.id = ?1"))?
        .query_row([file_id], entry_from_row)
        .optional()?)
}

/// `list_grid`'s query: the entries in name order, then the sort key if it isn't the name.
fn grid_sql(scope: &str, cond: &str, sort: Sort) -> String {
    let key = sort.key_sql();
    // Name order backwards for "by name, descending"; the other keys take their ties in name order.
    let dir = if key.is_none() && sort.descending { "DESC" } else { "ASC" };
    // `missing = 0` is written out, not bound: it's the `files_order` index's condition.
    format!(
        "SELECT {ENTRY_COLUMNS}{} FROM files f {FILE_JOINS}
         WHERE f.missing = 0 AND {scope} AND {cond}
         ORDER BY f.order_key {dir}, f.id {dir}",
        key.map_or(String::new(), |k| format!(", {k}")),
    )
}

/// The present files of a view: a merged folder (optionally with everything below it), filtered and ordered.
///
/// They are read in name order from the `files_order` index, so SQLite never sorts. Another order is a stable
/// sort here on that key alone, files without it last: files with the same key stay in name order.
pub fn list_grid(conn: &Connection, merged: &str, recursive: bool, query: &Query, sort: Sort) -> Result<Vec<Entry>> {
    let (scope, mut values) = scope(merged, recursive);
    let cond = query.condition();
    values.extend(cond.params);
    let mut stmt = conn.prepare(&grid_sql(&scope, &cond.sql, sort))?;
    if sort.key_sql().is_none() {
        let entries = stmt.query_map(params_from_iter(values), entry_from_row)?;
        return Ok(entries.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    let keyed =
        stmt.query_map(params_from_iter(values), |r| Ok((r.get::<_, Option<i64>>(SORT_COLUMN)?, entry_from_row(r)?)))?;
    let mut keyed = keyed.collect::<rusqlite::Result<Vec<_>>>()?;
    keyed.sort_by(|(a, _), (b, _)| match (a, b) {
        (Some(a), Some(b)) if sort.descending => b.cmp(a),
        (Some(a), Some(b)) => a.cmp(b),
        (a, b) => b.is_some().cmp(&a.is_some()),
    });
    Ok(keyed.into_iter().map(|(_, e)| e).collect())
}

/// What a file's grid entry leaves out, for the file shown (the selection, the viewer's).
#[derive(Clone, PartialEq, Debug)]
pub struct Details {
    pub path: String,
    pub size: i64,
    /// Whether the file can hold its rating too (`false`: kept in the catalog only). Unknown before the import.
    pub rating_in_file: bool,
    /// Every side the border detector found, accepted and rejected (`borders::detect`).
    pub border_sides: Vec<borders::Side>,
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
                border_sides: Vec::new(),
            })
        })
        .optional()?;
    match found {
        Some(d) => Ok(Some(Details { border_sides: border_sides(conn, file_id)?, ..d })),
        None => Ok(None),
    }
}

/// The sides the border detector found in a file, in the order found.
pub fn border_sides(conn: &Connection, file_id: i64) -> Result<Vec<borders::Side>> {
    let mut stmt = conn.prepare_cached(
        "SELECT pass, edge, kind, depth, color, straight, dev, drift, rejected FROM border_sides WHERE file_id = ?1
         ORDER BY pass, CASE edge WHEN 'top' THEN 0 WHEN 'bottom' THEN 1 WHEN 'left' THEN 2 ELSE 3 END",
    )?;
    let rows = stmt.query_map([file_id], |r| {
        let text = |i: usize| -> rusqlite::Result<String> { r.get(i) };
        let unknown = |i: usize, v: String| rusqlite::Error::InvalidColumnType(i, v, Type::Text);
        let edge = text(1)?;
        let kind = text(2)?;
        let rejected: Option<String> = r.get(8)?;
        let color: Option<i64> = r.get(4)?;
        Ok(borders::Side {
            pass: r.get(0)?,
            edge: borders::Edge::from_key(&edge).ok_or_else(|| unknown(1, edge.clone()))?,
            kind: borders::Kind::from_key(&kind).ok_or_else(|| unknown(2, kind.clone()))?,
            depth: r.get(3)?,
            color: color.map(|c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]),
            straight: r.get(5)?,
            dev: r.get(6)?,
            drift: r.get(7)?,
            rejected: match rejected {
                Some(key) => Some(borders::Reason::from_key(&key).ok_or_else(|| unknown(8, key.clone()))?),
                None => None,
            },
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Mark a file's borders as not a border, or take the mark back (the review).
pub fn set_border_mark(conn: &Connection, file_id: i64, marked: bool) -> Result<()> {
    Catalog::set_border_mark(conn, file_id, marked)
}

/// What the filter bar shows next to its choices, for a view's folder.
#[derive(Default, Debug, PartialEq)]
pub struct Facets {
    /// Present files in the folder, unfiltered.
    pub total: i64,
    /// Those the query passes.
    pub matching: i64,
    /// Per type and per extension (most files first): how many files choosing it would show, given the other
    /// facets.
    pub types: Vec<(MediaType, i64)>,
    pub exts: Vec<(String, i64)>,
    /// Per border value (`BorderFacet::ALL`'s order), the same way.
    pub borders: Vec<(BorderFacet, i64)>,
}

pub fn facets(conn: &Connection, merged: &str, recursive: bool, query: &Query) -> Result<Facets> {
    let (scope, scope_values) = scope(merged, recursive);
    let count = |skip: Option<Facet>, select: &str, group: &str| -> Result<Vec<(String, i64)>> {
        let cond = query.condition_without(skip);
        let sql = format!(
            "SELECT {select}, count(*) FROM files f {FILE_JOINS} WHERE f.missing = 0 AND {scope} AND {} {group}",
            cond.sql
        );
        let values = scope_values.iter().cloned().chain(cond.params);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    };
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM files f WHERE f.missing = 0 AND {scope}"),
        params_from_iter(&scope_values),
        |r| r.get(0),
    )?;
    let matching = count(None, "''", "")?.first().map_or(0, |r| r.1);
    let mut types: Vec<(MediaType, i64)> = count(Some(Facet::Types), MediaType::CASE, "GROUP BY 1")?
        .into_iter()
        .filter_map(|(t, n)| Some((MediaType::from_key(&t)?, n)))
        .collect();
    types.sort();
    let exts = count(Some(Facet::Exts), "f.ext", "GROUP BY 1 ORDER BY 2 DESC, 1")?;
    // In one pass: each value's condition summed (a file can have sides of two classes).
    let sums: Vec<String> = BorderFacet::ALL.iter().map(|b| format!("coalesce(sum({}), 0)", b.sql())).collect();
    let cond = query.condition_without(Some(Facet::Borders));
    let sql = format!(
        "SELECT {} FROM files f {FILE_JOINS} WHERE f.missing = 0 AND {scope} AND {}",
        sums.join(", "),
        cond.sql
    );
    let values = scope_values.iter().cloned().chain(cond.params);
    let borders = conn.query_row(&sql, params_from_iter(values), |r| {
        BorderFacet::ALL.iter().enumerate().map(|(i, &b)| Ok((b, r.get(i)?))).collect::<rusqlite::Result<Vec<_>>>()
    })?;
    Ok(Facets { total, matching, types, exts, borders })
}

/// Rate a file: 1–5, or 0 to clear. The catalog changes at once. With `write_back`, the rating is also queued
/// for the file itself (`ingest::writeback`), when its format can hold one and it differs from what the file
/// holds; clearing writes 0 only into a file that holds a rating.
pub fn set_rating(conn: &Connection, file_id: i64, rating: u8, write_back: bool) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let rating = rating.min(5);
    if rating == 0 {
        tx.execute("DELETE FROM ratings WHERE file_id = ?1", [file_id])?;
    } else {
        tx.execute(
            "INSERT INTO ratings(file_id, rating, updated) VALUES (?1, ?2, unixepoch())
             ON CONFLICT(file_id) DO UPDATE SET rating = excluded.rating, updated = excluded.updated",
            params![file_id, rating],
        )?;
    }
    let format: Option<String> = tx
        .query_row("SELECT format FROM analysis WHERE file_id = ?1 AND error IS NULL", [file_id], |r| r.get(0))
        .optional()?
        .flatten();
    let can_hold = format.as_deref().and_then(Format::from_name).is_some_and(metadata::can_hold_rating);
    let in_file: Option<u8> =
        tx.query_row("SELECT value FROM file_ratings WHERE file_id = ?1", [file_id], |r| r.get(0)).optional()?;
    let wanted = if rating > 0 || in_file.is_some_and(|v| v > 0) { Some(rating) } else { in_file };
    if write_back && can_hold && wanted != in_file {
        tx.execute(
            "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rating', ?2, unixepoch())
             ON CONFLICT(file_id, kind) DO UPDATE SET value = excluded.value, queued = excluded.queued,
                attempts = 0, error = NULL",
            params![file_id, wanted],
        )?;
    } else {
        // Back to what the file holds (or not written back): nothing left to write. A write already being
        // copied in finishes, and then this value follows it.
        tx.execute("DELETE FROM file_writes WHERE file_id = ?1 AND kind = 'rating' AND temp IS NULL", [file_id])?;
        tx.execute(
            "UPDATE file_writes SET value = ?2 WHERE file_id = ?1 AND kind = 'rating'",
            params![file_id, wanted.unwrap_or(0)],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Turn a file `turns` quarter turns clockwise: queued for `ingest::writeback`, which turns the file itself
/// losslessly where it can (`rotate::rotate`) and keeps the turn in the catalog otherwise. Turns add up until
/// written.
pub fn queue_rotation(conn: &Connection, file_id: i64, turns: u8) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rotation', ?2 % 4, unixepoch())
         ON CONFLICT(file_id, kind) DO UPDATE SET value = (value + excluded.value) % 4, queued = excluded.queued,
            attempts = 0, error = NULL",
        params![file_id, turns],
    )?;
    tx.execute(
        "DELETE FROM file_writes WHERE file_id = ?1 AND kind = 'rotation' AND value = 0 AND temp IS NULL",
        [file_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// What the image providers need to find a file.
pub struct FileSource {
    pub path: String,
    /// With `rotation`, the thumbnail cache's validity key.
    pub mtime: i64,
    pub rotation: u8,
    pub thumbhash: Option<Vec<u8>>,
    /// For a video, the time of the frame that stands for it (`video::poster_ms`; 0 before the import).
    pub poster_ms: Option<i64>,
}

pub fn file_source(conn: &Connection, file_id: i64) -> Result<Option<FileSource>> {
    Ok(conn
        .prepare_cached(
            "SELECT fo.path || '\\' || f.name, f.mtime, f.rotation, a.thumbhash, f.kind, a.poster_ms FROM files f
             JOIN folders fo ON fo.id = f.folder_id LEFT JOIN analysis a ON a.file_id = f.id
             WHERE f.id = ?1",
        )?
        .query_row([file_id], |r| {
            let video = r.get::<_, String>(4)? == Kind::Video.key();
            Ok(FileSource {
                path: r.get(0)?,
                mtime: r.get(1)?,
                rotation: r.get(2)?,
                thumbhash: r.get(3)?,
                poster_ms: if video { Some(r.get::<_, Option<i64>>(5)?.unwrap_or(0)) } else { None },
            })
        })
        .optional()?)
}

/// A saved view (ImageRater's bookmarks): where, which files, in what order.
#[derive(Clone, PartialEq, Debug)]
pub struct View {
    pub name: String,
    /// Merged path; `""` for all folders.
    pub folder: String,
    pub recursive: bool,
    pub query: Query,
    pub sort: Sort,
}

/// The saved views, by name as the grid orders file names (`name_key`).
pub fn views(conn: &Connection) -> Result<Vec<View>> {
    let mut stmt = conn.prepare("SELECT name, folder, recursive, query, sort FROM views")?;
    let rows = stmt.query_map([], |r| {
        Ok(View {
            name: r.get(0)?,
            folder: r.get(1)?,
            recursive: r.get(2)?,
            query: Query::from_json(&r.get::<_, String>(3)?),
            sort: Sort::from_json(&r.get::<_, String>(4)?),
        })
    })?;
    let mut views: Vec<View> = rows.collect::<rusqlite::Result<_>>()?;
    views.sort_by_cached_key(|v| name_key(&v.name));
    Ok(views)
}

/// Save a view, replacing one of the same name.
pub fn save_view(conn: &Connection, v: &View) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO views(name, folder, recursive, query, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![v.name, v.folder, v.recursive, v.query.to_json(), v.sort.to_json()],
    )?;
    Ok(())
}

pub fn delete_view(conn: &Connection, name: &str) -> Result<()> {
    conn.execute("DELETE FROM views WHERE name = ?1", [name])?;
    Ok(())
}

/// Where the user was (`session` table): read at start, written as they move.
pub fn session_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT value FROM session WHERE key = ?1", [key], |r| r.get(0)).optional()?)
}

pub fn session_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO session(key, value) VALUES (?1, ?2)", [key, value])?;
    Ok(())
}

/// Several session keys, in one transaction.
pub fn session_set_many(conn: &Connection, values: &[(&str, &str)]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for (key, value) in values {
        session_set(&tx, key, value)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::fixture;
    use crate::query::{DateRange, Orientation, Range, SortKey, Status};

    #[test]
    fn the_cut_is_described_as_shown() {
        // Stored 736 × 1593 with bars of 163 at the top and 131 at the bottom, shown turned a quarter clockwise
        // (orientation 6: upright 1593 × 736): the top bar is on the right.
        let mut e = Entry {
            id: 1,
            mtime: 0,
            width: 1593,
            height: 736,
            duration_ms: Entry::NO_DURATION,
            ext: Ext::of("jpg").unwrap(),
            rotation: 0,
            pending_turns: 0,
            rating: 0,
            animated: false,
            orientation: 6,
            cut: [0, 163, 0, 131],
            borders: Borders::Found,
        };
        assert_eq!(e.shown_cut(), Some([131, 0, 163, 0]));
        // A quarter turn more, not yet written: upside down from stored, the top bar at the bottom.
        e.pending_turns = 1;
        assert_eq!(e.shown_cut(), Some([0, 131, 0, 163]));
        e.borders = Borders::None;
        assert_eq!(e.shown_cut(), None);
    }

    #[test]
    fn merged_paths() {
        assert_eq!(merged_path(r"E:\pinterest\a"), r"pinterest\a");
        assert_eq!(merged_path("D:"), "");
        assert_eq!(merged_path(r"\\server\share\x"), r"\\server\share\x");
    }

    /// Files on two drives under the same merged folder, plus a subfolder: (id, folder, name, size, mtime,
    /// format, rating).
    fn library() -> crate::catalog::Catalog {
        let c = crate::catalog::Catalog::open(std::path::Path::new(":memory:")).unwrap();
        c.conn
            .execute_batch(
                "INSERT INTO scans(id, root, started) VALUES (1, 'D:', 0);
                 INSERT INTO folders(id, path) VALUES (1, 'D:\\p'), (2, 'E:\\p'), (3, 'D:\\p\\sub');",
            )
            .unwrap();
        for (id, folder, name, size, mtime, format, rating) in [
            (1, 1, "b.jpg", 30, 3, "jpeg", 4),
            (2, 2, "a.png", 10, 9, "png", 0),
            (3, 1, "Ä.jpg", 20, 1, "jpeg", 2),
            (4, 3, "c.bmp", 40, 5, "bmp", 0),
        ] {
            fixture::insert(&c.conn, &[fixture::File::new(id, folder, name, size, mtime)]);
            c.conn
                .execute(
                    "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, format, width, height)
                     VALUES (?1, 1, ?2, ?3, 0, 'image', 0, ?4, 4, 3)",
                    params![id, size, mtime, format],
                )
                .unwrap();
            set_rating(&c.conn, id, rating, false).unwrap();
        }
        c
    }

    fn ids(c: &crate::catalog::Catalog, folder: &str, recursive: bool, query: &Query, sort: Sort) -> Vec<i64> {
        list_grid(&c.conn, folder, recursive, query, sort).unwrap().iter().map(|e| e.id).collect()
    }

    fn by(key: SortKey, descending: bool) -> Sort {
        Sort { key, descending }
    }

    #[test]
    fn ratings_and_orders() {
        let c = library();
        let all = Query::default();
        let ratings = |r: &[u8]| Query { ratings: r.to_vec(), ..Query::default() };
        // Name: both drives' "p" as one folder, by name (Unicode case-insensitive: Ä after b), then "p\sub".
        assert_eq!(ids(&c, "p", true, &all, Sort::default()), [2, 1, 3, 4]);
        assert_eq!(ids(&c, "p", false, &all, Sort::default()), [2, 1, 3]);
        assert_eq!(ids(&c, "p", true, &all, by(SortKey::Name, true)), [4, 3, 1, 2]);
        assert_eq!(ids(&c, "", true, &all, by(SortKey::Size, true)), [4, 1, 3, 2]);
        assert_eq!(ids(&c, "", true, &all, by(SortKey::Modified, true)), [2, 4, 1, 3]);
        assert_eq!(ids(&c, "", true, &ratings(&[0]), by(SortKey::Size, true)), [4, 2]);
        assert_eq!(ids(&c, "", true, &ratings(&[1, 2, 3, 4, 5]), Sort::default()), [1, 3]);
        assert_eq!(ids(&c, "", true, &ratings(&[2]), Sort::default()), [3]);
        assert_eq!(ids(&c, "", true, &ratings(&[0, 2]), Sort::default()), [2, 3, 4]);
        let bmp = details(&c.conn, 4).unwrap().unwrap();
        assert!(!bmp.rating_in_file, "BMP keeps its rating in the catalog");
        assert!(details(&c.conn, 1).unwrap().unwrap().rating_in_file);
        assert_eq!(
            details(&c.conn, 2).unwrap(),
            Some(Details { path: r"E:\p\a.png".into(), size: 10, rating_in_file: true, border_sides: Vec::new() })
        );
        assert_eq!(details(&c.conn, 99).unwrap(), None);
    }

    #[test]
    fn folders_in_the_trees_order() {
        let c = library();
        // "p-x" sorts before "p\sub" as a whole path ('-' < '\'), but after it in the tree (p's children first).
        c.conn.execute("INSERT INTO folders(id, path) VALUES (4, 'D:\\p-x')", []).unwrap();
        fixture::insert(&c.conn, &[fixture::File::new(9, 4, "z.jpg", 1, 1)]);
        assert_eq!(ids(&c, "", true, &Query::default(), Sort::default()), [2, 1, 3, 4, 9]);
        let tree = folder_tree(&c.conn).unwrap();
        let roots: Vec<&str> = tree.roots.iter().map(|&n| tree.nodes[n].name.as_str()).collect();
        assert_eq!(roots, ["p", "p-x"]);
    }

    #[test]
    fn name_order() {
        use std::cmp::Ordering::*;
        let cmp = |a: &str, b: &str| name_key(a).cmp(&name_key(b));
        // Case-insensitively first (Unicode, unlike SQLite's NOCASE), then exactly, so the order is total.
        assert_eq!(cmp("a.jpg", "B.jpg"), Less);
        assert_eq!(cmp("Ä.jpg", "b.jpg"), Greater);
        assert_eq!(cmp("ä.jpg", "Ä.JPG"), Greater);
        assert_eq!(cmp("B.jpg", "b.jpg"), Less);
        assert_eq!(cmp("img10", "IMG10"), Greater);
        assert_eq!(cmp("abc", "ABC"), Greater);
        assert_eq!(cmp("ab", "abc"), Less);
        assert_eq!(cmp("same", "same"), Equal);
    }

    /// The order before the key (kept as the reference): folders by their lowercased merged path component by
    /// component (the scope's sort), then names by the PATH collation (characters lowercased one by one, then
    /// bytes). Empty components are dropped, as the tree did.
    fn reference_order(a: (&str, &str), b: (&str, &str)) -> std::cmp::Ordering {
        let folder = |p: &str| -> Vec<String> {
            merged_path(p).to_lowercase().split('\\').filter(|c| !c.is_empty()).map(String::from).collect()
        };
        let collation = |a: &str, b: &str| {
            a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase)).then_with(|| a.cmp(b))
        };
        folder(a.0).cmp(&folder(b.0)).then_with(|| collation(a.1, b.1))
    }

    /// A small xorshift: the same cases on every run, without a dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
            from[(self.next() % from.len() as u64) as usize]
        }
    }

    /// Paths and names made of the pieces where lowercasing and byte order are tricky.
    fn random_file(rng: &mut Rng) -> (String, String) {
        const PIECES: &[&str] = &[
            "a", "A", "b", "B", "z", "ä", "Ä", "ß", "İ", "i", "\u{212A}", "k", "K", "Σ", "σ", "ς", "e\u{301}", "é",
            "-", ".", " ", "1", "10", "\u{1}", "\u{2}", "\u{3}", "ΑΣ", "ας", "日本", "😀",
        ];
        let part = |rng: &mut Rng| -> String { (0..1 + rng.next() % 3).map(|_| rng.pick(PIECES)).collect() };
        let drive = rng.pick(&["D:", "E:", "d:", r"\\srv\share"]);
        let mut folder = drive.to_string();
        for _ in 0..rng.next() % 4 {
            folder.push('\\');
            folder.push_str(&part(rng));
        }
        (folder, part(rng))
    }

    #[test]
    fn order_keys_sort_as_before() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let files: Vec<(String, String)> = (0..400).map(|_| random_file(&mut rng)).collect();
        for a in &files {
            for b in &files {
                let key = order_key(&a.0, &a.1).cmp(&order_key(&b.0, &b.1));
                assert_eq!(key, reference_order((&a.0, &a.1), (&b.0, &b.1)), "{a:?} against {b:?}");
            }
        }
        // The cases named.
        let before = |a: (&str, &str), b: (&str, &str)| assert!(order_key(a.0, a.1) < order_key(b.0, b.1));
        before((r"D:\p", "z.jpg"), (r"D:\p\sub", "a.jpg"));
        before((r"D:\p\sub", "z.jpg"), (r"D:\p-x", "a.jpg"));
        before((r"E:\P", "a.jpg"), (r"D:\p", "b.jpg"));
        before((r"D:\ας", "a"), (r"D:\ασ", "a"));
        before((r"D:\a\b", "z"), ("D:\\a\\b\u{1}c", "a"));
        // A final Σ in a folder name lowercases to ς, as the whole path did.
        assert_eq!(order_key(r"D:\ΑΣ", "a"), order_key(r"E:\ας", "a"));
    }

    /// On a real catalog's names (read-only, any layout): `PG_TEST_CATALOG=… cargo test -p pg-core real_names --
    /// --ignored`.
    #[test]
    #[ignore = "needs PG_TEST_CATALOG, a real catalog (opened read-only)"]
    fn real_names_sort_and_scope_as_before() {
        let path = std::env::var_os("PG_TEST_CATALOG").expect("PG_TEST_CATALOG");
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let files: Vec<(String, String)> = conn
            .prepare("SELECT fo.path, f.name FROM files f JOIN folders fo ON fo.id = f.folder_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let mut by_key = files.clone();
        by_key.sort_by_cached_key(|(folder, name)| order_key(folder, name));
        for pair in by_key.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            assert_ne!(reference_order((&a.0, &a.1), (&b.0, &b.1)), std::cmp::Ordering::Greater, "{a:?}, {b:?}");
        }
        // Every folder against every scope, recursive and not: the old match (`in_scope`) and the key range.
        let folders: Vec<String> = conn
            .prepare("SELECT path FROM folders")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let in_range = |scope_of: (&str, bool), folder: &str| {
            let (_, values) = scope(scope_of.0, scope_of.1);
            let key = order_key(folder, "a");
            let blob = |v: &Value| match v {
                Value::Blob(b) => b.clone(),
                _ => unreachable!(),
            };
            key >= blob(&values[0]) && values.get(1).is_none_or(|hi| key < blob(hi))
        };
        for scope_folder in &folders {
            let merged = merged_path(scope_folder).to_lowercase();
            for folder in &folders {
                let m = merged_path(folder).to_lowercase();
                for recursive in [false, true] {
                    let before = m == merged
                        || (recursive
                            && (merged.is_empty()
                                || m.strip_prefix(&merged).is_some_and(|rest| rest.starts_with('\\'))));
                    assert_eq!(in_range((&merged, recursive), folder), before, "{folder} in {merged} ({recursive})");
                }
            }
        }
        eprintln!("{} files, {} folders: same order and scopes", files.len(), folders.len());
    }

    #[test]
    fn scopes_are_key_ranges() {
        let inside = |scope_of: (&str, bool), file: (&str, &str)| {
            let (sql, values) = scope(scope_of.0, scope_of.1);
            let key = order_key(file.0, file.1);
            let bound = |v: &Value| match v {
                Value::Blob(b) => b.clone(),
                _ => unreachable!(),
            };
            let lo = bound(&values[0]);
            let hi = values.get(1).map(bound);
            assert_eq!(sql.matches('?').count(), values.len());
            key >= lo && hi.is_none_or(|hi| key < hi)
        };
        // Recursive: the folder and below it, on every drive, in any case; not `p-x`.
        assert!(inside(("p", true), (r"D:\p", "a")));
        assert!(inside(("p", true), (r"E:\P\sub", "a")));
        assert!(!inside(("p", true), (r"D:\p-x", "a")));
        assert!(!inside(("p", true), (r"D:\pa", "a")));
        // Not recursive: the folder's own files only.
        assert!(inside(("p", false), (r"D:\p", "a")));
        assert!(!inside(("p", false), (r"D:\p\sub", "a")));
        // The root: everything, or the drives' own files.
        assert!(inside(("", true), (r"D:\p\sub", "a")));
        assert!(inside(("", false), ("D:", "a")));
        assert!(!inside(("", false), (r"D:\p", "a")));
        // A UNC folder, named as the tree names it.
        assert!(inside((r"srv\share\x", false), (r"\\srv\share\x", "a")));
        // A control byte in a folder name is a different folder.
        assert!(!inside((r"a\b", true), ("D:\\a\\b\u{1}c", "a")));
    }

    #[test]
    fn lists_read_the_order_index() {
        let c = library();
        for (folder, recursive) in [("p", true), ("p", false), ("", true)] {
            for sort in [Sort::default(), by(SortKey::Name, true), by(SortKey::Taken, true)] {
                let (scope, values) = scope(folder, recursive);
                let unrated = Query { ratings: vec![0], ..Query::default() }.condition();
                let plan: Vec<String> = c
                    .conn
                    .prepare(&format!("EXPLAIN QUERY PLAN {}", grid_sql(&scope, &unrated.sql, sort)))
                    .unwrap()
                    .query_map(params_from_iter(values.into_iter().chain(unrated.params)), |r| r.get(3))
                    .unwrap()
                    .collect::<rusqlite::Result<_>>()
                    .unwrap();
                let plan = plan.join("\n");
                assert!(plan.contains("USING INDEX files_order"), "{plan}");
                assert!(!plan.contains("TEMP B-TREE"), "sorted: {plan}");
            }
        }
        let plan: Vec<String> = c
            .conn
            .prepare("EXPLAIN QUERY PLAN SELECT folder_id, count(*) FROM files WHERE missing = 0 GROUP BY folder_id")
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let plan = plan.join("\n");
        assert!(plan.contains("COVERING INDEX files_present") && !plan.contains("TEMP B-TREE"), "{plan}");
    }

    /// `library()` plus a video, an animated GIF, a file the import couldn't read and one not imported yet, with
    /// duplicates: 1 and 3 are the same bytes, 2–4 a close tier-1 pair, 2–6 a distant one.
    fn mixed_library() -> crate::catalog::Catalog {
        let c = library();
        use fixture::File;
        fixture::insert(
            &c.conn,
            &[
                File { kind: "video", ..File::new(5, 3, "Clip.MP4", 50, 7) },
                File::new(6, 1, "anim.gif", 15, 2),
                File::new(7, 2, "broken.jpg", 5, 4),
                File::new(8, 2, "new.webp", 6, 6),
            ],
        );
        c.conn
            .execute_batch(
                "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, format, width, height,
                                      frames, duration_ms) VALUES
                    (5, 1, 50, 7, 0, 'video', 0, NULL, 1920, 1080, 300, 12000),
                    (6, 1, 15, 2, 0, 'image', 0, 'gif', 3, 3, 10, NULL);
                 INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error)
                     VALUES (7, 1, 5, 4, 0, 'image', 0, 'bad');
                 UPDATE analysis SET file_hash = x'01' WHERE file_id IN (1, 3);
                 UPDATE analysis SET taken = 1714557600 WHERE file_id = 1; -- 2024-05-01 10:00
                 INSERT INTO pdq_pairs(a, b, distance, variant) VALUES (2, 4, 10, 0), (2, 6, 30, 0);",
            )
            .unwrap();
        c
    }

    #[test]
    fn facets_filter() {
        let c = mixed_library();
        let q = |f: &dyn Fn(&mut Query)| {
            let mut q = Query::default();
            f(&mut q);
            ids(&c, "", true, &q, Sort::default())
        };
        assert_eq!(q(&|_| {}), [2, 6, 1, 7, 8, 3, 4, 5]);
        assert_eq!(q(&|q| q.types = vec![MediaType::Video]), [5]);
        assert_eq!(q(&|q| q.types = vec![MediaType::Animated, MediaType::Video]), [6, 5]);
        assert_eq!(q(&|q| q.types = vec![MediaType::Image]), [2, 1, 7, 8, 3, 4]);
        assert_eq!(q(&|q| q.exts = vec!["JPG".into()]), [1, 7, 3]);
        assert_eq!(q(&|q| q.orientations = vec![Orientation::Square]), [6]);
        assert_eq!(q(&|q| q.orientations = vec![Orientation::Landscape]), [2, 1, 3, 4, 5]);
        assert!(q(&|q| q.orientations = vec![Orientation::Portrait]).is_empty());
        assert_eq!(q(&|q| q.megapixels.min = Some(5e-6)), [2, 6, 1, 3, 4, 5]);
        assert_eq!(q(&|q| q.megapixels.max = Some(1e-5)), [6]);
        assert_eq!(
            q(&|q| q.size_mb = Range { min: Some(20.0 / 1_048_576.0), max: Some(40.0 / 1_048_576.0) }),
            [1, 3, 4]
        );
        assert_eq!(q(&|q| q.duration_s.min = Some(10.0)), [5]);
        assert_eq!(q(&|q| q.duplicates = Some(true)), [2, 1, 3, 4]);
        assert_eq!(q(&|q| q.duplicates = Some(false)), [6, 5]);
        assert_eq!(q(&|q| q.status = vec![Status::Failed]), [7]);
        assert_eq!(q(&|q| q.status = vec![Status::Pending]), [8]);
        assert_eq!(q(&|q| q.status = vec![Status::Imported]), [2, 6, 1, 3, 4, 5]);
        let day = |from: &str, to: &str| DateRange { from: Some(from.into()), to: Some(to.into()) };
        assert_eq!(q(&|q| q.taken = day("2024-05-01", "2024-05-01")), [1]);
        assert!(q(&|q| q.taken.from = Some("2024-05-02".into())).is_empty());
        // A malformed date doesn't filter.
        assert_eq!(q(&|q| q.taken.from = Some("2024-13-45".into())).len(), 8);
        // The test files' mtimes are FILETIMEs a few hundred ns after 1601.
        assert_eq!(q(&|q| q.modified = day("1600-12-30", "1601-01-02")).len(), 8);
        assert!(q(&|q| q.modified.from = Some("2000-01-01".into())).is_empty());
        assert_eq!(q(&|q| q.name = "ä".into()), [3]);
        assert_eq!(q(&|q| q.name = "clip".into()), [5]);
        // Facets combine with AND.
        assert_eq!(
            q(&|q| {
                q.exts = vec!["jpg".into()];
                q.duplicates = Some(true)
            }),
            [1, 3]
        );
    }

    #[test]
    fn sorts_put_missing_values_last() {
        let c = mixed_library();
        let all = Query::default();
        assert_eq!(ids(&c, "", true, &all, by(SortKey::Megapixels, true)), [5, 2, 1, 3, 4, 6, 7, 8]);
        assert_eq!(ids(&c, "", true, &all, by(SortKey::Megapixels, false)), [6, 2, 1, 3, 4, 5, 7, 8]);
        assert_eq!(ids(&c, "", true, &all, by(SortKey::Duration, true)), [5, 2, 6, 1, 7, 8, 3, 4]);
        assert_eq!(ids(&c, "", true, &all, by(SortKey::Taken, true))[0], 1);
    }

    /// The order SQLite gave when it sorted (before the sort moved here): the key, files without it last, then
    /// name order ascending.
    fn sorted_by_sqlite(c: &crate::catalog::Catalog, sort: Sort) -> Vec<i64> {
        let order = match sort.key_sql() {
            None if sort.descending => "f.order_key DESC, f.id DESC".to_string(),
            None => "f.order_key, f.id".to_string(),
            Some(k) => format!("{k} IS NULL, {k} {}, f.order_key, f.id", if sort.descending { "DESC" } else { "ASC" }),
        };
        let sql = format!("SELECT f.id FROM files f {FILE_JOINS} WHERE f.missing = 0 ORDER BY {order}");
        c.conn.prepare(&sql).unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    #[test]
    fn grid_lists_sort_as_sqlite_did() {
        let c = mixed_library();
        // Ties: four 4×3 images, and no taken date or duration on most.
        for key in
            [SortKey::Name, SortKey::Size, SortKey::Modified, SortKey::Taken, SortKey::Megapixels, SortKey::Duration]
        {
            for descending in [false, true] {
                let sort = by(key, descending);
                assert_eq!(ids(&c, "", true, &Query::default(), sort), sorted_by_sqlite(&c, sort), "{sort:?}");
            }
        }
        // Each entry as read alone.
        for e in list_grid(&c.conn, "", true, &Query::default(), Sort::default()).unwrap() {
            assert_eq!(entry(&c.conn, e.id).unwrap(), Some(e));
        }
        let entries = list_grid(&c.conn, "", true, &Query::default(), Sort::default()).unwrap();
        let get = |id: i64| *entries.iter().find(|e| e.id == id).unwrap();
        let (clip, gif, broken) = (get(5), get(6), get(7));
        assert_eq!((clip.media_type(), clip.duration_ms(), clip.ext.name()), (MediaType::Video, Some(12000), "mp4"));
        assert_eq!((gif.media_type(), gif.duration_ms(), gif.ext.name()), (MediaType::Animated, None, "gif"));
        // Not imported (its analysis failed): no size yet, square.
        assert_eq!((broken.shown_size(), broken.aspect()), ((0, 0), 1.0));
        assert_eq!(entry(&c.conn, 99).unwrap(), None);
    }

    #[test]
    fn facet_counts() {
        let c = mixed_library();
        let videos = Query { types: vec![MediaType::Video], ..Query::default() };
        let f = facets(&c.conn, "", true, &videos).unwrap();
        assert_eq!((f.total, f.matching), (8, 1));
        // A facet's own choices are counted without it; the other facets apply.
        assert_eq!(f.types, [(MediaType::Image, 6), (MediaType::Animated, 1), (MediaType::Video, 1)]);
        assert_eq!(f.exts, [("mp4".to_string(), 1)]);
        let f = facets(&c.conn, "p", false, &Query::default()).unwrap();
        assert_eq!(
            f.exts,
            [("jpg".to_string(), 3), ("gif".to_string(), 1), ("png".to_string(), 1), ("webp".to_string(), 1)]
        );
    }

    #[test]
    fn views_keep_their_query() {
        let c = library();
        let v = View {
            name: "Unrated videos".into(),
            folder: "p".into(),
            recursive: true,
            query: Query { types: vec![MediaType::Video], ratings: vec![0], ..Query::default() },
            sort: by(SortKey::Size, true),
        };
        save_view(&c.conn, &v).unwrap();
        assert_eq!(views(&c.conn).unwrap(), [v]);
    }

    fn queued(c: &crate::catalog::Catalog, id: i64, kind: &str) -> Option<i64> {
        c.conn
            .query_row("SELECT value FROM file_writes WHERE file_id = ?1 AND kind = ?2", params![id, kind], |r| {
                r.get(0)
            })
            .optional()
            .unwrap()
    }

    #[test]
    fn ratings_and_turns_queue_for_the_file() {
        let c = library();
        set_rating(&c.conn, 2, 3, true).unwrap();
        assert_eq!(queued(&c, 2, "rating"), Some(3));
        // Back to what the file holds (nothing): nothing to write.
        set_rating(&c.conn, 2, 0, true).unwrap();
        assert_eq!(queued(&c, 2, "rating"), None);
        // A file that holds 4: clearing writes 0.
        crate::catalog::Catalog::set_file_rating(&c.conn, 1, Some(4)).unwrap();
        set_rating(&c.conn, 1, 0, true).unwrap();
        assert_eq!(queued(&c, 1, "rating"), Some(0));
        // BMP can't hold one; write-back off writes nothing.
        set_rating(&c.conn, 4, 5, true).unwrap();
        set_rating(&c.conn, 3, 5, false).unwrap();
        assert_eq!((queued(&c, 4, "rating"), queued(&c, 3, "rating")), (None, None));
        // Turns add up modulo 4; a full turn leaves nothing to do.
        queue_rotation(&c.conn, 1, 1).unwrap();
        queue_rotation(&c.conn, 1, 1).unwrap();
        assert_eq!(queued(&c, 1, "rotation"), Some(2));
        queue_rotation(&c.conn, 1, 2).unwrap();
        assert_eq!(queued(&c, 1, "rotation"), None);
        // The grid shows a pending turn, aspect included.
        queue_rotation(&c.conn, 1, 3).unwrap();
        let entries = list_grid(&c.conn, "", true, &Query::default(), Sort::default()).unwrap();
        let e = entries.iter().find(|e| e.id == 1).unwrap();
        assert_eq!(e.pending_turns, 3);
        assert_eq!((e.shown_size(), e.aspect()), ((3, 4), 3.0 / 4.0));
        assert_eq!(entry(&c.conn, 1).unwrap().map(|e| e.pending_turns), Some(3));
    }
}
