//! Finishing the removals of folders from the library (`Catalog::begin_removal`): the folder's files are hidden, then deleted
//! from the catalog batch by batch, with their thumbnails, and its folders go; the catalog's free pages are given back and
//! the thumbnail cache's orphans are dropped. [`finish`] is the one place this runs: the library's removals thread runs it
//! as a job of its own (with a progress row and a pause), and a command, or the app when the library is not running, runs it
//! on the caller. Every step is idempotent, so a removal interrupted anywhere is finished by the next call.

use std::path::Path;

use crate::{Error, Result};
use pg_catalog::{Catalog, Driven};
use pg_model::ids::FileId;
use pg_thumbcache::ThumbCache;

/// Finishes every removal not finished. `progress` is told how many files each batch is about to delete, before the
/// batch goes, and returns `false` to stop (the removal then waits for the next call). The batch's thumbnails are
/// forgotten first; a thumbnail that could not be forgotten is not a reason to keep the removal from going on, so the
/// first such failure is returned after the rest is done, and the sweep at the end drops what it can.
pub fn finish(
    catalog: &Catalog,
    thumbs: &ThumbCache,
    catalog_path: &Path,
    progress: &mut dyn FnMut(usize) -> bool,
) -> Result<Driven> {
    for marker in catalog.removal_markers()? {
        catalog.hide_pages(&marker.path)?;
    }
    drive(catalog, thumbs, catalog_path, progress)
}

/// The deletes of [`finish`], once the markers' folders are hidden: their files go batch by batch, each batch's thumbnails
/// forgotten before it, then the free pages are given back and the orphans dropped. The library's removal job calls this
/// after it has hidden the folders under its parks (section 1); a caller with no lanes to park uses [`finish`].
pub fn drive(
    catalog: &Catalog,
    thumbs: &ThumbCache,
    catalog_path: &Path,
    progress: &mut dyn FnMut(usize) -> bool,
) -> Result<Driven> {
    let mut forget_failed = None;
    let driven = catalog.drive_removals(&mut |ids| {
        let ids: Vec<FileId> = ids.iter().map(|&id| FileId::new(id)).collect();
        if let Err(e) = thumbs.forget_many(&ids) {
            forget_failed.get_or_insert(e);
        }
        progress(ids.len())
    })?;
    // The deletes are in the WAL: this job moves them into the file at the end of its work (PASSIVE).
    catalog.checkpoint_passive()?;
    catalog.shrink()?;
    thumbs.sweep_orphans(catalog_path)?;
    // The forgotten thumbnails' pages go back to the file system too (the cache keeps a little free space, as its own
    // maintenance does, and gives the rest back).
    thumbs.shrink_all()?;
    match forget_failed {
        Some(e) => Err(Error::Cache(e)),
        None => Ok(driven),
    }
}
