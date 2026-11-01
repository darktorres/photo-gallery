//! Exact PDQ radius search by multi-index hashing (PLAN §5 tier 1).
//!
//! The 256-bit hash is split into 16 chunks of 16 bits, each with its own table. Two hashes within distance
//! d ≤ 31 differ in at most one bit in at least one chunk (pigeonhole: 16 chunks × 2 bits > 31), so probing each
//! chunk's exact value and its 16 one-bit neighbours finds every match, then the full distance filters the
//! candidates. Rebuilt in RAM from the catalog at startup; queried then inserted during the import (Pinterest's
//! new-vs-old pattern), so there is never a separate all-pairs job.
//!
//! Each chunk's table is flat: the entries' positions grouped by chunk value, with each value's range in
//! `offsets`, built by a counting sort. Entries inserted since are chained per value (a head per value, a link per
//! entry), so an insert is O(1). Once the chains hold an eighth as many entries as the flat tables (at least
//! [`MIN_REINDEX`]), the flat tables are rebuilt over every entry: each entry is placed a bounded number of times
//! (amortized O(1)) however large the index grows, and an empty catalog's first import stays fast. An index without
//! entries holds no table.

use crate::pdq::Hash;

const CHUNKS: usize = 16;
/// The largest radius the one-bit probes guarantee to find.
pub const MAX_RADIUS: u32 = 2 * CHUNKS as u32 - 1;
const VALUES: usize = 1 << 16;
/// The fewest chained entries a rebuild of the flat tables waits for.
const MIN_REINDEX: usize = 4096;
/// The end of a chain. Heads and links hold an entry's position + 1.
const END: u32 = 0;

#[derive(Default)]
pub struct PdqIndex {
    ids: Vec<i64>,
    hashes: Vec<[u64; 4]>,
    /// How many entries (the first ones) are in the flat tables; the rest are chained.
    indexed: usize,
    chunks: [Chunk; CHUNKS],
    /// Per-entry stamp of the last query that saw it, to skip duplicate candidates without a set.
    seen: Vec<u32>,
    query_no: u32,
}

#[derive(Default)]
struct Chunk {
    /// `positions[offsets[v]..offsets[v + 1]]`: the indexed entries whose chunk value is `v`, in insertion order.
    offsets: Vec<u32>,
    positions: Vec<u32>,
    /// The entries inserted since: `head[v]` links to the newest whose value is `v`, and `next[i]` from entry
    /// `indexed + i` to the one before it. Allocated (zeroed, so only its touched pages are committed) by the
    /// first insert.
    head: Vec<u32>,
    next: Vec<u32>,
}

fn value(words: &[u64; 4], c: usize) -> u16 {
    (words[c / 4] >> ((c % 4) * 16)) as u16
}

impl PdqIndex {
    /// The index of these entries: `ids[i]`'s hash words are `hashes[i]`.
    pub fn build(ids: Vec<i64>, hashes: Vec<[u64; 4]>) -> PdqIndex {
        assert_eq!(ids.len(), hashes.len());
        let mut index = PdqIndex { seen: vec![0; ids.len()], ids, hashes, ..PdqIndex::default() };
        index.reindex();
        index
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn insert(&mut self, id: i64, hash: &Hash) {
        let words = hash.words();
        let link = u32::try_from(self.ids.len() + 1).expect("PDQ index of 2³² entries");
        for (c, chunk) in self.chunks.iter_mut().enumerate() {
            if chunk.head.is_empty() {
                chunk.head = vec![END; VALUES];
            }
            let v = value(&words, c) as usize;
            chunk.next.push(chunk.head[v]);
            chunk.head[v] = link;
        }
        self.ids.push(id);
        self.hashes.push(words);
        self.seen.push(0);
        if self.ids.len() - self.indexed >= MIN_REINDEX.max(self.indexed / 8) {
            self.reindex();
        }
    }

    /// Put every entry in the flat tables, by a counting sort per chunk, and empty the chains.
    fn reindex(&mut self) {
        let n = self.hashes.len();
        if n == 0 {
            return;
        }
        for (c, chunk) in self.chunks.iter_mut().enumerate() {
            let offsets = &mut chunk.offsets;
            offsets.clear();
            offsets.resize(VALUES + 1, 0);
            for h in &self.hashes {
                offsets[value(h, c) as usize] += 1;
            }
            // Each value's end, then (placing back to front) its start.
            let mut end = 0;
            for o in &mut offsets[..VALUES] {
                end += *o;
                *o = end;
            }
            offsets[VALUES] = end;
            chunk.positions.clear();
            chunk.positions.resize(n, 0);
            for (p, h) in self.hashes.iter().enumerate().rev() {
                let v = value(h, c) as usize;
                offsets[v] -= 1;
                chunk.positions[offsets[v] as usize] = p as u32;
            }
            chunk.head.fill(END);
            chunk.next.clear();
        }
        self.indexed = n;
    }

    /// Every entry within `radius` (≤ [`MAX_RADIUS`]) of `hash`, as (id, distance), into `out` (cleared first), in
    /// no particular order.
    pub fn query(&mut self, hash: &Hash, radius: u32, out: &mut Vec<(i64, u32)>) {
        assert!(radius <= MAX_RADIUS);
        out.clear();
        self.query_no = self.query_no.wrapping_add(1);
        if self.query_no == 0 {
            self.seen.fill(0);
            self.query_no = 1;
        }
        let PdqIndex { ids, hashes, indexed, chunks, seen, query_no } = self;
        let words = hash.words();
        let mut check = |p: usize| {
            if seen[p] == *query_no {
                return;
            }
            seen[p] = *query_no;
            let e = &hashes[p];
            let d: u32 = (0..4).map(|i| (e[i] ^ words[i]).count_ones()).sum();
            if d <= radius {
                out.push((ids[p], d));
            }
        };
        for (c, chunk) in chunks.iter().enumerate() {
            let v = value(&words, c);
            for probe in std::iter::once(v).chain((0..16).map(|b| v ^ (1 << b))) {
                let probe = probe as usize;
                if let Some(&[start, end]) = chunk.offsets.get(probe..probe + 2) {
                    chunk.positions[start as usize..end as usize].iter().for_each(|&p| check(p as usize));
                }
                let mut link = chunk.head.get(probe).copied().unwrap_or(END);
                while link != END {
                    let p = link as usize - 1;
                    check(p);
                    link = chunk.next[p - *indexed];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rand_hash(state: &mut u64) -> Hash {
        let mut h = [0u8; 32];
        for b in h.iter_mut() {
            *state ^= *state << 13;
            *state ^= *state >> 7;
            *state ^= *state << 17;
            *b = *state as u8;
        }
        Hash(h)
    }

    /// `bases` random hashes, each with copies at 5 to 40 random bit flips, and the queries: every 10th base hash
    /// and every 25th copy.
    fn sample(bases: usize) -> (Vec<Hash>, Vec<Hash>) {
        let mut s = 0x9E3779B97F4A7C15u64;
        let mut all = Vec::new();
        for _ in 0..bases {
            let base = rand_hash(&mut s);
            for flips in [0, 5, 16, 24, 31, 32, 40] {
                let mut h = base;
                for _ in 0..flips {
                    let bit = rand_hash(&mut s).0[0] as usize;
                    h.0[bit / 8] ^= 1 << (bit % 8);
                }
                all.push(h);
            }
        }
        let queries = all.iter().step_by(70).chain(all.iter().skip(3).step_by(25)).copied().collect();
        (all, queries)
    }

    /// Each query's results are the brute-force set over the first `index.len()` hashes, ids being positions.
    fn check(index: &mut PdqIndex, all: &[Hash], queries: &[Hash], out: &mut Vec<(i64, u32)>) {
        for q in queries {
            index.query(q, MAX_RADIUS, out);
            out.sort();
            let want: Vec<(i64, u32)> = all[..index.len()]
                .iter()
                .enumerate()
                .map(|(id, h)| (id as i64, q.distance(h)))
                .filter(|&(_, d)| d <= MAX_RADIUS)
                .collect();
            assert_eq!(*out, want, "{} entries, {} indexed", index.len(), index.indexed);
        }
    }

    #[test]
    fn a_built_index_finds_exactly_the_brute_force_set() {
        let (all, queries) = sample(1100);
        let mut index = PdqIndex::build((0..all.len() as i64).collect(), all.iter().map(Hash::words).collect());
        assert_eq!(index.indexed, all.len());
        check(&mut index, &all, &queries, &mut Vec::new());
    }

    #[test]
    fn inserts_are_found_before_and_after_a_reindex() {
        let (all, queries) = sample(1100);
        let mut index = PdqIndex::default();
        let mut out = Vec::new();
        check(&mut index, &all, &queries, &mut out);
        for (id, h) in all.iter().enumerate() {
            index.insert(id as i64, h);
            if [1, 100, MIN_REINDEX - 1, MIN_REINDEX, MIN_REINDEX + 1, all.len()].contains(&index.len()) {
                check(&mut index, &all, &queries, &mut out);
            }
        }
        // One reindex, at MIN_REINDEX: the rest are chained.
        assert_eq!(index.indexed, MIN_REINDEX);
    }

    #[test]
    fn inserts_into_a_built_index_are_found_across_reindexes() {
        let (all, queries) = sample(1300);
        let built = 3000;
        let mut index = PdqIndex::build((0..built as i64).collect(), all[..built].iter().map(Hash::words).collect());
        let mut out = Vec::new();
        for (id, h) in all.iter().enumerate().skip(built) {
            index.insert(id as i64, h);
            let chained = index.len() - index.indexed;
            if [1, 2000, MIN_REINDEX - 1].contains(&chained) || index.len() == all.len() {
                check(&mut index, &all, &queries, &mut out);
            }
        }
        // Reindexed at 3000 + 4096; the last 2004 are chained.
        assert_eq!((index.indexed, index.len()), (built + MIN_REINDEX, all.len()));
    }
}
