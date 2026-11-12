//! The lock files of the test folders (see the parent module). A lock is a file its holder keeps open: unshared on Windows,
//! `flock`ed on Unix, so no other open takes the file while the holder lives. Each function's name says what it is for:
//! a claim creates a lock, a cleanup creates or takes one to remove what it guards.

use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
use super::seam::{Step, seam};
use std::sync::atomic::{AtomicU64, Ordering};

/// How many names a claim tries before it gives up.
pub(super) const ATTEMPTS: usize = 1000;

/// Numbers the claims this process makes, so that every call gets a name of its own (a run folder's and a claim's
/// hidden name alike).
static CLAIMS: AtomicU64 = AtomicU64::new(0);

/// The next number for a name a claim makes: one per call in this process.
pub(super) fn next_number() -> u64 {
    CLAIMS.fetch_add(1, Ordering::Relaxed)
}

/// A lock on a file. It is held until it drops, and a lock from [`create_for_delete`] or [`take_for_delete`] removes its
/// name when it goes ([`HeldLock::delete`]).
#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct HeldLock {
    file: fs::File,
    path: std::path::PathBuf,
}

/// A lock on a file. It is held until it drops; the name goes with the last handle when `delete_on_close` (see
/// [`open_for_delete`]).
#[cfg(windows)]
#[derive(Debug)]
pub(crate) struct HeldLock {
    file: fs::File,
    delete_on_close: bool,
}

/// The Win32 values the lock needs (winnt.h and winerror.h), which std does not name.
#[cfg(windows)]
mod win {
    pub(super) const DELETE: u32 = 0x0001_0000;
    pub(super) const GENERIC_READ: u32 = 0x8000_0000;
    pub(super) const GENERIC_WRITE: u32 = 0x4000_0000;
    pub(super) const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
    pub(super) const ERROR_ACCESS_DENIED: i32 = 5;
    pub(super) const ERROR_SHARING_VIOLATION: i32 = 32;
}

/// A new lock on `path`, held by the caller. Fails with `AlreadyExists` if the name is taken, whether by a held lock or
/// by a file left behind. A claim's lock outlives its process as a file nothing holds: a cleanup takes it.
#[cfg(windows)]
pub(crate) fn create_held(path: &Path) -> io::Result<HeldLock> {
    use std::os::windows::fs::OpenOptionsExt;
    // No sharing at all: any other open of the name fails while this handle is open.
    let file = fs::OpenOptions::new().write(true).create_new(true).share_mode(0).open(path)?;
    Ok(HeldLock { file, delete_on_close: false })
}

/// A new lock on `path` for a cleanup, held by the caller, whose name is removed when the lock is deleted (Windows: when
/// the last handle closes).
#[cfg(windows)]
pub(crate) fn create_for_delete(path: &Path) -> io::Result<HeldLock> {
    open_for_delete(path, true)
}

/// Opens `path` for a cleanup: unshared, with DELETE access and delete-on-close, so the name goes when the handle does.
/// `create` makes the file, and fails if it is there.
#[cfg(windows)]
fn open_for_delete(path: &Path, create: bool) -> io::Result<HeldLock> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options
        .access_mode(win::GENERIC_READ | win::GENERIC_WRITE | win::DELETE)
        .share_mode(0)
        .custom_flags(win::FILE_FLAG_DELETE_ON_CLOSE);
    if create {
        options.create_new(true);
    }
    Ok(HeldLock { file: options.open(path)?, delete_on_close: true })
}

/// The lock on the file at `path`, taken for a cleanup, or `None` if another holds it. Fails if there is no file.
#[cfg(windows)]
pub(crate) fn take_for_delete(path: &Path) -> io::Result<Option<HeldLock>> {
    match open_for_delete(path, false) {
        Ok(lock) => Ok(Some(lock)),
        // Another open of the file holds it (a sharing violation), or it is being deleted (access denied): not ours.
        Err(err) if matches!(err.raw_os_error(), Some(win::ERROR_SHARING_VIOLATION | win::ERROR_ACCESS_DENIED)) => {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

#[cfg(windows)]
impl HeldLock {
    /// Removes the lock's name. On Windows the name goes when the handle closes, which is the case for a lock from
    /// [`create_for_delete`] or [`take_for_delete`]; a claim's lock is kept, and deleting it is an error.
    pub(crate) fn delete(self) -> io::Result<()> {
        if !self.delete_on_close {
            return Err(io::Error::other("a claim's lock is kept, not deleted"));
        }
        drop(self.file);
        Ok(())
    }
}

/// A new lock on `path`, held by the caller. Fails with `AlreadyExists` if the name is taken. The file is made under a
/// hidden name first, locked, and then linked to `path`: a link fails if the name is taken, and the lock is held from
/// the moment the name is there.
#[cfg(unix)]
pub(crate) fn create_held(path: &Path) -> io::Result<HeldLock> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(std::ffi::OsStr::to_str)) else {
        return Err(io::Error::other(format!("{} has no folder and name", path.display())));
    };
    for _ in 0..ATTEMPTS {
        let n = next_number();
        let hidden = dir.join(format!(".{name}.claim-{}-{n}", std::process::id()));
        let file = match fs::OpenOptions::new().write(true).create_new(true).open(&hidden) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };
        seam(Step::ClaimHidden(&hidden));
        let linked = match file.lock() {
            Ok(()) => fs::hard_link(&hidden, path),
            Err(err) => Err(err),
        };
        // The hidden name goes either way: a cleanup may have taken it already.
        let _ = fs::remove_file(&hidden);
        match linked {
            Ok(()) => return Ok(HeldLock { file, path: path.to_path_buf() }),
            // A cleanup took the hidden name before the lock: this claim starts over with the next number.
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::other(format!("no free claim for {} after {ATTEMPTS} tries", path.display())))
}

/// A new lock on `path` for a cleanup, held by the caller, whose name is removed when the lock is deleted.
#[cfg(unix)]
pub(crate) fn create_for_delete(path: &Path) -> io::Result<HeldLock> {
    create_held(path)
}

/// The lock on the file at `path`, taken for a cleanup, or `None` if another holds it. Fails if there is no file. The
/// file taken is the one at `path` when it was locked: a file made in its place since is not taken.
#[cfg(unix)]
pub(crate) fn take_for_delete(path: &Path) -> io::Result<Option<HeldLock>> {
    use std::fs::TryLockError;
    use std::os::unix::fs::MetadataExt;
    let file = fs::File::open(path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(err)) => return Err(err),
    }
    seam(Step::TakeLocked(path));
    let named = fs::symlink_metadata(path)?;
    let opened = file.metadata()?;
    if named.dev() != opened.dev() || named.ino() != opened.ino() {
        return Ok(None);
    }
    Ok(Some(HeldLock { file, path: path.to_path_buf() }))
}

#[cfg(unix)]
impl HeldLock {
    /// Removes the lock's name while the lock is still held (so no other claim can take the name and have it removed
    /// under it), then closes the file.
    pub(crate) fn delete(self) -> io::Result<()> {
        let removed = fs::remove_file(&self.path);
        seam(Step::BeforeClose(&self.path));
        drop(self.file);
        removed
    }
}

/// The lock a cleanup takes on `path`: made if the name is free, else taken from the file that is there. `None` if
/// another holds it.
pub(crate) fn acquire_for_delete(path: &Path) -> io::Result<Option<HeldLock>> {
    match create_for_delete(path) {
        Ok(lock) => Ok(Some(lock)),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => take_for_delete(path),
        Err(err) => Err(err),
    }
}
