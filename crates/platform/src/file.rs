//! Files as the library opens them, and their times.

use std::fs::{File, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use crate::native::NativePath;

/// Open the file at `path` for reading, with every sharing mode and no flags: the call std makes, without std's
/// conversion of the path into a new buffer for each file.
#[cfg(windows)]
pub fn open_read(path: &NativePath) -> io::Result<File> {
    use std::os::windows::io::FromRawHandle;
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
    Ok(unsafe { File::from_raw_handle(h) })
}

/// Elsewhere, the same file, opened by its bytes.
#[cfg(unix)]
pub fn open_read(path: &NativePath) -> io::Result<File> {
    use std::os::unix::ffi::OsStrExt;
    File::open(std::ffi::OsStr::from_bytes(path.units()))
}

/// Open the file at `path` to write it in place (the writer's copy of a new content into the original), while denying
/// other writers: other programs may read it meanwhile, but an open of theirs for writing or deleting fails until this
/// handle is closed, so a save of theirs can't interleave with the copy. A program that has it open for writing
/// already makes this open fail (a sharing violation). The handle can also read the file's attributes, so its size and
/// write time can be read from it (`File::metadata`).
#[cfg(windows)]
pub fn open_for_copy(path: &NativePath) -> io::Result<File> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, OPEN_EXISTING};
    let path = path.units();
    debug_assert_eq!(path.last(), Some(&0));
    // SAFETY: `path` is NUL-terminated and outlives the call; the null pointers are optional arguments.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_WRITE | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ,
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
    Ok(unsafe { File::from_raw_handle(h) })
}

/// Elsewhere, the same file opened for writing by its bytes. Unix has no sharing modes: other writers aren't denied.
#[cfg(unix)]
pub fn open_for_copy(path: &NativePath) -> io::Result<File> {
    use std::os::unix::ffi::OsStrExt;
    std::fs::OpenOptions::new().write(true).open(std::ffi::OsStr::from_bytes(path.units()))
}

/// A temporary file that is removed when this is dropped, unless [`TempFile::persist`] said it was kept (renamed
/// into place). Armed as soon as the temporary's path is known, before its first write, so a failed or partial
/// write, or a failure after it (a time not set, a rename refused), leaves no temporary behind. The removal is
/// best-effort: a file that can't be removed (or was never made) is left as it is.
#[must_use = "a TempFile dropped at once removes its file at once"]
#[derive(Debug)]
pub struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl TempFile {
    /// The guard for the temporary file at `path` (made or not yet).
    pub fn new(path: &Path) -> TempFile {
        TempFile { path: path.to_path_buf(), armed: true }
    }

    /// Where the temporary file is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The temporary is kept (it has been renamed into place): the guard no longer removes anything.
    pub fn persist(mut self) {
        self.armed = false;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A file's last write time in FILETIME ticks: 100 ns since 1601-01-01 UTC, as the catalog stores it.
#[cfg(windows)]
pub fn last_write_time(meta: &Metadata) -> i64 {
    use std::os::windows::fs::MetadataExt;
    i64::try_from(meta.last_write_time()).unwrap_or(i64::MAX)
}

/// From 1601-01-01 to 1970-01-01, in 100 ns ticks.
#[cfg(unix)]
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// Elsewhere, the same count from the Unix time.
#[cfg(unix)]
pub fn last_write_time(meta: &Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime() * 10_000_000 + meta.mtime_nsec() / 100 + UNIX_EPOCH_TICKS
}

/// The birth time in FILETIME ticks, where the file system gives one (`statx` with `STATX_BTIME`, which std asks for in
/// its metadata calls). `None` when it doesn't (drvfs, older kernels) or when the time is before 1970.
#[cfg(unix)]
pub fn created_time(meta: &Metadata) -> Option<i64> {
    let since = meta.created().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    let secs = i64::try_from(since.as_secs()).ok()?;
    Some(UNIX_EPOCH_TICKS + secs * 10_000_000 + i64::from(since.subsec_nanos() / 100))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// The copy's open writes the file in place; other programs may read it meanwhile, and on Windows a writer of theirs
    /// is refused until the handle closes. Its size is read from the handle.
    #[test]
    fn a_copy_open_writes_in_place_and_denies_other_writers() {
        let dir = pg_testkit::test_folder("scratch_rate", "pg-open-copy").unwrap();
        let path = dir.join("a.bin");
        std::fs::write(&path, b"before").unwrap();
        let mut native = NativePath::default();
        crate::native::set_os_str(&mut native, path.as_os_str());
        let mut file = open_for_copy(&native).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 6);
        if cfg!(windows) {
            assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err(), "another writer is refused");
        }
        let mut read = String::new();
        std::fs::File::open(&path).unwrap().read_to_string(&mut read).unwrap();
        assert_eq!(read, "before", "a reader is let in");
        file.write_all(b"after!").unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), b"after!");
        // A missing file is the open's error, as std's.
        crate::native::set_os_str(&mut native, dir.join("missing.bin").as_os_str());
        assert_eq!(open_for_copy(&native).unwrap_err().kind(), io::ErrorKind::NotFound);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A temporary is removed when its guard drops, unless it was persisted; one never made is no error.
    #[test]
    fn a_temp_file_is_removed_unless_persisted() {
        let dir = pg_testkit::test_folder("scratch_rate", "pg-temp-file").unwrap();
        let path = dir.join("a.tmp");
        {
            let guard = TempFile::new(&path);
            std::fs::write(guard.path(), b"partial").unwrap();
        }
        assert!(!path.exists(), "dropped: removed");
        let guard = TempFile::new(&path);
        std::fs::write(&path, b"whole").unwrap();
        guard.persist();
        assert_eq!(std::fs::read(&path).unwrap(), b"whole", "persisted: kept");
        drop(TempFile::new(&dir.join("never-made.tmp")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
