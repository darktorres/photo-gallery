//! PDQ's model types (tier 1): the 256-bit hash, its quality, and the thresholds the catalog and the grouping use.
//! The hasher that makes them is pg-analysis's `pdq` module.

pub const HASH_BYTES: usize = 32;

/// Meta's recommended match threshold (FINDINGS "PDQ": d ≤ 31 finds 82.8% of the labeled pairs).
pub const MATCH_DISTANCE: u32 = 31;
/// Below this quality the hash is unreliable (flat or near-flat images); Meta's recommendation.
pub const MIN_QUALITY: u8 = 50;
/// [`GROUP_DISTANCE`] as a literal, for SQL made at compile time.
#[macro_export]
macro_rules! group_distance {
    () => {
        22
    };
}
/// Distance up to which a tier-1 pair counts as the same picture when grouping and filtering (FINDINGS "Shortcut and
/// grouping": PDQ ≤ 22 between reliable hashes was right on every pair checked).
pub const GROUP_DISTANCE: u32 = group_distance!();

/// A 256-bit hash in Meta's byte order (byte 0 holds the highest coefficients), so its hex string matches the
/// reference tools.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Hash(pub [u8; HASH_BYTES]);

impl Hash {
    pub fn distance(&self, other: &Hash) -> u32 {
        let (a, b) = (self.words(), other.words());
        a.iter().zip(&b).map(|(x, y)| (x ^ y).count_ones()).sum()
    }

    pub fn words(&self) -> [u64; 4] {
        let mut words = [0; 4];
        for (word, bytes) in words.iter_mut().zip(self.0.as_chunks::<8>().0) {
            *word = u64::from_le_bytes(*bytes);
        }
        words
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pdq {
    pub hash: Hash,
    /// 0–100, Meta's gradient-based quality metric.
    pub quality: u8,
}
