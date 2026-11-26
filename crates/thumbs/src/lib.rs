//! The background thumbnail task: after the import, thumbnails for every analyzed file at the height
//! the grid currently uses, in on-disk order, until done or until the cache is full of that height. Runs at idle
//! priority, as it competes with the user for the disk and the CPU. The cache is shared with the app's on-demand
//! thumbnails; neither waits for the other but in the cache's writer, one batch at a time.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use pg_catalog::ledger::Failure as LedgerFailure;
use pg_catalog::{Catalog, Listed};
use pg_render::maker::{MakeError, Maker};
use pg_thumbcache::{Pending, Thumb, ThumbCache, ThumbWork};

use pg_jobs::control::Control;
use pg_jobs::failure::{Class, Failed as JobFailed, file_failure, keep_file_failure, stored_path};
use pg_jobs::pipeline::{self, Batch, Done, Halt, ItemError, Options, Read};
use pg_model::ids::FileId;
use pg_model::notice::FileFailure;
use pg_model::notice::FilePass;
use pg_model::progress::Phase;
use pg_model::size::ThumbHeight;

mod pending;
pub mod placeholders;

/// What a thumbnail or placeholder pass couldn't do: the catalog failed, the cache couldn't store or couldn't be read, a
/// file couldn't be read (its read's error), a thumbnail couldn't be made (`Make`), the maker couldn't be started
/// (`Render`), the pipeline failed, or the cache said something it can't hold (`Invalid`). The cache's two are told apart
/// where it is called (section 10, D7): its stores are one round error, its reads another, each with its own notice.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The catalog failed (see `pg_catalog::Error`).
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    /// The thumbnail cache couldn't store what the pass made, or keep its marks (`put_batch`, `checked`, `shrink`).
    #[error(transparent)]
    CacheStore(pg_thumbcache::Error),
    /// The thumbnail cache couldn't be read (the pending work's marks and entries, its totals, a cached thumbnail).
    #[error(transparent)]
    CacheRead(pg_thumbcache::Error),
    /// The maker couldn't make a picture of a file (see `pg_render::MakeError`).
    #[error(transparent)]
    Make(#[from] MakeError),
    /// The maker couldn't be started (see `pg_render::Error`).
    #[error(transparent)]
    Render(#[from] pg_render::Error),
    /// A file couldn't be read: its words, and the I/O error under them.
    #[error("{what}")]
    Read {
        /// The file, in words.
        what: String,
        /// The I/O error under the read.
        #[source]
        source: std::io::Error,
    },
    /// The pass's pipeline failed (see `pg_jobs::failure::Failed`).
    #[error(transparent)]
    Pipeline(#[from] JobFailed),
    /// The cache said something it can't hold (a negative size).
    #[error("{0}")]
    Invalid(&'static str),
}

/// The result of a thumbnail or placeholder pass's steps.
pub type Result<T> = std::result::Result<T, Error>;

/// A pass that ended with an error (section 10, D6): the error, and what the pass did before it (its ledger outcome, its
/// unreadable count, its failures), which its caller reports in every case. Before the pass's setup, the stats are empty.
pub struct Stopped<S> {
    /// Why the pass ended.
    pub error: Error,
    /// What it did before.
    pub stats: S,
}

impl<S> std::fmt::Debug for Stopped<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stopped").field("error", &self.error).finish_non_exhaustive()
    }
}

impl<S> std::fmt::Display for Stopped<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() { write!(f, "{:#}", self.error) } else { write!(f, "{}", self.error) }
    }
}

impl<S> std::error::Error for Stopped<S> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.error)
    }
}

impl<S> From<Stopped<S>> for Error {
    fn from(stopped: Stopped<S>) -> Error {
        stopped.error
    }
}

/// What a thumbnail or placeholder pass returns: its stats, or its error with them ([`Stopped`], boxed: the stats are
/// large beside a pass's `Ok`).
pub type Ran<S> = std::result::Result<S, Box<Stopped<S>>>;

/// A pass step's error, with the stats so far: what `?` makes of an error inside a pass.
pub(crate) fn stopped<S>(stats: S) -> impl FnOnce(Error) -> Box<Stopped<S>> {
    move |error| Box::new(Stopped { error, stats })
}

/// A file's thumbnail or placeholder that couldn't be made, as the pipeline's one contract has it (section 10, D4): a
/// read that failed (the maker's, or the pass's own) is `Read`, tried again by the next pass; a picture that doesn't
/// decode is `Data`, recorded. Every other error is the pass's, and halts it.
pub(crate) fn file_failed(error: Error) -> ItemError<Error> {
    match error {
        Error::Make(MakeError::Unreadable(_)) | Error::Read { .. } => {
            ItemError::File(JobFailed::new(Class::Read, error))
        }
        Error::Make(MakeError::Undecodable(_)) => ItemError::File(JobFailed::new(Class::Data, error)),
        other => ItemError::Pass(other),
    }
}

/// The maker a worker starts with (section 10, D5): production's makes a new one, its failure the pass's `Render`; a test
/// can give one that fails.
pub type MakerSource = dyn Fn() -> Result<Maker> + Sync;

/// Production's maker source: a new [`Maker`], its failure the pass's `Render` error.
pub fn new_maker() -> Result<Maker> {
    Maker::new().map_err(Error::Render)
}

/// The files one batch of a pass takes: the thumbnail pass's and the placeholder pass's (`placeholders`).
pub(crate) const BATCH: usize = 200;

/// The failures found are written to the ledger at least this often, so the inbox follows a long pass.
const LEDGER_EVERY: Duration = Duration::from_secs(2);

/// What a thumbnail pass did: the counts its report states, the failures it kept, and whether the cache held.
pub struct ThumbStats {
    /// The files that needed a thumbnail at the height when the pass began.
    pub needed: usize,
    /// The thumbnail cache's size (bytes) when the pass began.
    pub cache_bytes: u64,
    /// The thumbnails made by the pass.
    pub made: usize,
    /// The files whose thumbnail couldn't be made, for whatever reason ([`ThumbStats::unreadable`] among them).
    pub failed: usize,
    /// Of the failed files, those that couldn't be read (an offline drive, a file in use): tried again by the next pass,
    /// and not recorded in the failure ledger.
    pub unreadable: usize,
    /// The first failures (`FAILURES_KEPT`), for whoever shows the pass.
    pub failures: Vec<FileFailure>,
    /// Files changed while they were read (or being rewritten by the app): not kept, tried again.
    pub changed: usize,
    /// Whether the cache had no room for the next thumbnail.
    pub cache_full: bool,
    /// The cache was turned off during the pass (`ThumbCache::set_suspended`): a batch it was asked to store was not
    /// stored, so the pass ended there. Not a full cache.
    pub stopped: bool,
    /// Every file of the work was tried (not stopped, the cache not full, the cache not turned off during the pass):
    /// the next pass starts from this one's snapshot (`ThumbCache::checked`).
    pub complete: bool,
    /// What the pass's writes to the failure ledger came to.
    pub ledger: LedgerOutcome,
}

/// What a thumbnail pass's writes to the failure ledger came to. The ledger is in the catalog, apart from the cache the
/// thumbnails go to, so it can fail on its own: then the files it would have held are tried again by the next pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerOutcome {
    /// The pass had nothing to write or clear.
    Untouched,
    /// Every write and clear it made went through.
    Written,
    /// A write or a clear failed (the first error), whatever went through besides.
    Failed(String),
}

impl LedgerOutcome {
    /// A write or a clear went through.
    fn written(&mut self) {
        if *self == LedgerOutcome::Untouched {
            *self = LedgerOutcome::Written;
        }
    }

    /// A write or a clear failed with `error`: the first failure is kept.
    fn failed(&mut self, error: &(dyn std::error::Error + 'static)) {
        if !matches!(self, LedgerOutcome::Failed(_)) {
            *self = LedgerOutcome::Failed(pg_model::error::chain(error));
        }
    }
}

/// What a failed thumbnail is, for what is done about it.
#[derive(Debug, PartialEq, Eq)]
enum Failed {
    /// It panicked: recorded, and tried again only once the file changes (it would panic again, and log its backtrace
    /// at every pass), or the user retries it, or a new thumbnailer comes.
    Panicked,
    /// It doesn't decode: recorded, and tried again only once the file changes (or the user retries it), as a panic is: a
    /// file whose header the import read may still not decode, and it would fail again at every pass.
    Undecodable,
    /// It couldn't be read (an offline drive, a file in use): tried again by the next pass, not recorded.
    Unreadable,
    /// Anything else: tried again, not recorded.
    Other,
}

impl Failed {
    /// By the error's type, never its text or its root cause: a truncated file's decode error has an I/O error at its
    /// root, and is no read failure.
    fn of(e: &JobFailed) -> Failed {
        match e.class() {
            Class::Panic => Failed::Panicked,
            Class::Data => Failed::Undecodable,
            Class::Read => Failed::Unreadable,
            Class::Other => Failed::Other,
        }
    }

    /// Whether the ledger records it, and with which retry: `Some(false)` is recorded and tried again only once the file
    /// changes (or the user retries it), which is every recorded failure today. `Some(true)` (the ledger's retry flag,
    /// tried again by the next pass) is not produced by any failure yet. `None` is not recorded (tried again, and no
    /// failure of the file's).
    fn recorded(&self) -> Option<bool> {
        match self {
            Failed::Panicked | Failed::Undecodable => Some(false),
            Failed::Unreadable | Failed::Other => None,
        }
    }
}

/// Unix seconds now.
pub(crate) use pg_platform::clock::unix_now;

/// `seq` folded into the smallest change number of the files to try again.
fn fold(first_failed: &mut Option<i64>, seq: i64) {
    *first_failed = Some(first_failed.map_or(seq, |s: i64| s.min(seq)));
}

/// The failures waiting to be written to the ledger, with each file's change number: one that couldn't be written is
/// tried again by the next pass (its number holds the checkpoint back), so no panicked file is lost with it.
struct Unwritten {
    failures: Vec<(LedgerFailure, i64)>,
    since: Instant,
}

impl Unwritten {
    /// Write the failures waiting, in a transaction of their own. A write that changed rows is announced
    /// (`Control::ledger_changed`); one that failed holds their files' change numbers back.
    fn write(&mut self, catalog: &Catalog, first_failed: &mut Option<i64>, ledger: &mut LedgerOutcome, ctl: &Control) {
        self.since = Instant::now();
        if self.failures.is_empty() {
            return;
        }
        let failures: Vec<LedgerFailure> = self.failures.iter().map(|(f, _)| f.clone()).collect();
        match catalog.record_failures(FilePass::Thumbs, &failures, unix_now()) {
            Ok(changed) => {
                ledger.written();
                if changed {
                    ctl.ledger_changed();
                }
            }
            Err(e) => {
                for (_, seq) in &self.failures {
                    fold(first_failed, *seq);
                }
                ledger.failed(&e);
            }
        }
        self.failures.clear();
    }
}

/// What the thumbs pass's writer keeps across its batches: the catalog the files are checked against and the failures are
/// written to, the thumbnails waiting to be kept, and what the pass has counted so far.
struct Writing {
    catalog: Catalog,
    thumbs: Vec<Thumb>,
    unwritten: Unwritten,
    first_failed: Option<i64>,
    tried: usize,
    stats: ThumbStats,
}

/// One pass at `height`: the files changed since the last complete one (or all of them, `ThumbCache::pending`). The
/// files whose thumbnail couldn't be made are recorded in the catalog's failure ledger (`pg_catalog::ledger`), and the
/// ledger's files whose thumbnail it makes leave it.
pub fn run(
    cache: &ThumbCache,
    catalog_path: &Path,
    thumb: ThumbHeight,
    opts: &Options,
    ctl: &Control,
) -> Ran<ThumbStats> {
    run_while(cache, catalog_path, thumb, opts, &pg_catalog::VolumeSet::All, &|| true, &new_maker, ctl)
}

impl ThumbStats {
    /// A pass's stats before it made anything: `needed` files to make.
    fn empty(needed: usize) -> ThumbStats {
        ThumbStats {
            needed,
            cache_bytes: 0,
            made: 0,
            failed: 0,
            unreadable: 0,
            failures: Vec::new(),
            changed: 0,
            cache_full: false,
            stopped: false,
            complete: false,
            ledger: LedgerOutcome::Untouched,
        }
    }
}

/// [`run`], which ends after the batch in which `still_wanted` turns false: the files it had not made stay changed (no
/// `checked` row is written), so the next pass lists them again. The grid's height changing is such a reason. Each worker's
/// maker comes from `maker` (section 10, D5: production passes [`new_maker`]). An error comes back with what the pass did
/// before it ([`Stopped`], D6), its kind telling the cache's stores and reads from the rest (D7).
#[expect(
    clippy::too_many_arguments,
    reason = "the pass's inputs, each its own: the cache, the catalog, the height, ..."
)]
pub fn run_while(
    cache: &ThumbCache,
    catalog_path: &Path,
    thumb: ThumbHeight,
    opts: &Options,
    volumes: &pg_catalog::VolumeSet,
    still_wanted: &(dyn Fn() -> bool + Sync),
    maker: &MakerSource,
    ctl: &Control,
) -> Ran<ThumbStats> {
    // The pass's span in the perf log: the pending query and the setup before its phase, the phase, and the finish after.
    let mut pass = pg_perf::span("pass.thumbs");
    let tx_at = pg_catalog::tx_mark();
    // The maker's steps the workers run for this pass: process-wide tallies, so the pass attaches what they grew by.
    let steps = [&pg_render::maker::DECODE, &pg_render::maker::RESIZE, &pg_render::maker::ENCODE];
    let step_marks = steps.map(|step| step.mark());
    // To read the candidates from, check each file against before its thumbnail is kept, and for the failure ledger.
    let catalog = Catalog::open(catalog_path, pg_platform::native::PATH_RULES)
        .map_err(Error::from)
        .map_err(stopped(ThumbStats::empty(0)))?;
    let pending = pg_perf::span("pending.thumbs");
    // The catalog's candidates and the cache's entries, merged (section 10, D10): each read on its own owner's
    // connection, so a failure says whose it was.
    let Pending { work, snapshot } =
        pending::work(cache, &catalog, thumb, volumes).map_err(stopped(ThumbStats::empty(0)))?;
    drop(pending);
    let items = &work.items;
    let mut stats = ThumbStats::empty(items.len());
    if items.is_empty() {
        let finished = cache.checked(&snapshot, None).and_then(|()| cache.shrink()).map_err(Error::CacheStore);
        stats.complete = true;
        return finished.map(|()| stats).map_err(stopped(ThumbStats::empty(0)));
    }
    stats.cache_bytes = match cache.totals().map_err(Error::CacheRead) {
        Ok((bytes, _)) => match u64::try_from(bytes) {
            Ok(bytes) => bytes,
            Err(_) => return Err(stopped(stats)(Error::Invalid("the cache's size is negative"))),
        },
        Err(error) => return Err(stopped(stats)(error)),
    };
    let setup = pg_perf::span("thumbs.setup");
    let sep = catalog.rules().sep_byte();
    // The files the ledger holds: only those leave it when they are made (a clear of every file made would take the
    // catalog's write lock at every batch).
    let known: BTreeSet<FileId> = match catalog.recorded_failures(FilePass::Thumbs) {
        Ok(known) => known,
        Err(e) => return Err(stopped(stats)(Error::Catalog(e))),
    };
    drop(setup);
    // The thumbnails' buffers, back from the cache's writer with each batch (and from the ones not kept), for the
    // workers to make the next ones in: about a batch's worth and one per worker and slot, not one per file.
    let spare: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
    // The phase ends when this is dropped: after the last flush, before the cache is checked and shrunk, or when the
    // pass leaves early (an error, a panic).
    let phase = ctl.begin(Phase::Thumbs { height: thumb }, u64::try_from(items.len()).unwrap_or(u64::MAX));
    // The writer's state: the catalog (moved in, and back when the pass ends), the thumbnails to keep, the failures.
    let writing = Writing {
        catalog,
        thumbs: Vec::with_capacity(BATCH),
        unwritten: Unwritten { failures: Vec::new(), since: Instant::now() },
        first_failed: None,
        tried: 0,
        stats,
    };

    // A batch of results, written by the writer's thread: the thumbnails made from the file as listed are kept, the
    // failures recorded; false once the cache is full.
    let flush = |writing: &mut Writing, batch: &mut Vec<Done<'_, ThumbWork, Vec<u8>, Error>>| -> Result<bool> {
        let Writing { catalog, thumbs, unwritten, first_failed, tried, stats } = writing;
        let rules = catalog.rules();
        // The pass's own error, met on an item (section 10, D2): nothing of the batch is kept, and the pass ends with it.
        if let Some(at) = batch.iter().position(|(_, result)| matches!(result, Err(ItemError::Pass(_))))
            && let Some((_, Err(ItemError::Pass(e)))) = batch.drain(..).nth(at)
        {
            return Err(e);
        }
        // The made thumbnails' files are checked together, in one read per few hundred (`Catalog::currents`), in the
        // order of the results that are `Ok`: the loop takes one answer for each.
        let checks: Vec<(FileId, Listed)> = batch
            .iter()
            .filter(|(_, result)| result.is_ok())
            .map(|(item, _)| (item.id, Listed { size: item.size, mtime: item.source.mtime, turns: item.source.turns }))
            .collect();
        let mut current = catalog.currents(&checks)?.into_iter();
        for (item, result) in batch.drain(..) {
            *tried += 1;
            let result = match result {
                Ok(data) => Ok(data),
                Err(ItemError::File(failed)) => Err(failed),
                Err(ItemError::Pass(e)) => return Err(e),
            };
            match result {
                // Kept only if made from the file as listed: not changed since (size and mtime, as the scanner
                // decides), nor being rewritten by the app (`Catalog::is_current`). Otherwise tried again by the next
                // pass, from its change number on.
                Ok(data) => {
                    if current.next().unwrap_or(false) {
                        thumbs.push(Thumb { file_id: item.id, height: thumb, source: item.source, data });
                    } else {
                        spare.lock().unwrap_or_else(PoisonError::into_inner).push(data);
                        stats.changed += 1;
                        fold(first_failed, item.seq);
                    }
                }
                Err(e) => {
                    stats.failed += 1;
                    let failed = Failed::of(&e);
                    let retry = failed.recorded();
                    if matches!(failed, Failed::Unreadable) {
                        stats.unreadable += 1;
                    }
                    // A file tried again only once it changes is not held back: the next pass starts after it.
                    if !matches!(failed, Failed::Panicked | Failed::Undecodable) {
                        fold(first_failed, item.seq);
                    }
                    let shown = file_failure(stored_path(rules, &work.path(item.path, sep)), &e);
                    if let Some(retry) = retry {
                        let failure = LedgerFailure {
                            file_id: item.id,
                            mtime: item.source.mtime,
                            size: item.size,
                            turns: item.source.turns,
                            error: shown.error.clone(),
                            retry,
                        };
                        unwritten.failures.push((failure, item.seq));
                    }
                    keep_file_failure(&mut stats.failures, shown);
                }
            }
        }
        if unwritten.failures.len() >= BATCH || unwritten.since.elapsed() >= LEDGER_EVERY {
            unwritten.write(catalog, first_failed, &mut stats.ledger, ctl);
        }
        if thumbs.is_empty() {
            return Ok(still_wanted());
        }
        let n = thumbs.len();
        let made_known: Vec<FileId> = thumbs.iter().map(|t| t.file_id).filter(|id| known.contains(id)).collect();
        let stored = cache.put_batch(thumbs, Some(thumb)).map_err(Error::CacheStore)?;
        spare.lock().unwrap_or_else(PoisonError::into_inner).extend(thumbs.drain(..).map(|t| t.data));
        if stored {
            stats.made += n;
            if !made_known.is_empty() {
                match catalog.clear_failures(FilePass::Thumbs, &made_known) {
                    Ok(changed) => {
                        stats.ledger.written();
                        if changed {
                            ctl.ledger_changed();
                        }
                    }
                    Err(e) => stats.ledger.failed(&e),
                }
            }
        } else if cache.suspended() {
            // Turned off during the pass: not full, and the batch is not stored. Nothing it did not store is counted as done.
            stats.stopped = true;
        } else {
            stats.cache_full = true;
        }
        Ok(stored && still_wanted())
    };

    let (writing, ran) = pipeline::batched(
        pipeline::Pass { items, opts, ctl },
        |item, wide| {
            // A video's poster frame is read by FFmpeg, from its path: a part of the file, not counted as read.
            if item.poster_ms.is_some() {
                return Read::Streamed { bytes: 0 };
            }
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            Read::File
        },
        maker,
        |maker, item, bytes| {
            #[cfg(test)]
            tests::fault(item.id);
            let mut out = spare.lock().unwrap_or_else(PoisonError::into_inner).pop().unwrap_or_default();
            let made = match item.poster_ms {
                // A video: its poster frame, read by FFmpeg itself.
                Some(at) => {
                    // FFmpeg opens the file by its disk path, as in import.
                    let disk = pg_platform::names::disk_path_of(&work.path(item.path, sep));
                    let height = pg_model::num::index(thumb.get());
                    maker
                        .load_frame(&disk, at, item.source.turns, height)
                        .and_then(|()| maker.encode_frame_into(height, &mut out))
                }
                None => maker.make_into(bytes, pg_model::num::index(thumb.get()), item.source.turns, &mut out),
            };
            match made {
                Ok(()) => Ok(out),
                Err(e) => {
                    spare.lock().unwrap_or_else(PoisonError::into_inner).push(out);
                    Err(file_failed(Error::Make(e)))
                }
            }
        },
        // The failures go to the ledger at least every `LEDGER_EVERY`, so the inbox follows a long pass.
        Batch { size: BATCH, every: Some(LEDGER_EVERY) },
        writing,
        flush,
    );
    let Writing { catalog, mut unwritten, mut first_failed, tried, mut stats, .. } = writing;
    // The failures are written whatever ended the pass (an error, a stop, the cache full), before its checkpoint.
    unwritten.write(&catalog, &mut first_failed, &mut stats.ledger, ctl);
    if let Err(halt) = ran {
        // A setup or pipeline failure is a round error; the pass's own keeps its kind.
        let error = match halt {
            Halt::Pass(e) => e,
            Halt::Pipeline(failed) => Error::Pipeline(failed),
        };
        return Err(stopped(stats)(error));
    }
    drop(phase);
    stats.complete = !stats.cache_full && !stats.stopped && tried == items.len();
    let finish = pg_perf::span("thumbs.finish");
    // The checks after the pipeline are the cache's stores: an error comes back with the stats so far (D6).
    let checked = if stats.complete { cache.checked(&snapshot, first_failed) } else { Ok(()) };
    if let Err(e) = checked.and_then(|()| cache.shrink()) {
        return Err(stopped(stats)(Error::CacheStore(e)));
    }
    drop(finish);
    pass.items(u64::try_from(stats.made).unwrap_or(u64::MAX));
    pass.failed(u64::try_from(stats.failed).unwrap_or(0));
    for (step, mark) in steps.iter().zip(step_marks) {
        pass.attach_since(step, mark);
    }
    pg_catalog::attach_tx(&mut pass, tx_at);
    Ok(stats)
}
#[cfg(test)]
mod tests {

    /// A height the cache holds, for a test's literal.
    fn h(px: u32) -> ThumbHeight {
        ThumbHeight::new(px).expect("a height the cache holds")
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use pg_catalog::fixture::state;
    use pg_jobs_testkit::Quiet;
    use pg_model::ids::FileId;
    use pg_platform::file::ReadFailed;
    use pg_platform::priority::Priority;

    use super::*;

    /// The file whose thumbnail panics.
    static PANIC_ON: Mutex<Option<FileId>> = Mutex::new(None);
    /// The file whose thumbnail asks its pass to stop (before it is made, so the pass is stopped with that file's result
    /// in hand).
    static STOP_ON: Mutex<Option<(FileId, Arc<Control>)>> = Mutex::new(None);
    use pg_testkit::ONE_PASS;

    /// Asks the pass to stop when `id` is the file asked for, and panics when it is the file that panics.
    pub(super) fn fault(id: FileId) {
        if let Some((stop, ctl)) = STOP_ON.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
            && *stop == id
        {
            ctl.request_stop();
        }
        if *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) == Some(id) {
            panic!("a fault: the thumbnail of file {} panics", id.get());
        }
    }

    // A panic is counted as a failure, but doesn't hold the pass's checkpoint back: the next pass doesn't list the
    // file again (it would only panic again, and log its backtrace every time), until it changes.
    #[test]
    fn a_file_whose_thumbnail_panics_is_not_listed_again() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (dir, catalog_path, mut c) = pictures("thumbs-panic", &["a.png", "b.png"]);
        let ctl = Control::new();
        // The ledger's changes the pass announces (`Control::ledger_changed`).
        let announced = Arc::new(AtomicUsize::new(0));
        let counted = announced.clone();
        ctl.on_ledger_change(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
        c.add_root(&root).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
        let opts = Options::new(Priority::Normal);
        pg_import::import(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = Some(state::file_id(&c, "a.png").unwrap());
        let before = announced.load(Ordering::SeqCst);

        let (events, _quiet) = Quiet::bus();
        let cache = ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_model::cache::ThumbCap::from_bytes(1 << 30),
            pg_model::versions::THUMBS_VERSION,
            events,
        )
        .unwrap();
        let first = run(&cache, &catalog_path, h(64), &opts, &ctl).unwrap();
        assert_eq!((first.needed, first.made, first.failed, first.complete), (2, 1, 1, true));
        assert_eq!((first.unreadable, first.ledger.clone()), (0, LedgerOutcome::Written));
        assert_eq!(announced.load(Ordering::SeqCst), before + 1, "the ledger's write was announced");
        assert_eq!(ctl.snapshot().phase, Phase::Idle, "the phase ended");
        // The ledger holds it, not to be tried again by the pass.
        let panicked = state::file_id(&c, "a.png").unwrap();
        let [failed] = &c.failed_files(FilePass::Thumbs, 10).unwrap()[..] else { panic!("one failure") };
        assert_eq!((failed.file_id, failed.retry), (panicked, false));
        assert!(failed.error.contains("a fault"), "{}", failed.error);
        let second = run(&cache, &catalog_path, h(64), &opts, &ctl).unwrap();
        assert_eq!((second.needed, second.made, second.failed), (0, 0, 0), "the file that panicked is listed again");
        assert_eq!(second.ledger, LedgerOutcome::Untouched, "an empty pass touches no ledger");
        // The user's Retry, the fault gone: the file is made, and leaves the ledger.
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = None;
        c.retry_failures(FilePass::Thumbs).unwrap();
        let third = run(&cache, &catalog_path, h(64), &opts, &ctl).unwrap();
        assert_eq!((third.needed, third.made, third.failed), (1, 1, 0));
        assert!(c.recorded_failures(FilePass::Thumbs).unwrap().is_empty());
        // The Retry deleted the row itself, so the made file clears nothing: nothing more is announced.
        assert_eq!(third.ledger, LedgerOutcome::Untouched);
        assert_eq!(announced.load(Ordering::SeqCst), before + 1);
    }

    /// What [`imported`] makes: the run folder, the catalog's path, the open catalog, the cache, and the watch on the
    /// cache's events.
    type Imported = (std::path::PathBuf, std::path::PathBuf, Catalog, ThumbCache, Quiet<pg_thumbcache::Event>);

    /// A run folder of its own whose `images` folder holds `names` as small PNGs, and the catalog prepared beside them:
    /// the folder, the catalog's path, and the catalog.
    fn pictures(prefix: &str, names: &[&str]) -> (std::path::PathBuf, std::path::PathBuf, Catalog) {
        let dir = pg_testkit::run_folder_with("scratch_thumbs", prefix, &["images"]).unwrap();
        for name in names {
            std::fs::write(dir.join("images").join(name), pg_codec::fixtures::png_rgb(8, 8, &[128; 192]).unwrap())
                .unwrap();
        }
        let catalog_path = dir.join("catalog.sqlite");
        let c =
            Catalog::prepare(&catalog_path, pg_catalog::Versions::current(), pg_platform::native::PATH_RULES).unwrap();
        (dir, catalog_path, c)
    }

    /// A library folder in a run folder with `names` (small PNGs), scanned and imported, and a cache of `cap` bytes for
    /// its catalog.
    fn imported(prefix: &str, names: &[&str], cap: i64) -> Imported {
        let (dir, catalog_path, mut c) = pictures(prefix, names);
        let ctl = Control::new();
        let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
        c.add_root(&root).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
        pg_import::import(&mut c, &Options::new(Priority::Normal), &pg_catalog::VolumeSet::All, &ctl).unwrap();
        let (events, quiet) = Quiet::bus();
        let cache = ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_model::cache::ThumbCap::from_bytes(cap),
            pg_model::versions::THUMBS_VERSION,
            events,
        )
        .unwrap();
        (dir, catalog_path, c, cache, quiet)
    }

    // A pass stopped after its batch (`still_wanted` false, as a zoom makes it) writes no `checked` row: the files it did
    // not make are listed again, and the pass that runs through writes the row and leaves nothing listed.
    #[test]
    fn a_stopped_pass_writes_no_checked_row_and_the_next_one_makes_the_rest() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        // More files than one batch (`BATCH`), so the stop leaves some.
        let names: Vec<String> = (0..=BATCH).map(|i| format!("{i:03}.png")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (_dir, catalog_path, _c, cache, _quiet) = imported("thumbs-stop", &refs, 1 << 30);
        let opts = Options::new(Priority::Normal);
        let stopped = run_while(
            &cache,
            &catalog_path,
            h(64),
            &opts,
            &pg_catalog::VolumeSet::All,
            &|| false,
            &new_maker,
            &Control::new(),
        )
        .unwrap();
        assert!(!stopped.complete, "a stopped pass is not complete");
        assert!(stopped.made > 0 && stopped.made < stopped.needed, "one batch was made: {}", stopped.made);
        let listed = cache.pending(&catalog_path, h(64)).unwrap();
        assert!(!listed.snapshot.since_checked, "no checked row: the pass is not done");
        assert_eq!(listed.work.items.len(), stopped.needed - stopped.made, "the files not made are listed again");
        let done = run_while(
            &cache,
            &catalog_path,
            h(64),
            &opts,
            &pg_catalog::VolumeSet::All,
            &|| true,
            &new_maker,
            &Control::new(),
        )
        .unwrap();
        assert!(done.complete);
        let after = cache.pending(&catalog_path, h(64)).unwrap();
        assert!(after.snapshot.since_checked, "the complete pass wrote the checked row");
        assert!(after.work.items.is_empty(), "nothing is listed after it");
    }

    // A ledger that can't be written (its writes fail, as an aborted statement would) is `Failed`; the pass goes on,
    // and the file it would have recorded holds the pass's checkpoint back, so the next pass lists it again (here, one
    // that panics, which a recorded failure would keep out of the next pass).
    #[test]
    fn a_ledger_that_cant_be_written_fails_the_ledger_and_not_the_pass() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (_dir, catalog_path, c, cache, _quiet) = imported("thumbs-ledger-fails", &["a.png", "b.png"], 1 << 30);
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = Some(state::file_id(&c, "a.png").unwrap());
        let opts = Options::new(Priority::Normal);
        let failing = pg_catalog::fixture::LedgerWritesFail::start();
        let first = run(&cache, &catalog_path, h(64), &opts, &Control::new()).unwrap();
        drop(failing);
        assert!(matches!(first.ledger, LedgerOutcome::Failed(_)), "{:?}", first.ledger);
        assert_eq!((first.made, first.failed, first.complete), (1, 1, true), "the pass went on");
        assert!(c.failed_files(FilePass::Thumbs, 10).unwrap().is_empty(), "nothing was recorded");
        // Its change number held the checkpoint back: the next pass lists it again, and records it.
        let second = run(&cache, &catalog_path, h(64), &opts, &Control::new()).unwrap();
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = None;
        assert_eq!((second.needed, second.failed, second.ledger), (1, 1, LedgerOutcome::Written));
        assert_eq!(c.failed_files(FilePass::Thumbs, 10).unwrap().len(), 1);
    }

    // The failures are written however the pass ends: here, with the cache full at the first batch.
    #[test]
    fn the_ledger_is_written_when_the_cache_fills() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (_dir, catalog_path, c, cache, _quiet) = imported("thumbs-ledger-full", &["a.png", "b.png"], 1);
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = Some(state::file_id(&c, "a.png").unwrap());
        let first = run(&cache, &catalog_path, h(64), &Options::new(Priority::Normal), &Control::new()).unwrap();
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = None;
        assert!(first.cache_full && !first.complete);
        assert_eq!(first.ledger, LedgerOutcome::Written);
        assert_eq!(c.failed_files(FilePass::Thumbs, 10).unwrap().len(), 1);
    }

    // The failures are written when the pass is stopped too, not only when it runs to its end: a file fails, and the
    // stop is asked in its thumbnail, so the pass ends with that failure waiting for the ledger.
    #[test]
    fn the_ledger_is_written_when_the_pass_is_stopped() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (_dir, catalog_path, c, cache, _quiet) = imported("thumbs-ledger-stop", &["a.png", "b.png"], 1 << 30);
        let a = state::file_id(&c, "a.png").unwrap();
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = Some(a);
        let ctl = Arc::new(Control::new());
        *STOP_ON.lock().unwrap_or_else(PoisonError::into_inner) = Some((a, ctl.clone()));
        let first = run(&cache, &catalog_path, h(64), &Options::new(Priority::Normal), &ctl).unwrap();
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = None;
        *STOP_ON.lock().unwrap_or_else(PoisonError::into_inner) = None;
        assert_eq!(first.failed, 1, "the file that panicked was reached before the stop");
        assert_eq!(first.ledger, LedgerOutcome::Written);
        let [failed] = &c.failed_files(FilePass::Thumbs, 10).unwrap()[..] else { panic!("one failure recorded") };
        assert_eq!(failed.file_id, a);
    }

    // A pass that ends with an error (here, the cache refuses the batch it stores) writes the failures it has before
    // it returns the error, so the ledger holds the file that panicked, and the pass's checkpoint is not moved. The
    // next pass, the cache working, makes the file that was refused; the one that panicked is held by its row (not
    // tried again until it changes), so the ledger holds it once.
    #[test]
    fn a_pass_that_errors_writes_its_failures_before_it_returns_the_error() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (dir, catalog_path, c, cache, _quiet) = imported("thumbs-pass-error", &["a.png", "b.png"], 1 << 30);
        let a = state::file_id(&c, "a.png").unwrap();
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = Some(a);
        // The cache's data table refuses every thumbnail: the store's error comes back to the pass.
        pg_thumbcache::fault::refuse_stores(&dir.join("thumbs.sqlite")).unwrap();
        let opts = Options::new(Priority::Normal);
        let err = run(&cache, &catalog_path, h(64), &opts, &Control::new())
            .err()
            .expect("the pass returns the store's error");
        pg_thumbcache::fault::allow_stores(&dir.join("thumbs.sqlite")).unwrap();
        assert!(format!("{err:#}").contains("the cache refuses the thumbnail"), "{err:#}");
        // The failure was written before the error was returned, and it is the one that panicked.
        let [failed] = &c.failed_files(FilePass::Thumbs, 10).unwrap()[..] else { panic!("one failure recorded") };
        assert_eq!((failed.file_id, failed.retry), (a, false));
        assert!(failed.error.contains("a fault"), "{}", failed.error);
        // The next pass, with the cache working: the refused file is made, the one that panicked is not listed (its row
        // holds it), and the ledger still holds it once.
        let second = run(&cache, &catalog_path, h(64), &opts, &Control::new()).unwrap();
        *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = None;
        assert_eq!((second.needed, second.made, second.failed, second.complete), (1, 1, 0, true));
        assert_eq!(c.failed_files(FilePass::Thumbs, 10).unwrap().len(), 1);
    }

    // A failure's kind, by its error's type, never its text: what the ledger records, and with which retry.
    #[test]
    fn a_failure_is_classified_by_its_class() {
        use std::io::{Error as IoError, ErrorKind};

        // A file's failure, as the pass's contract makes it (D4); anything else is the pass's.
        let file = |e: Error| match file_failed(e) {
            ItemError::File(failed) => failed,
            ItemError::Pass(e) => panic!("{e} is the pass's, not the file's"),
        };
        // A picture that doesn't decode is the file's: recorded, and not tried again until it changes.
        let undecodable = file(Error::Make(MakeError::Undecodable(pg_render::Error::Invalid("decoding the JPEG"))));
        assert_eq!((Failed::of(&undecodable), Failed::of(&undecodable).recorded()), (Failed::Undecodable, Some(false)));
        let unreadable =
            file(Error::Make(MakeError::Unreadable(ReadFailed::from(IoError::new(ErrorKind::NotFound, "gone")))));
        assert_eq!((Failed::of(&unreadable), Failed::of(&unreadable).recorded()), (Failed::Unreadable, None));
        let panicked = JobFailed::panic("boom".into());
        assert_eq!((Failed::of(&panicked), Failed::of(&panicked).recorded()), (Failed::Panicked, Some(false)));
        // The pipeline's read of the file, before any decoder.
        let read = file(Error::Read {
            what: "reading a.jpg".into(),
            source: IoError::new(ErrorKind::UnexpectedEof, "cut short"),
        });
        assert_eq!((Failed::of(&read), Failed::of(&read).recorded()), (Failed::Unreadable, None));
        // No per-file failure is `Other` (D4): a catalog or a cache error is the pass's.
        assert!(matches!(file_failed(Error::Invalid("x")), ItemError::Pass(_)));
        assert!(matches!(
            file_failed(Error::CacheStore(pg_thumbcache::Error::Invalid("refused".into()))),
            ItemError::Pass(Error::CacheStore(_))
        ));
    }

    // A maker that can't start fails the pass with its own error, typed (section 10, D3 and D5): `Render`, a round error,
    // with the pass's stats beside it.
    #[test]
    fn a_maker_that_cant_start_is_the_passes_render_error() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (_dir, catalog_path, _c, cache, _quiet) = imported("thumbs-no-maker", &["a.png"], 1 << 30);
        let failing = || -> Result<Maker> { Err(Error::Render(pg_render::Error::Invalid("no maker here"))) };
        let opts = Options::new(Priority::Normal);
        let stopped = run_while(
            &cache,
            &catalog_path,
            h(64),
            &opts,
            &pg_catalog::VolumeSet::All,
            &|| true,
            &failing,
            &Control::new(),
        )
        .err()
        .expect("the pass fails");
        assert!(matches!(stopped.error, Error::Render(_)), "{stopped:?}");
        assert_eq!(stopped.stats.needed, 1, "the stats come back with the error");
    }

    // The streamed merge of the catalog's candidates and the cache's entries (section 10, D10) lists what the query over
    // the attached catalog did, in its order, with the same snapshot: before a pass, and after one (nothing left).
    #[test]
    fn the_streamed_merge_lists_what_the_attached_query_did() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (_dir, catalog_path, c, cache, _quiet) = imported("thumbs-merge", &["a.png", "b.png", "c.png"], 1 << 30);
        let ids = |p: &Pending| p.work.items.iter().map(|item| item.id).collect::<Vec<_>>();
        let marks = |p: &Pending| (p.snapshot.seq, p.snapshot.evictions, p.snapshot.since_checked);
        let same = || {
            let attached = cache.pending(&catalog_path, h(64)).unwrap();
            let merged = pending::work(&cache, &c, h(64), &pg_catalog::VolumeSet::All).unwrap();
            assert_eq!(ids(&merged), ids(&attached));
            assert_eq!(marks(&merged), marks(&attached));
            ids(&merged).len()
        };
        assert_eq!(same(), 3, "every file is owed before the pass");
        run(&cache, &catalog_path, h(64), &Options::new(Priority::Normal), &Control::new()).unwrap();
        assert_eq!(same(), 0, "nothing is owed after it");
    }

    #[test]
    fn a_ledger_outcome_keeps_its_first_failure() {
        let mut outcome = LedgerOutcome::Untouched;
        outcome.written();
        assert_eq!(outcome, LedgerOutcome::Written);
        outcome.failed(&std::io::Error::other("busy"));
        outcome.written();
        outcome.failed(&std::io::Error::other("later"));
        assert_eq!(outcome, LedgerOutcome::Failed("busy".into()));
    }

    // A file that doesn't decode is recorded, and not tried again until it changes (or Retry clears its row, as a panic
    // does); once it decodes it is made, and its row is gone. A file that can't be read (gone since the listing) is
    // counted apart, and not recorded.
    #[test]
    fn a_file_that_doesnt_decode_is_recorded_and_one_unread_is_not() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (dir, catalog_path, mut c) = pictures("thumbs-ledger", &["a.png", "b.png", "c.png"]);
        let images = dir.join("images");
        let ctl = Control::new();
        let root = pg_platform::names::normalize_path(&images).unwrap();
        c.add_root(&root).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
        let opts = Options::new(Priority::Normal);
        pg_import::import(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
        // Damaged after the import, at the same size and time (as a disk error would leave it): the catalog has it as
        // it was. Another goes.
        let a = images.join("a.png");
        let (len, mtime) = {
            let meta = std::fs::metadata(&a).unwrap();
            (meta.len(), meta.modified().unwrap())
        };
        std::fs::write(&a, vec![0u8; usize::try_from(len).unwrap()]).unwrap();
        std::fs::File::options().write(true).open(&a).unwrap().set_modified(mtime).unwrap();
        std::fs::remove_file(images.join("b.png")).unwrap();

        let (events, _quiet) = Quiet::bus();
        let cache = ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_model::cache::ThumbCap::from_bytes(1 << 30),
            pg_model::versions::THUMBS_VERSION,
            events,
        )
        .unwrap();
        let first = run(&cache, &catalog_path, h(64), &opts, &ctl).unwrap();
        assert_eq!((first.needed, first.made, first.failed, first.unreadable), (3, 1, 2, 1));
        let [failed] = &c.failed_files(FilePass::Thumbs, 10).unwrap()[..] else { panic!("one failure") };
        assert_eq!((failed.file_id, failed.retry), (state::file_id(&c, "a.png").unwrap(), false));
        // Not tried again (its row holds it); the file that could not be read is, and the recorded one is not news again.
        let summary = c.ledger_entries(false).unwrap();
        let second = run(&cache, &catalog_path, h(64), &opts, &ctl).unwrap();
        assert_eq!((second.needed, second.failed), (1, 1));
        assert_eq!(c.ledger_entries(false).unwrap(), summary);
        // Mended in place, and the user's Retry: it is made, and leaves the ledger.
        std::fs::write(&a, pg_codec::fixtures::png_rgb(8, 8, &[128; 192]).unwrap()).unwrap();
        std::fs::File::options().write(true).open(&a).unwrap().set_modified(mtime).unwrap();
        c.retry_failures(FilePass::Thumbs).unwrap();
        let third = run(&cache, &catalog_path, h(64), &opts, &ctl).unwrap();
        assert_eq!((third.made, third.unreadable), (1, 1));
        assert!(c.recorded_failures(FilePass::Thumbs).unwrap().is_empty());
    }

    // The placeholders follow the thumbnails: a file whose thumbnail is made gets its placeholder from its original (these
    // thumbnails are too small to serve one), and a file the thumbnails record as undecodable owes none (its row in the
    // Thumbs ledger holds it, as the owed query has it), so the owed count is what is left.
    #[test]
    fn placeholders_follow_the_thumbnails_and_an_undecodable_file_owes_none() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (dir, catalog_path, mut c) = pictures("thumbs-placeholders", &["a.png", "b.png"]);
        let images = dir.join("images");
        let ctl = Control::new();
        let root = pg_platform::names::normalize_path(&images).unwrap();
        c.add_root(&root).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
        let opts = Options::new(Priority::Normal);
        pg_import::import(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
        // Damaged after the import, at the same size and time: the import's analysis is what the catalog has.
        let b = images.join("b.png");
        let (len, mtime) = {
            let meta = std::fs::metadata(&b).unwrap();
            (meta.len(), meta.modified().unwrap())
        };
        std::fs::write(&b, vec![0u8; usize::try_from(len).unwrap()]).unwrap();
        std::fs::File::options().write(true).open(&b).unwrap().set_modified(mtime).unwrap();

        let (events, _quiet) = Quiet::bus();
        let cache = ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_model::cache::ThumbCap::from_bytes(1 << 30),
            pg_model::versions::THUMBS_VERSION,
            events,
        )
        .unwrap();
        assert_eq!(c.owed_placeholders().unwrap(), 2, "both are owed before the thumbnails");
        let thumbs = run(&cache, &catalog_path, h(64), &opts, &ctl).unwrap();
        assert_eq!((thumbs.made, thumbs.failed), (1, 1));
        assert_eq!(c.owed_placeholders().unwrap(), 1, "the undecodable file is held by its ledger row");
        let placed = crate::placeholders::run_while(
            &mut c,
            &cache,
            &opts,
            &pg_catalog::VolumeSet::All,
            &|| true,
            &new_maker,
            &ctl,
        )
        .unwrap();
        assert_eq!((placed.needed, placed.made, placed.from_cache, placed.failed), (1, 1, 0, 0));
        assert!(placed.complete);
        assert_eq!(c.owed_placeholders().unwrap(), 0);
        let [failed] = &c.failed_files(FilePass::Thumbs, 10).unwrap()[..] else { panic!("one failure") };
        assert_eq!((failed.file_id, failed.retry), (state::file_id(&c, "b.png").unwrap(), false));
    }
}
