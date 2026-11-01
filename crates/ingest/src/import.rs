//! The import pass (PLAN §4): every pending file is read once, in on-disk order, and analyzed (tiers 0a/0b/1,
//! dimensions, ThumbHash). Each new PDQ hash is queried against the index, then inserted, so tier-1 pairs appear
//! during the import (query-then-insert). Results are committed in batches: stopping at any point loses at most
//! the current batch, and the next run picks up the rest from the catalog.

use std::path::Path;

use anyhow::Result;
use pg_core::analyze::{Analysis, Analyzer};
use pg_core::catalog::Catalog;
use pg_core::media::Kind;
use pg_core::pdq::{MATCH_DISTANCE, MIN_QUALITY};
use pg_core::pdq_index::PdqIndex;

use crate::control::Control;
use crate::pipeline::{self, Options};

const BATCH: usize = 500;

pub struct ImportStats {
    pub files: usize,
    pub failed: usize,
    pub pairs: usize,
}

pub fn import(catalog: &mut Catalog, opts: &Options, retry_errors: bool, ctl: &Control) -> Result<ImportStats> {
    let items = catalog.pending_import(retry_errors)?;
    let mut stats = ImportStats { files: 0, failed: 0, pairs: 0 };
    // Most rounds (a new thumbnail height, a turn) have nothing to import: don't load every hash for them.
    if items.is_empty() {
        return Ok(stats);
    }
    let mut index = PdqIndex::default();
    for (id, hash) in catalog.pdq_hashes()? {
        index.insert(id, &hash);
    }
    eprintln!("{} files to import; tier-1 index holds {} hashes", items.len(), index.len());
    let paths: Vec<String> = items.iter().map(|i| i.path.clone()).collect();

    let mut batch: Vec<(usize, Result<Analysis>)> = Vec::with_capacity(BATCH);
    ctl.begin("import", String::new(), items.len() as u64);

    let mut flush = |batch: &mut Vec<(usize, Result<Analysis>)>, stats: &mut ImportStats| -> Result<()> {
        let tx = catalog.conn.transaction()?;
        for (i, result) in batch.drain(..) {
            let item = &items[i];
            match result {
                Ok(a) => {
                    Catalog::store_analysis(&tx, item, &a)?;
                    if let Some(px) = a.pixels.as_ref().filter(|px| px.pdq.quality >= MIN_QUALITY) {
                        // Best distance per neighbour over the 8 orientations of the new image.
                        let mut best: Vec<(i64, u32, usize)> = Vec::new();
                        for (v, h) in px.pdq_dihedral.iter().enumerate() {
                            for (other, d) in index.query(h, MATCH_DISTANCE) {
                                match best.iter_mut().find(|b| b.0 == other) {
                                    Some(b) if d < b.1 => *b = (other, d, v),
                                    Some(_) => {}
                                    None => best.push((other, d, v)),
                                }
                            }
                        }
                        for (other, d, v) in best {
                            if other != item.id {
                                Catalog::store_pdq_pair(&tx, item.id, other, d, v)?;
                                stats.pairs += 1;
                            }
                        }
                        index.insert(item.id, &px.pdq.hash);
                    }
                }
                Err(e) => {
                    Catalog::store_failure(&tx, item, &format!("{e:#}"))?;
                    stats.failed += 1;
                }
            }
            stats.files += 1;
        }
        tx.commit()?;
        Ok(())
    };

    let video = |i: usize| items[i].kind == Kind::Video;
    pipeline::run(
        &paths,
        |i| !video(i),
        opts,
        ctl,
        Analyzer::new,
        |analyzer, i, bytes| {
            if video(i) {
                pg_core::video::analyze(Path::new(&pipeline::long_path(&items[i].path)), items[i].rotation, analyzer)
            } else {
                analyzer.analyze(bytes, items[i].rotation)
            }
        },
        |i, result, len| {
            // A video is streamed by its worker: count its size as read.
            ctl.add(if video(i) { items[i].size as usize } else { len }, result.is_err());
            batch.push((i, result));
            if batch.len() >= BATCH {
                flush(&mut batch, &mut stats)?;
            }
            Ok(true)
        },
    )?;
    flush(&mut batch, &mut stats)?;
    ctl.finish();
    Ok(stats)
}
