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
//! pass; a slice's query pictures add their transposed orientations, renamings of the stored ones' chunks
//! (`permuted`), made where they are read: in the sort, and on a hit. A stage (one chunk) sorts both sides into 256
//! regions per side, one per high byte of the chunk. Each region holds its entries' codes (`picture << 4 | owed << 3 |
//! orientation`) and heads: the stage's `w` words of the chunks after its own (`head_words`), what the kernels filter
//! on, against a bound exact for the pairs the
//! stage reports (`bound`); the stored side's heads in rows, the query side's in columns. The full hash of a candidate
//! that passes the filter is read back from the store by its code.
//!
//! **The scan.** A query bucket's 17 probes are ten runs: stored buckets of one region at one bound. Each stored entry of
//! a run is tested against its suffix of the query bucket eight entries at a time; a bucket of at most eight entries is
//! loaded once, and the ordering rule is a mask of its lanes (`scan_runs`). Every kernel tests the same candidates.
//!
//! **Two paths, one set of candidates.** The sorted path sorts each stage's sides and scans them, a stage at a time. A
//! slice with few query entries ([`Tuning::few_entries`]) sorts nothing of the stored side: each chunk lists the query
//! entries under their values' probes, and one pass over the store looks up every stored entry's values. Both test the
//! same (stored, query, chunk) candidates with the same rules, so they return the same pairs.
//!
//! **One team.** A join's threads are one team, which does all of its work: the sorts' passes by blocks and regions,
//! the scans by blocks of buckets, the few-owed path's lists and pass, claimed from one schedule (`run_team`). The whole
//! team sorts a stage, then scans it; a thread waits only when its phase has no task left to claim and the next phase
//! has not begun (the phase's tail).

#[cfg(test)]
use std::cell::Cell;
use std::ops::Range;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use pg_model::num::index;
use pg_model::pdq::{Hash, MATCH_DISTANCE, ORIENTATIONS, transpose_rows};

/// The chunks a hash is split into: 16 of 16 bits.
const CHUNKS: usize = 16;
/// The values a chunk takes.
const VALUES: usize = 1 << 16;
/// The query buckets a scan task takes by default ([`Tuning::block`]; its unit of work, and of control).
pub const BLOCK: usize = 128;
/// A query's orientations: the kept ones and their transposes.
const QUERY_ORIENTATIONS: usize = 2 * ORIENTATIONS;
/// The most pictures the join holds: an entry's code is the picture's index, an owed bit and an orientation (3 bits) in
/// a `u32`.
pub const MAX_PICTURES: u32 = u32::MAX >> 4;
/// The code's owed bit.
const OWED: u32 = 1 << 3;
/// The values after a region's codes: [`suffix_avx2`] reads eight codes from any position up to the region's length.
const CODE_PADDING: usize = 8;
/// The query entries the kernels test at a time: one 256-bit register of `u32` lanes.
const LANES: usize = 8;
/// The most head words: two chunks to a word, of the 15 other than the stage's own.
const MAX_HEAD_WORDS: usize = 7;

/// What a full test of a candidate costs at the margin, in head words per lane: about 100 ns for a test whose two
/// hashes miss the cache (the stored side is 32 bytes a picture's orientation, past L3), against 0.16 ns for one more
/// head word per lane (24 ms a stage over its 150 M lanes). Measured on the i7-8700 at one to five words at every stage
/// (docs/reference/PROFILING.md, "The scan kernel"); the widths it gives ([`WIDTHS`]) are the same from about 500 to
/// past 1,200, and are those the measurements found fastest.
const FULL_TEST_COST: f64 = 600.0;

/// `P(Bin(n, 1/2) <= t)`: the share of unrelated hashes (whose bits differ by coin flips) within `t` bits over `n`. The
/// counts kept in floats, as `f64::from` is not const.
const fn tail(n: u32, t: u32) -> f64 {
    let (mut term, mut k) = (1.0f64, 0);
    while k < n {
        term *= 0.5;
        k += 1;
    }
    let (mut sum, mut i, mut fi, mut fnn) = (0.0, 0, 0.0, 0.0);
    k = 0;
    while k < n {
        fnn += 1.0;
        k += 1;
    }
    while i <= t && i <= n {
        sum += term;
        term *= (fnn - fi) / (fi + 1.0);
        i += 1;
        fi += 1.0;
    }
    sum
}

/// Each stage's head words: the fewest for which one word more costs more than the false candidates it removes. A word
/// costs one per lane; the candidates that pass the head test without being pairs are the unrelated ones within the
/// bound ([`tail`] of the head's bits, at the bound of the probes that flip a bit, 16 of the 17), each a full test
/// ([`FULL_TEST_COST`]). The pairs within the radius pass at every width, so they do not choose it.
#[expect(clippy::indexing_slicing, reason = "built at compile time, `c` below the array's 16")]
const fn widths() -> [usize; CHUNKS] {
    let mut out = [1; CHUNKS];
    // Each count in the types it is used as, kept side by side (a const fn has no conversions between them).
    let (mut c, mut cu) = (0usize, 0u32);
    while c < CHUNKS {
        let (mut best, mut best_cost) = (1, f64::MAX);
        let (mut w, mut wu, mut wf) = (1usize, 1u32, 1.0f64);
        while w <= MAX_HEAD_WORDS {
            let cost = wf + FULL_TEST_COST * tail(32 * wu, bound(cu, wu, true));
            if cost < best_cost {
                (best, best_cost) = (w, cost);
            }
            (w, wu, wf) = (w + 1, wu + 1, wf + 1.0);
        }
        out[c] = best;
        (c, cu) = (c + 1, cu + 1);
    }
    out
}

/// The head words of each stage by default ([`widths`]).
const WIDTHS: [usize; CHUNKS] = widths();

/// The last chunk, the one before chunk 0 in a head that wraps ([`earlier_outside`]).
const LAST_CHUNK: u32 = 15;

/// The chunks before stage `c` that a head of `w` words leaves out. The head is chunks `c + 1 ..= c + 2w` (mod 16), of
/// which `c + 2w - 15` wrap around to the chunks before `c` (at most `c`, as `2w <= 15`).
const fn earlier_outside(c: u32, w: u32) -> u32 {
    c - (c + 2 * w).saturating_sub(LAST_CHUNK)
}

/// The bound of a stage's head test, exact for the pairs the stage reports. A pair is reported at the first chunk within
/// one bit (`matched_earlier`), so at stage `c` each chunk before `c` is at distance 2 or more, and chunk `c` itself at 1
/// when the probe flipped a bit (`flipped`), at 0 when not. Of the radius, the head may then take what those leave:
/// `31 - [flipped] - 2 * earlier_outside(c, w)`. A pair over it is not this stage's to report (it is over the radius, or
/// an earlier chunk reports it), and every pair under it is tested in full, so the pairs and their distances are those
/// of the full test.
const fn bound(c: u32, w: u32, flipped: bool) -> u32 {
    MATCH_DISTANCE - 2 * earlier_outside(c, w) - if flipped { 1 } else { 0 }
}

// No bound is negative: what the chunks before a stage and its own flipped bit take is within the radius, at every
// stage and width (the least is 4: one word, from stage 13). Evaluating `bound` here would fail the build on overflow.
const _: () = {
    let mut c = 0;
    while c <= LAST_CHUNK {
        let mut w = 1;
        while w <= 7 {
            assert!(2 * earlier_outside(c, w) < MATCH_DISTANCE && bound(c, w, true) >= 4);
            w += 1;
        }
        c += 1;
    }
    assert!(MAX_HEAD_WORDS == 7 && CHUNKS == 16);
};

/// The bounds of stage `c` at `w` head words: of probe 0, and of the probes that flip a bit.
fn bounds(c: usize, w: usize) -> [u32; 2] {
    let (c, w) = (u32::try_from(c).unwrap_or(LAST_CHUNK), u32::try_from(w).unwrap_or(1));
    [bound(c, w, false), bound(c, w, true)]
}

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
///
/// Within a chunk, the bits of diagonals 0 - 8 are in row order (bit `15 - row` is the cell of that row) and those of
/// diagonals 9 - 15 in column order (bit `15 - column`). The transpose maps diagonal `d` to `16 - d`, and with this
/// order it maps the cell at bit `b` of one to bit `b` of the other: the transposed hash is a renaming of chunks
/// ([`transposed`]), not a transpose of bits. The order within a chunk changes no distance and no neighbour.
pub(crate) fn permuted(h: &Hash) -> [u64; 4] {
    let mut rows = h.rows();
    for (r, row) in (0u32..).zip(rows.iter_mut()) {
        *row = row.rotate_left(r);
    }
    let mut diagonals = transpose_rows(rows);
    // Diagonal d's cell of row r is at bit 15 - r; of column r + d at bit 15 - (r + d): a rotation right by d.
    for (d, diagonal) in diagonals.iter_mut().enumerate().skip(CHUNKS / 2 + 1) {
        *diagonal = diagonal.rotate_right(u32::try_from(d).unwrap_or(0));
    }
    let mut words = [0u64; 4];
    for (word, four) in words.iter_mut().zip(diagonals.as_chunks::<4>().0) {
        *word = four.iter().rev().fold(0, |w, &d| (w << 16) | u64::from(d));
    }
    words
}

/// The hash a permuted one was made from ([`permuted`]'s inverse): the diagonals unpacked and turned back, the grid
/// transposed again (a transpose is its own inverse), and each row rotated back.
pub(crate) fn unpermuted(words: &[u64; 4]) -> Hash {
    let mut diagonals = [0u16; CHUNKS];
    for (four, &word) in diagonals.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        for (k, diagonal) in four.iter_mut().enumerate() {
            *diagonal = u16::try_from((word >> (16 * k)) & 0xFFFF).unwrap_or(0);
        }
    }
    for (d, diagonal) in diagonals.iter_mut().enumerate().skip(CHUNKS / 2 + 1) {
        *diagonal = diagonal.rotate_left(u32::try_from(d).unwrap_or(0));
    }
    let mut rows = transpose_rows(diagonals);
    for (r, row) in (0u32..).zip(rows.iter_mut()) {
        *row = row.rotate_right(r);
    }
    Hash::from_rows(&rows)
}

/// A permuted hash's 16 chunks ([`chunk`] of each), in order.
#[inline(always)]
fn chunks(words: &[u64; 4]) -> [u16; CHUNKS] {
    let mut chunks = [0u16; CHUNKS];
    for (four, &word) in chunks.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        for (k, slot) in four.iter_mut().enumerate() {
            *slot = u16::try_from((word >> (16 * k)) & 0xFFFF).unwrap_or(0);
        }
    }
    chunks
}

/// The chunks of the permuted hash ([`permuted`]) of a picture's transpose, from the chunks of its own: chunk `c` is
/// chunk `16 - c` (`0` stays), and chunk 8, which the transpose maps to itself with each cell's partner eight rows on,
/// has its two bytes swapped.
#[inline(always)]
fn transposed_chunks(chunks: &[u16; CHUNKS]) -> [u16; CHUNKS] {
    std::array::from_fn(|c| {
        let v = chunks.get((CHUNKS - c) % CHUNKS).copied().unwrap_or(0);
        if c == CHUNKS / 2 { v.swap_bytes() } else { v }
    })
}

/// The permuted hash of a picture's transpose, from its own ([`transposed_chunks`]).
#[inline(always)]
fn transposed(words: &[u64; 4]) -> [u64; 4] {
    let renamed = transposed_chunks(&chunks(words));
    let mut out = [0u64; 4];
    for (word, four) in out.iter_mut().zip(renamed.as_chunks::<4>().0) {
        *word = four.iter().rev().fold(0, |w, &v| (w << 16) | u64::from(v));
    }
    out
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

/// The most owed pictures of a slice the few-owed path takes by default, on a join of `T` threads: `FEW_BASE +
/// FEW_PER_THREAD * T` ([`Tuning::few_entries_on`]); a larger slice takes the sorted path. Both are the least-squares
/// line through the two paths' measured crossovers on 1, 2, 4, 6, 8 and 12 threads (253, 309, 357, 415, 585 and 652
/// owed pictures; docs/reference/PROFILING.md, "The settings again"). The few-owed path's work spreads over every
/// thread; the sorted path's floor, sorting the stored side, gains little past the physical cores, so the crossover
/// grows with the threads, though slower than in proportion to them. Both paths' costs grow with the stored side, so
/// the crossover in owed pictures does not depend on the library's size to first order.
pub const FEW_BASE: usize = 221;
/// See [`FEW_BASE`].
pub const FEW_PER_THREAD: usize = 38;
/// The stored entries a thread of the few-owed path takes at a time (its unit of work, and of control).
const FEW_BLOCK: usize = 4096;
/// What a join may be tuned by (the gates of docs/plans/PAIRING_PASS_PLAN.md set the defaults, which `None` takes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tuning {
    /// The most query entries (`QUERY_ORIENTATIONS` per owed picture) the few-owed path takes. `None`: [`FEW_BASE`]
    /// and [`FEW_PER_THREAD`] for each of the join's threads, in owed pictures.
    pub few_entries: Option<usize>,
    /// Each stage's head words, 1 - 7 (others are clamped to them). `None`: by the cost of a word against the false
    /// candidates it removes (`widths`).
    pub widths: Option<[usize; CHUNKS]>,
    /// The query buckets a scan task takes (at least 1). `None`: [`BLOCK`].
    pub block: Option<usize>,
}

impl Tuning {
    /// The most query entries the few-owed path takes in a join on `threads` threads: [`Tuning::few_entries`], or
    /// [`FEW_BASE`] and [`FEW_PER_THREAD`] for each thread, in owed pictures.
    pub fn few_entries_on(&self, threads: usize) -> usize {
        self.few_entries.unwrap_or_else(|| {
            FEW_PER_THREAD.saturating_mul(threads).saturating_add(FEW_BASE).saturating_mul(QUERY_ORIENTATIONS)
        })
    }
}

/// Where a join's time went: the slice's preparation on the thread that runs the join, and the rest as the team's
/// threads' time, summed over them (on `n` threads, about `n` times the wall clock in all).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JoinTimes {
    /// The slice's query side: its owed pictures (wall clock).
    pub prepare: Duration,
    /// In the sorts' passes, or the few-owed path's table and lists.
    pub sort: Duration,
    /// In the scans' blocks, or the few-owed path's pass over the store.
    pub scan: Duration,
    /// Waiting with nothing to take: a sort's pass not done yet, or a stage's last blocks on other threads.
    pub starved: Duration,
    /// In the caller's `control`: a pause, mostly.
    pub paused: Duration,
}

impl std::ops::AddAssign for JoinTimes {
    fn add_assign(&mut self, other: JoinTimes) {
        self.prepare += other.prepare;
        self.sort += other.sort;
        self.scan += other.scan;
        self.starved += other.starved;
        self.paused += other.paused;
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
    /// Whether each picture owes its pairs (missing entries owe nothing).
    owed: &'a [bool],
    /// Orientation `o` of picture `i` (in file-id order: the pairs are by index), permuted, at `ORIENTATIONS * i + o`.
    stored: Vec<[u64; 4]>,
    kernel: Kernel,
    tuning: Tuning,
    /// The tests' injected panic: in the task of this kind that brings the count to zero.
    #[cfg(test)]
    fault: Option<(Kind, AtomicUsize)>,
    /// The tests' check that every task runs on a thread that ran `start` ([`CHECK_START`] on the building thread).
    #[cfg(test)]
    checks_start: bool,
}

#[cfg(test)]
thread_local! {
    /// The tests' check that the caller's `start` runs before all of a join's work: set on the thread that builds a
    /// [`Joiner`], the store's build on the join's threads and every task of its joins assert [`STARTED`] on their own
    /// thread.
    static CHECK_START: Cell<bool> = const { Cell::new(false) };
    /// Set by the tests' `start` on the thread it runs on.
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

impl<'a> Joiner<'a> {
    /// The join over `hashes` (every picture, in file-id order) where picture `i` owes its pairs when `owed[i]`.
    /// The store is permuted on the caller's thread.
    pub fn new(hashes: &[[Hash; ORIENTATIONS]], owed: &'a [bool]) -> Result<Joiner<'a>, TooManyPictures> {
        Joiner::build(hashes, owed, None)
    }

    /// [`Joiner::new`] with the store permuted on `threads` threads of its own (at least one), each started by `start`
    /// (the pass's priority): each permutes an equal run of the pictures into its own part of the store. The caller's
    /// thread permutes nothing, so all of it runs at `start`'s priority, and the caller's is never changed.
    pub fn new_on(
        hashes: &[[Hash; ORIENTATIONS]],
        owed: &'a [bool],
        threads: usize,
        start: &(dyn Fn() + Sync),
    ) -> Result<Joiner<'a>, TooManyPictures> {
        Joiner::build(hashes, owed, Some((threads, start)))
    }

    /// The store permuted on the caller's thread (`team` none), or on a team's threads.
    fn build(
        hashes: &[[Hash; ORIENTATIONS]],
        owed: &'a [bool],
        team: Option<(usize, &(dyn Fn() + Sync))>,
    ) -> Result<Joiner<'a>, TooManyPictures> {
        if !u32::try_from(hashes.len()).is_ok_and(|n| n <= MAX_PICTURES) {
            return Err(TooManyPictures(hashes.len()));
        }
        let mut stored = vec![[0u64; 4]; ORIENTATIONS * hashes.len()];
        #[cfg(test)]
        let (checks_start, on_team) = (CHECK_START.get(), team.is_some());
        let permute = |from: &[[Hash; ORIENTATIONS]], to: &mut [[[u64; 4]; ORIENTATIONS]]| {
            #[cfg(test)]
            assert!(
                !(checks_start && on_team) || STARTED.get(),
                "the store permuted on a thread that did not run start"
            );
            for (h, s) in from.iter().zip(to) {
                *s = h.each_ref().map(permuted);
            }
        };
        let (store, _) = stored.as_chunks_mut::<ORIENTATIONS>();
        match team {
            None => permute(hashes, store),
            Some((threads, start)) => {
                let run = hashes.len().div_ceil(threads.max(1)).max(1);
                std::thread::scope(|scope| {
                    for (from, to) in hashes.chunks(run).zip(store.chunks_mut(run)) {
                        scope.spawn(move || {
                            start();
                            permute(from, to);
                        });
                    }
                });
            }
        }
        Ok(Joiner {
            owed,
            stored,
            kernel: best_kernel(),
            tuning: Tuning::default(),
            #[cfg(test)]
            fault: None,
            #[cfg(test)]
            checks_start,
        })
    }

    /// The query buckets a scan task takes ([`Tuning::block`]).
    fn block(&self) -> usize {
        self.tuning.block.unwrap_or(BLOCK).clamp(1, VALUES)
    }

    /// Stage `c`'s head words ([`Tuning::widths`]).
    fn width(&self, c: usize) -> usize {
        let widths = self.tuning.widths.unwrap_or(WIDTHS);
        widths.get(c).copied().unwrap_or(MAX_HEAD_WORDS).clamp(1, MAX_HEAD_WORDS)
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

    /// The same join, panicking in its `nth` task of `kind` (the tests' injected fault).
    #[cfg(test)]
    fn with_fault(mut self, kind: Kind, nth: usize) -> Joiner<'a> {
        self.fault = Some((kind, AtomicUsize::new(nth)));
        self
    }

    fn owes(&self, i: usize) -> bool {
        self.owed.get(i).copied().unwrap_or(false)
    }

    /// Picture `i`'s hashes as they were given to [`Joiner::new`] (the store's permutation undone), `None` past the
    /// last: the join keeps the only copy, so the caller can drop its own once the join is made.
    pub fn original(&self, i: usize) -> Option<[Hash; ORIENTATIONS]> {
        let stored = self.stored.get(ORIENTATIONS * i..ORIENTATIONS * (i + 1))?;
        let stored = <[[u64; 4]; ORIENTATIONS]>::try_from(stored).ok()?;
        Some(stored.map(|words| unpermuted(&words)))
    }

    /// The pairs in which the query side is one of `queries` (a range of indices; the owed ones in it query).
    ///
    /// On a team of `threads` threads (at least one), which do all of the join's work but finding the slice's owed
    /// pictures (`run_team`). `start` runs once in each thread before it works (the caller sets its priority there).
    /// `control` is asked before each task, returns false to stop, and may block (a pause). `abort` is called when a
    /// thread panics, after the team is told to stop: it should wake a `control` that is blocked, and make it return
    /// false. `None` when `control` stopped the join: nothing is returned then. Not inlined, so a profiler can toggle on
    /// it.
    #[inline(never)]
    pub fn join(
        &self,
        queries: Range<usize>,
        threads: usize,
        start: &(dyn Fn() + Sync),
        control: &(dyn Fn() -> bool + Sync),
        abort: &(dyn Fn() + Sync),
    ) -> Option<Joined> {
        let threads = threads.max(1);
        let preparing = Instant::now();
        let pictures: Vec<u32> = queries.filter(|&i| self.owes(i)).filter_map(|i| u32::try_from(i).ok()).collect();
        let sides = Sides { joiner: self, pictures: &pictures };
        let prepare = preparing.elapsed();
        let queries = pictures.len() * QUERY_ORIENTATIONS;
        let (pairs, spent) = if queries <= self.tuning.few_entries_on(threads) {
            let work =
                |task: FewTask, out: &mut Vec<Pair>, _: &mut ReadBuffers| self.few_task(&sides, queries, task, out);
            run_team(threads, start, control, abort, FewSchedule::new(self.stored.len()), &work)?
        } else {
            let widths = std::array::from_fn(|c| self.width(c));
            let schedule = SortedSchedule::new(widths, [self.stored.len(), queries], VALUES.div_ceil(self.block()));
            let work = |task: SortedTask, out: &mut Vec<Pair>, read: &mut ReadBuffers| {
                self.sorted_task(&sides, task, out, read)
            };
            run_team(threads, start, control, abort, schedule, &work)?
        };
        let times =
            JoinTimes { prepare, sort: spent.sort, scan: spent.scan, starved: spent.starved, paused: spent.paused };
        Some(Joined { pairs, times })
    }

    /// One task of the sorted path: a pass of a side's sort, or a block of a stage's scan.
    fn sorted_task(
        &self,
        sides: &Sides<'_, '_>,
        task: SortedTask,
        out: &mut Vec<Pair>,
        read: &mut ReadBuffers,
    ) -> (Kind, SortedReport) {
        match task {
            SortedTask::First { side, b, mut block, entries, c, w } => {
                self.fault(Kind::Sort);
                if side == STORED {
                    first_pass(&mut block, read, entries, &|e| sides.stored_chunks(e), c, w);
                } else {
                    first_pass(&mut block, read, entries, &|e| sides.query_chunks(e), c, w);
                }
                (Kind::Sort, SortedReport::First { side, b, block })
            }
            SortedTask::Second { side, h, mut region, blocks, count, w } => {
                self.fault(Kind::Sort);
                let layout = if side == STORED { Layout::Rows } else { Layout::Columns };
                second_pass(&mut region, blocks.get(..count).unwrap_or_default(), h, w, layout);
                (Kind::Sort, SortedReport::Second { side, h, region })
            }
            SortedTask::Scan { sorts, block } => {
                self.fault(Kind::Scan);
                let Some([stored, query]) = sorts.sides.get(..2).and_then(|s| <&[Sorted; 2]>::try_from(s).ok()) else {
                    return (Kind::Scan, SortedReport::Scan);
                };
                let stage = Stage::new(sorts.c, sorts.w, stored, query, sides);
                let lo = block * self.block();
                scan(self.kernel, &stage, lo..(lo + self.block()).min(VALUES), out);
                (Kind::Scan, SortedReport::Scan)
            }
        }
    }

    /// One task of the few-owed path: the query side's table, a chunk's lists, or a block of the pass over the store.
    fn few_task(&self, sides: &Sides<'_, '_>, queries: usize, task: FewTask, out: &mut Vec<Pair>) -> (Kind, FewReport) {
        match task {
            FewTask::Table => {
                self.fault(Kind::Sort);
                let (words, codes) = (0..queries).map(|e| sides.query_entry(e)).unzip();
                (Kind::Sort, FewReport::Table(FewTable { codes, words }))
            }
            FewTask::Chunk { c, table } => {
                self.fault(Kind::Sort);
                (Kind::Sort, FewReport::Chunk { c, chunk: FewChunk::new(&table.words, c) })
            }
            FewTask::Scan { few, block } => {
                self.fault(Kind::Scan);
                let lo = block * FEW_BLOCK;
                for x in lo..(lo + FEW_BLOCK).min(self.stored.len()) {
                    few_entry(sides, &few, x, out);
                }
                (Kind::Scan, FewReport::Scan)
            }
        }
    }

    /// The tests' injected panic: at the task of `kind` they name, counted over the join. Every task begins here, so
    /// it also checks that its thread ran `start` (`checks_start`).
    #[cfg(test)]
    fn fault(&self, kind: Kind) {
        assert!(!self.checks_start || STARTED.get(), "a {kind:?} task on a thread that did not run start");
        if let Some((at, nth)) = &self.fault
            && *at == kind
            && nth.fetch_sub(1, Relaxed) == 1
        {
            panic!("the injected fault, in a {kind:?} task");
        }
    }

    #[cfg(not(test))]
    #[inline(always)]
    fn fault(&self, _: Kind) {}
}

/// The few-owed path's query side: its entries' codes and hashes, in entry order (`codes[j]`, `words[j]`).
struct FewTable {
    codes: Vec<u32>,
    words: Vec<[u64; 4]>,
}

/// The few-owed path's query side as its pass reads it: the table, and each chunk's lists of its entries ([`FewChunk`]).
struct Few {
    table: Arc<FewTable>,
    chunks: Vec<FewChunk>,
}

/// One chunk's query entries, each under its own value and that value's 16 one-bit neighbours, so a stored entry whose
/// value is `v` meets exactly the query entries its 17 probes would (`entries[offsets[v]..offsets[v + 1]]`, indices
/// into [`FewTable`]): the value `u` is in `v`'s probes exactly when `v` is in `u`'s, and a value's 17 probes are
/// distinct, so each candidate is listed once. `bits` marks the values with entries: 8 KB a chunk, 128 KB for all 16,
/// which stay in L2 while the pass streams the store; the offsets are read only on a hit.
struct FewChunk {
    bits: Vec<u64>,
    offsets: Vec<u32>,
    entries: Vec<u32>,
}

/// The probes of a value: itself and its 16 one-bit neighbours.
const PROBES: usize = 1 + 16;

impl FewChunk {
    /// Chunk `c` of the query entries `words`, by a counting sort of their (probe value, entry) pairs: each value's
    /// count, then its end, then the entries placed from the end down (so in entry order, the last first), which leaves
    /// each value's offset at its start.
    fn new(words: &[[u64; 4]], c: usize) -> FewChunk {
        let mut offsets = vec![0u32; VALUES + 1];
        for w in words {
            let v = chunk(w, c);
            for p in 0..PROBES {
                if let Some(count) = offsets.get_mut(probe(v, p)) {
                    *count += 1;
                }
            }
        }
        let mut bits = vec![0u64; VALUES / 64];
        let mut end = 0u32;
        for (v, slot) in offsets.iter_mut().enumerate() {
            if *slot > 0
                && let Some(word) = bits.get_mut(v >> 6)
            {
                *word |= 1 << (v & 63);
            }
            end += *slot;
            *slot = end;
        }
        let mut entries = vec![0u32; index(end)];
        let n = u32::try_from(words.len()).unwrap_or(u32::MAX);
        for (j, w) in (0..n).zip(words).rev() {
            let v = chunk(w, c);
            for p in 0..PROBES {
                if let Some(cursor) = offsets.get_mut(probe(v, p)) {
                    *cursor -= 1;
                    if let Some(entry) = entries.get_mut(index(*cursor)) {
                        *entry = j;
                    }
                }
            }
        }
        FewChunk { bits, offsets, entries }
    }

    /// The query entries a stored entry of value `v` meets, if any.
    #[inline(always)]
    fn met(&self, v: usize) -> &[u32] {
        if self.bits.get(v >> 6).is_some_and(|word| (word >> (v & 63)) & 1 != 0) {
            let at = |v: usize| index(self.offsets.get(v).copied().unwrap_or(0));
            self.entries.get(at(v)..at(v + 1)).unwrap_or_default()
        } else {
            &[]
        }
    }
}

/// The few-owed path's work for stored entry `x`: in each chunk, the query entries its value meets ([`FewChunk`]),
/// each tested in full on the table's hash, then by the ordering rule and the first chunk that matches.
#[inline(always)]
fn few_entry(sides: &Sides<'_, '_>, few: &Few, x: usize, out: &mut Vec<Pair>) {
    let (xw, xc) = sides.stored_entry(x);
    for (c, chunk_of) in few.chunks.iter().enumerate() {
        for &j in chunk_of.met(chunk(&xw, c)) {
            let (Some(&yc), Some(yw)) = (few.table.codes.get(index(j)), few.table.words.get(index(j))) else {
                continue;
            };
            let distance: u32 = xw.iter().zip(yw).map(|(a, b)| (a ^ b).count_ones()).sum();
            if distance <= MATCH_DISTANCE && owed(xc, yc) && !matched_earlier(&xw, yw, c) {
                let (xp, yp) = (xc >> 4, yc >> 4);
                out.push(Pair { a: xp.min(yp), b: xp.max(yp), distance });
            }
        }
    }
}

/// The candidate stored entry `xc` (hash `xw`), query entry `yc` at stage `c`, past the head test: the pair recorded if
/// it is within the radius and first found in this chunk.
#[inline(always)]
fn report(sides: &Sides<'_, '_>, c: usize, xc: u32, xw: &[u64; 4], yc: u32, out: &mut Vec<Pair>) {
    let yw = sides.words(yc);
    let distance: u32 = xw.iter().zip(&yw).map(|(a, b)| (a ^ b).count_ones()).sum();
    if distance <= MATCH_DISTANCE && !matched_earlier(xw, &yw, c) {
        let (xp, yp) = (xc >> 4, yc >> 4);
        out.push(Pair { a: xp.min(yp), b: xp.max(yp), distance });
    }
}

/// What one slice's join reads the entries' hashes from: the stored orientations of every picture, and the slice's
/// query pictures (`pictures`, in order), whose transposed orientations are renamings of their stored ones
/// ([`transposed`]), made where they are read.
struct Sides<'a, 'j> {
    joiner: &'a Joiner<'j>,
    pictures: &'a [u32],
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
        let code = (picture << 4) | OWED | u32::try_from(o).unwrap_or(0);
        (self.words(code), code)
    }

    /// Stored entry `e` as its chunks ([`chunks`]), with its code: what the sort reads.
    #[inline(always)]
    fn stored_chunks(&self, e: usize) -> ([u16; CHUNKS], u32) {
        let (words, code) = self.stored_entry(e);
        (chunks(&words), code)
    }

    /// Query entry `e` as its chunks, with its code: a transposed orientation's renamed from the stored one's
    /// ([`transposed_chunks`]), as the sort reads it, with no hash built.
    #[inline(always)]
    fn query_chunks(&self, e: usize) -> ([u16; CHUNKS], u32) {
        let (k, o) = (e / QUERY_ORIENTATIONS, e % QUERY_ORIENTATIONS);
        let picture = self.pictures.get(k).copied().unwrap_or(0);
        let code = (picture << 4) | OWED | u32::try_from(o).unwrap_or(0);
        let words =
            self.joiner.stored.get(index(picture) * ORIENTATIONS + o % ORIENTATIONS).copied().unwrap_or_default();
        let own = chunks(&words);
        (if o < ORIENTATIONS { own } else { transposed_chunks(&own) }, code)
    }

    /// The full permuted hash of the entry with `code`.
    #[inline(always)]
    fn words(&self, code: u32) -> [u64; 4] {
        let (picture, o) = (index(code >> 4), index(code & 7));
        let words = self.joiner.stored.get(picture * ORIENTATIONS + o % ORIENTATIONS).copied().unwrap_or_default();
        if o < ORIENTATIONS { words } else { transposed(&words) }
    }
}

/// How a region holds its entries' heads ([`head_words`], `w` words each).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// Entry `j`'s words at `head[j * w..][..w]`: the stored side, whose kernels read one entry's head at a time.
    Rows,
    /// Word `k` of entry `j` at `head[k * stride + j]`, `stride` the region's entries and [`LANES`] more: the query
    /// side, whose kernels read eight entries' word `k` at a time, from any entry of the region.
    Columns,
}

/// The entries of one stage side whose chunk has one high byte: entry `j`'s head (by the side's [`Layout`]) and its
/// code `code[j]` (with [`CODE_PADDING`] more values, for [`suffix_avx2`]'s reads); `offsets[l]..offsets[l + 1]` are
/// the entries whose chunk's low byte is `l`, in the order the entries were given (the sort is stable). The padding
/// holds any value: what reads it stops at the bucket's end, or drops the lanes past it.
#[derive(Default)]
struct Region {
    offsets: Vec<u32>,
    head: Vec<u32>,
    code: Vec<u32>,
    /// The head's step from one word to the next: `w` in rows, a column's length in columns.
    stride: usize,
}

/// The region of a side that has none yet (a set not sorted): every bucket empty.
static NO_REGION: Region = Region { offsets: Vec::new(), head: Vec::new(), code: Vec::new(), stride: 0 };

/// One side of a stage, sorted by its chunk: the region of each high byte (`regions[v >> 8]` holds chunk value `v`).
/// Each region's arrays are its own, so a region is filled, kept and reused on its own.
#[derive(Default)]
struct Sorted {
    regions: Vec<Region>,
}

impl Sorted {
    /// The region of high byte `h`.
    #[inline(always)]
    fn region(&self, h: usize) -> &Region {
        self.regions.get(h).unwrap_or(&NO_REGION)
    }

    /// The bucket of chunk value `v`: its region, and its entries' positions in it.
    #[cfg(test)]
    fn bucket(&self, v: usize) -> (&Region, usize, usize) {
        let (lo, hi) = View::of(self.region(v >> 8)).bucket(v & 0xFF);
        (self.region(v >> 8), lo, hi)
    }
}

/// A region's arrays as slices: what the kernels read, resolved once for the buckets of one high byte.
#[derive(Clone, Copy)]
struct View<'a> {
    offsets: &'a [u32],
    head: &'a [u32],
    code: &'a [u32],
    stride: usize,
}

impl<'a> View<'a> {
    const EMPTY: View<'static> = View { offsets: &[], head: &[], code: &[], stride: 0 };

    fn of(region: &'a Region) -> View<'a> {
        View { offsets: &region.offsets, head: &region.head, code: &region.code, stride: region.stride }
    }

    /// The entries of low byte `l`.
    #[inline(always)]
    fn bucket(&self, l: usize) -> (usize, usize) {
        let at = |l: usize| index(self.offsets.get(l).copied().unwrap_or(0));
        (at(l), at(l + 1))
    }
}

/// The 17 probes of a query bucket `v = h << 8 | l` ([`probe`]), by the stored region each is in: probe 0 and probes
/// 1 - 8 (a low bit flipped) are in region `h`, at the low bytes [`low_probes`] gives; probe `9 + k` is in region
/// `h ^ 1 << k`, at low byte `l`. The kernels walk them in that order, which is [`probe`]'s.
struct Probed<'a> {
    own: View<'a>,
    others: [View<'a>; 8],
}

impl<'a> Probed<'a> {
    const EMPTY: Probed<'static> = Probed { own: View::EMPTY, others: [View::EMPTY; 8] };

    fn of(stored: &'a Sorted, h: usize) -> Probed<'a> {
        Probed {
            own: View::of(stored.region(h)),
            others: std::array::from_fn(|k| View::of(stored.region(h ^ (1 << k)))),
        }
    }
}

/// The low bytes of probes 0 - 8 of a query bucket with low byte `l`: `l`, then `l` with each bit flipped.
#[inline(always)]
fn low_probes(l: usize) -> [usize; 9] {
    std::array::from_fn(|p| if p == 0 { l } else { l ^ (1 << (p - 1)) })
}

/// A core's L2 cache when the CPU does not report it: the i7-8700's, 256 KiB.
const L2_FALLBACK: usize = 256 * 1024;

/// A core's L2 cache, as the CPU reports it, or the fallback. Intel and AMD (from Zen) describe their caches in the same
/// form, CPUID leaf 4 and leaf 0x8000_001D: per subleaf a cache's type (EAX bits 4 - 0, 1 data, 3 unified, 0 the end),
/// level (bits 7 - 5) and size, ways x partitions x line x sets (EBX bits 31 - 22, 21 - 12, 11 - 0 and ECX, each less
/// one). Read once.
fn l2_cache() -> usize {
    static L2: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *L2.get_or_init(|| {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{__cpuid, __cpuid_count};
            let vendor = __cpuid(0);
            // "AuthenticAMD" in EBX, EDX, ECX: its leaf is 0x8000_001D, if the extended leaves reach it.
            let leaf = if vendor.ebx == 0x6874_7541 { 0x8000_001D } else { 4 };
            let top = if leaf == 4 { vendor.eax } else { __cpuid(0x8000_0000).eax };
            if top >= leaf {
                for sub in 0..16 {
                    let r = __cpuid_count(leaf, sub);
                    let kind = r.eax & 0x1F;
                    if kind == 0 {
                        break;
                    }
                    if (r.eax >> 5) & 0x7 == 2 && matches!(kind, 1 | 3) {
                        return index((r.ebx >> 22) + 1)
                            * index(((r.ebx >> 12) & 0x3FF) + 1)
                            * index((r.ebx & 0xFFF) + 1)
                            * index(r.ecx.saturating_add(1));
                    }
                }
            }
        }
        L2_FALLBACK
    })
}

/// The entries of one block of the sort's first pass at `w` head words: the block's records (`w` words, the code and
/// the `u16` key) as read and as scattered, both in L2.
fn sort_block(w: usize) -> usize {
    (l2_cache() / (2 * (4 * (w + 1) + 2))).max(1)
}

/// One block of a side's first pass: its records scattered by their chunk's high byte (`run[h]..run[h + 1]` are those
/// of high byte `h`, in the order read), each its head (`w` words), code and chunk value. Reused from stage to stage.
#[derive(Default)]
struct Block {
    heads: Vec<u32>,
    codes: Vec<u32>,
    keys: Vec<u16>,
    run: Vec<u32>,
}

/// A thread's buffers for the first pass: one block's records as read, before they are scattered (in L2 with the block).
#[derive(Default)]
struct ReadBuffers {
    heads: Vec<u32>,
    codes: Vec<u32>,
    keys: Vec<u16>,
}

/// The first pass of the sort of a side by its chunk `c`, over one block of its entries (`entry(entries)`, at most
/// [`sort_block`]): each entry's key and head of `w` words ([`head_words`]) read into `read`, then scattered into
/// `block` by the key's high byte. The block's records are written whole, so its buffers need no clearing.
/// Monomorphized by the head words, as the kernels are, so a head is copied as a fixed-size array.
fn first_pass(
    block: &mut Block,
    read: &mut ReadBuffers,
    entries: Range<usize>,
    entry: &impl Fn(usize) -> ([u16; CHUNKS], u32),
    c: usize,
    w: usize,
) {
    match w {
        1 => first_pass_width::<1>(block, read, entries, entry, c),
        2 => first_pass_width::<2>(block, read, entries, entry, c),
        3 => first_pass_width::<3>(block, read, entries, entry, c),
        4 => first_pass_width::<4>(block, read, entries, entry, c),
        5 => first_pass_width::<5>(block, read, entries, entry, c),
        6 => first_pass_width::<6>(block, read, entries, entry, c),
        _ => first_pass_width::<MAX_HEAD_WORDS>(block, read, entries, entry, c),
    }
}

/// The second pass into region `h`: the records of high byte `h` from each of the first pass's `blocks`, in block
/// order (so the sort is stable), placed by the key's low byte, with heads of `w` words in `layout`. Every slot an
/// entry needs is written, and the padding may hold anything, so the region's buffers need no clearing.
fn second_pass(region: &mut Region, blocks: &[Block], h: usize, w: usize, layout: Layout) {
    match w {
        1 => second_pass_width::<1>(region, blocks, h, layout),
        2 => second_pass_width::<2>(region, blocks, h, layout),
        3 => second_pass_width::<3>(region, blocks, h, layout),
        4 => second_pass_width::<4>(region, blocks, h, layout),
        5 => second_pass_width::<5>(region, blocks, h, layout),
        6 => second_pass_width::<6>(region, blocks, h, layout),
        _ => second_pass_width::<MAX_HEAD_WORDS>(region, blocks, h, layout),
    }
}

/// The head of a permuted hash at stage `c`, `W` words, from its 16 chunks ([`chunks`]): chunks `c + 1, c + 2, ...`
/// (mod 16), two to a word (the first in the low half). The kernels test it against the stage's [`bound`].
#[inline(always)]
fn head_words<const W: usize>(chunks: &[u16; CHUNKS], c: usize) -> [u32; W] {
    let at = |i: usize| u32::from(chunks.get(i % CHUNKS).copied().unwrap_or(0));
    std::array::from_fn(|j| at(c + 1 + 2 * j) | (at(c + 2 + 2 * j) << 16))
}

/// [`first_pass`] at `W` head words.
#[expect(
    clippy::indexing_slicing,
    reason = "a byte is below 256 (the counts' length, the run's less one); each position is a running sum of the \
              counts, below the block's records, which the arrays are sized to"
)]
fn first_pass_width<const W: usize>(
    block: &mut Block,
    read: &mut ReadBuffers,
    entries: Range<usize>,
    entry: &impl Fn(usize) -> ([u16; CHUNKS], u32),
    c: usize,
) {
    let len = entries.len();
    read.heads.resize(len * W, 0);
    read.codes.resize(len, 0);
    read.keys.resize(len, 0);
    // Each buffer as its own slice, so a store through one does not make the loop read the others' bounds again.
    let (read_heads, read_codes, read_keys) = (&mut read.heads[..], &mut read.codes[..], &mut read.keys[..]);
    let mut counts = [0u32; 256];
    for (((head, code), key), e) in
        read_heads.as_chunks_mut::<W>().0.iter_mut().zip(read_codes.iter_mut()).zip(read_keys.iter_mut()).zip(entries)
    {
        let (chunks, entry_code) = entry(e);
        let value = chunks.get(c).copied().unwrap_or(0);
        counts[usize::from(value >> 8)] += 1;
        *head = head_words::<W>(&chunks, c);
        *code = entry_code;
        *key = value;
    }
    block.run.resize(257, 0);
    let mut at = 0u32;
    for (start, count) in block.run.iter_mut().zip(&mut counts) {
        (*start, at) = (at, at + *count);
        *count = *start;
    }
    block.run[256] = at;
    block.heads.resize(len * W, 0);
    block.codes.resize(len, 0);
    block.keys.resize(len, 0);
    let (heads, codes, keys) = (&mut block.heads[..], &mut block.codes[..], &mut block.keys[..]);
    // `counts` now holds each high byte's next position.
    for ((head, &code), &key) in read_heads.as_chunks::<W>().0.iter().zip(read_codes.iter()).zip(read_keys.iter()) {
        let slot = &mut counts[usize::from(key >> 8)];
        let j = index(*slot);
        heads[j * W..(j + 1) * W].copy_from_slice(head);
        codes[j] = code;
        keys[j] = key;
        *slot += 1;
    }
}

/// [`second_pass`] at `W` head words.
#[expect(
    clippy::indexing_slicing,
    reason = "a byte is below 256 (the counts' length; the run's and offsets' less one); each position is a running sum \
              of the counts, below the region's entries, which the arrays are sized to; a block's runs are within it"
)]
fn second_pass_width<const W: usize>(region: &mut Region, blocks: &[Block], h: usize, layout: Layout) {
    let mut counts = [0u32; 256];
    for block in blocks {
        for &key in &block.keys[index(block.run[h])..index(block.run[h + 1])] {
            counts[usize::from(key & 0xFF)] += 1;
        }
    }
    region.offsets.resize(257, 0);
    let mut at = 0u32;
    for (l, count) in counts.iter_mut().enumerate() {
        region.offsets[l] = at;
        (*count, at) = (at, at + *count);
    }
    region.offsets[256] = at;
    let len = index(at);
    region.code.resize(len + CODE_PADDING, 0);
    let stride = match layout {
        Layout::Rows => W,
        Layout::Columns => len + LANES,
    };
    region.stride = stride;
    region.head.resize(W * if layout == Layout::Rows { len } else { stride }, 0);
    // The region's buffers as their own slices, and block by block each run's slices taken once, so a store does not
    // make the loop read any buffer's bounds again.
    let (out_head, out_code) = (&mut region.head[..], &mut region.code[..]);
    for block in blocks {
        let run = index(block.run[h])..index(block.run[h + 1]);
        let keys = &block.keys[run.clone()];
        let codes = &block.codes[run.clone()];
        let (heads, _) = block.heads[run.start * W..run.end * W].as_chunks::<W>();
        for ((&key, &code), head) in keys.iter().zip(codes).zip(heads) {
            let slot = &mut counts[usize::from(key & 0xFF)];
            let j = index(*slot);
            match layout {
                Layout::Rows => out_head[j * W..(j + 1) * W].copy_from_slice(head),
                Layout::Columns => {
                    for (k, &word) in head.iter().enumerate() {
                        out_head[k * stride + j] = word;
                    }
                }
            }
            out_code[j] = code;
            *slot += 1;
        }
    }
}

/// The `n` entries `entry(0..n)` sorted by their chunk `c` into `out` as the team sorts a side, on one thread: the first
/// pass by blocks of [`sort_block`] entries, then the second by regions.
#[cfg(test)]
fn sort_into(
    out: &mut Sorted,
    n: usize,
    entry: impl Fn(usize) -> ([u16; CHUNKS], u32),
    c: usize,
    w: usize,
    layout: Layout,
) {
    let size = sort_block(w);
    let mut read = ReadBuffers::default();
    let blocks: Vec<Block> = (0..n.div_ceil(size))
        .map(|b| {
            let mut block = Block::default();
            first_pass(&mut block, &mut read, b * size..((b + 1) * size).min(n), &entry, c, w);
            block
        })
        .collect();
    out.regions.resize_with(REGIONS, Region::default);
    for (h, region) in out.regions.iter_mut().enumerate() {
        second_pass(region, &blocks, h, w, layout);
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

/// One chunk's stage: the stored and query sides sorted by it, its entries' head words and the head test's bounds, and
/// where the full hashes are.
struct Stage<'a, 's, 'j> {
    c: usize,
    w: usize,
    /// The bound of probe 0, and of the probes that flip a bit ([`bound`]).
    bounds: [u32; 2],
    stored: &'a Sorted,
    query: &'a Sorted,
    sides: &'a Sides<'s, 'j>,
}

impl<'a, 's, 'j> Stage<'a, 's, 'j> {
    /// Stage `c`, its sides `stored` and `query` sorted with `w` head words.
    fn new(c: usize, w: usize, stored: &'a Sorted, query: &'a Sorted, sides: &'a Sides<'s, 'j>) -> Stage<'a, 's, 'j> {
        Stage { c, w, bounds: bounds(c, w), stored, query, sides }
    }
}

/// Stored entry `xc` against query entry `yc`, past the head test: [`report`]ed if the stored one is compared with it.
#[inline(always)]
fn consider(st: &Stage<'_, '_, '_>, xc: u32, yc: u32, out: &mut Vec<Pair>) {
    if owed(xc, yc) {
        report(st.sides, st.c, xc, &st.sides.words(xc), yc, out);
    }
}

/// Where the queries of the stored entry with code `xc` start in the query bucket `[lo, hi)` of `codes`: the whole
/// bucket when it owes nothing, else past the entries of pictures up to its own (`from`, the probe's pointer, moves
/// forward only: the stored entries come in picture order).
#[inline(always)]
fn suffix(xc: u32, codes: &[u32], lo: usize, hi: usize, from: &mut usize) -> usize {
    if xc & OWED == 0 {
        return lo;
    }
    let picture = xc >> 4;
    while *from < hi && codes.get(*from).is_some_and(|&yc| yc >> 4 <= picture) {
        *from += 1;
    }
    *from
}

/// [`suffix`] without its data-dependent walk: eight codes compared at a time, the pointer moved past those of pictures
/// up to `xc`'s (a bucket is in picture order, so they lead). The loop repeats only when all eight do, and the pointer
/// moves at most a bucket per probe, so its branch is almost always predicted. The codes past `hi` (the next bucket's,
/// or the padding) can only make the step longer, and the step is cut at `hi`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
fn suffix_avx2(xc: u32, codes: &[u32], lo: usize, hi: usize, from: &mut usize) -> usize {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_ps, _mm256_cmpgt_epi32, _mm256_loadu_si256, _mm256_movemask_ps, _mm256_set1_epi32,
        _mm256_srli_epi32,
    };
    if xc & OWED == 0 {
        return lo;
    }
    // A picture is below MAX_PICTURES (28 bits), so it and every shifted code are non-negative as an i32.
    let picture = _mm256_set1_epi32(i32::try_from(xc >> 4).unwrap_or(i32::MAX));
    loop {
        let Some(eight) = codes.get(*from..*from + CODE_PADDING) else {
            // Not reached: `from <= hi <= n`, and the codes hold n + CODE_PADDING values.
            return suffix(xc, codes, lo, hi, from);
        };
        // SAFETY: `eight` is CODE_PADDING (8) u32s, the 32 bytes the unaligned load reads.
        let eight = unsafe { _mm256_loadu_si256(eight.as_ptr().cast::<__m256i>()) };
        let later = _mm256_cmpgt_epi32(_mm256_srli_epi32::<4>(eight), picture);
        let later = u32::try_from(_mm256_movemask_ps(_mm256_castsi256_ps(later))).unwrap_or(0);
        let step = index((later | (1 << CODE_PADDING)).trailing_zeros());
        *from = (*from + step).min(hi);
        if step < CODE_PADDING || *from == hi {
            return *from;
        }
    }
}

/// The probe of query bucket `v`'s `p`th (0: the same value; 1..17: the value with bit `p - 1` flipped).
#[inline(always)]
fn probe(v: usize, p: usize) -> usize {
    if p == 0 { v } else { v ^ (1 << (p - 1)) }
}

/// One run of a query bucket's probes: stored buckets of one region (`sr`, its heads as `rows`), tested against the
/// query bucket `[qlo, qhi)` of `qr` at one bound. A bucket has ten: probe 0; probes 1 - 8, which flip a low bit, in
/// its own region; and each of probes 9 - 16 in a region of its own. With them, the query region's `W` columns of
/// heads ([`Columns`]); the regions are resolved once for the buckets of one high byte.
struct Run<'v, 'a, const W: usize> {
    sr: &'v View<'a>,
    rows: &'a [[u32; W]],
    qr: &'v View<'a>,
    columns: &'v Columns<'a, W>,
    qlo: usize,
    qhi: usize,
    bound: u32,
}

/// A query region's heads as its `W` columns (word `k` of entry `j` at `columns[k][j]`, each the region's entries and
/// [`LANES`] more), and the offsets a load of eight may start at, `0..starts` (none when the region holds no heads).
struct Columns<'a, const W: usize> {
    columns: [&'a [u32]; W],
    starts: usize,
}

impl<'a, const W: usize> Columns<'a, W> {
    const EMPTY: Columns<'static, W> = Columns { columns: [&[]; W], starts: 0 };

    fn of(qr: &View<'a>) -> Columns<'a, W> {
        let stride = qr.stride;
        let columns: [&[u32]; W] = std::array::from_fn(|k| qr.head.get(k * stride..(k + 1) * stride).unwrap_or(&[]));
        let whole = columns.iter().all(|column| column.len() == stride);
        Columns { columns, starts: if whole { (stride + 1).saturating_sub(LANES) } else { 0 } }
    }
}

/// The buckets `v` of the query side, each probed by its ten runs ([`Run`]), in one of two ways by the bucket's size.
///
/// - **More than eight entries:** each stored entry of a run takes its suffix of the query bucket ([`suffix`], by the
///   kernel's `suffix`), and `test` tests it eight entries at a time.
/// - **At most eight** (one load of the columns): `load` reads the bucket's heads and codes once, and `direct` tests
///   each stored entry against them, the ordering rule as a mask of the lanes rather than a suffix. Each stored entry is
///   then one pass through straight-line code.
///
/// The kernels' parts are closures, so that they are compiled with the kernel's target features ([`scan_avx2`]).
#[inline(always)]
fn scan_runs<const W: usize, B>(
    st: &Stage<'_, '_, '_>,
    buckets: Range<usize>,
    out: &mut Vec<Pair>,
    suffix: impl Fn(u32, &[u32], usize, usize, &mut usize) -> usize,
    test: impl Fn(&Run<'_, '_, W>, usize, usize, &mut Vec<Pair>),
    load: impl Fn(&Columns<'_, W>, &[u32], usize, usize) -> B,
    direct: impl Fn(&Run<'_, '_, W>, &B, usize, u32, &mut Vec<Pair>),
) {
    let [bound0, bound1] = st.bounds;
    let (mut h, mut qr, mut probed, mut columns) = (usize::MAX, View::EMPTY, Probed::EMPTY, Columns::EMPTY);
    let (mut own_rows, mut other_rows): (&[[u32; W]], [&[[u32; W]]; 8]) = (&[], [&[]; 8]);
    for v in buckets {
        // The regions this bucket and its probes are in, resolved when the high byte changes: once per block.
        if v >> 8 != h {
            h = v >> 8;
            qr = View::of(st.query.region(h));
            probed = Probed::of(st.stored, h);
            columns = Columns::of(&qr);
            own_rows = probed.own.head.as_chunks::<W>().0;
            other_rows = probed.others.map(|sr| sr.head.as_chunks::<W>().0);
        }
        let l = v & 0xFF;
        let (qlo, qhi) = qr.bucket(l);
        if qlo == qhi {
            continue;
        }
        let lows = low_probes(l);
        let (own, flipped) = lows.split_at(1);
        // The runs as data, walked by one loop here: a closure would be compiled without the kernel's target features
        // (a closure has its defining function's), and so would the kernel's parts, called from it, not inlined.
        let runs = [(&probed.own, own_rows, own, bound0), (&probed.own, own_rows, flipped, bound1)]
            .into_iter()
            .chain(probed.others.iter().zip(other_rows).map(|(sr, rows)| (sr, rows, own, bound1)));
        if qhi - qlo <= LANES {
            let bucket = load(&columns, qr.code, qlo, qhi);
            for (sr, rows, lows, bound) in runs {
                let r = Run { sr, rows, qr: &qr, columns: &columns, qlo, qhi, bound };
                for &u in lows {
                    let (slo, shi) = sr.bucket(u);
                    let Some(codes) = sr.code.get(slo..shi) else { continue };
                    for (x, &xc) in (slo..).zip(codes) {
                        direct(&r, &bucket, x, xc, out);
                    }
                }
            }
            continue;
        }
        for (sr, rows, lows, bound) in runs {
            let r = Run { sr, rows, qr: &qr, columns: &columns, qlo, qhi, bound };
            for &u in lows {
                let (slo, shi) = sr.bucket(u);
                let Some(codes) = sr.code.get(slo..shi) else { continue };
                let mut from = qlo;
                for (x, &xc) in (slo..).zip(codes) {
                    let s = suffix(xc, qr.code, qlo, qhi, &mut from);
                    test(&r, x, s, out);
                }
            }
        }
    }
}

/// The lanes before a bucket's end, of a slot `left` entries before it: `LANE_MASKS[left]`, all eight from 8 on.
const LANE_MASKS: [u32; LANES] = [0, 0x01, 0x03, 0x07, 0x0F, 0x1F, 0x3F, 0x7F];

/// The lanes of the slot at `off` before the bucket's end `qhi`, a bit each.
#[inline(always)]
fn lanes_before(qhi: usize, off: usize) -> u32 {
    LANE_MASKS.get(qhi.saturating_sub(off)).copied().unwrap_or(0xFF)
}

/// The lanes of stored entry `x` against the eight query entries from `off` past the head test (`lanes`, a bit each),
/// tested in full: those before the bucket's end (`r.qhi`), as the lanes past it are the next bucket's entries, or the
/// padding. Out of the kernels' loop: hits are rare, and inlined there they would take registers the loop needs.
#[cold]
#[inline(never)]
fn hits<const W: usize>(
    st: &Stage<'_, '_, '_>,
    r: &Run<'_, '_, W>,
    x: usize,
    off: usize,
    lanes: u32,
    out: &mut Vec<Pair>,
) {
    let Some(&xc) = r.sr.code.get(x) else { return };
    let mut lanes = lanes & lanes_before(r.qhi, off);
    while lanes != 0 {
        let lane = index(lanes.trailing_zeros());
        lanes &= lanes - 1;
        if let Some(&yc) = r.qr.code.get(off + lane) {
            consider(st, xc, yc, out);
        }
    }
}

/// The scalar kernel's test of stored entry `x` against the query entries `[s, qhi)`: one at a time, in eights.
#[inline(always)]
fn entry_scalar<const W: usize>(st: &Stage<'_, '_, '_>, r: &Run<'_, '_, W>, x: usize, s: usize, out: &mut Vec<Pair>) {
    let Some(xh) = r.rows.get(x) else { return };
    for off in (s..r.qhi).step_by(LANES) {
        let mut lanes = 0u32;
        for lane in 0..(r.qhi - off).min(LANES) {
            let y = off + lane;
            let distance: u32 = r
                .columns
                .columns
                .iter()
                .zip(xh)
                .map(|(column, &word)| column.get(y).map_or(u32::BITS, |&q| (q ^ word).count_ones()))
                .sum();
            if distance <= r.bound {
                lanes |= 1 << lane;
            }
        }
        if lanes != 0 {
            hits(st, r, x, off, lanes, out);
        }
    }
}

/// The scalar kernel over a block of query buckets, at `W` head words: each stored entry against its suffix, in a small
/// bucket as in a large one.
#[inline(always)]
fn scan_scalar<const W: usize>(st: &Stage<'_, '_, '_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    scan_runs::<W, ()>(
        st,
        buckets,
        out,
        suffix,
        |r, x, s, out| entry_scalar(st, r, x, s, out),
        |_, _, _, _| (),
        |r, _, x, xc, out| {
            let mut from = r.qlo;
            let s = suffix(xc, r.qr.code, r.qlo, r.qhi, &mut from);
            entry_scalar(st, r, x, s, out);
        },
    );
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "popcnt")]
unsafe fn scan_popcnt<const W: usize>(st: &Stage<'_, '_, '_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    scan_scalar::<W>(st, buckets, out);
}

/// The vector kernel: eight query entries at a time. Per head word, the stored entry's word broadcast against eight
/// query entries' (from the query side's column), the bits that differ counted by nibble lookups and added up per byte;
/// then the bytes summed per lane and compared with the run's bound, and only the lanes within it tested in full
/// ([`hits`]). A large bucket's words are loaded per eight entries of each stored entry's suffix, and its lanes past
/// the bucket's end dropped on a hit; a small bucket's are loaded once, with its pictures, and the ordering rule and
/// the bucket's end are a lane mask.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,popcnt")]
unsafe fn scan_avx2<const W: usize>(st: &Stage<'_, '_, '_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi8, _mm256_and_si256, _mm256_castsi256_ps, _mm256_cmpgt_epi32, _mm256_loadu_si256,
        _mm256_madd_epi16, _mm256_maddubs_epi16, _mm256_movemask_ps, _mm256_set1_epi8, _mm256_set1_epi16,
        _mm256_set1_epi32, _mm256_setr_epi8, _mm256_setzero_si256, _mm256_shuffle_epi8, _mm256_srli_epi16,
        _mm256_srli_epi32, _mm256_xor_si256,
    };
    let lut = _mm256_setr_epi8(
        0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4, 0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4,
    );
    let low = _mm256_set1_epi8(0x0f);
    let (ones8, ones16) = (_mm256_set1_epi8(1), _mm256_set1_epi16(1));
    let broadcast = |value: u32| _mm256_set1_epi32(i32::from_ne_bytes(value.to_ne_bytes()));
    // The bits of `q ^ word` counted per byte, added to `bytes`: at most 8 W (56) per byte, so no byte overflows.
    let add_word = |bytes: __m256i, q: __m256i, word: __m256i| -> __m256i {
        let v = _mm256_xor_si256(q, word);
        let lo = _mm256_and_si256(v, low);
        let hi = _mm256_and_si256(_mm256_srli_epi16(v, 4), low);
        _mm256_add_epi8(bytes, _mm256_add_epi8(_mm256_shuffle_epi8(lut, lo), _mm256_shuffle_epi8(lut, hi)))
    };
    // The lanes whose head distance (their four byte counts summed) is within `bound`.
    let within = |bytes: __m256i, bound: u32| -> u32 {
        let distance = _mm256_madd_epi16(_mm256_maddubs_epi16(bytes, ones8), ones16);
        let over = _mm256_movemask_ps(_mm256_castsi256_ps(_mm256_cmpgt_epi32(distance, broadcast(bound))));
        !u32::try_from(over).unwrap_or(0) & 0xFF
    };
    let test = |r: &Run<'_, '_, W>, x: usize, s: usize, out: &mut Vec<Pair>| {
        let Some(xh) = r.rows.get(x) else { return };
        let Columns { columns, starts } = r.columns;
        let words: [__m256i; W] = xh.map(broadcast);
        let mut off = s;
        while off < r.qhi && off < *starts {
            let mut bytes = _mm256_setzero_si256();
            for (column, &word) in columns.iter().zip(&words) {
                // SAFETY: `off < starts`: each column holds the region's entries and LANES more, and the eight u32
                // from `off` are in it (`Columns::of`).
                let q = unsafe { _mm256_loadu_si256(column.as_ptr().add(off).cast::<__m256i>()) };
                bytes = add_word(bytes, q, word);
            }
            let lanes = within(bytes, r.bound);
            if lanes != 0 {
                hits(st, r, x, off, lanes, out);
            }
            off += LANES;
        }
    };
    // A small bucket's query words, its entries' pictures, and its lanes (none if it cannot be loaded, which a region
    // with heads does not do: a bucket starts before its region's end, so its eight entries are in the columns).
    let load = |columns: &Columns<'_, W>, codes: &[u32], qlo: usize, qhi: usize| {
        let mut heads = [_mm256_setzero_si256(); W];
        let codes = codes.get(qlo..qlo + CODE_PADDING).filter(|_| qlo < columns.starts);
        let Some(codes) = codes else { return (heads, _mm256_setzero_si256(), 0) };
        for (q, column) in heads.iter_mut().zip(&columns.columns) {
            // SAFETY: `qlo < starts` (above), so the eight u32 from `qlo` are in the column (`Columns::of`).
            *q = unsafe { _mm256_loadu_si256(column.as_ptr().add(qlo).cast::<__m256i>()) };
        }
        // SAFETY: `codes` is CODE_PADDING (8) u32s, the 32 bytes the unaligned load reads.
        let codes = unsafe { _mm256_loadu_si256(codes.as_ptr().cast::<__m256i>()) };
        (heads, _mm256_srli_epi32::<4>(codes), lanes_before(qhi, qlo))
    };
    let direct = |r: &Run<'_, '_, W>, bucket: &([__m256i; W], __m256i, u32), x: usize, xc: u32, out: &mut Vec<Pair>| {
        let (heads, pictures, end) = bucket;
        let Some(xh) = r.rows.get(x) else { return };
        let mut bytes = _mm256_setzero_si256();
        for (&q, &word) in heads.iter().zip(xh) {
            bytes = add_word(bytes, q, broadcast(word));
        }
        // The ordering rule's lanes, as `suffix` takes them: all when the stored picture owes nothing, else those of
        // later pictures (a picture is below MAX_PICTURES, 28 bits, so these compare as non-negative i32).
        let later = _mm256_cmpgt_epi32(*pictures, broadcast(xc >> 4));
        let later = u32::try_from(_mm256_movemask_ps(_mm256_castsi256_ps(later))).unwrap_or(0);
        let all = u32::from(xc & OWED == 0) * 0xFF;
        let lanes = within(bytes, r.bound) & (later | all) & end;
        if lanes != 0 {
            hits(st, r, x, r.qlo, lanes, out);
        }
    };
    scan_runs::<W, _>(
        st,
        buckets,
        out,
        |xc, codes, lo, hi, from| suffix_avx2(xc, codes, lo, hi, from),
        test,
        load,
        direct,
    );
}

/// The kind of a team's task, for the join's timings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A sort's pass, or a few-owed path's lists.
    Sort,
    /// A block of a scan.
    Scan,
}

/// What a thread of a join's team is given: a task, nothing for now (it waits until the schedule changes), or nothing
/// more (it leaves).
enum Claim<T> {
    Task(T),
    Wait,
    Done,
}

/// A join's schedule, kept under its team's lock: the tasks there are to claim, in order of priority, and what a
/// finished one changes. A task owns what it writes (its buffers are taken out of the schedule and come back in its
/// report) and shares what it reads (an `Arc` it drops before it reports), so no two tasks write one buffer, and a
/// buffer comes back to the schedule only when no task holds it.
trait Schedule: Send {
    type Task: Send;
    type Report: Send;
    fn claim(&mut self) -> Claim<Self::Task>;
    /// A task's result back: true when it made tasks claimable that were not, or finished the schedule, so the
    /// waiting threads are woken.
    fn report(&mut self, report: Self::Report) -> bool;
}

/// Where a join's threads spent their time, summed over them.
#[derive(Clone, Copy, Default)]
struct Spent {
    sort: Duration,
    scan: Duration,
    starved: Duration,
    paused: Duration,
}

/// A join's team: its schedule and whether it stopped, under one lock, and the condition its waiting threads wait on.
struct Team<S> {
    shared: Mutex<Shared<S>>,
    changed: Condvar,
}

struct Shared<S> {
    schedule: S,
    stopped: bool,
}

impl<S> Team<S> {
    /// The shared state. A thread that panicked holding the lock leaves it poisoned; the team is then stopping, and the
    /// state is still what it was, so the others take it as it is.
    fn lock(&self) -> MutexGuard<'_, Shared<S>> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops the team: no thread claims again, and the waiting ones are woken to leave.
    fn stop(&self) {
        self.lock().stopped = true;
        self.changed.notify_all();
    }
}

/// Stops the team and calls the caller's `abort` when its thread panics: the others leave instead of waiting for what
/// the panicked thread held, and a thread paused in the caller's `control` is woken to see the stop.
struct PanicGuard<'t, S> {
    team: &'t Team<S>,
    abort: &'t (dyn Fn() + Sync),
}

impl<S> Drop for PanicGuard<'_, S> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.team.stop();
            (self.abort)();
        }
    }
}

/// The work of a task, as [`run_team`] calls it: the task, the thread's pairs and first-pass buffers; the task's kind
/// (for the timings) and its report back to the schedule.
type Work<'w, S> =
    dyn Fn(<S as Schedule>::Task, &mut Vec<Pair>, &mut ReadBuffers) -> (Kind, <S as Schedule>::Report) + Sync + 'w;

/// Runs `schedule` on `threads` scoped threads, each `work`ing the tasks it claims (with its pairs and its first-pass
/// buffers), until the schedule is done or the team stops. The pairs found, compacted, and where the threads' time
/// went; `None` when `control` stopped it. Each thread runs `start` first, and asks `control` before each claim, never
/// while it holds a task; it waits only when it can claim nothing, and asks `control` again when woken. Every thread is
/// joined, and the first panic resumes on the caller.
fn run_team<S: Schedule>(
    threads: usize,
    start: &(dyn Fn() + Sync),
    control: &(dyn Fn() -> bool + Sync),
    abort: &(dyn Fn() + Sync),
    schedule: S,
    work: &Work<'_, S>,
) -> Option<(Vec<Pair>, Spent)> {
    let team = Team { shared: Mutex::new(Shared { schedule, stopped: false }), changed: Condvar::new() };
    let results: Vec<(Vec<Pair>, Spent)> = std::thread::scope(|scope| {
        let members: Vec<_> =
            (0..threads.max(1)).map(|_| scope.spawn(|| member(&team, start, control, abort, work))).collect();
        let mut done = Vec::new();
        let mut panic = None;
        for member in members {
            match member.join() {
                Ok(result) => done.push(result),
                Err(payload) => {
                    panic.get_or_insert(payload);
                }
            }
        }
        if let Some(payload) = panic {
            std::panic::resume_unwind(payload);
        }
        done
    });
    if team.lock().stopped {
        return None;
    }
    let mut pairs = Vec::new();
    let mut spent = Spent::default();
    for (mine, theirs) in results {
        pairs.extend(mine);
        spent.sort += theirs.sort;
        spent.scan += theirs.scan;
        spent.starved += theirs.starved;
        spent.paused += theirs.paused;
    }
    compact(&mut pairs);
    Some((pairs, spent))
}

/// One thread of [`run_team`]. Not inlined, so a profiler can toggle on it: a team thread starts here, and the callgrind
/// bench of the join (`crates/bench/benches/ir/pairing.rs`) counts the team's work by it.
#[inline(never)]
fn member<S: Schedule>(
    team: &Team<S>,
    start: &(dyn Fn() + Sync),
    control: &(dyn Fn() -> bool + Sync),
    abort: &(dyn Fn() + Sync),
    work: &Work<'_, S>,
) -> (Vec<Pair>, Spent) {
    let _guard = PanicGuard { team, abort };
    start();
    let (mut out, mut read, mut spent) = (Vec::new(), ReadBuffers::default(), Spent::default());
    loop {
        let asking = Instant::now();
        let go = control();
        spent.paused += asking.elapsed();
        if !go {
            team.stop();
            break;
        }
        let mut shared = team.lock();
        if shared.stopped {
            break;
        }
        let task = match shared.schedule.claim() {
            Claim::Task(task) => task,
            Claim::Done => break,
            Claim::Wait => {
                let waiting = Instant::now();
                shared = team.changed.wait(shared).unwrap_or_else(PoisonError::into_inner);
                drop(shared);
                spent.starved += waiting.elapsed();
                continue;
            }
        };
        drop(shared);
        let working = Instant::now();
        let (kind, report) = work(task, &mut out, &mut read);
        match kind {
            Kind::Sort => spent.sort += working.elapsed(),
            Kind::Scan => spent.scan += working.elapsed(),
        }
        if team.lock().schedule.report(report) {
            team.changed.notify_all();
        }
    }
    compact(&mut out);
    (out, spent)
}

/// The stored side of a stage, and its query side.
const STORED: usize = 0;
const QUERY: usize = 1;
/// A side's regions: one per high byte of the chunk.
const REGIONS: usize = 256;

/// One side of the stage's sort, as it goes: the first pass's blocks (those not lent to a task; once all are back,
/// frozen into an `Arc` the second pass's tasks share), the regions (those not lent), and how far each pass is. The
/// buffers are kept from stage to stage, so a stage writes over the last one's and allocates only what grows.
#[derive(Default)]
struct SideSort {
    blocks: Vec<Option<Block>>,
    frozen: Option<Arc<Vec<Block>>>,
    regions: Vec<Option<Region>>,
    /// The first pass's blocks of this stage.
    count: usize,
    next_block: usize,
    done_blocks: usize,
    next_region: usize,
    done_regions: usize,
}

impl SideSort {
    /// Freezes the blocks for the second pass, once the first pass has given them all back.
    fn freeze(&mut self) {
        let blocks = self.blocks.iter_mut().take(self.count).map(|block| block.take().unwrap_or_default()).collect();
        self.frozen = Some(Arc::new(blocks));
    }

    /// Takes the blocks back once the second pass is done (no task holds them then).
    fn thaw(&mut self) {
        if let Some(blocks) = self.frozen.take().and_then(Arc::into_inner) {
            for (slot, block) in self.blocks.iter_mut().zip(blocks) {
                *slot = Some(block);
            }
        }
    }
}

/// A stage's two sides, sorted: what its scan's tasks share.
struct Sorts {
    c: usize,
    w: usize,
    sides: [Sorted; 2],
}

/// A stage's scan: the sorted sides its tasks share, the next block to claim, and the blocks done.
struct ScanState {
    sorts: Arc<Sorts>,
    next: usize,
    done: usize,
}

/// The sorted path's schedule: the stages in order, each sorted by the whole team (both sides: the first pass by
/// blocks, the second by regions) and then scanned (by blocks of buckets), in one set of buffers. The next stage begins
/// when this one's scan is done. A second set, so the next stage's sorts could fill the scan's tail, was measured
/// against this at 2 - 12 threads and was no faster at any (PROFILING.md, "One thread team"): it removes the threads'
/// wait at the tail, but its sorts cost more than that wait, as the two sets' memory is twice one's.
struct SortedSchedule {
    widths: [usize; CHUNKS],
    /// A stage's scan tasks (`Joiner::block` buckets each).
    scan_blocks: usize,
    /// The stored side's entries and the query side's.
    entries: [usize; 2],
    next_stage: usize,
    /// The stage in the buffers (none: between stages), its sides' sorts, and its scan once both sides are sorted.
    stage: Option<usize>,
    sides: [SideSort; 2],
    scan: Option<ScanState>,
}

enum SortedTask {
    /// The first pass of `entries`, block `b` of a side, into `block`.
    First { side: usize, b: usize, block: Block, entries: Range<usize>, c: usize, w: usize },
    /// The second pass of a side into region `h`, from the first pass's `count` blocks.
    Second { side: usize, h: usize, region: Region, blocks: Arc<Vec<Block>>, count: usize, w: usize },
    /// The scan of one block of buckets.
    Scan { sorts: Arc<Sorts>, block: usize },
}

enum SortedReport {
    First { side: usize, b: usize, block: Block },
    Second { side: usize, h: usize, region: Region },
    Scan,
}

impl SortedSchedule {
    /// The stages at `widths` head words, of `entries` stored and query entries.
    fn new(widths: [usize; CHUNKS], entries: [usize; 2], scan_blocks: usize) -> SortedSchedule {
        SortedSchedule {
            widths,
            entries,
            scan_blocks,
            next_stage: 0,
            stage: None,
            sides: Default::default(),
            scan: None,
        }
    }

    /// Begins the next stage, if there is one and the buffers are free.
    fn begin(&mut self) {
        if self.stage.is_some() {
            return;
        }
        let c = self.next_stage;
        let Some(&w) = self.widths.get(c) else { return };
        self.next_stage += 1;
        self.stage = Some(c);
        for (side, &n) in self.sides.iter_mut().zip(&self.entries) {
            side.count = n.div_ceil(sort_block(w));
            if side.blocks.len() < side.count {
                side.blocks.resize_with(side.count, || Some(Block::default()));
            }
            side.regions.resize_with(REGIONS, || Some(Region::default()));
            (side.next_block, side.done_blocks, side.next_region, side.done_regions) = (0, 0, 0, 0);
            if side.count == 0 {
                side.freeze();
            }
        }
    }
}

impl Schedule for SortedSchedule {
    type Task = SortedTask;
    type Report = SortedReport;

    fn claim(&mut self) -> Claim<SortedTask> {
        self.begin();
        let Some(c) = self.stage else { return Claim::Done };
        let Some(&w) = self.widths.get(c) else { return Claim::Done };
        if let Some(scan) = self.scan.as_mut() {
            if scan.next < self.scan_blocks {
                let block = scan.next;
                scan.next += 1;
                return Claim::Task(SortedTask::Scan { sorts: Arc::clone(&scan.sorts), block });
            }
            return Claim::Wait;
        }
        for side in [QUERY, STORED] {
            let (Some(sort), Some(&n)) = (self.sides.get_mut(side), self.entries.get(side)) else { continue };
            if sort.next_block < sort.count {
                let b = sort.next_block;
                sort.next_block += 1;
                let block = sort.blocks.get_mut(b).and_then(Option::take).unwrap_or_default();
                let size = sort_block(w);
                let entries = b * size..((b + 1) * size).min(n);
                return Claim::Task(SortedTask::First { side, b, block, entries, c, w });
            }
            if let Some(blocks) = &sort.frozen
                && sort.next_region < REGIONS
            {
                let h = sort.next_region;
                sort.next_region += 1;
                let region = sort.regions.get_mut(h).and_then(Option::take).unwrap_or_default();
                let (blocks, count) = (Arc::clone(blocks), sort.count);
                return Claim::Task(SortedTask::Second { side, h, region, blocks, count, w });
            }
        }
        Claim::Wait
    }

    fn report(&mut self, report: SortedReport) -> bool {
        match report {
            SortedReport::First { side, b, block } => {
                let Some(sort) = self.sides.get_mut(side) else { return true };
                if let Some(slot) = sort.blocks.get_mut(b) {
                    *slot = Some(block);
                }
                sort.done_blocks += 1;
                let all = sort.done_blocks == sort.count;
                if all {
                    sort.freeze();
                }
                all
            }
            SortedReport::Second { side, h, region } => {
                let Some(sort) = self.sides.get_mut(side) else { return true };
                if let Some(slot) = sort.regions.get_mut(h) {
                    *slot = Some(region);
                }
                sort.done_regions += 1;
                if sort.done_regions < REGIONS {
                    return false;
                }
                sort.thaw();
                if self.sides.iter().any(|sort| sort.done_regions < REGIONS) {
                    return false;
                }
                let (Some(c), Some(w)) = (self.stage, self.stage.and_then(|c| self.widths.get(c).copied())) else {
                    return true;
                };
                let sides = self.sides.each_mut().map(|sort| Sorted {
                    regions: sort.regions.iter_mut().map(|region| region.take().unwrap_or_default()).collect(),
                });
                self.scan = Some(ScanState { sorts: Arc::new(Sorts { c, w, sides }), next: 0, done: 0 });
                true
            }
            SortedReport::Scan => {
                let Some(scan) = self.scan.as_mut() else { return true };
                scan.done += 1;
                if scan.done < self.scan_blocks {
                    return false;
                }
                // Every task of the scan has reported, and dropped its handle first: the sides come back.
                if let Some(sorts) = self.scan.take().and_then(|scan| Arc::into_inner(scan.sorts)) {
                    for (sort, sorted) in self.sides.iter_mut().zip(sorts.sides) {
                        for (slot, region) in sort.regions.iter_mut().zip(sorted.regions) {
                            *slot = Some(region);
                        }
                    }
                }
                self.stage = None;
                true
            }
        }
    }
}

/// The few-owed path's schedule: the query side's table, then each chunk's lists ([`FewChunk`]), then the pass over the
/// store by blocks of [`FEW_BLOCK`] entries.
struct FewSchedule {
    /// The store's blocks.
    blocks: usize,
    table_claimed: bool,
    table: Option<Arc<FewTable>>,
    chunks: Vec<Option<FewChunk>>,
    next_chunk: usize,
    done_chunks: usize,
    few: Option<Arc<Few>>,
    next_block: usize,
    done_blocks: usize,
}

enum FewTask {
    Table,
    Chunk { c: usize, table: Arc<FewTable> },
    Scan { few: Arc<Few>, block: usize },
}

enum FewReport {
    Table(FewTable),
    Chunk { c: usize, chunk: FewChunk },
    Scan,
}

impl FewSchedule {
    /// The schedule over a store of `stored` entries.
    fn new(stored: usize) -> FewSchedule {
        FewSchedule {
            blocks: stored.div_ceil(FEW_BLOCK),
            table_claimed: false,
            table: None,
            chunks: (0..CHUNKS).map(|_| None).collect(),
            next_chunk: 0,
            done_chunks: 0,
            few: None,
            next_block: 0,
            done_blocks: 0,
        }
    }
}

impl Schedule for FewSchedule {
    type Task = FewTask;
    type Report = FewReport;

    fn claim(&mut self) -> Claim<FewTask> {
        if !self.table_claimed {
            self.table_claimed = true;
            return Claim::Task(FewTask::Table);
        }
        if let Some(table) = &self.table
            && self.next_chunk < CHUNKS
        {
            let c = self.next_chunk;
            self.next_chunk += 1;
            return Claim::Task(FewTask::Chunk { c, table: Arc::clone(table) });
        }
        if let Some(few) = &self.few {
            if self.next_block < self.blocks {
                let block = self.next_block;
                self.next_block += 1;
                return Claim::Task(FewTask::Scan { few: Arc::clone(few), block });
            }
            if self.done_blocks >= self.blocks {
                return Claim::Done;
            }
        }
        Claim::Wait
    }

    fn report(&mut self, report: FewReport) -> bool {
        match report {
            FewReport::Table(table) => {
                self.table = Some(Arc::new(table));
                true
            }
            FewReport::Chunk { c, chunk } => {
                if let Some(slot) = self.chunks.get_mut(c) {
                    *slot = Some(chunk);
                }
                self.done_chunks += 1;
                if self.done_chunks < CHUNKS {
                    return false;
                }
                let (Some(table), Some(chunks)) =
                    (self.table.clone(), self.chunks.iter_mut().map(Option::take).collect::<Option<Vec<_>>>())
                else {
                    return true;
                };
                self.few = Some(Arc::new(Few { table, chunks }));
                true
            }
            FewReport::Scan => {
                self.done_blocks += 1;
                self.done_blocks >= self.blocks
            }
        }
    }
}

/// Dispatches one block to the kernel at the stage's head words.
fn scan(kernel: Kernel, st: &Stage<'_, '_, '_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    match st.w {
        1 => scan_width::<1>(kernel, st, buckets, out),
        2 => scan_width::<2>(kernel, st, buckets, out),
        3 => scan_width::<3>(kernel, st, buckets, out),
        4 => scan_width::<4>(kernel, st, buckets, out),
        5 => scan_width::<5>(kernel, st, buckets, out),
        6 => scan_width::<6>(kernel, st, buckets, out),
        _ => scan_width::<MAX_HEAD_WORDS>(kernel, st, buckets, out),
    }
}

/// Dispatches one block to the kernel at `W` head words.
fn scan_width<const W: usize>(kernel: Kernel, st: &Stage<'_, '_, '_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    match kernel {
        Kernel::Scalar => scan_scalar::<W>(st, buckets, out),
        #[cfg(target_arch = "x86_64")]
        // SAFETY: `Popcnt` is chosen only when `best_kernel` detected POPCNT.
        Kernel::Popcnt => unsafe { scan_popcnt::<W>(st, buckets, out) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: `Avx2` is chosen only when `best_kernel` detected AVX2 and POPCNT.
        Kernel::Avx2 => unsafe { scan_avx2::<W>(st, buckets, out) },
        #[cfg(not(target_arch = "x86_64"))]
        _ => scan_scalar::<W>(st, buckets, out),
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
    use std::sync::atomic::AtomicBool;

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
            .join(queries, threads, &|| {}, &|| true, &|| {})
            .unwrap()
            .pairs
    }

    /// Every way a join can run: the few-owed path never and always, and the sorted path's scan cut in blocks of an odd
    /// count (97: the last block holds the 61 left) and of the whole stage.
    fn tunings() -> Vec<Tuning> {
        let mut tunings: Vec<Tuning> = [Some(0), Some(usize::MAX)]
            .into_iter()
            .map(|few_entries| Tuning { few_entries, widths: None, block: None })
            .collect();
        tunings.extend([97, VALUES].map(|b| Tuning { few_entries: Some(0), widths: None, block: Some(b) }));
        tunings
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
    fn the_transpose_of_a_permuted_hash_is_a_renaming_of_its_chunks() {
        let mut rng = Rng(11);
        let mut hashes: Vec<Hash> = (0..2000).map(|_| rng.hash()).collect();
        hashes.extend([Hash([0; 32]), Hash([0xFF; 32])]);
        // Each single bit, so every cell's place is checked on its own.
        for k in 0..256 {
            let mut bytes = [0u8; 32];
            bytes[k / 8] = 1 << (7 - k % 8);
            hashes.push(Hash(bytes));
        }
        for h in &hashes {
            assert_eq!(transposed(&permuted(h)), permuted(&h.transpose()), "{h:?}");
        }
    }

    /// The store's permutation undone gives back every hash, and `Joiner::original` every picture's orientations.
    #[test]
    fn the_original_hashes_come_back_from_the_store() {
        let mut rng = Rng(12);
        let mut hashes: Vec<Hash> = (0..2000).map(|_| rng.hash()).collect();
        hashes.extend([Hash([0; 32]), Hash([0xFF; 32])]);
        for k in 0..256 {
            let mut bytes = [0u8; 32];
            bytes[k / 8] = 1 << (7 - k % 8);
            hashes.push(Hash(bytes));
        }
        for h in &hashes {
            assert_eq!(unpermuted(&permuted(h)), *h, "{h:?}");
        }
        let pictures: Vec<[Hash; ORIENTATIONS]> = hashes.as_chunks::<ORIENTATIONS>().0.to_vec();
        let owed = vec![true; pictures.len()];
        let joiner = Joiner::new(&pictures, &owed).unwrap();
        for (i, picture) in pictures.iter().enumerate() {
            assert_eq!(joiner.original(i), Some(*picture));
        }
        assert_eq!(joiner.original(pictures.len()), None);
        // The store permuted on several threads is the same, however the pictures divide among them.
        for threads in [2, 3, 7, pictures.len() + 1] {
            let on = Joiner::new_on(&pictures, &owed, threads, &|| {}).unwrap();
            assert_eq!(on.stored, joiner.stored, "{threads} threads");
        }
    }

    /// A sorted side read back in value order: each entry's head (`w` words, held in `layout`) and code, and each
    /// region's lengths checked.
    fn read_back(out: &Sorted, w: usize, layout: Layout) -> Vec<(Vec<u32>, u32)> {
        let mut all = Vec::new();
        for v in 0..VALUES {
            let (r, lo, hi) = out.bucket(v);
            for j in lo..hi {
                let head = match layout {
                    Layout::Rows => r.head[j * w..(j + 1) * w].to_vec(),
                    Layout::Columns => (0..w).map(|k| r.head[k * r.stride + j]).collect(),
                };
                all.push((head, r.code[j]));
            }
        }
        for r in &out.regions {
            let len = index(r.offsets[256]);
            let (head, stride) = match layout {
                Layout::Rows => (len * w, w),
                Layout::Columns => (w * (len + LANES), len + LANES),
            };
            assert_eq!((r.head.len(), r.stride, r.code.len()), (head, stride, len + CODE_PADDING));
        }
        all
    }

    /// The head of `words` at stage `c`, `w` words, by its definition: chunks `c + 1, c + 2, ...` (mod 16), two to a word.
    fn head_of(words: &[u64; 4], c: usize, w: usize) -> Vec<u32> {
        (0..w)
            .map(|j| {
                let (a, b) = (chunk(words, (c + 1 + 2 * j) % 16), chunk(words, (c + 2 + 2 * j) % 16));
                u32::try_from(a + 65536 * b).unwrap()
            })
            .collect()
    }

    #[test]
    fn the_radix_sort_is_a_stable_sort_by_the_chunk() {
        let mut rng = Rng(99);
        // Few distinct chunk values, so the buckets are long and stability shows; more entries than a block of the
        // first pass, so the runs of several blocks are merged.
        let crowded = |rng: &mut Rng, n: u32| -> Vec<([u64; 4], u32)> {
            (0..n)
                .map(|i| {
                    let w = [
                        rng.draw() & 0x0303_0303_0303_0303,
                        rng.draw(),
                        rng.draw(),
                        rng.draw() & 0x0101_0101_0101_0101,
                    ];
                    (w, i)
                })
                .collect()
        };
        let random = |rng: &mut Rng, n: u32| -> Vec<([u64; 4], u32)> {
            (0..n).map(|i| ([rng.draw(), rng.draw(), rng.draw(), rng.draw()], i)).collect()
        };
        // One output per layout through every case: a side is reused from stage to stage without being cleared, with
        // more entries and then fewer, and wider heads and then narrower, so stale values sit in every array.
        for layout in [Layout::Rows, Layout::Columns] {
            let mut out = Sorted::default();
            for w in [1, 4, 7, 2] {
                let big = u32::try_from(3 * sort_block(w) + 17).unwrap();
                for entries in [
                    crowded(&mut rng, 5000),
                    random(&mut rng, big),
                    crowded(&mut rng, big),
                    random(&mut rng, 40),
                    Vec::new(),
                ] {
                    for c in [0, 3, 7, 8, 12, 15] {
                        let of = |e: usize| (chunks(&entries[e].0), entries[e].1);
                        sort_into(&mut out, entries.len(), of, c, w, layout);
                        let mut want = entries.clone();
                        want.sort_by_key(|(words, _)| chunk(words, c));
                        let want: Vec<(Vec<u32>, u32)> =
                            want.iter().map(|(words, i)| (head_of(words, c, w), *i)).collect();
                        let n = entries.len();
                        assert_eq!(read_back(&out, w, layout), want, "{n} entries, chunk {c}, {w} words, {layout:?}");
                        for v in [0usize, 1, 0x0101, 0x0303, 0xFFFF] {
                            let (r, lo, hi) = out.bucket(v);
                            assert!(r.code[lo..hi].iter().all(|&i| chunk(&entries[index(i)].0, c) == v));
                            assert_eq!(hi - lo, entries.iter().filter(|(words, _)| chunk(words, c) == v).count());
                        }
                    }
                }
            }
        }
    }

    /// The head test's bound is exact: a pair at the radius whose first chunk within one bit is stage `c`, with every
    /// bit it may spend placed in the head (so the head is at the bound), is found at every stage, every width, every
    /// kind of probe (the value itself, a low bit flipped in the bucket's own region, a high bit flipped in another),
    /// on every kernel and path; one bit more is over the radius.
    #[test]
    fn the_head_test_is_exact_at_every_stage_width_and_probe() {
        // Where each bit of a hash lands: chunk and bit of the chunk's value.
        let mut place = [[0usize; 16]; 16];
        for k in 0..256 {
            let mut bytes = [0u8; 32];
            bytes[k / 8] = 1 << (7 - k % 8);
            let words = permuted(&Hash(bytes));
            let c = (0..CHUNKS).find(|&c| chunk(&words, c) != 0).unwrap();
            place[c][index(chunk(&words, c).trailing_zeros())] = k;
        }
        let mut rng = Rng(23);
        for w in 1..=MAX_HEAD_WORDS {
            let mut lib = Library { hashes: Vec::new(), owed: Vec::new() };
            for c in 0..CHUNKS {
                // The bit flipped in chunk c: none, the value's bit 0 (low byte), its bit 15 (high byte).
                for flip_at in [None, Some(0), Some(15)] {
                    for over in [false, true] {
                        let x = rng.four();
                        let mut flips: Vec<(usize, usize)> = (0..c).flat_map(|e| [(e, 0), (e, 1)]).collect();
                        flips.extend(flip_at.map(|b| (c, b)));
                        // The rest of the radius, all in the head: its chunks after c first, then those that wrap.
                        let mut rest = 31 - flips.len() + usize::from(over);
                        let head: Vec<usize> = (1..=2 * w).map(|i| (c + i) % 16).collect();
                        for &e in head.iter().filter(|&&e| e > c).chain(head.iter().filter(|&&e| e < c)) {
                            let from = if e < c { 2 } else { 0 };
                            for b in from..16 {
                                if rest == 0 {
                                    break;
                                }
                                flips.push((e, b));
                                rest -= 1;
                            }
                        }
                        assert_eq!(rest, 0, "the head holds the radius: stage {c}, {w} words");
                        let mut y = x[0];
                        for (e, b) in flips {
                            y = flip(&y, place[e][b]);
                        }
                        assert_eq!(x[0].distance(&y), 31 + u32::from(over));
                        lib.hashes.push(x);
                        lib.hashes.push([y, rng.hash(), rng.hash(), rng.hash()]);
                        lib.owed.extend([true, true]);
                    }
                }
            }
            let want = brute(&lib, 0..lib.hashes.len());
            assert_eq!(want.len(), 16 * 3, "the pairs at the radius, and only those");
            for kernel in kernels() {
                for few_entries in [Some(0), Some(usize::MAX)] {
                    let tuning = Tuning { few_entries, widths: Some([w; CHUNKS]), block: None };
                    let got = Joiner::new(&lib.hashes, &lib.owed)
                        .unwrap()
                        .with_kernel(kernel)
                        .tuned(tuning)
                        .join(0..lib.hashes.len(), 2, &|| {}, &|| true, &|| {})
                        .unwrap()
                        .pairs;
                    assert_eq!(got, want, "{kernel:?}, {w} words, {tuning:?}");
                }
            }
        }
    }

    /// The binomial tail against exact counts, and the widths it chooses: those the measurements found fastest (stage
    /// 10's best was 3 by a margin within the noise, at the same full tests as 2).
    #[test]
    fn the_widths_are_the_fewest_words_worth_their_cost() {
        // Exact counts (each product in u128, the sums below 2^63), their f64 within 2^-52 of them.
        for (n, ts) in [(32u32, vec![0, 4, 16, 31, 32]), (64, vec![0, 4, 16, 31])] {
            let total = 2f64.powi(i32::try_from(n).unwrap());
            for t in ts {
                let exact: u64 = (0..=t)
                    .map(|i| {
                        u64::try_from((0..i).fold(1u128, |c, k| c * u128::from(n - k) / u128::from(k + 1))).unwrap()
                    })
                    .sum();
                let want = pg_model::num::size(exact) / total;
                assert!((tail(n, t) - want).abs() <= want * 1e-12, "tail({n}, {t}) {} against {want}", tail(n, t));
            }
        }
        assert_eq!(WIDTHS, [3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1]);
    }
    /// Every head width, the same at every stage and mixed, finds the pairs brute force does, on every kernel and path.
    #[test]
    fn every_head_width_matches_brute_force() {
        let mut rng = Rng(0xC0FF_EE00_D15E_A5E5);
        let lib = corpus(&mut rng, 300, 4);
        let want = brute(&lib, 0..lib.hashes.len());
        assert!(want.len() > 10, "the corpus must have pairs to find");
        let mut widths: Vec<[usize; CHUNKS]> = (1..=MAX_HEAD_WORDS).map(|w| [w; CHUNKS]).collect();
        widths.push(std::array::from_fn(|c| 1 + c % MAX_HEAD_WORDS));
        widths.push(std::array::from_fn(|c| MAX_HEAD_WORDS - c % MAX_HEAD_WORDS));
        for w in widths {
            for kernel in kernels() {
                for few_entries in [Some(0), Some(usize::MAX)] {
                    let tuning = Tuning { few_entries, widths: Some(w), block: None };
                    let got = Joiner::new(&lib.hashes, &lib.owed)
                        .unwrap()
                        .with_kernel(kernel)
                        .tuned(tuning)
                        .join(0..lib.hashes.len(), 3, &|| {}, &|| true, &|| {})
                        .unwrap()
                        .pairs;
                    assert_eq!(got, want, "{kernel:?} {w:?} {tuning:?}");
                }
            }
        }
    }

    /// Buckets of hundreds of entries: hundreds of copies of one picture, owed and not, so a query bucket spans a low
    /// byte's worth of entries and many loads of eight, and the stored entries' suffixes start anywhere in it.
    #[test]
    fn buckets_of_hundreds_of_entries_match_brute_force() {
        let mut rng = Rng(77);
        let base = rng.four();
        let mut lib = Library { hashes: Vec::new(), owed: Vec::new() };
        for i in 0..500 {
            let mut four = base;
            for h in &mut four {
                for _ in 0..rng.below(4) {
                    *h = flip(h, rng.below(256));
                }
            }
            lib.hashes.push(four);
            lib.owed.push(i % 3 != 1);
        }
        let want = brute(&lib, 0..lib.hashes.len());
        // Most chunks of an orientation's copies agree, so a bucket holds about all 500 of them on the stored side and the
        // 333 owed ones on the query side: over 256, and over forty loads of eight.
        let queries = 500 * 2 / 3;
        assert!(queries > 40 * LANES, "past 256 and 40 loads: {queries}");
        assert!(want.len() > 100_000, "the copies pair with each other: {}", want.len());
        for kernel in kernels() {
            for w in [1, 4] {
                let got = Joiner::new(&lib.hashes, &lib.owed)
                    .unwrap()
                    .with_kernel(kernel)
                    .tuned(Tuning { few_entries: Some(0), widths: Some([w; CHUNKS]), block: None })
                    .join(0..lib.hashes.len(), 2, &|| {}, &|| true, &|| {})
                    .unwrap()
                    .pairs;
                assert_eq!(got, want, "{kernel:?}, {w} words");
            }
        }
    }

    /// The few-owed path's lists are the probes: for every chunk value, the query entries listed under it are exactly
    /// those whose own value is it or one bit from it, each once. With queries whose values are one bit apart (each in
    /// the other's probes), two bits apart (sharing neighbours), and equal (two entries of one value).
    #[test]
    fn the_few_owed_lists_are_the_probes() {
        let mut rng = Rng(41);
        let mut words: Vec<[u64; 4]> = (0..40).map(|_| [rng.draw(), rng.draw(), rng.draw(), rng.draw()]).collect();
        let base = words[0];
        for k in [0, 5, 6, 15] {
            // Chunk 0 is the low 16 bits of the first word: one bit, then two bits, from the first entry's.
            words.push([base[0] ^ (1 << k), base[1], base[2], base[3]]);
            words.push([base[0] ^ (1 << k) ^ (1 << ((k + 3) % 16)), base[1], base[2], base[3]]);
        }
        words.push(base);
        for c in [0, 7, 15] {
            let few = FewChunk::new(&words, c);
            for v in 0..VALUES {
                let mut got = few.met(v).to_vec();
                got.sort_unstable();
                let want: Vec<u32> =
                    (0u32..).zip(&words).filter(|(_, w)| (chunk(w, c) ^ v).count_ones() <= 1).map(|(j, _)| j).collect();
                assert_eq!(got, want, "chunk {c}, value {v:#06x}");
            }
        }
    }

    #[test]
    fn the_vector_suffix_starts_where_the_walk_does() {
        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("avx2") {
            let mut rng = Rng(7);
            let code = |rng: &mut Rng, picture: u32| (picture << 4) | u32::try_from(rng.below(16)).unwrap();
            // Buckets of 0 - 40 queries in picture order (a picture repeats: several of its orientations share a
            // bucket), with pictures near 0 and near MAX_PICTURES (the vector compare is signed). The last bucket
            // ends at the codes' length, so its reads go into the padding.
            let (mut codes, mut buckets) = (Vec::new(), Vec::new());
            for b in 0..400 {
                let base = if b % 3 == 0 { MAX_PICTURES - 100 } else { 0 };
                let mut pictures: Vec<u32> =
                    (0..rng.below(41)).map(|_| base + u32::try_from(rng.below(60)).unwrap()).collect();
                pictures.sort_unstable();
                let lo = codes.len();
                codes.extend(pictures.iter().map(|&p| code(&mut rng, p)));
                buckets.push((lo, codes.len(), base));
            }
            codes.resize(codes.len() + CODE_PADDING, 0);
            for &(lo, hi, base) in &buckets {
                // One probe's stored entries, in picture order, owed or not (bit 3 of the code).
                let mut stored: Vec<u32> = (0..rng.below(30))
                    .map(|_| {
                        let picture = base + u32::try_from(rng.below(64)).unwrap();
                        code(&mut rng, picture)
                    })
                    .collect();
                stored.sort_unstable_by_key(|c| c >> 4);
                let (mut walk, mut vector) = (lo, lo);
                for &xc in &stored {
                    let want = suffix(xc, &codes, lo, hi, &mut walk);
                    // SAFETY: AVX2 was detected above.
                    let got = unsafe { suffix_avx2(xc, &codes, lo, hi, &mut vector) };
                    assert_eq!((got, vector), (want, walk), "bucket {lo}..{hi}, stored code {xc:#x}");
                }
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
            let got = joiner.join(slice.clone(), 2, &|| {}, &|| true, &|| {}).unwrap().pairs;
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
            assert!(joiner.join(0..50, 4, &|| {}, &|| false, &|| {}).is_none(), "{tuning:?}");
        }
    }

    /// A stop at any point (before the first task, mid-sort, mid-scan, mid-pass over the store) ends the join, with
    /// every thread joined: the call returns, with nothing.
    #[test]
    fn a_stop_at_any_point_returns_none() {
        let mut rng = Rng(23);
        let lib = corpus(&mut rng, 600, 8);
        for tuning in tunings() {
            for after in [1, 5, 40, 300, 3000] {
                let (asked, refused) = (AtomicUsize::new(0), AtomicBool::new(false));
                let control = || {
                    let go = asked.fetch_add(1, Relaxed) < after;
                    refused.fetch_or(!go, Relaxed);
                    go
                };
                let joiner = Joiner::new(&lib.hashes, &lib.owed).unwrap().tuned(tuning);
                let joined = joiner.join(0..600, 4, &|| {}, &control, &|| {});
                // A join with fewer tasks than `after` finishes before the stop: it returns its pairs then.
                assert_eq!(joined.is_none(), refused.load(Relaxed), "{tuning:?}, stopped after {after}");
            }
        }
    }

    /// A thread paused in `control` while the others take the work, and while they wait at the end for nothing: the
    /// join's pairs are the same.
    #[test]
    fn a_pause_while_the_others_wait_changes_nothing() {
        let mut rng = Rng(29);
        let lib = corpus(&mut rng, 400, 5);
        let want = brute(&lib, 0..400);
        for tuning in tunings() {
            for pause_at in [0, 7, 200] {
                let asked = AtomicUsize::new(0);
                let control = || {
                    if asked.fetch_add(1, Relaxed) == pause_at {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    true
                };
                let joiner = Joiner::new(&lib.hashes, &lib.owed).unwrap().tuned(tuning);
                let joined = joiner.join(0..400, 4, &|| {}, &control, &|| {}).unwrap();
                assert_eq!(joined.pairs, want, "{tuning:?}, paused at {pause_at}");
                // A schedule with fewer tasks than `pause_at` (the few-owed path's) ends before the pause.
                if asked.load(Relaxed) > pause_at {
                    assert!(joined.times.paused >= Duration::from_millis(30), "{tuning:?}: {:?}", joined.times);
                }
            }
        }
    }

    /// A panic in a task of either kind, in `start` or in `control` resumes on the caller once every thread is joined,
    /// and a task's panic calls `abort`: no thread is left waiting for what the panicked one held. Also with another
    /// thread paused in `control` until `abort` wakes it.
    #[test]
    fn a_panic_anywhere_resumes_on_the_caller() {
        let mut rng = Rng(31);
        let lib = corpus(&mut rng, 300, 5);
        for tuning in tunings() {
            for (kind, nth) in [(Kind::Sort, 1), (Kind::Sort, 40), (Kind::Scan, 1), (Kind::Scan, 30)] {
                // Paused only where the fault is sure to fire (every schedule has a first task of each kind): with
                // no fault, nothing would wake the paused thread.
                for paused in [false, nth == 1] {
                    let aborted = AtomicBool::new(false);
                    let asked = AtomicUsize::new(0);
                    // The first ask blocks until `abort` is called: that thread holds nothing meanwhile.
                    let control = || {
                        if paused && asked.fetch_add(1, Relaxed) == 0 {
                            while !aborted.load(Relaxed) {
                                std::thread::sleep(Duration::from_millis(1));
                            }
                            return false;
                        }
                        true
                    };
                    let abort = || aborted.store(true, Relaxed);
                    let joiner = Joiner::new(&lib.hashes, &lib.owed).unwrap().tuned(tuning).with_fault(kind, nth);
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        joiner.join(0..300, 4, &|| {}, &control, &abort)
                    }));
                    // A schedule with fewer tasks of the kind than `nth` finishes without the fault.
                    if let Err(payload) = result {
                        let message = payload.downcast_ref::<String>().cloned().unwrap_or_default();
                        assert!(message.contains("injected fault"), "{tuning:?} {kind:?} {nth}: {message}");
                        assert!(aborted.load(Relaxed), "{tuning:?} {kind:?} {nth}: abort was not called");
                    } else {
                        assert!(nth > 1, "{tuning:?} {kind:?} {nth}: the fault did not fire");
                    }
                }
            }
            let joiner = Joiner::new(&lib.hashes, &lib.owed).unwrap().tuned(tuning);
            let started = AtomicUsize::new(0);
            let start = || {
                if started.fetch_add(1, Relaxed) == 2 {
                    panic!("in start");
                }
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                joiner.join(0..300, 4, &start, &|| true, &|| {})
            }));
            assert!(result.is_err(), "{tuning:?}: a panic in start");
            let asked = AtomicUsize::new(0);
            let control = || {
                assert!(asked.fetch_add(1, Relaxed) != 9, "in control");
                true
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                joiner.join(0..300, 4, &|| {}, &control, &|| {})
            }));
            assert!(result.is_err(), "{tuning:?}: a panic in control");
        }
    }

    /// Many threads on tiny stages, again and again: most of them wait for nothing most of the time, and the join
    /// still ends, with the same pairs. And `start` runs on every thread before it asks for work.
    #[test]
    fn many_threads_on_tiny_stages_end_with_the_same_pairs() {
        let mut rng = Rng(37);
        let lib = corpus(&mut rng, 40, 5);
        let want = brute(&lib, 0..40);
        for tuning in tunings() {
            for _ in 0..20 {
                // Each thread of a team is new, so its flag is set only by its own `start`.
                thread_local!(static STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
                let started = AtomicUsize::new(0);
                let start = || {
                    STARTED.set(true);
                    started.fetch_add(1, Relaxed);
                };
                let control = || {
                    assert!(STARTED.get(), "control before start");
                    true
                };
                let joiner = Joiner::new(&lib.hashes, &lib.owed).unwrap().tuned(tuning);
                let joined = joiner.join(0..40, 16, &start, &control, &|| {}).unwrap();
                assert_eq!(joined.pairs, want, "{tuning:?}");
                assert_eq!(started.load(Relaxed), 16, "{tuning:?}: start on every thread");
            }
        }
    }

    /// All of a join's work runs at the caller's priority: the store's build on one thread or several, and every task
    /// of every path, each on a thread that ran `start` first (with `CHECK_START` on, the build and each task assert
    /// it). Without a `start` that marks its thread, the build and the join fail: the check is live.
    #[test]
    fn start_runs_before_the_build_and_every_task() {
        let mut rng = Rng(41);
        let lib = corpus(&mut rng, 300, 5);
        let want = brute(&lib, 0..300);
        CHECK_START.set(true);
        let start = || STARTED.set(true);
        for threads in [1, 4] {
            for tuning in tunings() {
                let joiner = Joiner::new_on(&lib.hashes, &lib.owed, threads, &start).unwrap().tuned(tuning);
                let joined = joiner.join(0..300, threads, &start, &|| true, &|| {}).unwrap();
                assert_eq!(joined.pairs, want, "{tuning:?} on {threads}");
                let unmarked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    joiner.join(0..300, threads, &|| {}, &|| true, &|| {})
                }));
                assert!(unmarked.is_err(), "{tuning:?} on {threads}: a join whose threads did not run start");
            }
            let unmarked = std::panic::catch_unwind(|| Joiner::new_on(&lib.hashes, &lib.owed, threads, &|| {}).is_ok());
            assert!(unmarked.is_err(), "{threads} threads: a build whose threads did not run start");
        }
        CHECK_START.set(false);
    }

    /// The sorted path and the few-owed path find the same pairs as brute force, on every
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
