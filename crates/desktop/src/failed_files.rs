//! What an inbox entry of a ledger lists when it is expanded (InboxPanel.qml, under the entry): a pass's failed files,
//! newest first, each with its folder and name, its error, when it first failed and its id for Reveal; or the folders
//! the scans couldn't take, grouped by the library folder that holds them, each with why. Read off the window's thread,
//! through the window's reader (`pg_app::Session::failed_files`): the list says it is loading until the read is
//! delivered, and shows the read's error when it fails. The listing has only the entries; the counts of the one shown
//! come with its read.

pub(crate) mod state;

use crate::failed_files_bridge::qobject;

use core::pin::Pin;
use std::cell::RefCell;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_app::{Failed, FailedItems};
use pg_client::text;
use pg_model::notice::LedgerKind;
use pg_model::rules::Path as RulePath;

use crate::app;
use state::Shown;

/// The most rows an entry lists.
const SHOWN: usize = 200;

const ROLE_PATH: i32 = 256;
const ROLE_ERROR: i32 = 257;
const ROLE_FILE_ID: i32 = 258;
const ROLE_FIRST_SEEN: i32 = 259;
const ROLE_NEW: i32 = 260;
const ROLE_RETRIED: i32 = 261;
const ROLE_ROOT: i32 = 262;
const ROLE_REASON: i32 = 263;
const ROLE_IS_FOLDER: i32 = 264;

thread_local! {
    /// The folders the list shows, row for row (none while it shows files): what a folder's Reveal opens the parent of
    /// (`Shell::reveal_folder`), which reaches the list only through this. Only the GUI thread touches it.
    static FOLDERS: RefCell<Vec<RulePath>> = const { RefCell::new(Vec::new()) };
}

/// The folder the list shows at `row`, if it shows folders.
pub(crate) fn shown_folder(row: usize) -> Option<RulePath> {
    FOLDERS.with(|folders| folders.borrow().get(row).cloned())
}

/// A ledger's code in QML (a `Notices` row's `pass`, a bit of `Notices.passes`, `Library.failedPass`, `--inbox-expand`):
/// its place in [`LedgerKind::ALL`], -1 for none. A pass's ledger has the pass's place in `FilePass::ALL`, which
/// `LedgerKind::ALL` lists first.
pub(crate) fn kind_code(kind: Option<LedgerKind>) -> i32 {
    kind.and_then(|k| LedgerKind::ALL.iter().position(|&q| q == k)).and_then(|i| i32::try_from(i).ok()).unwrap_or(-1)
}

/// The ledger of a QML code ([`kind_code`]).
pub(crate) fn kind_of(code: i32) -> Option<LedgerKind> {
    usize::try_from(code).ok().and_then(|i| LedgerKind::ALL.get(i)).copied()
}

/// The rows of a delivered read.
fn rows_of(items: FailedItems) -> Vec<Shown> {
    match items {
        FailedItems::Files(files) => files.into_iter().map(Shown::File).collect(),
        FailedItems::Folders(folders) => folders.into_iter().map(Shown::Folder).collect(),
    }
}

/// A count as QML takes it.
fn qt_int(n: impl TryInto<i32>) -> i32 {
    n.try_into().unwrap_or(i32::MAX)
}

thread_local! {
    /// Whether the list is being read (the Qt thread's): the screenshot's settle check waits for it (`loading`).
    static LOADING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the expanded ledger's list is still being read.
pub(crate) fn loading() -> bool {
    LOADING.get()
}

impl qobject::FailedFiles {
    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row()).ok().and_then(|row| self.rows.get(row)) else {
            return QVariant::default();
        };
        let rules = pg_platform::native::PATH_RULES;
        match (row, role) {
            (Shown::File(file), ROLE_PATH) => QVariant::from(&QString::from(&rules.display(&file.path))),
            (Shown::Folder(folder), ROLE_PATH) => QVariant::from(&QString::from(&rules.display(&folder.path))),
            (Shown::File(file), ROLE_ERROR) => QVariant::from(&QString::from(&file.error)),
            (Shown::Folder(folder), ROLE_ERROR) => QVariant::from(&QString::from(&folder.error)),
            // A number QML holds exactly (ids are far below 2^53); a folder has none (-1).
            (Shown::File(file), ROLE_FILE_ID) => QVariant::from(&pg_model::num::signed(file.file_id.get())),
            (Shown::Folder(_), ROLE_FILE_ID) => QVariant::from(&-1.0_f64),
            (Shown::File(file), ROLE_FIRST_SEEN) => QVariant::from(&pg_model::num::signed(file.first_seen)),
            (Shown::Folder(folder), ROLE_FIRST_SEEN) => QVariant::from(&pg_model::num::signed(folder.first_seen)),
            (Shown::File(file), ROLE_NEW) => QVariant::from(&file.new),
            (Shown::Folder(folder), ROLE_NEW) => QVariant::from(&folder.new),
            // The pass tries it again by itself; else it waits for a change, or Retry. A folder is walked again by the
            // next scan of it.
            (Shown::File(file), ROLE_RETRIED) => QVariant::from(&file.retry),
            (Shown::Folder(_), ROLE_RETRIED) => QVariant::from(&true),
            // The library folder a folder is listed under (the list's sections); none for a file.
            (Shown::File(_), ROLE_ROOT | ROLE_REASON) => QVariant::from(&QString::default()),
            (Shown::Folder(folder), ROLE_ROOT) => QVariant::from(&QString::from(&rules.display(&folder.root_path))),
            (Shown::Folder(folder), ROLE_REASON) => {
                QVariant::from(&QString::from(&text::failed_folder_reason(folder.failure.code())))
            }
            (row, ROLE_IS_FOLDER) => QVariant::from(&matches!(row, Shown::Folder(_))),
            _ => QVariant::default(),
        }
    }

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (ROLE_PATH, "path"),
            (ROLE_ERROR, "error"),
            (ROLE_FILE_ID, "fileId"),
            (ROLE_FIRST_SEEN, "firstSeen"),
            (ROLE_NEW, "isNew"),
            (ROLE_RETRIED, "retried"),
            (ROLE_ROOT, "root"),
            (ROLE_REASON, "reason"),
            (ROLE_IS_FOLDER, "isFolder"),
        ] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        qt_int(self.rows.len())
    }

    /// Ask for the list of the ledger `code` ([`kind_code`]; -1 shows none). Another ledger's rows go at once; the same
    /// ledger's (asked again because its newest failure changed) stay until the new read is delivered.
    pub(crate) fn show(mut self: Pin<&mut Self>, code: i32) {
        let kind = kind_of(code);
        let asked = self.asked + 1;
        self.as_mut().rust_mut().asked = asked;
        if kind != self.kind {
            self.as_mut().rust_mut().kind = kind;
            self.as_mut().put(Vec::new(), 0);
        }
        self.as_mut().put_error(QString::default());
        let Some(kind) = kind else {
            self.put_loading(false);
            return;
        };
        self.as_mut().put_loading(true);
        let list = self.qt_thread();
        let seen_to = crate::notices::seen_at_open(kind);
        app::get().reading().session().failed_files(kind, SHOWN, seen_to, move |read| {
            // Fails only when the window is gone: nothing left to show it in.
            let _ = list.queue(move |list| list.delivered(asked, read));
        });
    }

    /// A read `show` asked for, on the Qt thread: shown, unless a later `show` asked for another since.
    fn delivered(mut self: Pin<&mut Self>, asked: u64, read: pg_app::Result<Failed>) {
        if asked != self.asked {
            return;
        }
        self.as_mut().put_loading(false);
        match read {
            Ok(Failed { kind, items, counts }) => {
                if Some(kind) != self.kind {
                    return;
                }
                self.put(rows_of(items), counts.total);
            }
            Err(e) => {
                pg_log::warn("inbox", &format!("the ledger's list couldn't be read: {e}"));
                // The error alone: InboxPanel words it (`ui-inbox-unreadable`).
                self.put_error(QString::from(&app::error_text(&e)));
            }
        }
    }

    fn put_loading(mut self: Pin<&mut Self>, loading: bool) {
        LOADING.set(loading);
        if self.loading != loading {
            self.as_mut().rust_mut().loading = loading;
            self.loading_changed();
        }
    }

    fn put_error(mut self: Pin<&mut Self>, error: QString) {
        if self.error != error {
            self.as_mut().rust_mut().error = error;
            self.error_changed();
        }
    }

    /// Show `rows`, of the `total` the ledger holds.
    fn put(mut self: Pin<&mut Self>, rows: Vec<Shown>, total: u64) {
        let folders = rows
            .iter()
            .filter_map(|row| match row {
                Shown::Folder(folder) => Some(folder.path.clone()),
                Shown::File(_) => None,
            })
            .collect();
        FOLDERS.with(|shown| *shown.borrow_mut() = folders);
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = rows;
        self.as_mut().end_reset_model();
        let total = qt_int(total);
        if self.total != total {
            self.as_mut().rust_mut().total = total;
            self.total_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::notice::FilePass;

    /// The codes QML holds: a pass's ledger keeps the pass's place, the scans' folders come after, and none is -1.
    #[test]
    fn a_ledgers_code_is_its_place() {
        assert_eq!(kind_code(Some(LedgerKind::File(FilePass::Thumbs))), 0);
        assert_eq!(kind_code(Some(LedgerKind::File(FilePass::Import))), 2);
        assert_eq!(kind_code(Some(LedgerKind::ScanFolders)), 3);
        assert_eq!(kind_code(None), -1);
        for kind in LedgerKind::ALL {
            assert_eq!(kind_of(kind_code(Some(kind))), Some(kind));
        }
        assert_eq!(kind_of(-1), None);
        assert_eq!(kind_of(99), None);
        for (i, pass) in FilePass::ALL.into_iter().enumerate() {
            assert_eq!(kind_code(Some(LedgerKind::File(pass))), i32::try_from(i).unwrap());
        }
    }
}
