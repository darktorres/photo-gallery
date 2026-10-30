//! What the UI browses (PLAN §7): a folder tree where folders with the same path on different drives are one
//! node ("merged drives"), the files of a node, and ratings.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{Connection, params};

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
    let mut by_key: HashMap<String, usize> = HashMap::new();
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
    pub width: u32,
    pub height: u32,
    pub rating: u8,
}

impl Item {
    pub fn aspect(&self) -> f64 {
        if self.width > 0 && self.height > 0 { self.width as f64 / self.height as f64 } else { 1.0 }
    }
}

/// The present files of a merged folder (on every drive), optionally with everything below it, sorted by folder
/// and then by name.
pub fn list_files(conn: &Connection, merged: &str, recursive: bool) -> Result<Vec<Item>> {
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

    let mut stmt = conn.prepare_cached(
        "SELECT f.id, fo.path || '\\' || f.name, f.mtime, coalesce(a.width, 0), coalesce(a.height, 0),
                coalesce(r.rating, 0)
         FROM files f JOIN folders fo ON fo.id = f.folder_id
         LEFT JOIN analysis a ON a.file_id = f.id AND a.error IS NULL
         LEFT JOIN ratings r ON r.file_id = f.id
         WHERE f.folder_id = ?1 AND f.missing = 0",
    )?;
    let mut items = Vec::new();
    // Folders with the same merged path (one per drive) are listed together and sorted by name as one.
    for group in folders.chunk_by(|a, b| a.0 == b.0) {
        let start = items.len();
        for (_, id) in group {
            let rows = stmt.query_map([id], |r| {
                Ok(Item {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    mtime: r.get(2)?,
                    width: r.get(3)?,
                    height: r.get(4)?,
                    rating: r.get(5)?,
                })
            })?;
            for item in rows {
                items.push(item?);
            }
        }
        items[start..].sort_by_cached_key(|i| file_name(&i.path).to_lowercase());
    }
    Ok(items)
}

fn file_name(path: &str) -> &str {
    path.rsplit_once('\\').map_or(path, |(_, n)| n)
}

/// 1–5, or 0 to clear.
pub fn set_rating(conn: &Connection, file_id: i64, rating: u8) -> Result<()> {
    if rating == 0 {
        conn.execute("DELETE FROM ratings WHERE file_id = ?1", [file_id])?;
    } else {
        conn.execute(
            "INSERT INTO ratings(file_id, rating, updated) VALUES (?1, ?2, unixepoch())
             ON CONFLICT(file_id) DO UPDATE SET rating = excluded.rating, updated = excluded.updated",
            params![file_id, rating.min(5)],
        )?;
    }
    Ok(())
}

/// What the image providers need to find a file.
pub struct FileSource {
    pub path: String,
    /// The thumbnail cache's validity key.
    pub mtime: i64,
    pub thumbhash: Option<Vec<u8>>,
}

pub fn file_source(conn: &Connection, file_id: i64) -> Result<Option<FileSource>> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .prepare_cached(
            "SELECT fo.path || '\\' || f.name, f.mtime, a.thumbhash FROM files f
             JOIN folders fo ON fo.id = f.folder_id LEFT JOIN analysis a ON a.file_id = f.id
             WHERE f.id = ?1",
        )?
        .query_row([file_id], |r| Ok(FileSource { path: r.get(0)?, mtime: r.get(1)?, thumbhash: r.get(2)? }))
        .optional()?)
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
}
