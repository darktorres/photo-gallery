//! The background thumbnail task (PLAN §6): after the import, thumbnails for every analyzed file at the height
//! the grid currently uses, in on-disk order, until done or until the cache is full of that height. Runs at idle
//! priority, as it competes with the user for the disk and the CPU. The cache is shared with the app's on-demand
//! thumbnails; neither waits for the other but in the cache's writer, one batch at a time.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use anyhow::{Context, Result};
use pg_catalog::Catalog;
use pg_model::rules::PathRules;
use pg_render::maker::Maker;
use pg_thumbcache::{Pending, Thumb, ThumbCache};

use pg_jobs::control::{Control, Phase};
use pg_jobs::failure::{Failure, keep_failure};
use pg_jobs::panics;
use pg_jobs::pipeline::{self, Options};

const BATCH: usize = 200;

pub struct ThumbStats {
    /// The files that needed a thumbnail at the height when the pass began.
    pub needed: usize,
    /// The thumbnail cache's size (bytes) when the pass began.
    pub cache_bytes: u64,
    pub made: usize,
    pub failed: usize,
    /// The first failures (`FAILURES_KEPT`), for whoever shows the pass.
    pub failures: Vec<Failure>,
    /// Files changed while they were read (or being rewritten by the app): not kept, tried again.
    pub changed: usize,
    pub cache_full: bool,
    /// Every file of the work was tried (not stopped, the cache not full): the next pass starts from this one's
    /// snapshot (`ThumbCache::checked`).
    pub complete: bool,
}

/// One pass at `height`: the files changed since the last complete one (or all of them, `ThumbCache::pending`).
pub fn run(cache: &ThumbCache, catalog_path: &Path, height: u32, opts: &Options, ctl: &Control) -> Result<ThumbStats> {
    let Pending { work, snapshot } = cache.pending(catalog_path, height)?;
    let items = &work.items;
    let mut stats = ThumbStats {
        needed: items.len(),
        cache_bytes: 0,
        made: 0,
        failed: 0,
        failures: Vec::new(),
        changed: 0,
        cache_full: false,
        complete: false,
    };
    if items.is_empty() {
        cache.checked(&snapshot, None)?;
        stats.complete = true;
        cache.shrink()?;
        return Ok(stats);
    }
    stats.cache_bytes = u64::try_from(cache.totals()?.0)?;
    // To check each file against before its thumbnail is kept.
    let catalog = Catalog::open(catalog_path, PathRules::NATIVE)?;
    let sep = catalog.rules().sep_byte();
    let mut batch = Vec::with_capacity(BATCH);
    // The thumbnails' buffers, back from the cache's writer with each batch (and from the ones not kept), for the
    // workers to make the next ones in: about a batch's worth and one per worker and slot, not one per file.
    let spare: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
    let (mut tried, mut first_failed) = (0, None);
    // The phase ends when this is dropped: after the last flush, before the cache is checked and shrunk, or when the
    // pass leaves early (an error, a panic).
    let phase = ctl.begin(Phase::Thumbs { height }, u64::try_from(items.len()).unwrap_or(u64::MAX));

    let flush = |batch: &mut Vec<Thumb>, stats: &mut ThumbStats| -> Result<bool> {
        let n = batch.len();
        let stored = cache.put_batch(batch, Some(height))?;
        spare.lock().unwrap_or_else(PoisonError::into_inner).extend(batch.drain(..).map(|t| t.data));
        if stored {
            stats.made += n;
        } else {
            stats.cache_full = true;
        }
        Ok(stored)
    };

    pipeline::run(
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
                    // The verbatim form, as in import: FFmpeg opens the file this path names.
                    let path = work.path(item.path, sep);
                    let verbatim = pg_platform::native::verbatim(&pg_platform::names::path_buf(&path));
                    maker
                        .load_frame(&verbatim, at, item.source.rotation)
                        .and_then(|()| maker.encode_frame_into(pg_model::num::index(height), &mut out))
                }
                None => maker.make_into(bytes, pg_model::num::index(height), item.source.rotation, &mut out),
            };
            match made {
                Ok(()) => Ok(out),
                Err(e) => {
                    spare.lock().unwrap_or_else(PoisonError::into_inner).push(out);
                    Err(e)
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
                        first_failed = Some(first_failed.map_or(item.seq, |s: i64| s.min(item.seq)));
                    }
                },
                // The import already decoded these files; a failure here is a file changed or gone since (or its
                // drive offline): the next pass tries it again. A panic (`pg_jobs::panics::Panicked`) would only
                // panic again, and write its backtrace to the log on every pass: it is counted, and the file is tried
                // again only once it changes.
                Err(e) => {
                    stats.failed += 1;
                    if !panics::is_panic(&e) {
                        first_failed = Some(first_failed.map_or(item.seq, |s: i64| s.min(item.seq)));
                    }
                    let path = String::from_utf8_lossy(&work.path(item.path, sep)).into_owned();
                    keep_failure(&mut stats.failures, path, format!("{e:#}"));
                }
            }
            if batch.len() >= BATCH {
                return flush(&mut batch, &mut stats);
            }
            Ok(true)
        },
    )?;
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
    use pg_model::ids::FileId;
    use pg_platform::priority::Priority;
    use pg_testkit::events::Quiet;

    use super::*;

    /// The file whose thumbnail panics.
    static PANIC_ON: Mutex<Option<FileId>> = Mutex::new(None);

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
        let dir = pg_testkit::run_folder_with("scratch_rate", "thumbs-panic", &["images"]).unwrap();
        for name in ["a.png", "b.png"] {
            std::fs::write(dir.join("images").join(name), pg_testkit::png_rgb(8, 8, &[128; 192]).unwrap()).unwrap();
        }
        let catalog_path = dir.join("catalog.sqlite");
        let mut c = Catalog::prepare(&catalog_path, pg_catalog::fixture::versions(), PathRules::NATIVE).unwrap();
        let ctl = Control::new();
        let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
        let opts = Options::new(Priority::Normal);
        pg_import::import(&mut c, &opts, false, &ctl).unwrap();
        *PANIC_ON.lock().unwrap() = Some(state::file_id(&c, "a.png").unwrap());

        let (events, _quiet) = Quiet::bus();
        let cache = ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap();
        let first = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((first.needed, first.made, first.failed, first.complete), (2, 1, 1, true));
        assert_eq!(ctl.snapshot().phase, Phase::Idle, "the phase ended");
        let second = run(&cache, &catalog_path, 64, &opts, &ctl).unwrap();
        assert_eq!((second.needed, second.made, second.failed), (0, 0, 0), "the file that panicked is listed again");
    }
}
