//! The log's file: each record appended as it comes (the file is opened, written in one call and closed, so nothing is
//! held open: a second instance of the app, or a reader, sees every line), and the file rotated past its cap
//! (`gallery.log` to `gallery.log.1` to `gallery.log.2`, the oldest dropped). When the file can't be rotated, it is
//! emptied instead, so the log never grows without bound.

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// The most bytes of a log file before it is rotated.
pub const FILE_CAP: u64 = 5 * 1024 * 1024;

/// How many files the log keeps: the file and its older ones (`.1`, `.2`).
pub const FILES: usize = 3;

/// The log's file at `path`, rotated past `cap` bytes.
#[derive(Debug)]
pub(crate) struct LogFile {
    path: PathBuf,
    cap: u64,
}

impl LogFile {
    pub(crate) fn new(path: PathBuf, cap: u64) -> LogFile {
        LogFile { path, cap }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Append `line`, rotating the file first if it would pass its cap. The caller serialises the calls.
    pub(crate) fn append(&self, line: &str) -> io::Result<()> {
        let len = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        let line_len = u64::try_from(line.len()).unwrap_or(u64::MAX);
        if len > 0 && len.saturating_add(line_len) > self.cap {
            self.rotate()?;
        }
        OpenOptions::new().create(true).append(true).open(&self.path)?.write_all(line.as_bytes())
    }

    /// The older files move up one (`.1` to `.2`, the file to `.1`), the oldest dropped. A file that can't take its new
    /// name (another process has it open without sharing its deletion) is tried once more, then emptied in place: the
    /// lines it held are lost, the cap is kept.
    fn rotate(&self) -> io::Result<()> {
        for n in (1..FILES).rev() {
            let from = if n == 1 { self.path.clone() } else { older(&self.path, n - 1) };
            let to = older(&self.path, n);
            match std::fs::rename(&from, &to) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(_) if n == 1 => {
                    if std::fs::rename(&from, &to).is_err() {
                        OpenOptions::new().write(true).truncate(true).open(&self.path)?;
                    }
                }
                // An older file that can't move is overwritten by the next rotation that can.
                Err(_) => {}
            }
        }
        Ok(())
    }
}

/// The `n`th older file of the log at `path` (`gallery.log.1`).
pub(crate) fn older(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

/// The log's files at `path`, oldest first, as their text (a file that can't be read is left out; a byte that is no
/// UTF-8 is replaced).
pub(crate) fn read_all(path: &Path) -> String {
    let mut text = String::new();
    for n in (0..FILES).rev() {
        let file = if n == 0 { path.to_path_buf() } else { older(path, n) };
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
    use super::*;

    #[test]
    fn past_its_cap_the_file_rotates_and_the_oldest_is_dropped() {
        let dir = pg_testkit::run_folder("scratch_log", "rotate").unwrap();
        let path = dir.join("gallery.log");
        let file = LogFile::new(path.clone(), 100);
        for i in 0..20 {
            file.append(&format!("{i:040}\n")).unwrap();
        }
        // 41 bytes a line, so each file holds two: the newest in the file, the two before in .1, then .2.
        let current = std::fs::read_to_string(&path).unwrap();
        assert_eq!(current, format!("{:040}\n{:040}\n", 18, 19));
        assert_eq!(std::fs::read_to_string(older(&path, 1)).unwrap(), format!("{:040}\n{:040}\n", 16, 17));
        assert_eq!(std::fs::read_to_string(older(&path, 2)).unwrap(), format!("{:040}\n{:040}\n", 14, 15));
        assert!(!older(&path, 3).exists());
        let all = read_all(&path);
        assert_eq!(all.lines().count(), 6);
        assert!(all.starts_with(&format!("{:040}", 14)));
    }

    #[test]
    fn a_line_longer_than_the_cap_is_still_written() {
        let dir = pg_testkit::run_folder("scratch_log", "long").unwrap();
        let path = dir.join("gallery.log");
        let file = LogFile::new(path.clone(), 10);
        file.append("a line longer than the cap\n").unwrap();
        file.append("another\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "another\n");
        assert_eq!(std::fs::read_to_string(older(&path, 1)).unwrap(), "a line longer than the cap\n");
    }
}
