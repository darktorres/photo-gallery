//! The folder SQLite opens files in, exact. On Unix SQLite takes a path's bytes as they are (rusqlite passes them), so a
//! folder is used as it is. On Windows SQLite takes a name as UTF-8, and rusqlite refuses any other: a folder whose name
//! has no UTF-8 form (an unpaired surrogate) is reached through a directory junction whose name is ASCII, made in the
//! alias folder the caller names (the data folder's `sqlite-aliases`, `pg_library::layout::Layout::sqlite_aliases`),
//! from the folder's exact UTF-16 units; it points at the folder on any local NTFS volume. SQLite's own files (the
//! journal, the `-wal`) are then made in the folder, through the junction. Nothing is made outside the alias folder.

use std::io;
use std::path::{Path, PathBuf};

/// The folder SQLite opens files of `folder` in: `folder` itself where SQLite can take its name; on Windows, for a
/// folder with no UTF-8 form, a junction to it made (or reused) in `aliases`. The files in it are named by the app
/// (ASCII), so a path under the folder returned is one SQLite opens.
pub fn openable_folder(folder: &Path, aliases: &Path) -> io::Result<PathBuf> {
    #[cfg(unix)]
    {
        let _ = aliases;
        Ok(folder.to_path_buf())
    }
    #[cfg(windows)]
    {
        if folder.to_str().is_some() {
            return Ok(folder.to_path_buf());
        }
        windows::alias_under(aliases, folder)
    }
}

/// The file name SQLite is given for an ATTACH of `path`, a path SQLite opens (under an [`openable_folder`]): on Unix
/// the path's own bytes, which SQLite takes as they are; on Windows its UTF-8 (an error for a path with none). Bound as
/// a BLOB, so no text conversion is made.
pub fn attach_bytes(path: &Path) -> io::Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(path.as_os_str().as_bytes().to_vec())
    }
    #[cfg(windows)]
    {
        path.to_str().map(|text| text.as_bytes().to_vec()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "a path with no UTF-8 form, which SQLite can't take")
        })
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsString;
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NOT_A_REPARSE_POINT, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, RemoveDirectoryW,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    /// `FSCTL_SET_REPARSE_POINT` and `FSCTL_GET_REPARSE_POINT` (winioctl.h).
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00A8;
    /// A junction's reparse tag (winnt.h).
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
    /// The largest reparse data a read can return (winnt.h).
    const MAXIMUM_REPARSE_DATA_BUFFER_SIZE: usize = 16 * 1024;
    /// The prefix of a junction's substitute name: the object-manager form of an absolute path.
    const NT_PREFIX: [u16; 4] = [0x5C, 0x3F, 0x3F, 0x5C];

    /// The junction at `root\<hash of folder>` to `folder`, made or reused. `root` must have a UTF-8 path (the link's
    /// name is ASCII, and SQLite is given `root\...`). Links in `root` whose folder is gone are removed first. A folder
    /// that holds the name but isn't a junction is an error, not replaced.
    pub(super) fn alias_under(root: &Path, folder: &Path) -> io::Result<PathBuf> {
        if root.to_str().is_none() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "the alias folder has no UTF-8 path"));
        }
        let target = std::path::absolute(folder)?;
        if !target.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("{}: no such folder", target.display())));
        }
        let units: Vec<u16> = target.as_os_str().encode_wide().collect();
        let substitute: Vec<u16> = NT_PREFIX.iter().copied().chain(units.iter().copied()).collect();
        std::fs::create_dir_all(root)?;
        sweep(root)?;
        let link = root.join(format!("{:016x}", fnv1a(&units)));
        match read_mount_point(&link)? {
            Some(existing) if existing == substitute => return Ok(link),
            Some(_) => remove_link(&link)?,
            None if link.symlink_metadata().is_ok() => {
                return Err(io::Error::other(format!("{}: is not an alias", link.display())));
            }
            None => {}
        }
        make_link(&link, &substitute, &units)?;
        Ok(link)
    }

    /// Removes the junctions in `root` whose folder is gone (a junction is removed as a link: its folder is not touched).
    fn sweep(root: &Path) -> io::Result<()> {
        for entry in std::fs::read_dir(root)? {
            let link = entry?.path();
            let Some(substitute) = read_mount_point(&link)? else { continue };
            let Some(target) = substitute.strip_prefix(&NT_PREFIX[..]) else { continue };
            if !Path::new(&OsString::from_wide(target)).exists() {
                remove_link(&link)?;
            }
        }
        Ok(())
    }

    /// A junction's substitute name (its `\??\` prefix included), or None when `link` is not a junction (or doesn't
    /// exist).
    fn read_mount_point(link: &Path) -> io::Result<Option<Vec<u16>>> {
        let name = nul_terminated(link.as_os_str().encode_wide());
        // SAFETY: `name` is NUL-terminated and lives for the call; the handle is closed below in every path.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::NotFound { Ok(None) } else { Err(e) };
        }
        let mut buf = vec![0u8; MAXIMUM_REPARSE_DATA_BUFFER_SIZE];
        let mut returned = 0u32;
        // SAFETY: an open handle; `buf` is the output buffer, and its length is the size given.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_GET_REPARSE_POINT,
                std::ptr::null(),
                0,
                buf.as_mut_ptr().cast(),
                u32::try_from(buf.len()).map_err(io::Error::other)?,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        let error = (ok == 0).then(io::Error::last_os_error);
        close(handle);
        if let Some(e) = error {
            // Not a reparse point at all: not a junction.
            return if e.raw_os_error() == Some(i32::try_from(ERROR_NOT_A_REPARSE_POINT).unwrap_or(-1)) {
                Ok(None)
            } else {
                Err(e)
            };
        }
        let len = usize::try_from(returned).map_err(io::Error::other)?;
        let data = buf.get(..len).ok_or_else(|| io::Error::other("the reparse data is longer than its buffer"))?;
        Ok(parse_mount_point(data))
    }

    /// The substitute name of a mount point's reparse data (`REPARSE_DATA_BUFFER`): the tag, the length, then the
    /// offsets and lengths of the two names, and the names' units (the offsets are from the names' buffer, in bytes).
    fn parse_mount_point(data: &[u8]) -> Option<Vec<u16>> {
        let tag = u32::from_le_bytes(data.get(0..4)?.try_into().ok()?);
        if tag != IO_REPARSE_TAG_MOUNT_POINT {
            return None;
        }
        let offset = usize::from(u16::from_le_bytes(data.get(8..10)?.try_into().ok()?));
        let len = usize::from(u16::from_le_bytes(data.get(10..12)?.try_into().ok()?));
        let bytes = data.get(16 + offset..16 + offset + len)?;
        let (pairs, _) = bytes.as_chunks::<2>();
        Some(pairs.iter().map(|&[lo, hi]| u16::from_le_bytes([lo, hi])).collect())
    }

    /// Makes the junction `link` with the substitute name and the print name (the folder's own units). The folder is
    /// created first and removed again if the reparse data can't be set.
    fn make_link(link: &Path, substitute: &[u16], print: &[u16]) -> io::Result<()> {
        let name = nul_terminated(link.as_os_str().encode_wide());
        // SAFETY: `name` is NUL-terminated and lives for the call.
        if unsafe { CreateDirectoryW(name.as_ptr(), std::ptr::null()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = set_reparse(&name, substitute, print);
        if result.is_err() {
            // SAFETY: `name` is NUL-terminated; the folder was just made, and is empty.
            unsafe { RemoveDirectoryW(name.as_ptr()) };
        }
        result
    }

    fn set_reparse(name: &[u16], substitute: &[u16], print: &[u16]) -> io::Result<()> {
        // The names as the buffer holds them: each NUL-terminated, the print name after the substitute name.
        let mut names: Vec<u16> = substitute.to_vec();
        names.push(0);
        let print_offset = names.len() * 2;
        names.extend_from_slice(print);
        names.push(0);
        let path_bytes: Vec<u8> = names.iter().flat_map(|u| u.to_le_bytes()).collect();
        let data_len = 8 + path_bytes.len();
        let mut buf = Vec::with_capacity(8 + data_len);
        buf.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
        buf.extend_from_slice(&u16::try_from(data_len).map_err(io::Error::other)?.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // substitute name offset
        buf.extend_from_slice(&u16::try_from(substitute.len() * 2).map_err(io::Error::other)?.to_le_bytes());
        buf.extend_from_slice(&u16::try_from(print_offset).map_err(io::Error::other)?.to_le_bytes());
        buf.extend_from_slice(&u16::try_from(print.len() * 2).map_err(io::Error::other)?.to_le_bytes());
        buf.extend_from_slice(&path_bytes);
        // SAFETY: `name` is NUL-terminated; the handle is closed below; `buf` is the input, with its length given.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut returned = 0u32;
        // SAFETY: an open handle; the input buffer and its length are `buf`'s.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_REPARSE_POINT,
                buf.as_ptr().cast(),
                u32::try_from(buf.len()).map_err(io::Error::other)?,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        let error = (ok == 0).then(io::Error::last_os_error);
        close(handle);
        error.map_or(Ok(()), Err)
    }

    /// Removes the junction `link` (its folder is not touched).
    fn remove_link(link: &Path) -> io::Result<()> {
        let name = nul_terminated(link.as_os_str().encode_wide());
        // SAFETY: `name` is NUL-terminated and lives for the call.
        if unsafe { RemoveDirectoryW(name.as_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn close(handle: HANDLE) {
        // SAFETY: the handle came from a successful CreateFileW and is closed once, here.
        unsafe { CloseHandle(handle) };
    }

    fn nul_terminated(units: impl Iterator<Item = u16>) -> Vec<u16> {
        let mut v: Vec<u16> = units.collect();
        v.push(0);
        v
    }

    /// FNV-1a over the units' bytes: the same folder gives the same name in every run (the link is reused).
    fn fnv1a(units: &[u16]) -> u64 {
        units
            .iter()
            .flat_map(|u| u.to_le_bytes())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3))
    }

    #[cfg(test)]
    mod tests {
        use super::alias_under;
        use crate::names::path_buf;

        /// A folder with a lone surrogate in its name (WTF-8 `ED A0 80`), in `parent`.
        fn odd_folder(parent: &std::path::Path, name: &[u8]) -> std::path::PathBuf {
            let folder = parent.join(path_buf(name));
            std::fs::create_dir_all(&folder).unwrap();
            folder
        }

        /// A file made through the alias is in the folder; a second request reuses the same junction.
        #[test]
        fn a_folder_without_a_utf8_name_is_reached_through_an_alias() {
            let dir = pg_testkit::test_folder("scratch_sqlite", "alias").unwrap();
            let folder = odd_folder(&dir, b"cat\xED\xA0\x80");
            let root = dir.join("aliases");
            let link = alias_under(&root, &folder).unwrap();
            assert!(link.to_str().is_some(), "{}", link.display());
            std::fs::write(link.join("made-through-it.txt"), b"x").unwrap();
            assert!(folder.join("made-through-it.txt").exists());
            assert_eq!(alias_under(&root, &folder).unwrap(), link, "the same junction is reused");
            std::fs::remove_dir_all(&dir).unwrap();
        }

        /// A junction whose folder is gone is swept by the next alias made in its folder.
        #[test]
        fn a_stale_alias_is_swept() {
            let dir = pg_testkit::test_folder("scratch_sqlite", "stale").unwrap();
            let root = dir.join("aliases");
            let gone = odd_folder(&dir, b"gone\xED\xA0\x80");
            let stale = alias_under(&root, &gone).unwrap();
            std::fs::remove_dir_all(&gone).unwrap();
            let kept = odd_folder(&dir, b"kept\xED\xA0\x80");
            alias_under(&root, &kept).unwrap();
            assert!(stale.symlink_metadata().is_err(), "{} is still there", stale.display());
            std::fs::remove_dir_all(&dir).unwrap();
        }

        /// The folder may be on another volume than the alias: the temp folder (C:) is the target, the test folder the
        /// alias's (D:).
        #[test]
        fn an_alias_can_point_at_a_folder_on_another_volume() {
            let dir = pg_testkit::test_folder("scratch_sqlite", "volume").unwrap();
            let folder = odd_folder(&std::env::temp_dir(), b"pg-alias-\xED\xA0\x80");
            let link = alias_under(&dir.join("aliases"), &folder).unwrap();
            std::fs::write(link.join("across.txt"), b"x").unwrap();
            assert!(folder.join("across.txt").exists());
            std::fs::remove_dir_all(&folder).unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::openable_folder;

    /// On Unix the folder is used as it is, with any bytes, and nothing is made in the alias folder.
    #[test]
    fn a_folder_is_used_as_it_is_on_unix() {
        let folder = crate::names::path_buf(b"/tmp/odd\xFF");
        let aliases = std::path::Path::new("/nonexistent/aliases");
        assert_eq!(openable_folder(&folder, aliases).unwrap(), folder);
        assert!(!aliases.exists());
    }
}
