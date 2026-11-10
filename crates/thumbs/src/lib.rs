//! The background thumbnail task (PLAN §6): after the import, thumbnails for every analyzed file at the height
//! the grid currently uses, in on-disk order, until done or until the cache is full of that height. Runs at idle
//! priority, as it competes with the user for the disk and the CPU. The cache is shared with the app's on-demand
//! thumbnails; neither waits for the other but in the cache's writer, one batch at a time.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use pg_catalog::Catalog;
use pg_catalog::ledger::{self, Failure as LedgerFailure};
use pg_render::maker::{MakeError, Maker};
use pg_thumbcache::{Pending, Thumb, ThumbCache};

use pg_jobs::control::Control;
use pg_jobs::failure::{Failure, keep_failure};
use pg_jobs::panics;
use pg_jobs::pipeline::{self, Options};
use pg_model::ids::FileId;
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
    pub failures: Vec<Failure>,
    /// Files changed while they were read (or being rewritten by the app): not kept, tried again.
    pub changed: usize,
    pub cache_full: bool,
    /// Every file of the work was tried (not stopped, the cache not full): the next pass starts from this one's
    /// snapshot (`ThumbCache::checked`).
    pub complete: bool,
    /// The failure ledger couldn't be written (the first error): the files it would have held are tried again by the
    /// next pass.
    pub ledger_error: Option<String>,
}

/// What a failed thumbnail is, for what is done about it.
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
    fn write(&mut self, catalog: &Catalog, first_failed: &mut Option<i64>, error: &mut Option<String>) {
        self.since = Instant::now();
        if self.failures.is_empty() {
            return;
        }
        let failures: Vec<LedgerFailure> = self.failures.iter().map(|(f, _)| f.clone()).collect();
        if let Err(e) = ledger::record(catalog, FilePass::Thumbs, &failures, unix_now()) {
            for (_, seq) in &self.failures {
                fold(first_failed, *seq);
            }
            error.get_or_insert_with(|| format!("{e:#}"));
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
        ledger_error: None,
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
    let sep = catalog.rules().sep_byte();
    // The files the ledger holds: only those leave it when they are made (a clear of every file made would take the
    // catalog's write lock at every batch).
    let known: BTreeSet<FileId> = ledger::recorded(&catalog, FilePass::Thumbs)?;
    let mut batch = Vec::with_capacity(BATCH);
    // The thumbnails' buffers, back from the cache's writer with each batch (and from the ones not kept), for the
    // workers to make the next ones in: about a batch's worth and one per worker and slot, not one per file.
    let spare: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
    let (mut tried, mut first_failed) = (0, None);
    let mut unwritten = Unwritten { failures: Vec::new(), since: Instant::now() };
    // The phase ends when this is dropped: after the last flush, before the cache is checked and shrunk, or when the
    // pass leaves early (an error, a panic).
    let phase = ctl.begin(Phase::Thumbs { height }, u64::try_from(items.len()).unwrap_or(u64::MAX));

    let flush = |batch: &mut Vec<Thumb>, stats: &mut ThumbStats| -> Result<bool> {
        let n = batch.len();
        let made_known: Vec<FileId> = batch.iter().map(|t| t.file_id).filter(|id| known.contains(id)).collect();
        let stored = cache.put_batch(batch, Some(height))?;
        spare.lock().unwrap_or_else(PoisonError::into_inner).extend(batch.drain(..).map(|t| t.data));
        if stored {
            stats.made += n;
            if let Err(e) = ledger::clear(&catalog, FilePass::Thumbs, &made_known) {
                stats.ledger_error.get_or_insert_with(|| format!("{e:#}"));
            }
        } else {
            stats.cache_full = true;
        }
        Ok(stored)
    };

    let ran = pipeline::run(
        items,
        |item, wide| {
            // A video's poster frame is read by FFmpeg, from its path.
            let whole = item.poster_ms.is_none();
            if whole {
                let [folder, name] = work.parts(item.path);
                pg_platform::names::set_file(wide, folder, name);
            }
            whole
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
        |i, result, len| {
            ctl.add(len, result.is_err());
            tried += 1;
            let item = items.get(i).context("a thumbnail result for no work item")?;
            match result {
                // Kept only if made from the file as listed: not changed since (size and mtime, as the scanner
                // decides), nor being rewritten by the app
                // (pg_model::state::FileState). Otherwise tried again by the next pass, from its change number on.
                Ok(data) => match pg_catalog::browse::read::file_state(&catalog, item.id)? {
                    Some(s)
                        if !s.writing
                            && (s.mtime, s.size, s.rotation)
                                == (item.source.mtime, item.size, item.source.rotation) =>
                    {
                        batch.push(Thumb { file_id: item.id, height, source: item.source, data });
                    }
                    _ => {
                        spare.lock().unwrap_or_else(PoisonError::into_inner).push(data);
                        stats.changed += 1;
                        fold(&mut first_failed, item.seq);
                    }
                },
                Err(e) => {
                    stats.failed += 1;
                    let failed = Failed::of(&e);
                    let retry = match failed {
                        Failed::Panicked => Some(false),
                        Failed::Undecodable => Some(true),
                        Failed::Unreadable | Failed::Other => None,
                    };
                    if matches!(failed, Failed::Unreadable) {
                        stats.unreadable += 1;
                    }
                    if !matches!(failed, Failed::Panicked) {
                        fold(&mut first_failed, item.seq);
                    }
                    let path = String::from_utf8_lossy(&work.path(item.path, sep)).into_owned();
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
            if unwritten.failures.len() >= BATCH || unwritten.since.elapsed() >= LEDGER_EVERY {
                unwritten.write(&catalog, &mut first_failed, &mut stats.ledger_error);
            }
            if batch.len() >= BATCH {
                return flush(&mut batch, &mut stats);
            }
            Ok(true)
        },
    );
    // The failures are written whatever ended the pass (an error, a stop, the cache full), before its checkpoint.
    unwritten.write(&catalog, &mut first_failed, &mut stats.ledger_error);
    ran?;
    if !stats.cache_full {
        flush(&mut batch, &mut stats)?;
    }
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
    use std::sync::Mutex;

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
        let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
        let opts = Options::new(Priority::Normal);
        pg_import::import(&mut c, &opts, false, &ctl).unwrap();
        *PANIC_ON.lock().unwrap() = Some(state::file_id(&c, "a.png").unwrap());

        let (events, _quiet) = Quiet::bus();
        let cache = ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), 1, events).unwrap();
        let first = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((first.needed, first.made, first.failed, first.complete), (2, 1, 1, true));
        assert_eq!((first.unreadable, first.ledger_error), (0, None));
        assert_eq!(ctl.snapshot().phase, Phase::Idle, "the phase ended");
        // The ledger holds it, not to be tried again by the pass.
        let panicked = state::file_id(&c, "a.png").unwrap();
        let [failed] = &ledger::files(&c, FilePass::Thumbs, 10).unwrap()[..] else { panic!("one failure") };
        assert_eq!((failed.file_id, failed.retry), (panicked, false));
        assert!(failed.error.contains("a fault"), "{}", failed.error);
        let second = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((second.needed, second.made, second.failed), (0, 0, 0), "the file that panicked is listed again");
        // The user's Retry, the fault gone: the file is made, and leaves the ledger.
        *PANIC_ON.lock().unwrap() = None;
        ledger::retry(&c, FilePass::Thumbs).unwrap();
        let third = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((third.needed, third.made, third.failed), (1, 1, 0));
        assert!(ledger::recorded(&c, FilePass::Thumbs).unwrap().is_empty());
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
        let root = pg_scan::normalize_path(&images).unwrap();
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
        let cache = ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), 1, events).unwrap();
        let first = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((first.needed, first.made, first.failed, first.unreadable), (3, 1, 2, 1));
        let [failed] = &ledger::files(&c, FilePass::Thumbs, 10).unwrap()[..] else { panic!("one failure") };
        assert_eq!((failed.file_id, failed.retry), (state::file_id(&c, "a.png").unwrap(), true));
        // Both are tried again, and the recorded one is not news again.
        let summary = ledger::entries(&c).unwrap();
        let second = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((second.needed, second.failed), (2, 2));
        assert_eq!(ledger::entries(&c).unwrap(), summary);
        // Mended in place (as a restored backup would be): it is made, and leaves the ledger.
        std::fs::write(&a, pg_testkit::png_rgb(8, 8, &[128; 192]).unwrap()).unwrap();
        std::fs::File::options().write(true).open(&a).unwrap().set_modified(mtime).unwrap();
        let third = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((third.made, third.unreadable), (1, 1));
        assert!(ledger::recorded(&c, FilePass::Thumbs).unwrap().is_empty());
    }

    fn versions() -> pg_catalog::Versions {
        pg_catalog::Versions {
            import: pg_analysis::analyze::IMPORT_VERSION,
            borders: pg_analysis::borders::BORDERS_VERSION,
            thumbs: 1,
        }
    }
}
