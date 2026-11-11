//! Borders (`pg_analysis::borders`): the pass after the import that looks for them in the files it analyzed, and in
//! every file after a new detector (`analysis.borders_due`), each decoded at full size. The tools that report on
//! borders, or take review marks into the catalog, are the command line's (`pg_cli::borders`).

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use pg_analysis::analyze::{Analyzer, FileBorders};
use pg_catalog::Catalog;
use pg_catalog::ledger::Failure;
use pg_jobs::control::Control;
use pg_jobs::panics;
use pg_jobs::pipeline::{self, Batch, Options, Read};
use pg_model::ids::FileId;
use pg_model::notice::FilePass;
use pg_model::progress::Phase;

#[cfg(feature = "fault")]
pub mod fault;

const BATCH: usize = 500;

#[derive(Clone, Copy, Debug)]
pub struct Stats {
    /// The files looked at, read or not.
    pub files: usize,
    /// Of those, the ones that couldn't be read or decoded.
    pub failed: usize,
    /// Of the failed ones, those that couldn't be read (a locked file, an offline drive): tried again by the next pass,
    /// and not recorded in the failure ledger.
    pub unreadable: usize,
    /// The files whose result went into the catalog: those read (borders or none), and those that don't decode
    /// (stored as none). A file that couldn't be read stores nothing and stays due.
    pub stored: usize,
    /// Every file due was looked at (the pass wasn't stopped).
    pub complete: bool,
}

/// Unix seconds now.
fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Look for the borders that are due, committing in batches (stopping loses at most the batch). The files that don't
/// decode, or that the detector panics on, are recorded in the catalog's failure ledger (`pg_catalog::ledger`); the
/// ledger's files whose borders are found leave it. The ledger's rows are written in the transaction of the borders
/// they go with (`Tx::ledger_writer`): a batch stores both or neither, so a failure to write the ledger stores
/// nothing, leaves the batch's files due, and fails the pass.
pub fn find(catalog: &mut Catalog, opts: &Options, ctl: &Control) -> Result<Stats> {
    let work = catalog.pending_borders()?;
    let items = &work.items;
    let mut stats = Stats { files: 0, failed: 0, unreadable: 0, stored: 0, complete: true };
    if items.is_empty() {
        return Ok(stats);
    }
    let known: BTreeSet<FileId> = catalog.recorded_failures(FilePass::Borders)?;
    // The phase ends when this is dropped, however the pass leaves.
    let _phase = ctl.begin(Phase::Borders, u64::try_from(items.len()).unwrap_or(u64::MAX));
    let mut flush = |batch: &mut Vec<(usize, Result<FileBorders>)>, stats: &mut Stats| -> Result<()> {
        let tx = catalog.tx()?;
        let mut done = *stats;
        let changed = {
            let mut writer = tx.borders_writer()?;
            let (mut failures, mut found_known) = (Vec::new(), Vec::new());
            for (i, result) in batch.drain(..) {
                done.files += 1;
                let item = items.get(i).context("an item outside the pass")?;
                let found = match result {
                    Ok(b) => {
                        if known.contains(&item.id) {
                            found_known.push(item.id);
                        }
                        b.borders
                    }
                    // Couldn't be read (a locked file, an offline drive): nothing is stored, so the file stays due and
                    // the next pass tries it again. Storing "no borders" here would hide it until BORDERS_VERSION
                    // changed. A panic in the read is the detector's own (below).
                    Err(e) if e.downcast_ref::<std::io::Error>().is_some() && !panics::is_panic(&e) => {
                        done.failed += 1;
                        done.unreadable += 1;
                        continue;
                    }
                    // It doesn't decode, or the detector panicked on it (`pg_jobs::panics::Panicked`): no borders, for
                    // the file as it is, so a file that panics isn't tried again every round; the ledger records it.
                    Err(e) => {
                        done.failed += 1;
                        failures.push(Failure {
                            file_id: item.id,
                            mtime: item.mtime,
                            size: item.size,
                            rotation: item.rotation,
                            error: format!("{e:#}"),
                            retry: false,
                        });
                        None
                    }
                };
                writer.store_borders(item, found.as_ref())?;
                done.stored += 1;
            }
            drop(writer);
            // In the batch's transaction: the failures it found, and the files of the ledger it found borders for (a
            // file that failed again is never among those, so the failure path never clears its own row).
            let mut ledger = tx.ledger_writer();
            let recorded = ledger.record(FilePass::Borders, &failures, unix_now())?;
            let cleared = ledger.clear(FilePass::Borders, &found_known)?;
            recorded || cleared
        };
        tx.commit()?;
        // Counted once the batch is in: a batch that failed stored nothing.
        *stats = done;
        if changed {
            ctl.ledger_changed();
        }
        Ok(())
    };
    pipeline::batched(
        items,
        |item, wide| {
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            Read::File
        },
        opts,
        ctl,
        Analyzer::new,
        |analyzer, i, bytes| {
            let item = items.get(i).context("an item outside the pass")?;
            #[cfg(feature = "fault")]
            fault::in_detector(item.id);
            analyzer.borders(bytes, item.rotation)
        },
        Batch { size: BATCH, every: None },
        |batch| flush(batch, &mut stats).map(|()| true),
    )?;
    stats.complete = stats.files == items.len();
    Ok(stats)
}
