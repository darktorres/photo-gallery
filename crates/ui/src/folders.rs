//! The folder tree (merged across drives, `pg_core::browse::folder_tree`) as a flat list model: each visible
//! node is a row with its depth; expanding a node inserts its visible descendants below it. Row 0 is "All".

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
        type FolderModel = super::FolderRust;
    }

    unsafe extern "RustQt" {
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

        /// Re-read the tree from the catalog (after a scan).
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
    }
}

use core::pin::Pin;
use std::collections::BTreeSet;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_core::browse::{self, FolderTree};

use crate::state;

const ROLE_NAME: i32 = 256;
const ROLE_DEPTH: i32 = 257;
const ROLE_EXPANDABLE: i32 = 258;
const ROLE_EXPANDED: i32 = 259;
const ROLE_TOTAL: i32 = 260;
const ROLE_DRIVES: i32 = 261;
const ROLE_MERGED: i32 = 262;

/// A visible row: `None` is the "All" row.
type Visible = Option<usize>;

pub struct FolderRust {
    tree: FolderTree,
    visible: Vec<Visible>,
    expanded: BTreeSet<usize>,
}

impl Default for FolderRust {
    fn default() -> Self {
        let mut this = FolderRust {
            tree: FolderTree { nodes: Vec::new(), roots: Vec::new() },
            visible: Vec::new(),
            expanded: BTreeSet::new(),
        };
        this.load();
        this
    }
}

impl FolderRust {
    fn load(&mut self) {
        // Expanded folders stay expanded across reloads (the background job reloads after every scan).
        let keep: BTreeSet<String> = self.expanded.iter().map(|&n| self.tree.nodes[n].merged_path.clone()).collect();
        self.tree = state::with_catalog(|c| browse::folder_tree(&c.conn)).unwrap_or_else(|e| {
            eprintln!("folder tree: {e:#}");
            FolderTree { nodes: Vec::new(), roots: Vec::new() }
        });
        self.expanded =
            (0..self.tree.nodes.len()).filter(|&n| keep.contains(&self.tree.nodes[n].merged_path)).collect();
        // Otherwise open single-child chains from the top (D:\PhotoGallery\datasets\pinterest shows its contents
        // at once).
        if self.expanded.is_empty() {
            let mut level = self.tree.roots.clone();
            while level.len() == 1 {
                self.expanded.insert(level[0]);
                level = self.tree.nodes[level[0]].children.clone();
            }
        }
        let roots = self.tree.roots.clone();
        self.visible = std::iter::once(None).chain(self.visible_below(&roots)).collect();
    }

    /// `nodes` and, for the expanded ones, their visible descendants, in display order.
    fn visible_below(&self, nodes: &[usize]) -> Vec<Visible> {
        let mut out = Vec::new();
        for &n in nodes {
            out.push(Some(n));
            if self.expanded.contains(&n) {
                out.extend(self.visible_below(&self.tree.nodes[n].children));
            }
        }
        out
    }

    fn node(&self, row: i32) -> Option<Visible> {
        usize::try_from(row).ok().and_then(|r| self.visible.get(r)).copied()
    }
}

impl qobject::FolderModel {
    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(v) = self.node(index.row()) else { return QVariant::default() };
        let Some(n) = v else {
            let total: i64 = self.tree.roots.iter().map(|&r| self.tree.nodes[r].total).sum();
            return match role {
                ROLE_NAME => QVariant::from(&QString::from("All folders")),
                ROLE_DEPTH => QVariant::from(&0i32),
                ROLE_EXPANDABLE | ROLE_EXPANDED => QVariant::from(&false),
                ROLE_TOTAL => QVariant::from(&total),
                ROLE_DRIVES => QVariant::from(&QString::default()),
                ROLE_MERGED => QVariant::from(&false),
                _ => QVariant::default(),
            };
        };
        let node = &self.tree.nodes[n];
        match role {
            ROLE_NAME => QVariant::from(&QString::from(&node.name)),
            ROLE_DEPTH => QVariant::from(&(node.depth as i32)),
            ROLE_EXPANDABLE => QVariant::from(&!node.children.is_empty()),
            ROLE_EXPANDED => QVariant::from(&self.expanded.contains(&n)),
            ROLE_TOTAL => QVariant::from(&node.total),
            ROLE_DRIVES => QVariant::from(&QString::from(&node.drives)),
            // On more than one drive.
            ROLE_MERGED => QVariant::from(&node.drives.contains(' ')),
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
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().load();
        self.end_reset_model();
    }

    fn toggle(mut self: Pin<&mut Self>, row: i32) {
        let Some(Some(n)) = self.node(row) else { return };
        if self.tree.nodes[n].children.is_empty() {
            return;
        }
        // The rows shown under `n` when it is expanded (its children's own expansion is remembered).
        let below = self.visible_below(&self.tree.nodes[n].children);
        let count = below.len() as i32;
        let root = QModelIndex::default();
        if self.expanded.contains(&n) {
            self.as_mut().begin_remove_rows(&root, row + 1, row + count);
            {
                let mut this = self.as_mut().rust_mut();
                this.expanded.remove(&n);
                this.visible.drain(row as usize + 1..(row + 1 + count) as usize);
            }
            self.as_mut().end_remove_rows();
        } else {
            self.as_mut().begin_insert_rows(&root, row + 1, row + count);
            {
                let mut this = self.as_mut().rust_mut();
                this.expanded.insert(n);
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
            Some(Some(n)) => QString::from(&self.tree.nodes[n].merged_path),
            _ => QString::default(),
        }
    }

    fn reveal(mut self: Pin<&mut Self>, merged: &QString) -> i32 {
        let row = self.row_of(merged);
        if row >= 0 {
            return row;
        }
        let target = merged.to_string().to_lowercase();
        let above: Vec<usize> = (0..self.tree.nodes.len())
            .filter(|&n| {
                let path = self.tree.nodes[n].merged_path.to_lowercase();
                target.strip_prefix(&path).is_some_and(|rest| rest.starts_with('\\'))
            })
            .collect();
        if above.is_empty() {
            return -1;
        }
        self.as_mut().begin_reset_model();
        {
            let mut this = self.as_mut().rust_mut();
            this.expanded.extend(above);
            let roots = this.tree.roots.clone();
            this.visible = std::iter::once(None).chain(this.visible_below(&roots)).collect();
        }
        self.as_mut().end_reset_model();
        self.row_of(merged)
    }

    fn row_of(&self, merged: &QString) -> i32 {
        let merged = merged.to_string();
        self.visible
            .iter()
            .position(|v| v.map_or(merged.is_empty(), |n| self.tree.nodes[n].merged_path == merged))
            .map_or(-1, |r| r as i32)
    }
}
