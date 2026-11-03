//! The folder tree (merged across drives, `pg_core::browse::read::folder_tree`) as a flat list model: each visible
//! node is a row with its depth; expanding a node inserts its visible descendants below it. Row 0 is "All".
//!
//! The tree is read off the GUI thread (`loader`): `reload` asks, `loaded` says it's shown. Until the first one,
//! only "All" is there (`ready` is false). Paths match as Windows matches them, ignoring case: the tree keeps the
//! case of one drive's folder, the session another's.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        /// The tree has been read at least once.
        #[qproperty(bool, ready, READ, WRITE, NOTIFY, FINAL)]
        type FolderModel = super::FolderRust;
    }

    impl cxx_qt::Threading for FolderModel {}

    unsafe extern "RustQt" {
        /// The tree `reload` asked for is shown.
        #[qsignal]
        fn loaded(self: Pin<&mut FolderModel>);

        #[qinvokable]
        #[cxx_override]
        fn data(self: &FolderModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &FolderModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &FolderModel, parent: &QModelIndex) -> i32;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut FolderModel>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut FolderModel>);

        #[inherit]
        #[cxx_name = "beginInsertRows"]
        fn begin_insert_rows(self: Pin<&mut FolderModel>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endInsertRows"]
        fn end_insert_rows(self: Pin<&mut FolderModel>);

        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        fn begin_remove_rows(self: Pin<&mut FolderModel>, parent: &QModelIndex, first: i32, last: i32);

        #[inherit]
        #[cxx_name = "endRemoveRows"]
        fn end_remove_rows(self: Pin<&mut FolderModel>);

        #[inherit]
        #[cxx_name = "dataChanged"]
        fn data_changed(self: Pin<&mut FolderModel>, top_left: &QModelIndex, bottom_right: &QModelIndex);

        #[inherit]
        fn index(self: &FolderModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        /// Read the tree from the catalog (at the start, after a scan); it is shown when it arrives (`loaded`).
        #[qinvokable]
        fn reload(self: Pin<&mut FolderModel>);

        /// Expand or collapse a row.
        #[qinvokable]
        fn toggle(self: Pin<&mut FolderModel>, row: i32);

        /// The merged path of a row (`""` for "All").
        #[qinvokable]
        #[cxx_name = "mergedPath"]
        fn merged_path(self: &FolderModel, row: i32) -> QString;

        /// The visible row of a merged path (`""` = "All"), or -1.
        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &FolderModel, merged: &QString) -> i32;

        /// Expand the folders above a merged path so it shows; its row, or -1 if there is no such folder.
        #[qinvokable]
        fn reveal(self: Pin<&mut FolderModel>, merged: &QString) -> i32;

        /// Whether the tree has a merged path (`"` = "All"), shown or under a collapsed folder.
        #[qinvokable]
        fn contains(self: &FolderModel, merged: &QString) -> bool;
    }
}

use core::pin::Pin;
use std::cell::RefCell;
use std::ops::Range;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_core::browse::{self, read::FolderTree};

use crate::loader::{self, Kind};

const ROLE_NAME: i32 = 256;
const ROLE_DEPTH: i32 = 257;
const ROLE_EXPANDABLE: i32 = 258;
const ROLE_EXPANDED: i32 = 259;
const ROLE_TOTAL: i32 = 260;
const ROLE_DRIVES: i32 = 261;
const ROLE_MERGED: i32 = 262;

/// A visible row: `None` is the "All" row. The rows are in the tree's pre-order, so in node order.
type Visible = Option<usize>;

pub struct FolderRust {
    ready: bool,
    tree: FolderTree,
    /// Only "All" until the first tree.
    visible: Vec<Visible>,
    /// Per node.
    expanded: Vec<bool>,
    /// The latest tree asked for; older ones arriving are dropped.
    generation: u64,
    /// A folder key being looked up, kept from one lookup to the next.
    key: RefCell<Vec<u8>>,
}

impl Default for FolderRust {
    fn default() -> Self {
        FolderRust {
            ready: false,
            tree: FolderTree::default(),
            visible: vec![None],
            expanded: Vec::new(),
            generation: 0,
            key: RefCell::default(),
        }
    }
}

impl FolderRust {
    /// Show a new tree. Expanded folders stay expanded (the background job reloads after every scan), as they are
    /// now: the user may have opened some while it was read.
    fn replace(&mut self, tree: FolderTree) {
        let mut expanded = vec![false; tree.len()];
        let mut any = false;
        for n in (0..self.tree.len()).filter(|&n| self.expanded[n]) {
            if let Some(m) = tree.find_key(self.tree.key(n)) {
                expanded[m] = true;
                any = true;
            }
        }
        (self.tree, self.expanded) = (tree, expanded);
        // Otherwise open single-child chains from the top, so a folder whose only child holds the files shows
        // its contents at once.
        if !any {
            let mut parent = None;
            loop {
                let mut children = self.tree.children(parent);
                let (Some(only), None) = (children.next(), children.next()) else { break };
                self.expanded[only] = true;
                parent = Some(only);
            }
        }
        self.show_all();
    }

    /// The visible rows from the expansion: "All", then the tree.
    fn show_all(&mut self) {
        let mut visible = std::mem::take(&mut self.visible);
        visible.clear();
        visible.push(None);
        self.visible_in(0..self.tree.len(), &mut visible);
        self.visible = visible;
    }

    /// The nodes of a range of whole subtrees that show, in order: each one, and below the expanded ones their
    /// visible descendants.
    fn visible_in(&self, nodes: Range<usize>, out: &mut Vec<Visible>) {
        let mut n = nodes.start;
        while n < nodes.end {
            out.push(Some(n));
            n = if self.expanded[n] { n + 1 } else { self.tree.subtree_end(n) };
        }
    }

    /// The node of a merged path, in any case.
    fn find(&self, merged: &str) -> Option<usize> {
        self.tree.find(merged, &mut self.key.borrow_mut())
    }

    fn node(&self, row: i32) -> Option<Visible> {
        usize::try_from(row).ok().and_then(|r| self.visible.get(r)).copied()
    }

    /// The row a merged path shows on (`""` = "All"), if it shows.
    fn row_of_path(&self, merged: &str) -> Option<usize> {
        if merged.is_empty() {
            return Some(0);
        }
        self.visible.binary_search(&Some(self.find(merged)?)).ok()
    }
}

impl qobject::FolderModel {
    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(v) = self.node(index.row()) else { return QVariant::default() };
        let Some(n) = v else {
            return match role {
                ROLE_NAME => QVariant::from(&QString::from("All folders")),
                ROLE_DEPTH => QVariant::from(&0i32),
                ROLE_EXPANDABLE | ROLE_EXPANDED => QVariant::from(&false),
                ROLE_TOTAL => QVariant::from(&self.tree.total()),
                ROLE_DRIVES => QVariant::from(&QString::default()),
                ROLE_MERGED => QVariant::from(&false),
                _ => QVariant::default(),
            };
        };
        let node = self.tree.node(n);
        match role {
            ROLE_NAME => QVariant::from(&QString::from(self.tree.name(n))),
            ROLE_DEPTH => QVariant::from(&(node.depth as i32)),
            ROLE_EXPANDABLE => QVariant::from(&self.tree.has_children(n)),
            ROLE_EXPANDED => QVariant::from(&self.expanded[n]),
            ROLE_TOTAL => QVariant::from(&node.total),
            ROLE_DRIVES => QVariant::from(&QString::from(&self.tree.drives(n))),
            ROLE_MERGED => QVariant::from(&self.tree.merged(n)),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (ROLE_NAME, "name"),
            (ROLE_DEPTH, "depth"),
            (ROLE_EXPANDABLE, "expandable"),
            (ROLE_EXPANDED, "expanded"),
            (ROLE_TOTAL, "total"),
            (ROLE_DRIVES, "drives"),
            (ROLE_MERGED, "merged"),
        ] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.visible.len() as i32
    }

    fn reload(mut self: Pin<&mut Self>) {
        let generation = {
            let mut this = self.as_mut().rust_mut();
            this.generation += 1;
            this.generation
        };
        let model = self.qt_thread();
        loader::submit(
            Kind::Tree,
            |c| browse::read::folder_tree(&c.conn),
            move |tree| {
                let tree = tree.unwrap_or_else(|e| {
                    eprintln!("folder tree: {e:#}");
                    FolderTree::default()
                });
                // Fails only when the model is gone (the window closing).
                let _ = model.queue(move |model| model.apply(generation, tree));
            },
        );
    }

    fn apply(mut self: Pin<&mut Self>, generation: u64, tree: FolderTree) {
        if generation != self.generation {
            return;
        }
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().replace(tree);
        self.as_mut().end_reset_model();
        self.as_mut().set_ready(true);
        pg_trace::pg_stage_tree_shown();
        self.loaded();
    }

    fn toggle(mut self: Pin<&mut Self>, row: i32) {
        let Some(Some(n)) = self.node(row) else { return };
        if !self.tree.has_children(n) {
            return;
        }
        // The rows shown under `n` when it is expanded (its children's own expansion is remembered).
        let mut below = Vec::new();
        self.visible_in(n + 1..self.tree.subtree_end(n), &mut below);
        let count = below.len() as i32;
        let root = QModelIndex::default();
        if self.expanded[n] {
            self.as_mut().begin_remove_rows(&root, row + 1, row + count);
            {
                let mut this = self.as_mut().rust_mut();
                this.expanded[n] = false;
                this.visible.drain(row as usize + 1..(row + 1 + count) as usize);
            }
            self.as_mut().end_remove_rows();
        } else {
            self.as_mut().begin_insert_rows(&root, row + 1, row + count);
            {
                let mut this = self.as_mut().rust_mut();
                this.expanded[n] = true;
                let at = row as usize + 1;
                this.visible.splice(at..at, below);
            }
            self.as_mut().end_insert_rows();
        }
        let index = self.index(row, 0, &root);
        self.data_changed(&index, &index);
    }

    fn merged_path(&self, row: i32) -> QString {
        match self.node(row) {
            Some(Some(n)) => QString::from(self.tree.merged_path(n)),
            _ => QString::default(),
        }
    }

    fn reveal(mut self: Pin<&mut Self>, merged: &QString) -> i32 {
        let merged = merged.to_string();
        if let Some(row) = self.row_of_path(&merged) {
            return row as i32;
        }
        let Some(n) = self.find(&merged) else { return -1 };
        self.as_mut().begin_reset_model();
        {
            let mut this = self.as_mut().rust_mut();
            let mut above = this.tree.parent(n);
            while let Some(p) = above {
                this.expanded[p] = true;
                above = this.tree.parent(p);
            }
            this.show_all();
        }
        self.as_mut().end_reset_model();
        self.row_of_path(&merged).map_or(-1, |r| r as i32)
    }

    fn contains(&self, merged: &QString) -> bool {
        let merged = merged.to_string();
        merged.is_empty() || self.find(&merged).is_some()
    }

    fn row_of(&self, merged: &QString) -> i32 {
        self.row_of_path(&merged.to_string()).map_or(-1, |r| r as i32)
    }
}
