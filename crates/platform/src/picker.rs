//! The folder dialog of Windows: the Explorer's own folder picker (`IFileOpenDialog`, folders only). The folder it gives
//! comes back in the system's units, so a name with an unpaired surrogate is exact (a dialog's text would not be). On
//! the other systems the app has its own picker (the desktop's folder picker, over `dir::folders_in`), so [`pick`]
//! is `Unsupported` there ([`NATIVE`] says which).
//!
//! Qt initializes COM on the GUI thread (a single-threaded apartment), so this initializes nothing, and it is called on
//! that thread only. The dialog runs its own message loop while it is open: a caller holds no borrow of its state
//! across the call. The dialog's steps are behind [`FolderDialog`] and [`ShellItem`], so `pick_with` is tested with a
//! fake dialog; the Windows implementation is only the calls to the system.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Whether the system's own folder dialog is the one the app uses: Windows' is, the other systems have the app's own picker.
pub const NATIVE: bool = cfg!(windows);

/// Whether `path` starts with a drive or a share (`C:\`, `\\?\C:\`, `\\server\share`), the forms the shell takes.
#[cfg(windows)]
fn has_drive_or_share(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(
            prefix.kind(),
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..)
        )
    )
}

/// Every other system has no drives or shares to tell apart: the test is always met.
#[cfg(not(windows))]
fn has_drive_or_share(_: &Path) -> bool {
    true
}

/// The folder the dialog opens in, from the one requested: given in its real form (`canonicalize`), which the shell takes
/// where the path as given may not. It refuses (`0x80070057`) a `.` or `..` part, a doubled separator, and a part that is
/// only spaces; a folder under a volume with no drive letter canonicalizes to a volume path the shell refuses, so its path
/// as given is kept. The shell also normalizes every path it parses, which drops a folder's trailing dot or space, so
/// a folder with such a name (or under one) starts the dialog at its nearest ancestor without one: the shell would
/// otherwise open a sibling (`a` for `a.`), or refuse the verbatim form. Without a folder that is one now, the user's
/// own, under the environment `var` gives (`dirs::user_folder`; the callers pass `std::env::var_os`).
pub fn start_folder(requested: Option<PathBuf>, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    requested
        .map(|folder| match std::fs::canonicalize(&folder) {
            Ok(real) if has_drive_or_share(&real) => real,
            _ => folder,
        })
        .filter(|folder| folder.is_dir())
        .map(|folder| reachable_by_shell(&folder))
        .or_else(|| crate::dirs::user_folder(var))
}

/// `folder`, or its nearest ancestor whose names all survive the shell's normalization
/// (`native::check_folder_names`). Off Windows, `folder` itself.
fn reachable_by_shell(folder: &Path) -> PathBuf {
    // A name is checked as a folder of a path, so each ancestor is given one more part.
    let reachable = |at: &&Path| crate::native::check_folder_names(&at.join("_")).is_ok();
    folder.ancestors().find(reachable).unwrap_or(folder).to_path_buf()
}

/// The start folder in the shell's units, for `pick_with`.
#[cfg(windows)]
fn start_units(start: Option<&Path>) -> Option<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    start.map(|path| path.as_os_str().encode_wide().collect())
}

mod shell;
pub use shell::{FolderDialog, ShellItem};
#[cfg(windows)]
pub use shell::{path_of_units, shell_path};

/// Runs `dialog` and gives the folder it chose, exactly. None when the user cancelled. `start` is the folder the dialog
/// opens in (its units), and `owner` the window it belongs to (None for none).
#[cfg(windows)]
fn pick_with(
    dialog: &mut dyn FolderDialog,
    start: Option<&[u16]>,
    owner: Option<usize>,
) -> io::Result<Option<PathBuf>> {
    if !dialog.run(start, owner)? {
        return Ok(None);
    }
    let item = dialog.chosen()?.ok_or_else(|| io::Error::other("the dialog chose no folder"))?;
    // In the verbatim form, so std reaches the folder chosen: the plain form of `a.` names its sibling `a`.
    Ok(Some(crate::native::verbatim(&path_of_units(&item.file_system_path()?))))
}

#[cfg(windows)]
use native::Dialog;

/// The system's folder dialog, opened in `start` and owned by the window `owner` (its handle; None: unowned): the folder
/// the user chose, exactly, or None when they cancelled. It runs its own message loop while it is open, so the caller
/// holds no borrow of its state across the call. `Unsupported` off Windows, where the app's own picker is used
/// ([`NATIVE`]).
#[cfg(windows)]
pub fn pick(start: Option<&Path>, owner: Option<usize>) -> io::Result<Option<PathBuf>> {
    let mut dialog = Dialog::new()?;
    pick_with(&mut dialog, start_units(start).as_deref(), owner)
}

/// The system's folder dialog is Windows only: the app's own picker is used here ([`NATIVE`]).
#[cfg(not(windows))]
pub fn pick(_start: Option<&Path>, _owner: Option<usize>) -> io::Result<Option<PathBuf>> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "the native folder dialog is Windows only"))
}

/// The system call of the dialog that failed, for an error the dialog gave (`None` for any other error). The log names
/// it; the message the user sees is the system's text alone.
pub fn failed_call(error: &io::Error) -> Option<&'static str> {
    #[cfg(windows)]
    {
        error.get_ref()?.downcast_ref::<native::CallError>().map(|e| e.call)
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        None
    }
}

#[cfg(windows)]
mod native {
    use std::ffi::c_void;
    use std::io;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
    use windows::Win32::UI::Shell::{
        FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
        SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    use windows::Win32::UI::WindowsAndMessaging::{PM_REMOVE, PeekMessageW, PostMessageW, WM_MOUSEMOVE};
    use windows::core::{HRESULT, PCWSTR};

    use super::shell::{FolderDialog, ShellItem, shell_path};

    /// ERROR_CANCELLED, which the dialog reports when the user closes it without choosing.
    const ERROR_CANCELLED: u32 = 1223;

    /// A system call of the dialog that failed, and the system's error. Its text (the message the user sees) is the
    /// system's alone; its Debug form, which the log takes, names the call too.
    #[derive(Debug)]
    pub(super) struct CallError {
        pub(super) call: &'static str,
        error: windows::core::Error,
    }

    impl std::fmt::Display for CallError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Display::fmt(&self.error, f)
        }
    }

    impl std::error::Error for CallError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.error)
        }
    }

    pub(super) fn step(call: &'static str, error: windows::core::Error) -> io::Error {
        io::Error::other(CallError { call, error })
    }

    /// The system's folder dialog, made with folders-only options (as Qt's native dialog sets them).
    pub(super) struct Dialog {
        dialog: IFileOpenDialog,
        chosen: Option<IShellItem>,
    }

    impl Dialog {
        pub(super) fn new() -> io::Result<Dialog> {
            // SAFETY: COM is initialized on this thread by Qt (see the module's note). The dialog is released when the
            // smart pointer is dropped.
            let dialog: IFileOpenDialog = unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| step("CoCreateInstance(FileOpenDialog)", e))?;
            // SAFETY: plain calls on the dialog made just above.
            unsafe {
                let options = dialog.GetOptions().map_err(|e| step("GetOptions", e))?;
                dialog
                    .SetOptions(options | FOS_PICKFOLDERS | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM)
                    .map_err(|e| step("SetOptions", e))?;
            }
            Ok(Dialog { dialog, chosen: None })
        }
    }

    /// The shell item of the start folder, from its path in the form the shell takes (`shell_path`): None when there
    /// is no path, and an error when the shell refuses it.
    fn start_item(start: &[u16]) -> io::Result<Option<IShellItem>> {
        let mut units = shell_path(start);
        if units.is_empty() {
            return Ok(None);
        }
        units.push(0);
        // SAFETY: the units are NUL-terminated for the call, and the item is returned owned.
        let item: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(units.as_ptr()), None) }
            .map_err(|e| step("SHCreateItemFromParsingName", e))?;
        Ok(Some(item))
    }

    impl FolderDialog for Dialog {
        fn run(&mut self, start: Option<&[u16]>, owner: Option<usize>) -> io::Result<bool> {
            // A start folder the shell refuses is an error, which the picker shows (the desktop picker passes only a
            // folder that is there).
            if let Some(item) = start.map(start_item).transpose()?.flatten() {
                // SAFETY: a plain call on the dialog and the item made just above.
                unsafe { self.dialog.SetFolder(&item) }.map_err(|e| step("SetFolder", e))?;
            }
            // SAFETY: the owner is the window's handle, given to the dialog as the window system gave it.
            unsafe {
                // The handle is an address the window system gave (the app's window's HWND), not a pointer of ours.
                let owner = owner.map(|handle| HWND(std::ptr::with_exposed_provenance_mut::<c_void>(handle)));
                // The dialog runs its own message loop here; the caller holds nothing of the app across it.
                let shown = self.dialog.Show(owner);
                eat_mouse_move();
                match shown {
                    Ok(()) => {}
                    Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED) => return Ok(false),
                    Err(e) => return Err(step("Show", e)),
                }
                self.chosen = Some(self.dialog.GetResult().map_err(|e| step("GetResult", e))?);
                Ok(true)
            }
        }

        fn chosen(&self) -> io::Result<Option<Box<dyn ShellItem>>> {
            Ok(self.chosen.clone().map(|item| -> Box<dyn ShellItem> { Box::new(Item(item)) }))
        }
    }

    /// The shell's item, read for its file-system path.
    struct Item(IShellItem);

    impl ShellItem for Item {
        fn file_system_path(&self) -> io::Result<Vec<u16>> {
            // SAFETY: the shell allocates the name; it is copied, and then freed once.
            unsafe {
                let name = self.0.GetDisplayName(SIGDN_FILESYSPATH).map_err(|e| step("GetDisplayName", e))?;
                let units = name.as_wide().to_vec();
                CoTaskMemFree(Some(name.as_ptr().cast()));
                Ok(units)
            }
        }
    }

    /// Removes the mouse moves the dialog leaves in the queue, and reposts the last one with no button held: the dialog
    /// closes on a double click, and the stale move would read as the button still down (Qt's `eatMouseMove`, which
    /// does the same in its native file dialog).
    fn eat_mouse_move() {
        let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        // SAFETY: the queue of this thread; the message is only read after PeekMessageW filled it.
        unsafe {
            while PeekMessageW(&mut msg, None, WM_MOUSEMOVE, WM_MOUSEMOVE, PM_REMOVE).as_bool() {}
            if msg.message == WM_MOUSEMOVE {
                let _ = PostMessageW(Some(msg.hwnd), msg.message, Default::default(), msg.lParam);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::Path;

    use super::{FolderDialog, ShellItem, pick_with};
    use crate::names::path_buf;

    /// A failed call of the dialog names the call for the log, and its message is the system's text alone.
    #[test]
    fn a_failed_call_is_named_for_the_log_and_not_in_the_message() {
        // ERROR_INVALID_PARAMETER (87), the "Parâmetro incorreto" of the dialog's failure: 0x80070057.
        let system = windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(87));
        let error = super::native::step("SetFolder", system);
        assert_eq!(super::failed_call(&error), Some("SetFolder"));
        let message = error.to_string();
        assert!(message.contains("0x80070057"), "the system's error: {message}");
        assert!(!message.contains("SetFolder"), "the message the user sees: {message}");
        assert_eq!(super::failed_call(&io::Error::other("not a dialog call")), None);
    }

    /// What the dialog was opened with: its start folder's units and its owner.
    type Opened = (Option<Vec<u16>>, Option<usize>);

    /// A stand-in for the system's dialog: it records what it was opened with, and answers as set.
    struct Fake {
        chose: bool,
        chosen: Vec<u16>,
        seen: RefCell<Option<Opened>>,
    }

    struct Units(Vec<u16>);

    impl ShellItem for Units {
        fn file_system_path(&self) -> io::Result<Vec<u16>> {
            Ok(self.0.clone())
        }
    }

    impl FolderDialog for Fake {
        fn run(&mut self, start: Option<&[u16]>, owner: Option<usize>) -> io::Result<bool> {
            *self.seen.borrow_mut() = Some((start.map(<[u16]>::to_vec), owner));
            Ok(self.chose)
        }

        fn chosen(&self) -> io::Result<Option<Box<dyn ShellItem>>> {
            Ok(self.chose.then(|| -> Box<dyn ShellItem> { Box::new(Units(self.chosen.clone())) }))
        }
    }

    fn surrogate_folder_units() -> Vec<u16> {
        vec![0x43, 0x3A, 0x5C, 0x61, 0xD800, 0x62]
    }

    #[test]
    fn a_chosen_folder_is_the_items_path_and_the_dialog_is_opened_as_asked() {
        let mut fake = Fake { chose: true, chosen: surrogate_folder_units(), seen: RefCell::new(None) };
        let start = [0x43, 0x3A, 0x5C, 0xD800];
        let picked = pick_with(&mut fake, Some(&start), Some(0x1234)).unwrap();
        assert_eq!(picked, Some(crate::native::verbatim(&path_buf(b"C:\\a\xED\xA0\x80b"))));
        assert_eq!(fake.seen.into_inner(), Some((Some(start.to_vec()), Some(0x1234))), "start and owner, exact");
    }

    #[test]
    fn a_cancelled_dialog_picks_nothing() {
        let mut fake = Fake { chose: false, chosen: Vec::new(), seen: RefCell::new(None) };
        assert_eq!(pick_with(&mut fake, None, None).unwrap(), None);
        assert_eq!(fake.seen.into_inner(), Some((None, None)), "opened unowned, with no start folder");
    }

    #[test]
    fn the_path_of_a_chosen_item_is_read_once_the_dialog_closed() {
        // The folder in the verbatim form, which reaches it exactly.
        let folder = Path::new(r"\\?\C:\a");
        let mut fake = Fake { chose: true, chosen: vec![0x43, 0x3A, 0x5C, 0x61], seen: RefCell::new(None) };
        assert_eq!(pick_with(&mut fake, None, None).unwrap(), Some(folder.to_path_buf()));
    }

    /// A folder whose name ends in a dot, or one under it, starts the dialog at the nearest folder without one: the
    /// shell would take `a.` for its sibling `a`, or refuse its verbatim form.
    #[test]
    fn a_dotted_folder_starts_the_dialog_at_its_nearest_plain_ancestor() {
        use crate::native::verbatim;
        let dir =
            std::path::absolute(crate::testing::run_folder("scratch_picker", "pg-start-dotted").unwrap()).unwrap();
        let (dotted, sibling) = (dir.join("a."), dir.join("a"));
        std::fs::create_dir_all(verbatim(&dotted.join("inner"))).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let real = std::fs::canonicalize(&dir).unwrap();
        for asked in [verbatim(&dotted), verbatim(&dotted.join("inner"))] {
            assert_eq!(super::start_folder(Some(asked.clone()), |_| None), Some(real.clone()), "{}", asked.display());
        }
        assert_eq!(
            super::start_folder(Some(sibling.clone()), |_| None),
            Some(std::fs::canonicalize(&sibling).unwrap())
        );
    }
}

#[cfg(test)]
mod start_tests {
    use std::path::{Component, Path};

    /// A start folder given with a `..` part is opened in its real form: the shell refuses the path as it is.
    #[test]
    fn a_start_folder_is_given_in_its_real_form() {
        let tree = crate::testing::run_folder("scratch_picker", "pg-start-real").unwrap();
        std::fs::create_dir(tree.join("inner")).unwrap();
        let got = super::start_folder(Some(tree.join("inner").join("..")), |_| None).expect("the start folder");
        assert_eq!(got, std::fs::canonicalize(&tree).unwrap());
        assert!(!got.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)));
        let _ = std::fs::remove_dir_all(&tree);
    }

    /// Without a folder, the dialog opens in the user's own.
    #[test]
    fn without_a_folder_the_dialog_opens_in_the_users() {
        let home = std::env::current_dir().unwrap().into_os_string();
        let var = |name: &str| (name == "USERPROFILE" || name == "HOME").then(|| home.clone());
        let gone = Path::new("no-such-folder-for-the-start-test").to_path_buf();
        assert_eq!(super::start_folder(Some(gone), var), Some(std::path::PathBuf::from(home.clone())));
        assert_eq!(super::start_folder(None, |_| None), None);
    }

    /// A path under a volume with no drive letter is not a drive or share, so the start folder keeps its path as given;
    /// the drive and share forms are.
    #[cfg(windows)]
    #[test]
    fn only_a_drive_or_a_share_is_given_in_its_canonical_form() {
        use super::has_drive_or_share;
        assert!(has_drive_or_share(Path::new(r"\\?\C:\photos")));
        assert!(has_drive_or_share(Path::new(r"C:\photos")));
        assert!(has_drive_or_share(Path::new(r"\\?\UNC\server\share\a")));
        assert!(has_drive_or_share(Path::new(r"\\server\share\a")));
        assert!(!has_drive_or_share(Path::new(r"\\?\Volume{38788d0c-38dd-46ca-b547-211e9c17f3ac}\photos")));
    }
}
