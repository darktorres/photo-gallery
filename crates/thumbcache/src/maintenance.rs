//! The cache file's upkeep, below the schema and the writer: the check of the totals against the entries (`heal`,
//! and `heal_if_due` once a day at open), giving free pages back (`shrink`), and the Unix clock they use.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{Connection, TransactionBehavior, params};

use crate::config::HEAL_INTERVAL;

/// Shrinking (after a pass, at idle) starts when this many pages are free, and frees this many per step, for at
/// most this many steps; freed pages are otherwise reused by later writes.
pub(crate) const SHRINK_ABOVE_PAGES: i64 = if cfg!(test) { 16 } else { 16_384 };

macro_rules! shrink_step_pages {
    () => {
        4096
    };
}

const SHRINK_STEPS: usize = 16;

/// The totals as they were and as the entries have them, when `heal` had to fix them: (bytes, count) each.
pub type Drift = ((i64, i64), (i64, i64));

pub(crate) fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// `heal`, unless it was done within `HEAL_INTERVAL` (`meta.healed`, the Unix second it last was): the open's check.
pub(crate) fn heal_if_due(conn: &mut Connection) -> Result<Option<Drift>> {
    let now = unix_now();
    let last: i64 =
        conn.query_row("SELECT coalesce((SELECT value FROM meta WHERE key = 'healed'), 0)", [], |r| r.get(0))?;
    if now.saturating_sub(last) < i64::try_from(HEAL_INTERVAL.as_secs()).unwrap_or(i64::MAX) {
        return Ok(None);
    }
    let fixed = heal(conn)?;
    conn.execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('healed', ?1)", [now])?;
    Ok(fixed)
}

pub(crate) fn heal(conn: &mut Connection) -> Result<Option<Drift>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let kept: (i64, i64) = tx.query_row("SELECT bytes, count FROM totals", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let actual: (i64, i64) =
        tx.query_row("SELECT coalesce(sum(bytes), 0), count(*) FROM entries", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    if kept == actual {
        return Ok(None);
    }
    tx.execute("UPDATE totals SET bytes = ?1, count = ?2", params![actual.0, actual.1])?;
    tx.commit()?;
    Ok(Some((kept, actual)))
}

pub(crate) fn shrink(conn: &Connection) -> Result<()> {
    for _ in 0..SHRINK_STEPS {
        let free: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
        if free <= SHRINK_ABOVE_PAGES {
            break;
        }
        // It frees one page per step (a row each), so it is stepped to the end: `execute_batch` would free one.
        let mut step = conn.prepare(concat!("PRAGMA incremental_vacuum(", shrink_step_pages!(), ")"))?;
        let mut rows = step.query([])?;
        while rows.next()?.is_some() {}
    }
    Ok(())
}
