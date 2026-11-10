//! The cache's size limit and its constants: the cap a new cache gets and how a cap is parsed, the cache's path,
//! the timing constants of the writer and of the daily check, and the cache's layout version. A leaf: the schema
//! and the writer read these.

use std::path::{Path, PathBuf};
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

/// Parse a cache size limit in GB, as the command line's `--cap-gb` and the app's `PG_THUMB_CAP_GB` give it,
/// into bytes. Zero, negative, NaN, infinite and absurd sizes are refused: `as` would turn them into a cap of 0
/// (which evicts every thumbnail) or a saturated i64.
pub fn parse_cap_gb(text: &str) -> Result<i64, String> {
    let gb: f64 = text.trim().parse().map_err(|_| format!("{text} isn't a number of GB"))?;
    let bytes = gb * 1e9;
    if !(1.0..MAX_CAP_BYTES).contains(&bytes) {
        return Err(format!("{text} GB isn't a size a cache can have (a number of GB above zero)"));
    }
    Ok(whole_bytes(bytes))
}

/// The largest cap `parse_cap_gb` takes, in bytes: under `i64::MAX` (about 9.22e18).
const MAX_CAP_BYTES: f64 = 9.0e18;

/// `bytes` as a whole number of them (the fraction dropped), held to 0..=`MAX_CAP_BYTES` (NaN reads as 0).
fn whole_bytes(bytes: f64) -> i64 {
    pg_model::num::f64_to_i64(bytes.clamp(0.0, MAX_CAP_BYTES))
}

/// The cache file in the catalog folder.
pub fn cache_path(dir: &Path) -> PathBuf {
    dir.join("thumbs.sqlite")
}

/// A hit not touched for this long is touched again: least recently used to the hour, which is what eviction
/// needs, without a write for every thumbnail shown.
pub(crate) const TOUCH_AGE_S: i64 = 3600;

/// Touches are written together, at the latest this long after the first one waiting (or after the last failed
/// try).
pub(crate) const TOUCH_FLUSH: Duration = Duration::from_secs(60);

/// How often `open` checks the totals against the entries, which is a scan of them: at most once in this interval.
pub(crate) const HEAL_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
