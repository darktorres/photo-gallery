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

/// A file's identity on its volume: the volume's serial number and the file's NTFS file ID. The file ID survives
/// renames and moves, so a moved file keeps its row and its analysis (PLAN §4). Its low 48 bits are the MFT record
/// number, which the catalog reads its files in.
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
