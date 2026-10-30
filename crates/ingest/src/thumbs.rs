//! The background thumbnail task (PLAN §6): after the import, thumbnails for every analyzed file at the height
//! the grid currently uses, in on-disk order, until done or until the cache is full of that height. Runs at low
//! priority by default, as it competes with the user for the disk and the CPU.

use std::path::Path;

use anyhow::Result;
use pg_core::decode::Decoder;
use pg_core::thumbs::{self, ThumbCache};
use turbojpeg::Compressor;

use crate::pipeline::{self, Options};
use crate::progress::Progress;

const BATCH: usize = 200;

pub struct ThumbStats {
    pub made: usize,
    pub failed: usize,
    pub cache_full: bool,
}

pub fn run(cache: &mut ThumbCache, catalog_path: &Path, height: u32, opts: &Options) -> Result<ThumbStats> {
    let items = cache.pending(catalog_path, height)?;
    eprintln!(
        "{} files need a {height} px thumbnail; cache holds {:.2} GB",
        items.len(),
        cache.total_bytes() as f64 / 1e9
    );
    let paths: Vec<String> = items.iter().map(|i| i.path.clone()).collect();
    let mut stats = ThumbStats { made: 0, failed: 0, cache_full: false };
    let mut batch = Vec::with_capacity(BATCH);
    let mut progress = Progress::new("thumbs", items.len());

    let mut flush = |batch: &mut Vec<(i64, u32, i64, Vec<u8>)>, stats: &mut ThumbStats| -> Result<bool> {
        let stored = cache.put_batch(batch, Some(height))?;
        if stored {
            stats.made += batch.len();
        } else {
            stats.cache_full = true;
        }
        batch.clear();
        Ok(stored)
    };

    pipeline::run(
        &paths,
        opts,
        || Ok((Decoder::new()?, Compressor::new()?)),
        |(decoder, encoder), bytes| thumbs::make(bytes, height as usize, decoder, encoder),
        |i, result, len| {
            progress.add(len, result.is_err());
            match result {
                Ok(data) => batch.push((items[i].id, height, items[i].mtime, data)),
                // The import already decoded these files; a failure here is a file changed or gone since.
                Err(e) => {
                    stats.failed += 1;
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
    progress.finish();
    Ok(stats)
}
