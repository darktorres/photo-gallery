//! The folder dialog of Windows: the Explorer's own folder picker (`IFileOpenDialog`, folders only). The folder it gives
//! comes back in the system's units, so a name with an unpaired surrogate is exact (a dialog's text would not be). On
//! the other systems the app has its own picker (the desktop's folder picker, over a listing of the folders), so there
//! is no native one here.
//!
//! Qt initializes COM on the GUI thread (a single-threaded apartment), so this initializes nothing, and it is called on
//! that thread only. The dialog runs its own message loop while it is open: a caller holds no borrow of its state
//! across the call. The dialog's steps are behind [`FolderDialog`] and [`ShellItem`], so `pick_with` is tested with a
//! fake dialog; the Windows implementation is only the calls to the system.

use std::io;
#[cfg(windows)]
use std::path::PathBuf;

/// The item the dialog chose: its file-system path in the system's units (`SIGDN_FILESYSPATH`).
pub trait ShellItem {
    fn file_system_path(&self) -> io::Result<Vec<u16>>;
}

/// The folder dialog's steps that touch the system.
pub trait FolderDialog {
    /// Shows the dialog until the user chooses or cancels. `start` is the folder it opens in (its units, without a NUL),
    /// `owner` the window it belongs to (its handle; None: unowned). Ok(true) when a folder was chosen.
    fn run(&mut self, start: Option<&[u16]>, owner: Option<usize>) -> io::Result<bool>;
    /// The item `run` chose, if it chose one.
    fn chosen(&self) -> io::Result<Option<Box<dyn ShellItem>>>;
}

/// Runs `dialog` and gives the folder it chose, exactly. None when the user cancelled. `start` is the folder the dialog
/// opens in (its units), and `owner` the window it belongs to (None for none).
#[cfg(windows)]
pub fn pick_with(
    dialog: &mut dyn FolderDialog,
    start: Option<&[u16]>,
    owner: Option<usize>,
) -> io::Result<Option<PathBuf>> {
    if !dialog.run(start, owner)? {
        return Ok(None);
    }
    let item = dialog.chosen()?.ok_or_else(|| io::Error::other("the dialog chose no folder"))?;
    Ok(Some(path_of_units(&item.file_system_path()?)))
}

/// A path from the system's units (UTF-16 on Windows), exactly.
#[cfg(windows)]
pub fn path_of_units(units: &[u16]) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_wide(units))
}

/// A path in the form the shell takes: backslashes for forward slashes, and no `\\?\` verbatim prefix (the shell
/// refuses both with "invalid parameter"). `\\?\UNC\server\share` is `\\server\share`.
#[cfg(windows)]
pub fn shell_path(units: &[u16]) -> Vec<u16> {
    const VERBATIM: [u16; 4] = [0x5C, 0x5C, 0x3F, 0x5C];
    const UNC: [u16; 4] = [0x55, 0x4E, 0x43, 0x5C];
    let units: Vec<u16> = units.iter().map(|&u| if u == 0x2F { 0x5C } else { u }).collect();
    let Some(rest) = units.strip_prefix(&VERBATIM) else { return units };
    match rest.strip_prefix(&UNC) {
        Some(share) => [0x5C, 0x5C].into_iter().chain(share.iter().copied()).collect(),
        None => rest.to_vec(),
    }
}

#[cfg(windows)]
pub use native::Dialog;

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

    use super::{FolderDialog, ShellItem, shell_path};

    /// ERROR_CANCELLED, which the dialog reports when the user closes it without choosing.
    const ERROR_CANCELLED: u32 = 1223;

    /// The system's folder dialog, made with folders-only options (as Qt's native dialog sets them).
    pub struct Dialog {
        dialog: IFileOpenDialog,
        chosen: Option<IShellItem>,
    }

    impl Dialog {
        pub fn new() -> io::Result<Dialog> {
            // SAFETY: COM is initialized on this thread by Qt (see the module's note). The dialog is released when the
            // smart pointer is dropped.
            let dialog: IFileOpenDialog =
                unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }.map_err(io::Error::other)?;
            // SAFETY: plain calls on the dialog made just above.
            unsafe {
                let options = dialog.GetOptions().map_err(io::Error::other)?;
                dialog
                    .SetOptions(options | FOS_PICKFOLDERS | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM)
                    .map_err(io::Error::other)?;
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
        let item: IShellItem =
            unsafe { SHCreateItemFromParsingName(PCWSTR(units.as_ptr()), None) }.map_err(io::Error::other)?;
        Ok(Some(item))
    }

    impl FolderDialog for Dialog {
        fn run(&mut self, start: Option<&[u16]>, owner: Option<usize>) -> io::Result<bool> {
            // A start folder the shell refuses is an error, which the picker shows (the desktop picker passes only a
            // folder that is there).
            if let Some(item) = start.map(start_item).transpose()?.flatten() {
                // SAFETY: a plain call on the dialog and the item made just above.
                unsafe { self.dialog.SetFolder(&item) }.map_err(io::Error::other)?;
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
                    Err(e) => return Err(io::Error::other(e)),
                }
                self.chosen = Some(self.dialog.GetResult().map_err(io::Error::other)?);
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
                let name = self.0.GetDisplayName(SIGDN_FILESYSPATH).map_err(io::Error::other)?;
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

    use super::{FolderDialog, ShellItem, path_of_units, pick_with, shell_path};
    use crate::names::path_buf;

    /// The units of a folder with an unpaired surrogate come back as that folder's path, exactly.
    #[test]
    fn a_folder_from_its_units_keeps_an_unpaired_surrogate() {
        let units = [0x43, 0x3A, 0x5C, 0x61, 0xD800, 0x62];
        assert_eq!(path_of_units(&units), path_buf(b"C:\\a\xED\xA0\x80b"));
        assert_eq!(path_of_units(&[0x43, 0x3A]), path_buf(b"C:"));
    }

    /// A start folder reaches the shell in the form it takes: the forward slashes and the verbatim prefix the shell
    /// refuses are written as backslashes and dropped.
    #[test]
    fn a_start_folder_is_in_the_form_the_shell_takes() {
        let units = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(shell_path(&units("D:/GitHub/photo-gallery")), units("D:\\GitHub\\photo-gallery"));
        assert_eq!(shell_path(&units("\\\\?\\C:\\Users")), units("C:\\Users"));
        assert_eq!(shell_path(&units("\\\\?\\UNC\\server\\share")), units("\\\\server\\share"));
        assert_eq!(shell_path(&units("C:\\Users\\a")), units("C:\\Users\\a"));
        assert!(shell_path(&units("")).is_empty());
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
        assert_eq!(picked, Some(path_buf(b"C:\\a\xED\xA0\x80b")));
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
        let folder = Path::new("C:\\a");
        let mut fake = Fake { chose: true, chosen: vec![0x43, 0x3A, 0x5C, 0x61], seen: RefCell::new(None) };
        assert_eq!(pick_with(&mut fake, None, None).unwrap(), Some(folder.to_path_buf()));
    }
}
