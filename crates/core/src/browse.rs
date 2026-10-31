//! What the UI browses (PLAN §7): a folder tree where folders with the same path on different drives are one
//! node ("merged drives"), the files of a node filtered and sorted, ratings and turns (queued for the files),
//! saved views and where the user was.

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};

use crate::decode::Format;
use crate::media::Kind;
use crate::metadata;
use crate::query::{Facet, MediaType, Query, Sort};

/// A folder's path without its drive: `E:\pinterest\a` and `D:\pinterest\a` are both `pinterest\a`; a drive
/// root is the empty path. Other paths (UNC) are kept whole.
pub fn merged_path(path: &str) -> &str {
    match path.as_bytes() {
        [d, b':', ..] if d.is_ascii_alphabetic() => path[2..].trim_start_matches('\\'),
        _ => path,
    }
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
    let mut stmt = conn.prepare(
        "SELECT fo.path, count(*) FROM folders fo JOIN files f ON f.folder_id = fo.id AND f.missing = 0
         GROUP BY fo.id",
    )?;
    let folders: Vec<(String, i64)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;

    let mut tree = FolderTree { nodes: Vec::new(), roots: Vec::new() };
    let mut by_key: BTreeMap<String, usize> = BTreeMap::new();
    for (path, count) in &folders {
        let drive = if merged_path(path).len() < path.len() { &path[..2] } else { "" };
        let mut parent: Option<usize> = None;
        let mut merged = String::new();
        let parts: Vec<&str> = merged_path(path).split('\\').filter(|p| !p.is_empty()).collect();
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
    let names: Vec<String> = tree.nodes.iter().map(|n| n.name.to_lowercase()).collect();
    tree.roots.sort_by(|&a, &b| names[a].cmp(&names[b]));
    for node in &mut tree.nodes {
        node.children.sort_by(|&a, &b| names[a].cmp(&names[b]));
    }
    Ok(tree)
}

/// A file as the grid shows it. `width`/`height` are 0 until the file is imported.
pub struct Item {
    pub id: i64,
    pub path: String,
    pub mtime: i64,
    pub size: i64,
    /// Catalog rotation (quarter turns clockwise); with `mtime`, what makes an image URL current.
    pub rotation: u8,
    pub width: u32,
    pub height: u32,
    pub rating: u8,
    /// Whether the file can hold its rating too (`false`: kept in the catalog only). Unknown before the import.
    pub rating_in_file: bool,
    /// Quarter turns clockwise asked for but not yet written (`queue_rotation`): the grid shows them already.
    pub pending_turns: u8,
    pub kind: Kind,
    /// More than one frame (known after the import).
    pub animated: bool,
    /// Videos, once imported.
    pub duration_ms: Option<i64>,
}

impl Item {
    /// Width ÷ height as shown, pending turns included.
    pub fn aspect(&self) -> f64 {
        let (w, h) = if self.pending_turns % 2 == 1 { (self.height, self.width) } else { (self.width, self.height) };
        if w > 0 && h > 0 { w as f64 / h as f64 } else { 1.0 }
    }
}

/// Case-insensitive comparison of names and paths as Windows sees them (Unicode, unlike SQLite's NOCASE).
/// Registered on every catalog connection (`Catalog::open`).
pub fn path_collation(a: &str, b: &str) -> std::cmp::Ordering {
    // Character by character, without allocating: a sort compares every name many times (FINDINGS "Startup").
    a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase)).then_with(|| a.cmp(b))
}

/// What an `Item` is read from (`item_from_row`), with `f` the file.
const ITEM_COLUMNS: &str = "f.id, fo.path || '\\' || f.name, f.mtime, f.size, f.rotation, coalesce(a.width, 0),
    coalesce(a.height, 0), coalesce(r.rating, 0), a.format, coalesce(w.value, 0), f.kind, coalesce(a.frames, 1),
    a.duration_ms";
const ITEM_JOINS: &str = "JOIN folders fo ON fo.id = f.folder_id
    LEFT JOIN analysis a ON a.file_id = f.id AND a.error IS NULL
    LEFT JOIN ratings r ON r.file_id = f.id
    LEFT JOIN file_writes w ON w.file_id = f.id AND w.kind = 'rotation'";

fn item_from_row(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    let format: Option<String> = r.get(8)?;
    let kind = Kind::from_key(&r.get::<_, String>(10)?).unwrap_or(Kind::Image);
    Ok(Item {
        id: r.get(0)?,
        path: r.get(1)?,
        mtime: r.get(2)?,
        size: r.get(3)?,
        rotation: r.get(4)?,
        width: r.get(5)?,
        height: r.get(6)?,
        rating: r.get(7)?,
        // Videos keep theirs in the catalog (rewriting a video to add XMP would copy GBs).
        rating_in_file: kind == Kind::Image
            && format.as_deref().and_then(Format::from_name).is_none_or(metadata::can_hold_rating),
        pending_turns: r.get(9)?,
        kind,
        animated: r.get::<_, i64>(11)? > 1,
        duration_ms: r.get(12)?,
    })
}

/// Read an item again (after its file was turned), keeping its place in the list. `None` if it is gone.
pub fn reload_item(conn: &Connection, file_id: i64) -> Result<Option<Item>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {ITEM_COLUMNS} FROM files f {ITEM_JOINS} WHERE f.id = ?1"))?
        .query_row([file_id], item_from_row)
        .optional()?)
}

/// Run `f` with `temp.view_folders` holding the folders of a merged folder (on every drive), optionally with
/// everything below it, each with its rank in path order (`ord`; the drives of one merged folder share it). The
/// folder match is done here, as merged paths are a Rust function; the rest is SQL.
fn in_scope<T>(
    conn: &Connection,
    merged: &str,
    recursive: bool,
    f: impl FnOnce(&Transaction) -> Result<T>,
) -> Result<T> {
    let merged = merged.to_lowercase();
    // A row that fails (the read cancelled) fails the scope: a shorter list of folders would look like a result.
    let all: Vec<(String, i64)> = conn
        .prepare("SELECT path, id FROM folders")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut folders: Vec<(String, i64)> = all
        .into_iter()
        .filter_map(|(path, id)| {
            let m = merged_path(&path).to_lowercase();
            let inside = m == merged
                || (recursive
                    && (merged.is_empty() || m.strip_prefix(&merged).is_some_and(|rest| rest.starts_with('\\'))));
            inside.then_some((m, id))
        })
        .collect();
    // In the tree's order: by folder name at each level (`p\sub` right after `p`, before `p-x`).
    folders.sort_by(|(a, a_id), (b, b_id)| a.split('\\').cmp(b.split('\\')).then(a_id.cmp(b_id)));

    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS view_folders(id INTEGER PRIMARY KEY, ord INTEGER NOT NULL);
         DELETE FROM temp.view_folders;",
    )?;
    {
        let mut insert = tx.prepare_cached("INSERT INTO temp.view_folders(id, ord) VALUES (?1, ?2)")?;
        let mut ord = 0;
        for (i, (m, id)) in folders.iter().enumerate() {
            if i > 0 && *m != folders[i - 1].0 {
                ord += 1;
            }
            insert.execute(params![id, ord])?;
        }
    }
    let out = f(&tx)?;
    tx.commit()?;
    Ok(out)
}

/// The present files of a view: a merged folder (optionally with everything below it), filtered and ordered.
pub fn list_files(conn: &Connection, merged: &str, recursive: bool, query: &Query, sort: Sort) -> Result<Vec<Item>> {
    in_scope(conn, merged, recursive, |tx| {
        let cond = query.condition();
        let sql = format!(
            "SELECT {ITEM_COLUMNS} FROM temp.view_folders v JOIN files f ON f.folder_id = v.id {ITEM_JOINS}
             WHERE f.missing = 0 AND {}
             ORDER BY {}",
            cond.sql,
            sort.sql()
        );
        let mut stmt = tx.prepare(&sql)?;
        let items = stmt.query_map(params_from_iter(cond.params), item_from_row)?;
        Ok(items.collect::<rusqlite::Result<Vec<_>>>()?)
    })
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
}

pub fn facets(conn: &Connection, merged: &str, recursive: bool, query: &Query) -> Result<Facets> {
    in_scope(conn, merged, recursive, |tx| {
        let from = format!("FROM temp.view_folders v JOIN files f ON f.folder_id = v.id {ITEM_JOINS}");
        let count = |skip: Option<Facet>, select: &str, group: &str| -> Result<Vec<(String, i64)>> {
            let cond = query.condition_without(skip);
            let sql = format!("SELECT {select}, count(*) {from} WHERE f.missing = 0 AND {} {group}", cond.sql);
            let mut stmt = tx.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(cond.params), |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        let total: i64 = tx.query_row(
            "SELECT count(*) FROM temp.view_folders v JOIN files f ON f.folder_id = v.id WHERE f.missing = 0",
            [],
            |r| r.get(0),
        )?;
        let matching = count(None, "''", "")?.first().map_or(0, |r| r.1);
        let mut types: Vec<(MediaType, i64)> = count(Some(Facet::Types), MediaType::CASE, "GROUP BY 1")?
            .into_iter()
            .filter_map(|(t, n)| Some((MediaType::from_key(&t)?, n)))
            .collect();
        types.sort();
        let exts = count(Some(Facet::Exts), "f.ext", "GROUP BY 1 ORDER BY 2 DESC, 1")?;
        Ok(Facets { total, matching, types, exts })
    })
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

pub fn views(conn: &Connection) -> Result<Vec<View>> {
    let mut stmt = conn.prepare("SELECT name, folder, recursive, query, sort FROM views ORDER BY name COLLATE PATH")?;
    let rows = stmt.query_map([], |r| {
        Ok(View {
            name: r.get(0)?,
            folder: r.get(1)?,
            recursive: r.get(2)?,
            query: Query::from_json(&r.get::<_, String>(3)?),
            sort: Sort::from_json(&r.get::<_, String>(4)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
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
    use crate::query::{DateRange, Orientation, Range, SortKey, Status};

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
            c.conn
                .execute(
                    "INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan)
                     VALUES (?1, 1, ?1, ?2, ?3, ?4, 'image', ?5, ?6, 1)",
                    params![id, folder, name, crate::media::extension(name), size, mtime],
                )
                .unwrap();
            c.conn
                .execute(
                    "INSERT INTO analysis(file_id, version, size, mtime, analyzed, format, width, height)
                     VALUES (?1, 1, ?2, ?3, 0, ?4, 4, 3)",
                    params![id, size, mtime, format],
                )
                .unwrap();
            set_rating(&c.conn, id, rating, false).unwrap();
        }
        c
    }

    fn ids(c: &crate::catalog::Catalog, folder: &str, recursive: bool, query: &Query, sort: Sort) -> Vec<i64> {
        list_files(&c.conn, folder, recursive, query, sort).unwrap().iter().map(|i| i.id).collect()
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
        let bmp = list_files(&c.conn, "p\\sub", false, &all, Sort::default()).unwrap();
        assert!(!bmp[0].rating_in_file, "BMP keeps its rating in the catalog");
    }

    #[test]
    fn folders_in_the_trees_order() {
        let c = library();
        // "p-x" sorts before "p\sub" as a whole path ('-' < '\'), but after it in the tree (p's children first).
        c.conn
            .execute_batch(
                "INSERT INTO folders(id, path) VALUES (4, 'D:\\p-x');
                 INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan)
                 VALUES (9, 1, 9, 4, 'z.jpg', 'jpg', 'image', 1, 1, 1);",
            )
            .unwrap();
        assert_eq!(ids(&c, "", true, &Query::default(), Sort::default()), [2, 1, 3, 4, 9]);
        let tree = folder_tree(&c.conn).unwrap();
        let roots: Vec<&str> = tree.roots.iter().map(|&n| tree.nodes[n].name.as_str()).collect();
        assert_eq!(roots, ["p", "p-x"]);
    }

    #[test]
    fn path_order() {
        use std::cmp::Ordering::*;
        // Case-insensitively first (Unicode, unlike SQLite's NOCASE), then exactly, so the order is total.
        assert_eq!(path_collation("a.jpg", "B.jpg"), Less);
        assert_eq!(path_collation("Ä.jpg", "b.jpg"), Greater);
        assert_eq!(path_collation("ä.jpg", "Ä.JPG"), Greater);
        assert_eq!(path_collation("B.jpg", "b.jpg"), Less);
        assert_eq!(path_collation("img10", "IMG10"), Greater);
        assert_eq!(path_collation("abc", "ABC"), Greater);
        assert_eq!(path_collation("ab", "abc"), Less);
        assert_eq!(path_collation("same", "same"), Equal);
    }

    /// `library()` plus a video, an animated GIF, a file the import couldn't read and one not imported yet, with
    /// duplicates: 1 and 3 are the same bytes, 2–4 a close tier-1 pair, 2–6 a distant one.
    fn mixed_library() -> crate::catalog::Catalog {
        let c = library();
        c.conn
            .execute_batch(
                "INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan) VALUES
                    (5, 1, 5, 3, 'Clip.MP4', 'mp4', 'video', 50, 7, 1),
                    (6, 1, 6, 1, 'anim.gif', 'gif', 'image', 15, 2, 1),
                    (7, 1, 7, 2, 'broken.jpg', 'jpg', 'image', 5, 4, 1),
                    (8, 1, 8, 2, 'new.webp', 'webp', 'image', 6, 6, 1);
                 INSERT INTO analysis(file_id, version, size, mtime, analyzed, format, width, height, frames,
                                      duration_ms) VALUES
                    (5, 1, 50, 7, 0, NULL, 1920, 1080, 300, 12000),
                    (6, 1, 15, 2, 0, 'gif', 3, 3, 10, NULL);
                 INSERT INTO analysis(file_id, version, size, mtime, analyzed, error) VALUES (7, 1, 5, 4, 0, 'bad');
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
        let items = list_files(&c.conn, "", true, &Query::default(), Sort::default()).unwrap();
        let item = items.iter().find(|i| i.id == 1).unwrap();
        assert_eq!(item.pending_turns, 3);
        assert_eq!(item.aspect(), 3.0 / 4.0);
        assert_eq!(reload_item(&c.conn, 1).unwrap().map(|i| i.pending_turns), Some(3));
    }
}
