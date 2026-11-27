//! The inbox's journal: the user's writes (a mark, a dismissal, "Clear all", a Retry) kept in a file of the app's home,
//! not in the catalog, so that a write is durable before it is acknowledged and does not depend on the catalog's lock.
//!
//! The file is one JSON line per write: `{"id":<n>,"write":<QueuedInboxWrite>}`. A write is appended and flushed
//! (`sync_data`) before its caller is told it went through (`Inbox::journal_write`), so an acknowledged write is on disk. A
//! write leaves the file once the writer has applied it to the catalog, or the catalog refused it (`Journal::applied`): the
//! file is then rewritten without it, through a temporary file renamed over it, and it is empty when none is left. At the
//! start the writes still in the file are replayed (`Writer::replay`). A line that doesn't read (a torn last line, from a
//! crash during an append) was never acknowledged, so it is skipped.
//!
//! When a write to the file fails (disk full, a permission), the write is kept in memory, and the next write rewrites the
//! whole file, so it is still written. Until then it is not durable, which the caller tells the user.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use pg_model::inbox::QueuedInboxWrite;
use serde::{Deserialize, Serialize};

/// One line of the journal, as read.
#[derive(Deserialize)]
struct Line {
    id: u64,
    write: QueuedInboxWrite,
}

/// One line of the journal, as written.
#[derive(Serialize)]
struct Encoded<'a> {
    id: u64,
    write: &'a QueuedInboxWrite,
}

/// A write the journal holds.
struct Entry {
    id: u64,
    write: QueuedInboxWrite,
    /// The writer has applied it (or the catalog refused it): it leaves the file at the next rewrite.
    applied: bool,
}

/// The journal of the user's inbox writes, at `path`. Shared by the handles (which append) and the writer (which marks
/// them applied), under one lock.
pub(crate) struct Journal {
    path: PathBuf,
    /// The id the next write gets: above every id in the file.
    next: u64,
    entries: Vec<Entry>,
    /// The file is exactly the entries that are not applied, flushed, so an append may add one line to it. False after a
    /// failed write (which may have left part of a line), and when the file was read: the next write then rewrites it
    /// (a file read may end in a line without its newline, which an append would join to the next line).
    clean: bool,
}

impl Journal {
    /// The journal at `path`, with the writes the last run left in it (they are not applied). A file that can't be read
    /// is an error: the writes in it are not known, so the app doesn't start the inbox without them. A missing file is
    /// an empty journal, made by the first write.
    pub(crate) fn open(path: PathBuf) -> io::Result<Journal> {
        let mut entries = Vec::new();
        let mut skipped = 0u64;
        match File::open(&path) {
            Ok(file) => {
                for line in BufReader::new(file).split(b'\n') {
                    let line = line?;
                    if line.iter().all(u8::is_ascii_whitespace) {
                        continue;
                    }
                    match serde_json::from_slice::<Line>(&line) {
                        Ok(Line { id, write }) => entries.push(Entry { id, write, applied: false }),
                        Err(_) => skipped += 1,
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        if skipped > 0 {
            pg_log::warn("inbox", &format!("{skipped} unreadable lines of the inbox's journal were skipped"));
        }
        let next = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        Ok(Journal { path, next, entries, clean: false })
    }

    /// The writes not applied yet, oldest first, each with its id: what the start replays.
    pub(crate) fn pending(&self) -> Vec<(u64, QueuedInboxWrite)> {
        self.entries.iter().filter(|e| !e.applied).map(|e| (e.id, e.write.clone())).collect()
    }

    /// Journal `write`: its id, and whether the file holds it now. On an error the write is still kept in memory and
    /// the next write rewrites the file with it; the caller tells the user it is not saved yet.
    pub(crate) fn append(&mut self, write: QueuedInboxWrite) -> (u64, io::Result<()>) {
        let id = self.next;
        self.next += 1;
        self.entries.push(Entry { id, write, applied: false });
        let result = if self.clean { self.write_last() } else { self.rewrite() };
        self.clean = result.is_ok();
        (id, result)
    }

    /// The write `id` was applied to the catalog, or refused: it leaves the file (a rewrite). An id the journal doesn't
    /// hold is nothing to remove.
    pub(crate) fn applied(&mut self, id: u64) -> io::Result<()> {
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) else { return Ok(()) };
        entry.applied = true;
        let result = self.rewrite();
        self.clean = result.is_ok();
        result
    }

    /// Append the last entry's line to the file, flushed.
    fn write_last(&self) -> io::Result<()> {
        let Some(entry) = self.entries.last() else { return Ok(()) };
        let line = encode(entry)?;
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        file.write_all(&line)?;
        file.sync_data()
    }

    /// Rewrite the file with the writes not applied: written to a temporary file, flushed, and renamed over the journal,
    /// so a failure leaves the old file whole. Empty when none is left. The applied ones are dropped from memory.
    fn rewrite(&mut self) -> io::Result<()> {
        let mut text = Vec::new();
        for entry in self.entries.iter().filter(|e| !e.applied) {
            text.extend(encode(entry)?);
        }
        let tmp = self.path.with_extension("tmp");
        {
            let mut file = File::create(&tmp)?;
            file.write_all(&text)?;
            file.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;
        self.entries.retain(|e| !e.applied);
        Ok(())
    }
}

/// `entry` as one line of the journal, with its newline.
fn encode(entry: &Entry) -> io::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(&Encoded { id: entry.id, write: &entry.write }).map_err(io::Error::other)?;
    line.push(b'\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::inbox::InboxEntryId;
    use pg_model::notice::FilePass;

    fn mark(row: i64) -> QueuedInboxWrite {
        QueuedInboxWrite::Mark(vec![InboxEntryId::Row { id: row, uid: row, rev: 0 }])
    }

    fn journal_in(name: &str) -> (PathBuf, PathBuf) {
        let dir = pg_testkit::run_folder("scratch_app", name).unwrap();
        (dir.join(crate::inbox::JOURNAL_FILE), dir)
    }

    // A write is in the file once appended; after it is applied, a restart finds only the writes not applied, with the
    // ids they had, and the next write gets an id above them.
    #[test]
    fn the_writes_not_applied_are_replayed_after_a_restart() {
        let (path, dir) = journal_in("journal-restart");
        let mut journal = Journal::open(path.clone()).unwrap();
        let (first, ok) = journal.append(mark(1));
        assert!(ok.is_ok());
        let (second, ok) = journal.append(QueuedInboxWrite::Retry(FilePass::Thumbs));
        assert!(ok.is_ok());
        journal.applied(first).unwrap();
        drop(journal);
        let mut journal = Journal::open(path.clone()).unwrap();
        assert_eq!(journal.pending(), vec![(second, QueuedInboxWrite::Retry(FilePass::Thumbs))]);
        let (third, _) = journal.append(mark(2));
        assert!(third > second);
        journal.applied(second).unwrap();
        journal.applied(third).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"", "empty when none is left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A torn last line (a crash during an append) is skipped, and the next write rewrites the file without it.
    #[test]
    fn a_torn_line_is_skipped_and_rewritten_away() {
        let (path, dir) = journal_in("journal-torn");
        let good = serde_json::to_string(&Encoded { id: 4, write: &mark(4) }).unwrap();
        std::fs::write(&path, format!("{good}\n{{\"id\":5,\"wri")).unwrap();
        let mut journal = Journal::open(path.clone()).unwrap();
        assert_eq!(journal.pending(), vec![(4, mark(4))]);
        let (id, ok) = journal.append(mark(6));
        assert!(ok.is_ok());
        assert_eq!(id, 5, "above the ids in the file");
        drop(journal);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        let journal = Journal::open(path.clone()).unwrap();
        assert_eq!(journal.pending().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
