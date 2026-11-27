//! Finishing the removals of folders from the library (`Catalog::begin_removal`): the folder's files are hidden, then deleted
//! from the catalog batch by batch, with their thumbnails, and its folders go; the catalog's free pages are given back and
//! the thumbnail cache's orphans are dropped. [`finish`] is the one place this runs: the library's delete job runs [`drive`]
//! as a job of its own (with a progress row and a pause), after its hide job has hidden each folder; a command, or the app
//! when the library is not running, runs [`finish`] on the caller. Every step is idempotent, so a removal interrupted
//! anywhere is finished by the next call.

use std::ops::ControlFlow;

use crate::{Error, Result};
use pg_catalog::{Catalog, Driven};
use pg_model::ids::FileId;
use pg_model::rules::Path as RulePath;
use pg_thumbcache::ThumbCache;

/// What [`finish`] and [`drive`] did: the removals' deletes, and apart from them the sweep of the cache's orphans
/// (section 10, D12). The folder is hidden and deleted whatever the sweep came to, so a failed sweep is not the removal's
/// failure: its caller says it (`CacheNotSwept`) or clears that state, and a later sweep drops what this one couldn't.
#[derive(Debug)]
pub struct Finished {
    /// The removals' deletes.
    pub driven: Driven,
    /// The sweep's outcome: the files whose thumbnails it dropped, or why it couldn't.
    pub swept: Result<usize>,
}

impl std::ops::Deref for Finished {
    type Target = Driven;

    fn deref(&self) -> &Driven {
        &self.driven
    }
}

/// Finishes every removal not finished. `progress` is told how many files each batch is about to delete, before the
/// batch goes, and returns `false` to stop (the removal then waits for the next call). The batch's thumbnails are
/// forgotten first; a thumbnail that could not be forgotten is not a reason to keep the removal from going on, so the
/// first such failure is told as the sweep's (`Finished::swept`) after the rest is done, and the sweep at the end drops
/// what it can.
pub fn finish(catalog: &Catalog, thumbs: &ThumbCache, progress: &mut dyn FnMut(usize) -> bool) -> Result<Finished> {
    let markers = catalog.removal_markers()?;
    for marker in &markers {
        catalog.hide_pages(&marker.path)?;
    }
    let folders: Vec<RulePath> = markers.into_iter().map(|marker| marker.path).collect();
    drive(catalog, thumbs, &folders, progress)
}

/// The deletes of [`finish`], for the removals at `folders`, once their folders are hidden: their files go batch by batch,
/// each batch's thumbnails forgotten before it, then the free pages are given back and the orphans dropped. The library's
/// delete job calls this for the removals its hide job has hidden under their parks (section 1); a caller with no lanes to
/// park uses [`finish`].
pub fn drive(
    catalog: &Catalog,
    thumbs: &ThumbCache,
    folders: &[RulePath],
    progress: &mut dyn FnMut(usize) -> bool,
) -> Result<Finished> {
    let mut forget_failed = None;
    let driven = catalog.drive_removals_of(folders, &mut |ids| {
        let ids: Vec<FileId> = ids.iter().map(|&id| FileId::new(id)).collect();
        if let Err(e) = thumbs.forget_many(&ids) {
            forget_failed.get_or_insert(e);
        }
        progress(ids.len())
    })?;
    // The deletes are in the WAL: this job moves them into the file at the end of its work (PASSIVE).
    catalog.checkpoint_passive()?;
    catalog.shrink()?;
    // The sweep's outcome is returned, not the removal's failure (D12).
    let swept = sweep_orphans(thumbs, catalog);
    // The forgotten thumbnails' pages go back to the file system too (the cache keeps a little free space, as its own
    // maintenance does, and gives the rest back).
    let shrunk = thumbs.shrink_all();
    // A thumbnail the cache could not forget or give its pages back is the cache's failure, not the removal's: the folder
    // is hidden and its files deleted already, so it is told as the sweep's (`CacheNotSwept`), not as a failed removal.
    let swept = match forget_failed.or(shrunk.err()) {
        Some(e) => Err(Error::Cache(e)),
        None => swept,
    };
    Ok(Finished { driven, swept })
}

/// Drops the thumbnails of the files the catalog no longer has (section 10, D11): what a removal left behind, or an entry of
/// a catalog that was rebuilt. The catalog's id high-water mark is read first (ids are never reused): an entry at or below
/// it whose file is absent was deleted for good, and a file added during the merge is above it. The catalog's file ids and
/// the cache's entries are then streamed in id order, each on its owner's connection, and merged: no file is attached to
/// the other. Returns how many files' thumbnails were dropped.
pub fn sweep_orphans(thumbs: &ThumbCache, catalog: &Catalog) -> Result<usize> {
    let high = catalog.thumb_high_water()?;
    let mut orphans: Vec<FileId> = Vec::new();
    let mut entry_error: Option<pg_thumbcache::Error> = None;
    let walked = thumbs.with_entry_ids(high, |next| {
        let mut entry = pull(next, &mut entry_error);
        let read = catalog.thumb_file_ids(high, &mut |id| {
            // The cache's entries below this file id have no file.
            while let Some(e) = entry
                && e.get() < id
            {
                orphans.push(e);
                entry = pull(next, &mut entry_error);
            }
            if entry_error.is_some() {
                return ControlFlow::Break(());
            }
            if entry.is_some_and(|e| e.get() == id) {
                entry = pull(next, &mut entry_error);
            }
            ControlFlow::Continue(())
        });
        // The entries after the catalog's last file id have none either.
        if read.is_ok() {
            while let Some(e) = entry {
                orphans.push(e);
                entry = pull(next, &mut entry_error);
            }
        }
        read
    })?;
    if let Some(e) = entry_error {
        return Err(Error::Cache(e));
    }
    walked?;
    thumbs.forget_many(&orphans)?;
    Ok(orphans.len())
}

/// The next entry of the cache's stream; an error ends the stream, and is kept for the merge to return.
fn pull(
    next: &mut dyn FnMut() -> pg_thumbcache::Result<Option<FileId>>,
    error: &mut Option<pg_thumbcache::Error>,
) -> Option<FileId> {
    match next() {
        Ok(entry) => entry,
        Err(e) => {
            *error = Some(e);
            None
        }
    }
}
