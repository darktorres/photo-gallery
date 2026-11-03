//! The import pass (PLAN §4): every pending file is read once, in on-disk order, and analyzed (tiers 0a/0b/1,
//! dimensions, ThumbHash). Each new PDQ hash is queried against the index, then inserted, so tier-1 pairs appear
//! during the import (query-then-insert). Results are committed in batches: stopping at any point loses at most
//! the current batch, and the next run picks up the rest from the catalog.

use std::path::Path;

use anyhow::Result;
use pg_core::analysis::Analysis;
use pg_core::analyze::Analyzer;
use pg_core::catalog::{AnalysisWriter, Catalog};
use pg_core::media::Kind;
use pg_core::pdq::{MATCH_DISTANCE, MIN_QUALITY};
use pg_core::pdq_index::PdqIndex;
use pg_platform::verbatim::verbatim;

use crate::control::Control;
use crate::pipeline::{self, Options};

const BATCH: usize = 500;

pub struct ImportStats {
    pub files: usize,
    pub failed: usize,
    pub pairs: usize,
    /// Files changed while they were read (or being rewritten by the app): nothing stored, due again.
    pub changed: usize,
}

pub fn import(catalog: &mut Catalog, opts: &Options, retry_errors: bool, ctl: &Control) -> Result<ImportStats> {
    let work = catalog.pending_import(retry_errors)?;
    let items = &work.items;
    let mut stats = ImportStats { files: 0, failed: 0, pairs: 0, changed: 0 };
    // Most rounds (a new thumbnail height, a turn) have nothing to import: don't load every hash for them.
    if items.is_empty() {
        return Ok(stats);
    }
    let (ids, hashes) = catalog.pdq_hashes()?;
    let mut index = PdqIndex::build(ids, hashes);
    // Every file of the list may add its 8 orientations.
    index.reserve(items.len() * 8);
    eprintln!("{} files to import; tier-1 index holds {} hashes", items.len(), index.len());

    let mut batch: Vec<(usize, Result<Analysis>)> = Vec::with_capacity(BATCH);
    // Kept across files: one orientation's neighbours, and each neighbour's best (distance, orientation).
    let mut found: Vec<(i64, u32)> = Vec::new();
    let mut best: Vec<(i64, u32, usize)> = Vec::new();
    ctl.begin("import", String::new(), items.len() as u64);

    let mut flush = |batch: &mut Vec<(usize, Result<Analysis>)>, stats: &mut ImportStats| -> Result<()> {
        let tx = catalog.conn.transaction()?;
        let mut writer = AnalysisWriter::new(&tx)?;
        for (i, result) in batch.drain(..) {
            let item = &items[i];
            match result {
                Ok(a) => {
                    if !writer.store_analysis(item, &a)? {
                        stats.changed += 1;
                        continue;
                    }
                    if let Some(px) = a.image().map(|i| &i.pixels).filter(|px| px.pdq.quality >= MIN_QUALITY) {
                        // The new image's 8 orientations against every stored orientation of the earlier ones:
                        // the pair is found the same whichever image comes first. The best per neighbour.
                        index.best_matches(&px.pdq_dihedral, MATCH_DISTANCE, &mut found, &mut best);
                        for &(other, d, v) in &best {
                            if other != item.id {
                                writer.store_pdq_pair(item.id, other, d, v)?;
                                stats.pairs += 1;
                            }
                        }
                        index.insert_orientations(item.id, &px.pdq_dihedral);
                    }
                }
                Err(e) => {
                    if !writer.store_failure(item, &format!("{e:#}"))? {
                        stats.changed += 1;
                        continue;
                    }
                    stats.failed += 1;
                }
            }
            stats.files += 1;
        }
        drop(writer);
        tx.commit()?;
        Ok(())
    };

    let video = |i: usize| items[i].kind == Kind::Video;
    pipeline::run(
        items,
        |item, wide| {
            // A video is read by FFmpeg, from its path.
            let whole = item.kind != Kind::Video;
            if whole {
                let [folder, name] = work.parts(item.path);
                pg_platform::verbatim::verbatim_wide_file(folder, name, wide);
            }
            whole
        },
        opts,
        ctl,
        Analyzer::new,
        |analyzer, i, bytes| {
            if video(i) {
                let path = work.path(items[i].path);
                pg_core::analyze::video(Path::new(&*verbatim(&path)), items[i].rotation, analyzer)
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
