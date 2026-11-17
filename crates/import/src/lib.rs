//! The import pass: every pending file is read once, in on-disk order, and analyzed from its header and container alone
//! (format, dimensions, orientation, frames, dates, rating; a video's probe). No pixel is decoded and no hash is made:
//! the duplicate hashes are the Hashes pass's (`hashes`, for the files stored owing them, `hashes_owed`), the placeholders
//! and thumbnails are the thumbnail crate's, and the tier-1 pairs are the pairing pass's (`pairs`). Results are committed in
//! batches: stopping at any point loses at most the current batch, and the next run picks up the rest from the catalog.
//!
//! A JPEG is read by its first [`PREFIX_BYTES`] when that holds its whole header (`pg_analysis::jpeg_header_complete`), so a
//! large camera file is not read whole for its facts.

pub mod hashes;
pub mod pairs;

use anyhow::{Context, Result};
use pg_analysis::analyze::Analyzer;
use pg_catalog::{Catalog, WorkItem};
use pg_model::analysis::Analysis;
use pg_model::media::Kind;

use pg_jobs::control::Control;
use pg_jobs::pipeline::{self, Batch, Options, Read};
use pg_model::progress::Phase;

const BATCH: usize = 500;

/// The first bytes of a JPEG the import reads before it knows the header is whole (256 KiB: the header of a camera
/// file, with its metadata, is in it; a larger one is read again, whole, from the start of the rest).
pub const PREFIX_BYTES: usize = 256 * 1024;

pub struct ImportStats {
    /// The files the pass had to import.
    pub pending: usize,
    pub files: usize,
    pub failed: usize,
    /// Files changed while they were read (or being rewritten by the app): nothing stored, due again.
    pub changed: usize,
}

/// What the writer has stored so far.
#[derive(Default)]
struct Counts {
    files: usize,
    failed: usize,
    changed: usize,
}

/// The pending files analyzed, and never stopped but by `ctl`.
pub fn import(catalog: &mut Catalog, opts: &Options, retry_errors: bool, ctl: &Control) -> Result<ImportStats> {
    import_while(catalog, opts, retry_errors, &|| true, ctl)
}

/// Analyzes the pending files. `still_wanted` is asked after each batch: false ends the pass there, and the rest stays
/// pending for the next round, which reads the features again.
pub fn import_while(
    catalog: &mut Catalog,
    opts: &Options,
    retry_errors: bool,
    still_wanted: &(dyn Fn() -> bool + Sync),
    ctl: &Control,
) -> Result<ImportStats> {
    let work = catalog.pending_import(retry_errors)?;
    let items = &work.items;
    let sep = catalog.rules().sep_byte();
    let mut stats = ImportStats { pending: items.len(), files: 0, failed: 0, changed: 0 };
    if items.is_empty() {
        return Ok(stats);
    }

    // The phase ends when this is dropped, however the pass leaves.
    let _phase = ctl.begin(Phase::Import, u64::try_from(items.len()).unwrap_or(u64::MAX));

    let (state, result) = pipeline::batched(
        items,
        |item, wide| {
            if item.kind == Kind::Video {
                // A video is read by FFmpeg, from its path: its size is counted as read.
                return Read::Streamed { bytes: u64::try_from(item.size).unwrap_or(0) };
            }
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            if usize::try_from(item.size).is_ok_and(|size| size > PREFIX_BYTES) && is_jpeg_name(name) {
                return Read::Prefix { len: PREFIX_BYTES, complete: pg_analysis::jpeg_header_complete };
            }
            Read::File
        },
        opts,
        ctl,
        Analyzer::new,
        |analyzer, i, bytes| {
            let item = items.get(i).context("no file to import at this index")?;
            if item.kind == Kind::Video {
                // FFmpeg opens the file by its disk path: the plain one would lose a trailing dot or space in a folder's
                // name, and open a sibling's file.
                let disk = pg_platform::names::disk_path_of(&work.path(item.path, sep));
                analyzer.video(&disk, item.rotation)
            } else {
                analyzer.analyze(bytes, item.rotation)
            }
        },
        Batch { size: BATCH, every: None },
        (catalog, Counts::default()),
        |(catalog, counts), batch| store(catalog, counts, items, batch, ctl).map(|()| still_wanted()),
    );
    result?;
    let (_, counts) = state;
    stats.files = counts.files;
    stats.failed = counts.failed;
    stats.changed = counts.changed;
    Ok(stats)
}

/// Whether a file's name is a JPEG's by extension (`.jpg`, `.jpeg`, `.jfif`, in any case): the ones read by prefix. Another
/// format under such a name is read by its prefix too, and then whole, as its header is not complete to the JPEG reader.
fn is_jpeg_name(name: &[u8]) -> bool {
    [&b".jpg"[..], b".jpeg", b".jfif"].into_iter().any(|ext| {
        name.len() >= ext.len() && name.get(name.len() - ext.len()..).is_some_and(|tail| tail.eq_ignore_ascii_case(ext))
    })
}

/// Stores a batch of analyses (or failures) in one transaction. It runs on the writer's thread (`pipeline::batched`),
/// while the workers analyze the next files.
fn store(
    catalog: &mut Catalog,
    counts: &mut Counts,
    items: &[WorkItem],
    batch: &mut Vec<(usize, Result<Analysis>)>,
    ctl: &Control,
) -> Result<()> {
    let tx = catalog.tx()?;
    let mut writer = tx.analysis_writer(pg_analysis::analyze::IMPORT_VERSION)?;
    for (i, result) in batch.drain(..) {
        let item = items.get(i).context("an import result for no file")?;
        match result {
            Ok(a) => {
                if !writer.store_analysis(item, &a)? {
                    counts.changed += 1;
                    continue;
                }
            }
            Err(e) => {
                if !writer.store_failure(item, &format!("{e:#}"))? {
                    counts.changed += 1;
                    continue;
                }
                counts.failed += 1;
            }
        }
        counts.files += 1;
    }
    drop(writer);
    tx.commit()?;
    // The import's failure ledger is kept by the catalog's triggers on the analyses, whose changes don't show in a
    // statement's count: every batch committed may have moved it, so whoever shows it reads it again (cheap, and
    // the reads coalesce).
    ctl.ledger_changed();
    Ok(())
}
