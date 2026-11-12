//! The catalog's row ids, as types: a file's (`files.id`) and a folder's (`folders.id`). They are SQLite's row numbers,
//! so a file's id can't be passed where a folder's is meant. The catalog binds and reads them as numbers ([`get`]).
//! A file's identity on its volume ([`FileIdentity`]) is the other way a file is named, and it survives moves.
//!
//! [`get`]: FileId::get

/// A file's row in the catalog (`files.id`). Serialized as the number (the preferences' last file, the controller's
/// recordings).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct FileId(i64);

impl FileId {
    pub const fn new(id: i64) -> FileId {
        FileId(id)
    }

    /// The row number, for binding it in a statement or reading it from a row.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A file's identity on its volume: on Windows, the volume's serial number and the file's NTFS file ID; on Unix,
/// the volume and the inode (on Linux the volume is the file system's UUID, `pg_platform::dir`; a device without one
/// keeps its device number, which can change across a restart, and the scan says so). The file ID survives renames and moves, so a moved
/// file keeps its row and its analysis (PLAN §4). On NTFS its low 48 bits are the MFT record number, which the
/// catalog reads its files in (`files.file_ref & 0xFFFFFFFFFFFF` in SQL: a locality order, a heuristic that is
/// harmless where the file system doesn't follow it). An inode can be reused after a file is deleted; the size and
/// mtime the catalog also keeps tell the scanner when a row is a different file.
///
/// The volume is 64 bits on both platforms: the NTFS `FileIdInfo` serial, or on Unix the file system's UUID (the device number where there is none). A 32-bit
/// serial is not unique (two NTFS volumes on one machine can share its low 32 bits), so a catalog from before the
/// 64-bit volume is moved aside and rebuilt, not converted (S-17.7 decision 1).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FileIdentity {
    pub volume: u64,
    pub file_index: u64,
}

impl FileIdentity {
    pub const fn new(volume: u64, file_index: u64) -> FileIdentity {
        FileIdentity { volume, file_index }
    }
}

/// A folder's row in the catalog (`folders.id`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FolderId(i64);

impl FolderId {
    pub const fn new(id: i64) -> FolderId {
        FolderId(id)
    }

    /// The row number, for binding it in a statement or reading it from a row.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A scan's row in the catalog (`scans.id`): the scan a file was last seen by.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ScanId(i64);

impl ScanId {
    pub const fn new(id: i64) -> ScanId {
        ScanId(id)
    }

    /// The row number, for binding it in a statement or reading it from a row.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A library folder's row in the catalog (`roots.id`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RootId(i64);

impl RootId {
    pub const fn new(id: i64) -> RootId {
        RootId(id)
    }

    /// The row number, for binding it in a statement or reading it from a row.
    pub const fn get(self) -> i64 {
        self.0
    }
}
