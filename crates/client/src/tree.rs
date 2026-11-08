//! The folder tree's expansion: the rows the folder pane shows, of a tree whose folders open and close. Row 0 is "All"
//! (the merged folder `""` with its subfolders); row 1 is "Drive roots" (`""` without them: the files directly in a
//! drive root, which no folder holds) when there are such files; the other rows are the tree's nodes in its pre-order,
//! the visible ones: a node is shown when every folder above it is open. Recursion is the listing's one toggle, so the
//! two first rows are the pairs (`""`, recursive) and (`""`, not recursive). The desktop wraps this in its list model
//! and does the Qt part (the insert and remove signals); the rows it inserts or removes come from here, before the
//! change, so the signals say what changed.

use std::cell::RefCell;
use std::ops::Range;

use pg_model::rules::Merged;
use pg_model::tree::FolderTree;

/// A visible row. The order is the rows' order (All, then Drive roots, then the nodes in pre-order), which
/// [`Tree::row_of`] searches by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Visible {
    /// Every folder, with the subfolders: the folder `""`, recursive.
    All,
    /// The files directly in the drive roots: the folder `""`, not recursive. Shown only when there are some.
    DriveRoots,
    /// A node of the tree.
    Folder(usize),
}

/// A toggle of a folder, before it is made: whether it opens, and the rows below its row that it inserts (opening) or
/// removes (closing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toggle {
    pub expand: bool,
    pub rows: Range<usize>,
}

/// The folder tree as shown: which folders are open, and the rows that follow from it.
pub struct Tree {
    tree: FolderTree,
    /// Per node.
    expanded: Vec<bool>,
    visible: Vec<Visible>,
    /// A folder key being looked up, kept from one lookup to the next.
    key: RefCell<Vec<u8>>,
}

impl Default for Tree {
    /// Only "All", until the first tree.
    fn default() -> Tree {
        Tree { tree: FolderTree::default(), expanded: Vec::new(), visible: vec![Visible::All], key: RefCell::default() }
    }
}

impl Tree {
    /// The folders, for their names, totals and children.
    pub fn folders(&self) -> &FolderTree {
        &self.tree
    }

    /// The visible rows, "All" first (then "Drive roots", when there are files in them).
    pub fn rows(&self) -> &[Visible] {
        &self.visible
    }

    /// The row a number shows, if it is a row.
    pub fn row(&self, row: i32) -> Option<Visible> {
        usize::try_from(row).ok().and_then(|r| self.visible.get(r)).copied()
    }

    /// Whether a folder is open (not, for a node outside the tree).
    pub fn is_expanded(&self, n: usize) -> bool {
        self.expanded.get(n).copied().unwrap_or(false)
    }

    /// Show a new tree. Open folders stay open (the background job reloads after every scan), as far as they are still
    /// there. With none open, single-child chains open from the top, so a folder whose only child holds the files
    /// shows its contents at once.
    pub fn replace(&mut self, tree: FolderTree) {
        let mut expanded = vec![false; tree.len()];
        let mut any = false;
        for (n, _) in self.expanded.iter().enumerate().filter(|&(_, &open)| open) {
            if let Some(open) = tree.find_key(self.tree.key(n)).and_then(|m| expanded.get_mut(m)) {
                *open = true;
                any = true;
            }
        }
        (self.tree, self.expanded) = (tree, expanded);
        if !any {
            let mut parent = None;
            loop {
                let mut children = self.tree.children(parent);
                let (Some(only), None) = (children.next(), children.next()) else { break };
                let Some(open) = self.expanded.get_mut(only) else { break };
                *open = true;
                parent = Some(only);
            }
        }
        self.show_all();
    }

    /// The node of a folder (the empty folder is "All" or "Drive roots", not a node), in any case.
    pub fn find(&self, folder: &Merged) -> Option<usize> {
        self.tree.find(folder, &mut self.key.borrow_mut())
    }

    /// The row a listing's folder shows on, if it shows. The empty folder is "All" (row 0) with its subfolders, and
    /// "Drive roots" (row 1) without them when that row is shown; with no files in the drive roots it is "All" either
    /// way, so the empty folder always has a row. Recursion makes no other folder's row.
    pub fn row_of(&self, folder: &Merged, recursive: bool) -> Option<usize> {
        let row = if !folder.is_empty() {
            Visible::Folder(self.find(folder)?)
        } else if !recursive && self.tree.root_files() > 0 {
            Visible::DriveRoots
        } else {
            Visible::All
        };
        self.visible.binary_search(&row).ok()
    }

    /// What toggling the folder on `row` would do, without doing it: `None` for "All", for "Drive roots" and for a
    /// folder with no children.
    pub fn plan_toggle(&self, row: usize) -> Option<Toggle> {
        let Visible::Folder(n) = *self.visible.get(row)? else { return None };
        if !self.tree.has_children(n) {
            return None;
        }
        let below = self.visible_count(n + 1..self.tree.subtree_end(n));
        let at = row + 1;
        Some(Toggle { expand: !self.is_expanded(n), rows: at..at + below })
    }

    /// Opens or closes the folder on `row`, as [`Tree::plan_toggle`] says; nothing when there is nothing to toggle.
    pub fn toggle(&mut self, row: usize) {
        let Some(plan) = self.plan_toggle(row) else { return };
        let Some(Visible::Folder(n)) = self.visible.get(row).copied() else { return };
        let Some(open) = self.expanded.get_mut(n) else { return };
        *open = plan.expand;
        if plan.expand {
            // The rows shown under `n` when it is open (its children's own expansion is remembered).
            let mut below = Vec::with_capacity(plan.rows.len());
            self.visible_in(n + 1..self.tree.subtree_end(n), &mut below);
            self.visible.splice(plan.rows.start..plan.rows.start, below);
        } else {
            self.visible.drain(plan.rows);
        }
    }

    /// Opens the folders above a listing's folder so it shows: its row ([`Tree::row_of`]), or `None` when there is no
    /// such folder.
    pub fn reveal(&mut self, folder: &Merged, recursive: bool) -> Option<usize> {
        if let Some(row) = self.row_of(folder, recursive) {
            return Some(row);
        }
        let n = self.find(folder)?;
        let mut above = self.tree.parent(n);
        while let Some(p) = above {
            if let Some(open) = self.expanded.get_mut(p) {
                *open = true;
            }
            above = self.tree.parent(p);
        }
        self.show_all();
        self.row_of(folder, recursive)
    }

    /// The visible rows from the expansion: "All", "Drive roots" when there are files in them, then the tree.
    fn show_all(&mut self) {
        let mut visible = std::mem::take(&mut self.visible);
        visible.clear();
        visible.push(Visible::All);
        if self.tree.root_files() > 0 {
            visible.push(Visible::DriveRoots);
        }
        self.visible_in(0..self.tree.len(), &mut visible);
        self.visible = visible;
    }

    /// The nodes of a range of whole subtrees that show, in order: each one, and below the open ones their visible
    /// descendants.
    fn visible_in(&self, nodes: Range<usize>, out: &mut Vec<Visible>) {
        let mut n = nodes.start;
        while n < nodes.end {
            let Some(&open) = self.expanded.get(n) else { break };
            out.push(Visible::Folder(n));
            n = if open { n + 1 } else { self.tree.subtree_end(n) };
        }
    }

    /// How many of a range of nodes show: [`Tree::visible_in`] without the rows.
    fn visible_count(&self, nodes: Range<usize>) -> usize {
        let mut n = nodes.start;
        let mut count = 0;
        while n < nodes.end {
            let Some(&open) = self.expanded.get(n) else { break };
            count += 1;
            n = if open { n + 1 } else { self.tree.subtree_end(n) };
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::rules::PathRules;
    use pg_model::tree::TreeBuilder;

    /// A folder given as its Windows path text, merged.
    fn folder(text: &str) -> Merged {
        PathRules::Windows.merge(&PathRules::Windows.parse(text).unwrap())
    }

    /// The folders `D:\a`, `D:\a\b`, `D:\a\b\c`, `D:\a\d` and `D:\e` (merged: `a`, `a\b`, `a\b\c`, `a\d`, `e`), as
    /// nodes 0..5 in pre-order.
    fn tree() -> FolderTree {
        let mut b = TreeBuilder::with_capacity(PathRules::Windows, 5, 0, 0);
        for (id, path) in [(1, r"D:\a"), (2, r"D:\a\b"), (3, r"D:\a\b\c"), (4, r"D:\a\d"), (5, r"D:\e")] {
            b.add(id, &PathRules::Windows.parse(path).unwrap(), 1);
        }
        b.build()
    }

    /// The visible rows as the merged paths they show (`""` for "All", `"/"` for "Drive roots").
    fn shown(t: &Tree) -> Vec<String> {
        let rules = t.folders().rules();
        t.rows()
            .iter()
            .map(|r| match *r {
                Visible::All => String::new(),
                Visible::DriveRoots => "/".to_string(),
                Visible::Folder(n) => rules.display_merged(&t.folders().merged(n)),
            })
            .collect()
    }

    /// `tree()`'s folders, with `count` files directly in `D:\` and in `E:\`.
    fn tree_with_roots(count: i64) -> FolderTree {
        let mut b = TreeBuilder::with_capacity(PathRules::Windows, 7, 0, 0);
        for (id, path) in [(1, r"D:\a"), (2, r"D:\a\b"), (3, r"D:\a\b\c"), (4, r"D:\a\d"), (5, r"D:\e")] {
            b.add(id, &PathRules::Windows.parse(path).unwrap(), 1);
        }
        b.add(6, &PathRules::Windows.parse(r"D:\").unwrap(), count);
        b.add(7, &PathRules::Windows.parse(r"E:\").unwrap(), count);
        b.build()
    }

    /// With files in the drive roots, "Drive roots" is row 1: the folder `""` without subfolders, which neither
    /// toggles nor moves; with none, the row is not there and `""` is "All" either way.
    #[test]
    fn drive_roots_is_row_one_when_the_roots_hold_files() {
        let mut t = Tree::default();
        t.replace(tree_with_roots(2));
        assert_eq!(shown(&t), ["", "/", "a", "e"]);
        assert_eq!(t.row(0), Some(Visible::All));
        assert_eq!(t.row(1), Some(Visible::DriveRoots));
        assert_eq!(t.folders().total(), 9);
        assert_eq!(t.folders().root_files(), 4);
        let all = Merged::default();
        assert_eq!(t.row_of(&all, true), Some(0));
        assert_eq!(t.row_of(&all, false), Some(1));
        assert_eq!(t.reveal(&all, false), Some(1));
        // Recursion makes no other folder's row.
        assert_eq!(t.row_of(&folder("e"), true), Some(3));
        assert_eq!(t.row_of(&folder("e"), false), Some(3));
        // Neither of the first two rows toggles; a folder below them toggles at its own row.
        assert_eq!(t.plan_toggle(0), None);
        assert_eq!(t.plan_toggle(1), None);
        t.toggle(1);
        assert_eq!(shown(&t), ["", "/", "a", "e"]);
        let plan = t.plan_toggle(2).unwrap();
        assert_eq!((plan.expand, plan.rows), (true, 3..5));
        t.toggle(2);
        assert_eq!(shown(&t), ["", "/", "a", r"a\b", r"a\d", "e"]);
        assert_eq!(t.reveal(&folder(r"a\b\c"), true), Some(4));

        // Without root files the row goes; the folder `""` is "All" with or without subfolders.
        t.replace(tree_with_roots(0));
        assert_eq!(shown(&t)[..2], ["", "a"]);
        assert_eq!(t.row_of(&all, false), Some(0));
        assert_eq!(t.row_of(&all, true), Some(0));
        assert_eq!(t.folders().total(), 5);
    }

    #[test]
    fn folders_open_and_close_and_their_rows_follow() {
        let mut t = Tree::default();
        assert_eq!(shown(&t), [""]);
        t.replace(tree());
        // Two top-level folders, so nothing opens by itself.
        assert_eq!(shown(&t), ["", "a", "e"]);
        // Opening `a` (row 1) shows its two children.
        let plan = t.plan_toggle(1).unwrap();
        assert_eq!((plan.expand, plan.rows.clone()), (true, 2..4));
        t.toggle(1);
        assert_eq!(shown(&t), ["", "a", r"a\b", r"a\d", "e"]);
        // `b` opens too, and keeps its state while `a` closes: closing `a` takes `b`, `c` and `d` (three rows).
        t.toggle(2);
        assert_eq!(shown(&t).len(), 6);
        let plan = t.plan_toggle(1).unwrap();
        assert_eq!((plan.expand, plan.rows), (false, 2..5));
        t.toggle(1);
        assert_eq!(shown(&t), ["", "a", "e"]);
        t.toggle(1);
        assert_eq!(shown(&t), ["", "a", r"a\b", r"a\b\c", r"a\d", "e"]);
        // "All", a folder with no children, and a row past the end don't toggle.
        assert_eq!(t.plan_toggle(0), None);
        assert_eq!(t.plan_toggle(4), None);
        assert_eq!(t.plan_toggle(99), None);
        t.toggle(99);
        assert_eq!(shown(&t).len(), 6);
    }

    #[test]
    fn reveal_opens_the_folders_above_and_gives_its_row() {
        let mut t = Tree::default();
        t.replace(tree());
        assert_eq!(t.reveal(&folder(r"a\b\c"), true), Some(3));
        assert_eq!(shown(&t), ["", "a", r"a\b", r"a\b\c", r"a\d", "e"]);
        // Any case; a folder that isn't there; "All" (with or without subfolders: no files are in the drive roots).
        assert_eq!(t.row_of(&folder(r"A\D"), false), Some(4));
        assert_eq!(t.reveal(&folder("nope"), true), None);
        assert_eq!(t.reveal(&Merged::default(), true), Some(0));
        assert_eq!(t.reveal(&Merged::default(), false), Some(0));
    }

    #[test]
    fn an_open_folder_stays_open_in_a_new_tree() {
        let mut t = Tree::default();
        t.replace(tree());
        t.toggle(1);
        t.replace(tree());
        assert_eq!(shown(&t), ["", "a", r"a\b", r"a\d", "e"]);
        assert!(t.is_expanded(0));
    }

    // A folder whose name has a lone surrogate is opened by its row, and it reveals that row: the folder it opens is its
    // own parts, exactly (no text stands in for the name).
    #[test]
    fn a_folder_with_a_lone_surrogate_opens_and_reveals_by_its_row() {
        let rules = PathRules::Windows;
        let odd = pg_model::name::Name::from_wide(&[0x61, 0xD800]);
        let mut path = rules.parse(r"D:").unwrap();
        path.parts = vec![odd.clone()];
        let mut b = TreeBuilder::with_capacity(rules, 2, 0, 0);
        b.add(1, &path, 1);
        let mut t = Tree::default();
        t.replace(b.build());
        let Some(Visible::Folder(node)) = t.row(1) else { panic!("the folder's row") };
        let opened = t.folders().merged(node);
        assert_eq!(opened, Merged::from(vec![odd]));
        assert_eq!(t.reveal(&opened, true), Some(1));
    }
}
