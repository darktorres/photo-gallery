//! The inbox's rows (docs/NOTIFICATIONS.md): what the app told the user, kept until they dismiss it. A row is a
//! [`Notice`], stored as JSON in this build's form and read back typed; one of a kind this build doesn't know reads as
//! [`Notice::Unknown`]. Uses `core` (the connection and the library's folders) and `schema` (the write transaction).
//!
//! How a notice is kept is its [`Lifecycle`]: an event is a row each time it is posted; a state is one row per key, which
//! the next post of the key replaces and counts and which its producer clears ([`Catalog::clear_notices`], [`Catalog::replace_notices`]); a transient one is
//! never a row. A notice about a library folder is tied to the folder of the library that holds it, and goes with that
//! folder; one about a folder no longer in the library is not kept. A pass's failure ledger is an entry of its own, read
//! from the ledger as the inbox is listed (`crate::ledger`), never a row here.

use rusqlite::{Connection, params};

use pg_model::notice::{Key, Level, Lifecycle, Notice, Part};
use pg_model::rules::{Path as RulePath, PathRules};

use crate::core::Catalog;
use crate::error::Result;
use crate::transaction::write_transaction;

mod id;
pub use id::EntryId;

/// The most rows the inbox keeps that aren't dismissed ([`Catalog::prune_inbox`]): past it, the oldest go, read ones first.
pub const KEPT_ROWS: i64 = 1000;

/// How long a dismissed row is kept (seconds), so a mark sent for it still finds it.
pub const DISMISSED_FOR: i64 = 30 * 24 * 60 * 60;

/// A row as the inbox lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: EntryId,
    pub notice: Notice,
    /// How many times it was posted (a state's repeats).
    pub count: u64,
    /// When it was first posted, and last (Unix seconds).
    pub created: i64,
    pub updated: i64,
    pub read: bool,
}

/// What tells one listing of the inbox from the next ([`Catalog::inbox_marks`]): a row that is new, posted again, or newly read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub id: EntryId,
    pub count: u64,
    pub level: Level,
    pub read: bool,
}

impl Catalog {
    /// Keep `notice`, posted at `now`, as its lifecycle says. Whether it was kept: a transient notice and a ledger's are not
    /// (the ledger is its own), nor is one about a folder no longer in the library.
    ///
    /// A state posted again is counted and holds the latest; if it says something new (its data changed) it is listed as
    /// unread again, even if it was dismissed, and if not it keeps its marks: a problem that holds is not news each time.
    pub fn post_notice(&self, notice: &Notice, now: i64) -> Result<bool> {
        if matches!(notice.lifecycle(), Lifecycle::Transient | Lifecycle::Ledger(_)) {
            return Ok(false);
        }
        let tx = write_transaction(&self.conn)?;
        let kept = upsert(self, &tx, notice, now)?.is_some();
        tx.commit()?;
        Ok(kept)
    }

    /// The kept notices of `scope`'s kind and folder (and its part, if it has one) are now `notices`, posted at `now`: each
    /// is posted ([`Catalog::post_notice`]), and the others of the scope are removed, their problem gone. A notice that is not a kept one
    /// of the scope's kind is not posted. With no notices, it is a clear, which writes nothing when there is nothing to
    /// remove.
    pub fn replace_notices(&self, scope: &Key<'_>, notices: &[Notice], now: i64) -> Result<()> {
        let rules = self.rules();
        let root = match scope.root {
            Some(path) => match covering_root(self, &self.conn, path)? {
                Some(id) => Some(id),
                None => return Ok(()),
            },
            None => None,
        };
        let part = scope.part.as_ref().map(|p| part_text(rules, p));
        let stale = |conn: &Connection| -> Result<Vec<String>> {
            let mut stmt = conn.prepare_cached(
                "SELECT key FROM notices WHERE key IS NOT NULL AND kind = ?1 AND root IS ?2 AND (?3 IS NULL OR part = ?3)",
            )?;
            let keys = stmt.query_map(params![scope.code, root, part], |r| r.get(0))?;
            Ok(keys.collect::<rusqlite::Result<_>>()?)
        };
        if notices.is_empty() && stale(&self.conn)?.is_empty() {
            return Ok(());
        }
        let tx = write_transaction(&self.conn)?;
        let mut posted = Vec::new();
        for notice in notices {
            let Lifecycle::State(key) = notice.lifecycle() else { continue };
            if key.code != scope.code {
                continue;
            }
            if let Some(Some(key)) = upsert(self, &tx, notice, now)? {
                posted.push(key);
            }
        }
        for key in stale(&tx)? {
            if !posted.contains(&key) {
                tx.execute("DELETE FROM notices WHERE key = ?1", [key])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove the kept notices `key` names, their problem gone: the one of its part, or with no part every one of its kind
    /// and folder. Nothing is written when there are none.
    pub fn clear_notices(&self, key: &Key<'_>) -> Result<()> {
        self.replace_notices(key, &[], 0)
    }

    /// The entries listed, newest first, at most `limit`: the rows not dismissed, and the ledgers' entries.
    pub fn inbox_entries(&self, limit: usize) -> Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, uid, kind, data, count, created, updated, read IS NOT NULL FROM notices
             WHERE dismissed IS NULL ORDER BY updated DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
            let kind: String = r.get(2)?;
            let data: String = r.get(3)?;
            Ok(Entry {
                id: EntryId::Row { id: r.get(0)?, uid: r.get(1)? },
                notice: decode(&kind, &data),
                count: count(r.get(4)?),
                created: r.get(5)?,
                updated: r.get(6)?,
                read: r.get(7)?,
            })
        })?;
        let mut out: Vec<Entry> = rows.collect::<rusqlite::Result<_>>()?;
        out.extend(self.ledger_entries()?.into_iter().map(|s| Entry {
            id: EntryId::Ledger { pass: s.pass, seen: s.newest },
            notice: s.notice(),
            count: s.total,
            created: s.created,
            updated: s.updated,
            read: s.read,
        }));
        // Stable: rows of one time keep their order (newest id first).
        out.sort_by_key(|e| std::cmp::Reverse(e.updated));
        out.truncate(limit);
        Ok(out)
    }

    /// What the entries listed are now, newest first: to tell what changed since the last look without reading the
    /// notices.
    pub fn inbox_marks(&self) -> Result<Vec<Mark>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, uid, count, level, read IS NOT NULL, updated FROM notices
             WHERE dismissed IS NULL ORDER BY updated DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let mark = Mark {
                id: EntryId::Row { id: r.get(0)?, uid: r.get(1)? },
                count: count(r.get(2)?),
                level: level_of(r.get(3)?),
                read: r.get(4)?,
            };
            Ok((mark, r.get::<_, i64>(5)?))
        })?;
        let mut out: Vec<(Mark, i64)> = rows.collect::<rusqlite::Result<_>>()?;
        out.extend(self.ledger_entries()?.into_iter().map(|s| {
            let mark = Mark {
                id: EntryId::Ledger { pass: s.pass, seen: s.newest },
                count: s.total,
                level: s.notice().level(),
                read: s.read,
            };
            (mark, s.updated)
        }));
        out.sort_by_key(|(_, updated)| std::cmp::Reverse(*updated));
        Ok(out.into_iter().map(|(mark, _)| mark).collect())
    }

    /// Mark the entries `ids` read (a row since gone, or posted again under the same id, is left; a ledger is read up to
    /// what was listed).
    pub fn mark_read(&self, ids: &[EntryId], now: i64) -> Result<()> {
        let tx = write_transaction(&self.conn)?;
        {
            let mut stmt =
                tx.prepare_cached("UPDATE notices SET read = ?3 WHERE id = ?1 AND uid = ?2 AND read IS NULL")?;
            for id in ids {
                match *id {
                    EntryId::Row { id, uid } => {
                        stmt.execute(params![id, uid, now])?;
                    }
                    EntryId::Ledger { pass, seen } => self.mark_ledger_read(pass, seen, now)?,
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Mark every row read, and the ledgers `ledgers` (as listed).
    pub fn mark_all_read(&self, ledgers: &[EntryId], now: i64) -> Result<()> {
        let tx = write_transaction(&self.conn)?;
        tx.execute("UPDATE notices SET read = ?1 WHERE read IS NULL", [now])?;
        for id in ledgers {
            if let EntryId::Ledger { pass, seen } = *id {
                self.mark_ledger_read(pass, seen, now)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Dismiss the entry `id`: it is no longer listed (a kept state comes back when it is posted again, a ledger with its
    /// next new failure).
    pub fn dismiss(&self, id: EntryId, now: i64) -> Result<()> {
        match id {
            EntryId::Row { id, uid } => {
                self.conn.execute(
                    "UPDATE notices SET dismissed = ?3, read = coalesce(read, ?3)
                     WHERE id = ?1 AND uid = ?2 AND dismissed IS NULL",
                    params![id, uid, now],
                )?;
            }
            EntryId::Ledger { pass, seen } => self.dismiss_ledger(pass, seen, now)?,
        }
        Ok(())
    }

    /// Dismiss every row, and the ledgers `ledgers` (as listed).
    pub fn dismiss_all(&self, ledgers: &[EntryId], now: i64) -> Result<()> {
        let tx = write_transaction(&self.conn)?;
        tx.execute("UPDATE notices SET dismissed = ?1, read = coalesce(read, ?1) WHERE dismissed IS NULL", [now])?;
        for id in ledgers {
            if let EntryId::Ledger { pass, seen } = *id {
                self.dismiss_ledger(pass, seen, now)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// How many entries are listed and not read.
    pub fn unread(&self) -> Result<u64> {
        Ok(u64::try_from(self.inbox_marks()?.iter().filter(|m| !m.read).count()).unwrap_or(u64::MAX))
    }

    /// Keep the inbox bounded: dismissed rows older than [`DISMISSED_FOR`] go, and past [`KEPT_ROWS`] listed rows the
    /// oldest, read ones first. How many went.
    pub fn prune_inbox(&self, now: i64) -> Result<usize> {
        let tx = write_transaction(&self.conn)?;
        let mut n =
            tx.execute("DELETE FROM notices WHERE dismissed IS NOT NULL AND dismissed < ?1", [now - DISMISSED_FOR])?;
        n += tx.execute(
            "DELETE FROM notices WHERE dismissed IS NULL AND id NOT IN (SELECT id FROM notices WHERE dismissed IS NULL
             ORDER BY read IS NULL DESC, updated DESC, id DESC LIMIT ?1)",
            [KEPT_ROWS],
        )?;
        tx.commit()?;
        Ok(n)
    }
}

/// Write `notice` in `tx`: a row, or a kept state's row replaced. `None` when it is about a folder no longer in the
/// library (nothing is written); else the state's key, or `None` for an event.
fn upsert(catalog: &Catalog, tx: &Connection, notice: &Notice, now: i64) -> Result<Option<Option<String>>> {
    let root = match notice.root() {
        Some(path) => match covering_root(catalog, tx, path)? {
            Some(id) => Some(id),
            None => return Ok(None),
        },
        None => None,
    };
    let lifecycle = notice.lifecycle();
    let state = match &lifecycle {
        Lifecycle::State(key) => Some(key),
        Lifecycle::Event | Lifecycle::Transient | Lifecycle::Ledger(_) => None,
    };
    let part = state.and_then(|k| k.part.as_ref()).map(|p| part_text(catalog.rules(), p));
    let key = state.map(|k| key_text(k.code, root, part.as_deref()));
    let data = serde_json::to_string(notice).map_err(anyhow::Error::from)?;
    // In the update every column is the old row's: a repeat that says the same keeps the row's marks.
    tx.execute(
        "INSERT INTO notices(key, part, root, level, source, kind, data, created, updated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT(key) DO UPDATE SET level = excluded.level, data = excluded.data, count = count + 1,
             updated = excluded.updated,
             read = CASE WHEN data IS excluded.data THEN read END,
             dismissed = CASE WHEN data IS excluded.data THEN dismissed END",
        params![key, part, root, level_code(notice.level()), notice.source().code(), notice.code(), data, now],
    )?;
    Ok(Some(key))
}

/// The id of the library folder `path` is, or is in: `None` when no folder of the library holds it.
fn covering_root(catalog: &Catalog, conn: &Connection, path: &RulePath) -> Result<Option<i64>> {
    let rules = catalog.rules();
    Ok(catalog.roots_on(conn)?.into_iter().find(|r| rules.is_under(path, &r.path)).map(|r| r.id.get()))
}

/// A state's key: its kind's code, its folder's id and its part, apart by a character no code or hex holds.
fn key_text(code: &str, root: Option<i64>, part: Option<&str>) -> String {
    let root = root.map(|r| r.to_string()).unwrap_or_default();
    format!("{code}\u{1f}{root}\u{1f}{}", part.unwrap_or(""))
}

/// A key's part as stored: a code as it is, a folder as the hex of its stored bytes (exact, whatever its names hold).
fn part_text(rules: PathRules, part: &Part<'_>) -> String {
    match part {
        Part::Code(code) => (*code).to_string(),
        Part::Path(path) => rules.join(path).iter().map(|b| format!("{b:02x}")).collect(),
    }
}

/// A stored notice, or [`Notice::Unknown`] when this build can't read it (a kind it doesn't know, or a field it needs
/// that the row lacks).
fn decode(kind: &str, data: &str) -> Notice {
    serde_json::from_str(data).unwrap_or_else(|_| Notice::Unknown { code: kind.to_string() })
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
    use crate::ledger;
    use crate::test_support::{p, versions};
    use pg_model::notice::{EditOp, FilePass, ScanNote};
    use pg_model::refusal::RefusedReason;

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

    // A state's posts are one row, counted, holding the latest; events are a row each; a transient notice is none.
    fn a_state_is_one_row_and_an_event_a_row_each(rules: PathRules) {
        let c = catalog(rules);
        assert!(c.post_notice(&failed("a"), 10).unwrap());
        assert!(c.post_notice(&failed("b"), 20).unwrap());
        assert!(c.post_notice(&Notice::FolderNotAdded { error: "x".into() }, 30).unwrap());
        assert!(c.post_notice(&Notice::FolderNotAdded { error: "x".into() }, 40).unwrap());
        assert!(!c.post_notice(&Notice::FileNoLongerInLibrary, 50).unwrap());
        assert!(!c.post_notice(&Notice::ThumbsFailed { counts: None }, 50).unwrap());
        let rows = c.inbox_entries(100).unwrap();
        assert_eq!(rows.len(), 3);
        let state = rows.iter().find(|e| e.notice.code() == "edit-failed").unwrap();
        assert_eq!((state.count, state.created, state.updated), (2, 10, 20));
        assert_eq!(state.notice, failed("b"));
        assert_eq!(c.unread().unwrap(), 3);
    }

    under_both_rules!(
        a_state_is_one_row_and_an_event_a_row_each,
        a_state_is_one_row_and_an_event_a_row_each_windows,
        a_state_is_one_row_and_an_event_a_row_each_posix
    );

    // A read or dismissed state posted again keeps its marks when it says the same, and is new again when it says
    // something new; a mark for a row since gone (its id reused) does nothing.
    fn a_repeat_is_new_again_and_a_stale_mark_does_nothing(rules: PathRules) {
        let c = catalog(rules);
        c.post_notice(&failed("a"), 10).unwrap();
        let first = c.inbox_entries(10).unwrap()[0].id;
        c.dismiss(first, 11).unwrap();
        assert!(c.inbox_entries(10).unwrap().is_empty());
        c.post_notice(&failed("a"), 12).unwrap();
        assert!(c.inbox_entries(10).unwrap().is_empty());
        c.post_notice(&failed("b"), 12).unwrap();
        let again = &c.inbox_entries(10).unwrap()[0];
        assert_eq!((again.id, again.read, again.count), (first, false, 3));
        c.mark_read(&[first], 12).unwrap();
        c.post_notice(&failed("b"), 13).unwrap();
        assert_eq!(c.unread().unwrap(), 0);
        // The row goes and its id is taken by another: the old pair names nothing.
        c.clear_notices(&Key::all("edit-failed", None)).unwrap();
        c.post_notice(&Notice::FolderNotAdded { error: "x".into() }, 13).unwrap();
        let other = c.inbox_entries(10).unwrap()[0].id;
        let (EntryId::Row { id: a, uid: was }, EntryId::Row { id: b, uid: now }) = (first, other) else {
            panic!("rows")
        };
        assert_eq!(a, b);
        assert_ne!(was, now);
        c.mark_read(&[first], 14).unwrap();
        c.dismiss(first, 14).unwrap();
        assert_eq!(c.unread().unwrap(), 1);
        c.mark_read(&[other], 15).unwrap();
        assert_eq!(c.unread().unwrap(), 0);
    }

    under_both_rules!(
        a_repeat_is_new_again_and_a_stale_mark_does_nothing,
        a_repeat_is_new_again_and_a_stale_mark_does_nothing_windows,
        a_repeat_is_new_again_and_a_stale_mark_does_nothing_posix
    );

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
        assert!(c.post_notice(&note(&root, ScanNote::Rebound), 3).unwrap());
        let mount =
            |m: &RulePath| Notice::MountRefused { root: root.clone(), mount: m.clone(), reason: RefusedReason::Fat };
        assert!(c.post_notice(&mount(&under), 4).unwrap());
        assert!(c.post_notice(&mount(&path(rules, &["p", "r"])), 5).unwrap());
        // The note of the sub-folder and the root's are one key (the folder is the root's).
        assert_eq!(c.inbox_entries(10).unwrap().len(), 4);
        c.clear_notices(&Key { code: "scan-note", root: Some(&root), part: Some(Part::Code("duplicates")) }).unwrap();
        assert_eq!(c.inbox_entries(10).unwrap().len(), 3);
        // A complete walk found one of the two mounts again: the other goes, the one found keeps its row.
        let kept = c.inbox_entries(10).unwrap().iter().find(|e| e.notice == mount(&under)).unwrap().id;
        c.replace_notices(&Key::all("mount-refused", Some(&under)), &[mount(&under)], 6).unwrap();
        let mounts: Vec<EntryId> =
            c.inbox_entries(10).unwrap().iter().filter(|e| e.notice.code() == "mount-refused").map(|e| e.id).collect();
        assert_eq!(mounts, [kept]);
        c.clear_notices(&Key::all("mount-refused", Some(&root))).unwrap();
        assert_eq!(c.inbox_entries(10).unwrap().len(), 1);
        c.post_notice(&Notice::ScanFailed { root: root.clone(), error: "e".into() }, 6).unwrap();
        c.post_notice(&failed("a"), 7).unwrap();
        let id = c.roots().unwrap()[0].id;
        c.remove_root(id).unwrap();
        let left: Vec<String> = c.inbox_entries(10).unwrap().iter().map(|e| e.notice.code().to_string()).collect();
        assert_eq!(left, ["edit-failed"]);
    }

    under_both_rules!(
        a_folders_notices_go_with_the_folder,
        a_folders_notices_go_with_the_folder_windows,
        a_folders_notices_go_with_the_folder_posix
    );

    // A row of a kind this build doesn't know reads as Unknown, by its code; one this build wrote reads back.
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
        let codes: Vec<Notice> = c.inbox_entries(10).unwrap().into_iter().map(|e| e.notice).collect();
        assert_eq!(
            codes,
            [Notice::Unknown { code: "edit-failed".into() }, Notice::Unknown { code: "from-later".into() }]
        );
        // Unknown is never written.
        for notice in Notice::samples().into_iter().filter(|n| !matches!(n, Notice::Unknown { .. })) {
            assert_eq!(decode(notice.code(), &serde_json::to_string(&notice).unwrap()), notice);
        }
    }

    // A pass's failures are one entry, among the rows by its time; it is marked and dismissed as listed.
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
        let listed = c.inbox_entries(10).unwrap();
        assert_eq!(listed.iter().map(|e| e.notice.code()).collect::<Vec<_>>(), ["thumbs-failed", "edit-failed"]);
        assert_eq!(c.unread().unwrap(), 2);
        let ledgers = [listed[0].id];
        c.mark_all_read(&ledgers, 30).unwrap();
        assert_eq!(c.unread().unwrap(), 0);
        assert_eq!(c.inbox_marks().unwrap()[0].id.entry(), EntryId::Ledger { pass: FilePass::Thumbs, seen: 0 });
        c.dismiss_all(&ledgers, 31).unwrap();
        assert!(c.inbox_entries(10).unwrap().is_empty());
    }

    // Past the bound the oldest read rows go first, then the oldest unread; dismissed rows go after their time.
    #[test]
    fn the_inbox_is_bounded() {
        let c = catalog(PathRules::Posix);
        for i in 0..KEPT_ROWS + 2 {
            c.post_notice(&Notice::RescanFailed { error: format!("{i}") }, i).unwrap();
        }
        let oldest = c.inbox_entries(usize::MAX).unwrap().last().unwrap().id;
        let newest = c.inbox_entries(1).unwrap()[0].id;
        c.mark_read(&[newest], 0).unwrap();
        c.dismiss(oldest, 0).unwrap();
        assert_eq!(c.prune_inbox(1).unwrap(), 1);
        let left = c.inbox_entries(usize::MAX).unwrap();
        assert_eq!(i64::try_from(left.len()).unwrap(), KEPT_ROWS);
        assert!(!left.iter().any(|e| e.id == newest));
        assert_eq!(c.prune_inbox(DISMISSED_FOR + 1).unwrap(), 1);
    }
}
