//! A queued write as the writer takes it, and what a finished one changed. A leaf below `core`: `Catalog`'s methods
//! take it, and the compiler graph puts an inherent method in its type's module, so a type they name has to be below
//! `core` too.

use pg_model::ids::FileId;
use pg_model::rating::Rating;
use pg_model::rules::Path as RulePath;
use pg_model::turns::Turns;
use pg_model::write::{Write, WriteKind};

/// A queued write, as the writer takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
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
    pub before: Option<(i64, i64)>,
    pub after: (i64, i64),
    pub metadata_only: Option<pg_model::analysis::FileHash>,
}

/// What a finished write changed in the catalog. Applied in the transaction that takes the job off the queue
/// (`done`), so the catalog and the queue never disagree. The effect's value is the job's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteEffect {
    /// The rating is in the file, which was rewritten to hold it (0 stars: it holds none).
    FileRating { rewrite: Rewrite, rating: Rating },
    /// The turns are in the file, which was rewritten to show them: the analysis is turned back to match.
    TurnedBack { rewrite: Rewrite, turns: Turns },
    /// The file can't hold the turns: the catalog keeps them (`files.rotation`).
    KeptInCatalog(Turns),
    /// Nothing to record: the write is done, and the catalog already says so.
    Nothing,
}
