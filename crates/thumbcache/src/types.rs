//! The data the cache's layers pass between them: the thumbnails and the work for them, the snapshot a pass read
//! against, the events the writer reports and the usage. A leaf (the reason a file was started over is the model's, for the notice it becomes).

use std::path::PathBuf;

use pg_model::ids::FileId;
use pg_model::notice::{ERROR_CAP, Key, KeyBuf, Notice, StartOverReason, tidy};
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
    /// The times entries were shown were written, the first time after [`Event::TouchesNotWritten`].
    TouchesWritten,
    /// An on-demand thumbnail could not be stored (it was shown anyway).
    NotStored { error: String },
    /// A thumbnail was stored, the first time after [`Event::NotStored`].
    Stored,
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

/// What an [`Event`] does to the user's notices ([`Event::effect`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Tell the user this notice.
    Post(Notice),
    /// The kept notice this key names no longer holds (the writer recovered).
    Clear(KeyBuf),
}

impl Event {
    /// What the event does to the user's notices (the app's inbox; the command line prints only a [`Effect::Post`]):
    /// a failure is a notice, and the writer's first success after one clears its kept notice.
    pub fn effect(&self) -> Option<Effect> {
        let error = |e: &str| tidy(e, ERROR_CAP);
        let post = |notice| Some(Effect::Post(notice));
        match self {
            Event::TotalsFixed { kept: (kept_bytes, kept_count), actual: (bytes, count) } => {
                post(Notice::CacheTotalsFixed {
                    kept_bytes: *kept_bytes,
                    kept_count: *kept_count,
                    bytes: *bytes,
                    count: *count,
                })
            }
            Event::TouchesNotWritten { error: e } => post(Notice::CacheTouchesNotWritten { error: error(e) }),
            Event::TouchesWritten => Some(Effect::Clear(Key::all("cache-touches-not-written", None).to_buf())),
            Event::NotStored { error: e } => post(Notice::CacheNotStored { error: error(e) }),
            Event::Stored => Some(Effect::Clear(Key::all("cache-not-stored", None).to_buf())),
            Event::WalNotMerged { error: e } => post(Notice::CacheWalNotMerged { error: error(e) }),
            Event::StartedOver { reason, path, detail } => post(Notice::CacheStartedOver {
                reason: *reason,
                path: path.display().to_string(),
                detail: error(detail),
            }),
        }
    }
}

/// What a cache holds (bytes, entries) and may hold (`cap`, bytes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Usage {
    pub bytes: i64,
    pub count: i64,
    pub cap: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure posts its notice, and the recovery after it clears that notice's key: the key the notice is kept under.
    #[test]
    fn a_recovery_clears_the_key_its_failure_is_kept_under() {
        for (failure, recovery) in [
            (Event::TouchesNotWritten { error: "busy".into() }, Event::TouchesWritten),
            (Event::NotStored { error: "busy".into() }, Event::Stored),
        ] {
            let Some(Effect::Post(notice)) = failure.effect() else { panic!("{failure:?} posts") };
            let pg_model::notice::Lifecycle::State(key, _) = notice.lifecycle() else { panic!("{notice:?} is kept") };
            assert_eq!(recovery.effect(), Some(Effect::Clear(key.to_buf())), "{recovery:?}");
        }
    }
}
