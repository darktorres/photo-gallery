//! Exact PDQ radius search by multi-index hashing (PLAN §5 tier 1).
//!
//! The 256-bit hash is split into 16 chunks of 16 bits, each with its own table. Two hashes within distance
//! d ≤ 31 differ in at most one bit in at least one chunk (pigeonhole: 16 chunks × 2 bits > 31), so probing each
//! chunk's exact value and its 16 one-bit neighbours finds every match, then the full distance filters the
//! candidates. Rebuilt in RAM from the catalog at startup; queried then inserted during the import (Pinterest's
//! new-vs-old pattern), so there is never a separate all-pairs job.
//!
//! Each chunk's table is flat: the entries' positions grouped by chunk value, with each value's range in
//! `offsets`, built by a counting sort. Entries inserted since are chained per value (a link per entry, and the
//! newest of each value in `Heads`, whose size follows the values chained, not the 65,536 there are), so an insert
//! is O(1). Once the chains hold an eighth as many entries as the flat tables (at least `MIN_REINDEX`), the flat
//! tables are rebuilt over every entry: each entry is placed a bounded number of times (amortized O(1)) however
//! large the index grows, and an empty catalog's first import stays fast. An index without entries holds no table.

use pg_model::ids::FileId;
use pg_model::pdq::Hash;

/// `CHUNK_COUNT` as a `usize`, for the tables and loops: a const can't convert the u32, so both are written out.
const CHUNKS: usize = 16;
/// `CHUNKS`, as the radius counts.
const CHUNK_COUNT: u32 = 16;
/// The largest radius the one-bit probes guarantee to find.
pub const MAX_RADIUS: u32 = 2 * CHUNK_COUNT - 1;
const VALUES: usize = 1 << 16;
/// The fewest chained entries a rebuild of the flat tables waits for.
const MIN_REINDEX: usize = 4096;
/// The end of a chain. Heads and links hold an entry's position + 1.
const END: u32 = 0;
/// The most slots [`Heads`] keeps as a map: past that, a table of every value (`VALUES` heads) is smaller.
const MAP_MAX_SLOTS: usize = 1 << 15;

/// An insert into an index that holds as many entries as its links can count (2³² − 1): nothing was inserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexFull;

impl std::fmt::Display for IndexFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the PDQ index is full (2³² − 1 entries)")
    }
}

impl std::error::Error for IndexFull {}

#[derive(Default)]
pub struct PdqIndex {
    ids: Vec<FileId>,
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
    /// The entries inserted since: `heads` links each value to the newest whose value it is, and `next[i]` entry
    /// `indexed + i` to the one before it.
    heads: Heads,
    next: Vec<u32>,
}

/// The newest chained entry of each chunk value (its link: position + 1). A map while few values are chained: open
/// addressing, at most half full, so it holds about two slots per value; once it would outgrow [`MAP_MAX_SLOTS`],
/// a table of every value, which is then smaller.
enum Heads {
    Map { keys: Vec<u16>, links: Vec<u32>, len: usize },
    Table(Vec<u32>),
}

impl Default for Heads {
    fn default() -> Self {
        Heads::Map { keys: Vec::new(), links: Vec::new(), len: 0 }
    }
}

/// Where `v` is, or would go, in a map of `keys`/`links` (as long as each other, a power of two long, never full).
fn map_slot(keys: &[u16], links: &[u32], v: u16) -> usize {
    let mask = keys.len() - 1;
    let mut i = pg_model::num::index(u32::from(v).wrapping_mul(0x9E37_79B1) >> 16) & mask;
    while let (Some(&link), Some(&key)) = (links.get(i), keys.get(i))
        && link != END
        && key != v
    {
        i = (i + 1) & mask;
    }
    i
}

impl Heads {
    fn get(&self, v: u16) -> u32 {
        match self {
            Heads::Map { keys, .. } if keys.is_empty() => END,
            Heads::Map { keys, links, .. } => links.get(map_slot(keys, links, v)).copied().unwrap_or(END),
            Heads::Table(heads) => heads.get(usize::from(v)).copied().unwrap_or(END),
        }
    }

    fn set(&mut self, v: u16, link: u32) {
        if let Heads::Map { keys, links, .. } = self
            && !keys.is_empty()
        {
            let i = map_slot(keys, links, v);
            if let Some(l) = links.get_mut(i)
                && *l != END
            {
                *l = link;
                return;
            }
        }
        // A value not chained yet: room for it first (which may make the table).
        self.reserve(1);
        match self {
            Heads::Map { keys, links, len } => {
                let i = map_slot(keys, links, v);
                if let (Some(key), Some(l)) = (keys.get_mut(i), links.get_mut(i)) {
                    (*key, *l) = (v, link);
                    *len += 1;
                }
            }
            Heads::Table(heads) => {
                if let Some(head) = heads.get_mut(usize::from(v)) {
                    *head = link;
                }
            }
        }
    }

    /// Room for `additional` more values without growing.
    fn reserve(&mut self, additional: usize) {
        let Heads::Map { keys, links, len } = self else { return };
        let want = ((*len + additional) * 2).next_power_of_two().max(16);
        if want <= keys.len() {
            return;
        }
        let old: Vec<(u16, u32)> =
            (keys.iter().zip(links.iter())).filter(|(_, l)| **l != END).map(|(k, l)| (*k, *l)).collect();
        if want > MAP_MAX_SLOTS {
            let mut table = vec![END; VALUES];
            for (k, l) in old {
                if let Some(head) = table.get_mut(usize::from(k)) {
                    *head = l;
                }
            }
            *self = Heads::Table(table);
            return;
        }
        *keys = vec![0; want];
        *links = vec![END; want];
        for (k, l) in old {
            let i = map_slot(keys, links, k);
            if let (Some(key), Some(link)) = (keys.get_mut(i), links.get_mut(i)) {
                (*key, *link) = (k, l);
            }
        }
    }

    /// No value chained; the room is kept.
    fn clear(&mut self) {
        match self {
            Heads::Map { links, len, .. } => {
                links.fill(END);
                *len = 0;
            }
            Heads::Table(heads) => heads.fill(END),
        }
    }
}

/// Chunk `c`'s value: bits `16c..16c + 16` of the hash (zero past its 256 bits).
fn value(words: &[u64; 4], c: usize) -> u16 {
    words.get(c / 4).map_or(0, |&w| u16::try_from((w >> ((c % 4) * 16)) & 0xFFFF).unwrap_or(u16::MAX))
}

impl PdqIndex {
    /// The index of these entries: `ids[i]`'s hash words are `hashes[i]`.
    pub fn build(ids: Vec<FileId>, hashes: Vec<[u64; 4]>) -> PdqIndex {
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

    /// Room for `additional` more entries: the import reserves its whole work list once, so neither the entries
    /// nor the chains grow per insert (the chains only up to the next reindex).
    pub fn reserve(&mut self, additional: usize) {
        self.ids.reserve(additional);
        self.hashes.reserve(additional);
        self.seen.reserve(additional);
        let chained = additional.min(MIN_REINDEX.max(self.indexed / 8));
        for chunk in &mut self.chunks {
            chunk.next.reserve(chained);
            chunk.heads.reserve(chained.min(VALUES));
        }
    }

    /// Store `hash` under `id`. [`IndexFull`], and nothing stored, when the index holds 2³² − 1 entries already.
    pub fn insert(&mut self, id: FileId, hash: &Hash) -> Result<(), IndexFull> {
        let words = hash.words();
        let link = u32::try_from(self.ids.len() + 1).map_err(|_| IndexFull)?;
        for (c, chunk) in self.chunks.iter_mut().enumerate() {
            let v = value(&words, c);
            chunk.next.push(chunk.heads.get(v));
            chunk.heads.set(v, link);
        }
        self.ids.push(id);
        self.hashes.push(words);
        self.seen.push(0);
        if self.ids.len() - self.indexed >= MIN_REINDEX.max(self.indexed / 8) {
            self.reindex();
        }
        Ok(())
    }

    /// Put every entry in the flat tables, by a counting sort per chunk, and empty the chains.
    fn reindex(&mut self) {
        let n = self.hashes.len();
        if n == 0 {
            return;
        }
        // Positions are counted as `u32`s. An index past 2³² − 1 entries (only `build` could make one, and its
        // links couldn't count them) places just the first that many.
        let count = u32::try_from(n).unwrap_or(u32::MAX);
        for (c, chunk) in self.chunks.iter_mut().enumerate() {
            let offsets = &mut chunk.offsets;
            offsets.clear();
            offsets.resize(VALUES + 1, 0);
            for h in &self.hashes {
                if let Some(o) = offsets.get_mut(usize::from(value(h, c))) {
                    *o += 1;
                }
            }
            // Each value's end, then (placing back to front) its start. The last offset counted no value, so it
            // ends as the total.
            let mut end = 0;
            for o in offsets.iter_mut() {
                end += *o;
                *o = end;
            }
            chunk.positions.clear();
            chunk.positions.resize(n, 0);
            for (p, h) in (0..count).zip(&self.hashes).rev() {
                if let Some(o) = offsets.get_mut(usize::from(value(h, c))) {
                    *o -= 1;
                    if let Some(position) = chunk.positions.get_mut(pg_model::num::index(*o)) {
                        *position = p;
                    }
                }
            }
            chunk.heads.clear();
            chunk.next.clear();
        }
        self.indexed = n;
    }

    /// Store every orientation of an image under its id. A pair matches when some orientation of one is within the
    /// radius of some orientation of the other, so the index needs all of them to find it whichever of the two is
    /// imported first. [`IndexFull`], and none of them stored, when they don't all fit.
    pub fn insert_orientations(&mut self, id: FileId, orientations: &[Hash]) -> Result<(), IndexFull> {
        // The last one's link must fit too: then every link before it does.
        u32::try_from(self.ids.len() + orientations.len()).map_err(|_| IndexFull)?;
        for hash in orientations {
            self.insert(id, hash)?;
        }
        Ok(())
    }

    /// The entries matching any of `orientations` within `radius`: for each neighbour, the least distance and the
    /// index of the orientation it matched, in `best`. `found` and `best` are the caller's buffers (cleared here).
    pub fn best_matches(
        &mut self,
        orientations: &[Hash],
        radius: u32,
        found: &mut Vec<(FileId, u32)>,
        best: &mut Vec<(FileId, u32, usize)>,
    ) {
        best.clear();
        for (v, hash) in orientations.iter().enumerate() {
            self.query(hash, radius, found);
            for &(other, d) in found.iter() {
                match best.iter_mut().find(|b| b.0 == other) {
                    Some(b) if d < b.1 => *b = (other, d, v),
                    Some(_) => {}
                    None => best.push((other, d, v)),
                }
            }
        }
    }

    /// Every entry within `radius` (≤ [`MAX_RADIUS`]) of `hash`, as (id, distance), into `out` (cleared first), in
    /// no particular order.
    pub fn query(&mut self, hash: &Hash, radius: u32, out: &mut Vec<(FileId, u32)>) {
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
            let (Some(stamp), Some(e), Some(&id)) = (seen.get_mut(p), hashes.get(p), ids.get(p)) else {
                return;
            };
            if *stamp == *query_no {
                return;
            }
            *stamp = *query_no;
            let d: u32 = e.iter().zip(&words).map(|(a, b)| (a ^ b).count_ones()).sum();
            if d <= radius {
                out.push((id, d));
            }
        };
        for (c, chunk) in chunks.iter().enumerate() {
            let v = value(&words, c);
            for probe in std::iter::once(v).chain((0..16).map(|b| v ^ (1 << b))) {
                let at = usize::from(probe);
                if let Some(&[start, end]) = chunk.offsets.get(at..at + 2)
                    && let Some(positions) = chunk.positions.get(pg_model::num::index(start)..pg_model::num::index(end))
                {
                    positions.iter().for_each(|&p| check(pg_model::num::index(p)));
                }
                let mut link = chunk.heads.get(probe);
                while link != END {
                    let p = pg_model::num::index(link) - 1;
                    check(p);
                    link = chunk.next.get(p - *indexed).copied().unwrap_or(END);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::pdq::MATCH_DISTANCE;

    fn rand_hash(state: &mut u64) -> Hash {
        let mut h = [0u8; 32];
        for b in h.iter_mut() {
            *state ^= *state << 13;
            *state ^= *state >> 7;
            *state ^= *state << 17;
            *b = state.to_le_bytes()[0];
        }
        Hash(h)
    }

    /// The pairs found when the images are imported in `order`, as (smaller, larger) image indices: each one is
    /// queried with all its orientations against the earlier ones' (what the import does).
    fn pairs_in(order: &[usize], images: &[Vec<Hash>]) -> std::collections::BTreeSet<(FileId, FileId)> {
        let mut index = PdqIndex::default();
        let (mut found, mut best) = (Vec::new(), Vec::new());
        let mut pairs = std::collections::BTreeSet::new();
        for &i in order {
            index.best_matches(&images[i], MATCH_DISTANCE, &mut found, &mut best);
            for &(other, _, _) in &best {
                pairs.insert((
                    other.min(FileId::new(i64::try_from(i).unwrap())),
                    other.max(FileId::new(i64::try_from(i).unwrap())),
                ));
            }
            index.insert_orientations(FileId::new(i64::try_from(i).unwrap()), &images[i]).unwrap();
        }
        pairs
    }

    /// Whichever image is imported first, the same pairs are found: a pair matches when some orientation of one is
    /// within the radius of some orientation of the other, and both sides are in the index.
    #[test]
    fn pairs_do_not_depend_on_import_order() {
        let mut s = 0xA5A5_5A5A_1234_5678u64;
        // Each image: eight orientations. Each image gets an orientation that is a near copy of a non-identity
        // orientation of the previous image (within the radius). Neither side's copy is its first orientation, so a
        // pair exists only if the index holds every orientation of the earlier image.
        let mut images: Vec<Vec<Hash>> = (0..40).map(|_| (0..8).map(|_| rand_hash(&mut s)).collect()).collect();
        for i in 1..images.len() {
            let source = i - 1;
            let orientation = 1 + (i * 3) % 7;
            let mut copy = images[source][1 + (i * 5) % 7];
            for _ in 0..(10 + i % 20) {
                let bit = usize::from(rand_hash(&mut s).0[0]);
                copy.0[bit / 8] ^= 1 << (bit % 8);
            }
            images[i][orientation] = copy;
        }
        let n = images.len();
        let forward: Vec<usize> = (0..n).collect();
        let reverse: Vec<usize> = (0..n).rev().collect();
        let mut shuffled = forward.clone();
        for i in (1..n).rev() {
            let j = usize::from(rand_hash(&mut s).0[0]) % (i + 1);
            shuffled.swap(i, j);
        }
        let expected = pairs_in(&forward, &images);
        assert!(!expected.is_empty(), "the test needs pairs to find");
        assert_eq!(pairs_in(&reverse, &images), expected);
        assert_eq!(pairs_in(&shuffled, &images), expected);
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
                    let bit = usize::from(rand_hash(&mut s).0[0]);
                    h.0[bit / 8] ^= 1 << (bit % 8);
                }
                all.push(h);
            }
        }
        let queries = all.iter().step_by(70).chain(all.iter().skip(3).step_by(25)).copied().collect();
        (all, queries)
    }

    /// Each query's results are the brute-force set over the first `index.len()` hashes, ids being positions.
    fn check(index: &mut PdqIndex, all: &[Hash], queries: &[Hash], out: &mut Vec<(FileId, u32)>) {
        for q in queries {
            index.query(q, MAX_RADIUS, out);
            out.sort();
            let want: Vec<(FileId, u32)> = all[..index.len()]
                .iter()
                .enumerate()
                .map(|(id, h)| (FileId::new(i64::try_from(id).unwrap()), q.distance(h)))
                .filter(|&(_, d)| d <= MAX_RADIUS)
                .collect();
            assert_eq!(*out, want, "{} entries, {} indexed", index.len(), index.indexed);
        }
    }

    #[test]
    fn a_built_index_finds_exactly_the_brute_force_set() {
        let (all, queries) = sample(1100);
        let mut index = PdqIndex::build(
            (0..i64::try_from(all.len()).unwrap()).map(FileId::new).collect(),
            all.iter().map(Hash::words).collect(),
        );
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
            index.insert(FileId::new(i64::try_from(id).unwrap()), h).unwrap();
            if [1, 100, MIN_REINDEX - 1, MIN_REINDEX, MIN_REINDEX + 1, all.len()].contains(&index.len()) {
                check(&mut index, &all, &queries, &mut out);
            }
        }
        // One reindex, at MIN_REINDEX: the rest are chained.
        assert_eq!(index.indexed, MIN_REINDEX);
    }

    /// The heads, as a map and once a table, give the newest link of each value, as a table of every value would.
    #[test]
    fn heads_give_each_values_newest_link_as_map_and_table() {
        let mut heads = Heads::default();
        let mut want = vec![END; VALUES];
        let mut s = 0x2545F4914F6CDD1Du64;
        for link in 1..=200_000u32 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            // Few values at first (the map), then all of them (the table).
            let v = if link < 5000 { u16::try_from(s % 3000).unwrap() } else { u16::try_from(s & 0xFFFF).unwrap() };
            heads.set(v, link);
            want[usize::from(v)] = link;
            if link == 4999 {
                assert!(matches!(&heads, Heads::Map { keys, .. } if keys.len() == 8192));
                assert!((0..=u16::MAX).all(|v| heads.get(v) == want[usize::from(v)]));
            }
        }
        assert!(matches!(heads, Heads::Table(_)));
        assert!((0..=u16::MAX).all(|v| heads.get(v) == want[usize::from(v)]));
        heads.clear();
        assert!((0..=u16::MAX).all(|v| heads.get(v) == END));
        // Reserved, a map takes its values without growing.
        let mut heads = Heads::default();
        heads.reserve(1000);
        let Heads::Map { keys, .. } = &heads else { panic!() };
        let slots = keys.len();
        (0..1000).for_each(|v| heads.set(v, u32::from(v) + 1));
        assert!(matches!(&heads, Heads::Map { keys, len: 1000, .. } if keys.len() == slots));
        assert!((0..1000).all(|v| heads.get(v) == u32::from(v) + 1) && heads.get(1000) == END);
    }

    #[test]
    fn a_reserved_index_finds_its_inserts() {
        let (all, queries) = sample(700);
        let mut index = PdqIndex::default();
        index.reserve(all.len());
        for (id, h) in all.iter().enumerate() {
            index.insert(FileId::new(i64::try_from(id).unwrap()), h).unwrap();
        }
        check(&mut index, &all, &queries, &mut Vec::new());
    }

    #[test]
    fn inserts_into_a_built_index_are_found_across_reindexes() {
        let (all, queries) = sample(1300);
        let built = 3000;
        let mut index = PdqIndex::build(
            (0..i64::try_from(built).unwrap()).map(FileId::new).collect(),
            all[..built].iter().map(Hash::words).collect(),
        );
        let mut out = Vec::new();
        for (id, h) in all.iter().enumerate().skip(built) {
            index.insert(FileId::new(i64::try_from(id).unwrap()), h).unwrap();
            let chained = index.len() - index.indexed;
            if [1, 2000, MIN_REINDEX - 1].contains(&chained) || index.len() == all.len() {
                check(&mut index, &all, &queries, &mut out);
            }
        }
        // Reindexed at 3000 + 4096; the last 2004 are chained.
        assert_eq!((index.indexed, index.len()), (built + MIN_REINDEX, all.len()));
    }
}
