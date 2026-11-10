//! A directory listing's entry: what the two backends of `dir` (`unix` and `windows`) fill in, each with its own
//! `Lister`. A leaf of `dir`: the backends import it from here, and `dir` re-exports it, so its public path does not
//! change.

use crate::native::Unit;

/// A directory entry as the listing gives it. The name is borrowed from the lister's buffer until the next entry.
pub struct Entry<'a> {
    /// The name as the file system stores it: UTF-16 units on Windows, bytes elsewhere (exact in both).
    pub name: &'a [Unit],
    /// A folder rather than a file.
    pub folder: bool,
    /// A reparse point (a junction or a symbolic link): a listing doesn't follow it.
    pub reparse: bool,
    /// The file's identity on its volume (NTFS's file ID, which survives renames and moves). 0 when there is none.
    pub file_id: u64,
    /// The size in bytes.
    pub size: u64,
    /// The last write time in FILETIME ticks: 100 ns since 1601-01-01 UTC.
    pub mtime: i64,
    /// The file's own volume, read with the entry (Unix: the stable identity of its device's volume, see
    /// `Lister::folder_volume`; a mount under a folder has another).
    /// `None` where every entry of the folder is on the folder's volume (Windows).
    pub volume: Option<u64>,
    /// The creation time in FILETIME ticks; `None` where the file system or the platform doesn't give one (drvfs has
    /// none). Never an error: an unknown creation time is a NULL in the catalog.
    pub created: Option<i64>,
}
