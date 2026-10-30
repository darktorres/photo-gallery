//! Exact PDQ radius search by multi-index hashing (PLAN §5 tier 1).
//!
//! The 256-bit hash is split into 16 chunks of 16 bits, each with its own table. Two hashes within distance
//! d ≤ 31 differ in at most one bit in at least one chunk (pigeonhole: 16 chunks × 2 bits > 31), so probing each
//! chunk's exact value and its 16 one-bit neighbours finds every match, then the full distance filters the
//! candidates. Rebuilt in RAM from the catalog at startup; queried then inserted during the import (Pinterest's
//! new-vs-old pattern), so there is never a separate all-pairs job.

use crate::pdq::Hash;

const CHUNKS: usize = 16;
/// The largest radius the one-bit probes guarantee to find.
pub const MAX_RADIUS: u32 = 2 * CHUNKS as u32 - 1;

pub struct PdqIndex {
    ids: Vec<i64>,
    hashes: Vec<[u64; 4]>,
    /// tables[chunk][value] = positions in `ids`.
    tables: Vec<Vec<Vec<u32>>>,
    /// Per-entry stamp of the last query that saw it, to skip duplicate candidates without a set.
    seen: Vec<u32>,
    query_no: u32,
}

impl Default for PdqIndex {
    fn default() -> Self {
        PdqIndex {
            ids: Vec::new(),
            hashes: Vec::new(),
            tables: (0..CHUNKS).map(|_| vec![Vec::new(); 1 << 16]).collect(),
            seen: Vec::new(),
            query_no: 0,
        }
    }
}

fn chunk(words: &[u64; 4], c: usize) -> usize {
    ((words[c / 4] >> ((c % 4) * 16)) & 0xFFFF) as usize
}

impl PdqIndex {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn insert(&mut self, id: i64, hash: &Hash) {
        let pos = self.ids.len() as u32;
        let words = hash.words();
        for c in 0..CHUNKS {
            self.tables[c][chunk(&words, c)].push(pos);
        }
        self.ids.push(id);
        self.hashes.push(words);
        self.seen.push(0);
    }

    /// Every entry within `radius` (≤ [`MAX_RADIUS`]) of `hash`, as (id, distance).
    pub fn query(&mut self, hash: &Hash, radius: u32) -> Vec<(i64, u32)> {
        assert!(radius <= MAX_RADIUS);
        self.query_no = self.query_no.wrapping_add(1);
        if self.query_no == 0 {
            self.seen.fill(0);
            self.query_no = 1;
        }
        let words = hash.words();
        let mut out = Vec::new();
        for c in 0..CHUNKS {
            let v = chunk(&words, c);
            for probe in std::iter::once(v).chain((0..16).map(|b| v ^ (1 << b))) {
                for &pos in &self.tables[c][probe] {
                    let p = pos as usize;
                    if self.seen[p] == self.query_no {
                        continue;
                    }
                    self.seen[p] = self.query_no;
                    let e = &self.hashes[p];
                    let d: u32 = (0..4).map(|i| (e[i] ^ words[i]).count_ones()).sum();
                    if d <= radius {
                        out.push((self.ids[p], d));
                    }
                }
            }
        }
        out
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

    #[test]
    fn finds_exactly_the_brute_force_set() {
        let mut s = 0x9E3779B97F4A7C15u64;
        let base: Vec<Hash> = (0..300).map(|_| rand_hash(&mut s)).collect();
        let mut index = PdqIndex::default();
        let mut all = Vec::new();
        for (i, b) in base.iter().enumerate() {
            // Each base hash plus copies with 0..=40 random bit flips.
            for flips in [0, 5, 16, 24, 31, 32, 40] {
                let mut h = *b;
                for _ in 0..flips {
                    let bit = rand_hash(&mut s).0[0] as usize;
                    h.0[bit / 8] ^= 1 << (bit % 8);
                }
                let id = all.len() as i64;
                index.insert(id, &h);
                all.push((i, h));
            }
        }
        for q in base.iter().take(50) {
            let mut got = index.query(q, MAX_RADIUS);
            got.sort();
            let mut want: Vec<(i64, u32)> = all
                .iter()
                .enumerate()
                .map(|(id, (_, h))| (id as i64, q.distance(h)))
                .filter(|&(_, d)| d <= MAX_RADIUS)
                .collect();
            want.sort();
            assert_eq!(got, want);
        }
    }
}
