//! The folder tree (merged across drives, read by `pg_app`'s session) as a flat list model: each visible node is a row
//! with its depth; expanding a node inserts its visible descendants below it. Row 0 is "All"; row 1 is "Drive roots"
//! when files lie directly in a drive root (`pg_client::tree`). The `kind` role tells the rows apart, and QML gives
//! the first two their words.
//!
//! The tree is read off the GUI thread (the app's catalog reader): `reload` asks, `apply` shows it. Until the first one,
//! only "All" is there (`ready` is false). The tree itself is kept on the GUI thread (`TREE`, beside the controller), so
//! the rows, the reveal of the listed folder and the spelling of a row's folder are read here and never passed as text:
//! QML names a row by its number, and the folders go by value.

pub(crate) mod state;

use crate::folders_bridge::qobject;

use core::pin::Pin;
use std::cell::RefCell;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_client::tree::{Tree, Visible};
use pg_model::rules::Merged;
use pg_model::tree::FolderTree;

use crate::controller;

const ROLE_NAME: i32 = 256;
const ROLE_DEPTH: i32 = 257;
const ROLE_EXPANDABLE: i32 = 258;
const ROLE_EXPANDED: i32 = 259;
const ROLE_TOTAL: i32 = 260;
const ROLE_DRIVES: i32 = 261;
const ROLE_MERGED: i32 = 262;
const ROLE_KIND: i32 = 263;

/// The `kind` role: which of the rows it is (FolderTree.qml names the first two by their `ui.ftl` words). The shell's
/// `folderKind` uses the same numbers for the listed folder's label.
pub(crate) const KIND_ALL: i32 = 0;
pub(crate) const KIND_DRIVE_ROOTS: i32 = 1;
pub(crate) const KIND_FOLDER: i32 = 2;

/// A row as QML's int, or -1 (not found) for one past what an int can hold.
fn row_index(row: usize) -> i32 {
    i32::try_from(row).unwrap_or(-1)
}

thread_local! {
    /// The folder tree as shown: which folders are open, and the rows that follow (GUI thread only, beside the
    /// controller). Each borrow is one read or one change; none calls back into another.
    static TREE: RefCell<Tree> = RefCell::new(Tree::new(pg_platform::native::PATH_RULES));
}

/// The tree as shown, read.
fn with<T>(read: impl FnOnce(&Tree) -> T) -> T {
    TREE.with(|tree| read(&tree.borrow()))
}

/// The tree as shown, changed.
fn with_mut<T>(change: impl FnOnce(&mut Tree) -> T) -> T {
    TREE.with(|tree| change(&mut tree.borrow_mut()))
}

/// What a row shows, read off the tree as shown (the shell's click on a row: `shell::row_event`).
pub(crate) fn row_kind(row: i32) -> RowKind {
    with(|tree| RowKind::of(tree, row))
}

/// What a row shows, by value: the folder of a node, merged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    /// "All", and a row the tree doesn't have.
    All,
    /// "Drive roots".
    DriveRoots,
    Folder(Merged),
}

impl RowKind {
    pub(crate) fn of(tree: &Tree, row: i32) -> RowKind {
        match crate::rows::row_of(row).and_then(|row| tree.row(row)) {
            Some(Visible::Folder(n)) => RowKind::Folder(tree.folders().merged(n)),
            Some(Visible::DriveRoots) => RowKind::DriveRoots,
            Some(Visible::All) | None => RowKind::All,
        }
    }
}

/// The kind of the listed folder's label, as the tree names it (the top bar, the save dialog): the empty folder is
/// `KIND_DRIVE_ROOTS` or `KIND_ALL` by the tree's rule (`Tree::shown_as`), any other folder `KIND_FOLDER` (its name is
/// the label). Read off the tree as shown.
pub(crate) fn listing_kind(folder: &Merged, recursive: bool) -> i32 {
    with(|tree| listing_kind_of(tree, folder, recursive))
}

/// [`listing_kind`] of a given tree.
pub(crate) fn listing_kind_of(tree: &Tree, folder: &Merged, recursive: bool) -> i32 {
    match (folder.is_empty(), tree.shown_as(folder, recursive)) {
        (false, _) => KIND_FOLDER,
        (true, Some(Visible::DriveRoots)) => KIND_DRIVE_ROOTS,
        (true, _) => KIND_ALL,
    }
}

/// The listed folder as the tree spells it, when the tree shows it under another spelling (the start's spelled-folder
/// step: the folder is kept as the tree has it). `None` when the folder isn't shown or is spelled the same.
pub(crate) fn spelling_of(folder: &Merged, recursive: bool) -> Option<Merged> {
    with(|tree| {
        tree.row_of(folder, recursive)?;
        let n = tree.find(folder)?;
        let spelled = tree.folders().merged(n);
        (spelled != *folder).then_some(spelled)
    })
}

impl qobject::FolderModel {
    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        with(|tree| {
            let Some(v) = crate::rows::row_of(index.row()).and_then(|row| tree.row(row)) else {
                return QVariant::default();
            };
            let (kind, total) = match v {
                Visible::All => (KIND_ALL, tree.folders().total()),
                Visible::DriveRoots => (KIND_DRIVE_ROOTS, tree.folders().root_files()),
                Visible::Folder(_) => (KIND_FOLDER, 0),
            };
            let Visible::Folder(n) = v else {
                // The words of "All folders" and "Drive roots" are the QML's (`ui.ftl`), by the row's kind.
                return match role {
                    ROLE_NAME | ROLE_DRIVES => QVariant::from(&QString::default()),
                    ROLE_DEPTH => QVariant::from(&0i32),
                    ROLE_EXPANDABLE | ROLE_EXPANDED | ROLE_MERGED => QVariant::from(&false),
                    ROLE_TOTAL => QVariant::from(&total),
                    ROLE_KIND => QVariant::from(&kind),
                    _ => QVariant::default(),
                };
            };
            let folders = tree.folders();
            let Some(node) = folders.node(n) else { return QVariant::default() };
            match role {
                ROLE_KIND => QVariant::from(&kind),
                ROLE_NAME => QVariant::from(&QString::from(folders.name(n).display().as_ref())),
                ROLE_DEPTH => QVariant::from(&(i32::try_from(node.depth).unwrap_or(i32::MAX))),
                ROLE_EXPANDABLE => QVariant::from(&folders.has_children(n)),
                ROLE_EXPANDED => QVariant::from(&tree.is_expanded(n)),
                ROLE_TOTAL => QVariant::from(&node.total),
                ROLE_DRIVES => QVariant::from(&QString::from(&folders.anchors(n))),
                ROLE_MERGED => QVariant::from(&folders.multi_anchor(n)),
                _ => QVariant::default(),
            }
        })
    }

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (ROLE_NAME, "name"),
            (ROLE_DEPTH, "depth"),
            (ROLE_EXPANDABLE, "expandable"),
            (ROLE_EXPANDED, "expanded"),
            (ROLE_TOTAL, "total"),
            (ROLE_DRIVES, "drives"),
            (ROLE_MERGED, "merged"),
            (ROLE_KIND, "kind"),
        ] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        i32::try_from(with(|tree| tree.rows().len())).unwrap_or(i32::MAX)
    }

    pub(crate) fn reload(mut self: Pin<&mut Self>) {
        let generation = {
            let mut this = self.as_mut().rust_mut();
            this.generation += 1;
            this.generation
        };
        let model = self.qt_thread();
        crate::app::get().reading().session().tree(move |tree| {
            let tree = match tree {
                Ok(tree) => {
                    crate::app::resolved("tree-failed");
                    tree
                }
                Err(e) => {
                    let error = crate::app::error_text(&format!("{e:#}"));
                    crate::app::post(pg_model::notice::Notice::TreeFailed { error });
                    FolderTree::empty(pg_platform::native::PATH_RULES)
                }
            };
            // Fails only when the model is gone (the window closing).
            let _ = model.queue(move |model| model.apply(generation, tree));
        });
    }

    fn apply(mut self: Pin<&mut Self>, generation: u64, tree: FolderTree) {
        if generation != self.generation {
            return;
        }
        self.as_mut().begin_reset_model();
        with_mut(|shown| shown.replace(tree));
        self.as_mut().end_reset_model();
        self.as_mut().set_ready(true);
        pg_trace::pg_stage_tree_shown();
        self.loaded();
    }

    pub(crate) fn toggle(mut self: Pin<&mut Self>, row: i32) {
        let Ok(at) = usize::try_from(row) else { return };
        // The rows it inserts or removes are the ones the tree says, from before the change.
        let Some(plan) = with(|tree| tree.plan_toggle(at)) else { return };
        let root = QModelIndex::default();
        let (Ok(first), Ok(end)) = (i32::try_from(plan.rows.start), i32::try_from(plan.rows.end)) else { return };
        let last = end - 1;
        if plan.expand {
            self.as_mut().begin_insert_rows(&root, first, last);
            with_mut(|tree| tree.toggle(at));
            self.as_mut().end_insert_rows();
        } else {
            self.as_mut().begin_remove_rows(&root, first, last);
            with_mut(|tree| tree.toggle(at));
            self.as_mut().end_remove_rows();
        }
        let index = self.index(row, 0, &root);
        self.data_changed(&index, &index);
    }

    pub(crate) fn tree_loaded(mut self: Pin<&mut Self>) -> i32 {
        let first = !self.shown;
        self.as_mut().rust_mut().shown = true;
        self.reveal_listing(first)
    }

    pub(crate) fn reveal_listing(mut self: Pin<&mut Self>, first: bool) -> i32 {
        let listing = controller::listing();
        let (folder, recursive) = (listing.folder, listing.recursive);
        let shown = with(|tree| tree.row_of(&folder, recursive));
        let row = match shown {
            Some(row) => Some(row),
            // Opening the folders above it changes the rows: a reset, as the model's rows are read again.
            None if first && with(|tree| tree.find(&folder)).is_some() => {
                self.as_mut().begin_reset_model();
                let row = with_mut(|tree| tree.reveal(&folder, recursive));
                self.as_mut().end_reset_model();
                row
            }
            None => None,
        };
        if let Some(row) = row {
            return row_index(row);
        }
        if with(|tree| tree.find(&folder)).is_some() { -1 } else { -2 }
    }
}
