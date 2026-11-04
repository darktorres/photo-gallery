//! The folder tree (merged across drives, `pg_catalog::browse::read::folder_tree`) as a flat list model: each visible
//! node is a row with its depth; expanding a node inserts its visible descendants below it. Row 0 is "All".
//!
//! The tree is read off the GUI thread (the app's catalog reader): `reload` asks, `loaded` says it's shown. Until the
//! first one, only "All" is there (`ready` is false). `restore` says which folder to show at start. Paths match as
//! Windows matches them, ignoring case: the tree keeps the case of one drive's folder, the session another's.

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

        /// The folder to show at start: the saved one when the tree has it (its merged path, as the saved one spells
        /// it), else `""` (All folders). Call it once the tree is in (`loaded`).
        #[qinvokable]
        fn restore(self: &FolderModel) -> QString;
    }
}

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_app::Kind;
use pg_catalog::browse;
use pg_client::tree::Tree;
use pg_model::tree::FolderTree;

const ROLE_NAME: i32 = 256;
const ROLE_DEPTH: i32 = 257;
const ROLE_EXPANDABLE: i32 = 258;
const ROLE_EXPANDED: i32 = 259;
const ROLE_TOTAL: i32 = 260;
const ROLE_DRIVES: i32 = 261;
const ROLE_MERGED: i32 = 262;

#[derive(Default)]
pub struct FolderRust {
    ready: bool,
    /// The folders as shown: which are expanded, and the rows that follow.
    tree: Tree,
    /// The latest tree asked for; older ones arriving are dropped.
    generation: u64,
}

impl qobject::FolderModel {
    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(v) = self.tree.row(index.row()) else { return QVariant::default() };
        let Some(n) = v else {
            return match role {
                ROLE_NAME => QVariant::from(&QString::from("All folders")),
                ROLE_DEPTH => QVariant::from(&0i32),
                ROLE_EXPANDABLE | ROLE_EXPANDED => QVariant::from(&false),
                ROLE_TOTAL => QVariant::from(&self.tree.folders().total()),
                ROLE_DRIVES => QVariant::from(&QString::default()),
                ROLE_MERGED => QVariant::from(&false),
                _ => QVariant::default(),
            };
        };
        let folders = self.tree.folders();
        let node = folders.node(n);
        match role {
            ROLE_NAME => QVariant::from(&QString::from(folders.name(n))),
            ROLE_DEPTH => QVariant::from(&(node.depth as i32)),
            ROLE_EXPANDABLE => QVariant::from(&folders.has_children(n)),
            ROLE_EXPANDED => QVariant::from(&self.tree.is_expanded(n)),
            ROLE_TOTAL => QVariant::from(&node.total),
            ROLE_DRIVES => QVariant::from(&QString::from(&folders.drives(n))),
            ROLE_MERGED => QVariant::from(&folders.merged(n)),
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
        self.tree.rows().len() as i32
    }

    fn reload(mut self: Pin<&mut Self>) {
        let generation = {
            let mut this = self.as_mut().rust_mut();
            this.generation += 1;
            this.generation
        };
        let model = self.qt_thread();
        crate::app::get().session().submit(Kind::Tree, browse::read::folder_tree, move |tree| {
            let tree = tree.unwrap_or_else(|e| {
                eprintln!("folder tree: {e:#}");
                FolderTree::default()
            });
            // Fails only when the model is gone (the window closing).
            let _ = model.queue(move |model| model.apply(generation, tree));
        });
    }

    fn apply(mut self: Pin<&mut Self>, generation: u64, tree: FolderTree) {
        if generation != self.generation {
            return;
        }
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().tree.replace(tree);
        self.as_mut().end_reset_model();
        self.as_mut().set_ready(true);
        pg_trace::pg_stage_tree_shown();
        self.loaded();
    }

    fn toggle(mut self: Pin<&mut Self>, row: i32) {
        let Ok(at) = usize::try_from(row) else { return };
        // The rows it inserts or removes are the ones the tree says, from before the change.
        let Some(plan) = self.tree.plan_toggle(at) else { return };
        let root = QModelIndex::default();
        let (first, last) = (plan.rows.start as i32, plan.rows.end as i32 - 1);
        if plan.expand {
            self.as_mut().begin_insert_rows(&root, first, last);
            self.as_mut().rust_mut().tree.toggle(at);
            self.as_mut().end_insert_rows();
        } else {
            self.as_mut().begin_remove_rows(&root, first, last);
            self.as_mut().rust_mut().tree.toggle(at);
            self.as_mut().end_remove_rows();
        }
        let index = self.index(row, 0, &root);
        self.data_changed(&index, &index);
    }

    fn merged_path(&self, row: i32) -> QString {
        match self.tree.row(row) {
            Some(Some(n)) => QString::from(self.tree.folders().merged_path(n)),
            _ => QString::default(),
        }
    }

    fn reveal(mut self: Pin<&mut Self>, merged: &QString) -> i32 {
        let merged = merged.to_string();
        if let Some(row) = self.tree.row_of_path(&merged) {
            return row as i32;
        }
        if self.tree.find(&merged).is_none() {
            return -1;
        }
        self.as_mut().begin_reset_model();
        let row = self.as_mut().rust_mut().tree.reveal(&merged);
        self.as_mut().end_reset_model();
        row.map_or(-1, |r| r as i32)
    }

    fn contains(&self, merged: &QString) -> bool {
        let merged = merged.to_string();
        merged.is_empty() || self.tree.find(&merged).is_some()
    }

    fn row_of(&self, merged: &QString) -> i32 {
        self.tree.row_of_path(&merged.to_string()).map_or(-1, |r| r as i32)
    }

    fn restore(&self) -> QString {
        QString::from(&crate::app::get().restore(self.tree.folders()).unwrap_or_default())
    }
}
