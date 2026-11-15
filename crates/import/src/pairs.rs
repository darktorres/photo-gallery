//! The pairing pass: the tier-1 pairs of the pictures that owe them (a `pairs_owed` row, set by the import), against
//! every picture with stored hashes. The join (`pg_dedup::join::Joiner`) holds every picture's stored orientations,
//! built once, and finds the pairs in slices of the owed pictures; each slice is written in short transactions: a pair
//! with its guards (`PairsWriter`), then the picture marked done. A stop or a crash keeps what was written; what is
//! left stays owed, and the next pass makes it.
//!
//! Which pairs a picture owns: a pair (a, b) with `a < b` (by file id) is owned by `b` if `b` is owed, else by `a` (it is
//! owed: the pass never pairs two pictures that owe nothing). The owner's slice is the one that queries it, so each pair
//! is found and written once, and a picture is done only after every pair it owns is written.

use std::time::{Duration, Instant};

use anyhow::Result;
use pg_catalog::Catalog;
use pg_dedup::join::{Joiner, Pair};
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::num::index;
use pg_model::progress::Phase;
use pg_platform::priority;

/// The owed pictures per slice of the join: bounds a slice's pairs in memory, and makes the writes incremental.
pub const SLICE: usize = 50_000;
/// The page cache of the pass's connections, in KiB: the load's read connection, and the catalog's own while the pass
/// writes (`Catalog::bounded_cache`). The load reads `analysis` once, in order, and the writes look rows up by key, so
/// neither needs the app's 256 MB. Measured on 187,804 pictures (docs/reference/PROFILING.md, "The pairing pass"): 16,
/// 32, 64 and 256 MB gave a peak commit of 131, 150, 184 and 214 MB, the load and the writes no faster above 16.
pub const CACHE_KIB: u32 = 16 * 1024;
/// The owed pictures per write transaction.
const WRITE: usize = 2_000;

/// What a pass may be tuned by: the defaults are [`SLICE`] and [`CACHE_KIB`]. The tests and the probe
/// (`PG_PROBE_SLICE`, `PG_PROBE_CACHE_KIB`) set others: a small slice runs the multi-slice path on a small library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The owed pictures per slice (at least 1).
    pub slice: usize,
    /// The page cache of the pass's connections, in KiB.
    pub cache_kib: u32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { slice: SLICE, cache_kib: CACHE_KIB }
    }
}

/// What a pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PairStats {
    /// The owed pictures paired (their pairs stored where they are found, and marked done).
    pub files: usize,
    /// The pairs stored.
    pub pairs: usize,
    /// The analyses left out for having no stored hashes (`PairingWork::legacy`).
    pub legacy: usize,
    /// Whether every owed picture was paired (false: the pass was stopped).
    pub complete: bool,
    /// Where the pass's time went.
    pub timings: Timings,
}

/// Where a pass's time went (wall clock, on the calling thread).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timings {
    /// Reading the pictures (`Catalog::pairing_work`).
    pub load: Duration,
    /// The joins.
    pub join: Duration,
    /// The write statements, their transactions' commits left out.
    pub write: Duration,
    /// The commits of the write transactions.
    pub commit: Duration,
}

/// Pairs what is owed, with the default [`Settings`]. Nothing owed is one probe, and no hash is read.
pub fn pair(catalog: &mut Catalog, opts: &Options, ctl: &Control) -> Result<PairStats> {
    pair_with(catalog, opts, Settings::default(), ctl)
}

/// [`pair`] with `settings`.
pub fn pair_with(catalog: &mut Catalog, opts: &Options, settings: Settings, ctl: &Control) -> Result<PairStats> {
    if !catalog.has_owed_pairs()? {
        return Ok(PairStats { complete: true, ..PairStats::default() });
    }
    // The catalog's cache is bounded until the pass returns, however it leaves.
    let mut catalog = catalog.bounded_cache(settings.cache_kib)?;
    let loading = Instant::now();
    // Read on a connection of its own, closed (its cache freed) as soon as the pictures are read.
    let work = catalog.reader(settings.cache_kib)?.pairing_work()?;
    let load = loading.elapsed();
    let owed: Vec<usize> = work.owed.iter().enumerate().filter(|&(_, &o)| o).map(|(i, _)| i).collect();
    let mut stats = PairStats { legacy: work.legacy, complete: true, ..PairStats::default() };
    stats.timings.load = load;
    if owed.is_empty() {
        return Ok(stats);
    }
    let joining = Instant::now();
    let joiner = Joiner::new(&work.hashes, &work.owed)?;
    stats.timings.join += joining.elapsed();
    // The stored hashes as the guards compare them (`analysis.pdq_dihedral`).
    let blob = |i: usize| work.hashes.get(i).map(pg_model::pdq::to_blob);

    // The phase ends when this is dropped, however the pass leaves. Its progress is the owed pictures paired so far.
    let _phase = ctl.begin(Phase::Pairing, u64::try_from(owed.len()).unwrap_or(u64::MAX));
    let start = || priority::set_current_thread(opts.priority);
    let pause = || ctl.checkpoint();

    'slices: for slice in owed.chunks(settings.slice.max(1)) {
        let (Some(&first), Some(&last)) = (slice.first(), slice.last()) else { continue };
        let joining = Instant::now();
        let joined = joiner.join(first..last + 1, opts.workers, &start, &pause);
        stats.timings.join += joining.elapsed();
        let Some(mut pairs) = joined else {
            stats.complete = false;
            break;
        };
        pairs.sort_unstable_by_key(|p| owner(p, &work.owed));
        let mut at = 0;
        for batch in slice.chunks(WRITE) {
            // A stop between transactions leaves them as they are; the rest of the slice stays owed.
            if !ctl.checkpoint() {
                stats.complete = false;
                break 'slices;
            }
            let writing = Instant::now();
            let tx = catalog.tx()?;
            let mut writer = tx.pairs_writer()?;
            for &i in batch {
                while let Some(pair) = pairs.get(at).filter(|p| owner(p, &work.owed) == i) {
                    let (a, b) = (index(pair.a), index(pair.b));
                    if let (Some(&ia), Some(&ib), Some(ha), Some(hb)) =
                        (work.ids.get(a), work.ids.get(b), blob(a), blob(b))
                        && writer.pair(ia, ib, pair.distance, &ha, &hb)?
                    {
                        stats.pairs += 1;
                    }
                    at += 1;
                }
                // Every pair the picture owns is in its transaction: the picture is done, if it still holds the hashes
                // they were made from.
                if let (Some(&id), Some(hashes)) = (work.ids.get(i), blob(i))
                    && writer.done(id, &hashes)?
                {
                    stats.files += 1;
                }
            }
            drop(writer);
            let committing = Instant::now();
            tx.commit()?;
            stats.timings.commit += committing.elapsed();
            stats.timings.write += committing.duration_since(writing);
            ctl.set_done(u64::try_from(stats.files).unwrap_or(u64::MAX));
        }
    }
    Ok(stats)
}

/// The picture that owns the pair (see the module doc).
fn owner(pair: &Pair, owed: &[bool]) -> usize {
    let b = index(pair.b);
    if owed.get(b).copied().unwrap_or(false) { b } else { index(pair.a) }
}
