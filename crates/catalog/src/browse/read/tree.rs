//! The folder tree, merged across drives (PLAN §7): its nodes in key order, each with its count and size,
//! and the lookups the tree panel makes of it.

use anyhow::Result;
use rusqlite::Connection;

use pg_model::path::{components, merged_path, push_folder_key};

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
    pub(super) nodes: Vec<FolderNode>,
    /// Each folder's merged path, its components joined by `\`, one after the other.
    pub(super) paths: String,
    /// Each folder's `folder_key`, one after the other.
    pub(super) keys: Vec<u8>,
}

/// The present files of each folder, counted from the `files_present` index alone (no sort, no table), and each
/// folder's path.
pub(super) const FOLDER_COUNTS: &str = "SELECT c.folder_id, (SELECT path FROM folders WHERE id = c.folder_id), c.n
    FROM (SELECT folder_id, count(*) AS n FROM files WHERE missing = 0 GROUP BY folder_id) c";

/// What the tree's strings take at most: every folder's row, path bytes and components (backslashes + 1), from the
/// `folders` table (a few thousand rows, read in a millisecond). The tree has the folders with present files: no
/// more rows, and no more path bytes than their paths (a key adds a byte per component, and lower case rarely
/// lengthens one). `char(92)` is `\`.
pub(super) const FOLDER_SIZES: &str = "SELECT count(*), coalesce(sum(length(CAST(path AS BLOB))), 0),
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
pub(super) struct TreeBuilder {
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
    pub(super) fn with_capacity(rows: usize, bytes: usize, components: usize) -> TreeBuilder {
        let tree = FolderTree {
            nodes: Vec::new(),
            paths: String::with_capacity(bytes),
            keys: Vec::with_capacity(bytes + components),
        };
        TreeBuilder { tree, rows: Vec::with_capacity(rows) }
    }

    /// A folder (`folders.id`, `folders.path`) holding `count` present files. A drive root (no components) is in no
    /// node.
    pub(super) fn add(&mut self, id: i64, path: &str, count: i64) {
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
    pub(super) fn build(self) -> FolderTree {
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
