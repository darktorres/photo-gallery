//! Replacing an original's bytes in place, for the borders cut off it (`pg_crop::cut`). The new content is copied into the
//! original, not renamed over it: a rename would make the file a new one to NTFS, and the catalog keys its files by that
//! identity (see the module doc of `lib`). The steps are those of a queued write: the new content is written and synced
//! to a temp beside the original; the original is opened for the copy, denying other writers, and checked to be as it was
//! read; only then is the content copied in and the temp removed.
//!
//! The copy in place is not atomic. A failure before it starts leaves the original as it was ([`ReplaceError::Untouched`]).
//! A failure during it leaves the original part-written, and the whole new content in the temp, which the error names
//! ([`ReplaceError::Incomplete`]).

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pg_platform::native::NativePath;

use crate::file::{stat, sync, unchanged, write_into};

/// What a replacement failed with.
#[derive(Debug)]
pub enum ReplaceError {
    /// Before the copy into the original started: the original is as it was, and the temp is gone.
    Untouched(anyhow::Error),
    /// The copy may have started: the original may be part-written. `temp` holds the whole new content, and is kept.
    Incomplete { temp: PathBuf, error: anyhow::Error },
}

/// Replace the original at `path` (its disk path) with `bytes`, made from the original as it was when its (size, mtime)
/// were `before`. `native` is the same path for the system's open that denies other writers, and `temp` the temp beside
/// the original. Returns the original's (size, mtime) after the copy.
pub fn replace(
    path: &Path,
    native: &NativePath,
    temp: &Path,
    bytes: &[u8],
    before: (i64, i64),
) -> std::result::Result<(i64, i64), ReplaceError> {
    let file = match prepare(path, native, temp, bytes, before) {
        Ok(file) => file,
        Err(e) => {
            let _ = std::fs::remove_file(temp);
            return Err(ReplaceError::Untouched(e));
        }
    };
    let incomplete = |error: anyhow::Error| ReplaceError::Incomplete { temp: temp.to_path_buf(), error };
    write_into(file, bytes).map_err(incomplete)?;
    let after = stat(path).map_err(incomplete)?;
    let _ = std::fs::remove_file(temp);
    Ok(after)
}

/// The steps before the copy: the original as it was read, the new content written and synced to `temp`, and the
/// original opened for the copy and checked again under its lock. The file is returned open for the copy.
fn prepare(path: &Path, native: &NativePath, temp: &Path, bytes: &[u8], before: (i64, i64)) -> Result<File> {
    let shown = pg_platform::names::shown(path);
    unchanged(&shown, before, stat(path)?)?;
    std::fs::write(temp, bytes).with_context(|| format!("writing {}", pg_platform::names::shown(temp)))?;
    sync(temp).with_context(|| format!("writing {}", pg_platform::names::shown(temp)))?;
    let file = pg_platform::file::open_for_copy(native).with_context(|| format!("opening {shown} for writing"))?;
    unchanged(&shown, before, pg_platform::file::stat_of(&file)?)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder with an original in it (`original`), its (size, mtime), and the native path of the original.
    fn original(name: &str, content: &[u8]) -> (PathBuf, PathBuf, (i64, i64), NativePath) {
        let dir = pg_testkit::run_folder("scratch_writeback_inplace", name).unwrap();
        let path = dir.join("a.bin");
        std::fs::write(&path, content).unwrap();
        let before = pg_platform::file::stat(&path).unwrap();
        let mut native = NativePath::default();
        pg_platform::names::set_path(&mut native, path.as_os_str().as_encoded_bytes());
        (dir, path, before, native)
    }

    /// The copy replaces the original's bytes, keeps the file (the same path, no temp left), and reports its new time.
    #[test]
    fn the_copy_replaces_the_original_in_place() {
        let (dir, path, before, native) = original("inplace-copy", b"original bytes");
        let temp = dir.join(".a.bin.cut.pg-write");
        let after = replace(&path, &native, &temp, b"cut", before).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"cut");
        assert!(!temp.exists(), "the temp is removed once the copy is in");
        assert_eq!(after, pg_platform::file::stat(&path).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An original changed since it was read is not written; nor is it touched when the temp can't be written.
    #[test]
    fn a_changed_or_unwritable_original_is_left_as_it_is() {
        let (dir, path, before, native) = original("inplace-changed", b"original bytes");
        // Changed since it was read (a size of its own): refused before anything is written.
        let stale = (before.0 + 1, before.1);
        let temp = dir.join(".a.bin.cut.pg-write");
        let err = replace(&path, &native, &temp, b"cut", stale).unwrap_err();
        assert!(matches!(err, ReplaceError::Untouched(_)), "{err:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"original bytes");
        // The temp can't be made (a folder sits where it would go): untouched, and the error says so.
        let blocked = dir.join(".a.bin.cut.pg-write");
        std::fs::create_dir(&blocked).unwrap();
        let err = replace(&path, &native, &blocked, b"cut", before).unwrap_err();
        assert!(matches!(err, ReplaceError::Untouched(_)), "{err:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"original bytes");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An original another program holds open with a share mode that denies writers is not opened for the copy: the
    /// attempt fails before the copy, the original is intact, and the temp is removed.
    #[cfg(windows)]
    #[test]
    fn an_original_held_by_another_program_is_left_intact() {
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, path, before, native) = original("inplace-held", b"original bytes");
        // FILE_SHARE_READ only: no other writer may open it while this handle is open.
        let holder = std::fs::OpenOptions::new().read(true).share_mode(0x1).open(&path).unwrap();
        let temp = dir.join(".a.bin.cut.pg-write");
        let err = replace(&path, &native, &temp, b"cut", before).unwrap_err();
        assert!(matches!(err, ReplaceError::Untouched(_)), "{err:?}");
        assert!(!temp.exists(), "the temp is removed");
        drop(holder);
        assert_eq!(std::fs::read(&path).unwrap(), b"original bytes");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
