//! The Placeholders pass: the ThumbHash placeholder of each file that owes one (`Catalog::pending_placeholders`), made
//! after the thumbnails. A file's placeholder is read from its cached thumbnail when the cache holds one at least
//! `CACHED_MIN_SIDE` high (about a millisecond: the thumbnail is already decoded once, for the Thumbs pass), and from a
//! DCT-scaled decode of the original otherwise. A file whose original doesn't decode is recorded in the Thumbs ledger, as
//! the Thumbs pass records one (`retry = false`: tried again once the file changes, or when the user retries it).
//!
//! The pass is the Hashes pass's shape (`pg_import::hashes`): batches, one transaction per batch, the stop asked after each.
//! It runs with or without the cache, and whatever the grid's height is: it does not depend on the grid.

use crate::{Error, Result};
use pg_catalog::ledger::Failure as LedgerFailure;
use pg_catalog::{Catalog, PlaceholderWork, WorkItem};
use pg_jobs::control::Control;
use pg_jobs::failure::Failed as JobFailed;
use pg_jobs::pipeline::{self, Batch, Options, Read};
use pg_model::analysis::ThumbHash;
use pg_model::media::Kind;
use pg_model::notice::FilePass;
use pg_model::progress::Phase;
use pg_model::size::ThumbHeight;
use pg_render::maker::Maker;
use pg_thumbcache::{Source, ThumbCache};

use crate::{BATCH, Failed, unix_now};

/// The height of the cached thumbnail a placeholder is read from when there is one: the nearest the cache has to it
/// (`ThumbCache::get_nearest_into`), and it is used only if it is at least [`CACHED_MIN_SIDE`] high.
const CACHED_HEIGHT: u32 = 256;
/// The shortest cached thumbnail a placeholder is read from: a shorter one is fitted from a decode of the original,
/// as the placeholder's definition is (`pg_render::maker::PLACEHOLDER_DECODE_SIDE`).
const CACHED_MIN_SIDE: u32 = 200;

/// What a placeholder pass did: the counts its report states.
#[derive(Clone, Copy, Debug)]
pub struct PlaceholderStats {
    /// The files that owed a placeholder when the pass began.
    pub needed: usize,
    /// The files looked at, read or not.
    pub files: usize,
    /// The placeholders stored.
    pub made: usize,
    /// Of those, the ones read from a cached thumbnail (the rest from a decode of the original, or of a video's frame).
    pub from_cache: usize,
    /// The files whose placeholder couldn't be made, for whatever reason.
    pub failed: usize,
    /// Of the failed ones, those that couldn't be read (an offline drive, a file in use): they stay owed.
    pub unreadable: usize,
    /// Files changed since they were listed, or being written by the app: nothing stored, owed again.
    pub changed: usize,
    /// Every file owed was looked at (the pass wasn't stopped).
    pub complete: bool,
}

/// A worker's maker, and the bytes of the cached thumbnail it reads a placeholder from, kept from file to file.
struct Worker {
    maker: Maker,
    jpeg: Vec<u8>,
}

/// Makes the owed placeholders, committing in batches. `still_wanted` is asked after each batch: false ends the pass
/// there, and what is left stays owed for the next round.
pub fn run_while(
    catalog: &mut Catalog,
    cache: &ThumbCache,
    opts: &Options,
    volumes: &pg_catalog::VolumeSet,
    still_wanted: &(dyn Fn() -> bool + Sync),
    ctl: &Control,
) -> Result<PlaceholderStats> {
    // The pass's span in the perf log: the pending query before its phase, the phase, and the transaction time it caused.
    let mut pass = pg_perf::span("pass.placeholders");
    let tx_at = pg_catalog::tx_mark();
    // The maker's steps the workers run for this pass (process-wide tallies, as the thumbnails pass attaches them).
    let steps = [&pg_render::maker::DECODE, &pg_render::maker::RESIZE, &pg_render::maker::THUMBHASH];
    let step_marks = steps.map(|step| step.mark());
    let pending = pg_perf::span("pending.placeholders");
    let work = catalog.pending_placeholders(volumes)?;
    drop(pending);
    let items = &work.items;
    let sep = catalog.rules().sep_byte();
    let stats = PlaceholderStats {
        needed: items.len(),
        files: 0,
        made: 0,
        from_cache: 0,
        failed: 0,
        unreadable: 0,
        changed: 0,
        complete: true,
    };
    if items.is_empty() {
        return Ok(stats);
    }
    // The phase ends when this is dropped, however the pass leaves.
    let _phase = ctl.begin(Phase::Placeholders, u64::try_from(items.len()).unwrap_or(u64::MAX));
    let (state, result) = pipeline::batched(
        pipeline::Pass { items, opts, ctl },
        |pw, wide| {
            let item = &pw.item;
            // A video's frame is read by FFmpeg from its path, and a file whose thumbnail is cached is not read at all.
            if item.kind == Kind::Video || cached(cache, item, &mut Vec::new()).unwrap_or(false) {
                return Read::Streamed { bytes: 0 };
            }
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            Read::File
        },
        || -> Result<Worker> { Ok(Worker { maker: Maker::new()?, jpeg: Vec::new() }) },
        |worker, i, bytes| -> Result<(ThumbHash, bool)> {
            let pw = items.get(i).ok_or(Error::Invalid("a placeholder to make for no file"))?;
            let item = &pw.item;
            if item.kind == Kind::Video {
                // FFmpeg opens the file by its disk path, as the import does.
                let disk = pg_platform::names::disk_path_of(&work.path(item.path, sep));
                let hash = worker.maker.placeholder_of_frame(&disk, pw.poster_ms, item.turns)?;
                return Ok((hash, false));
            }
            if cached(cache, item, &mut worker.jpeg)? {
                let hash = worker.maker.placeholder_of_thumbnail(&worker.jpeg)?;
                return Ok((hash, true));
            }
            // The reader did not read this file (its thumbnail was cached when it looked, and is not now): read it here.
            let read;
            let whole = if bytes.is_empty() {
                let disk = pg_platform::names::disk_path_of(&work.path(item.path, sep));
                read = std::fs::read(&disk)
                    .map_err(|source| Error::Read { what: format!("can't read {}", disk.display()), source })?;
                read.as_slice()
            } else {
                bytes
            };
            Ok((worker.maker.placeholder(whole, item.turns)?, false))
        },
        Batch { size: BATCH, every: None },
        (catalog, stats),
        |(catalog, stats), batch| {
            flush(catalog, batch, items, stats, ctl).map(|()| still_wanted()).map_err(JobFailed::from)
        },
    );
    result?;
    let (_, mut stats) = state;
    stats.complete = stats.files == items.len();
    pass.items(u64::try_from(stats.files).unwrap_or(u64::MAX));
    pass.failed(u64::try_from(stats.failed).unwrap_or(0));
    for (step, mark) in steps.iter().zip(step_marks) {
        pass.attach_since(step, mark);
    }
    pg_catalog::attach_tx(&mut pass, tx_at);
    Ok(stats)
}

/// Whether the cache holds a thumbnail of `item` as it is, at least [`CACHED_MIN_SIDE`] high, and reads its bytes into
/// `jpeg` if so. An error reading the cache is an error of the file (the pass tries it again).
fn cached(cache: &ThumbCache, item: &WorkItem, jpeg: &mut Vec<u8>) -> Result<bool> {
    let source = Source { mtime: item.mtime, turns: item.turns };
    let height = ThumbHeight::new(CACHED_HEIGHT).unwrap_or(ThumbHeight::DEFAULT);
    Ok(cache.get_nearest_into(item.id, height, source, jpeg)?.is_some_and(|h| h.get() >= CACHED_MIN_SIDE))
}

/// Whether a failure is the file's read that failed (an offline drive, a file in use): tried again by the next pass, and
/// not recorded.
fn is_unreadable(e: &JobFailed) -> bool {
    matches!(Failed::of(e), Failed::Unreadable)
}

/// Whether a failure is recorded in the Thumbs ledger: a file that doesn't decode, or panicked. Neither is tried again
/// until it changes or the user retries it (`retry = false`), as the Thumbs pass records them.
fn is_recorded(e: &JobFailed) -> bool {
    matches!(Failed::of(e), Failed::Panicked | Failed::Undecodable)
}

/// One file of a batch: its index in the pass's list, and what making its placeholder came to (the placeholder, and
/// whether it was stored new).
type Made = (usize, std::result::Result<(ThumbHash, bool), JobFailed>);

/// Stores a batch: the placeholders made, and the failures recorded in the Thumbs ledger, in one transaction. A file that
/// changed since it was listed, or is being written, is left as it is (`PlaceholderWriter::store`).
fn flush(
    catalog: &mut Catalog,
    batch: &mut Vec<Made>,
    items: &[PlaceholderWork],
    stats: &mut PlaceholderStats,
    ctl: &Control,
) -> Result<()> {
    let tx = catalog.tx()?;
    let mut writer = tx.placeholder_writer()?;
    let mut done = *stats;
    let mut failures = Vec::new();
    for (i, result) in batch.drain(..) {
        done.files += 1;
        let pw = items.get(i).ok_or(Error::Invalid("a placeholder result for no file"))?;
        let item = &pw.item;
        match result {
            Ok((hash, from_cache)) => {
                if writer.store(item, &hash)? {
                    done.made += 1;
                    done.from_cache += usize::from(from_cache);
                } else {
                    done.changed += 1;
                }
            }
            Err(e) => {
                done.failed += 1;
                if is_unreadable(&e) {
                    done.unreadable += 1;
                } else if is_recorded(&e) {
                    failures.push(LedgerFailure {
                        file_id: item.id,
                        mtime: item.mtime,
                        size: item.size,
                        turns: item.turns,
                        error: e.text().to_string(),
                        retry: false,
                    });
                }
            }
        }
    }
    drop(writer);
    let changed =
        if failures.is_empty() { false } else { tx.ledger_writer().record(FilePass::Thumbs, &failures, unix_now())? };
    tx.commit()?;
    // Counted once the batch is in: a batch that failed stored nothing.
    *stats = done;
    if changed {
        ctl.ledger_changed();
    }
    Ok(())
}
