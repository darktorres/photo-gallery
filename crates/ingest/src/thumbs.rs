//! The background thumbnail task (PLAN §6): after the import, thumbnails for every analyzed file at the height
//! the grid currently uses, in on-disk order, until done or until the cache is full of that height. Runs at idle
//! priority, as it competes with the user for the disk and the CPU. The cache is shared with the app's on-demand
//! thumbnails; neither waits for the other but in the cache's writer, one batch at a time.

use std::path::Path;

use anyhow::Result;
use pg_core::browse;
use pg_core::catalog::Catalog;
use pg_core::paths::verbatim;
use pg_core::thumbs::{Maker, Pending, Thumb, ThumbCache};

use crate::control::Control;
use crate::pipeline::{self, Options};

const BATCH: usize = 200;

pub struct ThumbStats {
    pub made: usize,
    pub failed: usize,
    /// Files changed while they were read (or being rewritten by the app): not kept, tried again.
    pub changed: usize,
    pub cache_full: bool,
    /// Every file of the work was tried (not stopped, the cache not full): the next pass starts from this one's
    /// snapshot (`ThumbCache::checked`).
    pub complete: bool,
}

/// One pass at `height`: the files changed since the last complete one (or all of them, `ThumbCache::pending`).
pub fn run(cache: &ThumbCache, catalog_path: &Path, height: u32, opts: &Options, ctl: &Control) -> Result<ThumbStats> {
    let Pending { work, snapshot } = cache.pending(catalog_path, height)?;
    let items = &work.items;
    let mut stats = ThumbStats { made: 0, failed: 0, changed: 0, cache_full: false, complete: false };
    if items.is_empty() {
        cache.checked(&snapshot, None)?;
        stats.complete = true;
        cache.shrink()?;
        return Ok(stats);
    }
    eprintln!(
        "{} files need a {height} px thumbnail; cache holds {:.2} GB",
        items.len(),
        cache.totals()?.0 as f64 / 1e9
    );
    // To check each file against before its thumbnail is kept.
    let catalog = Catalog::open_current(catalog_path)?;
    let mut batch = Vec::with_capacity(BATCH);
    let (mut tried, mut first_failed) = (0, None);
    ctl.begin("thumbs", format!("{height} px"), items.len() as u64);

    let flush = |batch: &mut Vec<Thumb>, stats: &mut ThumbStats| -> Result<bool> {
        let n = batch.len();
        let stored = cache.put_batch(std::mem::replace(batch, Vec::with_capacity(BATCH)), Some(height))?;
        if stored {
            stats.made += n;
        } else {
            stats.cache_full = true;
        }
        Ok(stored)
    };

    pipeline::run(
        items,
        |item, wide| {
            // A video's poster frame is read by FFmpeg, from its path.
            let whole = item.poster_ms.is_none();
            if whole {
                work.wide(item.path, wide);
            }
            whole
        },
        opts,
        ctl,
        Maker::new,
        |maker, i, bytes| {
            let item = &items[i];
            match item.poster_ms {
                // A video: its poster frame, read by FFmpeg itself.
                Some(at) => {
                    let path = work.path(item.path);
                    maker.load_frame(Path::new(&*verbatim(&path)), at, item.source.rotation)?;
                    maker.encode_frame(height as usize)
                }
                None => maker.make(bytes, height as usize, item.source.rotation),
            }
        },
        |i, result, len| {
            ctl.add(len, result.is_err());
            tried += 1;
            let item = &items[i];
            match result {
                // Kept only if made from the file as listed: not changed since, nor being rewritten by the app
                // (browse::FileState). Otherwise tried again by the next pass, from its change number on.
                Ok(data) => match browse::file_state(&catalog.conn, item.id)? {
                    Some(s) if !s.writing && (s.mtime, s.rotation) == (item.source.mtime, item.source.rotation) => {
                        batch.push(Thumb { file_id: item.id, height, source: item.source, data });
                    }
                    _ => {
                        stats.changed += 1;
                        first_failed = Some(first_failed.map_or(item.seq, |s: i64| s.min(item.seq)));
                    }
                },
                // The import already decoded these files; a failure here is a file changed or gone since (or its
                // drive offline): the next pass tries it again.
                Err(e) => {
                    stats.failed += 1;
                    first_failed = Some(first_failed.map_or(items[i].seq, |s: i64| s.min(items[i].seq)));
                    eprintln!("  {}: {e:#}", work.path(items[i].path));
                }
            }
            if batch.len() >= BATCH {
                return flush(&mut batch, &mut stats);
            }
            Ok(true)
        },
    )?;
    if !stats.cache_full {
        flush(&mut batch, &mut stats)?;
    }
    ctl.finish();
    stats.complete = !stats.cache_full && tried == items.len();
    if stats.complete {
        cache.checked(&snapshot, first_failed)?;
    }
    cache.shrink()?;
    Ok(stats)
}
