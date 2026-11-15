//! The all-pairs join over the tier-1 hashes: every two pictures with an orientation of one within [`MATCH_DISTANCE`] of
//! an orientation of the other, found by sorting the hashes' chunks and probing buckets instead of a random-access index.
//!
//! The 256-bit hash is 16 chunks of 16 bits. Two hashes within the radius differ by at most one bit in some chunk
//! (pigeonhole: 16 chunks of 2 bits hold 32 bits, and the radius is 31), so for each chunk a query's value probes its 17
//! buckets (itself and its 16 one-bit neighbours) of the stored side, sorted by that chunk. Each (stored, query) pair is
//! found in exactly one probe of a chunk; a pair that matches in several chunks is reported only in the first, which is
//! checked on the hits, not on the candidates.
//!
//! Only orientations 0..4 are stored. Orientation `v + 4` is orientation `v` transposed bit-exactly (the DCT's symmetry,
//! `analysis::pdq::dihedral`), so the 64 orientation pairs of two pictures are 32 distinct distances: a query's 8
//! orientations against the stored 4 cover every one. [`Picture::orientations`] carries all 8; the caller checks the
//! identity once when it loads them (`transposed`).
//!
//! Between two pictures that are both due, the one with the higher index queries and the lower index is stored: the pair
//! is found once. A query picture is compared with a stored one only when they differ and the stored one is not due or
//! has the lower index (`owed`). The output is the least distance per pair, sorted by `(a, b)`, `a < b`.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

use pg_model::num::index;
use pg_model::pdq::{Hash, MATCH_DISTANCE};

/// The chunks a hash is split into: 16 of 16 bits.
const CHUNKS: usize = 16;
/// The values a chunk takes.
const VALUES: usize = 1 << 16;
/// The buckets a thread takes at a time (its unit of work, and of control).
const BLOCK: usize = 128;
/// The most pictures the join holds: an entry's code is the picture's index and an orientation (3 bits) in a `u32`.
pub const MAX_PICTURES: u32 = u32::MAX >> 3;

/// One picture for the join: its eight orientations (0..4 stored, 4..8 the transposes of those) and whether its pairs
/// are still owed.
#[derive(Clone, Copy, Debug)]
pub struct Picture {
    pub orientations: [Hash; 8],
    pub due: bool,
}

/// The hash of the same picture transposed: the 16x16 grid of its bits (most significant first, byte 0 first) turned
/// over its diagonal. Orientation `v + 4` of a picture is this of its orientation `v`, bit for bit.
pub fn transpose(h: &Hash) -> Hash {
    let bit = |k: usize| h.0.get(k / 8).map_or(0, |byte| (byte >> (7 - k % 8)) & 1);
    let mut out = [0u8; 32];
    for k in 0..256 {
        let t = (k % 16) * 16 + k / 16;
        if let Some(byte) = out.get_mut(k / 8) {
            *byte |= bit(t) << (7 - k % 8);
        }
    }
    Hash(out)
}

/// Whether the picture's orientations 4..8 are the transposes of 0..4, as the join assumes. A picture that is not (a blob
/// from another build) is stored with all eight, and queried with its transposes too: slower, never wrong.
pub fn is_symmetric(orientations: &[Hash; 8]) -> bool {
    (0..4).all(|v| match (orientations.get(v), orientations.get(v + 4)) {
        (Some(o), Some(t)) => *t == transpose(o),
        _ => false,
    })
}

/// A pair of pictures by their index in the slice given (`a < b`), and the least distance over their orientations.
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

/// The pairs among `pictures` in which the query side is one of `queries` (a range of indices; the due ones in it query).
///
/// `threads` workers per stage (at least one); `start` runs once in each worker before it works (the caller sets its
/// priority there); `control` is asked before each block of buckets and returns false to stop, and may block (a pause).
/// `None` when it stopped the join: nothing is returned then. The pictures must be ordered by file id, since the pairs
/// are by index.
pub fn join(
    pictures: &[Picture],
    queries: Range<usize>,
    threads: usize,
    start: &(dyn Fn() + Sync),
    control: &(dyn Fn() -> bool + Sync),
) -> Result<Option<Vec<Pair>>, TooManyPictures> {
    join_with(best_kernel(), pictures, queries, threads, start, control)
}

/// [`join`] with the kernel given (the tests run each).
pub(crate) fn join_with(
    kernel: Kernel,
    pictures: &[Picture],
    queries: Range<usize>,
    threads: usize,
    start: &(dyn Fn() + Sync),
    control: &(dyn Fn() -> bool + Sync),
) -> Result<Option<Vec<Pair>>, TooManyPictures> {
    if !u32::try_from(pictures.len()).is_ok_and(|n| n <= MAX_PICTURES) {
        return Err(TooManyPictures(pictures.len()));
    }
    let threads = threads.max(1);
    let due: Vec<bool> = pictures.iter().map(|p| p.due).collect();
    let mut stored = Vec::with_capacity(pictures.len() * 4);
    let mut query = Vec::new();
    for (i, p) in (0u32..).zip(pictures) {
        let symmetric = is_symmetric(&p.orientations);
        let in_slice = p.due && queries.contains(&index(i));
        for (o, hash) in (0u32..).zip(p.orientations) {
            let entry = Entry { words: hash.words(), code: (i << 3) | o };
            if o < 4 || !symmetric {
                stored.push(entry);
            }
            if in_slice {
                query.push(entry);
            }
        }
        if in_slice && !symmetric {
            // A stored symmetric picture's orientations 4..8 are the transposes of its 0..4, so against a picture that
            // isn't symmetric the query's own transposes are needed: the pair (transpose of stored, query) is the pair
            // (stored, transpose of query), at the same distance. The code's orientation bits are not read for a query.
            for (o, hash) in (0u32..).zip(p.orientations) {
                query.push(Entry { words: transpose(&hash).words(), code: (i << 3) | (o & 7) });
            }
        }
    }

    let mut found = Vec::new();
    for c in 0..CHUNKS {
        let stored_side = sort(&stored, c);
        let query_side = sort(&query, c);
        let stage = Stage { c, stored: &stored_side, query: &query_side, due: &due };
        match run_stage(kernel, &stage, threads, start, control) {
            Some(mut pairs) => found.append(&mut pairs),
            None => return Ok(None),
        }
    }
    compact(&mut found);
    Ok(Some(found))
}

/// A hash's words with the picture and orientation it came from: `code` is `picture << 3 | orientation`.
#[derive(Clone, Copy)]
struct Entry {
    words: [u64; 4],
    code: u32,
}

/// Entries sorted by one chunk: `entries[offsets[v]..offsets[v + 1]]` have chunk value `v`. The first two words of each
/// entry also sit in their own arrays (`head0`, `head1`, with four zeros of padding) for the vector kernel.
struct Sorted {
    offsets: Vec<usize>,
    entries: Vec<Entry>,
    head0: Vec<u64>,
    head1: Vec<u64>,
}

/// The entries counting-sorted by their chunk `c`.
#[expect(
    clippy::indexing_slicing,
    reason = "every chunk value is below VALUES, the length of offsets less one, and each position is below the \
              entries' length: the offsets are the running sums of the counts"
)]
fn sort(items: &[Entry], c: usize) -> Sorted {
    let mut offsets = vec![0usize; VALUES + 1];
    for e in items {
        offsets[chunk(&e.words, c) + 1] += 1;
    }
    for v in 0..VALUES {
        offsets[v + 1] += offsets[v];
    }
    let mut next = offsets.clone();
    let mut entries = vec![Entry { words: [0; 4], code: 0 }; items.len()];
    for e in items {
        let v = chunk(&e.words, c);
        entries[next[v]] = *e;
        next[v] += 1;
    }
    let head0 = entries.iter().map(|e| e.words[0]).chain([0; 4]).collect();
    let head1 = entries.iter().map(|e| e.words[1]).chain([0; 4]).collect();
    Sorted { offsets, entries, head0, head1 }
}

/// Chunk `c` of a hash: 16 bits, the little-endian order the words are stored in.
#[inline(always)]
fn chunk(words: &[u64; 4], c: usize) -> usize {
    let word = words.get(c >> 2).copied().unwrap_or(0);
    usize::try_from((word >> ((c & 3) * 16)) & 0xFFFF).unwrap_or(0)
}

/// The distance between two hashes if it is within the radius: the first two words first, which rejects most
/// candidates.
#[inline(always)]
fn near(x: &[u64; 4], y: &[u64; 4]) -> Option<u32> {
    let [x0, x1, x2, x3] = *x;
    let [y0, y1, y2, y3] = *y;
    let head = (x0 ^ y0).count_ones() + (x1 ^ y1).count_ones();
    if head > MATCH_DISTANCE {
        return None;
    }
    let d = head + (x2 ^ y2).count_ones() + (x3 ^ y3).count_ones();
    (d <= MATCH_DISTANCE).then_some(d)
}

/// Whether the pair is also a candidate in an earlier chunk, so it is reported there (the first chunk that matches).
#[inline(always)]
fn matched_earlier(x: &[u64; 4], y: &[u64; 4], c: usize) -> bool {
    (0..c).any(|e| (chunk(x, e) ^ chunk(y, e)).count_ones() <= 1)
}

/// Whether the stored picture `stored` is compared with the query picture `query`: they differ, and the stored one is
/// not due or comes first (a pair of two due pictures is found by the later one's query).
#[inline(always)]
fn owed(stored: u32, query: u32, due: &[bool]) -> bool {
    stored != query && (!due.get(index(stored)).copied().unwrap_or(false) || stored < query)
}

/// One chunk's pass: the stored and query sides sorted by it.
struct Stage<'a> {
    c: usize,
    stored: &'a Sorted,
    query: &'a Sorted,
    due: &'a [bool],
}

/// Tests one stored entry against one query entry, and records the pair if it is owed, within the radius and first found
/// in this chunk.
#[inline(always)]
fn consider(st: &Stage<'_>, x: &Entry, y: &Entry, out: &mut Vec<Pair>) {
    let (xp, yp) = (x.code >> 3, y.code >> 3);
    if !owed(xp, yp, st.due) {
        return;
    }
    if let Some(distance) = near(&x.words, &y.words)
        && !matched_earlier(&x.words, &y.words, st.c)
    {
        out.push(Pair { a: xp.min(yp), b: xp.max(yp), distance });
    }
}

/// The buckets `v` of the query side, and the 17 probes of each: the stored side's bucket of the same value, and of
/// each one-bit neighbour.
#[inline(always)]
#[expect(
    clippy::indexing_slicing,
    reason = "a bucket is below VALUES and the offsets have VALUES + 1 entries, so every range lies in its side"
)]
fn scan_block(st: &Stage<'_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    for v in buckets {
        let (qlo, qhi) = (st.query.offsets[v], st.query.offsets[v + 1]);
        if qlo == qhi {
            continue;
        }
        for p in 0..17 {
            let b = if p == 0 { v } else { v ^ (1 << (p - 1)) };
            for x in &st.stored.entries[st.stored.offsets[b]..st.stored.offsets[b + 1]] {
                for y in &st.query.entries[qlo..qhi] {
                    consider(st, x, y, out);
                }
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "popcnt")]
unsafe fn scan_popcnt(st: &Stage<'_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    scan_block(st, buckets, out);
}

/// The vector kernel: four query entries at a time, their first two words' distance in one register, and only the
/// lanes within the radius checked in full.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,popcnt")]
#[expect(
    clippy::indexing_slicing,
    reason = "a bucket is below VALUES and the offsets have VALUES + 1 entries; the vector loads read four lanes past a \
              query position, inside the padding of head0 and head1"
)]
unsafe fn scan_avx2(st: &Stage<'_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi8, _mm256_and_si256, _mm256_castsi256_pd, _mm256_cmpgt_epi64, _mm256_loadu_si256,
        _mm256_movemask_pd, _mm256_sad_epu8, _mm256_set1_epi8, _mm256_set1_epi64x, _mm256_setr_epi8,
        _mm256_setzero_si256, _mm256_shuffle_epi8, _mm256_srli_epi16, _mm256_xor_si256,
    };
    // SAFETY: `scan` calls this only after `best_kernel` detected AVX2 and POPCNT. The loads read four `u64` lanes at a
    // query position below `qhi`, and head0/head1 hold four zeros past the last entry, so every load is in bounds.
    unsafe {
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
        for v in buckets {
            let (qlo, qhi) = (st.query.offsets[v], st.query.offsets[v + 1]);
            if qlo == qhi {
                continue;
            }
            for p in 0..17 {
                let b = if p == 0 { v } else { v ^ (1 << (p - 1)) };
                for x in &st.stored.entries[st.stored.offsets[b]..st.stored.offsets[b + 1]] {
                    let [x0, x1, _, _] = x.words;
                    let vx0 = _mm256_set1_epi64x(i64::from_ne_bytes(x0.to_ne_bytes()));
                    let vx1 = _mm256_set1_epi64x(i64::from_ne_bytes(x1.to_ne_bytes()));
                    let mut y = qlo;
                    while y < qhi {
                        let a0 = _mm256_loadu_si256(st.query.head0.as_ptr().add(y).cast::<__m256i>());
                        let a1 = _mm256_loadu_si256(st.query.head1.as_ptr().add(y).cast::<__m256i>());
                        let bytes = _mm256_add_epi8(
                            popcount_bytes(_mm256_xor_si256(a0, vx0)),
                            popcount_bytes(_mm256_xor_si256(a1, vx1)),
                        );
                        // Per 64-bit lane, the sum of its eight byte counts: the distance over the first two words.
                        let head = _mm256_sad_epu8(bytes, zero);
                        let over = _mm256_movemask_pd(_mm256_castsi256_pd(_mm256_cmpgt_epi64(head, thr)));
                        let near = u32::try_from(!over & 0xF).unwrap_or(0);
                        let mut lanes = near;
                        while lanes != 0 {
                            let lane = usize::try_from(lanes.trailing_zeros()).unwrap_or(0);
                            lanes &= lanes - 1;
                            if let Some(query) = st.query.entries.get(y + lane)
                                && y + lane < qhi
                            {
                                consider(st, x, query, out);
                            }
                        }
                        y += 4;
                    }
                }
            }
        }
    }
}

/// Runs one chunk's stage on `kernel`, over the buckets in blocks, on `threads` workers. The pairs found, compacted; or
/// `None` when `control` stopped it.
fn run_stage(
    kernel: Kernel,
    st: &Stage<'_>,
    threads: usize,
    start: &(dyn Fn() + Sync),
    control: &(dyn Fn() -> bool + Sync),
) -> Option<Vec<Pair>> {
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let mut all = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    start();
                    let mut out = Vec::new();
                    loop {
                        if stopped.load(Relaxed) || !control() {
                            stopped.store(true, Relaxed);
                            break;
                        }
                        let lo = next.fetch_add(1, Relaxed).saturating_mul(BLOCK);
                        if lo >= VALUES {
                            break;
                        }
                        scan(kernel, st, lo..(lo + BLOCK).min(VALUES), &mut out);
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
fn scan(kernel: Kernel, st: &Stage<'_>, buckets: Range<usize>, out: &mut Vec<Pair>) {
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
    }

    /// A picture from its four stored orientations: the other four are their transposes.
    fn picture(stored: [Hash; 4], due: bool) -> Picture {
        let [a, b, c, d] = stored;
        Picture { orientations: [a, b, c, d, transpose(&a), transpose(&b), transpose(&c), transpose(&d)], due }
    }

    /// A picture whose eight orientations are unrelated (what the join must still get right).
    fn unrelated(rng: &mut Rng, due: bool) -> Picture {
        Picture { orientations: std::array::from_fn(|_| rng.hash()), due }
    }

    /// `n` pictures: random ones, and copies of earlier ones with a few bits flipped in every orientation (so they match
    /// near the radius), and some at exact distances around it.
    fn corpus(rng: &mut Rng, n: usize, due_in_ten: usize) -> Vec<Picture> {
        let mut bases: Vec<[Hash; 4]> = Vec::new();
        let mut out = Vec::new();
        for _ in 0..n {
            let stored = if bases.is_empty() || rng.below(3) == 0 {
                [rng.hash(), rng.hash(), rng.hash(), rng.hash()]
            } else {
                let base = bases[rng.below(bases.len())];
                let flips = rng.below(48);
                let mut s = base;
                for h in &mut s {
                    for _ in 0..flips {
                        *h = flip(h, rng.below(256));
                    }
                }
                s
            };
            bases.push(stored);
            out.push(picture(stored, rng.below(10) < due_in_ten));
        }
        out
    }

    /// The pairs the definition says, over the 8 x 8 orientations, for the pictures whose query side is in `queries`.
    fn brute(pictures: &[Picture], queries: Range<usize>) -> Vec<Pair> {
        let mut out = Vec::new();
        for i in 0..pictures.len() {
            for j in i + 1..pictures.len() {
                let (pi, pj) = (&pictures[i], &pictures[j]);
                if !pi.due && !pj.due {
                    continue;
                }
                let query = if pj.due { j } else { i };
                if !queries.contains(&query) {
                    continue;
                }
                let best = pi
                    .orientations
                    .iter()
                    .flat_map(|a| pj.orientations.iter().map(move |b| a.distance(b)))
                    .min()
                    .unwrap();
                if best <= MATCH_DISTANCE {
                    out.push(Pair { a: u32::try_from(i).unwrap(), b: u32::try_from(j).unwrap(), distance: best });
                }
            }
        }
        out.sort_unstable();
        out
    }

    fn run(kernel: Kernel, pictures: &[Picture], queries: Range<usize>, threads: usize) -> Vec<Pair> {
        join_with(kernel, pictures, queries, threads, &|| {}, &|| true).unwrap().unwrap()
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
    fn transpose_is_an_involution_and_the_symmetry_is_detected() {
        let mut rng = Rng(7);
        for _ in 0..50 {
            let h = rng.hash();
            assert_eq!(transpose(&transpose(&h)), h);
        }
        let symmetric = picture([rng.hash(), rng.hash(), rng.hash(), rng.hash()], true);
        assert!(is_symmetric(&symmetric.orientations));
        assert!(!is_symmetric(&unrelated(&mut rng, true).orientations));
    }

    #[test]
    fn a_picture_without_the_symmetry_is_still_exact() {
        let mut rng = Rng(77);
        let mut pictures = corpus(&mut rng, 200, 6);
        for (slot, picture) in pictures.iter_mut().enumerate() {
            if slot % 5 == 0 {
                let due = picture.due;
                *picture = unrelated(&mut rng, due);
            }
        }
        // Near-copies of unrelated pictures too: their orientations match another picture's only through the join.
        let copy = pictures[0].orientations;
        pictures.push(Picture { orientations: copy.map(|h| flip(&h, 3)), due: true });
        let want = brute(&pictures, 0..pictures.len());
        for kernel in kernels() {
            assert_eq!(run(kernel, &pictures, 0..pictures.len(), 2), want, "{kernel:?}");
        }
    }

    #[test]
    fn matches_brute_force_at_every_due_share_and_kernel() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for due_in_ten in [10, 5, 1] {
            let pictures = corpus(&mut rng, 400, due_in_ten);
            let want = brute(&pictures, 0..pictures.len());
            assert!(want.len() > 10, "the corpus must have pairs to find");
            for kernel in kernels() {
                assert_eq!(run(kernel, &pictures, 0..pictures.len(), 3), want, "{kernel:?}, due {due_in_ten} in 10");
            }
        }
    }

    #[test]
    fn slices_of_the_queries_add_up_to_the_whole() {
        let mut rng = Rng(42);
        let pictures = corpus(&mut rng, 300, 6);
        let whole = run(best_kernel(), &pictures, 0..pictures.len(), 2);
        assert_eq!(whole, brute(&pictures, 0..pictures.len()));
        let mut parts = Vec::new();
        for slice in [0..77, 77..150, 150..300] {
            let got = run(best_kernel(), &pictures, slice.clone(), 2);
            assert_eq!(got, brute(&pictures, slice));
            parts.extend(got);
        }
        parts.sort_unstable();
        assert_eq!(parts, whole);
    }

    #[test]
    fn threads_do_not_change_the_pairs() {
        let mut rng = Rng(5);
        let pictures = corpus(&mut rng, 250, 10);
        let one = run(best_kernel(), &pictures, 0..pictures.len(), 1);
        for threads in [2, 4, 12] {
            assert_eq!(run(best_kernel(), &pictures, 0..pictures.len(), threads), one);
        }
    }

    #[test]
    fn the_radius_is_inclusive_and_exact() {
        let mut rng = Rng(11);
        let base = [rng.hash(), rng.hash(), rng.hash(), rng.hash()];
        // Only the first orientation is near: the others are unrelated, so no other combination is within the radius.
        let unrelated_rest = [rng.hash(), rng.hash(), rng.hash()];
        for (flips, found) in [(31usize, true), (32, false)] {
            let mut first = base[0];
            for k in 0..flips {
                first = flip(&first, k * 7 + 1);
            }
            let moved = [first, unrelated_rest[0], unrelated_rest[1], unrelated_rest[2]];
            let pictures = [picture(base, true), picture(moved, true)];
            let want = brute(&pictures, 0..2);
            assert_eq!(!want.is_empty(), found, "brute force at {flips} flips");
            for kernel in kernels() {
                assert_eq!(run(kernel, &pictures, 0..2, 1), want, "{kernel:?} at {flips} flips");
            }
        }
    }

    /// The full library's join, from a dump of a catalog's hashes (`PG_JOIN_DUMP`: records of an id as `i64` little-endian,
    /// a due byte and the eight orientations, 265 bytes, by id). The pairs go to `PG_JOIN_OUT` as `id_a id_b distance`.
    /// `PG_JOIN_THREADS` sets the workers (default: the processor's). Prints the timing of each kernel on one thread,
    /// then of the best kernel on the workers.
    #[test]
    #[ignore = "needs a catalog dump: PG_JOIN_DUMP and PG_JOIN_OUT (see the test's doc comment)"]
    fn dump_join() {
        use std::io::Write;
        let dump = std::fs::read(std::env::var("PG_JOIN_DUMP").unwrap()).unwrap();
        const RECORD: usize = 8 + 1 + 256;
        let mut ids = Vec::new();
        let mut pictures = Vec::new();
        for rec in dump.as_chunks::<RECORD>().0 {
            ids.push(i64::from_le_bytes(rec[..8].try_into().unwrap()));
            let orientations: [Hash; 8] =
                std::array::from_fn(|v| Hash(rec[9 + 32 * v..9 + 32 * v + 32].try_into().unwrap()));
            pictures.push(Picture { orientations, due: rec[8] != 0 });
        }
        let n = pictures.len();
        let symmetric = pictures.iter().filter(|p| is_symmetric(&p.orientations)).count();
        println!("pictures {n}, symmetric {symmetric}, due {}", pictures.iter().filter(|p| p.due).count());
        let threads: usize = std::env::var("PG_JOIN_THREADS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |p| p.get()));
        let never = || true;
        for kernel in kernels() {
            let t = std::time::Instant::now();
            let pairs = join_with(kernel, &pictures, 0..n, 1, &|| {}, &never).unwrap().unwrap();
            println!("{kernel:?} 1 thread: {:?}, {} pairs", t.elapsed(), pairs.len());
        }
        let t = std::time::Instant::now();
        let pairs = join_with(best_kernel(), &pictures, 0..n, threads, &|| {}, &never).unwrap().unwrap();
        println!("{:?} {threads} threads: {:?}, {} pairs", best_kernel(), t.elapsed(), pairs.len());
        let mut out = std::io::BufWriter::new(std::fs::File::create(std::env::var("PG_JOIN_OUT").unwrap()).unwrap());
        for p in &pairs {
            let (a, b) = (ids[index(p.a)], ids[index(p.b)]);
            writeln!(out, "{a} {b} {}", p.distance).unwrap();
        }
    }

    #[test]
    fn a_stop_returns_none() {
        let mut rng = Rng(3);
        let pictures = corpus(&mut rng, 50, 10);
        let got = join_with(best_kernel(), &pictures, 0..50, 2, &|| {}, &|| false).unwrap();
        assert_eq!(got, None);
    }
}
