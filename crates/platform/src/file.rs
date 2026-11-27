//! Files as the library opens them, and their times.

#![expect(unsafe_code, reason = "FFI: the Windows file handle calls, and libc's posix_fadvise on Unix")]

use std::fs::{File, Metadata};
use std::io;
use std::path::{Path, PathBuf};

/// Writes `bytes` to a new file at `path` and flushes them to disk: the step that both an atomic replace and a claimed
/// save start with (each names its temporary its own way).
pub fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()
}

use crate::native::NativePath;

/// A file that couldn't be read, as the I/O error of the read (its kind and OS code kept whole). It is the one read
/// failure the library names: `MakeError::Unreadable` holds it, and an error chain that holds it also holds the I/O
/// error, as its [`std::error::Error::source`], so a check for either finds it.
#[derive(Debug)]
pub struct ReadFailed(io::Error);

impl ReadFailed {
    /// The I/O error of the read.
    pub fn io(&self) -> &io::Error {
        &self.0
    }
}

impl From<io::Error> for ReadFailed {
    fn from(error: io::Error) -> ReadFailed {
        ReadFailed(error)
    }
}

impl std::fmt::Display for ReadFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the file can't be read")
    }
}

impl std::error::Error for ReadFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Open the file at `path` for reading, with every sharing mode and no flags: the call std makes, without std's
/// conversion of the path into a new buffer for each file.
#[cfg(windows)]
pub fn open_read(path: &NativePath) -> io::Result<File> {
    open_read_with(path, 0)
}

/// [`open_read`] for reads here and there in the file: the system reads no further ahead than each read asks
/// (`FILE_FLAG_RANDOM_ACCESS`), so a read of a file's two ends doesn't pull in the bytes after each.
#[cfg(windows)]
pub fn open_read_random(path: &NativePath) -> io::Result<File> {
    open_read_with(path, windows_sys::Win32::Storage::FileSystem::FILE_FLAG_RANDOM_ACCESS)
}

#[cfg(windows)]
fn open_read_with(path: &NativePath, flags: u32) -> io::Result<File> {
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
            flags,
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

/// [`open_read`] for reads here and there in the file: on Linux the kernel is told so (`POSIX_FADV_RANDOM`), and reads
/// no further ahead than each read asks; elsewhere the plain open.
#[cfg(unix)]
pub fn open_read_random(path: &NativePath) -> io::Result<File> {
    let file = open_read(path)?;
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: the descriptor is open for the call; the advice is a hint, and its failure is not an error.
        let _ = unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_RANDOM) };
    }
    Ok(file)
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

/// A file's size in bytes and last write time ([`last_write_time`]), as the scanner records them, from its metadata.
pub fn size_and_time(meta: &Metadata) -> io::Result<(u64, i64)> {
    Ok((meta.len(), last_write_time(meta)))
}

/// [`size_and_time`] of the file at `path` (a `names::disk_path`, which names it exactly).
pub fn stat(path: &Path) -> io::Result<(u64, i64)> {
    size_and_time(&std::fs::metadata(path)?)
}

/// [`size_and_time`] of an open file, from its handle.
pub fn stat_of(file: &File) -> io::Result<(u64, i64)> {
    size_and_time(&file.metadata()?)
}

/// Elsewhere, the same count from the Unix time. A time too far out for an i64 of ticks saturates at the bound.
#[cfg(unix)]
pub fn last_write_time(meta: &Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime()
        .saturating_mul(10_000_000)
        .saturating_add(meta.mtime_nsec() / 100)
        .saturating_add(pg_model::date::FILETIME_UNIX_EPOCH_TICKS)
}

/// The birth time in FILETIME ticks, where the file system gives one (`statx` with `STATX_BTIME`, which std asks for in
/// its metadata calls). `None` when it doesn't (drvfs, older kernels) or when the time is before 1970.
#[cfg(unix)]
pub fn created_time(meta: &Metadata) -> Option<i64> {
    let since = meta.created().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    let secs = i64::try_from(since.as_secs()).ok()?;
    Some(
        pg_model::date::FILETIME_UNIX_EPOCH_TICKS
            .saturating_add(secs.saturating_mul(10_000_000))
            .saturating_add(i64::from(since.subsec_nanos() / 100)),
    )
}

/// The file at `path` (a `names::disk_path`), open for reading, as the raw handle the video player takes over and
/// closes: a `HANDLE` on Windows, a descriptor on Unix, as one integer. The player can't be given the file by name: on
/// Windows Qt's file engine normalizes every name it opens, the verbatim one too (`GetFullPathNameW`), which drops a
/// folder's trailing dot, and on Unix a name with a byte that isn't UTF-8 has no `QString` form.
#[cfg(windows)]
pub fn player_handle(path: &Path) -> io::Result<i64> {
    use std::os::windows::io::IntoRawHandle;
    let handle = File::open(path)?.into_raw_handle();
    // A handle is a small kernel table index, well inside an i64; one that weren't is closed and refused.
    i64::try_from(handle.addr()).map_err(|_| {
        // SAFETY: the handle was just taken from the `File`, and nothing else owns it.
        drop(unsafe { <File as std::os::windows::io::FromRawHandle>::from_raw_handle(handle) });
        io::Error::other("a file handle out of range")
    })
}

/// The file at `path` open for reading, as the descriptor the video player takes over (see the Windows one).
#[cfg(unix)]
pub fn player_handle(path: &Path) -> io::Result<i64> {
    use std::os::unix::io::IntoRawFd;
    Ok(i64::from(File::open(path)?.into_raw_fd()))
}

/// The suffix of a lock file, beside the file it locks (`beside(path, LOCK_SUFFIX)`).
pub const LOCK_SUFFIX: &str = ".lock";

/// `path` with `suffix` added to its file name, in the name's own units (`gallery.json` and `.bad` give
/// `gallery.json.bad`; a database and `-wal` its WAL).
pub fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Rename `from` to `to` unless `to` exists, which is then `AlreadyExists` and nothing moves. std's `rename` replaces
/// its target on both systems; this never does, in one call (no window between a check and the rename).
#[cfg(windows)]
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain([0]).collect() };
    let (from, to) = (wide(from), wide(to));
    // SAFETY: both are NUL-terminated and outlive the call. No flag: an existing `to` fails the move
    // (ERROR_ALREADY_EXISTS, which std reads as `AlreadyExists`).
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Rename `from` to `to` unless `to` exists (see the Windows one): `renameat2` with `RENAME_NOREPLACE`. A file system
/// without that flag (`EINVAL`) gets a hard link then the old name's removal, which fails the same way on an existing
/// `to`.
#[cfg(target_os = "linux")]
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(io::Error::other);
    let (c_from, c_to) = (c(from)?, c(to)?);
    // SAFETY: both are NUL-terminated and outlive the call; AT_FDCWD resolves them as std's calls do.
    let done = unsafe {
        libc::renameat2(libc::AT_FDCWD, c_from.as_ptr(), libc::AT_FDCWD, c_to.as_ptr(), libc::RENAME_NOREPLACE)
    };
    if done == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EINVAL) {
        return link_then_unlink(from, to);
    }
    Err(error)
}

/// Rename `from` to `to` unless `to` exists (see the Windows one): a hard link, which fails on an existing `to`, then
/// the old name's removal.
#[cfg(all(unix, not(target_os = "linux")))]
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    link_then_unlink(from, to)
}

/// The move as a hard link of `to` to `from`'s file, then the removal of `from`. When that removal fails the new name is
/// taken back, so an error means the move did not happen: `from` still names the file, and `to` is as it was.
#[cfg(unix)]
fn link_then_unlink(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::hard_link(from, to)?;
    if let Err(error) = std::fs::remove_file(from) {
        // The link was made just above (a hard link never replaces a name), so `to` is the name this call took.
        let _ = std::fs::remove_file(to);
        return Err(error);
    }
    Ok(())
}

/// Move `path` and each of its `parts` (suffixes beside it, such as `-wal`; `""` is `path` itself) aside together, to the
/// first free name beside `path`: `<name><mark>`, then `<name><mark>.1`, and so on. Returns the new path of `path`. The
/// parts move in the order given, so the sidecars go first. A part that isn't there is not moved. A name is taken when
/// any part's name beside it exists. Each move never replaces a file (see [`rename_no_replace`]), so a file another
/// program makes between that check and its move fails the move, and the error names the part. (A preferences file that
/// can't be read is kept so for the user, and an earlier one kept is never lost.) When a move fails, the parts already
/// moved go back to their names, so `path` is never left without the sidecars its changes are in.
pub fn move_aside(path: &Path, parts: &[&str], mark: &str) -> io::Result<PathBuf> {
    let base = (0..=u32::MAX)
        .map(|n| if n == 0 { mark.to_string() } else { format!("{mark}.{n}") })
        .find(|base| parts.iter().all(|part| !beside(path, &format!("{base}{part}")).exists()))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::AlreadyExists, format!("no free name beside {}", path.display()))
        })?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for part in parts {
        let from = beside(path, part);
        if from.exists() {
            let to = beside(path, &format!("{base}{part}"));
            if let Err(e) = rename_no_replace(&from, &to) {
                // Each name moved back is free (its file was just moved off it), so the move back never replaces.
                for (back_from, back_to) in moved.iter().rev() {
                    let _ = rename_no_replace(back_to, back_from);
                }
                return Err(io::Error::new(e.kind(), format!("can't move {} aside: {e}", from.display())));
            }
            moved.push((from, to));
        }
    }
    Ok(beside(path, &base))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    // A rename onto an existing name is refused and moves nothing; onto a free one it moves the file.
    #[test]
    fn a_rename_never_replaces() {
        let dir = crate::testing::run_folder("scratch_platform", "no-replace").unwrap();
        let (from, taken, free) = (dir.join("a"), dir.join("b"), dir.join("c"));
        std::fs::write(&from, b"a").unwrap();
        std::fs::write(&taken, b"b").unwrap();
        let refused = rename_no_replace(&from, &taken).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists, "{refused}");
        assert_eq!(std::fs::read(&taken).unwrap(), b"b", "the target is kept");
        assert_eq!(std::fs::read(&from).unwrap(), b"a", "nothing moved");
        rename_no_replace(&from, &free).unwrap();
        assert_eq!(std::fs::read(&free).unwrap(), b"a");
        assert!(!from.exists());
    }

    // Each file kept aside takes the next free name, so an earlier one kept is never overwritten.
    #[test]
    fn a_file_kept_aside_takes_the_next_free_name() {
        let dir = crate::testing::run_folder("scratch_platform", "keep-aside").unwrap();
        let path = dir.join("gallery.json");
        for (n, want) in [(1, "gallery.json.bad"), (2, "gallery.json.bad.1"), (3, "gallery.json.bad.2")] {
            std::fs::write(&path, format!("{n}")).unwrap();
            assert_eq!(move_aside(&path, &[""], ".bad").unwrap(), dir.join(want));
            assert!(!path.exists());
        }
        assert_eq!(std::fs::read_to_string(dir.join("gallery.json.bad")).unwrap(), "1", "the first one kept is kept");
        assert_eq!(std::fs::read_to_string(dir.join("gallery.json.bad.2")).unwrap(), "3");
    }

    // The video player's handle is the file, open for reading, through a folder whose name ends in a dot.
    #[cfg(windows)]
    #[test]
    fn the_player_handle_is_the_file_in_a_dotted_folder() {
        use std::os::windows::io::FromRawHandle;
        let dir =
            std::path::absolute(crate::testing::run_folder("scratch_platform", "player-handle").unwrap()).unwrap();
        let (dotted, sibling) = (dir.join("a."), dir.join("a"));
        std::fs::create_dir_all(crate::native::verbatim(&dotted)).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(crate::native::verbatim(&dotted.join("v.mp4")), b"dotted").unwrap();
        std::fs::write(sibling.join("v.mp4"), b"sibling").unwrap();
        let handle = player_handle(&crate::native::verbatim(&dotted.join("v.mp4"))).unwrap();
        let raw = std::ptr::without_provenance_mut(usize::try_from(handle).unwrap());
        // SAFETY: the handle was just opened for this test, and is owned by nothing else.
        let file = unsafe { File::from_raw_handle(raw) };
        assert_eq!(io::read_to_string(file).unwrap(), "dotted");
    }

    // The copy's open writes the file in place; other programs may read it meanwhile, and on Windows a writer of theirs
    // is refused until the handle closes. Its size is read from the handle.
    #[test]
    fn a_copy_open_writes_in_place_and_denies_other_writers() {
        let dir = crate::testing::run_folder("scratch_platform", "open-copy").unwrap();
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
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A temporary is removed when its guard drops, unless it was persisted; one never made is no error.
    #[test]
    fn a_temp_file_is_removed_unless_persisted() {
        let dir = crate::testing::run_folder("scratch_platform", "temp-file").unwrap();
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
