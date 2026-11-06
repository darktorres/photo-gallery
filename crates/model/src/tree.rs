//! The folder tree, merged across drives (PLAN §7): its nodes in key order, each with its count and size, and the
//! lookups the tree panel makes of it. Built from the folders' rows (`TreeBuilder`); the catalog reads the rows.

use crate::path::{components, merged_path, push_folder_key};

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
    /// One bit per anchor it is on: the anchor's rank in `anchor_names` (see [`bit`]).
    anchors: u64,
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
    /// The anchors the folders are on, in text order (`D:`, `E:`): a Windows drive's root as the path spells it.
    /// A folder with no anchor (a UNC or a Unix path) is on none.
    anchor_names: Vec<Box<str>>,
}

/// The bit of an anchor by its rank in text order. The 64th and later anchors share bit 63: no machine has that many
/// drive letters, and a node that shares it shows each of them ([`FolderTree::anchors`]).
fn bit(rank: usize) -> u64 {
    1 << rank.min(63)
}

/// A node's index, depth or part length as the nodes hold it (`u32`). `TreeBuilder::add` keeps every row within `u32`
/// offsets of the strings, and each node is at least one component's 0x01 in `keys`, so each fits; one that didn't
/// would saturate.
fn node_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The folders a tree is made from: each one's merged path and key written once, into the tree's strings.
#[derive(Default)]
pub struct TreeBuilder {
    tree: FolderTree,
    rows: Vec<TreeRow>,
    /// The anchors the rows name, interned in the order they came.
    interned: Vec<Box<str>>,
}

struct TreeRow {
    id: i64,
    count: i64,
    /// Its anchor's index in `interned`.
    anchor: Option<u32>,
    /// Its merged path's range in `paths`, and its key's in `keys`.
    path: (u32, u32),
    key: (u32, u32),
}

impl TreeBuilder {
    /// Room for `rows` folders whose paths are `bytes` long with `components` components in all.
    pub fn with_capacity(rows: usize, bytes: usize, components: usize) -> TreeBuilder {
        let tree = FolderTree {
            nodes: Vec::new(),
            paths: String::with_capacity(bytes),
            keys: Vec::with_capacity(bytes + components),
            anchor_names: Vec::new(),
        };
        TreeBuilder { tree, rows: Vec::with_capacity(rows), interned: Vec::new() }
    }

    /// A folder (`folders.id`, `folders.path`) holding `count` present files. A drive root (no components) is in no
    /// node.
    pub fn add(&mut self, id: i64, path: &str, count: i64) {
        let merged = merged_path(path);
        let anchor = match path.as_bytes() {
            [d, b':', ..] if merged.len() < path.len() => {
                let text = format!("{}:", char::from(d.to_ascii_uppercase()));
                let index = match self.interned.iter().position(|a| **a == *text) {
                    Some(index) => index,
                    None => {
                        self.interned.push(text.into());
                        self.interned.len() - 1
                    }
                };
                u32::try_from(index).ok()
            }
            _ => None,
        };
        let FolderTree { paths, keys, .. } = &mut self.tree;
        let path_start = paths.len();
        for (i, part) in components(merged).enumerate() {
            if i > 0 {
                paths.push('\\');
            }
            paths.push_str(part);
        }
        if paths.len() == path_start {
            return;
        }
        let key_start = keys.len();
        push_folder_key(keys, merged);
        // The strings are addressed by `u32` offsets: a folder past them is left out of the tree.
        let range = |start: usize, end: usize| u32::try_from(start).ok().zip(u32::try_from(end).ok());
        let (Some(path), Some(key)) = (range(path_start, paths.len()), range(key_start, keys.len())) else { return };
        self.rows.push(TreeRow { id, count, anchor, path, key });
    }

    /// The tree: the rows sorted by key once, then the nodes made in that order with a stack of the open ones. A
    /// parent's key is a prefix of its descendants', and a component's end (0x01) sorts below every other byte, so
    /// sorted keys are the tree's pre-order with children in key order: `p`, `p\sub`, `p-x`.
    #[expect(clippy::string_slice, reason = "every cut falls between whole pushed parts, so on a char boundary")]
    pub fn build(self) -> FolderTree {
        let TreeBuilder { mut tree, mut rows, interned } = self;
        // Every row's range was cut from `keys` when it was added.
        let key = |k: (u32, u32)| tree.keys.get(crate::num::index(k.0)..crate::num::index(k.1)).unwrap_or_default();
        rows.sort_unstable_by(|a, b| key(a.key).cmp(key(b.key)));
        // The anchors in text order (a drive's letter, so `D:` before `E:`), and each interned anchor's rank: how
        // many of them sort before it (they are distinct).
        let rank: Vec<usize> = interned.iter().map(|a| interned.iter().filter(|&b| b < a).count()).collect();
        let mut anchor_names = interned;
        anchor_names.sort();
        tree.anchor_names = anchor_names;
        let row_bit = |row: &TreeRow| row.anchor.and_then(|a| rank.get(crate::num::index(a))).map_or(0, |&r| bit(r));
        // The nodes, counted: in this order each row adds the components past those it shares with the row before
        // it (whole components, each ending with 0x01, which none holds), as the stack below finds them open.
        let components = |k: &[u8]| k.iter().filter(|&&b| b == 0x01).count();
        let mut before: &[u8] = &[];
        let mut nodes = 0;
        for row in &rows {
            let k = key(row.key);
            let shared = k.iter().zip(before).take_while(|(a, b)| a == b);
            nodes += components(k) - shared.filter(|&(&a, _)| a == 0x01).count();
            before = k;
        }
        tree.nodes.reserve_exact(nodes);
        let mut open: Vec<usize> = Vec::new();
        for row in &rows {
            let row_key = key(row.key);
            // Close the nodes the row isn't under.
            let end = node_u32(tree.nodes.len());
            while let Some(&top) = open.last() {
                let Some(node) = tree.nodes.get_mut(top) else { break };
                if row_key.starts_with(key((node.key_start, node.key_end))) {
                    break;
                }
                node.end = end;
                open.pop();
            }
            // The row's components, with where each ends in its path and its key.
            let path = &tree.paths[crate::num::index(row.path.0)..crate::num::index(row.path.1)];
            let mut path_end = row.path.0;
            let mut key_end = row.key.0;
            let mut key_parts = row_key.split_inclusive(|&b| b == 0x01);
            for (depth, part) in path.split('\\').enumerate() {
                let name_start = path_end + u32::from(depth > 0);
                path_end = name_start + node_u32(part.len());
                key_end += node_u32(key_parts.next().map_or(0, <[u8]>::len));
                let n = match open.get(depth) {
                    Some(&n) => n,
                    None => {
                        tree.nodes.push(FolderNode {
                            path_start: row.path.0,
                            path_end,
                            name_start,
                            key_start: row.key.0,
                            key_end,
                            parent: open.last().map_or(u32::MAX, |&p| node_u32(p)),
                            end: 0,
                            first: row.id,
                            anchors: 0,
                            depth: node_u32(depth),
                            direct: 0,
                            total: 0,
                        });
                        open.push(tree.nodes.len() - 1);
                        tree.nodes.len() - 1
                    }
                };
                let Some(node) = tree.nodes.get_mut(n) else { continue };
                node.total += row.count;
                node.anchors |= row_bit(row);
                // Spelled as the first folder that reached it (the old tree's order: by folder id).
                if row.id < node.first {
                    (node.first, node.path_start, node.path_end, node.name_start) =
                        (row.id, row.path.0, path_end, name_start);
                    (node.key_start, node.key_end) = (row.key.0, key_end);
                }
            }
            if let Some(&n) = open.last()
                && let Some(node) = tree.nodes.get_mut(n)
            {
                node.direct += row.count;
            }
        }
        let end = node_u32(tree.nodes.len());
        for n in open {
            if let Some(node) = tree.nodes.get_mut(n) {
                node.end = end;
            }
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

    /// The node at `n`; `None` for a node past the tree's end.
    pub fn node(&self, n: usize) -> Option<&FolderNode> {
        self.nodes.get(n)
    }

    /// Its merged path, as one of its drives spells it; empty for a node past the tree's end.
    #[expect(clippy::string_slice, reason = "the builder cut the range between whole parts, so on a char boundary")]
    pub fn merged_path(&self, n: usize) -> &str {
        let Some(node) = self.nodes.get(n) else { return "" };
        &self.paths[crate::num::index(node.path_start)..crate::num::index(node.path_end)]
    }

    /// Its path's last component; empty for a node past the tree's end.
    #[expect(clippy::string_slice, reason = "the builder cut the range between whole parts, so on a char boundary")]
    pub fn name(&self, n: usize) -> &str {
        let Some(node) = self.nodes.get(n) else { return "" };
        &self.paths[crate::num::index(node.name_start)..crate::num::index(node.path_end)]
    }

    /// Its `folder_key`; empty for a node past the tree's end.
    pub fn key(&self, n: usize) -> &[u8] {
        self.nodes.get(n).map(|node| self.node_key(node)).unwrap_or_default()
    }

    /// A node's `folder_key`: the range the builder cut from `keys`.
    fn node_key(&self, node: &FolderNode) -> &[u8] {
        self.keys.get(crate::num::index(node.key_start)..crate::num::index(node.key_end)).unwrap_or_default()
    }

    pub fn parent(&self, n: usize) -> Option<usize> {
        let p = self.nodes.get(n)?.parent;
        (p != u32::MAX).then_some(crate::num::index(p))
    }

    /// The node after its last descendant; the tree's end for a node past it.
    pub fn subtree_end(&self, n: usize) -> usize {
        self.nodes.get(n).map_or(self.nodes.len(), |node| crate::num::index(node.end))
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

    /// The anchors it is on, in text order: `D: E:`. Past 63 anchors a node shows every anchor of the shared bit 63.
    /// Empty for a node past the tree's end.
    pub fn anchors(&self, n: usize) -> String {
        let bits = self.nodes.get(n).map_or(0, |node| node.anchors);
        let mut text = String::new();
        for (_, name) in self.anchor_names.iter().enumerate().filter(|(rank, _)| bits & bit(*rank) != 0) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(name);
        }
        text
    }

    /// Whether it is on more than one anchor (a drive folder merged across drives).
    pub fn multi_anchor(&self, n: usize) -> bool {
        self.nodes.get(n).is_some_and(|node| node.anchors.count_ones() > 1)
    }

    /// Present files in every folder of the tree.
    pub fn total(&self) -> i64 {
        self.children(None).filter_map(|n| self.nodes.get(n)).map(|node| node.total).sum()
    }

    /// The node of a merged path, in any case (`key` holds its key meanwhile).
    pub fn find(&self, merged: &str, key: &mut Vec<u8>) -> Option<usize> {
        key.clear();
        push_folder_key(key, merged);
        self.find_key(key)
    }

    /// The node whose `folder_key` is `key`.
    pub fn find_key(&self, key: &[u8]) -> Option<usize> {
        self.nodes.binary_search_by(|n| self.node_key(n).cmp(key)).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strings and the nodes are made at the size they are reserved at: a tree built from rows whose paths are
    /// `bytes` long in all, with `components` components, never grows them.
    #[test]
    fn the_strings_and_nodes_are_sized_up_front() {
        let rows = [(1, r"D:\p\sub", 2), (2, r"E:\p", 1), (3, r"D:\p-x", 4)];
        let bytes: usize = rows.iter().map(|r| r.1.len()).sum();
        let mut builder = TreeBuilder::with_capacity(rows.len(), bytes, 6);
        for (id, path, count) in rows {
            builder.add(id, path, count);
        }
        let tree = builder.build();
        assert_eq!(tree.len(), 3);
        assert_eq!(tree.paths.capacity(), bytes);
        assert_eq!(tree.keys.capacity(), bytes + 6);
        assert_eq!(tree.nodes.capacity(), tree.nodes.len());
    }

    /// A folder's anchors are the drives it is on, in text order; a folder on one anchor is not merged, and a UNC
    /// folder is on none.
    #[test]
    fn a_folder_shows_each_anchor_it_is_on_in_text_order() {
        let mut builder = TreeBuilder::default();
        for (id, path) in [(1, r"E:\p"), (2, r"D:\p"), (3, r"F:\q"), (4, r"\\nas\share\p")] {
            builder.add(id, path, 1);
        }
        let tree = builder.build();
        let node = |merged: &str| (0..tree.len()).find(|&n| tree.merged_path(n) == merged).unwrap();
        assert_eq!(tree.anchors(node("p")), "D: E:");
        assert!(tree.multi_anchor(node("p")));
        assert_eq!(tree.anchors(node("q")), "F:");
        assert!(!tree.multi_anchor(node("q")));
        assert_eq!(tree.anchors(node(r"nas\share\p")), "");
        assert!(!tree.multi_anchor(node(r"nas\share\p")));
    }
}
