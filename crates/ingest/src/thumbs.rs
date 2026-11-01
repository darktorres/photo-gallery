//! The background thumbnail task (PLAN §6): after the import, thumbnails for every analyzed file at the height
//! the grid currently uses, in on-disk order, until done or until the cache is full of that height. Runs at idle
//! priority, as it competes with the user for the disk and the CPU. The cache is shared with the app's on-demand
//! thumbnails; neither waits for the other but in the cache's writer, one batch at a time.

use std::path::Path;

use anyhow::Result;
use pg_core::decode::Decoder;
use pg_core::thumbs::{self, Pending, Thumb, ThumbCache};
use turbojpeg::Compressor;

use crate::control::Control;
use crate::pipeline::{self, Options};

const BATCH: usize = 200;

pub struct ThumbStats {
    pub made: usize,
    pub failed: usize,
    pub cache_full: bool,
    /// Every file of the work was tried (not stopped, the cache not full): the next pass starts from this one's
    /// snapshot (`ThumbCache::checked`).
    pub complete: bool,
}

/// One pass at `height`: the files changed since the last complete one (or all of them, `ThumbCache::pending`).
pub fn run(cache: &ThumbCache, catalog_path: &Path, height: u32, opts: &Options, ctl: &Control) -> Result<ThumbStats> {
    let Pending { work: items, snapshot } = cache.pending(catalog_path, height)?;
    let mut stats = ThumbStats { made: 0, failed: 0, cache_full: false, complete: false };
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
    let paths: Vec<String> = items.iter().map(|i| i.path.clone()).collect();
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
        &paths,
        |i| items[i].poster_ms.is_none(),
        opts,
        ctl,
        || Ok((Decoder::new()?, Compressor::new()?)),
        |(decoder, encoder), i, bytes| {
            let item = &items[i];
            match item.poster_ms {
                // A video: its poster frame, read by FFmpeg itself.
                Some(at) => {
                    let path = pipeline::long_path(&item.path);
                    let frame = pg_core::video::frame(Path::new(&path), at, item.source.rotation)?;
                    thumbs::encode(&frame, height as usize, encoder)
                }
                None => thumbs::make(bytes, height as usize, item.source.rotation, decoder, encoder),
            }
        },
        |i, result, len| {
            ctl.add(len, result.is_err());
            tried += 1;
            match result {
                Ok(data) => batch.push(Thumb { file_id: items[i].id, height, source: items[i].source, data }),
                // The import already decoded these files; a failure here is a file changed or gone since (or its
                // drive offline): the next pass tries it again.
                Err(e) => {
                    stats.failed += 1;
                    first_failed = Some(first_failed.map_or(items[i].seq, |s: i64| s.min(items[i].seq)));
                    eprintln!("  {}: {e:#}", items[i].path);
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
