//! The import pass (PLAN §4): every pending file is read once, in on-disk order, and analyzed (tiers 0a/0b/1,
//! dimensions, ThumbHash). Each new PDQ hash is queried against the index, then inserted, so tier-1 pairs appear
//! during the import (query-then-insert). Results are committed in batches: stopping at any point loses at most
//! the current batch, and the next run picks up the rest from the catalog.

use anyhow::{Context, Result};
use pg_analysis::analyze::Analyzer;
use pg_catalog::Catalog;
use pg_dedup::Pairer;
use pg_model::analysis::Analyzed;
use pg_model::media::Kind;
use pg_model::pdq::MIN_QUALITY;

use pg_jobs::control::Control;
use pg_jobs::pipeline::{self, Batch, Options, Read};
use pg_model::progress::Phase;

const BATCH: usize = 500;

pub struct ImportStats {
    /// The files the pass had to import.
    pub pending: usize,
    /// The hashes the tier-1 index held when the pass began.
    pub indexed: usize,
    pub files: usize,
    pub failed: usize,
    pub pairs: usize,
    /// Files changed while they were read (or being rewritten by the app): nothing stored, due again.
    pub changed: usize,
}

pub fn import(catalog: &mut Catalog, opts: &Options, retry_errors: bool, ctl: &Control) -> Result<ImportStats> {
    let work = catalog.pending_import(retry_errors)?;
    let items = &work.items;
    let sep = catalog.rules().sep_byte();
    let mut stats = ImportStats { pending: items.len(), indexed: 0, files: 0, failed: 0, pairs: 0, changed: 0 };
    // Most rounds (a new thumbnail height, a turn) have nothing to import: don't load every hash for them.
    if items.is_empty() {
        return Ok(stats);
    }
    let (ids, hashes) = catalog.pdq_hashes()?;
    let mut pairer = Pairer::build(ids, hashes);
    // Every file of the list may add its 8 orientations.
    pairer.reserve(items.len() * 8);
    stats.indexed = pairer.len();

    // The phase ends when this is dropped, however the pass leaves.
    let _phase = ctl.begin(Phase::Import, u64::try_from(items.len()).unwrap_or(u64::MAX));

    let mut flush = |batch: &mut Vec<(usize, Result<Analyzed>)>, stats: &mut ImportStats| -> Result<()> {
        let tx = catalog.tx()?;
        let mut writer = tx.analysis_writer(pg_analysis::analyze::IMPORT_VERSION)?;
        for (i, result) in batch.drain(..) {
            let item = items.get(i).context("an import result for no file")?;
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
                        for &(other, d, v) in pairer.pair_and_insert(item.id, &dihedral)? {
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

    pipeline::batched(
        items,
        |item, wide| {
            if item.kind == Kind::Video {
                // A video is read by FFmpeg, from its path: its size is counted as read.
                return Read::Streamed { bytes: u64::try_from(item.size).unwrap_or(0) };
            }
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            Read::File
        },
        opts,
        ctl,
        Analyzer::new,
        |analyzer, i, bytes| {
            let item = items.get(i).context("no file to import at this index")?;
            if item.kind == Kind::Video {
                // FFmpeg and the hash open the file by its disk path: the plain one would lose a trailing dot or space
                // in a folder's name, and open a sibling's file.
                let disk = pg_platform::names::disk_path_of(&work.path(item.path, sep));
                analyzer.video(&disk, item.rotation)
            } else {
                analyzer.analyze(bytes, item.rotation)
            }
        },
        Batch { size: BATCH, every: None },
        |batch| flush(batch, &mut stats).map(|()| true),
    )?;
    Ok(stats)
}
