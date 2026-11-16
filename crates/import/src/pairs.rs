//! The pairing pass: the tier-1 pairs of the pictures that owe them (a `pairs_owed` row, set by the import), against
//! every picture with stored hashes. The join (`pg_dedup::join::Joiner`) holds every picture's stored orientations,
//! built once, and finds the pairs in slices of the owed pictures; each slice is written in short transactions: a pair
//! with its guards (`PairsWriter`), then the picture marked done. A stop or a crash keeps what was written; what is
//! left stays owed, and the next pass makes it. With several slices (a library above [`SLICE`] owed pictures), the next
//! slice's join runs on a thread of its own while this one's pairs are written; the writes stay on the calling thread,
//! which holds the catalog.
//!
//! Which pairs a picture owns: a pair (a, b) with `a < b` (by file id) is owned by `b` if `b` is owed, else by `a` (it is
//! owed: the pass never pairs two pictures that owe nothing). The owner's slice is the one that queries it, so each pair
//! is found and written once, and a picture is done only after every pair it owns is written.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use pg_catalog::{Catalog, PairingWork};
use pg_dedup::join::{JoinTimes, Joined, Joiner, Pair, Tuning};
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::num::index;
use pg_model::progress::Phase;
use pg_platform::priority;

/// The owed pictures per slice of the join: bounds a slice's query side and pairs in memory, and makes the writes
/// incremental. Every slice sorts the whole stored side again and sets up every probe again, so fewer, larger slices
/// cost less (docs/reference/PROFILING.md, "The pairing pass", gate G5).
pub const SLICE: usize = 200_000;
/// The page cache of the pass's connections, in KiB: the load's read connection, and the catalog's own while the pass
/// writes (`Catalog::bounded_cache`). The load reads `analysis` once, in order, and the writes look rows up by key, so
/// neither needs the app's 256 MB. Measured on 187,804 pictures (docs/reference/PROFILING.md, "The pairing pass"): 16,
/// 32, 64 and 256 MB gave a peak commit of 131, 150, 184 and 214 MB, the load and the writes no faster above 16.
pub const CACHE_KIB: u32 = 16 * 1024;
/// The owed pictures per write transaction.
const WRITE: usize = 2_000;

/// What a pass may be tuned by: the defaults are [`SLICE`], [`CACHE_KIB`] and the join's own ([`Tuning`]). The tests and
/// the probe (`PG_PROBE_SLICE`, `PG_PROBE_CACHE_KIB`, `PG_PROBE_SORTERS`, `PG_PROBE_FEW`) set others: a small slice runs
/// the multi-slice path, with its write overlap, on a small library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The owed pictures per slice (at least 1).
    pub slice: usize,
    /// The page cache of the pass's connections, in KiB.
    pub cache_kib: u32,
    /// The join's sorter threads and few-owed threshold.
    pub join: Tuning,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { slice: SLICE, cache_kib: CACHE_KIB, join: Tuning::default() }
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

/// Where a pass's time went (wall clock). The joins run on a thread of their own and the writes on the calling thread,
/// so with several slices the two overlap, and their sum is more than the pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timings {
    /// Reading the pictures (`Catalog::pairing_work`).
    pub load: Duration,
    /// Building the join's store, and the joins.
    pub join: Duration,
    /// The joins' parts, summed over the slices.
    pub joins: JoinTimes,
    /// The write statements, their transactions' commits left out.
    pub write: Duration,
    /// The commits of the write transactions.
    pub commit: Duration,
}

/// Pairs what is owed, with the default [`Settings`]. Nothing owed is one probe, and no hash is read.
pub fn pair(catalog: &mut Catalog, opts: &Options, ctl: &Control) -> Result<PairStats> {
    pair_with(catalog, opts, Settings::default(), ctl)
}

/// Sets the pass's own stop and wakes a paused join to see it, when the pass leaves however it leaves (the slices are
/// done, a stop, a write's error, a panic): a join still running for the next slice then ends, and the scope that runs
/// it is not held until the user resumes.
struct AbortOnDrop<'a> {
    abort: &'a AtomicBool,
    ctl: &'a Control,
}

impl Drop for AbortOnDrop<'_> {
    fn drop(&mut self) {
        self.abort.store(true, Ordering::SeqCst);
        self.ctl.wake();
    }
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
    let mut stats = PairStats { legacy: work.legacy, complete: true, ..PairStats::default() };
    stats.timings.load = loading.elapsed();
    let owed: Vec<usize> = work.owed.iter().enumerate().filter(|&(_, &o)| o).map(|(i, _)| i).collect();
    if owed.is_empty() {
        return Ok(stats);
    }
    let building = Instant::now();
    let joiner = Joiner::new(&work.hashes, &work.owed)?.tuned(settings.join);
    stats.timings.join += building.elapsed();

    // The phase ends when this is dropped, however the pass leaves. Its progress is the owed pictures paired so far.
    let _phase = ctl.begin(Phase::Pairing, u64::try_from(owed.len()).unwrap_or(u64::MAX));
    let start = || priority::set_current_thread(opts.priority);
    // The joins stop for the user's stop and pause, and for the pass's own stop (`AbortOnDrop`).
    let abort = AtomicBool::new(false);
    let control = || ctl.checkpoint_until(&abort);
    // A join's thread panicked: the pass stops (the panic resumes on it), and a thread paused in `control` is woken to
    // see the stop.
    let stop = || {
        abort.store(true, Ordering::SeqCst);
        ctl.wake();
    };
    let join = |slice: &[usize]| -> (Option<Joined>, Duration) {
        let joining = Instant::now();
        let joined = match (slice.first(), slice.last()) {
            (Some(&first), Some(&last)) => joiner.join(first..last + 1, opts.workers, &start, &control, &stop),
            _ => Some(Joined { pairs: Vec::new(), times: JoinTimes::default() }),
        };
        (joined, joining.elapsed())
    };
    let slices: Vec<&[usize]> = owed.chunks(settings.slice.max(1)).collect();
    std::thread::scope(|scope| -> Result<()> {
        // Dropped when this closure returns, before the scope joins a join still running.
        let _abort = AbortOnDrop { abort: &abort, ctl };
        let join = &join;
        let mut next = slices.first().map(|&slice| scope.spawn(move || join(slice)));
        for (k, &slice) in slices.iter().enumerate() {
            let Some(running) = next.take() else { break };
            let (joined, took) = running.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            stats.timings.join += took;
            let Some(joined) = joined else {
                stats.complete = false;
                break;
            };
            stats.timings.joins += joined.times;
            // The next slice's join runs while this one's pairs are written (a pair can come from any chunk, so no
            // picture is done before its slice's join ends).
            next = slices.get(k + 1).map(|&slice| scope.spawn(move || join(slice)));
            if !write_slice(&mut catalog, &work, slice, joined.pairs, ctl, &mut stats)? {
                stats.complete = false;
                break;
            }
        }
        Ok(())
    })?;
    Ok(stats)
}

/// Writes one slice's pairs and marks its pictures done, in transactions of [`WRITE`] pictures. False when a stop came
/// between two transactions: what was written stays, and the rest stays owed.
fn write_slice(
    catalog: &mut Catalog,
    work: &PairingWork,
    slice: &[usize],
    mut pairs: Vec<Pair>,
    ctl: &Control,
    stats: &mut PairStats,
) -> Result<bool> {
    pairs.sort_unstable_by_key(|p| owner(p, &work.owed));
    // The stored hashes as the guards compare them (`analysis.pdq_dihedral`).
    let blob = |i: usize| work.hashes.get(i).map(pg_model::pdq::to_blob);
    let mut at = 0;
    for batch in slice.chunks(WRITE) {
        if !ctl.checkpoint() {
            return Ok(false);
        }
        let writing = Instant::now();
        let tx = catalog.tx()?;
        let mut writer = tx.pairs_writer()?;
        for &i in batch {
            while let Some(pair) = pairs.get(at).filter(|p| owner(p, &work.owed) == i) {
                let (a, b) = (index(pair.a), index(pair.b));
                if let (Some(&ia), Some(&ib), Some(ha), Some(hb)) = (work.ids.get(a), work.ids.get(b), blob(a), blob(b))
                    && writer.pair(ia, ib, pair.distance, &ha, &hb)?
                {
                    stats.pairs += 1;
                }
                at += 1;
            }
            // Every pair the picture owns is in its transaction: the picture is done, if it still holds the hashes they
            // were made from.
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
    Ok(true)
}

/// The picture that owns the pair (see the module doc).
fn owner(pair: &Pair, owed: &[bool]) -> usize {
    let b = index(pair.b);
    if owed.get(b).copied().unwrap_or(false) { b } else { index(pair.a) }
}
