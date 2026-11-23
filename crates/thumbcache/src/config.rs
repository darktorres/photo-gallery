//! The cache's size limit and its constants: the cap a new cache gets (the cap as a value is `pg_model::cache::ThumbCap`),
//! the cache's path, the timing constants of the writer and of the daily check, and the cache's layout version. A leaf:
//! the schema and the writer read these.

use std::path::Path;
use std::time::Duration;

/// The cache's layout version (`PRAGMA user_version`), from the registry; `make_layout` sets it when it makes a file. Before
/// the first release, layout changes edit `SCHEMA` in place and the version stays put. A cache of another version, or whose
/// schema differs from this build's, is started over
/// ([`StartOverReason::OtherBuild`](pg_model::notice::StartOverReason::OtherBuild)), never migrated: it holds derived data only.
pub const CACHE_VERSION: i64 = pg_model::versions::CACHE_VERSION;

/// The largest cap a new cache gets when none is set (`ThumbCache::open`), and the cap where the volume's size can't be
/// read: 50 GiB.
pub const MAX_DEFAULT_CAP_BYTES: i64 = 50 * GIB;

/// The smallest cap a new cache gets by default: 1 GiB.
const MIN_DEFAULT_CAP_BYTES: i64 = GIB;

/// One gibibyte (2^30 bytes): the unit of a cap the front ends name.
const GIB: i64 = 1 << 30;

/// The cap of a new cache in `dir` when none is set: a twentieth (5%) of the volume `dir` is on, between 1 GiB
/// (`MIN_DEFAULT_CAP_BYTES`) and 50 GiB (`MAX_DEFAULT_CAP_BYTES`). A cache on a small disk can't fill it; one on a large
/// disk gets the 50 GiB it always had.
pub fn default_cap_bytes(dir: &Path) -> i64 {
    pg_platform::volume::space(dir)
        .and_then(|space| i64::try_from(space.total / 20).ok())
        .map_or(MAX_DEFAULT_CAP_BYTES, |share| share.clamp(MIN_DEFAULT_CAP_BYTES, MAX_DEFAULT_CAP_BYTES))
}

/// A hit not touched for this long is touched again: least recently used to the hour, which is what eviction
/// needs, without a write for every thumbnail shown.
pub(crate) const TOUCH_AGE: Duration = Duration::from_secs(60 * 60);

/// Touches are written together, at the latest this long after the first one waiting (or after the last failed
/// try).
pub(crate) const TOUCH_FLUSH: Duration = Duration::from_secs(60);

/// How often `open` checks the totals against the entries, which is a scan of them: at most once in this interval.
pub(crate) const HEAL_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
