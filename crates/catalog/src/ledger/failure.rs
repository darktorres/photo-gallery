//! A file a pass couldn't make its result of. A leaf below `core`: `Catalog`'s methods take it, and the compiler graph
//! puts an inherent method in its type's module, so a type they name has to be below `core` too.

use pg_model::ids::FileId;
use pg_model::turns::Turns;

/// A file a pass couldn't make its result of, as the pass read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub file_id: FileId,
    /// The file's state when the pass read it: the row is kept only while the catalog still has the file so.
    pub mtime: i64,
    pub size: i64,
    pub rotation: Turns,
    pub error: String,
    /// Whether the pass tries the file again by itself (a decode error, which a pass tries until the file changes), or
    /// not until it changes or the user retries it (a panic).
    pub retry: bool,
}
