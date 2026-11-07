//! The folder dialog of Windows: the Explorer's own folder picker (`IFileOpenDialog`, folders only). The folder it gives
//! comes back in the system's units, so a name with an unpaired surrogate is exact (a dialog's text would not be). On
//! the other systems the app has its own picker (the desktop's folder picker, over a listing of the folders), so there
//! is no native one here.
//!
//! Qt initializes COM on the GUI thread (a single-threaded apartment), so this initializes nothing, and it is called on
//! that thread only. The dialog runs its own message loop while it is open: a caller holds no borrow of its state
//! across the call.

use std::io;
use std::path::{Path, PathBuf};

/// The folder the user picks, or None when they cancel. `start` is the folder the dialog opens in, if it has one. The
/// dialog is owned by the app's window when that is the foreground one (the dialog then stays on top of the app).
#[cfg(windows)]
pub fn pick_folder(start: Option<&Path>) -> io::Result<Option<PathBuf>> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
    use windows::Win32::UI::Shell::{
        FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
        SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    use windows::core::{HRESULT, PCWSTR};

    /// ERROR_CANCELLED, which the dialog reports when the user closes it without choosing.
    const ERROR_CANCELLED: u32 = 1223;

    // SAFETY: COM is initialized on this thread by Qt. The dialog and the items are made and released here (the
    // smart pointers of the bindings), and the one string the shell allocates is freed once, after it is copied.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(io::Error::other)?;
        let options = dialog.GetOptions().map_err(io::Error::other)?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM)
            .map_err(io::Error::other)?;
        if let Some(start) = start {
            // The start folder from its own units (a NUL-terminated copy for the call).
            let units: Vec<u16> = start.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            // SAFETY (for this call): `units` is NUL-terminated and lives for the call.
            let item: IShellItem =
                SHCreateItemFromParsingName(PCWSTR(units.as_ptr()), None).map_err(io::Error::other)?;
            dialog.SetFolder(&item).map_err(io::Error::other)?;
        }
        if let Err(e) = dialog.Show(owner()) {
            if e.code() == HRESULT::from_win32(ERROR_CANCELLED) {
                return Ok(None);
            }
            return Err(io::Error::other(e));
        }
        let item = dialog.GetResult().map_err(io::Error::other)?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(io::Error::other)?;
        let path = path_of_units(name.as_wide());
        CoTaskMemFree(Some(name.as_ptr().cast()));
        Ok(Some(path))
    }
}

/// The folder dialog is native on Windows only (see the module's note).
#[cfg(not(windows))]
pub fn pick_folder(_start: Option<&Path>) -> io::Result<Option<PathBuf>> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "the native folder dialog is Windows only"))
}

/// A path from the system's units (UTF-16 on Windows), exactly.
#[cfg(windows)]
pub fn path_of_units(units: &[u16]) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_wide(units))
}

/// The window the dialog is owned by: the foreground window, when this process made it (a click in the app is what
/// opens the dialog, so the app's window is foreground then). Otherwise none.
#[cfg(windows)]
fn owner() -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    // SAFETY: plain queries. The window may be null (no foreground window), which is then no owner.
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        (pid == GetCurrentProcessId()).then_some(window)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::path_of_units;
    use crate::names::path_buf;

    /// The units of a folder with an unpaired surrogate come back as that folder's path, exactly.
    #[test]
    fn a_folder_from_its_units_keeps_an_unpaired_surrogate() {
        let units = [0x43, 0x3A, 0x5C, 0x61, 0xD800, 0x62];
        assert_eq!(path_of_units(&units), path_buf(b"C:\\a\xED\xA0\x80b"));
        assert_eq!(path_of_units(&[0x43, 0x3A]), path_buf(b"C:"));
    }
}
