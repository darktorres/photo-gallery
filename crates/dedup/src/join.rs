//! The all-pairs join over the tier-1 hashes: every two pictures with an orientation of one within [`MATCH_DISTANCE`] of
//! an orientation of the other, found by sorting the hashes' chunks and probing buckets instead of a random-access index.
//!
//! The 256-bit hash is 16 chunks of 16 bits. Two hashes within the radius differ by at most one bit in some chunk
//! (pigeonhole: 16 chunks of 2 bits hold 32 bits, and the radius is 31), so for each chunk a query's value probes its 17
//! buckets (itself and its 16 one-bit neighbours) of the stored side, sorted by that chunk. Each (stored, query) pair is
//! found in exactly one probe of a chunk; a pair that matches in several chunks is reported only in the first, which is
//! checked on the hits, not on the candidates.
//!
//! **The chunks are diagonals of the bit grid** (`permuted`). The pigeonhole argument holds for any partition of the
//! 256 bits into 16 groups of 16, and a distance is the same under any permutation of the bits. A row of the 16x16
//! coefficient grid, the obvious chunk, is skewed on real pictures: queries x stored candidates are 1.28x what uniform
//! chunks give; the wrapped diagonals (column minus row) give 1.04x. Every hash is permuted once, as it is stored, and the
//! join compares only permuted hashes.
//!
//! **Orientations.** A picture keeps orientations 0..4 ([`ORIENTATIONS`]); orientation `v + 4` is orientation `v`
//! transposed, bit for bit (`Hash::transpose`). So the 64 orientation pairs of two pictures are 32 distinct distances,
//! and a query's 8 orientations (the kept 4 and their transposes) against the stored 4 cover every one.
//!
//! **Who queries.** Between two owed pictures, the one with the higher index queries and the lower one is stored: the
//! pair is found once. A query picture is compared with a stored one only when they differ and the stored one is not
//! owed or has the lower index (`owed`). Both sides of a bucket are in picture order (the sort is stable), so the queries
//! an owed stored picture `i` may meet are a suffix of the bucket, the entries past `i`: the kernels start there, with
//! one pointer per probe that only moves forward. The output is the least distance per pair, sorted by `(a, b)`, `a < b`.
//!
//! **Layout.** The [`Joiner`] holds the permuted stored orientations of every picture (32 bytes each), built once per
//! pass; each slice's join adds the permuted transposed orientations of its query pictures. A stage (one chunk) sorts
//! both sides into three arrays per side, 20 bytes per entry: the first two words (what the kernels filter on) and the
//! code (`picture << 4 | owed << 3 | orientation`). The full hash of a candidate that passes the filter is read back from
//! the store by its code.
//!
//! **Two paths, one set of candidates.** The sorted path sorts the next stage on 1 - 2 sorter threads while the workers
//! scan this one (a ring of two stage sets; [`Tuning::sorters`]). A slice with few query entries ([`Tuning::few_entries`])
//! sorts nothing of the stored side: each chunk's query values go in a bitmap, and one pass over the store tests every
//! stored entry's values and their one-bit neighbours against them. Both test the same (stored, query, chunk) candidates
//! with the same rules, so they return the same pairs.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use pg_model::num::index;
use pg_model::pdq::{Hash, MATCH_DISTANCE, ORIENTATIONS, transpose_rows};

/// The chunks a hash is split into: 16 of 16 bits.
const CHUNKS: usize = 16;
/// The values a chunk takes.
const VALUES: usize = 1 << 16;
/// The buckets a thread takes at a time (its unit of work, and of control).
const BLOCK: usize = 128;
/// A query's orientations: the kept ones and their transposes.
const QUERY_ORIENTATIONS: usize = 2 * ORIENTATIONS;
/// The most pictures the join holds: an entry's code is the picture's index, an owed bit and an orientation (3 bits) in
/// a `u32`.
pub const MAX_PICTURES: u32 = u32::MAX >> 4;
/// The code's owed bit.
const OWED: u32 = 1 << 3;

/// A pair of pictures by their index (`a < b`), and the least distance over their orientations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pair {
    pub a: u32,
    pub b: u32,
    pub distance: u32,
}

/// More pictures than an entry's code holds ([`MAX_PICTURES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooManyPictures(pub usize);

impl std::fmt::Display for TooManyPictures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} pictures is more than the join holds ({MAX_PICTURES})", self.0)
    }
}

impl std::error::Error for TooManyPictures {}

/// The hash as the join compares it: its bits permuted so that chunk `c` (16 bits, [`chunk`]) is the wrapped diagonal
/// `column - row = c` of the 16x16 grid. Each row is rotated left by its index (bit `15 - col` goes to `15 - (col - row)`,
/// mod 16), the grid is transposed, and the rows are packed four to a word, little-endian, as [`chunk`] reads them.
pub(crate) fn permuted(h: &Hash) -> [u64; 4] {
    let mut rows = h.rows();
    for (r, row) in (0u32..).zip(rows.iter_mut()) {
        *row = row.rotate_left(r);
    }
    let diagonals = transpose_rows(rows);
    let mut words = [0u64; 4];
    for (word, four) in words.iter_mut().zip(diagonals.as_chunks::<4>().0) {
        *word = four.iter().rev().fold(0, |w, &d| (w << 16) | u64::from(d));
    }
    words
}

/// Chunk `c` of a permuted hash: 16 bits.
#[inline(always)]
fn chunk(words: &[u64; 4], c: usize) -> usize {
    let word = words.get(c >> 2).copied().unwrap_or(0);
    usize::try_from((word >> ((c & 3) * 16)) & 0xFFFF).unwrap_or(0)
}

/// The kernel that tests one stored entry against a bucket of queries. Chosen at runtime ([`best_kernel`]); each computes
/// the same pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kernel {
    Scalar,
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    Popcnt,
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    Avx2,
}

/// The fastest kernel this processor runs.
pub(crate) fn best_kernel() -> Kernel {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("popcnt") {
            return Kernel::Avx2;
        }
        if std::arch::is_x86_feature_detected!("popcnt") {
            return Kernel::Popcnt;
        }
    }
    Kernel::Scalar
}

/// The most query entries (`QUERY_ORIENTATIONS` per owed picture of a slice) the few-owed path takes by default, per
/// thread of the join ([`Tuning::few_entries`]); a larger slice takes the sorted path. Gate G6 (docs/reference/
/// PROFILING.md, "The pairing pass"): the few-owed path's work is spread over every thread, while the sorted path's
/// floor, sorting the stored side, is not, so the crossover grows with the threads: near 240 entries on one thread and
/// near 16,000 on twelve.
pub const FEW_ENTRIES_PER_THREAD: usize = 1_000;
/// The stored entries a worker of the few-owed path takes at a time (its unit of work, and of control).
const FEW_BLOCK: usize = 4096;
/// The stage sets in the sorted path's ring: the one the workers scan, and the one the sorters fill.
const RING: usize = 2;

/// What a join may be tuned by (the gates of docs/plans/PAIRING_PASS_PLAN.md set the defaults, which `None` takes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tuning {
    /// The threads that sort the next stage while the workers scan this one, out of the join's threads: 0 sorts on
    /// the workers' thread between the stages, 1 sorts both sides of a stage in turn, 2 sorts them side by side.
    /// `None`: by the thread count ([`default_sorters`]).
    pub sorters: Option<usize>,
    /// The most query entries the few-owed path takes. `None`: [`FEW_ENTRIES_PER_THREAD`] for each of the join's threads.
    pub few_entries: Option<usize>,
}

impl Tuning {
    /// The sorter threads a join on `threads` threads runs: [`Tuning::sorters`], or by the thread count, and two at
    /// most (side by side is the most there is).
    pub fn sorters_on(&self, threads: usize) -> usize {
        self.sorters.unwrap_or_else(|| default_sorters(threads)).min(2)
    }

    /// The most query entries the few-owed path takes in a join on `threads` threads: [`Tuning::few_entries`], or
    /// [`FEW_ENTRIES_PER_THREAD`] for each thread.
    pub fn few_entries_on(&self, threads: usize) -> usize {
        self.few_entries.unwrap_or_else(|| FEW_ENTRIES_PER_THREAD.saturating_mul(threads))
    }
}

/// The sorter threads of a join on `threads` threads: the workers are the rest. Gate G3 (docs/reference/PROFILING.md,
/// "The pairing pass"): below 4 threads a sorter costs a worker the scan needs more; from 4 one sorter keeps ahead of
/// the scan; from 10 the scan is fast enough that one sorter falls behind, and two side by side keep up.
pub fn default_sorters(threads: usize) -> usize {
    match threads {
        0..=3 => 0,
        4..=9 => 1,
        _ => 2,
    }
}

/// Where a join's time went (wall clock, on the thread that runs it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JoinTimes {
    /// The slice's query side: its pictures, their ranks and their transposed orientations, permuted.
    pub prepare: Duration,
    /// Waiting for the stages' sorts: all of them when the sorts run between the scans, the part the scans did not
    /// cover when sorters run ahead. The few-owed path's small sorts.
    pub sort: Duration,
    /// The scans of the stages, or the few-owed path's pass over the store.
    pub scan: Duration,
}

impl std::ops::AddAssign for JoinTimes {
    fn add_assign(&mut self, other: JoinTimes) {
        self.prepare += other.prepare;
        self.sort += other.sort;
        self.scan += other.scan;
    }
}

/// A slice's join: its pairs, and where its time went.
#[derive(Debug)]
pub struct Joined {
    /// By `(a, b)`, `a < b`, the least distance of each.
    pub pairs: Vec<Pair>,
    pub times: JoinTimes,
}

/// The pass's join: every picture's stored orientations, permuted, built once; [`Joiner::join`] then pairs one slice of
/// the queries at a time against them.
pub struct Joiner<'a> {
    /// Each picture's kept orientations, in file-id order (the pairs are by index).
    hashes: &'a [[Hash; ORIENTATIONS]],
    /// Whether each picture owes its pairs (missing entries owe nothing).
    owed: &'a [bool],
    /// Orientation `o` of picture `i`, permuted, at `ORIENTATIONS * i + o`.
    stored: Vec<[u64; 4]>,
    kernel: Kernel,
    tuning: Tuning,
}

impl<'a> Joiner<'a> {
    /// The join over `hashes` (every picture, in file-id order) where picture `i` owes its pairs when `owed[i]`.
    pub fn new(hashes: &'a [[Hash; ORIENTATIONS]], owed: &'a [bool]) -> Result<Joiner<'a>, TooManyPictures> {
        if !u32::try_from(hashes.len()).is_ok_and(|n| n <= MAX_PICTURES) {
            return Err(TooManyPictures(hashes.len()));
        }
        let stored = hashes.iter().flat_map(|h| h.iter().map(permuted)).collect();
        Ok(Joiner { hashes, owed, stored, kernel: best_kernel(), tuning: Tuning::default() })
    }

    /// The same join with `tuning`.
    #[must_use]
    pub fn tuned(mut self, tuning: Tuning) -> Joiner<'a> {
        self.tuning = tuning;
        self
    }

    /// The same join on `kernel` (the tests run each).
    #[cfg(test)]
    pub(crate) fn with_kernel(mut self, kernel: Kernel) -> Joiner<'a> {
        self.kernel = kernel;
        self
    }

    fn owes(&self, i: usize) -> bool {
        self.owed.get(i).copied().unwrap_or(false)
    }

    /// The pairs in which the query side is one of `queries` (a range of indices; the owed ones in it query).
    ///
    /// `threads` threads in all (at least one): the sorters ([`Tuning::sorters`]) and the workers. `start` runs once in
    /// each thread before it works (the caller sets its priority there); `control` is asked before each block of work,
    /// by the workers and the sorters, returns false to stop, and may block (a pause). `None` when it stopped the join:
    /// nothing is returned then. Not inlined, so a profiler can toggle on it.
    #[inline(never)]
    pub fn join(
        &self,
        queries: Range<usize>,
        threads: usize,
        start: &(dyn Fn() + Sync),
        control: &(dyn Fn() -> bool + Sync),
    ) -> Option<Joined> {
        let threads = threads.max(1);
        let preparing = Instant::now();
        let pictures: Vec<u32> = queries.filter(|&i| self.owes(i)).filter_map(|i| u32::try_from(i).ok()).collect();
        let mut rank = vec![0u32; self.hashes.len()];
        for (k, &p) in (0u32..).zip(&pictures) {
            if let Some(r) = rank.get_mut(index(p)) {
                *r = k;
            }
        }
        let transposed: Vec<[u64; 4]> = pictures
            .iter()
            .flat_map(|&p| self.hashes.get(index(p)).into_iter().flatten().map(|h| permuted(&h.transpose())))
            .collect();
        let sides = Sides { joiner: self, pictures: &pictures, transposed: &transposed, rank: &rank };
        let mut times = JoinTimes { prepare: preparing.elapsed(), ..JoinTimes::default() };
        let queries = pictures.len() * QUERY_ORIENTATIONS;
        let few = self.tuning.few_entries_on(threads);
        let mut pairs = if queries <= few {
            self.few_owed(&sides, queries, threads, start, control, &mut times)?
        } else {
            self.sorted(&sides, queries, threads, start, control, &mut times)?
        };
        compact(&mut pairs);
        Some(Joined { pairs, times })
    }

    /// The sorted path: per chunk, both sides sorted into buckets and the buckets probed. The sorts of a stage run on
    /// the sorter threads while the workers scan the one before (a ring of [`RING`] stage sets), or between the scans
    /// with no sorter.
    fn sorted(
        &self,
        sides: &Sides<'_, '_>,
        queries: usize,
        threads: usize,
        start: &(dyn Fn() + Sync),
        control: &(dyn Fn() -> bool + Sync),
        times: &mut JoinTimes,
    ) -> Option<Vec<Pair>> {
        let sorters = self.tuning.sorters_on(threads);
        let workers = threads.saturating_sub(sorters).max(1);
        let stored = self.stored.len();
        let mut found = Vec::new();
        if sorters == 0 {
            let (mut set, mut scratch) = (StageSet::default(), Scratch::default());
            for c in 0..CHUNKS {
                if !control() {
                    return None;
                }
                let sorting = Instant::now();
                set.sort(c, sides, stored, queries, &mut scratch, false, start);
                times.sort += sorting.elapsed();
                let scanning = Instant::now();
                let stage = Stage { c, stored: &set.stored, query: &set.query, sides };
                found.append(&mut run_stage(self.kernel, &stage, workers, start, control)?);
                times.scan += scanning.elapsed();
            }
            return Some(found);
        }
        let stopped = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let (full_tx, full_rx) = std::sync::mpsc::sync_channel::<(usize, StageSet)>(RING);
            let (free_tx, free_rx) = std::sync::mpsc::sync_channel::<StageSet>(RING);
            for _ in 0..RING {
                let _ = free_tx.send(StageSet::default());
            }
            let stopped = &stopped;
            let sorter = scope.spawn(move || {
                start();
                let mut scratch = Scratch::default();
                for c in 0..CHUNKS {
                    // The workers dropped their end: they stopped, or are done.
                    let Ok(mut set) = free_rx.recv() else { return };
                    if stopped.load(Relaxed) || !control() {
                        stopped.store(true, Relaxed);
                        return;
                    }
                    set.sort(c, sides, stored, queries, &mut scratch, sorters == 2, start);
                    // Never blocks: the channel holds every set there is.
                    if full_tx.send((c, set)).is_err() {
                        return;
                    }
                }
            });
            for _ in 0..CHUNKS {
                let waiting = Instant::now();
                // The sorter stopped (a stop, or its panic, which the join below resumes).
                let Ok((c, set)) = full_rx.recv() else { break };
                times.sort += waiting.elapsed();
                let scanning = Instant::now();
                let stage = Stage { c, stored: &set.stored, query: &set.query, sides };
                let scanned = run_stage(self.kernel, &stage, workers, start, control);
                times.scan += scanning.elapsed();
                let Some(mut pairs) = scanned else {
                    stopped.store(true, Relaxed);
                    break;
                };
                found.append(&mut pairs);
                let _ = free_tx.send(set);
            }
            // Releases a sorter waiting for a set.
            drop(free_tx);
            if let Err(panic) = sorter.join() {
                std::panic::resume_unwind(panic);
            }
        });
        (!stopped.load(Relaxed)).then_some(found)
    }

    /// The few-owed path: the query side is small, so the stored side is not sorted. Each chunk's query entries are
    /// sorted as a stage's are ([`Sorted`]), with a bitmap of the values they take, and one pass over the store tests
    /// each stored entry's value in every chunk, and its one-bit neighbours, against the bitmaps. A hit is the same
    /// candidate the sorted path's probe finds, tested by the same rules ([`few_hits`]), so the pairs are the same.
    fn few_owed(
        &self,
        sides: &Sides<'_, '_>,
        queries: usize,
        threads: usize,
        start: &(dyn Fn() + Sync),
        control: &(dyn Fn() -> bool + Sync),
        times: &mut JoinTimes,
    ) -> Option<Vec<Pair>> {
        let sorting = Instant::now();
        let mut scratch = Vec::new();
        let chunks: Vec<FewChunk> = (0..CHUNKS).map(|c| FewChunk::new(sides, queries, c, &mut scratch)).collect();
        drop(scratch);
        times.sort += sorting.elapsed();
        let scanning = Instant::now();
        let blocks = self.stored.len().div_ceil(FEW_BLOCK);
        let found = parallel(threads, blocks, start, control, |block, out| {
            let lo = block * FEW_BLOCK;
            for x in lo..(lo + FEW_BLOCK).min(self.stored.len()) {
                few_entry(sides, &chunks, x, out);
            }
        });
        times.scan += scanning.elapsed();
        found
    }
}

/// One chunk's query entries for the few-owed path: sorted by that chunk as a stage's are, and the values they take as a
/// bitmap of the 65,536 (8 KB, where a stage's offsets are 256 KB): the pass over the store tests the bitmap, and looks
/// at the offsets only on a hit.
struct FewChunk {
    bits: Vec<u64>,
    sorted: Sorted,
}

/// For a chunk value's low six bits `l`: the bits of its bitmap word that are the value itself and its six neighbours
/// in the low six bits (`l ^ 1 << k`, `k < 6`), which lie in the same word. The other ten neighbours are in other words.
#[expect(clippy::indexing_slicing, reason = "built at compile time, `l` below the array's 64")]
const NEAR: [u64; 64] = {
    let mut near = [0u64; 64];
    let mut l = 0;
    while l < 64 {
        let mut mask = 1u64 << l;
        let mut k = 0;
        while k < 6 {
            mask |= 1u64 << (l ^ (1 << k));
            k += 1;
        }
        near[l] = mask;
        l += 1;
    }
    near
};

impl FewChunk {
    fn new(sides: &Sides<'_, '_>, queries: usize, c: usize, scratch: &mut Vec<Rec>) -> FewChunk {
        let mut sorted = Sorted::default();
        sort_into(&mut sorted, scratch, queries, |e| sides.query_entry(e), c);
        let mut bits = vec![0u64; VALUES / 64];
        for (v, pair) in sorted.offsets.windows(2).enumerate() {
            if let ([lo, hi], Some(word)) = (pair, bits.get_mut(v >> 6))
                && hi > lo
            {
                *word |= 1 << (v & 63);
            }
        }
        FewChunk { bits, sorted }
    }

    /// The bitmap's word of value `v`.
    #[inline(always)]
    fn word(&self, v: usize) -> u64 {
        self.bits.get(v >> 6).copied().unwrap_or(0)
    }
}

/// The few-owed path's work for stored entry `x`: in each chunk, its value and the value's 16 one-bit neighbours tested
/// against that chunk's bitmap (the value and its six low-bit neighbours with one word, [`NEAR`]), and the query entries
/// of each value that is set tested in full.
#[inline(always)]
fn few_entry(sides: &Sides<'_, '_>, chunks: &[FewChunk], x: usize, out: &mut Vec<Pair>) {
    let (xw, xc) = sides.stored_entry(x);
    for (c, few) in chunks.iter().enumerate() {
        let v = chunk(&xw, c);
        let low = v & 63;
        let word = few.word(v);
        if word & NEAR.get(low).copied().unwrap_or(0) != 0 {
            for p in 0..7 {
                let u = probe(v, p);
                if (word >> (u & 63)) & 1 != 0 {
                    few_hits(sides, few, &xw, xc, u, c, out);
                }
            }
        }
        for k in 6..16 {
            let u = v ^ (1 << k);
            if (few.word(u) >> low) & 1 != 0 {
                few_hits(sides, few, &xw, xc, u, c, out);
            }
        }
    }
}

/// Stored entry `xw` (code `xc`) against the query entries of value `u` in chunk `c`: as the sorted path's [`consider`]
/// does, the ordering rule, then the first two words' distance, then the full one and the first chunk that matches.
#[inline(always)]
fn few_hits(sides: &Sides<'_, '_>, few: &FewChunk, xw: &[u64; 4], xc: u32, u: usize, c: usize, out: &mut Vec<Pair>) {
    let q = &few.sorted;
    let (lo, hi) = q.bucket(u);
    for y in lo..hi {
        let (Some(&yc), Some(&y0), Some(&y1)) = (q.code.get(y), q.head0.get(y), q.head1.get(y)) else { return };
        if !owed(xc, yc) {
            continue;
        }
        let head = (xw[0] ^ y0).count_ones() + (xw[1] ^ y1).count_ones();
        if head > MATCH_DISTANCE {
            continue;
        }
        let yw = sides.words(yc);
        let distance = head + (xw[2] ^ yw[2]).count_ones() + (xw[3] ^ yw[3]).count_ones();
        if distance <= MATCH_DISTANCE && !matched_earlier(xw, &yw, c) {
            let (xp, yp) = (xc >> 4, yc >> 4);
            out.push(Pair { a: xp.min(yp), b: xp.max(yp), distance });
        }
    }
}

/// One stage's two sorted sides.
#[derive(Default)]
struct StageSet {
    stored: Sorted,
    query: Sorted,
}

/// The radix sort's scratch of each side, reused from stage to stage.
#[derive(Default)]
struct Scratch {
    stored: Vec<Rec>,
    query: Vec<Rec>,
}

impl StageSet {
    /// Both sides sorted by chunk `c`: the `stored` entries and the `queries` entries; with `side_by_side`, the stored
    /// side on a thread of its own (which runs `start` first) while this one sorts the queries.
    #[expect(clippy::too_many_arguments, reason = "one stage's sort: its sides, sizes, scratch and how to run it")]
    fn sort(
        &mut self,
        c: usize,
        sides: &Sides<'_, '_>,
        stored: usize,
        queries: usize,
        scratch: &mut Scratch,
        side_by_side: bool,
        start: &(dyn Fn() + Sync),
    ) {
        let StageSet { stored: stored_out, query: query_out } = self;
        let Scratch { stored: stored_scratch, query: query_scratch } = scratch;
        if side_by_side {
            std::thread::scope(|scope| {
                let other = scope.spawn(|| {
                    start();
                    sort_into(stored_out, stored_scratch, stored, |e| sides.stored_entry(e), c);
                });
                sort_into(query_out, query_scratch, queries, |e| sides.query_entry(e), c);
                if let Err(panic) = other.join() {
                    std::panic::resume_unwind(panic);
                }
            });
        } else {
            sort_into(stored_out, stored_scratch, stored, |e| sides.stored_entry(e), c);
            sort_into(query_out, query_scratch, queries, |e| sides.query_entry(e), c);
        }
    }
}

/// What one slice's join reads the entries' hashes from: the stored orientations of every picture, and the transposed
/// orientations of the slice's query pictures (`pictures`, in order; `rank[p]` is picture `p`'s place among them).
struct Sides<'a, 'j> {
    joiner: &'a Joiner<'j>,
    pictures: &'a [u32],
    transposed: &'a [[u64; 4]],
    rank: &'a [u32],
}

impl Sides<'_, '_> {
    /// Stored entry `e`: orientation `e % 4` of picture `e / 4`, with its code.
    #[inline(always)]
    fn stored_entry(&self, e: usize) -> ([u64; 4], u32) {
        let (picture, o) = (e / ORIENTATIONS, e % ORIENTATIONS);
        let owed = if self.joiner.owes(picture) { OWED } else { 0 };
        let code = (u32::try_from(picture).unwrap_or(0) << 4) | owed | u32::try_from(o).unwrap_or(0);
        (self.joiner.stored.get(e).copied().unwrap_or_default(), code)
    }

    /// Query entry `e`: orientation `e % 8` of the slice's query picture `e / 8`, with its code (a query is always owed).
    #[inline(always)]
    fn query_entry(&self, e: usize) -> ([u64; 4], u32) {
        let (k, o) = (e / QUERY_ORIENTATIONS, e % QUERY_ORIENTATIONS);
        let picture = self.pictures.get(k).copied().unwrap_or(0);
        let words = if o < ORIENTATIONS {
            self.joiner.stored.get(index(picture) * ORIENTATIONS + o)
        } else {
            self.transposed.get(k * ORIENTATIONS + o - ORIENTATIONS)
        };
        (words.copied().unwrap_or_default(), (picture << 4) | OWED | u32::try_from(o).unwrap_or(0))
    }

    /// The full permuted hash of the entry with `code`.
    #[inline(always)]
    fn words(&self, code: u32) -> [u64; 4] {
        let (picture, o) = (index(code >> 4), index(code & 7));
        let words = if o < ORIENTATIONS {
            self.joiner.stored.get(picture * ORIENTATIONS + o)
        } else {
            let k = index(self.rank.get(picture).copied().unwrap_or(0));
            self.transposed.get(k * ORIENTATIONS + o - ORIENTATIONS)
        };
        words.copied().unwrap_or_default()
    }
}

/// Entries sorted by one chunk, in three arrays: entry `j` has the first two words `head0[j]`, `head1[j]` (with four
/// zeros of padding, for the vector kernel's loads) and `code[j]`; `offsets[v]..offsets[v + 1]` are the entries whose
/// chunk is `v`, in the order the entries were given (the sort is stable).
#[derive(Default)]
struct Sorted {
    offsets: Vec<u32>,
    head0: Vec<u64>,
    head1: Vec<u64>,
    code: Vec<u32>,
}

impl Sorted {
    #[inline(always)]
    fn bucket(&self, v: usize) -> (usize, usize) {
        let at = |v: usize| index(self.offsets.get(v).copied().unwrap_or(0));
        (at(v), at(v + 1))
    }
}

/// An entry between the radix sort's two passes.
#[derive(Clone, Copy, Default)]
struct Rec {
    h0: u64,
    h1: u64,
    code: u32,
    key: u16,
}

/// The `n` entries `entry(0..n)` sorted by their chunk `c` into `out`, stably: a two-pass LSD radix sort (the low byte
/// into `scratch`, then the high byte into `out`'s arrays), with `out` and `scratch` reused from stage to stage. The
/// full chunk's counts give the offsets.
#[expect(
    clippy::indexing_slicing,
    reason = "a chunk is below VALUES (the offsets' length less one) and a byte below 256; each position is a running \
              sum of the counts, below the n entries the arrays are sized to"
)]
fn sort_into(out: &mut Sorted, scratch: &mut Vec<Rec>, n: usize, entry: impl Fn(usize) -> ([u64; 4], u32), c: usize) {
    let (mut lo, mut hi) = ([0u32; 256], [0u32; 256]);
    out.offsets.clear();
    out.offsets.resize(VALUES + 1, 0);
    for e in 0..n {
        let key = chunk(&entry(e).0, c);
        lo[key & 0xFF] += 1;
        hi[key >> 8] += 1;
        out.offsets[key + 1] += 1;
    }
    let (mut at_lo, mut at_hi) = (0u32, 0u32);
    for (l, h) in lo.iter_mut().zip(hi.iter_mut()) {
        (*l, at_lo) = (at_lo, at_lo + *l);
        (*h, at_hi) = (at_hi, at_hi + *h);
    }
    for v in 0..VALUES {
        out.offsets[v + 1] += out.offsets[v];
    }
    scratch.clear();
    scratch.resize(n, Rec::default());
    for e in 0..n {
        let (words, code) = entry(e);
        let key = chunk(&words, c);
        let slot = &mut lo[key & 0xFF];
        scratch[index(*slot)] = Rec { h0: words[0], h1: words[1], code, key: u16::try_from(key).unwrap_or(0) };
        *slot += 1;
    }
    out.head0.clear();
    out.head0.resize(n + 4, 0);
    out.head1.clear();
    out.head1.resize(n + 4, 0);
    out.code.clear();
    out.code.resize(n, 0);
    for r in scratch.iter() {
        let slot = &mut hi[usize::from(r.key >> 8)];
        let at = index(*slot);
        out.head0[at] = r.h0;
        out.head1[at] = r.h1;
        out.code[at] = r.code;
        *slot += 1;
    }
}

/// Whether the pair is also a candidate in an earlier chunk, so it is reported there (the first chunk that matches).
#[inline(always)]
fn matched_earlier(x: &[u64; 4], y: &[u64; 4], c: usize) -> bool {
    (0..c).any(|e| (chunk(x, e) ^ chunk(y, e)).count_ones() <= 1)
}

/// Whether the stored entry `x` is compared with the query entry `y` (codes): they are of different pictures, and the
/// stored one owes nothing or comes first (a pair of two owed pictures is found by the later one's query).
#[inline(always)]
fn owed(x: u32, y: u32) -> bool {
    let (xp, yp) = (x >> 4, y >> 4);
    xp != yp && (x & OWED == 0 || xp < yp)
}

/// One chunk's stage: the stored and query sides sorted by it, and where the full hashes are.
struct Stage<'a, 's, 'j> {
    c: usize,
    stored: &'a Sorted,
    query: &'a Sorted,
    sides: &'a Sides<'s, 'j>,
}

/// Tests stored entry `x` against query entry `y` (positions in their sorted sides), and records the pair if it is owed,
/// within the radius and first found in this chunk.
#[inline(always)]
fn consider(st: &Stage<'_, '_, '_>, x: usize, y: usize, out: &mut Vec<Pair>) {
    let (Some(&xc), Some(&yc)) = (st.stored.code.get(x), st.query.code.get(y)) else { return };
    if !owed(xc, yc) {
        return;
    }
    let word =
        |side: &Sorted, j: usize| (side.head0.get(j).copied().unwrap_or(0), side.head1.get(j).copied().unwrap_or(0));
    let ((x0, x1), (y0, y1)) = (word(st.stored, x), word(st.query, y));
    let head = (x0 ^ y0).count_ones() + (x1 ^ y1).count_ones();
    if head > MATCH_DISTANCE {
        return;
    }
    let (xw, yw) = (st.sides.words(xc), st.sides.words(yc));
    let distance = head + (xw[2] ^ yw[2]).count_ones() + (xw[3] ^ yw[3]).count_ones();
    if distance <= MATCH_DISTANCE && !matched_earlier(&xw, &yw, st.c) {
        let (xp, yp) = (xc >> 4, yc >> 4);
        out.push(Pair { a: xp.min(yp), b: xp.max(yp), distance });
    }
}

/// Where stored entry `x`'s queries start in the query bucket `[lo, hi)`: the whole bucket when it owes nothing, else
/// past the entries of pictures up to its own (`from`, the probe's pointer, moves forward only: the stored entries come
/// in picture order).
#[inline(always)]
fn suffix(st: &Stage<'_, '_, '_>, x: usize, lo: usize, hi: usize, from: &mut usize) -> usize {
    let xc = st.stored.code.get(x).copied().unwrap_or(0);
    if xc & OWED == 0 {
        return lo;
    }
    let picture = xc >> 4;
    while *from < hi && st.query.code.get(*from).is_some_and(|&yc| yc >> 4 <= picture) {
        *from += 1;
    }
    *from
}

/// The probe of query bucket `v`'s `p`th (0: the same value; 1..17: the value with bit `p - 1` flipped).
#[inline(always)]
fn probe(v: usize, p: usize) -> usize {
    if p == 0 { v } else { v ^ (1 << (p - 1)) }
}

/// The buckets `v` of the query side, and the 17 probes of each: the stored side's bucket of the same value, and of
/// each one-bit neighbour.
#[inline(always)]
fn scan_block(st: &Stage<'_, '_, '_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    for v in buckets {
        let (qlo, qhi) = st.query.bucket(v);
        if qlo == qhi {
            continue;
        }
        for p in 0..17 {
            let (slo, shi) = st.stored.bucket(probe(v, p));
            let mut from = qlo;
            for x in slo..shi {
                for y in suffix(st, x, qlo, qhi, &mut from)..qhi {
                    consider(st, x, y, out);
                }
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "popcnt")]
unsafe fn scan_popcnt(st: &Stage<'_, '_, '_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    scan_block(st, buckets, out);
}

/// The vector kernel: four query entries at a time, their first two words' distance in one register, and only the
/// lanes within the radius checked in full.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,popcnt")]
unsafe fn scan_avx2(st: &Stage<'_, '_, '_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi8, _mm256_and_si256, _mm256_castsi256_pd, _mm256_cmpgt_epi64, _mm256_loadu_si256,
        _mm256_movemask_pd, _mm256_sad_epu8, _mm256_set1_epi8, _mm256_set1_epi64x, _mm256_setr_epi8,
        _mm256_setzero_si256, _mm256_shuffle_epi8, _mm256_srli_epi16, _mm256_xor_si256,
    };
    let lut = _mm256_setr_epi8(
        0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4, 0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4,
    );
    let low = _mm256_set1_epi8(0x0f);
    let thr = _mm256_set1_epi64x(i64::from(MATCH_DISTANCE));
    let zero = _mm256_setzero_si256();
    let popcount_bytes = |v: __m256i| -> __m256i {
        let lo = _mm256_and_si256(v, low);
        let hi = _mm256_and_si256(_mm256_srli_epi16(v, 4), low);
        _mm256_add_epi8(_mm256_shuffle_epi8(lut, lo), _mm256_shuffle_epi8(lut, hi))
    };
    let (head0, head1) = (st.query.head0.as_slice(), st.query.head1.as_slice());
    for v in buckets {
        let (qlo, qhi) = st.query.bucket(v);
        if qlo == qhi {
            continue;
        }
        for p in 0..17 {
            let (slo, shi) = st.stored.bucket(probe(v, p));
            let mut from = qlo;
            for x in slo..shi {
                let mut y = suffix(st, x, qlo, qhi, &mut from);
                let (x0, x1) =
                    (st.stored.head0.get(x).copied().unwrap_or(0), st.stored.head1.get(x).copied().unwrap_or(0));
                let vx0 = _mm256_set1_epi64x(i64::from_ne_bytes(x0.to_ne_bytes()));
                let vx1 = _mm256_set1_epi64x(i64::from_ne_bytes(x1.to_ne_bytes()));
                while y < qhi {
                    // SAFETY: `y < qhi <= n`, and head0/head1 hold n + 4 values (four zeros of padding), so the four
                    // lanes at `y` are in bounds.
                    let (a0, a1) = unsafe {
                        (
                            _mm256_loadu_si256(head0.as_ptr().add(y).cast::<__m256i>()),
                            _mm256_loadu_si256(head1.as_ptr().add(y).cast::<__m256i>()),
                        )
                    };
                    let bytes = _mm256_add_epi8(
                        popcount_bytes(_mm256_xor_si256(a0, vx0)),
                        popcount_bytes(_mm256_xor_si256(a1, vx1)),
                    );
                    // Per 64-bit lane, the sum of its eight byte counts: the distance over the first two words.
                    let head = _mm256_sad_epu8(bytes, zero);
                    let over = _mm256_movemask_pd(_mm256_castsi256_pd(_mm256_cmpgt_epi64(head, thr)));
                    let mut lanes = u32::try_from(!over & 0xF).unwrap_or(0);
                    while lanes != 0 {
                        let lane = usize::try_from(lanes.trailing_zeros()).unwrap_or(0);
                        lanes &= lanes - 1;
                        if y + lane < qhi {
                            consider(st, x, y + lane, out);
                        }
                    }
                    y += 4;
                }
            }
        }
    }
}

/// Runs one chunk's stage on `kernel`, over the buckets in blocks, on `threads` workers. The pairs found, compacted; or
/// `None` when `control` stopped it.
fn run_stage(
    kernel: Kernel,
    st: &Stage<'_, '_, '_>,
    threads: usize,
    start: &(dyn Fn() + Sync),
    control: &(dyn Fn() -> bool + Sync),
) -> Option<Vec<Pair>> {
    parallel(threads, VALUES.div_ceil(BLOCK), start, control, |block, out| {
        let lo = block * BLOCK;
        scan(kernel, st, lo..(lo + BLOCK).min(VALUES), out);
    })
}

/// Runs `work` on the blocks `0..blocks`, each once, on `threads` workers that take the next block as they finish one
/// and ask `control` before each. The pairs found, compacted; or `None` when `control` stopped it.
fn parallel(
    threads: usize,
    blocks: usize,
    start: &(dyn Fn() + Sync),
    control: &(dyn Fn() -> bool + Sync),
    work: impl Fn(usize, &mut Vec<Pair>) + Sync,
) -> Option<Vec<Pair>> {
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let mut all = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.max(1))
            .map(|_| {
                scope.spawn(|| {
                    start();
                    let mut out = Vec::new();
                    loop {
                        if stopped.load(Relaxed) || !control() {
                            stopped.store(true, Relaxed);
                            break;
                        }
                        let block = next.fetch_add(1, Relaxed);
                        if block >= blocks {
                            break;
                        }
                        work(block, &mut out);
                    }
                    compact(&mut out);
                    out
                })
            })
            .collect();
        for worker in workers {
            match worker.join() {
                Ok(pairs) => all.extend(pairs),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
    });
    if stopped.load(Relaxed) {
        return None;
    }
    compact(&mut all);
    Some(all)
}

/// Dispatches one block to the kernel.
fn scan(kernel: Kernel, st: &Stage<'_, '_, '_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    match kernel {
        Kernel::Scalar => scan_block(st, buckets, out),
        #[cfg(target_arch = "x86_64")]
        // SAFETY: `Popcnt` is chosen only when `best_kernel` detected POPCNT.
        Kernel::Popcnt => unsafe { scan_popcnt(st, buckets, out) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: `Avx2` is chosen only when `best_kernel` detected AVX2 and POPCNT.
        Kernel::Avx2 => unsafe { scan_avx2(st, buckets, out) },
        #[cfg(not(target_arch = "x86_64"))]
        _ => scan_block(st, buckets, out),
    }
}

/// Sorts the pairs by `(a, b, distance)` and keeps the least distance of each `(a, b)`.
fn compact(pairs: &mut Vec<Pair>) {
    pairs.sort_unstable();
    pairs.dedup_by_key(|p| (p.a, p.b));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flip(h: &Hash, k: usize) -> Hash {
        let mut out = *h;
        out.0[k / 8] ^= 1 << (7 - k % 8);
        out
    }

    struct Rng(u64);

    impl Rng {
        fn draw(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn hash(&mut self) -> Hash {
            let mut b = [0u8; 32];
            for chunk in b.chunks_mut(8) {
                chunk.copy_from_slice(&self.draw().to_le_bytes());
            }
            Hash(b)
        }

        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.draw() % u64::try_from(n).unwrap_or(1)).unwrap_or(0)
        }

        fn four(&mut self) -> [Hash; 4] {
            [self.hash(), self.hash(), self.hash(), self.hash()]
        }
    }

    /// A library: each picture's kept orientations and whether it owes its pairs.
    struct Library {
        hashes: Vec<[Hash; 4]>,
        owed: Vec<bool>,
    }

    /// `n` pictures: random ones, and copies of earlier ones with a few bits flipped in every orientation (so they match
    /// near the radius); `owed_in_ten` of every ten owe their pairs.
    fn corpus(rng: &mut Rng, n: usize, owed_in_ten: usize) -> Library {
        let mut hashes: Vec<[Hash; 4]> = Vec::new();
        let mut owed = Vec::new();
        for _ in 0..n {
            let four = if hashes.is_empty() || rng.below(3) == 0 {
                rng.four()
            } else {
                let mut s = hashes[rng.below(hashes.len())];
                let flips = rng.below(48);
                for h in &mut s {
                    for _ in 0..flips {
                        *h = flip(h, rng.below(256));
                    }
                }
                s
            };
            hashes.push(four);
            owed.push(rng.below(10) < owed_in_ten);
        }
        Library { hashes, owed }
    }

    /// The eight orientations of a picture by the definition: the kept four, then their transposes bit by bit.
    fn eight(four: &[Hash; 4]) -> Vec<Hash> {
        let bit_transpose = |h: &Hash| {
            let bit = |k: usize| (h.0[k / 8] >> (7 - k % 8)) & 1;
            let mut out = [0u8; 32];
            for k in 0..256 {
                out[k / 8] |= bit((k % 16) * 16 + k / 16) << (7 - k % 8);
            }
            Hash(out)
        };
        four.iter().copied().chain(four.iter().map(bit_transpose)).collect()
    }

    /// The pairs the definition says, over the 8 x 8 orientations, for the pictures whose query side is in `queries`.
    fn brute(lib: &Library, queries: Range<usize>) -> Vec<Pair> {
        let all: Vec<Vec<Hash>> = lib.hashes.iter().map(eight).collect();
        let mut out = Vec::new();
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                if !lib.owed[i] && !lib.owed[j] {
                    continue;
                }
                let query = if lib.owed[j] { j } else { i };
                if !queries.contains(&query) {
                    continue;
                }
                let best = all[i].iter().flat_map(|a| all[j].iter().map(move |b| a.distance(b))).min().unwrap();
                if best <= MATCH_DISTANCE {
                    out.push(Pair { a: u32::try_from(i).unwrap(), b: u32::try_from(j).unwrap(), distance: best });
                }
            }
        }
        out.sort_unstable();
        out
    }

    fn run(kernel: Kernel, lib: &Library, queries: Range<usize>, threads: usize) -> Vec<Pair> {
        run_tuned(kernel, lib, queries, threads, Tuning::default())
    }

    fn run_tuned(kernel: Kernel, lib: &Library, queries: Range<usize>, threads: usize, tuning: Tuning) -> Vec<Pair> {
        Joiner::new(&lib.hashes, &lib.owed)
            .unwrap()
            .with_kernel(kernel)
            .tuned(tuning)
            .join(queries, threads, &|| {}, &|| true)
            .unwrap()
            .pairs
    }

    /// Every way a join can run: no sorter, one, two side by side; and the few-owed path never and always.
    fn tunings() -> Vec<Tuning> {
        let mut all = Vec::new();
        for sorters in [Some(0), Some(1), Some(2)] {
            for few_entries in [Some(0), Some(usize::MAX)] {
                all.push(Tuning { sorters, few_entries });
            }
        }
        all
    }

    fn kernels() -> Vec<Kernel> {
        let mut k = vec![Kernel::Scalar];
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("popcnt") {
                k.push(Kernel::Popcnt);
            }
            if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("popcnt") {
                k.push(Kernel::Avx2);
            }
        }
        k
    }

    #[test]
    fn the_permutation_is_a_bijection_on_diagonals_that_keeps_distances() {
        let mut rng = Rng(7);
        for _ in 0..200 {
            let (a, b) = (rng.hash(), rng.hash());
            let (pa, pb) = (permuted(&a), permuted(&b));
            let d: u32 = pa.iter().zip(&pb).map(|(x, y)| (x ^ y).count_ones()).sum();
            assert_eq!(d, a.distance(&b));
        }
        // One bit at (row, column) lands in chunk `column - row` (mod 16), and no two bits land on one place.
        let mut seen = std::collections::BTreeSet::new();
        for row in 0..16 {
            for col in 0..16 {
                let k = row * 16 + col;
                let mut bytes = [0u8; 32];
                bytes[k / 8] = 1 << (7 - k % 8);
                let words = permuted(&Hash(bytes));
                let set: Vec<usize> = (0..CHUNKS).filter(|&c| chunk(&words, c) != 0).collect();
                assert_eq!(set, vec![(col + 16 - row) % 16], "bit ({row}, {col})");
                assert!(seen.insert(words), "bit ({row}, {col}) collides");
            }
        }
    }

    #[test]
    fn the_radix_sort_is_a_stable_sort_by_the_chunk() {
        let mut rng = Rng(99);
        // Few distinct chunk values, so the buckets are long and stability shows.
        let entries: Vec<([u64; 4], u32)> = (0..5000u32)
            .map(|i| {
                let w =
                    [rng.draw() & 0x0303_0303_0303_0303, rng.draw(), rng.draw(), rng.draw() & 0x0101_0101_0101_0101];
                (w, i)
            })
            .collect();
        let (mut out, mut scratch) = (Sorted::default(), Vec::new());
        for c in [0, 3, 7, 12, 15] {
            sort_into(&mut out, &mut scratch, entries.len(), |e| entries[e], c);
            let mut want = entries.clone();
            want.sort_by_key(|(w, _)| chunk(w, c));
            assert_eq!(out.code, want.iter().map(|(_, i)| *i).collect::<Vec<_>>(), "chunk {c}");
            assert_eq!(out.head0[..entries.len()], want.iter().map(|(w, _)| w[0]).collect::<Vec<_>>()[..]);
            assert_eq!(out.head1[..entries.len()], want.iter().map(|(w, _)| w[1]).collect::<Vec<_>>()[..]);
            assert_eq!(out.head0.len(), entries.len() + 4);
            for v in [0usize, 1, 0x0101, 0x0303, 0xFFFF] {
                let (lo, hi) = out.bucket(v);
                assert!(out.code[lo..hi].iter().all(|&i| chunk(&entries[index(i)].0, c) == v));
                assert_eq!(hi - lo, want.iter().filter(|(w, _)| chunk(w, c) == v).count());
            }
        }
    }

    #[test]
    fn matches_brute_force_at_every_owed_share_and_kernel() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for owed_in_ten in [10, 5, 1, 0] {
            let lib = corpus(&mut rng, 400, owed_in_ten);
            let want = brute(&lib, 0..lib.hashes.len());
            assert!(owed_in_ten == 0 || want.len() > 10, "the corpus must have pairs to find");
            for kernel in kernels() {
                assert_eq!(run(kernel, &lib, 0..lib.hashes.len(), 3), want, "{kernel:?}, owed {owed_in_ten} in 10");
            }
        }
    }

    /// The ordering rule's suffix, where it is easy to get wrong: buckets full of one picture's orientations and of
    /// copies of it, owed and not, interleaved, so a stored picture's queries start mid-bucket and the pointer passes
    /// several entries of one picture.
    #[test]
    fn crowded_buckets_of_owed_and_not_owed_copies_match_brute_force() {
        let mut rng = Rng(31);
        let base = rng.four();
        let mut lib = Library { hashes: Vec::new(), owed: Vec::new() };
        for i in 0..120 {
            let mut four = if i % 7 == 6 { rng.four() } else { base };
            for h in &mut four {
                for _ in 0..rng.below(6) {
                    *h = flip(h, rng.below(256));
                }
            }
            // Symmetric pictures too: an orientation equal to its own transpose puts two entries in one bucket.
            if i % 5 == 0 {
                four[1] = four[1].transpose();
                four[2] = four[1];
            }
            lib.hashes.push(four);
            lib.owed.push(matches!(i % 4, 0 | 3));
        }
        let want = brute(&lib, 0..lib.hashes.len());
        assert!(want.len() > 1000, "the copies pair with each other: {}", want.len());
        for kernel in kernels() {
            for tuning in tunings() {
                let all = 0..lib.hashes.len();
                assert_eq!(run_tuned(kernel, &lib, all, 3, tuning), want, "{kernel:?} {tuning:?}");
                for slice in [0..40, 40..41, 41..120] {
                    let got = run_tuned(kernel, &lib, slice.clone(), 3, tuning);
                    assert_eq!(got, brute(&lib, slice.clone()), "{kernel:?} {tuning:?} {slice:?}");
                }
            }
        }
    }

    #[test]
    fn slices_of_the_queries_add_up_to_the_whole() {
        let mut rng = Rng(42);
        let lib = corpus(&mut rng, 300, 6);
        let whole = run(best_kernel(), &lib, 0..lib.hashes.len(), 2);
        assert_eq!(whole, brute(&lib, 0..lib.hashes.len()));
        let joiner = Joiner::new(&lib.hashes, &lib.owed).unwrap();
        let mut parts = Vec::new();
        for slice in [0..77, 77..150, 150..300] {
            let got = joiner.join(slice.clone(), 2, &|| {}, &|| true).unwrap().pairs;
            assert_eq!(got, brute(&lib, slice));
            parts.extend(got);
        }
        parts.sort_unstable();
        assert_eq!(parts, whole);
    }

    #[test]
    fn threads_do_not_change_the_pairs() {
        let mut rng = Rng(5);
        let lib = corpus(&mut rng, 250, 10);
        let one = run(best_kernel(), &lib, 0..lib.hashes.len(), 1);
        for threads in [2, 4, 12] {
            assert_eq!(run(best_kernel(), &lib, 0..lib.hashes.len(), threads), one);
        }
    }

    #[test]
    fn the_radius_is_inclusive_and_exact() {
        let mut rng = Rng(11);
        let base = rng.four();
        // Only the first orientation is near: the others are unrelated, so no other combination is within the radius.
        let unrelated_rest = [rng.hash(), rng.hash(), rng.hash()];
        for (flips, found) in [(31usize, true), (32, false)] {
            let mut first = base[0];
            for k in 0..flips {
                first = flip(&first, k * 7 + 1);
            }
            let moved = [first, unrelated_rest[0], unrelated_rest[1], unrelated_rest[2]];
            let lib = Library { hashes: vec![base, moved], owed: vec![true, true] };
            let want = brute(&lib, 0..2);
            assert_eq!(!want.is_empty(), found, "brute force at {flips} flips");
            for kernel in kernels() {
                assert_eq!(run(kernel, &lib, 0..2, 1), want, "{kernel:?} at {flips} flips");
            }
        }
    }

    /// A picture that matches another only through a transpose: the query's transposed orientations are what find it.
    #[test]
    fn a_transposed_copy_is_found_through_the_query_transposes() {
        let mut rng = Rng(17);
        let a = rng.four();
        let b = [flip(&a[2].transpose(), 9), rng.hash(), rng.hash(), rng.hash()];
        for owed in [[true, false], [false, true], [true, true]] {
            let lib = Library { hashes: vec![a, b], owed: owed.to_vec() };
            let want = brute(&lib, 0..2);
            assert_eq!(want, vec![Pair { a: 0, b: 1, distance: 1 }], "{owed:?}");
            for kernel in kernels() {
                assert_eq!(run(kernel, &lib, 0..2, 1), want, "{kernel:?} {owed:?}");
            }
        }
    }

    #[test]
    fn a_stop_returns_none() {
        let mut rng = Rng(3);
        let lib = corpus(&mut rng, 50, 10);
        for tuning in tunings() {
            let joiner = Joiner::new(&lib.hashes, &lib.owed).unwrap().tuned(tuning);
            assert!(joiner.join(0..50, 4, &|| {}, &|| false).is_none(), "{tuning:?}");
        }
    }

    /// A stop that comes while the sorter is ahead of the scan (or mid-pass over the store) ends the join, with every
    /// thread joined: the call returns, with nothing.
    #[test]
    fn a_stop_in_the_middle_returns_none_with_the_sorters_running() {
        let mut rng = Rng(23);
        let lib = corpus(&mut rng, 600, 8);
        for tuning in tunings() {
            for after in [1, 5, 40, 300] {
                let (asked, refused) = (AtomicUsize::new(0), AtomicBool::new(false));
                let control = || {
                    let go = asked.fetch_add(1, Relaxed) < after;
                    refused.fetch_or(!go, Relaxed);
                    go
                };
                let joiner = Joiner::new(&lib.hashes, &lib.owed).unwrap().tuned(tuning);
                let joined = joiner.join(0..600, 4, &|| {}, &control);
                // A join with fewer blocks of work than `after` finishes before the stop: it returns its pairs then.
                assert_eq!(joined.is_none(), refused.load(Relaxed), "{tuning:?}, stopped after {after}");
            }
            let joiner = Joiner::new(&lib.hashes, &lib.owed).unwrap().tuned(tuning);
            let asked = AtomicUsize::new(0);
            assert!(joiner.join(0..600, 4, &|| {}, &|| asked.fetch_add(1, Relaxed) < 2).is_none(), "{tuning:?}");
        }
    }

    /// The sorted path with each number of sorters, and the few-owed path, find the same pairs as brute force, on every
    /// kernel and at every owed share (the few-owed path forced on and off by its threshold).
    #[test]
    fn every_path_matches_brute_force() {
        let mut rng = Rng(0x51_7CC1_B727_220A);
        for owed_in_ten in [10, 3, 1] {
            let lib = corpus(&mut rng, 350, owed_in_ten);
            let want = brute(&lib, 0..lib.hashes.len());
            assert!(want.len() > 10, "the corpus must have pairs to find");
            for kernel in kernels() {
                for tuning in tunings() {
                    for threads in [1, 3, 6] {
                        let got = run_tuned(kernel, &lib, 0..lib.hashes.len(), threads, tuning);
                        assert_eq!(got, want, "{kernel:?} {tuning:?} on {threads}, owed {owed_in_ten} in 10");
                    }
                }
            }
            // A slice, which the few-owed path takes by default when it is small.
            for slice in [0..20, 100..103, 200..350] {
                for tuning in tunings() {
                    assert_eq!(run_tuned(best_kernel(), &lib, slice.clone(), 4, tuning), brute(&lib, slice.clone()));
                }
            }
        }
    }

    /// The full library's join, from a dump of a catalog's hashes (`PG_JOIN_DUMP`: records of an id as `i64`
    /// little-endian, an owed byte and the four kept orientations, 137 bytes, by id). The pairs go to `PG_JOIN_OUT` as
    /// `id_a id_b distance`. `PG_JOIN_THREADS` sets the workers (default: the processor's). Prints the timing of each
    /// kernel on one thread, then of the best kernel on the workers.
    #[test]
    #[ignore = "needs a catalog dump: PG_JOIN_DUMP and PG_JOIN_OUT (see the test's doc comment)"]
    fn dump_join() {
        use std::io::Write;
        let dump = std::fs::read(std::env::var("PG_JOIN_DUMP").unwrap()).unwrap();
        const RECORD: usize = 8 + 1 + 128;
        let (mut ids, mut lib) = (Vec::new(), Library { hashes: Vec::new(), owed: Vec::new() });
        for rec in dump.as_chunks::<RECORD>().0 {
            ids.push(i64::from_le_bytes(rec[..8].try_into().unwrap()));
            lib.owed.push(rec[8] != 0);
            lib.hashes.push(std::array::from_fn(|v| Hash(rec[9 + 32 * v..9 + 32 * v + 32].try_into().unwrap())));
        }
        let n = lib.hashes.len();
        println!("pictures {n}, owed {}", lib.owed.iter().filter(|&&o| o).count());
        let threads: usize = std::env::var("PG_JOIN_THREADS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |p| p.get()));
        for kernel in kernels() {
            let t = std::time::Instant::now();
            let pairs = run(kernel, &lib, 0..n, 1);
            println!("{kernel:?} 1 thread: {:?}, {} pairs", t.elapsed(), pairs.len());
        }
        let t = std::time::Instant::now();
        let pairs = run(best_kernel(), &lib, 0..n, threads);
        println!("{:?} {threads} threads: {:?}, {} pairs", best_kernel(), t.elapsed(), pairs.len());
        let mut out = std::io::BufWriter::new(std::fs::File::create(std::env::var("PG_JOIN_OUT").unwrap()).unwrap());
        for p in &pairs {
            let (a, b) = (ids[index(p.a)], ids[index(p.b)]);
            writeln!(out, "{a} {b} {}", p.distance).unwrap();
        }
    }
}
