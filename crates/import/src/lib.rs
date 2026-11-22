//! The import pass: every pending file is read once, in on-disk order, and analyzed from its header and container alone
//! (format, dimensions, orientation, frames, dates, rating; a video's probe). No pixel is decoded and no hash is made:
//! the duplicate hashes are the Hashes pass's (`hashes`, for the files stored owing them, `hashes_owed`), the placeholders
//! and thumbnails are the thumbnail crate's, and the tier-1 pairs are the pairing pass's (`pairs`). Results are committed in
//! batches: stopping at any point loses at most the current batch, and the next run picks up the rest from the catalog.
//!
//! A JPEG is read up to the end of its header (`pg_container::jpeg_import_bytes`: a block, then on as its segment lengths
//! say), so a camera file is not read whole for its facts. A PNG is read at its two ends (`pg_container::png_import_bytes`):
//! the chunks before its image data and those after it, without the image data; so a large PNG is not read whole either.

#![forbid(unsafe_code)]

pub mod hashes;
pub mod pairs;

use pg_analysis::analyze::Analyzer;
use pg_catalog::{Catalog, WorkItem};
use pg_model::analysis::Analysis;
use pg_model::media::Kind;

use pg_jobs::control::Control;
use pg_jobs::failure::{Class, Failed};
use pg_jobs::pipeline::{self, Batch, Options, Read};
use pg_model::progress::Phase;

const BATCH: usize = pg_catalog::WRITE_BATCH;

/// What the import couldn't do: the analysis of a file failed, the catalog failed, the pipeline failed, or the pass's own
/// list didn't hold the item it was asked for.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The analysis of a file failed (see `pg_analysis::Error`): the file doesn't decode, or no analysis takes it.
    #[error(transparent)]
    Analysis(#[from] pg_analysis::Error),
    /// The catalog failed (see `pg_catalog::Error`).
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    /// The pass's pipeline failed (see `pg_jobs::failure::Failed`).
    #[error(transparent)]
    Pass(#[from] Failed),
    /// The duplicate join was given more pictures than it takes (see `pg_dedup::TooManyPictures`).
    #[error(transparent)]
    Dedup(#[from] pg_dedup::join::TooManyPictures),
    /// An item the pass's list doesn't hold.
    #[error("{0}")]
    Invalid(&'static str),
}

/// The result of an import pass.
pub type Result<T> = std::result::Result<T, Error>;

impl From<Error> for Failed {
    /// A file whose analysis refuses it is a data failure, stored as failed; anything else of the pass is ordinary.
    fn from(error: Error) -> Failed {
        match error {
            Error::Pass(failed) => failed,
            Error::Analysis(_) => Failed::new(Class::Data, error),
            Error::Catalog(_) | Error::Dedup(_) | Error::Invalid(_) => Failed::new(Class::Other, error),
        }
    }
}

/// The admission a JPEG takes before it is read: its header's first block (`pg_container::jpeg_import_bytes` reads 16 KiB,
/// then on to the header's end; the median header of a wallpaper set is under 1 KiB, the 99th percentile under 100). A
/// JPEG no longer than this is read whole.
const JPEG_HINT: usize = 16 << 10;

/// The bytes a PNG's gather is expected to need (its chunks before the image data, and the metadata after it): the
/// admission a PNG takes before it is read.
const PNG_HINT: usize = 64 << 10;

/// A PNG no longer than this is read whole: one read, where its gather makes two (the file's head and tail). The gather
/// is measured on larger files only.
const PNG_GATHER_FROM: usize = 256 << 10;

/// What the import pass did over the files it had to import: the counts its report states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportStats {
    /// The files the pass had to import.
    pub pending: usize,
    /// The files imported and stored.
    pub files: usize,
    /// The files that failed to import, recorded in the import ledger.
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
pub fn import(
    catalog: &mut Catalog,
    opts: &Options,
    retry_errors: bool,
    volumes: &pg_catalog::VolumeSet,
    ctl: &Control,
) -> Result<ImportStats> {
    import_while(catalog, opts, retry_errors, volumes, &|| true, ctl)
}

/// Analyzes the pending files of `volumes`. `still_wanted` is asked after each batch: false ends the pass there, and the
/// rest stays pending for the next round, which reads the features again.
pub fn import_while(
    catalog: &mut Catalog,
    opts: &Options,
    retry_errors: bool,
    volumes: &pg_catalog::VolumeSet,
    still_wanted: &(dyn Fn() -> bool + Sync),
    ctl: &Control,
) -> Result<ImportStats> {
    // The pass's span in the perf log: the pending query before its phase, the phase, and the store's share.
    let mut pass = pg_perf::span("pass.import");
    let tx_at = pg_catalog::tx_mark();
    let pending = pg_perf::span("pending.import");
    let work = catalog.pending_import(retry_errors, volumes)?;
    drop(pending);
    let stored_at = STORE.mark();
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
                return Read::Streamed { bytes: item.size };
            }
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            let size = usize::try_from(item.size).unwrap_or(usize::MAX);
            if size > JPEG_HINT && is_named(name, &[b".jpg", b".jpeg", b".jfif"]) {
                return Read::Gather { hint: JPEG_HINT, gather: |file| pg_container::jpeg_import_bytes(file) };
            }
            if size > PNG_GATHER_FROM && is_named(name, &[b".png"]) {
                return Read::Gather { hint: PNG_HINT, gather: |file| pg_container::png_import_bytes(file) };
            }
            Read::File
        },
        opts,
        ctl,
        || Analyzer::new().map_err(Error::Analysis),
        |analyzer, i, bytes| {
            let item = items.get(i).ok_or(Error::Invalid("no file to import at this index"))?;
            if item.kind == Kind::Video {
                // FFmpeg opens the file by its disk path: the plain one would lose a trailing dot or space in a folder's
                // name, and open a sibling's file.
                let disk = pg_platform::names::disk_path_of(&work.path(item.path, sep));
                analyzer.video(&disk, item.turns).map_err(Error::Analysis)
            } else {
                analyzer.analyze(bytes, item.turns).map_err(Error::Analysis)
            }
        },
        Batch { size: BATCH, every: None },
        (catalog, Counts::default()),
        |(catalog, counts), batch| {
            store(catalog, counts, items, batch, ctl).map(|()| still_wanted()).map_err(Failed::from)
        },
    );
    result?;
    let (_, counts) = state;
    stats.files = counts.files;
    stats.failed = counts.failed;
    stats.changed = counts.changed;
    pass.items(u64::try_from(stats.files).unwrap_or(u64::MAX));
    pass.failed(u64::try_from(stats.failed).unwrap_or(0));
    pass.field("pending", stats.pending);
    pass.field("changed", stats.changed);
    pass.attach_since(&STORE, stored_at);
    pg_catalog::attach_tx(&mut pass, tx_at);
    Ok(stats)
}

/// The analyses stored, batch by batch: each is one write transaction of the writer's thread.
static STORE: pg_perf::Tally = pg_perf::Tally::new("import.store");

/// Whether a file's name ends in one of `exts` (ASCII, in any case). The read is chosen by name: another format under a
/// JPEG's or a PNG's name is gathered as one too, and then read whole, as the gather finds no header of that format.
fn is_named(name: &[u8], exts: &[&[u8]]) -> bool {
    exts.iter().any(|ext| {
        name.len() >= ext.len() && name.get(name.len() - ext.len()..).is_some_and(|tail| tail.eq_ignore_ascii_case(ext))
    })
}

/// Stores a batch of analyses (or failures) in one transaction. It runs on the writer's thread (`pipeline::batched`),
/// while the workers analyze the next files.
fn store(
    catalog: &mut Catalog,
    counts: &mut Counts,
    items: &[WorkItem],
    batch: &mut Vec<(usize, std::result::Result<Analysis, Failed>)>,
    ctl: &Control,
) -> Result<()> {
    let mut store = STORE.timer();
    store.items(u64::try_from(batch.len()).unwrap_or(u64::MAX));
    let tx = catalog.tx()?;
    let mut writer = tx.analysis_writer(pg_analysis::analyze::IMPORT_VERSION)?;
    for (i, result) in batch.drain(..) {
        let item = items.get(i).ok_or(Error::Invalid("an import result for no file"))?;
        match result {
            Ok(a) => {
                if !writer.store_analysis(item, &a)? {
                    counts.changed += 1;
                    continue;
                }
            }
            Err(e) => {
                if !writer.store_failure(item, e.text())? {
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
