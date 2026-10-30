//! What the UI browses (PLAN §7): a folder tree where folders with the same path on different drives are one
//! node ("merged drives"), the files of a node filtered and sorted, ratings and turns (queued for the files),
//! saved views and where the user was.

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use crate::decode::Format;
use crate::metadata;

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
}

impl Item {
    /// Width ÷ height as shown, pending turns included.
    pub fn aspect(&self) -> f64 {
        let (w, h) = if self.pending_turns % 2 == 1 { (self.height, self.width) } else { (self.width, self.height) };
        if w > 0 && h > 0 { w as f64 / h as f64 } else { 1.0 }
    }
}

/// Which files a view shows, by rating (ImageRater's Unrated / Rated / N-star searches).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    All,
    Unrated,
    Rated,
    Exactly(u8),
}

impl Filter {
    pub fn key(self) -> String {
        match self {
            Filter::All => "all".into(),
            Filter::Unrated => "unrated".into(),
            Filter::Rated => "rated".into(),
            Filter::Exactly(n) => n.to_string(),
        }
    }

    pub fn from_key(key: &str) -> Filter {
        match key {
            "unrated" => Filter::Unrated,
            "rated" => Filter::Rated,
            k => k.parse().ok().filter(|n| (1..=5).contains(n)).map_or(Filter::All, Filter::Exactly),
        }
    }

    fn sql(self) -> String {
        match self {
            Filter::All => "1".into(),
            Filter::Unrated => "r.rating IS NULL".into(),
            Filter::Rated => "r.rating IS NOT NULL".into(),
            Filter::Exactly(n) => format!("r.rating = {}", n.clamp(1, 5)),
        }
    }
}

/// The order of a view. Name keeps folders together (in path order); the others sort across folders, as
/// ImageRater's triage did (largest first).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    Name,
    Largest,
    Newest,
}

impl Sort {
    pub fn key(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Largest => "size",
            Sort::Newest => "date",
        }
    }

    pub fn from_key(key: &str) -> Sort {
        match key {
            "size" => Sort::Largest,
            "date" => Sort::Newest,
            _ => Sort::Name,
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Sort::Name => "v.ord, f.name COLLATE PATH",
            Sort::Largest => "f.size DESC, v.ord, f.name COLLATE PATH",
            Sort::Newest => "f.mtime DESC, v.ord, f.name COLLATE PATH",
        }
    }
}

/// Case-insensitive comparison of names and paths as Windows sees them (Unicode, unlike SQLite's NOCASE).
/// Registered on every catalog connection (`Catalog::open`).
pub fn path_collation(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b))
}

/// What an `Item` is read from (`item_from_row`), with `f` the file.
const ITEM_COLUMNS: &str = "f.id, fo.path || '\\' || f.name, f.mtime, f.size, f.rotation, coalesce(a.width, 0),
    coalesce(a.height, 0), coalesce(r.rating, 0), a.format, coalesce(w.value, 0)";
const ITEM_JOINS: &str = "JOIN folders fo ON fo.id = f.folder_id
    LEFT JOIN analysis a ON a.file_id = f.id AND a.error IS NULL
    LEFT JOIN ratings r ON r.file_id = f.id
    LEFT JOIN file_writes w ON w.file_id = f.id AND w.kind = 'rotation'";

fn item_from_row(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    let format: Option<String> = r.get(8)?;
    Ok(Item {
        id: r.get(0)?,
        path: r.get(1)?,
        mtime: r.get(2)?,
        size: r.get(3)?,
        rotation: r.get(4)?,
        width: r.get(5)?,
        height: r.get(6)?,
        rating: r.get(7)?,
        rating_in_file: format.as_deref().and_then(Format::from_name).is_none_or(metadata::can_hold_rating),
        pending_turns: r.get(9)?,
    })
}

/// Read an item again (after its file was turned), keeping its place in the list. `None` if it is gone.
pub fn reload_item(conn: &Connection, file_id: i64) -> Result<Option<Item>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {ITEM_COLUMNS} FROM files f {ITEM_JOINS} WHERE f.id = ?1"))?
        .query_row([file_id], item_from_row)
        .optional()?)
}

/// The present files of a merged folder (on every drive), optionally with everything below it, that pass the
/// filter, in the given order. The folder match is done here (merged paths are a Rust function); filtering and
/// sorting are SQL.
pub fn list_files(conn: &Connection, merged: &str, recursive: bool, filter: Filter, sort: Sort) -> Result<Vec<Item>> {
    let merged = merged.to_lowercase();
    let mut folders: Vec<(String, i64)> = conn
        .prepare("SELECT path, id FROM folders")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?
        .filter_map(|r| {
            let (path, id) = r.ok()?;
            let m = merged_path(&path).to_lowercase();
            let inside = m == merged
                || (recursive
                    && (merged.is_empty() || m.strip_prefix(&merged).is_some_and(|rest| rest.starts_with('\\'))));
            inside.then_some((m, id))
        })
        .collect();
    folders.sort();

    // Folders with the same merged path (one per drive) share a rank, so they list as one.
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
    let sql = format!(
        "SELECT {ITEM_COLUMNS} FROM temp.view_folders v JOIN files f ON f.folder_id = v.id {ITEM_JOINS}
         WHERE f.missing = 0 AND {}
         ORDER BY {}",
        filter.sql(),
        sort.sql()
    );
    let items = tx.prepare(&sql)?.query_map([], item_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    tx.commit()?;
    Ok(items)
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
}

pub fn file_source(conn: &Connection, file_id: i64) -> Result<Option<FileSource>> {
    Ok(conn
        .prepare_cached(
            "SELECT fo.path || '\\' || f.name, f.mtime, f.rotation, a.thumbhash FROM files f
             JOIN folders fo ON fo.id = f.folder_id LEFT JOIN analysis a ON a.file_id = f.id
             WHERE f.id = ?1",
        )?
        .query_row([file_id], |r| {
            Ok(FileSource { path: r.get(0)?, mtime: r.get(1)?, rotation: r.get(2)?, thumbhash: r.get(3)? })
        })
        .optional()?)
}

/// A saved view (ImageRater's bookmarks): where, which ratings, in what order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct View {
    pub name: String,
    /// Merged path; `""` for all folders.
    pub folder: String,
    pub recursive: bool,
    pub filter: Filter,
    pub sort: Sort,
}

pub fn views(conn: &Connection) -> Result<Vec<View>> {
    let mut stmt =
        conn.prepare("SELECT name, folder, recursive, filter, sort FROM views ORDER BY name COLLATE PATH")?;
    let rows = stmt.query_map([], |r| {
        Ok(View {
            name: r.get(0)?,
            folder: r.get(1)?,
            recursive: r.get(2)?,
            filter: Filter::from_key(&r.get::<_, String>(3)?),
            sort: Sort::from_key(&r.get::<_, String>(4)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Save a view, replacing one of the same name.
pub fn save_view(conn: &Connection, v: &View) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO views(name, folder, recursive, filter, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![v.name, v.folder, v.recursive, v.filter.key(), v.sort.key()],
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

#[cfg(test)]
mod tests {
    use super::*;

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
                    "INSERT INTO files(id, volume, file_ref, folder_id, name, size, mtime, last_scan)
                     VALUES (?1, 1, ?1, ?2, ?3, ?4, ?5, 1)",
                    params![id, folder, name, size, mtime],
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

    fn ids(c: &crate::catalog::Catalog, folder: &str, recursive: bool, filter: Filter, sort: Sort) -> Vec<i64> {
        list_files(&c.conn, folder, recursive, filter, sort).unwrap().iter().map(|i| i.id).collect()
    }

    #[test]
    fn filters_and_sorts() {
        let c = library();
        // Name: both drives' "p" as one folder, by name (Unicode case-insensitive: Ä after b), then "p\sub".
        assert_eq!(ids(&c, "p", true, Filter::All, Sort::Name), [2, 1, 3, 4]);
        assert_eq!(ids(&c, "p", false, Filter::All, Sort::Name), [2, 1, 3]);
        assert_eq!(ids(&c, "", true, Filter::All, Sort::Largest), [4, 1, 3, 2]);
        assert_eq!(ids(&c, "", true, Filter::All, Sort::Newest), [2, 4, 1, 3]);
        assert_eq!(ids(&c, "", true, Filter::Unrated, Sort::Largest), [4, 2]);
        assert_eq!(ids(&c, "", true, Filter::Rated, Sort::Name), [1, 3]);
        assert_eq!(ids(&c, "", true, Filter::Exactly(2), Sort::Name), [3]);
        for f in [Filter::All, Filter::Unrated, Filter::Rated, Filter::Exactly(5)] {
            assert_eq!(Filter::from_key(&f.key()), f);
        }
        let bmp = list_files(&c.conn, "p\\sub", false, Filter::All, Sort::Name).unwrap();
        assert!(!bmp[0].rating_in_file, "BMP keeps its rating in the catalog");
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
        let items = list_files(&c.conn, "", true, Filter::All, Sort::Name).unwrap();
        let item = items.iter().find(|i| i.id == 1).unwrap();
        assert_eq!(item.pending_turns, 3);
        assert_eq!(item.aspect(), 3.0 / 4.0);
        assert_eq!(reload_item(&c.conn, 1).unwrap().map(|i| i.pending_turns), Some(3));
    }
}
