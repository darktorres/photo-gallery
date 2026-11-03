//! The import pass (PLAN §4): every pending file is read once, in on-disk order, and analyzed (tiers 0a/0b/1,
//! dimensions, ThumbHash). Each new PDQ hash is queried against the index, then inserted, so tier-1 pairs appear
//! during the import (query-then-insert). Results are committed in batches: stopping at any point loses at most
//! the current batch, and the next run picks up the rest from the catalog.

use std::path::Path;

use anyhow::Result;
use pg_analysis::analyze::Analyzer;
use pg_catalog::Catalog;
use pg_dedup::Pairer;
use pg_model::analysis::Analyzed;
use pg_model::media::Kind;
use pg_model::pdq::MIN_QUALITY;

use pg_jobs::control::{Control, Phase};
use pg_jobs::pipeline::{self, Options};

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
    let mut pairer = Pairer::build(ids, hashes);
    // Every file of the list may add its 8 orientations.
    pairer.reserve(items.len() * 8);
    eprintln!("{} files to import; tier-1 index holds {} hashes", items.len(), pairer.len());

    let mut batch: Vec<(usize, Result<Analyzed>)> = Vec::with_capacity(BATCH);
    ctl.begin(Phase::Import, items.len() as u64);

    let mut flush = |batch: &mut Vec<(usize, Result<Analyzed>)>, stats: &mut ImportStats| -> Result<()> {
        let tx = catalog.tx()?;
        let mut writer = tx.analysis_writer(crate::versions().import)?;
        for (i, result) in batch.drain(..) {
            let item = &items[i];
            match result {
                Ok(a) => {
                    if !writer.store_analysis(item, &a)? {
                        stats.changed += 1;
                        continue;
                    }
                    let quality_ok = a.record.image().is_some_and(|i| i.pixels.pdq.quality >= MIN_QUALITY);
                    if let Some(dihedral) = a.pdq_dihedral.filter(|_| quality_ok) {
                        // The new image's 8 orientations against every stored orientation of the earlier ones,
                        // then stored: the pair is found the same whichever image comes first, with the best per
                        // neighbour.
                        for &(other, d, v) in pairer.pair_and_insert(item.id, &dihedral) {
                            writer.store_pdq_pair(item.id, other, d, v)?;
                            stats.pairs += 1;
                        }
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
                pg_analysis::analyze::video(Path::new(&path), items[i].rotation, analyzer)
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
