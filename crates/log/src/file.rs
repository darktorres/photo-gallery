//! The log's file: each record appended as it comes (the file is opened, written in one call and closed, so nothing is
//! held open: a second instance of the app, or a reader, sees every line), and the file rotated past its cap
//! (`gallery.log` to `gallery.log.1` to `gallery.log.2`, the oldest dropped).
//!
//! A rotation never sleeps, and loses no line in normal operation:
//! - **The renames keep history safe.** The file is first renamed aside (`gallery.log.next`); only when that worked is
//!   the older file shifted (`.1` to `.2`), then the aside file becomes `.1`. A failed first rename shifts nothing, so
//!   a retry can't overwrite an older file.
//! - **Rotations are serialised between processes** by an OS lock on `gallery.log.lock` (std's `File::try_lock`, as
//!   `pg_dbfile::lock` does: released by the OS when a process ends). A process that finds it taken skips the rotation
//!   for this record; the holder is rotating.
//! - **An aside file left over** (a crash between the renames) is unambiguous under the lock, and is finished first.
//!   The reader reads it meanwhile, in its place between `.1` and the file.
//! - **When the file can't be renamed** (another process has it open without sharing its deletion), the rotation is
//!   skipped for this record and tried at the next: the file runs a few lines past its cap. Only once it has grown
//!   past twice the cap (the rename fails persistently) is it copied aside and emptied in place, which can lose a line
//!   another instance appends between the copy and the emptying: the one loss, documented in docs/NOTIFICATIONS.md.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use pg_platform::file::beside;

/// The most bytes of a log file before it is rotated.
pub const FILE_CAP: u64 = 5 * 1024 * 1024;

/// How many files the log keeps: the file and its older ones (`.1`, `.2`).
pub const FILES: usize = 3;

/// How a file is renamed: std's `rename` (which replaces its target), or a test's.
type Rename = Box<dyn Fn(&Path, &Path) -> io::Result<()> + Send + Sync>;

/// The log's file at `path`, rotated past `cap` bytes.
pub(crate) struct LogFile {
    path: PathBuf,
    cap: u64,
    /// The seam a test makes a rename fail through.
    rename: Rename,
}

impl LogFile {
    pub(crate) fn new(path: PathBuf, cap: u64) -> LogFile {
        LogFile { path, cap, rename: Box::new(rename) }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Append `line`, rotating the file first if it would pass its cap (or finishing a rotation a crash left). The
    /// caller serialises the calls of this process; the lock file serialises the rotations of every process.
    pub(crate) fn append(&self, line: &str) -> io::Result<()> {
        let line_len = u64::try_from(line.len()).unwrap_or(u64::MAX);
        if self.due(line_len) || aside(&self.path).exists() {
            self.rotate(line_len);
        }
        OpenOptions::new().create(true).append(true).open(&self.path)?.write_all(line.as_bytes())
    }

    /// The file's length now.
    fn len(&self) -> u64 {
        std::fs::metadata(&self.path).map_or(0, |m| m.len())
    }

    /// Whether the file, with `line_len` more bytes, would pass its cap (an empty file takes any line).
    fn due(&self, line_len: u64) -> bool {
        let len = self.len();
        len > 0 && len.saturating_add(line_len) > self.cap
    }

    /// Rotate, under the lock between processes. Whatever fails, the record is still appended (to a file past its cap
    /// at worst): a rotation is skipped, never waited for.
    fn rotate(&self, line_len: u64) {
        let Some(_lock) = self.lock() else { return };
        // A rotation a crash left half done goes first: the aside file is the newest of the older ones.
        if aside(&self.path).exists() && self.finish().is_err() {
            return;
        }
        // Another process may have rotated since this one looked.
        if !self.due(line_len) {
            return;
        }
        match (self.rename)(&self.path, &aside(&self.path)) {
            Ok(()) => {
                // A shift that fails leaves the aside file, which the next append finishes.
                let _ = self.finish();
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            // The file can't be renamed: tried again at the next record, unless it has grown past twice its cap.
            Err(_) if self.len() > self.cap.saturating_mul(2) => {
                if self.copy_aside().is_ok() {
                    let _ = self.finish();
                }
            }
            Err(_) => {}
        }
    }

    /// The lock that serialises the rotations of every process writing this log, or `None` when another holds it or
    /// it can't be had (the rotation is then skipped).
    fn lock(&self) -> Option<File> {
        let file =
            OpenOptions::new().create(true).write(true).truncate(false).open(beside(&self.path, ".lock")).ok()?;
        match file.try_lock() {
            Ok(()) => Some(file),
            Err(TryLockError::WouldBlock | TryLockError::Error(_)) => None,
        }
    }

    /// The rest of a rotation, once the file is aside: the older files shift up one (`.1` to `.2`, the oldest
    /// dropped), then the aside file is `.1`. Each step only after the one before worked.
    fn finish(&self) -> io::Result<()> {
        let next = aside(&self.path);
        for n in (2..FILES).rev() {
            match (self.rename)(&older(&self.path, n - 1), &older(&self.path, n)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        (self.rename)(&next, &older(&self.path, 1))
    }

    /// The fallback for a file that can't be renamed: its text copied aside, then the file emptied in place. A line
    /// another process appends between the two is lost.
    fn copy_aside(&self) -> io::Result<()> {
        std::fs::copy(&self.path, aside(&self.path))?;
        OpenOptions::new().write(true).truncate(true).open(&self.path).map(drop)
    }
}

/// A rename as the app does it: std's, which replaces its target.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

/// The `n`th older file of the log at `path` (`gallery.log.1`).
pub(crate) fn older(path: &Path, n: usize) -> PathBuf {
    beside(path, &format!(".{n}"))
}

/// The file a rotation renames the log at `path` to first (`gallery.log.next`), before the older files shift.
fn aside(path: &Path) -> PathBuf {
    beside(path, ".next")
}

/// The log's files at `path`, oldest first, as their text (a file that can't be read is left out; a byte that is no
/// UTF-8 is replaced). An aside file a rotation left is newer than `.1` and older than the file.
pub(crate) fn read_all(path: &Path) -> String {
    let mut files: Vec<PathBuf> = (1..FILES).rev().map(|n| older(path, n)).collect();
    files.push(aside(path));
    files.push(path.to_path_buf());
    let mut text = String::new();
    for file in files {
        if let Ok(bytes) = std::fs::read(&file) {
            text.push_str(&String::from_utf8_lossy(&bytes));
            if !text.ends_with('\n') && !text.is_empty() {
                text.push('\n');
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn folder(name: &str) -> PathBuf {
        pg_testkit::run_folder("scratch_log", name).unwrap().join("gallery.log")
    }

    /// A log file whose renames of the file itself (not of the older files) fail while `failing` is above 0, counted
    /// down by each.
    fn failing_file(path: &Path, cap: u64, failing: &Arc<AtomicUsize>) -> LogFile {
        let failing = Arc::clone(failing);
        let head = path.to_path_buf();
        let rename: Rename = Box::new(move |from: &Path, to: &Path| {
            if from == head.as_path() && failing.load(Ordering::SeqCst) > 0 {
                failing.fetch_sub(1, Ordering::SeqCst);
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "held by another process"));
            }
            std::fs::rename(from, to)
        });
        LogFile { path: path.to_path_buf(), cap, rename }
    }

    fn line(i: usize) -> String {
        format!("{i:040}\n")
    }

    #[test]
    fn past_its_cap_the_file_rotates_and_the_oldest_is_dropped() {
        let path = folder("rotate");
        let file = LogFile::new(path.clone(), 100);
        for i in 0..20 {
            file.append(&line(i)).unwrap();
        }
        // 41 bytes a line, so each file holds two: the newest in the file, the two before in .1, then .2.
        let current = std::fs::read_to_string(&path).unwrap();
        assert_eq!(current, format!("{}{}", line(18), line(19)));
        assert_eq!(std::fs::read_to_string(older(&path, 1)).unwrap(), format!("{}{}", line(16), line(17)));
        assert_eq!(std::fs::read_to_string(older(&path, 2)).unwrap(), format!("{}{}", line(14), line(15)));
        assert!(!older(&path, 3).exists());
        assert!(!aside(&path).exists());
        let all = read_all(&path);
        assert_eq!(all.lines().count(), 6);
        assert!(all.starts_with(line(14).trim_end()));
    }

    #[test]
    fn a_line_longer_than_the_cap_is_still_written() {
        let path = folder("long");
        let file = LogFile::new(path.clone(), 10);
        file.append("a line longer than the cap\n").unwrap();
        file.append("another\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "another\n");
        assert_eq!(std::fs::read_to_string(older(&path, 1)).unwrap(), "a line longer than the cap\n");
    }

    /// A file that can't be renamed once is not rotated for that record, and nothing older moves; the next record
    /// rotates it, and no line is lost.
    #[test]
    fn a_failed_rename_skips_the_rotation_and_the_next_record_rotates() {
        let path = folder("skip");
        let failing = Arc::new(AtomicUsize::new(0));
        let file = failing_file(&path, 100, &failing);
        for i in 0..4 {
            file.append(&line(i)).unwrap();
        }
        // The file holds 2 and 3, `.1` holds 0 and 1.
        failing.store(1, Ordering::SeqCst);
        file.append(&line(4)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{}{}{}", line(2), line(3), line(4)));
        assert_eq!(std::fs::read_to_string(older(&path, 1)).unwrap(), format!("{}{}", line(0), line(1)));
        assert!(!older(&path, 2).exists(), "nothing older moved");
        file.append(&line(5)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), line(5));
        assert_eq!(std::fs::read_to_string(older(&path, 1)).unwrap(), format!("{}{}{}", line(2), line(3), line(4)));
        assert_eq!(std::fs::read_to_string(older(&path, 2)).unwrap(), format!("{}{}", line(0), line(1)));
        let all: Vec<String> = read_all(&path).lines().map(str::to_string).collect();
        assert_eq!(all, (0..6).map(|i| line(i).trim_end().to_string()).collect::<Vec<_>>());
    }

    /// A file that can't be renamed at all grows past its cap, until twice the cap: then it is copied aside and
    /// emptied, and the history shifts as for a rename.
    #[test]
    fn a_rename_that_keeps_failing_falls_back_to_copy_and_empty_past_twice_the_cap() {
        let path = folder("fallback");
        let failing = Arc::new(AtomicUsize::new(usize::MAX));
        let file = failing_file(&path, 100, &failing);
        // 41 bytes a line: past 200 bytes after the fifth.
        for i in 0..5 {
            file.append(&line(i)).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 5, "past the cap, not rotated");
        assert!(!older(&path, 1).exists());
        file.append(&line(5)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), line(5));
        assert_eq!(std::fs::read_to_string(older(&path, 1)).unwrap().lines().count(), 5);
        assert!(!aside(&path).exists());
        let all: Vec<String> = read_all(&path).lines().map(str::to_string).collect();
        assert_eq!(all, (0..6).map(|i| line(i).trim_end().to_string()).collect::<Vec<_>>());
    }

    /// A crash between the renames leaves the aside file: it is read in its place, and finished before the next
    /// rotation, so nothing is overwritten.
    #[test]
    fn a_rotation_left_half_done_is_read_and_finished_first() {
        let path = folder("leftover");
        std::fs::write(older(&path, 2), line(0)).unwrap();
        std::fs::write(older(&path, 1), line(1)).unwrap();
        std::fs::write(aside(&path), line(2)).unwrap();
        std::fs::write(&path, line(3)).unwrap();
        let all: Vec<String> = read_all(&path).lines().map(str::to_string).collect();
        assert_eq!(all, (0..4).map(|i| line(i).trim_end().to_string()).collect::<Vec<_>>());

        let file = LogFile::new(path.clone(), 1000);
        file.append(&line(4)).unwrap();
        assert!(!aside(&path).exists());
        assert_eq!(std::fs::read_to_string(older(&path, 2)).unwrap(), line(1), "the oldest dropped");
        assert_eq!(std::fs::read_to_string(older(&path, 1)).unwrap(), line(2));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{}{}", line(3), line(4)));
    }

    /// While another process holds the rotation's lock, a rotation is skipped (the record is still written), and
    /// taken once it is let go.
    #[test]
    fn a_rotation_held_by_another_is_skipped() {
        let path = folder("locked");
        let file = LogFile::new(path.clone(), 100);
        for i in 0..2 {
            file.append(&line(i)).unwrap();
        }
        let other = OpenOptions::new().create(true).write(true).truncate(false).open(beside(&path, ".lock")).unwrap();
        other.try_lock().unwrap();
        file.append(&line(2)).unwrap();
        assert!(!older(&path, 1).exists(), "skipped while held");
        drop(other);
        file.append(&line(3)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), line(3));
        assert_eq!(std::fs::read_to_string(older(&path, 1)).unwrap(), format!("{}{}{}", line(0), line(1), line(2)));
    }
}
