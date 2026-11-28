//! A queued write as the writer takes it, and what a finished one changed. A leaf below `core` (the rule is in the
//! crate's docs).

use pg_model::ids::FileId;
use pg_model::rating::Rating;
use pg_model::rules::Path as RulePath;
use pg_model::turns::Turns;
use pg_model::write::{Write, WriteKind};

/// A queued write, as the writer takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// The file the write is for.
    pub file_id: FileId,
    /// The rating, or the quarter turns still to write.
    pub write: Write,
    /// Set once the copy of the new content into the original may have started (the temp is complete, and the
    /// original is open for it): the temp's path, and the write it carries (of the job's kind).
    pub temp: Option<(RulePath, Write)>,
    /// Failed attempts so far.
    pub attempts: i64,
}

impl Job {
    /// What the job writes: a rating, or turns.
    pub fn kind(&self) -> WriteKind {
        self.write.kind()
    }
}

/// A file a write rewrote: its (size, mtime) on disk before (`None` when it isn't known) and after, and the BLAKE3 of
/// the new bytes when only metadata changed (the analysis stays valid, and follows the file). `None` when the pixels
/// did, so the analysis is redone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rewrite {
    /// The file's size and mtime before the rewrite, if it was known.
    pub before: Option<(u64, i64)>,
    /// The file's size and mtime after the rewrite.
    pub after: (u64, i64),
    /// The file's hash when only its metadata changed.
    pub metadata_only: Option<pg_model::analysis::FileHash>,
}

/// What a finished write changed in the catalog. Applied in the transaction that takes the job off the queue
/// (`done`), so the catalog and the queue never disagree. The effect's value is the job's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteEffect {
    /// The rating is in the file, which was rewritten to hold it (`None`: it holds none).
    FileRating {
        /// The rewrite that holds the rating.
        rewrite: Rewrite,
        /// The rating the file now holds, or `None` for unrated.
        rating: Option<Rating>,
    },
    /// The turns are in the file, which was rewritten to show them: the analysis is turned back to match.
    TurnedBack {
        /// The rewrite that holds the turns.
        rewrite: Rewrite,
        /// The turns the file now shows.
        turns: Turns,
        /// The file's EXIF orientation tag as it now holds it (the turns in the tag, or 1 when the pixels were turned).
        exif: pg_model::orientation::Orientation,
    },
    /// The file can't hold the turns: the catalog keeps them (`files.rotation`).
    KeptInCatalog(Turns),
    /// Nothing to record: the write is done, and the catalog already says so.
    Nothing,
}
