// The pairing pass's join (pg_dedup::join, included from linux.rs): the store's build and one join on one thread, over
// a synthetic library made in setup. No catalog: the join takes the hashes and the owed flags, which the pass loads.
// `all` owes every picture (the sorted path: both sides sorted by each chunk, then scanned); `few` owes one in 200, below
// the few-owed threshold at one thread (the stored side is streamed once against the owed pictures' lists). The pass
// runs the join on the team of its workers; one thread keeps the count to the work alone.

use gungraun::Callgrind;
use pg_dedup::join::{Joined, Joiner};
use pg_model::pdq::{Hash, ORIENTATIONS};

/// The library's pictures: large enough that every chunk's buckets hold entries (65,536 values a chunk, 80,000 stored
/// entries), small enough that callgrind runs the sorted path in seconds.
const PICTURES: usize = 20_000;

/// The owed pictures of `few`: one in this many (100 of 20,000).
const FEW_EVERY: usize = 200;

/// A fixed sequence of numbers (xorshift64*), so every run measures the same library.
struct Draws(u64);

impl Draws {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n).unwrap_or(1)).unwrap_or(0)
    }

    fn hash(&mut self) -> Hash {
        let mut bytes = [0u8; 32];
        for chunk in bytes.chunks_mut(8) {
            chunk.copy_from_slice(&self.next().to_le_bytes());
        }
        Hash(bytes)
    }
}

/// The library: a third of the pictures random, the others copies of an earlier one with up to 47 bits flipped in each
/// orientation, so some are pairs within the radius and some just outside it (the tests' corpus, at this size).
fn library() -> Vec<[Hash; ORIENTATIONS]> {
    let mut draws = Draws(0x9E37_79B9_7F4A_7C15);
    let mut hashes: Vec<[Hash; ORIENTATIONS]> = Vec::with_capacity(PICTURES);
    for _ in 0..PICTURES {
        let original = if hashes.is_empty() || draws.below(3) == 0 { None } else { hashes.get(draws.below(hashes.len())) };
        let picture = match original.copied() {
            None => std::array::from_fn(|_| draws.hash()),
            Some(mut copy) => {
                let flips = draws.below(48);
                for hash in &mut copy {
                    for _ in 0..flips {
                        let bit = draws.below(256);
                        if let Some(byte) = hash.0.get_mut(bit / 8) {
                            *byte ^= 1 << (bit % 8);
                        }
                    }
                }
                copy
            }
        };
        hashes.push(picture);
    }
    hashes
}

fn for_join(owed_every: usize) -> (Vec<[Hash; ORIENTATIONS]>, Vec<bool>) {
    let hashes = library();
    let owed = (0..hashes.len()).map(|i| i % owed_every == 0).collect();
    (hashes, owed)
}

// The store's build and the join of every owed picture, on one thread. The join's work runs on its team's thread, which
// gungraun's entry point (a callgrind toggle on this function, per thread) would not count: the second toggle, on the
// function a team thread starts in, counts it too.
#[library_benchmark(
    config = LibraryBenchmarkConfig::default().tool(Callgrind::with_args(["--toggle-collect=*pg_dedup::join::member*"]))
)]
#[bench::all(args = (1), setup = for_join)]
#[bench::few(args = (FEW_EVERY), setup = for_join)]
fn join((hashes, owed): (Vec<[Hash; ORIENTATIONS]>, Vec<bool>)) -> Option<Joined> {
    let joiner = or_stop(Joiner::new(black_box(&hashes), &owed), "the join's store");
    black_box(joiner.join(0..hashes.len(), 1, &|| {}, &|| true, &|| {}))
}

library_benchmark_group!(name = pairing, benchmarks = [join]);
