//! The thumbnail cache's size limit as a value.

/// A cache's size limit, in bytes: above zero and under `i64::MAX`, so no cap evicts every thumbnail or saturates. The
/// front ends read the number (the app's `PG_THUMB_CAP_GIB` and its Library panel) and word their own errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbCap(i64);

/// The largest cap, in bytes: under `i64::MAX` (about 9.22e18).
const MAX_CAP_BYTES: f64 = 9.0e18;

impl ThumbCap {
    /// `gib` GiB (2^30 bytes) as a cap, the fraction of a byte dropped; `None` for zero, negative, NaN, infinite and
    /// absurd sizes.
    pub fn from_gib(gib: f64) -> Option<ThumbCap> {
        let bytes = gib * 1_073_741_824.0;
        (1.0..MAX_CAP_BYTES).contains(&bytes).then(|| ThumbCap(crate::num::f64_to_i64(bytes)))
    }

    /// The cap a number of GiB written as text names: the `--cap-gib` flag and the `PG_THUMB_CAP_GIB` variable both read
    /// it this way (see [`ThumbCap::from_gib`]). Surrounding spaces are ignored.
    pub fn parse_gib(text: &str) -> Option<ThumbCap> {
        text.trim().parse::<f64>().ok().and_then(ThumbCap::from_gib)
    }

    /// `bytes` as a cap; `None` unless it is above zero.
    pub const fn from_bytes(bytes: i64) -> Option<ThumbCap> {
        if bytes > 0 { Some(ThumbCap(bytes)) } else { None }
    }

    /// The cap in bytes, as the cache stores it.
    pub const fn bytes(self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::ThumbCap;

    #[test]
    fn a_cap_is_refused_when_it_would_be_no_cache() {
        assert_eq!(ThumbCap::from_gib(2.0).map(ThumbCap::bytes), Some(2 * 1_073_741_824));
        assert_eq!(ThumbCap::from_gib(0.5).map(ThumbCap::bytes), Some(536_870_912));
        for gib in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-12, 1e12] {
            assert_eq!(ThumbCap::from_gib(gib), None, "{gib} is refused");
        }
        assert_eq!(ThumbCap::from_bytes(-1), None);
        assert_eq!(ThumbCap::from_bytes(0), None);
    }

    // The flag and the variable read a cap the same way: a positive number of GiB, spaces ignored, anything else is no
    // cap (so the caller keeps the cache's own).
    #[test]
    fn a_cap_is_a_positive_number_of_gib_as_text() {
        assert_eq!(ThumbCap::parse_gib(" 2 "), ThumbCap::from_gib(2.0));
        assert_eq!(ThumbCap::parse_gib("1.5"), ThumbCap::from_gib(1.5));
        for other in ["", "0", "-1", "x", "NaN", "1e300"] {
            assert_eq!(ThumbCap::parse_gib(other), None, "{other:?}");
        }
    }
}
