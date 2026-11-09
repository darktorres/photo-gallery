//! A directory listing's entries and the lister's buffers: what the two backends of `dir` (`unix` and `windows`) fill in.
//! A leaf of `dir`: the backends import these from here, and `dir` re-exports both types, so the public paths do not
//! change.

#[cfg(windows)]
use crate::native::NativePath;
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

/// What listing a folder needs, kept from one folder to the next, so a listing reuses its buffers.
pub struct Lister {
    /// The folder's path, as the call is given it.
    #[cfg(windows)]
    pub(super) wide: NativePath,
    /// The records, as u64s to keep them 8-byte aligned as they must be.
    #[cfg(windows)]
    pub(super) buf: Vec<u64>,
    /// The stable identity of each device seen, by its device number (see `unix::Lister::folder_volume`).
    #[cfg(unix)]
    pub(super) volumes: std::collections::BTreeMap<u64, u64>,
    /// The devices that hold each identity, so a second device with the same one is found.
    #[cfg(unix)]
    pub(super) claimed: std::collections::BTreeMap<u64, u64>,
    /// How many devices were given their device number as their identity (no UUID, or a clone of another's).
    #[cfg(unix)]
    pub(super) fallbacks: u64,
}

#[cfg(windows)]
impl Lister {
    pub fn new() -> Lister {
        Lister { wide: NativePath::default(), buf: vec![0; 8192] }
    }

    /// The devices given their device number as identity: none on Windows, where a volume's serial is always there.
    pub fn fallback_volumes(&self) -> u64 {
        0
    }
}

#[cfg(unix)]
impl Lister {
    pub fn new() -> Lister {
        Lister { volumes: Default::default(), claimed: Default::default(), fallbacks: 0 }
    }

    /// The devices given their device number as identity (see `unix::Lister::folder_volume`).
    pub fn fallback_volumes(&self) -> u64 {
        self.fallbacks
    }
}

#[cfg(not(any(windows, unix)))]
impl Lister {
    pub fn new() -> Lister {
        Lister {}
    }
}

impl Default for Lister {
    fn default() -> Self {
        Self::new()
    }
}
