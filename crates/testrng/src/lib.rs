//! A deterministic generator for the tests' random cases: one seed draws the same cases on every run.
//!
//! The one generator every test uses (SplitMix64). It is a leaf with no dependencies: the layered crates take it as a
//! dev-dependency, and `pg-testkit` re-exports it, so the cases of every test come from the same mixing function.

/// A deterministic generator (SplitMix64): the same seed gives the same sequence on every run.
pub struct Rng(u64);

impl Rng {
    /// A generator from `seed`.
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n` (a range of at least one).
    pub fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(n.max(1)).unwrap_or(1)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn the_same_seed_draws_the_same_sequence() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        assert!((0..100).all(|_| a.next_u64() == b.next_u64()));
    }

    #[test]
    fn below_stays_below_and_a_range_of_one_is_zero() {
        let mut rng = Rng::new(3);
        assert!((0..1000).all(|_| rng.below(10) < 10));
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.below(1), 0);
    }
}
