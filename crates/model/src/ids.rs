//! The catalog's row ids, as types: a file's (`files.id`) and a folder's (`folders.id`). They are SQLite's row numbers,
//! so a file's id can't be passed where a folder's is meant. The catalog binds and reads them as numbers ([`get`]).
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
