//! The pairing pass: the tier-1 pairs of the pictures that owe them (a `pairs_owed` row, set by the import, of a file
//! present and not pending), against every picture with stored hashes, whatever its file's state
//! (`Catalog::pairing_work`). The join (`pg_dedup::join::Joiner`) holds every picture's stored orientations,
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

use crate::Result;
use pg_catalog::{Catalog, PairingWork, PairsBatch};
use pg_dedup::join::{JoinTimes, Joined, Joiner, Pair, Tuning};
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::num::{count_u64, index};
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
/// The owed pictures per write transaction: the most whose write holds the catalog's write lock no longer than a
/// transaction of 2,000 did before the pairing plan's second round, step 7 (with its guard read row by row, ~3.6 ms;
/// one picture's statements and commit now take ~0.88 µs, so ~4,050 fit; docs/reference/PROFILING.md, "The settings
/// again"). A stop lands between two transactions, so it waits no longer either.
pub const WRITE: usize = 4_000;

/// What a pass may be tuned by: the defaults are [`SLICE`], [`CACHE_KIB`], [`WRITE`], the join's own ([`Tuning`]), a
/// load in as many parts as the join has threads, and no forced guard. The tests and the probe (`PG_PROBE_SLICE`,
/// `PG_PROBE_CACHE_KIB`, `PG_PROBE_WRITE`, `PG_PROBE_LOAD_PARTS`, `PG_PROBE_FEW`, `PG_PROBE_WIDTHS`, `PG_PROBE_BLOCK`, `PG_PROBE_GUARD`)
/// set others: a small slice runs the multi-slice path, with its write overlap, on a small library. The probes, which
/// live in pg-catalog's examples, take the cache and the write batch from `PG_PROBE_CACHE_KIB` and `PG_PROBE_WRITE`
/// (they have no default, so a measurement names the values it used).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The owed pictures per slice (at least 1).
    pub slice: usize,
    /// The page cache of the pass's connections, in KiB.
    pub cache_kib: u32,
    /// The owed pictures per write transaction (at least 1).
    pub write: usize,
    /// The parts the load reads `analysis` in (`Reader::pairing_work_split`); `None`: the join's threads.
    pub load_parts: Option<usize>,
    /// The join's few-owed threshold and head widths.
    pub join: Tuning,
    /// Every write reads its pictures' analyses (`PairsWritten::guarded`), as when the catalog changed after the load:
    /// the guard's path, tested and timed on a catalog nothing changes.
    pub guard: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            slice: SLICE,
            cache_kib: CACHE_KIB,
            write: WRITE,
            load_parts: None,
            join: Tuning::default(),
            guard: false,
        }
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
    pub legacy: u64,
    /// The analyses left out for stored hashes that don't read as this build's (`PairingWork::malformed`).
    pub malformed: usize,
    /// Of those, the files made pending, to be analysed again (`PairsWriter::repend_malformed`).
    pub repended: usize,
    /// The write transactions that read their pictures' analyses (`PairsWritten::guarded`): something the pairs depend
    /// on changed after the load.
    pub guarded: usize,
    /// Whether every owed picture was paired (false: the pass was stopped, or the catalog's pairing became another
    /// build's).
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
    /// Building the join's store (`Joiner::new_on`: every picture's hashes permuted, on the join's threads).
    pub store: Duration,
    /// The joins.
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
    // The pass's span in the perf log. Its stages are timed already (`PairStats::timings`), and are recorded from those
    // at the end, as its children.
    let mut pass = pg_perf::span("pass.pairing");
    let tx_at = pg_catalog::tx_mark();
    let paused_before = ctl.paused_total();
    let probe = pg_perf::span("pair.probe");
    let owed = catalog.has_owed_pairs()?;
    drop(probe);
    if !owed {
        return Ok(PairStats { complete: true, ..PairStats::default() });
    }
    // The catalog's cache is bounded until the pass returns, however it leaves.
    let mut catalog = catalog.bounded_cache(settings.cache_kib)?;
    let start = || priority::set_current_thread(opts.priority);
    let loading = Instant::now();
    // Read on connections of their own, `analysis` in as many parts as the join has threads, closed (their caches
    // freed) as soon as the pictures are read.
    let workers = opts.workers.get();
    let parts = settings.load_parts.unwrap_or(workers);
    let mut work = catalog.reader(settings.cache_kib)?.pairing_work_split(parts, &start)?;
    if settings.guard {
        // An epoch no catalog has: every write finds another.
        work.marks.epoch = -1;
    }
    let mut stats = PairStats { legacy: work.legacy, complete: true, ..PairStats::default() };
    stats.timings.load = loading.elapsed();
    // Another build prepared the catalog since this one did: its pairing owes the pairs, and these would not be its.
    if work.marks.version != pg_model::pdq::PAIRING_VERSION {
        stats.complete = false;
        return Ok(stats);
    }
    // The stored hashes that don't read (section 6) were left out of the load, and are counted, said once per pass (its
    // span), and analysed again: their owed rows go and their files are pending, so no pass loads them again as they are.
    stats.malformed = work.malformed.len();
    if !work.malformed.is_empty() {
        pass.field("malformed", count_u64(stats.malformed));
        let tx = catalog.tx()?;
        stats.repended = tx.pairs_writer()?.repend_malformed(&work)?;
        tx.commit()?;
    }
    let owed: Vec<usize> = work.owed.iter().enumerate().filter(|&(_, &o)| o).map(|(i, _)| i).collect();
    if owed.is_empty() {
        return Ok(stats);
    }
    let building = Instant::now();
    // The join's store is the only copy of the hashes from here on: the load's (24 MB on 187,804 pictures) goes as soon
    // as the store is made, freed on a thread of its own beside the first join (on the pass's thread it took 1.9 ms, a
    // proven 1% of a one-worker join for 300 owed), and the guard asks the join for a picture's (`Joiner::original`).
    let hashes = std::mem::take(&mut work.hashes);
    let joiner = Joiner::new_on(&hashes, &work.owed, workers, &start)?.tuned(settings.join);
    stats.timings.store = building.elapsed();

    // The phase ends when this is dropped, however the pass leaves. Its progress is the owed pictures paired so far.
    let _phase = ctl.begin(Phase::Pairing, count_u64(owed.len()));
    // The joins stop for the user's stop and pause, and for the pass's own stop (`AbortOnDrop`).
    let abort = AtomicBool::new(false);
    let control = || ctl.checkpoint_until(&abort);
    // A join's thread panicked: the pass stops (the panic resumes on it), and a thread paused in `control` is woken to
    // see the stop.
    let stop = || {
        abort.store(true, Ordering::SeqCst);
        ctl.wake();
    };
    // On a thread of its own at the pass's priority: the join's preparation (the slice's owed pictures) runs there.
    let join = |slice: &[usize]| -> (Option<Joined>, Duration) {
        start();
        let joining = Instant::now();
        let joined = match (slice.first(), slice.last()) {
            (Some(&first), Some(&last)) => joiner.join(first..last + 1, workers, &start, &control, &stop),
            _ => Some(Joined { pairs: Vec::new(), times: JoinTimes::default() }),
        };
        (joined, joining.elapsed())
    };
    let slices: Vec<&[usize]> = owed.chunks(settings.slice.max(1)).collect();
    std::thread::scope(|scope| -> Result<()> {
        // Dropped when this closure returns, before the scope joins a join still running.
        let _abort = AbortOnDrop { abort: &abort, ctl };
        scope.spawn(move || {
            start();
            drop(hashes);
        });
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
            let batches = slice.chunks(settings.write.max(1));
            if !write_slice(&mut catalog, &work, &joiner, batches, joined.pairs, ctl, &mut stats)? {
                stats.complete = false;
                break;
            }
        }
        Ok(())
    })?;
    let t = &stats.timings;
    pg_perf::record("pairing.load", t.load, &[]);
    pg_perf::record("pairing.store", t.store, &[]);
    pg_perf::record("pairing.join", t.join, &[]);
    pg_perf::record("pairing.write", t.write, &[]);
    pg_perf::record("pairing.commit", t.commit, &[]);
    pass.items(count_u64(stats.files));
    pass.field("pairs", count_u64(stats.pairs));
    pass.paused(ctl.paused_total().saturating_sub(paused_before));
    pg_catalog::attach_tx(&mut pass, tx_at);
    Ok(stats)
}

/// Writes one slice's pairs and marks its pictures done, one transaction for each of `batches` (the slice's owed
/// pictures, ascending, in runs of `Settings::write`). False when a stop came between two transactions, or the catalog's
/// pairing became another build's: what was written stays, and the rest stays owed.
fn write_slice(
    catalog: &mut Catalog,
    work: &PairingWork,
    joiner: &Joiner<'_>,
    batches: std::slice::Chunks<'_, usize>,
    mut pairs: Vec<Pair>,
    ctl: &Control,
    stats: &mut PairStats,
) -> Result<bool> {
    pairs.sort_unstable_by_key(|p| owner(p, &work.owed));
    // The stored hashes as the guard compares them (`analysis.pdq_dihedral`), from the join's store.
    let hashes = |i: usize| joiner.original(i).map(|h| pg_model::pdq::to_blob(&h));
    let mut at = 0;
    let mut owned: Vec<(usize, usize, u32)> = Vec::new();
    for owners in batches {
        if !ctl.checkpoint() {
            return Ok(false);
        }
        // The pairs the batch's owners own: the slice's pairs are sorted by owner, and its owners ascend, so they are the
        // next run. Every pair a picture owns is in its transaction, so a picture done has all its pairs written.
        let last = owners.last().copied().unwrap_or(0);
        let end = at + pairs.get(at..).map_or(0, |rest| rest.partition_point(|p| owner(p, &work.owed) <= last));
        owned.clear();
        owned.extend(pairs.get(at..end).unwrap_or(&[]).iter().map(|p| (index(p.a), index(p.b), p.distance)));
        at = end;
        let writing = Instant::now();
        let tx = catalog.tx()?;
        let Some(written) = tx.pairs_writer()?.write(work, PairsBatch { owners, pairs: &owned }, &hashes)? else {
            // The pairing version is another build's: nothing of this batch was written (the transaction rolls back).
            return Ok(false);
        };
        let committing = Instant::now();
        tx.commit()?;
        stats.timings.commit += committing.elapsed();
        stats.timings.write += committing.duration_since(writing);
        stats.pairs += written.pairs;
        stats.files += written.done;
        stats.guarded += usize::from(written.guarded);
        ctl.set_done(count_u64(stats.files));
    }
    Ok(true)
}

/// The picture that owns the pair (see the module doc).
fn owner(pair: &Pair, owed: &[bool]) -> usize {
    let b = index(pair.b);
    if owed.get(b).copied().unwrap_or(false) { b } else { index(pair.a) }
}
