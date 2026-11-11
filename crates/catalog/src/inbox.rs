//! The inbox's rows (docs/NOTIFICATIONS.md): what the app told the user, kept until they dismiss it. A row is a
//! [`Notice`], stored as JSON in this build's form and read back typed; one of a kind this build doesn't know reads as
//! [`Notice::Unknown`]. Uses `core` (the connection), `ledger` (the ledgers' entries, read in the rows' snapshot),
//! `types` (the library folder a notice is about) and `transaction`.
//!
//! How a notice is kept is its [`Lifecycle`]: an event is a row each time it is posted, bounded by the `notices_uid`
//! trigger ([`KEPT_EVENTS`]) and deleted when dismissed; a state is one row per key, which the next post of the key
//! replaces and counts (its revision rising when it says something new) and which what resolves it clears
//! ([`Catalog::clear_notices`], [`Catalog::replace_notices`]); a transient one is never a row. A notice about a library
//! folder is tied to the folder of the library that holds it, and goes with that folder; one about a folder no longer
//! in the library is not kept. A ledger is an entry of its own, read from the ledger as the inbox is listed
//! (`crate::ledger`), never a row here.
//!
//! Every write says whether it changed a row (`changes()`), so a caller refreshes only on a real change.

use rusqlite::{Connection, params};

use pg_model::notice::{Key, Level, Lifecycle, Notice, Part};
use pg_model::rules::PathRules;

use crate::core::Catalog;
use crate::error::Result;
use crate::ledger::{Counts, dismiss_ledger_on, ledger_entries_on, mark_ledger_read_on};
use crate::transaction::write_transaction;
use crate::types::covering_root;

mod id;
pub use id::EntryId;

/// The most Events the inbox keeps (`notices_uid`): past it, each insert deletes the oldest, read ones first. States
/// are bounded by their keys.
pub const KEPT_EVENTS: i64 = kept_events!();

/// How long a dismissed state is kept (seconds): a same-data repost of it stays dismissed meanwhile.
pub const DISMISSED_FOR: i64 = 30 * 24 * 60 * 60;

/// An entry as the inbox lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: EntryId,
    pub notice: Notice,
    /// A row's: how many times it was posted (a state's repeats, in `total`). A ledger's: read only while the inbox is
    /// viewed, `None` otherwise (nothing renders a count then).
    pub counts: Option<Counts>,
    /// When it was first posted, and last (Unix seconds).
    pub created: i64,
    pub updated: i64,
    pub read: bool,
    /// Why a stored row of a kind this build knows couldn't be read (it then reads as [`Notice::Unknown`]): a bad
    /// payload, which the inbox logs. `None` for a row read, and for a retired or unknown kind (expected).
    pub decode_error: Option<String>,
    /// A ledger's acknowledgement mark as listed ([`crate::ledger::Summary::seen_to`]); `None` for a row.
    pub seen_to: Option<i64>,
}

/// What tells one listing of the inbox from the next ([`Catalog::inbox_marks`]): a row that is new, posted again (its
/// count, or its revision in its id), or newly read; a ledger by the newest row in its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub id: EntryId,
    /// A row's posts; 0 for a ledger, whose counts aren't read for its mark.
    pub count: u64,
    pub level: Level,
    pub read: bool,
}

impl Catalog {
    /// Keep `notice`, posted at `now`, as its lifecycle says. Whether a row changed: a transient notice and a ledger's
    /// are not kept (the ledger is its own), nor is one about a folder no longer in the library.
    ///
    /// A state posted again is counted and holds the latest; if it says something new (its data changed) its revision
    /// rises and it is listed as unread again, even if it was dismissed, and if not it keeps its marks: a problem that
    /// holds is not news each time.
    pub fn post_notice(&self, notice: &Notice, now: i64) -> Result<bool> {
        self.post_notice_times(notice, 1, now)
    }

    /// [`Catalog::post_notice`] of a state posted `times` times (the inbox folds a run of the same key's posts into
    /// one): one upsert whose count rises by `times`. An event is kept once, whatever `times` says.
    pub fn post_notice_times(&self, notice: &Notice, times: u64, now: i64) -> Result<bool> {
        if times == 0 || matches!(notice.lifecycle(), Lifecycle::Transient | Lifecycle::Ledger(_)) {
            return Ok(false);
        }
        let tx = write_transaction(&self.conn)?;
        let kept = upsert(self.rules, &tx, notice, times, now)?.is_some();
        tx.commit()?;
        Ok(kept)
    }

    /// The kept notices of `scope`'s kind and folder (and its part, if it has one) are now `notices`, posted at `now`:
    /// each is posted ([`Catalog::post_notice`]), and the others of the scope are removed, their problem gone. A notice
    /// that is not a kept one of the scope's kind is not posted. With no notices, it is a clear, which takes no write
    /// lock when there is nothing to remove (one read on `notices(kind, root)`). Whether a row changed.
    pub fn replace_notices(&self, scope: &Key<'_>, notices: &[Notice], now: i64) -> Result<bool> {
        let root = match scope.root {
            Some(path) => match covering_root(&self.conn, self.rules, path)? {
                Some(root) => Some(root.id.get()),
                None => return Ok(false),
            },
            None => None,
        };
        let part = scope.part.map(part_text);
        let stale = |conn: &Connection| -> Result<Vec<String>> {
            let mut stmt = conn.prepare_cached(
                "SELECT key FROM notices WHERE kind = ?1 AND root IS ?2 AND key IS NOT NULL AND (?3 IS NULL OR part = ?3)",
            )?;
            let keys = stmt.query_map(params![scope.code, root, part], |r| r.get(0))?;
            Ok(keys.collect::<rusqlite::Result<_>>()?)
        };
        if notices.is_empty() && stale(&self.conn)?.is_empty() {
            return Ok(false);
        }
        let tx = write_transaction(&self.conn)?;
        let mut posted = Vec::new();
        let mut changed = false;
        for notice in notices {
            let Lifecycle::State(key, _) = notice.lifecycle() else { continue };
            if key.code != scope.code {
                continue;
            }
            if let Some(key) = upsert(self.rules, &tx, notice, 1, now)? {
                changed = true;
                posted.extend(key);
            }
        }
        for key in stale(&tx)? {
            if !posted.contains(&key) {
                changed |= tx.execute("DELETE FROM notices WHERE key = ?1", [key])? > 0;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Remove the kept notices `key` names, their problem gone: the one of its part, or with no part every one of its
    /// kind and folder. Nothing is written when there are none. Whether a row went.
    pub fn clear_notices(&self, key: &Key<'_>) -> Result<bool> {
        self.replace_notices(key, &[], 0)
    }

    /// The entries listed, newest first, at most `limit`: the rows not dismissed, and the ledgers' entries (with their
    /// counts when `viewing`: the inbox is open), all read in one snapshot.
    pub fn inbox_entries(&self, limit: usize, viewing: bool) -> Result<Vec<Entry>> {
        let snapshot = self.conn.unchecked_transaction()?;
        let mut out: Vec<Entry> = {
            let mut stmt = snapshot.prepare_cached(
                "SELECT id, uid, rev, kind, data, count, created, updated, read IS NOT NULL FROM notices
                 WHERE dismissed IS NULL ORDER BY updated DESC, id DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
                let kind: String = r.get(3)?;
                let data: String = r.get(4)?;
                let Decoded { notice, error } = decode(&kind, &data);
                Ok(Entry {
                    id: EntryId::Row { id: r.get(0)?, uid: r.get(1)?, rev: r.get(2)? },
                    notice,
                    decode_error: error,
                    seen_to: None,
                    counts: Some(Counts { total: count(r.get(5)?), ..Counts::default() }),
                    created: r.get(6)?,
                    updated: r.get(7)?,
                    read: r.get(8)?,
                })
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        out.extend(ledger_entries_on(&snapshot, viewing)?.into_iter().map(|s| Entry {
            id: EntryId::Ledger { kind: s.kind, seen: s.newest },
            notice: s.notice(),
            counts: s.counts,
            created: s.created,
            updated: s.updated,
            read: s.read,
            decode_error: None,
            seen_to: Some(s.seen_to),
        }));
        snapshot.commit()?;
        // Stable: rows of one time keep their order (newest id first).
        out.sort_by_key(|e| std::cmp::Reverse(e.updated));
        out.truncate(limit);
        Ok(out)
    }

    /// What the entries listed are now, newest first: to tell what changed since the last look without reading the
    /// notices (nor any ledger's counts).
    pub fn inbox_marks(&self) -> Result<Vec<Mark>> {
        let snapshot = self.conn.unchecked_transaction()?;
        let mut out: Vec<(Mark, i64)> = {
            let mut stmt = snapshot.prepare_cached(
                "SELECT id, uid, rev, count, level, read IS NOT NULL, updated FROM notices
                 WHERE dismissed IS NULL ORDER BY updated DESC, id DESC",
            )?;
            let rows = stmt.query_map([], |r| {
                let mark = Mark {
                    id: EntryId::Row { id: r.get(0)?, uid: r.get(1)?, rev: r.get(2)? },
                    count: count(r.get(3)?),
                    level: level_of(r.get(4)?),
                    read: r.get(5)?,
                };
                Ok((mark, r.get::<_, i64>(6)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        out.extend(ledger_entries_on(&snapshot, false)?.into_iter().map(|s| {
            let id = EntryId::Ledger { kind: s.kind, seen: s.newest };
            let mark = Mark { id, count: 0, level: s.notice().level(), read: s.read };
            (mark, s.updated)
        }));
        snapshot.commit()?;
        out.sort_by_key(|(_, updated)| std::cmp::Reverse(*updated));
        Ok(out.into_iter().map(|(mark, _)| mark).collect())
    }

    /// Mark the entries `ids` read, as they were listed: a row at its revision (one since gone, or posted again with new
    /// data, is left), a ledger up to what was listed. Whether anything changed.
    pub fn mark_read(&self, ids: &[EntryId], now: i64) -> Result<bool> {
        let tx = write_transaction(&self.conn)?;
        let mut changed = false;
        {
            let mut stmt = tx.prepare_cached(
                "UPDATE notices SET read = ?4 WHERE id = ?1 AND uid = ?2 AND rev = ?3 AND read IS NULL",
            )?;
            for id in ids {
                changed |= match *id {
                    EntryId::Row { id, uid, rev } => stmt.execute(params![id, uid, rev, now])? > 0,
                    EntryId::Ledger { kind, seen } => mark_ledger_read_on(&tx, kind, seen, now)?,
                };
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// "Mark all read": the entries displayed, `ids` ([`Catalog::mark_read`]), and never one the window wasn't given.
    pub fn mark_all_read(&self, ids: &[EntryId], now: i64) -> Result<bool> {
        self.mark_read(ids, now)
    }

    /// Dismiss the entry `id`, as listed: it is no longer listed. An event's row goes (nothing can bring it back); a
    /// state's row is kept, dismissed, so a repost that says the same stays dismissed, and it comes back when it says
    /// something new; a ledger comes back with its next row. Whether anything changed.
    pub fn dismiss(&self, id: EntryId, now: i64) -> Result<bool> {
        self.dismiss_all(&[id], now)
    }

    /// "Clear all": dismiss the entries displayed, `ids` ([`Catalog::dismiss`]), and never one the window wasn't given.
    pub fn dismiss_all(&self, ids: &[EntryId], now: i64) -> Result<bool> {
        let tx = write_transaction(&self.conn)?;
        let mut changed = false;
        {
            let mut event =
                tx.prepare_cached("DELETE FROM notices WHERE id = ?1 AND uid = ?2 AND rev = ?3 AND key IS NULL")?;
            let mut state = tx.prepare_cached(
                "UPDATE notices SET dismissed = ?4, read = coalesce(read, ?4)
                 WHERE id = ?1 AND uid = ?2 AND rev = ?3 AND key IS NOT NULL AND dismissed IS NULL",
            )?;
            for id in ids {
                changed |= match *id {
                    EntryId::Row { id, uid, rev } => {
                        event.execute(params![id, uid, rev])? > 0 || state.execute(params![id, uid, rev, now])? > 0
                    }
                    EntryId::Ledger { kind, seen } => dismiss_ledger_on(&tx, kind, seen, now)?,
                };
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// How many entries are listed and not read.
    pub fn unread(&self) -> Result<u64> {
        Ok(u64::try_from(self.inbox_marks()?.iter().filter(|m| !m.read).count()).unwrap_or(u64::MAX))
    }

    /// Dismissed states older than [`DISMISSED_FOR`] go (events are bounded by `notices_uid`, and deleted when
    /// dismissed). How many went.
    pub fn prune_inbox(&self, now: i64) -> Result<usize> {
        let tx = write_transaction(&self.conn)?;
        let n =
            tx.execute("DELETE FROM notices WHERE dismissed IS NOT NULL AND dismissed < ?1", [now - DISMISSED_FOR])?;
        tx.commit()?;
        Ok(n)
    }
}

/// Write `notice` in `tx`: a row, or a kept state's row replaced (its revision rising when its data changed). `None`
/// when it is about a folder no longer in the library (nothing is written); else the state's key, or `None` for an
/// event. A state's count starts at, or rises by, `times`.
fn upsert(rules: PathRules, tx: &Connection, notice: &Notice, times: u64, now: i64) -> Result<Option<Option<String>>> {
    let root = match notice.root() {
        Some(path) => match covering_root(tx, rules, path)? {
            Some(root) => Some(root.id.get()),
            None => return Ok(None),
        },
        None => None,
    };
    let lifecycle = notice.lifecycle();
    let state = match &lifecycle {
        Lifecycle::State(key, _) => Some(key),
        Lifecycle::Event | Lifecycle::Transient | Lifecycle::Ledger(_) => None,
    };
    let part = state.and_then(|k| k.part).map(part_text);
    let key = state.map(|k| key_text(k.code, root, part.as_deref()));
    let data = serde_json::to_string(notice).map_err(anyhow::Error::from)?;
    let times = if key.is_some() { i64::try_from(times).unwrap_or(i64::MAX) } else { 1 };
    // In the update every column is the old row's: a repeat that says the same keeps the row's marks and revision.
    tx.prepare_cached(
        "INSERT INTO notices(key, part, root, level, source, kind, data, created, updated, count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
         ON CONFLICT(key) DO UPDATE SET level = excluded.level, data = excluded.data, count = count + excluded.count,
             updated = excluded.updated,
             rev = CASE WHEN data IS excluded.data THEN rev ELSE rev + 1 END,
             read = CASE WHEN data IS excluded.data THEN read END,
             dismissed = CASE WHEN data IS excluded.data THEN dismissed END",
    )?
    .execute(params![
        key,
        part,
        root,
        level_code(notice.level()),
        notice.source().code(),
        notice.code(),
        data,
        now,
        times
    ])?;
    Ok(Some(key))
}

/// A state's key: its kind's code, its folder's id and its part, apart by a character no code holds.
fn key_text(code: &str, root: Option<i64>, part: Option<&str>) -> String {
    let root = root.map(|r| r.to_string()).unwrap_or_default();
    format!("{code}\u{1f}{root}\u{1f}{}", part.unwrap_or(""))
}

/// A key's part as stored: its code.
fn part_text(part: Part) -> String {
    match part {
        Part::Code(code) => code.to_string(),
    }
}

/// A stored row read: its notice, and why a known kind's row couldn't be read.
struct Decoded {
    notice: Notice,
    error: Option<String>,
}

/// A stored notice, or [`Notice::Unknown`] when this build can't read it (a kind it doesn't know or retired, or a field
/// it needs that the row lacks). Only the last is an error worth reporting: the others are expected.
fn decode(kind: &str, data: &str) -> Decoded {
    match serde_json::from_str(data) {
        Ok(notice) => Decoded { notice, error: None },
        Err(e) => Decoded {
            notice: Notice::Unknown { code: kind.to_string() },
            error: Notice::CODES.contains(&kind).then(|| e.to_string()),
        },
    }
}

fn level_code(level: Level) -> i64 {
    match level {
        Level::Info => 0,
        Level::Warning => 1,
        Level::Error => 2,
    }
}

fn level_of(code: i64) -> Level {
    match code {
        0 => Level::Info,
        1 => Level::Warning,
        _ => Level::Error,
    }
}

/// A stored count (never negative).
fn count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::under_both_rules;
    use crate::fixture::raw;
    use crate::ledger;
    use crate::test_support::{p, versions};
    use pg_model::notice::{EditOp, FilePass, LedgerKind, ScanNote};
    use pg_model::rules::Path as RulePath;

    fn catalog(rules: PathRules) -> Catalog {
        Catalog::prepare(std::path::Path::new(":memory:"), versions(), rules).unwrap()
    }

    /// `D:\<parts>` (spelled for `rules`).
    fn path(rules: PathRules, parts: &[&str]) -> RulePath {
        p(rules, &format!(r"D:\{}", parts.join(r"\")))
    }

    fn failed(error: &str) -> Notice {
        Notice::EditFailed { op: EditOp::Rate, error: error.into() }
    }

    fn event(i: i64) -> Notice {
        Notice::RescanFailed { error: format!("{i}") }
    }

    fn total(e: &Entry) -> Option<u64> {
        e.counts.map(|c| c.total)
    }

    // A state's posts are one row, counted, holding the latest; events are a row each; a transient notice is none.
    fn a_state_is_one_row_and_an_event_a_row_each(rules: PathRules) {
        let c = catalog(rules);
        assert!(c.post_notice(&failed("a"), 10).unwrap());
        assert!(c.post_notice(&failed("b"), 20).unwrap());
        assert!(c.post_notice(&Notice::FolderNotAdded { error: "x".into() }, 30).unwrap());
        assert!(c.post_notice(&Notice::FolderNotAdded { error: "x".into() }, 40).unwrap());
        assert!(!c.post_notice(&Notice::FileNoLongerInLibrary, 50).unwrap());
        assert!(!c.post_notice(&Notice::ThumbsFailed { counts: None }, 50).unwrap());
        let rows = c.inbox_entries(100, false).unwrap();
        assert_eq!(rows.len(), 3);
        let state = rows.iter().find(|e| e.notice.code() == "edit-failed").unwrap();
        assert_eq!((total(state), state.created, state.updated), (Some(2), 10, 20));
        assert_eq!(state.notice, failed("b"));
        assert_eq!(c.unread().unwrap(), 3);
    }

    under_both_rules!(
        a_state_is_one_row_and_an_event_a_row_each,
        a_state_is_one_row_and_an_event_a_row_each_windows,
        a_state_is_one_row_and_an_event_a_row_each_posix
    );

    // A read or dismissed state posted again keeps its marks and its revision when it says the same, and is new again,
    // at its next revision, when it says something new; a mark for a revision since passed, or for a row since gone
    // (its id reused), does nothing.
    fn a_repeat_is_new_again_and_a_stale_mark_does_nothing(rules: PathRules) {
        let c = catalog(rules);
        c.post_notice(&failed("a"), 10).unwrap();
        let first = c.inbox_entries(10, false).unwrap()[0].id;
        assert!(c.dismiss(first, 11).unwrap());
        assert!(!c.dismiss(first, 11).unwrap(), "dismissed already");
        assert!(c.inbox_entries(10, false).unwrap().is_empty());
        c.post_notice(&failed("a"), 12).unwrap();
        assert!(c.inbox_entries(10, false).unwrap().is_empty(), "the same data stays dismissed");
        c.post_notice(&failed("b"), 12).unwrap();
        let again = c.inbox_entries(10, false).unwrap()[0].clone();
        assert_eq!((again.id.entry(), again.read, total(&again)), (first.entry(), false, Some(3)));
        assert_ne!(again.id, first, "a new revision");
        // A mark sent for the old revision doesn't land on the new one.
        assert!(!c.mark_read(&[first], 12).unwrap());
        assert_eq!(c.unread().unwrap(), 1);
        assert!(c.mark_read(&[again.id], 12).unwrap());
        assert!(!c.mark_read(&[again.id], 12).unwrap(), "read already");
        c.post_notice(&failed("b"), 13).unwrap();
        assert_eq!(c.unread().unwrap(), 0);
        // The row goes and its id is taken by another: the old pair names nothing.
        assert!(c.clear_notices(&Key::all("edit-failed", None)).unwrap());
        assert!(!c.clear_notices(&Key::all("edit-failed", None)).unwrap(), "nothing left to clear");
        c.post_notice(&Notice::FolderNotAdded { error: "x".into() }, 13).unwrap();
        let other = c.inbox_entries(10, false).unwrap()[0].id;
        let (EntryId::Row { id: a, uid: was, .. }, EntryId::Row { id: b, uid: now, .. }) = (again.id, other) else {
            panic!("rows")
        };
        assert_eq!(a, b);
        assert_ne!(was, now);
        assert!(!c.mark_read(&[again.id], 14).unwrap());
        assert!(!c.dismiss(again.id, 14).unwrap());
        assert_eq!(c.unread().unwrap(), 1);
    }

    under_both_rules!(
        a_repeat_is_new_again_and_a_stale_mark_does_nothing,
        a_repeat_is_new_again_and_a_stale_mark_does_nothing_windows,
        a_repeat_is_new_again_and_a_stale_mark_does_nothing_posix
    );

    // A dismissed event's row goes at once; a dismissed state's stays (dismissed) until its time is up.
    fn a_dismissed_event_goes_and_a_dismissed_state_is_kept(rules: PathRules) {
        let c = catalog(rules);
        c.post_notice(&event(1), 10).unwrap();
        c.post_notice(&failed("a"), 11).unwrap();
        let ids: Vec<EntryId> = c.inbox_entries(10, false).unwrap().iter().map(|e| e.id).collect();
        assert!(c.dismiss_all(&ids, 12).unwrap());
        let rows: i64 = c.conn.query_row("SELECT count(*) FROM notices", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1, "the state's row is kept, dismissed");
        assert_eq!(c.prune_inbox(12 + DISMISSED_FOR).unwrap(), 0);
        assert_eq!(c.prune_inbox(13 + DISMISSED_FOR).unwrap(), 1);
    }

    under_both_rules!(
        a_dismissed_event_goes_and_a_dismissed_state_is_kept,
        a_dismissed_event_goes_and_a_dismissed_state_is_kept_windows,
        a_dismissed_event_goes_and_a_dismissed_state_is_kept_posix
    );

    // "Clear all" and "Mark all read" reach only the entries displayed: one posted since is left as it is.
    #[test]
    fn the_all_operations_reach_only_what_was_displayed() {
        let c = catalog(PathRules::Posix);
        c.post_notice(&failed("a"), 10).unwrap();
        c.post_notice(&event(1), 11).unwrap();
        let displayed: Vec<EntryId> = c.inbox_entries(10, false).unwrap().iter().map(|e| e.id).collect();
        c.post_notice(&event(2), 12).unwrap();
        assert!(c.mark_all_read(&displayed, 13).unwrap());
        assert_eq!(c.unread().unwrap(), 1, "the one posted since is unread");
        assert!(c.dismiss_all(&displayed, 14).unwrap());
        let left = c.inbox_entries(10, false).unwrap();
        assert_eq!(left.iter().map(|e| e.notice.clone()).collect::<Vec<_>>(), [event(2)]);
    }

    // A scan's notices are tied to the library folder that holds the scanned one: kept only while it is in the
    // library, cleared per part or all at once, and gone when the folder is removed.
    fn a_folders_notices_go_with_the_folder(rules: PathRules) {
        let c = catalog(rules);
        let root = path(rules, &["p"]);
        // `_` and `%` match any character in a LIKE: the folder is found by its id, not by a pattern.
        let under = path(rules, &["p", "q_%x"]);
        // Not in the library yet: not kept.
        let note = |root: &RulePath, note| Notice::ScanNote { root: root.clone(), note, count: 1 };
        assert!(!c.post_notice(&note(&root, ScanNote::Duplicates), 1).unwrap());
        assert!(c.add_root(&root).unwrap());
        assert!(c.post_notice(&note(&under, ScanNote::Duplicates), 2).unwrap());
        assert!(c.post_notice(&note(&root, ScanNote::Duplicates), 3).unwrap());
        assert!(c.post_notice(&note(&root, ScanNote::SkippedReparse), 3).unwrap());
        // The note of the sub-folder and the root's are one key (the folder is the root's).
        assert_eq!(c.inbox_entries(10, false).unwrap().len(), 2);
        let duplicates = Key { code: "scan-note", root: Some(&root), part: Some(Part::Code("duplicates")) };
        assert!(c.clear_notices(&duplicates).unwrap());
        assert_eq!(c.inbox_entries(10, false).unwrap().len(), 1);
        // A root walk's replace: the one found again keeps its row, the others go.
        assert!(c.post_notice(&note(&root, ScanNote::Ambiguous), 4).unwrap());
        let entries = c.inbox_entries(10, false).unwrap();
        let kept = entries.iter().find(|e| e.notice == note(&root, ScanNote::Ambiguous)).unwrap().id;
        c.replace_notices(&Key::all("scan-note", Some(&root)), &[note(&root, ScanNote::Ambiguous)], 6).unwrap();
        let notes: Vec<EntryId> = c.inbox_entries(10, false).unwrap().iter().map(|e| e.id.entry()).collect();
        assert_eq!(notes, [kept.entry()]);
        c.post_notice(&Notice::ScanFailed { root: root.clone(), error: "e".into() }, 6).unwrap();
        c.post_notice(&failed("a"), 7).unwrap();
        let id = c.roots().unwrap()[0].id;
        c.remove_root(id).unwrap();
        let left: Vec<String> =
            c.inbox_entries(10, false).unwrap().iter().map(|e| e.notice.code().to_string()).collect();
        assert_eq!(left, ["edit-failed"]);
    }

    under_both_rules!(
        a_folders_notices_go_with_the_folder,
        a_folders_notices_go_with_the_folder_windows,
        a_folders_notices_go_with_the_folder_posix
    );

    // A row of a kind this build doesn't know (or retired) reads as Unknown, by its code; one this build wrote reads back.
    #[test]
    fn a_row_this_build_cant_read_is_unknown() {
        let c = catalog(PathRules::Posix);
        c.conn
            .execute(
                "INSERT INTO notices(level, source, kind, data, created, updated)
                 VALUES (1, 'app', 'from-later', '{\"kind\":\"from-later\"}', 1, 1),
                        (1, 'edit', 'edit-failed', '{\"kind\":\"edit-failed\"}', 2, 2)",
                [],
            )
            .unwrap();
        let entries = c.inbox_entries(10, false).unwrap();
        let codes: Vec<&Notice> = entries.iter().map(|e| &e.notice).collect();
        assert_eq!(
            codes,
            [&Notice::Unknown { code: "edit-failed".into() }, &Notice::Unknown { code: "from-later".into() }]
        );
        // A known kind with a bad payload says why; a kind this build doesn't know is expected, and says nothing.
        assert!(entries[0].decode_error.is_some());
        assert_eq!(entries[1].decode_error, None);
        let retired = Notice::RETIRED[0];
        assert!(decode(retired, "{}").error.is_none());
        // Unknown is never written.
        for notice in Notice::samples().into_iter().filter(|n| !matches!(n, Notice::Unknown { .. })) {
            assert_eq!(decode(notice.code(), &serde_json::to_string(&notice).unwrap()).notice, notice);
        }
    }

    // A pass's failures are one entry, among the rows by its time; it is marked and dismissed as listed, and its counts
    // are read only while the inbox is viewed.
    #[test]
    fn a_ledger_is_listed_among_the_rows() {
        let c = catalog(PathRules::Posix);
        crate::fixture::state::add_scan(&c, 1, "/p").unwrap();
        crate::fixture::state::add_folder(&c, 1, "/p").unwrap();
        let file = pg_model::ids::FileId::new(1);
        crate::fixture::insert(
            &c,
            &[crate::fixture::File::new(file, pg_model::ids::FolderId::new(1), "a.jpg", 10, 100)],
        )
        .unwrap();
        c.post_notice(&failed("a"), 10).unwrap();
        let failure = ledger::Failure {
            file_id: file,
            mtime: 100,
            size: 10,
            rotation: pg_model::turns::Turns::NONE,
            error: "x".into(),
            retry: false,
        };
        c.record_failures(FilePass::Thumbs, &[failure], 20).unwrap();
        let listed = c.inbox_entries(10, false).unwrap();
        assert_eq!(listed.iter().map(|e| e.notice.code()).collect::<Vec<_>>(), ["thumbs-failed", "edit-failed"]);
        assert_eq!(listed[0].counts, None, "no counts while the inbox isn't viewed");
        let viewed = c.inbox_entries(10, true).unwrap();
        assert_eq!(viewed[0].counts, Some(Counts { total: 1, new: 1, since_dismissed: 0 }));
        assert_eq!(c.unread().unwrap(), 2);
        let displayed: Vec<EntryId> = listed.iter().map(|e| e.id).collect();
        assert!(c.mark_all_read(&displayed, 30).unwrap());
        assert_eq!(c.unread().unwrap(), 0);
        let kind = LedgerKind::File(FilePass::Thumbs);
        assert_eq!(c.inbox_marks().unwrap()[0].id.entry(), EntryId::Ledger { kind, seen: 0 });
        assert!(c.dismiss_all(&displayed, 31).unwrap());
        assert!(c.inbox_entries(10, false).unwrap().is_empty());
    }

    /// The Events in the inbox, oldest first, by their errors.
    fn events(c: &Catalog) -> Vec<String> {
        raw::rows_of(c, "SELECT data FROM notices WHERE key IS NULL ORDER BY uid", [], |r| r.get::<_, String>(0))
    }

    // Past the bound the oldest Events go, by insertion and read ones first, whatever time they were posted with: the
    // one just posted always stays (with the production trigger's text).
    #[test]
    fn events_past_the_bound_go_oldest_first_and_never_the_one_just_posted() {
        let c = catalog(PathRules::Posix);
        for i in 0..KEPT_EVENTS {
            c.post_notice(&event(i), 100 + i).unwrap();
        }
        // The newest read: it goes before the unread ones.
        let newest = c.inbox_entries(1, false).unwrap()[0].id;
        c.mark_read(&[newest], 0).unwrap();
        // Two more, posted with an old time: they stay, and the read one and the oldest unread go.
        c.post_notice(&event(KEPT_EVENTS), 1).unwrap();
        c.post_notice(&event(KEPT_EVENTS + 1), 1).unwrap();
        let kept = events(&c);
        assert_eq!(i64::try_from(kept.len()).unwrap(), KEPT_EVENTS);
        let has = |i: i64| kept.iter().any(|d| *d == serde_json::to_string(&event(i)).unwrap());
        assert!(has(KEPT_EVENTS) && has(KEPT_EVENTS + 1), "the ones just posted stay");
        assert!(!has(KEPT_EVENTS - 1), "the read one went first");
        assert!(!has(0) && has(1), "then the oldest");
    }

    // States are bounded by their keys, not by the Events' bound: live and dismissed states past it are all kept, an
    // Event posted among them is too and the Events are still bounded, and an upsert that updates prunes nothing.
    #[test]
    fn states_are_never_pruned_and_an_update_prunes_nothing() {
        let c = catalog(PathRules::Posix);
        c.conn
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)
                 INSERT INTO notices(key, level, source, kind, data, created, updated, dismissed)
                 SELECT 's' || i, 1, 'app', 'edit-failed', '{}', 0, 0, CASE WHEN i % 2 = 0 THEN 1 END FROM n",
                [KEPT_EVENTS + 5],
            )
            .unwrap();
        for i in 0..3 {
            c.post_notice(&event(i), 10).unwrap();
        }
        let states: i64 =
            c.conn.query_row("SELECT count(*) FROM notices WHERE key IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(states, KEPT_EVENTS + 5);
        assert_eq!(events(&c).len(), 3);
        // Fill the Events to the bound; a state posted again (an update) deletes none of them.
        for i in 3..KEPT_EVENTS {
            c.post_notice(&event(i), 10).unwrap();
        }
        c.post_notice(&failed("a"), 11).unwrap();
        c.post_notice(&failed("b"), 12).unwrap();
        assert_eq!(i64::try_from(events(&c).len()).unwrap(), KEPT_EVENTS);
        c.post_notice(&event(KEPT_EVENTS), 13).unwrap();
        assert_eq!(i64::try_from(events(&c).len()).unwrap(), KEPT_EVENTS);
    }

    // A clear with nothing stored reads `notices(kind, root)` and takes no write lock.
    #[test]
    fn a_clear_with_nothing_stored_reads_the_kind_index() {
        let c = catalog(PathRules::Posix);
        let plan = raw::query_plan(
            &c,
            "SELECT key FROM notices WHERE kind = ?1 AND root IS ?2 AND key IS NOT NULL AND (?3 IS NULL OR part = ?3)",
            params!["scan-note", Option::<i64>::None, Option::<String>::None],
        );
        assert!(plan.contains("notices_kind_root"), "{plan}");
        let changes = c.conn.total_changes();
        assert!(!c.clear_notices(&Key::all("edit-failed", None)).unwrap());
        assert_eq!(c.conn.total_changes(), changes);
    }
}
