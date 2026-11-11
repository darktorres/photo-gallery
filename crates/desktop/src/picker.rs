//! The folder pickers: the folder the user picks for a library folder, for the copies of cropped photos, or for a Yandex
//! save. On Windows the system's dialog opens (`pg_platform::picker`); on the other systems the app's own does
//! (`FolderPickerDialog.qml`, over the folders listed here). Either way the chosen folder is kept here, exact, and the
//! screen only learns that one was chosen for a purpose (`done`): the action that purpose names takes it (`take`). So
//! the path never crosses to QML as text.

pub(crate) mod state;
use state::{Listing, PickerRust, Purpose};

use crate::picker_bridge::qobject;

use core::pin::Pin;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};
use pg_client::text::{self, Message};
use pg_model::notice::Notice;

/// The folder chosen and not yet taken, with its purpose.
type Chosen = (Purpose, PathBuf);

thread_local! {
    // GUI thread only, like the other Qt-facing state (`crate::controller`). One picker chooses and another QObject
    // takes it (the library, the gallery's edits, the Yandex panel); cxx-qt default-constructs both, so nothing hands
    // one a reference to the other.
    static CHOSEN: RefCell<Option<Chosen>> = const { RefCell::new(None) };
}

/// The folder chosen for `purpose`, taken (it is then none). None when no folder was chosen for it.
pub(crate) fn take(purpose: Purpose) -> Option<PathBuf> {
    CHOSEN.with(|chosen| {
        let mut chosen = chosen.borrow_mut();
        match chosen.take() {
            Some((p, path)) if p == purpose => Some(path),
            other => {
                *chosen = other;
                None
            }
        }
    })
}

/// The folder the dialog for `purpose` opens in: its purpose's own (a Yandex save's is the folder saved into last, or
/// the photo's: `pg_app::yandex::Yandex::start_folder`) while it is still a folder, else the user's folder, in the form
/// the system's dialog takes (`pg_platform::picker::start_folder`).
fn start_for(purpose: Purpose) -> Option<PathBuf> {
    let requested = match purpose {
        Purpose::YandexSave => crate::app::get().yandex().start_folder(),
        Purpose::AddFolder | Purpose::SaveCrops => None,
    };
    pg_platform::picker::start_folder(requested, |name| std::env::var_os(name))
}

/// The display names of the listing's folders (the names as text: a dialog shows them, and the path is kept apart).
fn names_of(folders: &[PathBuf]) -> QStringList {
    folders.iter().map(|p| QString::from(p.file_name().unwrap_or_default().to_string_lossy().as_ref())).collect()
}

/// The window the system's dialog belongs to: the app's (`main_window_handle`), None when no window is shown.
fn owner() -> Option<usize> {
    Some(qobject::main_window_handle()).filter(|handle| *handle != 0)
}

impl Default for PickerRust {
    fn default() -> Self {
        PickerRust { native: native_dialog(), names: QStringList::default(), here: QString::default(), listing: None }
    }
}

/// Whether the system's dialog is the one used: it is Windows' (the other systems use the app's own, `openInApp`).
fn native_dialog() -> bool {
    pg_platform::picker::NATIVE
}

impl qobject::FolderPicker {
    pub(crate) fn pick(mut self: Pin<&mut Self>, purpose: i32) {
        let Some(purpose) = Purpose::from_code(purpose) else { return };
        let start = start_for(purpose);
        if pg_platform::picker::NATIVE {
            // The system's dialog runs its own loop, which runs app code (the deadlines and the events): nothing of the
            // picker's is borrowed across it.
            let owner = owner();
            match pg_platform::picker::pick(start.as_deref(), owner) {
                Ok(Some(path)) => self.choose_path((purpose, path)),
                Ok(None) => {}
                Err(e) => {
                    let call = pg_platform::picker::failed_call(&e).unwrap_or("the dialog");
                    pg_log::warn(
                        "picker",
                        &format!("{purpose:?}: {call}: {e} (start folder {start:?}, owner window {owner:?})"),
                    );
                    crate::app::post(Notice::PickerFailed { error: crate::app::error_text(&e) });
                    let message = QString::from(&text::render(&Message::FolderDialogFailed { error: e.to_string() }));
                    self.as_mut().failed(purpose.code(), message);
                }
            }
            return;
        }
        let here = start.unwrap_or_else(|| PathBuf::from("/"));
        let folders = match pg_platform::dir::folders_in(&here) {
            Ok(folders) => folders,
            Err(e) => return self.report_unlisted(purpose, &here, &e),
        };
        let names = names_of(&folders);
        self.as_mut().set_here(QString::from(here.to_string_lossy().as_ref()));
        self.as_mut().set_names(names);
        self.as_mut().rust_mut().listing = Some(Listing { purpose, here, folders });
        self.open_in_app(purpose.code());
    }

    pub(crate) fn choose_command_line(self: Pin<&mut Self>, purpose: i32) {
        let Some(purpose) = Purpose::from_code(purpose) else { return };
        let name = match purpose {
            Purpose::AddFolder => "add-folder",
            Purpose::SaveCrops => "save-crops",
            Purpose::YandexSave => return,
        };
        if let Some(path) = crate::command_line::path(name) {
            self.choose_path((purpose, path.to_path_buf()));
        }
    }

    pub(crate) fn enter(mut self: Pin<&mut Self>, row: i32) {
        let Some(next) = self.listing.as_ref().and_then(|l| l.folders.get(usize::try_from(row).ok()?).cloned()) else {
            return;
        };
        self.as_mut().show_folder(next);
    }

    pub(crate) fn up(mut self: Pin<&mut Self>) {
        let parent = self.listing.as_ref().and_then(|l| l.here.parent().map(Path::to_path_buf));
        if let Some(parent) = parent {
            self.as_mut().show_folder(parent);
        }
    }

    pub(crate) fn choose(self: Pin<&mut Self>, row: i32) {
        let chosen =
            self.listing.as_ref().and_then(|l| Some((l.purpose, l.folders.get(usize::try_from(row).ok()?)?.clone())));
        if let Some(chosen) = chosen {
            self.choose_path(chosen);
        }
    }

    pub(crate) fn choose_here(self: Pin<&mut Self>) {
        let chosen = self.listing.as_ref().map(|l| (l.purpose, l.here.clone()));
        if let Some(chosen) = chosen {
            self.choose_path(chosen);
        }
    }

    pub(crate) fn cancel(self: Pin<&mut Self>) {
        self.rust_mut().listing = None;
    }

    /// The app's dialog can't list `folder` (`error`): the inbox and the purpose's picker say so (`failed`), and the
    /// dialog keeps what it showed.
    fn report_unlisted(self: Pin<&mut Self>, purpose: Purpose, folder: &Path, error: &std::io::Error) {
        crate::app::post(Notice::PickerFailed {
            error: crate::app::error_text(&format!("{}: {error}", folder.display())),
        });
        let message = QString::from(&text::render(&Message::FolderUnreadable { error: error.to_string() }));
        self.failed(purpose.code(), message);
    }

    /// The app's dialog now shows `folder` (and its folders).
    fn show_folder(mut self: Pin<&mut Self>, folder: PathBuf) {
        let folders = match pg_platform::dir::folders_in(&folder) {
            Ok(folders) => folders,
            Err(e) => {
                if let Some(purpose) = self.listing.as_ref().map(|l| l.purpose) {
                    self.as_mut().report_unlisted(purpose, &folder, &e);
                }
                return;
            }
        };
        let names = names_of(&folders);
        self.as_mut().set_here(QString::from(folder.to_string_lossy().as_ref()));
        self.as_mut().set_names(names);
        if let Some(listing) = self.as_mut().rust_mut().listing.as_mut() {
            listing.here = folder;
            listing.folders = folders;
        }
    }

    /// `chosen` is the folder for its purpose: kept for the action, which is told by `done`. The app's dialog is closed.
    fn choose_path(mut self: Pin<&mut Self>, chosen: Chosen) {
        let purpose = chosen.0;
        CHOSEN.with(|slot| *slot.borrow_mut() = Some(chosen));
        self.as_mut().rust_mut().listing = None;
        self.as_mut().set_names(QStringList::default());
        self.done(purpose.code());
    }
}
