//! The read side of browsing (PLAN §7): the folder tree (merged across drives), the files of a node filtered and
//! sorted, what the viewer and the thumbnails need of one file, and the views and the session as they were saved.
//! Nothing here writes; the commands are in [`super::writes`].

use anyhow::Result;
use rusqlite::types::{Type, Value, ValueRef};
use rusqlite::{Connection, OptionalExtension, params_from_iter};

use crate::analysis::ThumbHash;
use crate::detection;
use crate::display;
use crate::format::Format;
use crate::media::{Ext, Kind};
use crate::metadata;
use crate::path::{components, folder_key, merged_path, name_key, push_folder_key};
use crate::query::{BORDER_SUMS, BorderFacet, Facet, MediaType, Query, Sort};

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

/// A folder of the merged tree. Its strings are in its [`FolderTree`].
#[derive(Clone, Debug)]
pub struct FolderNode {
    /// Its merged path, `tree.paths[path_start..path_end]`, whose last component (its name) starts at `name_start`.
    path_start: u32,
    path_end: u32,
    name_start: u32,
    /// Its `folder_key`, `tree.keys[key_start..key_end]`.
    key_start: u32,
    key_end: u32,
    /// `u32::MAX` for a top-level folder.
    parent: u32,
    /// The node after its last descendant: its subtree is `n..end`.
    end: u32,
    /// The folder ids its spelling comes from: the smallest of its subtree (the first that reached it).
    first: i64,
    /// One bit per drive letter it is on (A is bit 0).
    drives: u32,
    pub depth: u32,
    /// Present files directly in this folder, and in it plus everything below.
    pub direct: i64,
    pub total: i64,
}

/// The merged tree of every folder holding present files, with their ancestors: nodes in pre-order, a node's
/// children in `folder_key` order (the grid's), so the nodes are also in key order. Merged paths and keys are ranges
/// of two strings, which makes the whole tree a handful of allocations.
#[derive(Clone, Debug, Default)]
pub struct FolderTree {
    nodes: Vec<FolderNode>,
    /// Each folder's merged path, its components joined by `\`, one after the other.
    paths: String,
    /// Each folder's `folder_key`, one after the other.
    keys: Vec<u8>,
}

/// The present files of each folder, counted from the `files_present` index alone (no sort, no table), and each
/// folder's path.
const FOLDER_COUNTS: &str = "SELECT c.folder_id, (SELECT path FROM folders WHERE id = c.folder_id), c.n
    FROM (SELECT folder_id, count(*) AS n FROM files WHERE missing = 0 GROUP BY folder_id) c";

/// What the tree's strings take at most: every folder's row, path bytes and components (backslashes + 1), from the
/// `folders` table (a few thousand rows, read in a millisecond). The tree has the folders with present files: no
/// more rows, and no more path bytes than their paths (a key adds a byte per component, and lower case rarely
/// lengthens one). `char(92)` is `\`.
const FOLDER_SIZES: &str = "SELECT count(*), coalesce(sum(length(CAST(path AS BLOB))), 0),
    coalesce(sum(length(path) - length(replace(path, char(92), '')) + 1), 0) FROM folders";

/// The merged tree of every folder holding present files, with their ancestors.
pub fn folder_tree(conn: &Connection) -> Result<FolderTree> {
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

/// The folders a tree is made from: each one's merged path and key written once, into the tree's strings.
#[derive(Default)]
struct TreeBuilder {
    tree: FolderTree,
    rows: Vec<TreeRow>,
}

struct TreeRow {
    id: i64,
    count: i64,
    drive: u32,
    /// Its merged path's range in `paths`, and its key's in `keys`.
    path: (u32, u32),
    key: (u32, u32),
}

impl TreeBuilder {
    /// Room for `rows` folders whose paths are `bytes` long with `components` components in all.
    fn with_capacity(rows: usize, bytes: usize, components: usize) -> TreeBuilder {
        let tree = FolderTree {
            nodes: Vec::new(),
            paths: String::with_capacity(bytes),
            keys: Vec::with_capacity(bytes + components),
        };
        TreeBuilder { tree, rows: Vec::with_capacity(rows) }
    }

    /// A folder (`folders.id`, `folders.path`) holding `count` present files. A drive root (no components) is in no
    /// node.
    fn add(&mut self, id: i64, path: &str, count: i64) {
        let merged = merged_path(path);
        let drive = match path.as_bytes() {
            [d, b':', ..] if merged.len() < path.len() => 1 << (d.to_ascii_uppercase() - b'A'),
            _ => 0,
        };
        let FolderTree { paths, keys, .. } = &mut self.tree;
        let path_start = paths.len() as u32;
        for (i, part) in components(merged).enumerate() {
            if i > 0 {
                paths.push('\\');
            }
            paths.push_str(part);
        }
        if paths.len() as u32 == path_start {
            return;
        }
        let key_start = keys.len() as u32;
        push_folder_key(keys, merged);
        let row =
            TreeRow { id, count, drive, path: (path_start, paths.len() as u32), key: (key_start, keys.len() as u32) };
        self.rows.push(row);
    }

    /// The tree: the rows sorted by key once, then the nodes made in that order with a stack of the open ones. A
    /// parent's key is a prefix of its descendants', and a component's end (0x01) sorts below every other byte, so
    /// sorted keys are the tree's pre-order with children in key order: `p`, `p\sub`, `p-x`.
    fn build(self) -> FolderTree {
        let TreeBuilder { mut tree, mut rows } = self;
        let key = |k: (u32, u32)| k.0 as usize..k.1 as usize;
        rows.sort_unstable_by(|a, b| tree.keys[key(a.key)].cmp(&tree.keys[key(b.key)]));
        // The nodes, counted: in this order each row adds the components past those it shares with the row before
        // it (whole components, each ending with 0x01, which none holds), as the stack below finds them open.
        let components = |k: &[u8]| k.iter().filter(|&&b| b == 0x01).count();
        let mut before: &[u8] = &[];
        let mut nodes = 0;
        for row in &rows {
            let k = &tree.keys[key(row.key)];
            let shared = k.iter().zip(before).take_while(|(a, b)| a == b).count();
            nodes += components(k) - components(&k[..shared]);
            before = k;
        }
        tree.nodes.reserve_exact(nodes);
        let mut open: Vec<usize> = Vec::new();
        for row in &rows {
            let row_key = &tree.keys[key(row.key)];
            // Close the nodes the row isn't under.
            while let Some(&top) = open.last() {
                let n = &tree.nodes[top];
                if row_key.starts_with(&tree.keys[n.key_start as usize..n.key_end as usize]) {
                    break;
                }
                tree.nodes[top].end = tree.nodes.len() as u32;
                open.pop();
            }
            // The row's components, with where each ends in its path and its key.
            let path = &tree.paths[row.path.0 as usize..row.path.1 as usize];
            let mut path_end = row.path.0;
            let mut key_end = row.key.0;
            let mut key_rest = row_key;
            for (depth, part) in path.split('\\').enumerate() {
                let name_start = path_end + u32::from(depth > 0);
                path_end = name_start + part.len() as u32;
                let part_key = key_rest.iter().position(|&b| b == 0x01).map_or(key_rest.len(), |i| i + 1);
                key_end += part_key as u32;
                key_rest = &key_rest[part_key..];
                let n = match open.get(depth) {
                    Some(&n) => n,
                    None => {
                        tree.nodes.push(FolderNode {
                            path_start: row.path.0,
                            path_end,
                            name_start,
                            key_start: row.key.0,
                            key_end,
                            parent: open.last().map_or(u32::MAX, |&p| p as u32),
                            end: 0,
                            first: row.id,
                            drives: 0,
                            depth: depth as u32,
                            direct: 0,
                            total: 0,
                        });
                        open.push(tree.nodes.len() - 1);
                        tree.nodes.len() - 1
                    }
                };
                let node = &mut tree.nodes[n];
                node.total += row.count;
                node.drives |= row.drive;
                // Spelled as the first folder that reached it (the old tree's order: by folder id).
                if row.id < node.first {
                    (node.first, node.path_start, node.path_end, node.name_start) =
                        (row.id, row.path.0, path_end, name_start);
                    (node.key_start, node.key_end) = (row.key.0, key_end);
                }
            }
            if let Some(&n) = open.last() {
                tree.nodes[n].direct += row.count;
            }
        }
        for n in open {
            tree.nodes[n].end = tree.nodes.len() as u32;
        }
        tree
    }
}

impl FolderTree {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node(&self, n: usize) -> &FolderNode {
        &self.nodes[n]
    }

    /// Its merged path, as one of its drives spells it.
    pub fn merged_path(&self, n: usize) -> &str {
        let node = &self.nodes[n];
        &self.paths[node.path_start as usize..node.path_end as usize]
    }

    /// Its path's last component.
    pub fn name(&self, n: usize) -> &str {
        let node = &self.nodes[n];
        &self.paths[node.name_start as usize..node.path_end as usize]
    }

    /// Its `folder_key`.
    pub fn key(&self, n: usize) -> &[u8] {
        let node = &self.nodes[n];
        &self.keys[node.key_start as usize..node.key_end as usize]
    }

    pub fn parent(&self, n: usize) -> Option<usize> {
        let p = self.nodes[n].parent;
        (p != u32::MAX).then_some(p as usize)
    }

    /// The node after its last descendant.
    pub fn subtree_end(&self, n: usize) -> usize {
        self.nodes[n].end as usize
    }

    pub fn has_children(&self, n: usize) -> bool {
        self.subtree_end(n) > n + 1
    }

    /// Its children (`None`: the top-level folders), in order.
    pub fn children(&self, n: Option<usize>) -> impl Iterator<Item = usize> + '_ {
        let (mut next, end) = n.map_or((0, self.nodes.len()), |n| (n + 1, self.subtree_end(n)));
        std::iter::from_fn(move || {
            let child = (next < end).then_some(next)?;
            next = self.subtree_end(child);
            Some(child)
        })
    }

    /// The drives it is on, by letter: `D: E:`.
    pub fn drives(&self, n: usize) -> String {
        let bits = self.nodes[n].drives;
        let mut text = String::new();
        for letter in (0..26).filter(|i| bits & (1 << i) != 0) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push(char::from(b'A' + letter as u8));
            text.push(':');
        }
        text
    }

    /// Whether it is on more than one drive.
    pub fn merged(&self, n: usize) -> bool {
        self.nodes[n].drives.count_ones() > 1
    }

    /// Present files in every folder of the tree.
    pub fn total(&self) -> i64 {
        self.children(None).map(|n| self.nodes[n].total).sum()
    }

    /// The node of a merged path, in any case (`key` holds its key meanwhile).
    pub fn find(&self, merged: &str, key: &mut Vec<u8>) -> Option<usize> {
        key.clear();
        push_folder_key(key, merged);
        self.find_key(key)
    }

    /// The node whose `folder_key` is `key`.
    pub fn find_key(&self, key: &[u8]) -> Option<usize> {
        self.nodes.binary_search_by(|n| self.keys[n.key_start as usize..n.key_end as usize].cmp(key)).ok()
    }
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
        if crate::orientation::turns_swap(self.pending_turns) {
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
        (w > 0.0 && h > 0.0).then_some(if crate::orientation::swaps_axes(self.orientation) { (h, w) } else { (w, h) })
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
fn grid_sql(scope: &str, cond: &str, sort: Sort) -> String {
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
    let cond = query.condition_without(Some(Facet::Borders));
    let sql =
        format!("SELECT {BORDER_SUMS} FROM files f {FILE_JOINS} WHERE f.missing = 0 AND {scope} AND {}", cond.sql);
    let values = scope_values.iter().cloned().chain(cond.params);
    let borders = conn.query_row(&sql, params_from_iter(values), |r| {
        BorderFacet::ALL.iter().enumerate().map(|(i, &b)| Ok((b, r.get(i)?))).collect::<rusqlite::Result<Vec<_>>>()
    })?;
    Ok(Facets { total, matching, types, exts, borders })
}

/// What a file's images are made from, as the catalog has it now. What is made from the file (a thumbnail, a
/// decode, an analysis) is kept under the state it was read in, checked before and after the read
/// ([`FileState::writing`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileState {
    /// With `rotation`, what the thumbnail cache keeps a file's thumbnails under (`thumbs::Source`).
    pub mtime: i64,
    pub rotation: u8,
    /// Quarter turns clockwise asked for and not yet written (`queue_rotation`): what is made from the file is shown
    /// turned by them.
    pub turns: u8,
    /// A write of the app's into the file is in progress (`file_writes.temp`, until the transaction that records
    /// it): the file may be part-written, or hold a write the catalog doesn't show yet.
    pub writing: bool,
}

/// [`FileState`]'s columns, `f` the file.
macro_rules! file_state_columns {
    () => {
        "f.mtime, f.rotation,
        coalesce((SELECT value FROM file_writes WHERE file_id = f.id AND kind = 'rotation'), 0),
        EXISTS(SELECT 1 FROM file_writes WHERE file_id = f.id AND temp IS NOT NULL)"
    };
}

fn file_state_at(r: &rusqlite::Row, first: usize) -> rusqlite::Result<FileState> {
    Ok(FileState {
        mtime: r.get(first)?,
        rotation: r.get(first + 1)?,
        turns: r.get(first + 2)?,
        writing: r.get(first + 3)?,
    })
}

/// The file's state now (`None`: no such file).
pub fn file_state(conn: &Connection, file_id: i64) -> Result<Option<FileState>> {
    Ok(conn
        .prepare_cached(concat!("SELECT ", file_state_columns!(), " FROM files f WHERE f.id = ?1"))?
        .query_row([file_id], |r| file_state_at(r, 0))
        .optional()?)
}

/// What the image providers need to make a file's images.
#[derive(Clone, PartialEq, Debug)]
pub struct FileSource {
    pub path: String,
    pub state: FileState,
    /// For a video, the time of the frame that stands for it (`video::poster_ms`; 0 before the import).
    pub poster_ms: Option<i64>,
}

pub fn file_source(conn: &Connection, file_id: i64) -> Result<Option<FileSource>> {
    Ok(conn
        .prepare_cached(concat!(
            "SELECT fo.path || '\\' || f.name, f.kind, a.poster_ms, ",
            file_state_columns!(),
            " FROM files f JOIN folders fo ON fo.id = f.folder_id LEFT JOIN analysis a ON a.file_id = f.id
             WHERE f.id = ?1"
        ))?
        .query_row([file_id], |r| {
            let video = r.get_ref(1)?.as_str()? == Kind::Video.key();
            Ok(FileSource {
                path: r.get(0)?,
                state: file_state_at(r, 3)?,
                poster_ms: if video { Some(r.get::<_, Option<i64>>(2)?.unwrap_or(0)) } else { None },
            })
        })
        .optional()?)
}

/// A file's ThumbHash placeholder, and the quarter turns clockwise that show it as the file is shown now: the
/// turns asked for and not yet written, and those kept in the catalog since the analysis that made the hash.
pub struct Placeholder {
    /// `None` before the import.
    pub hash: Option<ThumbHash>,
    pub turns: u8,
}

pub fn placeholder(conn: &Connection, file_id: i64) -> Result<Option<Placeholder>> {
    Ok(conn
        .prepare_cached(concat!(
            "SELECT a.thumbhash, (f.rotation - coalesce(a.rotation, f.rotation) + 4) % 4, ",
            file_state_columns!(),
            " FROM files f LEFT JOIN analysis a ON a.file_id = f.id WHERE f.id = ?1"
        ))?
        .query_row([file_id], |r| {
            let kept: u8 = r.get(1)?;
            // Read in place: the blob isn't copied into a `Vec` first.
            let hash = match r.get_ref(0)? {
                ValueRef::Null => None,
                v => ThumbHash::new(v.as_blob()?),
            };
            Ok(Placeholder { hash, turns: (kept + file_state_at(r, 2)?.turns) % 4 })
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

/// Where the user was (`session` table): read at start, written as they move.
pub fn session_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT value FROM session WHERE key = ?1", [key], |r| r.get(0)).optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::THUMBHASH_MAX;
    use crate::browse::fixtures::{by, ids, library, mixed_library, sorted_by_sqlite};
    use crate::browse::writes::queue_rotation;
    use crate::path::{order_key, order_key_of};

    /// No image gives a longer ThumbHash than `THUMBHASH_MAX`, and some give one that long: every aspect, with alpha
    /// and without.
    #[test]
    fn thumbhashes_fit_inline() {
        let mut longest = [0, 0];
        for k in 1..=100 {
            for (w, h) in [(100, k), (k, 100)] {
                for alpha in [false, true] {
                    let rgba: Vec<u8> = (0..w * h)
                        .flat_map(|i| {
                            let (x, y) = (i % w, i / w);
                            let a = if alpha { ((x * 7 + y * 3) % 256) as u8 } else { 255 };
                            [(x * 255 / w) as u8, (y * 255 / h) as u8, ((x ^ y) * 9 % 256) as u8, a]
                        })
                        .collect();
                    let hash = thumbhash::rgba_to_thumb_hash(w, h, &rgba);
                    assert!(hash.len() <= THUMBHASH_MAX, "{w} × {h}, alpha {alpha}: {}", hash.len());
                    let kept = ThumbHash::new(&hash).unwrap();
                    assert_eq!(kept.as_slice(), &hash[..]);
                    longest[alpha as usize] = longest[alpha as usize].max(hash.len());
                }
            }
        }
        assert_eq!(longest, [THUMBHASH_MAX - 1, THUMBHASH_MAX]);
        assert!(ThumbHash::new(&[0; THUMBHASH_MAX + 1]).is_none());
    }
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

    #[test]
    fn a_file_is_shown_turned_by_what_the_catalog_has_not_yet_put_in_its_pixels() {
        let c = library();
        c.conn.execute("UPDATE analysis SET thumbhash = x'01' WHERE file_id = 1", []).unwrap();
        let state = || file_state(&c.conn, 1).unwrap().unwrap();
        let turns = || placeholder(&c.conn, 1).unwrap().unwrap().turns;
        assert_eq!(state(), FileState { mtime: 3, rotation: 0, turns: 0, writing: false });
        assert_eq!(file_source(&c.conn, 1).unwrap().unwrap().state, state());
        assert_eq!((turns(), placeholder(&c.conn, 1).unwrap().unwrap().hash), (0, ThumbHash::new(&[1])));
        // A turn asked for, not yet written.
        queue_rotation(&c.conn, 1, 3).unwrap();
        assert_eq!((state().turns, turns()), (3, 3));
        // One kept in the catalog since the analysis, and one more asked for.
        c.conn.execute("UPDATE files SET rotation = 1 WHERE id = 1", []).unwrap();
        queue_rotation(&c.conn, 1, 2).unwrap();
        assert_eq!((state().rotation, state().turns, turns()), (1, 1, 2));
        // Being written into the file.
        assert!(!state().writing);
        c.conn
            .execute(r"UPDATE file_writes SET temp = 'D:\p\b.jpg.tmp', temp_value = 1 WHERE file_id = 1", [])
            .unwrap();
        assert!(state().writing);
        assert_eq!(file_state(&c.conn, 99).unwrap(), None);
        assert!(placeholder(&c.conn, 99).unwrap().is_none());
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
            Some(Details {
                path: r"E:\p\a.png".into(),
                size: 10,
                rating_in_file: true,
                border_sides: detection::Sides::new()
            })
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
        let roots: Vec<&str> = tree.children(None).map(|n| tree.name(n)).collect();
        assert_eq!(roots, ["p", "p-x"]);
        // Its strings were made at the size reserved for them, not grown.
        let (_, bytes, components): (usize, usize, usize) =
            c.conn.query_row(FOLDER_SIZES, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!((tree.paths.capacity(), tree.keys.capacity()), (bytes, bytes + components));
        assert_eq!(tree.nodes.capacity(), tree.nodes.len());
    }

    /// A node as compared: merged path, name, depth, direct and total counts, drives (upper case, by letter), and
    /// how many children, in pre-order.
    type Flat = (String, String, usize, i64, i64, Vec<String>, usize);

    /// The tree as it was built before (kept as the reference): rows (folder id, path, present files) taken in
    /// folder id order, nodes found by their lowercased merged path, children sorted by their name's key.
    fn reference_tree(rows: &[(i64, String, i64)]) -> Vec<Flat> {
        struct Node {
            name: String,
            merged_path: String,
            depth: usize,
            children: Vec<usize>,
            direct: i64,
            total: i64,
            drives: String,
        }
        let mut rows = rows.to_vec();
        rows.sort_by_key(|r| r.0);
        let (mut nodes, mut roots) = (Vec::<Node>::new(), Vec::<usize>::new());
        let mut by_key: std::collections::BTreeMap<String, usize> = Default::default();
        for (_, path, count) in &rows {
            let drive = if merged_path(path).len() < path.len() { &path[..2] } else { "" };
            let mut parent: Option<usize> = None;
            let mut merged = String::new();
            for (depth, part) in components(merged_path(path)).enumerate() {
                if !merged.is_empty() {
                    merged.push('\\');
                }
                merged.push_str(part);
                let id = *by_key.entry(merged.to_lowercase()).or_insert_with(|| {
                    nodes.push(Node {
                        name: part.to_string(),
                        merged_path: merged.clone(),
                        depth,
                        children: Vec::new(),
                        direct: 0,
                        total: 0,
                        drives: String::new(),
                    });
                    let id = nodes.len() - 1;
                    match parent {
                        Some(p) => nodes[p].children.push(id),
                        None => roots.push(id),
                    }
                    id
                });
                let node = &mut nodes[id];
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
                nodes[p].direct += count;
            }
        }
        let names: Vec<Vec<u8>> = nodes.iter().map(|n| folder_key(&n.name)).collect();
        roots.sort_by(|&a, &b| names[a].cmp(&names[b]));
        for node in &mut nodes {
            node.children.sort_by(|&a, &b| names[a].cmp(&names[b]));
        }
        fn walk(nodes: &[Node], level: &[usize], out: &mut Vec<Flat>) {
            for &n in level {
                let node = &nodes[n];
                let mut drives: Vec<String> =
                    node.drives.split(' ').filter(|d| !d.is_empty()).map(str::to_ascii_uppercase).collect();
                drives.sort();
                let (name, path) = (node.name.clone(), node.merged_path.clone());
                out.push((path, name, node.depth, node.direct, node.total, drives, node.children.len()));
                walk(nodes, &node.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&nodes, &roots, &mut out);
        out
    }

    fn flat(tree: &FolderTree) -> Vec<Flat> {
        (0..tree.len())
            .map(|n| {
                let node = tree.node(n);
                let drives = tree.drives(n).split(' ').filter(|d| !d.is_empty()).map(String::from).collect();
                let children = tree.children(Some(n)).count();
                let (path, name) = (tree.merged_path(n).to_string(), tree.name(n).to_string());
                (path, name, node.depth as usize, node.direct, node.total, drives, children)
            })
            .collect()
    }

    /// Builds a tree of these rows, and checks it against the reference and its own structure.
    fn check_tree(rows: &[(i64, String, i64)]) {
        let mut builder = TreeBuilder::default();
        for (id, path, count) in rows {
            builder.add(*id, path, *count);
        }
        let tree = builder.build();
        assert_eq!(flat(&tree), reference_tree(rows));
        // The nodes were counted exactly before they were made.
        assert_eq!(tree.nodes.capacity(), tree.nodes.len());
        let mut key = Vec::new();
        for n in 0..tree.len() {
            assert_eq!(tree.find(tree.merged_path(n), &mut key), Some(n));
            assert_eq!(
                tree.find(&tree.merged_path(n).to_ascii_uppercase(), &mut key),
                Some(n),
                "{}",
                tree.merged_path(n)
            );
            assert_eq!(tree.key(n), folder_key(tree.merged_path(n)));
            assert_eq!(tree.merged(n), tree.drives(n).contains(' '));
            if let Some(p) = tree.parent(n) {
                assert!(p < n && n < tree.subtree_end(p) && tree.children(Some(p)).any(|c| c == n));
            }
        }
        assert_eq!(tree.find("no such folder", &mut key), None);
    }

    #[test]
    fn the_tree_is_built_as_before() {
        let named = [r"D:\p\Sub", r"E:\P\sub\x", r"E:\p", r"D:\p-x", r"d:\P", r"\\srv\share\a", r"D:\ΑΣ\b", r"E:\ας"];
        let named = named.into_iter().chain([r"D:\a\b", "D:\\a\\b\u{1}c", "D:", r"D:\İ", r"E:\i\u{307}"]);
        // A deeper folder first: its spelling of `p` wins over `E:\P`'s.
        check_tree(&named.enumerate().map(|(i, p)| (i as i64 + 1, p.to_string(), i as i64 + 1)).collect::<Vec<_>>());
        let mut rng = Rng(0x5851_F42D_4C95_7F2D);
        for _ in 0..30 {
            // Distinct paths and ids, as `folders` has them.
            let (mut paths, mut ids) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
            let mut rows: Vec<(i64, String, i64)> = Vec::new();
            for _ in 0..rng.next() % 200 {
                let (path, id, count) =
                    (random_file(&mut rng).0, (rng.next() % 1000) as i64, 1 + (rng.next() % 5) as i64);
                if paths.insert(path.clone()) && ids.insert(id) {
                    rows.push((id, path, count));
                }
            }
            check_tree(&rows);
        }
        check_tree(&[]);
    }

    /// On a real catalog (read-only): `PG_TEST_CATALOG=… cargo test -p pg-core real_tree -- --ignored`.
    #[test]
    #[ignore = "needs PG_TEST_CATALOG, a real catalog (opened read-only)"]
    fn real_tree_is_built_as_before() {
        let path = std::env::var_os("PG_TEST_CATALOG").expect("PG_TEST_CATALOG");
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let rows: Vec<(i64, String, i64)> = conn
            .prepare(FOLDER_COUNTS)
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        check_tree(&rows);
        let tree = folder_tree(&conn).unwrap();
        assert_eq!(flat(&tree), reference_tree(&rows));
        assert_eq!(tree.nodes.capacity(), tree.nodes.len());
        eprintln!("{} folders, {} nodes, {} files", rows.len(), tree.len(), tree.total());
        let sizes = |len: usize, cap: usize| format!("{len} of {cap}");
        eprintln!(
            "paths {}, keys {}, nodes {}",
            sizes(tree.paths.len(), tree.paths.capacity()),
            sizes(tree.keys.len(), tree.keys.capacity()),
            sizes(tree.nodes.len(), tree.nodes.capacity())
        );
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
                let key = order_key_of(&a.0, &a.1).cmp(&order_key_of(&b.0, &b.1));
                assert_eq!(key, reference_order((&a.0, &a.1), (&b.0, &b.1)), "{a:?} against {b:?}");
            }
        }
        // The cases named.
        let before = |a: (&str, &str), b: (&str, &str)| assert!(order_key_of(a.0, a.1) < order_key_of(b.0, b.1));
        before((r"D:\p", "z.jpg"), (r"D:\p\sub", "a.jpg"));
        before((r"D:\p\sub", "z.jpg"), (r"D:\p-x", "a.jpg"));
        before((r"E:\P", "a.jpg"), (r"D:\p", "b.jpg"));
        before((r"D:\ας", "a"), (r"D:\ασ", "a"));
        before((r"D:\a\b", "z"), ("D:\\a\\b\u{1}c", "a"));
        // A final Σ in a folder name lowercases to ς, as the whole path did.
        assert_eq!(order_key_of(r"D:\ΑΣ", "a"), order_key_of(r"E:\ας", "a"));
    }

    /// Folder keys as they were made, each component through `str::to_lowercase`: stored keys don't change.
    #[test]
    fn folder_keys_are_made_as_before() {
        let reference = |merged: &str| -> Vec<u8> {
            let mut key = Vec::new();
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
        };
        let merged = [
            r"Pinterest\ΑΣ\ΣΑ Σ\aΣb",
            r"ΟΔΥΣΣΕΥΣ\Σ",
            "\u{130}stanbul\\\u{212A}elvin\\ẞ",
            "a\u{1}b\\c\u{2}d\\\u{1}",
            r"\\nas\Photos\2024",
            "",
        ];
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let random = (0..400).map(|_| random_file(&mut rng).0);
        for folder in merged.into_iter().map(String::from).chain(random) {
            assert_eq!(folder_key(merged_path(&folder)), reference(merged_path(&folder)), "{folder:?}");
        }
        for name in ["a.JPG", "ΑΣ.jpg", "\u{130}.png", ""] {
            // Made at its exact size (`Vec::with_capacity` gives exactly what it is asked for).
            let key = order_key(b"k\x01", name);
            assert_eq!(key, [&b"k\x01"[..], &name_key(name)].concat());
            assert_eq!(key.capacity(), key.len());
        }
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
        by_key.sort_by_cached_key(|(folder, name)| order_key_of(folder, name));
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
            let key = order_key_of(folder, "a");
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
            let key = order_key_of(file.0, file.1);
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

    /// Sized by the count: an unfiltered list exactly, a filtered or sorted one without much slack.
    #[test]
    fn lists_are_made_at_their_size() {
        let c = mixed_library();
        let all = list_grid(&c.conn, "", true, &Query::default(), Sort::default()).unwrap();
        assert_eq!(all.capacity(), all.len());
        let sorted = list_grid(&c.conn, "", true, &Query::default(), by(SortKey::Size, true)).unwrap();
        assert_eq!(sorted.capacity(), sorted.len());
        let one =
            list_grid(&c.conn, "", true, &Query { ratings: vec![5], ..Query::default() }, Sort::default()).unwrap();
        assert!(one.capacity() < all.len() * 3 / 4 || one.capacity() == one.len());
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
            // The list's size, from the index alone.
            let (scope, values) = scope(folder, recursive);
            let plan: String = c
                .conn
                .query_row(
                    &format!("EXPLAIN QUERY PLAN SELECT count(*) FROM files f WHERE f.missing = 0 AND {scope}"),
                    params_from_iter(values),
                    |r| r.get(3),
                )
                .unwrap();
            assert!(plan.contains("COVERING INDEX files_order"), "{plan}");
        }
        let plan: Vec<String> = c
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {FOLDER_COUNTS}"))
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let plan = plan.join("\n");
        assert!(plan.contains("COVERING INDEX files_present") && !plan.contains("TEMP B-TREE"), "{plan}");
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
}
