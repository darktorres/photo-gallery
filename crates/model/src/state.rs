//! What a reader checks a file against: its state (`FileState`), and what a thumbnail or the viewer needs of it
//! (`FileSource`, `Placeholder`). The catalog reads them (`pg_catalog::browse::read`); the readers' checks are plain
//! comparisons of these values.

use crate::analysis::ThumbHash;
use crate::rules;
use crate::turns::Turns;

/// What a file's images are made from, as the catalog has it now. What is made from the file (a thumbnail, a
/// decode, an analysis) is kept under the state it was read in, checked before and after the read
/// ([`FileState::writing`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileState {
    /// With `rotation`, what the thumbnail cache keeps a file's thumbnails under (`thumbs::Source`). The last write time in
    /// the platform listing's unit: FILETIME ticks (100 ns since 1601-01-01 UTC, `pg_platform::dir::listing`'s `mtime`).
    pub mtime: i64,
    pub rotation: Turns,
    /// The file's size in bytes (`files.size`): with `mtime`, what the scanner decides a file changed by.
    pub size: u64,
    /// Quarter turns clockwise asked for and not yet written (`queue_rotation`): what is made from the file is shown
    /// turned by them. Its name is `pending_turns`, as the grid's entry and `Version` name the same count; `rotation`
    /// above is the stored count.
    pub pending_turns: Turns,
    /// A write of the app's into the file is in progress (`file_writes.temp`, until the transaction that records
    /// it): the file may be part-written, or hold a write the catalog doesn't show yet.
    pub writing: bool,
}

/// What the image providers need to make a file's images.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileSource {
    /// The file's path, exact: what the image providers open (through the app's `file_path`).
    pub path: rules::Path,
    pub state: FileState,
    /// For a video, the time of the frame that stands for it (`video::poster_ms`; `None` before the import).
    pub poster_ms: Option<i64>,
}

/// A file's ThumbHash placeholder, and the quarter turns clockwise that show it as the file is shown now: the
/// turns asked for and not yet written, and those kept in the catalog since the analysis that made the hash.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placeholder {
    /// `None` before the import.
    pub hash: Option<ThumbHash>,
    /// The quarter turns the file is shown turned by (its stored turns, plus the pending ones).
    pub shown_turns: Turns,
}
