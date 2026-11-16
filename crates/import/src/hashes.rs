//! The Hashes pass: the duplicate hashes of the files the import stored without them (`hashes_owed`, while duplicates
//! detection was off). Each file is decoded as the import decodes it (`Analyzer::hash_image`, `Analyzer::video_file_hash`),
//! so its values are the import's own. The pairs they owe are the pairing pass's (`pairs_owed`, set by
//! `AnalysisWriter::store_hashes`).

use std::io;

use anyhow::{Context, Result};
use pg_analysis::analyze::Analyzer;
use pg_catalog::{Catalog, OwedHashes, WorkItem};
use pg_jobs::control::Control;
use pg_jobs::panics;
use pg_jobs::pipeline::{self, Batch, Options, Read};
use pg_model::media::Kind;
use pg_model::progress::Phase;

const BATCH: usize = 500;

#[derive(Clone, Copy, Debug)]
pub struct Stats {
    /// The files looked at, read or not.
    pub files: usize,
    /// Of those, the ones whose hashes couldn't be made: unreadable (they stay owed, for the next pass) or failed (stored
    /// as the import stores a failure: the file's analysis is an error, with no hashes to owe).
    pub failed: usize,
    /// Of the failed ones, those that couldn't be read (a locked file, an offline drive).
    pub unreadable: usize,
    /// The files whose hashes went into the catalog.
    pub stored: usize,
    /// Every file owed was looked at (the pass wasn't stopped).
    pub complete: bool,
}

/// Makes the owed hashes, committing in batches. `still_wanted` is asked after each batch: false ends the pass there, and
/// what is left stays owed for the next round.
pub fn hash(
    catalog: &mut Catalog,
    opts: &Options,
    still_wanted: &(dyn Fn() -> bool + Sync),
    ctl: &Control,
) -> Result<Stats> {
    let work = catalog.pending_hashes()?;
    let items = &work.items;
    let stats = Stats { files: 0, failed: 0, unreadable: 0, stored: 0, complete: true };
    if items.is_empty() {
        return Ok(stats);
    }
    let sep = catalog.rules().sep_byte();
    // The phase ends when this is dropped, however the pass leaves.
    let _phase = ctl.begin(Phase::Hashes, u64::try_from(items.len()).unwrap_or(u64::MAX));
    let (state, result) = pipeline::batched(
        items,
        |item, wide| {
            if item.kind == Kind::Video {
                // A video is read by FFmpeg, from its path, and streamed for its hash: its size is counted as read.
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
            let item = items.get(i).context("no file to hash at this index")?;
            if item.kind == Kind::Video {
                // The hash opens the file by its disk path, as the import does.
                let disk = pg_platform::names::disk_path_of(&work.path(item.path, sep));
                analyzer.video_file_hash(&disk).map(OwedHashes::Video)
            } else {
                analyzer.hash_image(bytes, item.rotation).map(OwedHashes::Image)
            }
        },
        Batch { size: BATCH, every: None },
        (catalog, stats),
        |(catalog, stats), batch| flush(catalog, batch, items, stats, ctl).map(|()| still_wanted()),
    );
    result?;
    let (_, mut stats) = state;
    stats.complete = stats.files == items.len();
    Ok(stats)
}

/// Stores a batch: the hashes made, the failures as the import stores them, in one transaction. A file that couldn't be
/// read stays owed; one that changed since it was listed is left as it is (the writer's guard).
fn flush(
    catalog: &mut Catalog,
    batch: &mut Vec<(usize, Result<OwedHashes>)>,
    items: &[WorkItem],
    stats: &mut Stats,
    ctl: &Control,
) -> Result<()> {
    let tx = catalog.tx()?;
    let mut writer = tx.analysis_writer(pg_analysis::analyze::IMPORT_VERSION)?;
    let mut done = *stats;
    for (i, result) in batch.drain(..) {
        done.files += 1;
        let item = items.get(i).context("a hash result for no file")?;
        match result {
            Ok(hashes) => {
                if writer.store_hashes(item, &hashes)? {
                    done.stored += 1;
                }
            }
            Err(e) if e.downcast_ref::<io::Error>().is_some() && !panics::is_panic(&e) => {
                done.failed += 1;
                done.unreadable += 1;
            }
            Err(e) => {
                done.failed += 1;
                writer.store_failure(item, &format!("{e:#}"))?;
            }
        }
    }
    drop(writer);
    tx.commit()?;
    // Counted once the batch is in: a batch that failed stored nothing.
    *stats = done;
    // The failure ledger is kept by the catalog's triggers, as the import's is: whoever shows it reads it again.
    ctl.ledger_changed();
    Ok(())
}
