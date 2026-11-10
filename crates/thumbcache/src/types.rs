//! The data the cache's layers pass between them: the thumbnails and the work for them, the snapshot a pass read
//! against, the events the writer reports and the usage. A leaf (the reason a file was started over is the model's, for the notice it becomes).

use std::path::PathBuf;

use pg_model::ids::FileId;
use pg_model::notice::StartOverReason;
use pg_model::path::{FilePath, WorkList};
use pg_model::turns::Turns;

pub const FORMAT_JPEG: &str = "jpeg";

/// What a thumbnail was made from: the file's mtime and its catalog rotation (`files.rotation`). A thumbnail is
/// current while both still match the file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Source {
    pub mtime: i64,
    pub rotation: Turns,
}

pub struct Thumb {
    pub file_id: FileId,
    /// The requested grid height (the image may be smaller).
    pub height: u32,
    pub source: Source,
    pub data: Vec<u8>,
}

/// A file the background task still has to thumbnail at the current height.
pub struct ThumbWork {
    pub id: FileId,
    /// In its list.
    pub path: FilePath,
    pub source: Source,
    /// The file's size when the work was listed: a thumbnail is kept only if the file still has it.
    pub size: i64,
    /// For a video, the time of its poster frame (`video::poster_ms`).
    pub poster_ms: Option<i64>,
    /// The catalog's change number for it (`files.seq`).
    pub seq: i64,
}

/// The files a pass at `height` has to thumbnail, and what they were read against (`ThumbCache::checked`).
pub struct Pending {
    pub work: WorkList<ThumbWork>,
    pub snapshot: Snapshot,
}

/// Where a pass's work list was read: the catalog's change sequence and the height's evictions at that moment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Snapshot {
    pub height: u32,
    pub seq: i64,
    pub evictions: i64,
    /// Only the files changed since the last complete pass were read (`false`: every file).
    pub since_checked: bool,
}

/// What the cache's writer reports. Nothing waits for these (the thumbnail was shown anyway, the totals were fixed),
/// so they are events on a bus, not results: a subscriber prints them, or counts them. `error` is the error chain as
/// `{:#}` prints it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The totals were off when the cache was opened, and were fixed: what they were, and what the entries hold.
    TotalsFixed { kept: (i64, i64), actual: (i64, i64) },
    /// The times entries were shown could not be written; they stay queued for the next try.
    TouchesNotWritten { error: String },
    /// An on-demand thumbnail could not be stored (it was shown anyway).
    NotStored { error: String },
    /// The WAL could not be merged when the writer closed.
    WalNotMerged { error: String },
    /// The cache file was started over when it was opened, for `reason`: because this build can't use it (made for
    /// another catalog, or by another build), or because it was damaged or not a database. `path` is where the old
    /// file is now for a damaged one, which is moved aside (`thumbs.sqlite.bad`), and the cache file itself, made
    /// anew, for one this build can't use, which is deleted. `detail` is the technical account (the first difference,
    /// SQLite's error), for a log: the words a person reads come from `reason` and `path`. Its thumbnails are made
    /// again; the cap it had is kept, except a damaged file's, which can't be read (the cap given, or the default).
    StartedOver { reason: StartOverReason, path: PathBuf, detail: String },
}

/// What a cache holds (bytes, entries) and may hold (`cap`, bytes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Usage {
    pub bytes: i64,
    pub count: i64,
    pub cap: i64,
}
