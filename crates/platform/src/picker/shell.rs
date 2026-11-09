//! The folder dialog's pieces that make no system call: the two traits the dialog is behind ([`FolderDialog`], [`ShellItem`])
//! and the path conversions the shell takes, in UTF-16 units. A leaf of `picker`: the Windows dialog (`picker::native`) imports
//! these from here, and `picker` re-exports them, so the public paths do not change.

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

#[cfg(all(test, windows))]
mod tests {
    use super::{path_of_units, shell_path};
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
}
