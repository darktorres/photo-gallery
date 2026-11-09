//! The folder pickers: the folder the user picks for a library folder, for the copies of cropped photos, or for a Yandex
//! save. On Windows the system's dialog opens (`pg_platform::picker`); on the other systems the app's own does
//! (`FolderPickerDialog.qml`, over the folders listed here). Either way the chosen folder is kept here, exact, and the
//! screen only learns that one was chosen for a purpose (`done`): the action that purpose names takes it (`take`). So
//! the path never crosses to QML as text.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("pg-desktop/cpp/picker.h");
        /// The app's window's native handle (its HWND on Windows), 0 when none is shown: the folder dialog's owner.
        #[namespace = "pg"]
        fn main_window_handle() -> usize;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// Whether the system's dialog is used (Windows). Otherwise `openInApp` asks QML to show the app's own dialog.
        #[qproperty(bool, native, READ, FINAL)]
        /// The app's dialog: the folders of the folder it is in (display names, in order).
        #[qproperty(QStringList, names, READ, WRITE, NOTIFY, FINAL)]
        /// The app's dialog: the folder it is in (display text).
        #[qproperty(QString, here, READ, WRITE, NOTIFY, FINAL)]
        type FolderPicker = super::PickerRust;
    }

    unsafe extern "RustQt" {
        /// A folder was chosen for `purpose` (see `Purpose`): the action takes it.
        #[qsignal]
        fn done(self: Pin<&mut FolderPicker>, purpose: i32);

        /// The app's own dialog should open for `purpose` (not on Windows, where the system's does).
        #[qsignal]
        #[cxx_name = "openInApp"]
        fn open_in_app(self: Pin<&mut FolderPicker>, purpose: i32);

        /// A folder dialog couldn't open, or couldn't give or list a folder, for `purpose`: `message` says why, and
        /// nothing was chosen.
        #[qsignal]
        fn failed(self: Pin<&mut FolderPicker>, purpose: i32, message: QString);

        /// Pick a folder for `purpose`: the system's dialog, or the app's (`openInApp`).
        #[qinvokable]
        fn pick(self: Pin<&mut FolderPicker>, purpose: i32);

        /// The folder from the command line for `purpose` (`--add-folder=…` for 0, `--save-crops=…` for 1), chosen as the
        /// dialog would choose it (`done`), with no dialog.
        #[qinvokable]
        #[cxx_name = "chooseCommandLine"]
        fn choose_command_line(self: Pin<&mut FolderPicker>, purpose: i32);

        /// The app's dialog: open the listed folder at `row`.
        #[qinvokable]
        fn enter(self: Pin<&mut FolderPicker>, row: i32);

        /// The app's dialog: the folder above the one it is in.
        #[qinvokable]
        fn up(self: Pin<&mut FolderPicker>);

        /// The app's dialog: choose the listed folder at `row`.
        #[qinvokable]
        fn choose(self: Pin<&mut FolderPicker>, row: i32);

        /// The app's dialog: choose the folder it is in.
        #[qinvokable]
        #[cxx_name = "chooseHere"]
        fn choose_here(self: Pin<&mut FolderPicker>);

        /// The app's dialog: closed without a choice.
        #[qinvokable]
        fn cancel(self: Pin<&mut FolderPicker>);
    }
}

use core::pin::Pin;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

use cxx_qt_lib::{QString, QStringList};
use pg_client::text;

/// What a chosen folder is for. The code is what QML passes (`pick`, `done`, `openInApp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A folder added to the library.
    AddFolder,
    /// The folder the cropped copies are saved into.
    SaveCrops,
    /// The folder the Yandex pictures are saved into.
    YandexSave,
}

impl Purpose {
    fn code(self) -> i32 {
        match self {
            Purpose::AddFolder => 0,
            Purpose::SaveCrops => 1,
            Purpose::YandexSave => 2,
        }
    }

    fn from_code(code: i32) -> Option<Purpose> {
        [Purpose::AddFolder, Purpose::SaveCrops, Purpose::YandexSave].into_iter().find(|p| p.code() == code)
    }
}

/// The folder chosen and not yet taken, with its purpose.
type Chosen = (Purpose, PathBuf);

/// The app's dialog: the purpose, the folder it is in, and the folders in it, in order.
struct Listing {
    purpose: Purpose,
    here: PathBuf,
    folders: Vec<PathBuf>,
}

thread_local! {
    // GUI thread only, like the other Qt-facing state (`crate::controller`).
    static CHOSEN: RefCell<Option<Chosen>> = const { RefCell::new(None) };
    static LISTING: RefCell<Option<Listing>> = const { RefCell::new(None) };
    static START: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// The folder chosen for `purpose`, taken (it is then none). None when no folder was chosen for it.
pub fn take(purpose: Purpose) -> Option<PathBuf> {
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

/// Set the folder the next dialog opens in (taken by it), for a screen that knows one (the Yandex save's: the folder
/// saved into last, or the photo's). Without one, the dialog opens in the user's folder.
pub fn set_start(path: Option<PathBuf>) {
    START.with(|start| *start.borrow_mut() = path);
}

/// The folder the dialog opens in: the one set for it (`set_start`, taken) while it is still a folder, else the user's
/// folder. The folder is given in its real form (`canonicalize`), which the shell takes where the path as given may
/// not be: it refuses (`0x80070057`) a `.` or `..` part, a doubled separator, and a part that is only spaces. A folder
/// under a volume with no drive letter canonicalizes to a volume path the shell refuses, so its path as given is kept.
fn start_for() -> Option<PathBuf> {
    pg_platform::picker::start_folder(START.with(|start| start.borrow_mut().take()))
}

/// The folders in `dir`, by name (a folder's name is kept exactly: the path is the listing's own). An error when `dir`
/// can't be read, or one of its entries can't.
fn folders_in(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut folders = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            folders.push(entry.path());
        }
    }
    folders.sort();
    Ok(folders)
}

/// The display names of the listing's folders (the names as text: a dialog shows them, and the path is kept apart).
fn names_of(folders: &[PathBuf]) -> QStringList {
    folders.iter().map(|p| QString::from(p.file_name().unwrap_or_default().to_string_lossy().as_ref())).collect()
}

/// The window the system's dialog belongs to: the app's (`main_window_handle`), None when no window is shown.
fn owner() -> Option<usize> {
    Some(qobject::main_window_handle()).filter(|handle| *handle != 0)
}

/// The system's dialog for `purpose`: the folder it chose, with the purpose (None when the user cancelled).
#[cfg(windows)]
fn native(purpose: Purpose, start: Option<&Path>, owner: Option<usize>) -> std::io::Result<Option<Chosen>> {
    let mut dialog = pg_platform::picker::Dialog::new()?;
    run_dialog(&mut dialog, purpose, start, owner)
}

/// The system's dialog is Windows only; the app's own picker is used on the other systems.
#[cfg(not(windows))]
fn native(_: Purpose, _: Option<&Path>, _: Option<usize>) -> std::io::Result<Option<Chosen>> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "the native folder dialog is Windows only"))
}

/// Runs `dialog` for `purpose`: the folder it chose, with the purpose. The dialog runs app code while it is open (the
/// deadlines and the events), so nothing of the picker's is borrowed across it.
#[cfg(windows)]
fn run_dialog(
    dialog: &mut dyn pg_platform::picker::FolderDialog,
    purpose: Purpose,
    start: Option<&Path>,
    owner: Option<usize>,
) -> std::io::Result<Option<Chosen>> {
    let start = pg_platform::picker::start_units(start);
    Ok(pg_platform::picker::pick_with(dialog, start.as_deref(), owner)?.map(|path| (purpose, path)))
}

pub struct PickerRust {
    native: bool,
    names: QStringList,
    here: QString,
}

impl Default for PickerRust {
    fn default() -> Self {
        PickerRust { native: native_dialog(), names: QStringList::default(), here: QString::default() }
    }
}

/// Whether the system's dialog is the one used: it is Windows' (the other systems use the app's own, `openInApp`).
fn native_dialog() -> bool {
    pg_platform::picker::NATIVE
}

impl qobject::FolderPicker {
    fn pick(mut self: Pin<&mut Self>, purpose: i32) {
        let Some(purpose) = Purpose::from_code(purpose) else { return };
        let start = start_for();
        if pg_platform::picker::NATIVE {
            // The system's dialog runs its own loop: nothing of the app is borrowed across it (`native`).
            let owner = owner();
            match native(purpose, start.as_deref(), owner) {
                Ok(Some(chosen)) => self.choose_path(chosen),
                Ok(None) => {}
                Err(e) => {
                    let call = pg_platform::picker::failed_call(&e).unwrap_or("the dialog");
                    crate::log::note(&format!(
                        "folder picker {purpose:?}: {call}: {e} (start folder {start:?}, owner window {owner:?})"
                    ));
                    let message = QString::from(&text::folder_dialog_failed(&e.to_string()));
                    self.as_mut().failed(purpose.code(), message);
                }
            }
            return;
        }
        let here = start.unwrap_or_else(|| PathBuf::from("/"));
        let folders = match folders_in(&here) {
            Ok(folders) => folders,
            Err(e) => return self.report_unlisted(purpose, &here, &e),
        };
        let names = names_of(&folders);
        self.as_mut().set_here(QString::from(here.to_string_lossy().as_ref()));
        self.as_mut().set_names(names);
        LISTING.with(|listing| *listing.borrow_mut() = Some(Listing { purpose, here, folders }));
        self.open_in_app(purpose.code());
    }

    fn choose_command_line(self: Pin<&mut Self>, purpose: i32) {
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

    fn enter(mut self: Pin<&mut Self>, row: i32) {
        let Some(next) = LISTING.with(|listing| {
            let listing = listing.borrow();
            let listing = listing.as_ref()?;
            listing.folders.get(usize::try_from(row).ok()?).cloned()
        }) else {
            return;
        };
        self.as_mut().show_folder(next);
    }

    fn up(mut self: Pin<&mut Self>) {
        let parent =
            LISTING.with(|listing| listing.borrow().as_ref().and_then(|l| l.here.parent().map(Path::to_path_buf)));
        if let Some(parent) = parent {
            self.as_mut().show_folder(parent);
        }
    }

    fn choose(self: Pin<&mut Self>, row: i32) {
        let chosen = LISTING.with(|listing| {
            let listing = listing.borrow();
            let listing = listing.as_ref()?;
            Some((listing.purpose, listing.folders.get(usize::try_from(row).ok()?)?.clone()))
        });
        if let Some(chosen) = chosen {
            self.choose_path(chosen);
        }
    }

    fn choose_here(self: Pin<&mut Self>) {
        let chosen = LISTING.with(|listing| listing.borrow().as_ref().map(|l| (l.purpose, l.here.clone())));
        if let Some(chosen) = chosen {
            self.choose_path(chosen);
        }
    }

    fn cancel(self: Pin<&mut Self>) {
        LISTING.with(|listing| *listing.borrow_mut() = None);
    }

    /// The app's dialog can't list `folder` (`error`): the purpose's picker says so (`failed`), and keeps what it
    /// showed.
    fn report_unlisted(self: Pin<&mut Self>, purpose: Purpose, folder: &Path, error: &std::io::Error) {
        crate::log::note(&format!("folder picker: {}: {error}", folder.display()));
        let message = QString::from(&text::folder_unreadable(&error.to_string()));
        self.failed(purpose.code(), message);
    }

    /// The app's dialog now shows `folder` (and its folders).
    fn show_folder(mut self: Pin<&mut Self>, folder: PathBuf) {
        let folders = match folders_in(&folder) {
            Ok(folders) => folders,
            Err(e) => {
                if let Some(purpose) = LISTING.with(|listing| listing.borrow().as_ref().map(|l| l.purpose)) {
                    self.as_mut().report_unlisted(purpose, &folder, &e);
                }
                return;
            }
        };
        let names = names_of(&folders);
        self.as_mut().set_here(QString::from(folder.to_string_lossy().as_ref()));
        self.as_mut().set_names(names);
        LISTING.with(|listing| {
            if let Some(listing) = listing.borrow_mut().as_mut() {
                listing.here = folder;
                listing.folders = folders;
            }
        });
    }

    /// `chosen` is the folder for its purpose: kept for the action, which is told by `done`. The app's dialog is closed.
    fn choose_path(mut self: Pin<&mut Self>, chosen: Chosen) {
        let purpose = chosen.0;
        CHOSEN.with(|slot| *slot.borrow_mut() = Some(chosen));
        LISTING.with(|listing| *listing.borrow_mut() = None);
        self.as_mut().set_names(QStringList::default());
        self.done(purpose.code());
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Component, Path, PathBuf};

    use super::{folders_in, set_start, start_for};

    /// A start folder given with a `..` part is opened in its real form: the shell refuses the path as it is.
    /// A path under a volume with no drive letter is not a drive or share, so the start folder keeps its path as given;
    /// the drive and share forms are.
    #[cfg(windows)]
    #[test]
    fn only_a_drive_or_a_share_is_given_in_its_canonical_form() {
        use pg_platform::picker::has_drive_or_share;
        assert!(has_drive_or_share(Path::new(r"\\?\C:\photos")));
        assert!(has_drive_or_share(Path::new(r"C:\photos")));
        assert!(has_drive_or_share(Path::new(r"\\?\UNC\server\share\a")));
        assert!(has_drive_or_share(Path::new(r"\\server\share\a")));
        assert!(!has_drive_or_share(Path::new(r"\\?\Volume{38788d0c-38dd-46ca-b547-211e9c17f3ac}\photos")));
    }

    #[test]
    fn a_start_folder_is_given_in_its_real_form() {
        let tree = Tree::new("start");
        std::fs::create_dir(tree.0.join("inner")).unwrap();
        set_start(Some(tree.0.join("inner").join("..")));
        let got = start_for().expect("the start folder");
        assert_eq!(got, std::fs::canonicalize(&tree.0).unwrap());
        assert!(!got.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)));
    }

    /// A folder whose name is no UTF-8: a lone surrogate on Windows, a raw `0xFF` byte on Unix.
    #[cfg(windows)]
    fn odd_name() -> PathBuf {
        pg_platform::names::path_buf(b"odd\xED\xA0\x80")
    }

    #[cfg(unix)]
    fn odd_name() -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(b"odd\xFF"))
    }

    /// A temporary folder, removed when dropped.
    struct Tree(PathBuf);

    impl Tree {
        fn new(tag: &str) -> Tree {
            let root = std::env::temp_dir().join(format!("pg-picker-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Tree(root)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_folders_listed_are_the_folders_by_their_exact_names() {
        let tree = Tree::new("listing");
        let odd = tree.0.join(odd_name());
        std::fs::create_dir(&odd).unwrap();
        std::fs::create_dir(tree.0.join("Beta")).unwrap();
        std::fs::create_dir(tree.0.join("alpha")).unwrap();
        std::fs::write(tree.0.join("a file.txt"), b"x").unwrap();
        let mut want = vec![tree.0.join("Beta"), tree.0.join("alpha"), odd];
        want.sort();
        assert_eq!(folders_in(&tree.0).unwrap(), want, "folders only, each by its exact path");
    }

    #[test]
    fn a_folder_that_is_gone_is_an_error_not_an_empty_listing() {
        let error = folders_in(Path::new("no-such-folder-for-the-picker-test")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }
}

#[cfg(all(test, windows))]
mod dialog_tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::PathBuf;

    use pg_platform::picker::{FolderDialog, ShellItem};

    use super::{CHOSEN, LISTING, Purpose, START, run_dialog};

    /// What the dialog was opened with: its start folder's units and its owner.
    type Opened = (Option<Vec<u16>>, Option<usize>);

    /// A stand-in for the system's dialog: while it is open it uses the picker's own state (as a deadline or an event
    /// would, through a mutable borrow), then answers with a folder whose name has a lone surrogate.
    struct Fake {
        opened: RefCell<Option<Opened>>,
    }

    struct Units(Vec<u16>);

    impl ShellItem for Units {
        fn file_system_path(&self) -> io::Result<Vec<u16>> {
            Ok(self.0.clone())
        }
    }

    impl FolderDialog for Fake {
        fn run(&mut self, start: Option<&[u16]>, owner: Option<usize>) -> io::Result<bool> {
            // A borrow held across the dialog by the picker would make these panic.
            CHOSEN.with(|c| drop(c.borrow_mut()));
            LISTING.with(|l| drop(l.borrow_mut()));
            START.with(|s| drop(s.borrow_mut()));
            *self.opened.borrow_mut() = Some((start.map(<[u16]>::to_vec), owner));
            Ok(true)
        }

        fn chosen(&self) -> io::Result<Option<Box<dyn ShellItem>>> {
            Ok(Some(Box::new(Units(vec![0x43, 0x3A, 0x5C, 0x61, 0xD800]))))
        }
    }

    #[test]
    fn the_dialog_may_use_the_pickers_state_while_it_is_open() {
        let mut fake = Fake { opened: RefCell::new(None) };
        let start = PathBuf::from(r"C:\a");
        let chosen = run_dialog(&mut fake, Purpose::SaveCrops, Some(&start), Some(0x40)).unwrap();
        let path = pg_platform::names::path_buf(b"C:\\a\xED\xA0\x80");
        assert_eq!(chosen, Some((Purpose::SaveCrops, path)), "the folder, exact, for its purpose");
        assert_eq!(fake.opened.into_inner(), Some((Some(vec![0x43, 0x3A, 0x5C, 0x61]), Some(0x40))));
    }
}
