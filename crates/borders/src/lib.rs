//! Borders (`pg_analysis::borders`): the pass after the import that looks for them in the files it analyzed, and in
//! every file after a new detector (`analysis.borders_due`), each decoded at full size. The tools that report on
//! borders, or take review marks into the catalog, are the command line's (`pg_cli::borders`).

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use pg_analysis::analyze::{Analyzer, FileBorders};
use pg_catalog::Catalog;
use pg_catalog::ledger::Failure;
use pg_jobs::control::Control;
use pg_jobs::failure::{Class, Failed};
use pg_jobs::pipeline::{self, Batch, Options, Read};
use pg_model::ids::FileId;
use pg_model::notice::FilePass;
use pg_model::progress::Phase;

#[cfg(feature = "fault")]
pub mod fault;

const BATCH: usize = pg_catalog::WRITE_BATCH;

/// What the borders pass couldn't do: the detector failed on a file, the catalog failed, the pipeline failed, or the
/// pass's own list didn't hold the item it was asked for.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The detector failed on a file's bytes (see `pg_analysis::Error`): the file doesn't decode, or the detector can't
    /// take it.
    #[error(transparent)]
    Analysis(#[from] pg_analysis::Error),
    /// The catalog failed (see `pg_catalog::Error`).
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    /// The pass's pipeline failed (see `pg_jobs::failure::Failed`).
    #[error(transparent)]
    Pass(#[from] Failed),
    /// An item the pass's list doesn't hold.
    #[error("{0}")]
    Invalid(&'static str),
}

/// The result of the borders pass.
pub type Result<T> = std::result::Result<T, Error>;

impl From<Error> for Failed {
    /// A file the detector refused is a data failure: recorded, with no borders, so it isn't tried again every pass.
    /// Anything else of the pass is an ordinary failure.
    fn from(error: Error) -> Failed {
        match error {
            Error::Pass(failed) => failed,
            Error::Analysis(_) => Failed::new(Class::Data, error),
            Error::Catalog(_) | Error::Invalid(_) => Failed::new(Class::Other, error),
        }
    }
}

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
use pg_platform::clock::unix_now;

/// Look for the borders that are due, committing in batches (stopping loses at most the batch). The files that don't
/// decode, or that the detector panics on, are recorded in the catalog's failure ledger (`pg_catalog::ledger`); the
/// ledger's files whose borders are found leave it. The ledger's rows are written in the transaction of the borders
/// they go with (`Tx::ledger_writer`): a batch stores both or neither, so a failure to write the ledger stores
/// nothing, leaves the batch's files due, and fails the pass.
pub fn find(catalog: &mut Catalog, opts: &Options, volumes: &pg_catalog::VolumeSet, ctl: &Control) -> Result<Stats> {
    find_while(catalog, opts, volumes, &|| true, ctl)
}

/// [`find`], which ends after the batch in which `still_wanted` turns false: the files it had not looked at stay due.
/// `volumes` limits the pass to those volumes' files.
pub fn find_while(
    catalog: &mut Catalog,
    opts: &Options,
    volumes: &pg_catalog::VolumeSet,
    still_wanted: &(dyn Fn() -> bool + Sync),
    ctl: &Control,
) -> Result<Stats> {
    let mut pass = pg_perf::span("pass.borders");
    let tx_at = pg_catalog::tx_mark();
    let pending = pg_perf::span("pending.borders");
    let work = catalog.pending_borders(volumes)?;
    drop(pending);
    let items = &work.items;
    let stats = Stats { files: 0, failed: 0, unreadable: 0, stored: 0, complete: true };
    if items.is_empty() {
        return Ok(stats);
    }
    let known: BTreeSet<FileId> = catalog.recorded_failures(FilePass::Borders)?;
    // The phase ends when this is dropped, however the pass leaves.
    let _phase = ctl.begin(Phase::Borders, u64::try_from(items.len()).unwrap_or(u64::MAX));
    type Done = std::result::Result<FileBorders, Failed>;
    let flush = |catalog: &mut Catalog, batch: &mut Vec<(usize, Done)>, stats: &mut Stats| -> Result<()> {
        let tx = catalog.tx()?;
        let mut done = *stats;
        let changed = {
            let mut writer = tx.borders_writer()?;
            let (mut failures, mut found_known) = (Vec::new(), Vec::new());
            for (i, result) in batch.drain(..) {
                done.files += 1;
                let item = items.get(i).ok_or(Error::Invalid("an item outside the pass"))?;
                let (size, found) = match result {
                    Ok(b) => {
                        if known.contains(&item.id) {
                            found_known.push(item.id);
                        }
                        // The decode's own size: the header's, corrected to it (`store_borders`).
                        (Some((b.width, b.height)), b.borders)
                    }
                    // Couldn't be read (a locked file, an offline drive): nothing is stored, so the file stays due and
                    // the next pass tries it again. Storing "no borders" here would hide it until BORDERS_VERSION
                    // changed. A panic in the read is the detector's own (below).
                    Err(e) if e.is_read() => {
                        done.failed += 1;
                        done.unreadable += 1;
                        continue;
                    }
                    // It doesn't decode, or the detector panicked on it: no borders, for the file as it is, so a file
                    // that panics isn't tried again every round; the ledger records it.
                    Err(e) => {
                        done.failed += 1;
                        failures.push(Failure {
                            file_id: item.id,
                            mtime: item.mtime,
                            size: item.size,
                            turns: item.turns,
                            error: e.text().to_string(),
                            retry: false,
                        });
                        (None, None)
                    }
                };
                if writer.store_borders(item, size, found.as_ref())? {
                    done.stored += 1;
                }
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
    let (state, result) = pipeline::batched(
        items,
        |item, wide| {
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            Read::File
        },
        opts,
        ctl,
        || Analyzer::new().map_err(Error::Analysis),
        |analyzer, i, bytes| {
            let item = items.get(i).ok_or(Error::Invalid("an item outside the pass"))?;
            #[cfg(feature = "fault")]
            fault::in_detector(item.id);
            analyzer.borders(bytes, item.turns).map_err(Error::Analysis)
        },
        Batch { size: BATCH, every: None },
        (catalog, stats),
        |(catalog, stats), batch| flush(catalog, batch, stats).map(|()| still_wanted()).map_err(Failed::from),
    );
    result?;
    let (_, mut stats) = state;
    stats.complete = stats.files == items.len();
    pass.items(u64::try_from(stats.stored).unwrap_or(u64::MAX));
    pg_catalog::attach_tx(&mut pass, tx_at);
    Ok(stats)
}
