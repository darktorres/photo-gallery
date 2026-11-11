//! The background thumbnail task (PLAN §6): after the import, thumbnails for every analyzed file at the height
//! the grid currently uses, in on-disk order, until done or until the cache is full of that height. Runs at idle
//! priority, as it competes with the user for the disk and the CPU. The cache is shared with the app's on-demand
//! thumbnails; neither waits for the other but in the cache's writer, one batch at a time.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use pg_catalog::ledger::Failure as LedgerFailure;
use pg_catalog::{Catalog, Listed};
use pg_render::maker::{MakeError, Maker};
use pg_thumbcache::{Pending, Thumb, ThumbCache};

use pg_jobs::control::Control;
use pg_jobs::failure::keep_failure;
use pg_jobs::panics;
use pg_jobs::pipeline::{self, Batch, Options, Read};
use pg_model::ids::FileId;
use pg_model::notice::FileFailure;
use pg_model::notice::FilePass;
use pg_model::progress::Phase;

const BATCH: usize = 200;

/// The failures found are written to the ledger at least this often, so the inbox follows a long pass.
const LEDGER_EVERY: Duration = Duration::from_secs(2);

pub struct ThumbStats {
    /// The files that needed a thumbnail at the height when the pass began.
    pub needed: usize,
    /// The thumbnail cache's size (bytes) when the pass began.
    pub cache_bytes: u64,
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
    pub cache_full: bool,
    /// Every file of the work was tried (not stopped, the cache not full): the next pass starts from this one's
    /// snapshot (`ThumbCache::checked`).
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
    fn failed(&mut self, error: &anyhow::Error) {
        if !matches!(self, LedgerOutcome::Failed(_)) {
            *self = LedgerOutcome::Failed(format!("{error:#}"));
        }
    }
}

/// What a failed thumbnail is, for what is done about it.
#[derive(Debug, PartialEq, Eq)]
enum Failed {
    /// It panicked: recorded, and tried again only once the file changes (it would panic again, and log its backtrace
    /// at every pass), or the user retries it, or a new thumbnailer comes.
    Panicked,
    /// It doesn't decode: recorded, and tried again by the next pass (the import decoded it, so it may be a file changed
    /// since).
    Undecodable,
    /// It couldn't be read (an offline drive, a file in use): tried again by the next pass, not recorded.
    Unreadable,
    /// Anything else: tried again, not recorded.
    Other,
}

impl Failed {
    /// By the error's type, never its text or its root cause: a truncated file's decode error has an I/O error at its
    /// root, and is no read failure.
    fn of(e: &anyhow::Error) -> Failed {
        if panics::is_panic(e) {
            return Failed::Panicked;
        }
        match e.downcast_ref::<MakeError>() {
            Some(MakeError::Unreadable(_)) => Failed::Unreadable,
            Some(MakeError::Undecodable(_)) => Failed::Undecodable,
            // The pipeline's read of the file.
            None if e.downcast_ref::<std::io::Error>().is_some() => Failed::Unreadable,
            None => Failed::Other,
        }
    }

    /// Whether the ledger records it, and with which retry: `Some(true)` tried again by the next pass, `Some(false)`
    /// only once the file changes (or the user retries it); `None` not recorded (tried again, and no failure of the
    /// file's).
    fn recorded(&self) -> Option<bool> {
        match self {
            Failed::Panicked => Some(false),
            Failed::Undecodable => Some(true),
            Failed::Unreadable | Failed::Other => None,
        }
    }
}

/// Unix seconds now.
fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

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
                ledger.failed(&e.into());
            }
        }
        self.failures.clear();
    }
}

/// One pass at `height`: the files changed since the last complete one (or all of them, `ThumbCache::pending`). The
/// files whose thumbnail couldn't be made are recorded in the catalog's failure ledger (`pg_catalog::ledger`), and the
/// ledger's files whose thumbnail it makes leave it.
pub fn run(cache: &ThumbCache, catalog_path: &Path, height: u32, opts: &Options, ctl: &Control) -> Result<ThumbStats> {
    let Pending { work, snapshot } = cache.pending(catalog_path, height)?;
    let items = &work.items;
    let mut stats = ThumbStats {
        needed: items.len(),
        cache_bytes: 0,
        made: 0,
        failed: 0,
        unreadable: 0,
        failures: Vec::new(),
        changed: 0,
        cache_full: false,
        complete: false,
        ledger: LedgerOutcome::Untouched,
    };
    if items.is_empty() {
        cache.checked(&snapshot, None)?;
        stats.complete = true;
        cache.shrink()?;
        return Ok(stats);
    }
    stats.cache_bytes = u64::try_from(cache.totals()?.0)?;
    // To check each file against before its thumbnail is kept, and for the failure ledger.
    let catalog = Catalog::open(catalog_path, pg_platform::native::PATH_RULES)?;
    let rules = catalog.rules();
    let sep = rules.sep_byte();
    // The files the ledger holds: only those leave it when they are made (a clear of every file made would take the
    // catalog's write lock at every batch).
    let known: BTreeSet<FileId> = catalog.recorded_failures(FilePass::Thumbs)?;
    // The thumbnails made from the files as listed, to keep (put in the cache) with each batch.
    let mut thumbs: Vec<Thumb> = Vec::with_capacity(BATCH);
    // The thumbnails' buffers, back from the cache's writer with each batch (and from the ones not kept), for the
    // workers to make the next ones in: about a batch's worth and one per worker and slot, not one per file.
    let spare: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
    let (mut tried, mut first_failed) = (0, None);
    let mut unwritten = Unwritten { failures: Vec::new(), since: Instant::now() };
    // The phase ends when this is dropped: after the last flush, before the cache is checked and shrunk, or when the
    // pass leaves early (an error, a panic).
    let phase = ctl.begin(Phase::Thumbs { height }, u64::try_from(items.len()).unwrap_or(u64::MAX));

    // A batch of results: the thumbnails made from the file as listed are kept, the failures recorded; false once the
    // cache is full.
    let mut flush = |batch: &mut Vec<(usize, Result<Vec<u8>>)>, stats: &mut ThumbStats| -> Result<bool> {
        for (i, result) in batch.drain(..) {
            tried += 1;
            let item = items.get(i).context("a thumbnail result for no work item")?;
            match result {
                // Kept only if made from the file as listed: not changed since (size and mtime, as the scanner
                // decides), nor being rewritten by the app (`Catalog::is_current`). Otherwise tried again by the next
                // pass, from its change number on.
                Ok(data) => {
                    let listed = Listed { size: item.size, mtime: item.source.mtime, rotation: item.source.rotation };
                    if catalog.is_current(item.id, &listed)? {
                        thumbs.push(Thumb { file_id: item.id, height, source: item.source, data });
                    } else {
                        spare.lock().unwrap_or_else(PoisonError::into_inner).push(data);
                        stats.changed += 1;
                        fold(&mut first_failed, item.seq);
                    }
                }
                Err(e) => {
                    stats.failed += 1;
                    let failed = Failed::of(&e);
                    let retry = failed.recorded();
                    if matches!(failed, Failed::Unreadable) {
                        stats.unreadable += 1;
                    }
                    if !matches!(failed, Failed::Panicked) {
                        fold(&mut first_failed, item.seq);
                    }
                    let path = rules.display_stored(&work.path(item.path, sep));
                    let error = format!("{e:#}");
                    if let Some(retry) = retry {
                        let failure = LedgerFailure {
                            file_id: item.id,
                            mtime: item.source.mtime,
                            size: item.size,
                            rotation: item.source.rotation,
                            error: error.clone(),
                            retry,
                        };
                        unwritten.failures.push((failure, item.seq));
                    }
                    keep_failure(&mut stats.failures, path, error);
                }
            }
        }
        if unwritten.failures.len() >= BATCH || unwritten.since.elapsed() >= LEDGER_EVERY {
            unwritten.write(&catalog, &mut first_failed, &mut stats.ledger, ctl);
        }
        if thumbs.is_empty() {
            return Ok(true);
        }
        let n = thumbs.len();
        let made_known: Vec<FileId> = thumbs.iter().map(|t| t.file_id).filter(|id| known.contains(id)).collect();
        let stored = cache.put_batch(&mut thumbs, Some(height))?;
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
                    Err(e) => stats.ledger.failed(&e.into()),
                }
            }
        } else {
            stats.cache_full = true;
        }
        Ok(stored)
    };

    let ran = pipeline::batched(
        items,
        |item, wide| {
            // A video's poster frame is read by FFmpeg, from its path: a part of the file, not counted as read.
            if item.poster_ms.is_some() {
                return Read::Streamed { bytes: 0 };
            }
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            Read::File
        },
        opts,
        ctl,
        Maker::new,
        |maker, i, bytes| {
            let item = items.get(i).context("a thumbnail to make for no work item")?;
            #[cfg(test)]
            tests::fault(item.id);
            let mut out = spare.lock().unwrap_or_else(PoisonError::into_inner).pop().unwrap_or_default();
            let made = match item.poster_ms {
                // A video: its poster frame, read by FFmpeg itself.
                Some(at) => {
                    // FFmpeg opens the file by its disk path, as in import.
                    let disk = pg_platform::names::disk_path_of(&work.path(item.path, sep));
                    maker
                        .load_frame(&disk, at, item.source.rotation)
                        .and_then(|()| maker.encode_frame_into(pg_model::num::index(height), &mut out))
                }
                None => maker.make_into(bytes, pg_model::num::index(height), item.source.rotation, &mut out),
            };
            match made {
                Ok(()) => Ok(out),
                Err(e) => {
                    spare.lock().unwrap_or_else(PoisonError::into_inner).push(out);
                    Err(e.into())
                }
            }
        },
        // The failures go to the ledger at least every `LEDGER_EVERY`, so the inbox follows a long pass.
        Batch { size: BATCH, every: Some(LEDGER_EVERY) },
        |batch| flush(batch, &mut stats),
    );
    drop(flush);
    // The failures are written whatever ended the pass (an error, a stop, the cache full), before its checkpoint.
    unwritten.write(&catalog, &mut first_failed, &mut stats.ledger, ctl);
    ran?;
    drop(phase);
    stats.complete = !stats.cache_full && tried == items.len();
    if stats.complete {
        cache.checked(&snapshot, first_failed)?;
    }
    cache.shrink()?;
    Ok(stats)
}
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use pg_catalog::fixture::state;
    use pg_jobs_testkit::Quiet;
    use pg_model::ids::FileId;
    use pg_platform::priority::Priority;

    use super::*;

    /// The file whose thumbnail panics.
    static PANIC_ON: Mutex<Option<FileId>> = Mutex::new(None);
    /// Held by each test that runs the pass: the fault is the process's, and file ids repeat from catalog to catalog.
    static ONE_PASS: Mutex<()> = Mutex::new(());

    /// Panics when `id` is the file asked for.
    pub(super) fn fault(id: FileId) {
        if *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) == Some(id) {
            panic!("a fault: the thumbnail of file {} panics", id.get());
        }
    }

    /// A panic is counted as a failure, but doesn't hold the pass's checkpoint back: the next pass doesn't list the
    /// file again (it would only panic again, and log its backtrace every time), until it changes.
    #[test]
    fn a_file_whose_thumbnail_panics_is_not_listed_again() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = pg_testkit::run_folder_with("scratch_rate", "thumbs-panic", &["images"]).unwrap();
        for name in ["a.png", "b.png"] {
            std::fs::write(dir.join("images").join(name), pg_testkit::png_rgb(8, 8, &[128; 192]).unwrap()).unwrap();
        }
        let catalog_path = dir.join("catalog.sqlite");
        let mut c = Catalog::prepare(
            &catalog_path,
            pg_catalog::Versions {
                import: pg_analysis::analyze::IMPORT_VERSION,
                borders: pg_analysis::borders::BORDERS_VERSION,
                thumbs: 1,
            },
            pg_platform::native::PATH_RULES,
        )
        .unwrap();
        let ctl = Control::new();
        // The ledger's changes the pass announces (`Control::ledger_changed`).
        let announced = Arc::new(AtomicUsize::new(0));
        let counted = announced.clone();
        ctl.on_ledger_change(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
        let opts = Options::new(Priority::Normal);
        pg_import::import(&mut c, &opts, false, &ctl).unwrap();
        *PANIC_ON.lock().unwrap() = Some(state::file_id(&c, "a.png").unwrap());
        let before = announced.load(Ordering::SeqCst);

        let (events, _quiet) = Quiet::bus();
        let cache = ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_thumbcache::Cap::from_bytes(1 << 30),
            1,
            events,
        )
        .unwrap();
        let first = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((first.needed, first.made, first.failed, first.complete), (2, 1, 1, true));
        assert_eq!((first.unreadable, first.ledger.clone()), (0, LedgerOutcome::Written));
        assert_eq!(announced.load(Ordering::SeqCst), before + 1, "the ledger's write was announced");
        assert_eq!(ctl.snapshot().phase, Phase::Idle, "the phase ended");
        // The ledger holds it, not to be tried again by the pass.
        let panicked = state::file_id(&c, "a.png").unwrap();
        let [failed] = &c.failed_files(FilePass::Thumbs, 10).unwrap()[..] else { panic!("one failure") };
        assert_eq!((failed.file_id, failed.retry), (panicked, false));
        assert!(failed.error.contains("a fault"), "{}", failed.error);
        let second = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((second.needed, second.made, second.failed), (0, 0, 0), "the file that panicked is listed again");
        assert_eq!(second.ledger, LedgerOutcome::Untouched, "an empty pass touches no ledger");
        // The user's Retry, the fault gone: the file is made, and leaves the ledger.
        *PANIC_ON.lock().unwrap() = None;
        c.retry_failures(FilePass::Thumbs).unwrap();
        let third = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((third.needed, third.made, third.failed), (1, 1, 0));
        assert!(c.recorded_failures(FilePass::Thumbs).unwrap().is_empty());
        // The Retry deleted the row itself, so the made file clears nothing: nothing more is announced.
        assert_eq!(third.ledger, LedgerOutcome::Untouched);
        assert_eq!(announced.load(Ordering::SeqCst), before + 1);
    }

    /// What [`imported`] makes: the run folder, the catalog's path, the open catalog, the cache, and the watch on the
    /// cache's events.
    type Imported = (std::path::PathBuf, std::path::PathBuf, Catalog, ThumbCache, Quiet<pg_thumbcache::Event>);

    /// A library folder in a run folder with `names` (small PNGs), scanned and imported, and a cache of `cap` bytes for
    /// its catalog.
    fn imported(prefix: &str, names: &[&str], cap: i64) -> Imported {
        let dir = pg_testkit::run_folder_with("scratch_rate", prefix, &["images"]).unwrap();
        for name in names {
            std::fs::write(dir.join("images").join(name), pg_testkit::png_rgb(8, 8, &[128; 192]).unwrap()).unwrap();
        }
        let catalog_path = dir.join("catalog.sqlite");
        let mut c = Catalog::prepare(&catalog_path, versions(), pg_platform::native::PATH_RULES).unwrap();
        let ctl = Control::new();
        let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
        pg_import::import(&mut c, &Options::new(Priority::Normal), false, &ctl).unwrap();
        let (events, quiet) = Quiet::bus();
        let cache = ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_thumbcache::Cap::from_bytes(cap),
            1,
            events,
        )
        .unwrap();
        (dir, catalog_path, c, cache, quiet)
    }

    /// A ledger that can't be written (its writes fail, as an aborted statement would) is `Failed`; the pass goes on,
    /// and the file it would have recorded holds the pass's checkpoint back, so the next pass lists it again (here, one
    /// that panics, which a recorded failure would keep out of the next pass).
    #[test]
    fn a_ledger_that_cant_be_written_fails_the_ledger_and_not_the_pass() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (_dir, catalog_path, c, cache, _quiet) = imported("thumbs-ledger-fails", &["a.png", "b.png"], 1 << 30);
        *PANIC_ON.lock().unwrap() = Some(state::file_id(&c, "a.png").unwrap());
        let opts = Options::new(Priority::Normal);
        let failing = pg_catalog::fixture::LedgerWritesFail::start();
        let first = run(&cache, &catalog_path, 64, &opts, &Control::new()).unwrap();
        drop(failing);
        assert!(matches!(first.ledger, LedgerOutcome::Failed(_)), "{:?}", first.ledger);
        assert_eq!((first.made, first.failed, first.complete), (1, 1, true), "the pass went on");
        assert!(c.failed_files(FilePass::Thumbs, 10).unwrap().is_empty(), "nothing was recorded");
        // Its change number held the checkpoint back: the next pass lists it again, and records it.
        let second = run(&cache, &catalog_path, 64, &opts, &Control::new()).unwrap();
        *PANIC_ON.lock().unwrap() = None;
        assert_eq!((second.needed, second.failed, second.ledger), (1, 1, LedgerOutcome::Written));
        assert_eq!(c.failed_files(FilePass::Thumbs, 10).unwrap().len(), 1);
    }

    /// The failures are written however the pass ends: here, with the cache full at the first batch.
    #[test]
    fn the_ledger_is_written_when_the_cache_fills() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let (_dir, catalog_path, c, cache, _quiet) = imported("thumbs-ledger-full", &["a.png", "b.png"], 1);
        *PANIC_ON.lock().unwrap() = Some(state::file_id(&c, "a.png").unwrap());
        let first = run(&cache, &catalog_path, 64, &Options::new(Priority::Normal), &Control::new()).unwrap();
        *PANIC_ON.lock().unwrap() = None;
        assert!(first.cache_full && !first.complete);
        assert_eq!(first.ledger, LedgerOutcome::Written);
        assert_eq!(c.failed_files(FilePass::Thumbs, 10).unwrap().len(), 1);
    }

    /// A failure's kind, by its error's type, never its text: what the ledger records, and with which retry.
    #[test]
    fn a_failure_is_classified_by_its_type() {
        use std::io::{Error as IoError, ErrorKind};

        let eof = || anyhow::Error::from(IoError::new(ErrorKind::UnexpectedEof, "cut short"));
        // A truncated file: an I/O error at its root, under the decoder's context, is no read failure.
        let truncated =
            anyhow::Error::from(MakeError::Undecodable(eof().context("decoding the JPEG"))).context("a.jpg");
        assert_eq!((Failed::of(&truncated), Failed::of(&truncated).recorded()), (Failed::Undecodable, Some(true)));
        let unreadable = anyhow::Error::from(MakeError::Unreadable(IoError::new(ErrorKind::NotFound, "gone")));
        assert_eq!((Failed::of(&unreadable), Failed::of(&unreadable).recorded()), (Failed::Unreadable, None));
        let panicked = anyhow::Error::new(panics::Panicked("boom".into()));
        assert_eq!((Failed::of(&panicked), Failed::of(&panicked).recorded()), (Failed::Panicked, Some(false)));
        // The pipeline's read of the file, before any decoder.
        let read = eof().context("reading a.jpg");
        assert_eq!((Failed::of(&read), Failed::of(&read).recorded()), (Failed::Unreadable, None));
        let other = anyhow::anyhow!("something else");
        assert_eq!((Failed::of(&other), Failed::of(&other).recorded()), (Failed::Other, None));
    }

    #[test]
    fn a_ledger_outcome_keeps_its_first_failure() {
        let mut outcome = LedgerOutcome::Untouched;
        outcome.written();
        assert_eq!(outcome, LedgerOutcome::Written);
        outcome.failed(&anyhow::anyhow!("busy"));
        outcome.written();
        outcome.failed(&anyhow::anyhow!("later"));
        assert_eq!(outcome, LedgerOutcome::Failed("busy".into()));
    }

    /// A file that doesn't decode is recorded and tried again by the next pass; once it decodes (it changed) it is made,
    /// and its row is gone. A file that can't be read (gone since the listing) is counted apart, and not recorded.
    #[test]
    fn a_file_that_doesnt_decode_is_recorded_and_one_unread_is_not() {
        let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = pg_testkit::run_folder_with("scratch_rate", "thumbs-ledger", &["images"]).unwrap();
        let images = dir.join("images");
        for name in ["a.png", "b.png", "c.png"] {
            std::fs::write(images.join(name), pg_testkit::png_rgb(8, 8, &[128; 192]).unwrap()).unwrap();
        }
        let catalog_path = dir.join("catalog.sqlite");
        let mut c = Catalog::prepare(&catalog_path, versions(), pg_platform::native::PATH_RULES).unwrap();
        let ctl = Control::new();
        let root = pg_platform::names::normalize_path(&images).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
        let opts = Options::new(Priority::Normal);
        pg_import::import(&mut c, &opts, false, &ctl).unwrap();
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
            pg_thumbcache::Cap::from_bytes(1 << 30),
            1,
            events,
        )
        .unwrap();
        let first = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((first.needed, first.made, first.failed, first.unreadable), (3, 1, 2, 1));
        let [failed] = &c.failed_files(FilePass::Thumbs, 10).unwrap()[..] else { panic!("one failure") };
        assert_eq!((failed.file_id, failed.retry), (state::file_id(&c, "a.png").unwrap(), true));
        // Both are tried again, and the recorded one is not news again.
        let summary = c.ledger_entries(false).unwrap();
        let second = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((second.needed, second.failed), (2, 2));
        assert_eq!(c.ledger_entries(false).unwrap(), summary);
        // Mended in place (as a restored backup would be): it is made, and leaves the ledger.
        std::fs::write(&a, pg_testkit::png_rgb(8, 8, &[128; 192]).unwrap()).unwrap();
        std::fs::File::options().write(true).open(&a).unwrap().set_modified(mtime).unwrap();
        let third = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((third.made, third.unreadable), (1, 1));
        assert!(c.recorded_failures(FilePass::Thumbs).unwrap().is_empty());
    }

    fn versions() -> pg_catalog::Versions {
        pg_catalog::Versions {
            import: pg_analysis::analyze::IMPORT_VERSION,
            borders: pg_analysis::borders::BORDERS_VERSION,
            thumbs: 1,
        }
    }
}
