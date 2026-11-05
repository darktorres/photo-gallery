//! Files as the library opens them, and their times.

use std::fs::{File, Metadata};
use std::io;

use crate::native::NativePath;

/// Open the file at `path` for reading, with every sharing mode and no flags: the call std makes, without std's
/// conversion of the path into a new buffer for each file.
#[cfg(windows)]
pub fn open_read(path: &NativePath) -> io::Result<File> {
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use windows_sys::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    let path = path.units();
    debug_assert_eq!(path.last(), Some(&0));
    // SAFETY: `path` is NUL-terminated and outlives the call; the null pointers are optional arguments.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a handle just opened, owned by nothing else; the `File` closes it.
    Ok(unsafe { File::from_raw_handle(h as RawHandle) })
}

/// Elsewhere, the same file, opened by its bytes.
#[cfg(unix)]
pub fn open_read(path: &NativePath) -> io::Result<File> {
    use std::os::unix::ffi::OsStrExt;
    File::open(std::ffi::OsStr::from_bytes(path.units()))
}

/// A file's last write time in FILETIME ticks: 100 ns since 1601-01-01 UTC, as the catalog stores it.
#[cfg(windows)]
pub fn last_write_time(meta: &Metadata) -> i64 {
    use std::os::windows::fs::MetadataExt;
    meta.last_write_time() as i64
}

/// Elsewhere, the same count from the Unix time.
#[cfg(unix)]
pub fn last_write_time(meta: &Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    /// From 1601-01-01 to 1970-01-01, in 100 ns ticks.
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    meta.mtime() * 10_000_000 + meta.mtime_nsec() / 100 + UNIX_EPOCH_TICKS
}
