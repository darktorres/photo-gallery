//! The thumbs pass's work (section 10, D10): the catalog's candidates and the cache's entries at the height, each read on
//! its owner's connection, merged as two streams in file-id order. No file is attached to the other, so a failure says
//! whose it was: the catalog's is a round error, the cache's is `CacheRead`. The order of the reads is fixed: the
//! catalog's volumes, the cache's marks for them, then the merge, whose catalog read gives the change number the pass's
//! checkpoint is written at (never a later one). No shared snapshot is needed: a made thumbnail is checked against the
//! catalog before it is kept (`Catalog::currents`), so a stale candidate costs work only.

use std::ops::ControlFlow;

use pg_catalog::{Catalog, ThumbCandidate, VolumeSet};
use pg_model::ids::FileId;
use pg_model::path::WorkList;
use pg_model::size::ThumbHeight;
use pg_model::turns::Turns;
use pg_thumbcache::{EntryRow, Pending, Snapshot, Source, ThumbCache, ThumbWork};

use crate::{Error, Result};

/// The pending work of a pass at `height` over `volumes`: the candidates that have no current entry, sorted by volume and
/// place on it (the order the files are read in), with the snapshot its checkpoint is written from. A cache entry whose
/// values don't read is a miss, and is forgotten (D8).
pub(crate) fn work(cache: &ThumbCache, catalog: &Catalog, height: ThumbHeight, volumes: &VolumeSet) -> Result<Pending> {
    // 1. The volumes the checkpoints are kept for.
    let volumes: Vec<i64> = match volumes {
        VolumeSet::All => catalog.thumb_volumes()?,
        // A volume id is stored as the SQL integer of its bits (the catalog's identity edge).
        VolumeSet::Only(volumes) => volumes.iter().map(|&v| v.cast_signed()).collect(),
    };
    // 2. The cache's marks for them.
    let (since, evictions) = cache.pending_marks(height, &volumes).map_err(Error::CacheRead)?;
    // 3. The merge: the candidates stream from the catalog, the entries from the cache, both in file-id order.
    let mut work: WorkList<ThumbWork> = WorkList::with_capacity(0);
    let mut kept: Vec<(i64, i64, ThumbWork)> = Vec::new();
    let mut bad: Vec<FileId> = Vec::new();
    let mut entry_error: Option<pg_thumbcache::Error> = None;
    let read = cache
        .with_entries(height, |next| {
            let mut entry = pull(next, &mut entry_error);
            catalog.thumb_candidates(&volumes, since, &mut |candidate| {
                while let Some(e) = entry
                    && e.file_id() < candidate.id
                {
                    entry = pull(next, &mut entry_error);
                }
                if entry_error.is_some() {
                    return ControlFlow::Break(());
                }
                let current = match entry {
                    Some(EntryRow::Current { file_id, mtime, rotation }) if file_id == candidate.id => {
                        mtime == candidate.mtime && rotation == i64::from(candidate.turns)
                    }
                    Some(EntryRow::Bad { file_id }) if file_id == candidate.id => {
                        bad.push(file_id);
                        false
                    }
                    _ => false,
                };
                if !current {
                    kept.push(owed(&mut work, candidate));
                }
                ControlFlow::Continue(())
            })
        })
        .map_err(Error::CacheRead)?;
    if let Some(e) = entry_error {
        return Err(Error::CacheRead(e));
    }
    let read = read?;
    // Best effort, as a touch is: an entry that can't be forgotten now is a miss again at the next pass.
    for file_id in bad {
        let _ = cache.forget_height(file_id, height);
    }
    // In the order the files are read: by volume, then by their place on it.
    kept.sort_by_key(|&(volume, place, _)| (volume, place));
    work.items = kept.into_iter().map(|(_, _, item)| item).collect();
    let snapshot = Snapshot { height, seq: read.seq, evictions, since_checked: read.since_checked, volumes };
    Ok(Pending { work, snapshot })
}

/// The next entry of the cache's stream; an error ends the stream, and is kept for the merge to return.
fn pull(
    next: &mut dyn FnMut() -> pg_thumbcache::Result<Option<EntryRow>>,
    error: &mut Option<pg_thumbcache::Error>,
) -> Option<EntryRow> {
    match next() {
        Ok(entry) => entry,
        Err(e) => {
            *error = Some(e);
            None
        }
    }
}

/// A candidate with no current entry, as a work item of `work` (its path in the list's arenas), with its volume and its
/// place on the volume for the sort.
fn owed(work: &mut WorkList<ThumbWork>, c: &ThumbCandidate<'_>) -> (i64, i64, ThumbWork) {
    let path = match work.file(c.folder_id, c.name, |arena| Ok::<_, std::convert::Infallible>(arena.push(c.folder))) {
        Ok(path) => path,
        Err(never) => match never {},
    };
    let item = ThumbWork {
        id: c.id,
        path,
        source: Source { mtime: c.mtime, turns: Turns::new(c.turns) },
        size: c.size,
        poster_ms: c.poster_ms,
        seq: c.seq,
    };
    (c.volume, c.file_ref & 0xFFFF_FFFF_FFFF, item)
}
