//! The cache's size limit and its constants: the cap a new cache gets and the cap as a value ([`Cap`]), the cache's path,
//! the timing constants of the writer and of the daily check, and the cache's layout version. A leaf: the schema
//! and the writer read these.

use std::path::Path;
use std::time::Duration;

/// The cache's layout version (`PRAGMA user_version`). Before the first release, layout changes edit `SCHEMA` in place
/// and it stays 1. A cache of another version, or whose schema differs from this build's, is started over
/// ([`StartOverReason::OtherBuild`](pg_model::notice::StartOverReason::OtherBuild)), never migrated: it holds derived data only. A
/// macro, so `SCHEMA` sets it as a constant.
macro_rules! cache_version {
    () => {
        1
    };
}
pub(crate) use cache_version;

pub const CACHE_VERSION: i64 = cache_version!();

/// The largest cap a new cache gets when none is set (`ThumbCache::open`), and the cap where the volume's size can't be
/// read.
pub const MAX_DEFAULT_CAP_BYTES: i64 = 50_000_000_000;

/// The smallest cap a new cache gets by default.
const MIN_DEFAULT_CAP_BYTES: i64 = 1_000_000_000;

/// The cap of a new cache in `dir` when none is set: a twentieth (5%) of the volume `dir` is on, between 1 GB
/// (`MIN_DEFAULT_CAP_BYTES`) and 50 GB (`MAX_DEFAULT_CAP_BYTES`). A cache on a small disk can't fill it; one on a large
/// disk gets the 50 GB it always had.
pub fn default_cap_bytes(dir: &Path) -> i64 {
    pg_platform::volume::total_bytes(dir)
        .and_then(|total| i64::try_from(total / 20).ok())
        .map_or(MAX_DEFAULT_CAP_BYTES, |share| share.clamp(MIN_DEFAULT_CAP_BYTES, MAX_DEFAULT_CAP_BYTES))
}

/// A cache's size limit, in bytes: above zero and under `i64::MAX`, so no cap evicts every thumbnail or saturates. The
/// front ends read the number (the command line's `--cap-gb`, the app's `PG_THUMB_CAP_GB`) and word their own errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cap(i64);

/// The largest cap, in bytes: under `i64::MAX` (about 9.22e18).
const MAX_CAP_BYTES: f64 = 9.0e18;

impl Cap {
    /// `gb` GB as a cap, the fraction of a byte dropped; `None` for zero, negative, NaN, infinite and absurd sizes.
    pub fn from_gb(gb: f64) -> Option<Cap> {
        let bytes = gb * 1e9;
        (1.0..MAX_CAP_BYTES).contains(&bytes).then(|| Cap(pg_model::num::f64_to_i64(bytes)))
    }

    /// `bytes` as a cap; `None` unless it is above zero.
    pub const fn from_bytes(bytes: i64) -> Option<Cap> {
        if bytes > 0 { Some(Cap(bytes)) } else { None }
    }

    /// The cap in bytes, as the cache stores it.
    pub const fn bytes(self) -> i64 {
        self.0
    }
}

/// A hit not touched for this long is touched again: least recently used to the hour, which is what eviction
/// needs, without a write for every thumbnail shown.
pub(crate) const TOUCH_AGE_S: i64 = 3600;

/// Touches are written together, at the latest this long after the first one waiting (or after the last failed
/// try).
pub(crate) const TOUCH_FLUSH: Duration = Duration::from_secs(60);

/// How often `open` checks the totals against the entries, which is a scan of them: at most once in this interval.
pub(crate) const HEAL_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
