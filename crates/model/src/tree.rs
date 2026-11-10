//! The folder tree, merged across drives (PLAN §7): its nodes in key order, each with its count and size, and the
//! lookups the tree panel makes of it. Built from the folders' paths (`TreeBuilder`); the catalog reads the rows. The
//! paths are typed ([`crate::rules::Path`]): a folder's parts and its key come from the rules the tree is built under.

use crate::name::Name;
use crate::num;
use crate::rules::{Merged, Path, PathRules};

/// A folder of the merged tree. Its bytes are in its [`FolderTree`].
#[derive(Clone, Debug)]
pub struct FolderNode {
    /// Its parts, `tree.paths[path_start..path_end]` joined by the separator, whose last part (its name) starts at
    /// `name_start`.
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
/// children in `folder_key` order (the grid's), so the nodes are also in key order. Paths and keys are ranges of two
/// byte strings, which makes the whole tree a handful of allocations.
#[derive(Clone, Debug)]
pub struct FolderTree {
    rules: PathRules,
    nodes: Vec<FolderNode>,
    /// Each folder's parts, joined by the separator, one after the other.
    paths: Vec<u8>,
    /// Each folder's `folder_key`, one after the other.
    keys: Vec<u8>,
    /// The anchors the folders are on, in text order (`D:`, `E:`): a Windows drive's root, uppercased. A folder with
    /// no anchor (a UNC or a Unix path) is on none.
    anchor_names: Vec<Name>,
    /// Present files directly in a drive root (a folder with no parts: `D:\`, or `/`), which no node holds. They are
    /// the merged folder `""` without its subfolders.
    root_files: i64,
}

impl FolderTree {
    /// A tree with no folders, under `rules` (a catalog's, before its first tree is read).
    pub fn empty(rules: PathRules) -> Self {
        FolderTree {
            rules,
            nodes: Vec::new(),
            paths: Vec::new(),
            keys: Vec::new(),
            anchor_names: Vec::new(),
            root_files: 0,
        }
    }
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

/// The folders a tree is made from: each one's parts and key written once, into the tree's bytes.
#[derive(Default)]
pub struct TreeBuilder {
    tree: FolderTree,
    rows: Vec<TreeRow>,
    /// The anchors the rows name, interned in the order they came.
    interned: Vec<Name>,
}

struct TreeRow {
    id: i64,
    count: i64,
    /// Its anchor's index in `interned`.
    anchor: Option<u32>,
    /// Its parts' range in `paths`, and its key's in `keys`.
    path: (u32, u32),
    key: (u32, u32),
}

impl TreeBuilder {
    /// Room for `rows` folders under `rules`, whose parts are `bytes` long in all with `components` components.
    pub fn with_capacity(rules: PathRules, rows: usize, bytes: usize, components: usize) -> TreeBuilder {
        let tree = FolderTree {
            rules,
            nodes: Vec::new(),
            paths: Vec::with_capacity(bytes),
            keys: Vec::with_capacity(bytes + components),
            anchor_names: Vec::new(),
            root_files: 0,
        };
        TreeBuilder { tree, rows: Vec::with_capacity(rows), interned: Vec::new() }
    }

    /// A folder (`folders.id`, its path) holding `count` present files. A drive root (no parts) is in no node: its
    /// files are counted in [`FolderTree::root_files`] (and so in [`FolderTree::total`]). Only a drive counts as an
    /// anchor: a UNC root or a Unix path's empty anchor is on none.
    pub fn add(&mut self, id: i64, path: &Path, count: i64) {
        if path.parts.is_empty() {
            self.tree.root_files = self.tree.root_files.saturating_add(count);
            return;
        }
        let rules = self.tree.rules;
        let anchor = match (rules, path.anchor.as_bytes()) {
            (PathRules::Windows, &[d, b':']) if d.is_ascii_alphabetic() => {
                let text = Name::from(format!("{}:", char::from(d.to_ascii_uppercase())).as_str());
                let index = match self.interned.iter().position(|a| *a == text) {
                    Some(index) => index,
                    None => {
                        self.interned.push(text);
                        self.interned.len() - 1
                    }
                };
                u32::try_from(index).ok()
            }
            _ => None,
        };
        let FolderTree { paths, keys, .. } = &mut self.tree;
        let path_start = paths.len();
        rules.push_parts(paths, &path.parts);
        let key_start = keys.len();
        rules.push_folder_key(keys, &path.parts);
        // The strings are addressed by `u32` offsets: a folder past them is left out of the tree.
        let range = |start: usize, end: usize| u32::try_from(start).ok().zip(u32::try_from(end).ok());
        let (Some(path), Some(key)) = (range(path_start, paths.len()), range(key_start, keys.len())) else { return };
        self.rows.push(TreeRow { id, count, anchor, path, key });
    }

    /// The tree: the rows sorted by key once, then the nodes made in that order with a stack of the open ones. A
    /// parent's key is a prefix of its descendants', and a component's end (0x01) sorts below every other byte, so
    /// sorted keys are the tree's pre-order with children in key order: `p`, `p\sub`, `p-x`.
    pub fn build(self) -> FolderTree {
        let TreeBuilder { mut tree, mut rows, interned } = self;
        let sep = tree.rules.sep_byte();
        // Every row's range was cut from `keys` when it was added.
        let key = |k: (u32, u32)| tree.keys.get(num::index(k.0)..num::index(k.1)).unwrap_or_default();
        rows.sort_unstable_by(|a, b| key(a.key).cmp(key(b.key)));
        // The anchors in text order (a drive's letter, so `D:` before `E:`), and each interned anchor's rank: how
        // many of them sort before it (they are distinct).
        let rank: Vec<usize> = interned.iter().map(|a| interned.iter().filter(|&b| b < a).count()).collect();
        let mut anchor_names = interned;
        anchor_names.sort();
        tree.anchor_names = anchor_names;
        let row_bit = |row: &TreeRow| row.anchor.and_then(|a| rank.get(num::index(a))).map_or(0, |&r| bit(r));
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
            // The row's parts, with where each ends in its path and its key.
            let path = tree.paths.get(num::index(row.path.0)..num::index(row.path.1)).unwrap_or(&[]);
            let mut path_end = row.path.0;
            let mut key_end = row.key.0;
            let mut key_parts = row_key.split_inclusive(|&b| b == 0x01);
            for (depth, part) in path.split(|&b| b == sep).enumerate() {
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
    /// The rules the tree was built under: its parts, keys and anchors follow them.
    pub fn rules(&self) -> PathRules {
        self.rules
    }

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

    /// Its folder, as one of its drives spells it: its parts, the anchor dropped. The empty folder for a node past the
    /// tree's end.
    pub fn merged(&self, n: usize) -> Merged {
        let Some(node) = self.nodes.get(n) else { return Merged::default() };
        let bytes = self.paths.get(num::index(node.path_start)..num::index(node.path_end)).unwrap_or_default();
        let sep = self.rules.sep_byte();
        bytes.split(|&b| b == sep).filter(|p| !p.is_empty()).map(Name::from_raw).collect::<Vec<Name>>().into()
    }

    /// Its last part (its name); the empty name for a node past the tree's end.
    pub fn name(&self, n: usize) -> Name {
        let Some(node) = self.nodes.get(n) else { return Name::from("") };
        Name::from_raw(self.paths.get(num::index(node.name_start)..num::index(node.path_end)).unwrap_or_default())
    }

    /// Its `folder_key`; empty for a node past the tree's end.
    pub fn key(&self, n: usize) -> &[u8] {
        self.nodes.get(n).map(|node| self.node_key(node)).unwrap_or_default()
    }

    /// A node's `folder_key`: the range the builder cut from `keys`.
    fn node_key(&self, node: &FolderNode) -> &[u8] {
        self.keys.get(num::index(node.key_start)..num::index(node.key_end)).unwrap_or_default()
    }

    pub fn parent(&self, n: usize) -> Option<usize> {
        let p = self.nodes.get(n)?.parent;
        (p != u32::MAX).then_some(num::index(p))
    }

    /// The node after its last descendant; the tree's end for a node past it.
    pub fn subtree_end(&self, n: usize) -> usize {
        self.nodes.get(n).map_or(self.nodes.len(), |node| num::index(node.end))
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
            text.push_str(&name.display());
        }
        text
    }

    /// Whether it is on more than one anchor (a drive folder merged across drives).
    pub fn multi_anchor(&self, n: usize) -> bool {
        self.nodes.get(n).is_some_and(|node| node.anchors.count_ones() > 1)
    }

    /// Present files in every folder of the tree, the drive roots' own files included: the files "All folders" lists.
    pub fn total(&self) -> i64 {
        let folders: i64 = self.children(None).filter_map(|n| self.nodes.get(n)).map(|node| node.total).sum();
        folders.saturating_add(self.root_files)
    }

    /// Present files directly in a drive root, which no node holds: the files the merged folder `""` lists without its
    /// subfolders (the "Drive roots" row).
    pub fn root_files(&self) -> i64 {
        self.root_files
    }

    /// The node of a folder, in any case (`key` holds its key meanwhile).
    pub fn find(&self, folder: &Merged, key: &mut Vec<u8>) -> Option<usize> {
        key.clear();
        self.rules.push_folder_key(key, folder.parts());
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
        let mut builder = TreeBuilder::with_capacity(PathRules::Windows, rows.len(), bytes, 6);
        for (id, path, count) in rows {
            builder.add(id, &PathRules::Windows.parse(path).unwrap(), count);
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
        let mut builder = TreeBuilder::with_capacity(PathRules::Windows, 4, 0, 0);
        for (id, path) in [(1, r"E:\p"), (2, r"D:\p"), (3, r"F:\q"), (4, r"\\nas\share\p")] {
            builder.add(id, &PathRules::Windows.parse(path).unwrap(), 1);
        }
        let tree = builder.build();
        let node = |folder: &str| {
            (0..tree.len()).find(|&n| PathRules::Windows.display_merged(&tree.merged(n)) == folder).unwrap()
        };
        assert_eq!(tree.anchors(node("p")), "D: E:");
        assert!(tree.multi_anchor(node("p")));
        assert_eq!(tree.anchors(node("q")), "F:");
        assert!(!tree.multi_anchor(node("q")));
        assert_eq!(tree.anchors(node(r"nas\share\p")), "");
        assert!(!tree.multi_anchor(node(r"nas\share\p")));
    }

    /// Files directly in drive roots are in no node, but they are counted: `root_files` holds them, and `total` (what
    /// "All folders" lists) includes them.
    #[test]
    fn drive_root_files_alone_are_counted_but_make_no_node() {
        let mut builder = TreeBuilder::with_capacity(PathRules::Windows, 2, 0, 0);
        builder.add(1, &PathRules::Windows.parse("D:").unwrap(), 3);
        builder.add(2, &PathRules::Windows.parse(r"E:\").unwrap(), 2);
        let tree = builder.build();
        assert!(tree.is_empty());
        assert_eq!(tree.root_files(), 5);
        assert_eq!(tree.total(), 5);
        assert_eq!(TreeBuilder::with_capacity(PathRules::Posix, 0, 0, 0).build().root_files(), 0);
    }

    /// With folders beside them, the drive roots' files add to the folders' total and to no node.
    #[test]
    fn drive_root_files_beside_folders_add_to_the_total_only() {
        let mut builder = TreeBuilder::with_capacity(PathRules::Windows, 4, 0, 0);
        for (id, path, count) in [(1, "D:", 3), (2, r"D:\p", 2), (3, r"E:\p\sub", 4), (4, "E:", 1)] {
            builder.add(id, &PathRules::Windows.parse(path).unwrap(), count);
        }
        let tree = builder.build();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.root_files(), 4);
        assert_eq!(tree.total(), 10);
        let folders: i64 = tree.children(None).map(|n| tree.node(n).unwrap().total).sum();
        assert_eq!(folders, 6);
        let mut posix = TreeBuilder::with_capacity(PathRules::Posix, 2, 0, 0);
        posix.add(1, &PathRules::Posix.parse("/").unwrap(), 2);
        posix.add(2, &PathRules::Posix.parse("/home").unwrap(), 1);
        let posix = posix.build();
        assert_eq!((posix.len(), posix.root_files(), posix.total()), (1, 2, 3));
    }

    /// A node's folder and name come back exactly, with a lone surrogate, and `find` takes the typed folder in any case.
    #[test]
    fn a_node_gives_back_its_folder_and_name_exactly() {
        let wide = Name::from_wide(&[0x61, 0xD800]);
        let folder = Merged::from(vec![Name::from("Pics"), wide.clone()]);
        let mut builder = TreeBuilder::with_capacity(PathRules::Posix, 1, 0, 0);
        let path = Path { anchor: Name::from(""), parts: folder.parts().to_vec() };
        builder.add(7, &path, 3);
        let tree = builder.build();
        // Two nodes: `Pics` (depth 0) and `Pics\<wide>` (depth 1, the folder itself).
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.merged(1), folder);
        assert_eq!(tree.name(1), wide);
        let mut key = Vec::new();
        assert_eq!(tree.find(&folder, &mut key), Some(1));
        assert_eq!(tree.find(&Merged::from(vec![Name::from("pics")]), &mut key), None);
    }
}
