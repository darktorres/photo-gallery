//! The Hashes pass: the duplicate hashes of the files the import stored without them (`hashes_owed`, while duplicates
//! detection was off). Each file is decoded as the import decodes it (`Analyzer::hash_image`, `Analyzer::video_file_hash`),
//! so its values are the import's own. The pairs they owe are the pairing pass's (`pairs_owed`, set by
//! `AnalysisWriter::store_hashes`).

use crate::{Error, Result, in_batch_tx, video_disk_path};
use pg_analysis::analyze::Analyzer;
use pg_catalog::{Catalog, OwedHashes, WorkItem};
use pg_jobs::control::Control;
use pg_jobs::failure::{Class, Failed};
use pg_jobs::pipeline::{self, Batch, Done, Halt, ItemError, Options, Read};
use pg_model::media::Kind;
use pg_model::num::count_u64;
use pg_model::progress::Phase;

/// What the hashes pass did over its files: the counts its report states.
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
    volumes: &pg_catalog::VolumeSet,
    still_wanted: &(dyn Fn() -> bool + Sync),
    ctl: &Control,
) -> Result<Stats> {
    let mut pass = pg_perf::span("pass.hashes");
    let tx_at = pg_catalog::tx_mark();
    let pending = pg_perf::span("pending.hashes");
    let work = catalog.pending_hashes(volumes)?;
    drop(pending);
    let items = &work.items;
    let stats = Stats { files: 0, failed: 0, unreadable: 0, stored: 0, complete: true };
    if items.is_empty() {
        return Ok(stats);
    }
    let sep = catalog.rules().sep_byte();
    // The phase ends when this is dropped, however the pass leaves.
    let _phase = ctl.begin(Phase::Hashes, count_u64(items.len()));
    let (state, result) = pipeline::batched(
        pipeline::Pass { items, opts, ctl },
        |item, wide| {
            if item.kind == Kind::Video {
                // A video is read by FFmpeg, from its path, and streamed for its hash: its size is counted as read.
                return Read::Streamed { bytes: item.size };
            }
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            Read::File
        },
        || Analyzer::new().map_err(Error::Analysis),
        |analyzer, item, bytes| {
            // A hash the analysis refuses is the file's failure (`Data`), as the import's (section 10, D4).
            let refused = |e| ItemError::File(Failed::new(Class::Data, Error::Analysis(e)));
            if item.kind == Kind::Video {
                // The hash opens the file by its disk path, as the import does. It is read in chunks, the stop looked for
                // between them (section 7): a stop is "not hashed yet", not a failure.
                let disk = video_disk_path(&work, item.path, sep);
                let stopped = || ctl.stop_requested();
                analyzer.video_file_hash(&disk, &stopped).map(|h| h.map(OwedHashes::Video)).map_err(refused)
            } else {
                analyzer.hash_image(bytes, item.turns).map(|h| Some(OwedHashes::Image(h))).map_err(refused)
            }
        },
        Batch { size: pg_catalog::WRITE_BATCH, every: None },
        (catalog, stats),
        |(catalog, stats), batch| flush(catalog, batch, stats, ctl).map(|()| still_wanted()),
    );
    result.map_err(Halt::into_pass)?;
    let (_, mut stats) = state;
    stats.complete = stats.files == items.len();
    pass.items(count_u64(stats.files));
    pass.failed(count_u64(stats.failed));
    pg_catalog::attach_tx(&mut pass, tx_at);
    Ok(stats)
}

/// Stores a batch: the hashes made, the failures as the import stores them, in one transaction. A file that couldn't be
/// read stays owed; one that changed since it was listed is left as it is (the writer's guard).
fn flush(
    catalog: &mut Catalog,
    batch: &mut Vec<Done<'_, WorkItem, Option<OwedHashes>, Error>>,
    stats: &mut Stats,
    ctl: &Control,
) -> Result<()> {
    let mut done = *stats;
    in_batch_tx(catalog, ctl, |tx| {
        let mut writer = tx.analysis_writer(pg_analysis::analyze::IMPORT_VERSION)?;
        for (item, result) in batch.drain(..) {
            done.files += 1;
            let result = match result {
                Ok(hashes) => Ok(hashes),
                Err(ItemError::File(failed)) => Err(failed),
                // The pass's own error, met on an item: the batch stores nothing, and the pass ends with it.
                Err(ItemError::Pass(e)) => return Err(e),
            };
            match result {
                Ok(Some(hashes)) => {
                    if writer.store_hashes(item, &hashes)? {
                        done.stored += 1;
                    }
                }
                // Stopped inside its hash (a video's, section 7): not hashed yet. It stays owed and is not counted as looked
                // at, so the pass is not complete; the next round hashes it.
                Ok(None) => done.files = done.files.saturating_sub(1),
                // Changed while it was read: nothing is stored, and it stays owed, as the import leaves such a file pending.
                Err(e) if e.is_changed() => {}
                Err(e) if e.is_read() => {
                    done.failed += 1;
                    done.unreadable += 1;
                }
                Err(e) => {
                    done.failed += 1;
                    writer.store_failure(item, e.text())?;
                }
            }
        }
        Ok(())
    })?;
    // Counted once the batch is in: a batch that failed stored nothing.
    *stats = done;
    Ok(())
}
