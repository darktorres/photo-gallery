//! The catalog's row ids, as types: a file's (`files.id`) and a folder's (`folders.id`). They are SQLite's row numbers,
//! so a file's id can't be passed where a folder's is meant. The catalog binds and reads them as numbers ([`get`]).
//! A file's identity on its volume ([`FileIdentity`]) is the other way a file is named, and it survives moves.
//!
//! [`get`]: FileId::get

/// A file's row in the catalog (`files.id`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
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
/// the device and the inode (the device truncated to 32 bits, which the platform does, so two devices could in
/// principle share one; a catalog-assigned volume id would remove that, and the `dev` number itself can change
/// across reboots, which is an open item for the Linux build). The file ID survives renames and moves, so a moved
/// file keeps its row and its analysis (PLAN §4). On NTFS its low 48 bits are the MFT record number, which the
/// catalog reads its files in (`files.file_ref & 0xFFFFFFFFFFFF` in SQL: a locality order, a heuristic that is
/// harmless where the file system doesn't follow it). An inode can be reused after a file is deleted; the size and
/// mtime the catalog also keeps tell the scanner when a row is a different file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileIdentity {
    pub volume: u32,
    pub file_index: u64,
}

impl FileIdentity {
    pub const fn new(volume: u32, file_index: u64) -> FileIdentity {
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
