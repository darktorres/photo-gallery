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
    /// With `rotation`, what the thumbnail cache keeps a file's thumbnails under (`thumbs::Source`).
    pub mtime: i64,
    pub rotation: Turns,
    /// The file's size (`files.size`): with `mtime`, what the scanner decides a file changed by.
    pub size: i64,
    /// Quarter turns clockwise asked for and not yet written (`queue_rotation`): what is made from the file is shown
    /// turned by them.
    pub turns: Turns,
    /// A write of the app's into the file is in progress (`file_writes.temp`, until the transaction that records
    /// it): the file may be part-written, or hold a write the catalog doesn't show yet.
    pub writing: bool,
}

/// What the image providers need to make a file's images.
#[derive(Clone, PartialEq, Debug)]
pub struct FileSource {
    /// The file's path, exact: what the image providers open (through the app's `file_path`).
    pub path: rules::Path,
    pub state: FileState,
    /// For a video, the time of the frame that stands for it (`video::poster_ms`; 0 before the import).
    pub poster_ms: Option<i64>,
}

/// A file's ThumbHash placeholder, and the quarter turns clockwise that show it as the file is shown now: the
/// turns asked for and not yet written, and those kept in the catalog since the analysis that made the hash.
pub struct Placeholder {
    /// `None` before the import.
    pub hash: Option<ThumbHash>,
    pub turns: Turns,
}
