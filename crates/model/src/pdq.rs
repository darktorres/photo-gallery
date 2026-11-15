//! PDQ's model types (tier 1): the 256-bit hash, its quality, and the thresholds the catalog and the grouping use.
//! The hasher that makes them is pg-analysis's `pdq` module.

pub const HASH_BYTES: usize = 32;
/// The orientations a picture's PDQ is kept in: the image as is and its three mirrors (`pg_analysis::pdq`'s variants
/// 0..4). The other four dihedral variants are their transposes ([`Hash::transpose`]), bit for bit, so they are not
/// kept: the DCT of a transposed image is the transposed DCT, and the median the bits are cut at is the same.
pub const ORIENTATIONS: usize = 4;

/// Meta's recommended match threshold (d ≤ 31 finds 82.8% of the labeled pairs).
pub const MATCH_DISTANCE: u32 = 31;
/// Below this quality the hash is unreliable (flat or near-flat images); Meta's recommendation.
pub const MIN_QUALITY: u8 = 50;
/// [`PAIRING_VERSION`] as a literal, for SQL made at compile time (the migration that seeds the catalog's mark).
#[macro_export]
macro_rules! pairing_version {
    () => {
        1
    };
}
/// The version of the pairs the pairing pass makes (what matches within [`MATCH_DISTANCE`], among the reliable hashes):
/// bumped when the matching rule or the pairs' distances change. A catalog whose `meta.pairing_version` is another marks
/// every eligible analysis due, and its pairs go (`pg_catalog::Versions::pairing`).
pub const PAIRING_VERSION: i64 = pairing_version!();
/// [`GROUP_DISTANCE`] as a literal, for SQL made at compile time.
#[macro_export]
macro_rules! group_distance {
    () => {
        22
    };
}
/// Distance up to which a tier-1 pair counts as the same picture when grouping and filtering (PDQ ≤ 22 between reliable
/// hashes was right on every pair checked).
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

    /// The 16 rows of the hash's 16x16 bit grid: row `r` is bytes `2r` and `2r + 1`, most significant first, so bit 15
    /// of a row is its column 0.
    pub fn rows(&self) -> [u16; 16] {
        let mut rows = [0; 16];
        for (row, bytes) in rows.iter_mut().zip(self.0.as_chunks::<2>().0) {
            *row = u16::from_be_bytes(*bytes);
        }
        rows
    }

    /// The hash made of 16 grid rows ([`Hash::rows`]'s inverse).
    pub fn from_rows(rows: &[u16; 16]) -> Hash {
        let mut out = [0u8; HASH_BYTES];
        for (bytes, row) in out.as_chunks_mut::<2>().0.iter_mut().zip(rows) {
            *bytes = row.to_be_bytes();
        }
        Hash(out)
    }

    /// The hash of the same picture transposed: its 16x16 bit grid turned over the main diagonal (row `r`, column `c`
    /// goes to row `c`, column `r`). Orientation `v + 4` of a picture is this of its orientation `v`, bit for bit. Four
    /// rounds of block swaps on the rows (Hacker's Delight, 7-3), about 30 ns.
    pub fn transpose(&self) -> Hash {
        Hash::from_rows(&transpose_rows(self.rows()))
    }
}

/// A 16x16 bit matrix (16 rows, bit 15 the leftmost column) turned over its main diagonal: at each of the four rounds
/// the top-right and bottom-left blocks of every `2j`-square swap, for `j` = 8, 4, 2, 1.
#[expect(
    clippy::indexing_slicing,
    reason = "k + j < 16 at every step: k runs over the rows whose bit j is clear, and j <= 8"
)]
pub fn transpose_rows(mut a: [u16; 16]) -> [u16; 16] {
    let (mut j, mut m): (usize, u16) = (8, 0x00FF);
    while j != 0 {
        let mut k = 0;
        while k < 16 {
            let t = (a[k] ^ (a[k + j] >> j)) & m;
            a[k] ^= t;
            a[k + j] ^= t << j;
            k = (k + j + 1) & !j;
        }
        j >>= 1;
        m ^= m << j;
    }
    a
}

/// A picture's PDQ: its hash in each kept orientation ([`ORIENTATIONS`]; the first is the image as is), and the quality.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pdq {
    pub orientations: [Hash; ORIENTATIONS],
    /// 0–100, Meta's gradient-based quality metric.
    pub quality: u8,
}

impl Pdq {
    /// The hash of the image as is (orientation 0).
    pub fn hash(&self) -> Hash {
        let [hash, ..] = self.orientations;
        hash
    }
}

/// The bytes the kept orientations are stored as (`analysis.pdq_dihedral`): each hash's 32 bytes, in orientation order.
pub const BLOB_BYTES: usize = ORIENTATIONS * HASH_BYTES;

/// The kept orientations as they are stored ([`BLOB_BYTES`]).
pub fn to_blob(orientations: &[Hash; ORIENTATIONS]) -> [u8; BLOB_BYTES] {
    let mut out = [0u8; BLOB_BYTES];
    for (bytes, hash) in out.as_chunks_mut::<HASH_BYTES>().0.iter_mut().zip(orientations) {
        *bytes = hash.0;
    }
    out
}

/// The kept orientations from their stored bytes; `None` unless the blob is [`BLOB_BYTES`] long.
pub fn from_blob(blob: &[u8]) -> Option<[Hash; ORIENTATIONS]> {
    if blob.len() != BLOB_BYTES {
        return None;
    }
    let hashes = <&[[u8; HASH_BYTES]; ORIENTATIONS]>::try_from(blob.as_chunks::<HASH_BYTES>().0).ok()?;
    Some(hashes.map(Hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The definition, bit by bit: bit `k` (byte `k / 8`, most significant first) is row `k / 16`, column `k % 16`.
    fn transpose_bits(h: &Hash) -> Hash {
        let bit = |k: usize| (h.0[k / 8] >> (7 - k % 8)) & 1;
        let mut out = [0u8; 32];
        for k in 0..256 {
            out[k / 8] |= bit((k % 16) * 16 + k / 16) << (7 - k % 8);
        }
        Hash(out)
    }

    #[test]
    fn the_word_transpose_is_the_bit_transpose_and_an_involution() {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..500 {
            let mut b = [0u8; 32];
            for byte in &mut b {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *byte = x.to_le_bytes()[0];
            }
            let h = Hash(b);
            assert_eq!(h.transpose(), transpose_bits(&h));
            assert_eq!(h.transpose().transpose(), h);
            assert_eq!(Hash::from_rows(&h.rows()), h);
            let four = [h, h.transpose(), Hash([7; 32]), Hash([0; 32])];
            assert_eq!(from_blob(&to_blob(&four)), Some(four));
        }
        assert_eq!(from_blob(&[0; BLOB_BYTES + 1]), None);
        assert_eq!(from_blob(&[0; 8 * HASH_BYTES]), None);
        // One bit: row 2, column 5 goes to row 5, column 2.
        let mut one = [0u8; 32];
        one[4] = 0b0000_0100;
        let mut moved = [0u8; 32];
        moved[10] = 0b0010_0000;
        assert_eq!(Hash(one).transpose(), Hash(moved));
    }
}
