//! The catalog file's locks and open holds, the path rules it was made under, and the version marks that
//! `Catalog::prepare` checks and sets. The rebuild is `rebuild`'s, the work lists `work`'s. Uses `types`, `error`,
//! `progress` (the free space check), `transaction` and `ledger` (the failures' relisting); its SQL macros are from `sql`.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction};

use pg_model::media::Ext;
use pg_model::rules::PathRules;

use crate::error::{Error, Result};
use crate::progress::FreeSpace;
use crate::transaction::write_transaction;
use crate::types::{TAKEN_ZONE_QUERY, Versions, key_version_text, rules_name};
use pg_model::name::Name;
use std::collections::BTreeMap;

/// Why the paths of the catalog on `conn` don't read as paths under `rules` (`meta.path_rules`): they were stored under
/// other rules, or before the rules were recorded. `None` when they were stored under these. Opening refuses such a
/// catalog ([`check_rules`]); a rebuild carries none of its rows (`rebuild::rebuild`).
pub(crate) fn rules_mismatch(conn: &Connection, rules: PathRules) -> rusqlite::Result<Option<String>> {
    let found: Option<String> =
        conn.query_row("SELECT value FROM meta WHERE key = 'path_rules'", [], |r| r.get(0)).optional()?;
    Ok(match found.as_deref() {
        Some(name) if name == rules_name(rules) => None,
        Some(name) => Some(format!("made with the {name} path rules")),
        None => Some("made before its path rules were recorded".to_string()),
    })
}

/// Refuse a catalog whose paths were stored under other rules than these ([`rules_mismatch`]): its bytes don't read as
/// paths under these. `Catalog::prepare` has already made sure the schema is this build's,
/// which includes the rules row, so a missing row can only be refused here, as a file this build can't read.
pub(crate) fn check_rules(conn: &Connection, path: &Path, rules: PathRules) -> Result<()> {
    match rules_mismatch(conn, rules)? {
        None => Ok(()),
        Some(detail) => Err(Error::Refused {
            path: path.display().to_string(),
            reason: Box::new(crate::error::Refusal::PathRules { detail, wanted: rules_name(rules) }),
        }),
    }
}

/// The lock `Catalog::prepare` holds on the catalog (`pg_dbfile::Lock`): two processes
/// preparing one catalog do it one after the other, and the second finds the first's catalog current. It is not taken
/// for `:memory:`.
pub(crate) fn take_lock(path: &Path) -> Result<pg_dbfile::Lock> {
    match pg_dbfile::Lock::take(path) {
        Ok(Some(lock)) => Ok(lock),
        Ok(None) => Err(Error::Held { path: path.display().to_string(), what: "another process is preparing it" }),
        // The lock file is made beside the catalog, so a folder that can't be written refuses it by name.
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err(refuse_read_only(path)),
        Err(e) => Err(Error::step(format!("catalog {}: its lock file", path.display()), e)),
    }
}

/// The refusal of a catalog that can't be written (its folder or file is read-only, or permission is denied). Nothing has
/// been changed.
pub(crate) fn refuse_read_only(path: &Path) -> Error {
    Error::Refused { path: path.display().to_string(), reason: Box::new(crate::error::Refusal::ReadOnly) }
}

/// The exclusive hold on the catalog file (`pg_dbfile::Live`), when no other process or connection holds it. It doesn't
/// wait: `None` is the answer when one does.
pub(crate) fn try_exclusive(path: &Path) -> Result<Option<pg_dbfile::Live>> {
    pg_dbfile::Live::exclusive(path).map_err(|e| Error::step(format!("catalog {}: its open marker", path.display()), e))
}

/// The refusal of a change to a catalog that another program or connection has open. Nothing has been moved.
pub(crate) fn in_use(path: &Path) -> Error {
    Error::Refused { path: path.display().to_string(), reason: Box::new(crate::error::Refusal::InUse) }
}

/// The shared hold a catalog keeps while it is open (`pg_dbfile::Live`). It waits a bounded time for a change in progress
/// to finish, and fails if that change does not finish in time.
pub(crate) fn take_shared(path: &Path) -> Result<pg_dbfile::Live> {
    pg_dbfile::Live::shared(path)
        .map_err(|e| Error::step(format!("catalog {}: its open marker", path.display()), e))?
        .ok_or_else(|| Error::Held { path: path.display().to_string(), what: "another program is changing it" })
}

/// Which way an integer mark moved, against this build's: `Raised` when the catalog's mark is older (or missing), `Same`
/// when it is this build's. A newer mark is refused before any mark is set ([`newer_mark`], by `prepare`), so none is
/// set back here: a newer one seen by [`direction`] is `Same`, and nothing is written for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkChange {
    Same,
    Raised,
}

/// The direction of a stored integer mark (`None`: not there) against `current`.
fn direction(stored: Option<i64>, current: i64) -> MarkChange {
    match stored {
        Some(stored) if stored >= current => MarkChange::Same,
        _ => MarkChange::Raised,
    }
}

/// An integer mark of the catalog that is above this build's: a later build made or changed the file. `prepare` refuses
/// such a catalog, as it refuses a newer schema, before anything is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NewerMark {
    /// The `meta` key of the mark.
    pub(crate) name: &'static str,
    /// The stored value, and this build's.
    pub(crate) found: i64,
    pub(crate) current: i64,
}

/// The first of the integer marks (`meta`) that is above this build's `versions`, if any. A mark that is not there, or a
/// file that the marks cannot be read from, has none.
pub(crate) fn newer_mark(conn: &Connection, versions: Versions) -> Option<NewerMark> {
    let marks = [
        ("import_version", versions.import),
        ("borders_version", versions.borders),
        ("pairing_version", versions.pairing),
        ("thumbs_version", versions.thumbs),
    ];
    marks.into_iter().find_map(|(name, current)| {
        let found: i64 =
            conn.query_row("SELECT value FROM meta WHERE key = ?1", [name], |r| r.get(0)).optional().ok().flatten()?;
        (found > current).then_some(NewerMark { name, found, current })
    })
}

/// Set the integer mark `key` to `current` and, when it moved, make the rows follow it: `recompute` runs in the same
/// transaction. Checked before and again inside one immediate write transaction, as [`set_mark`] is.
fn set_mark_i64(
    conn: &Connection,
    key: &str,
    current: i64,
    recompute: impl FnOnce(&Transaction) -> Result<()>,
) -> Result<MarkChange> {
    let stored = |c: &Connection| -> Result<Option<i64>> {
        match c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)) {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    };
    let first = direction(stored(conn)?, current);
    if first == MarkChange::Same {
        return Ok(first);
    }
    let tx = write_transaction(conn)?;
    let change = direction(stored(&tx)?, current);
    if change != MarkChange::Same {
        tx.execute(
            "INSERT INTO meta(key, value) VALUES (?2, ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![current, key],
        )?;
        recompute(&tx)?;
    }
    tx.commit()?;
    Ok(change)
}

/// Make `files.pending` mean this build's IMPORT_VERSION: when the catalog's `meta.import_version` is older (a new
/// catalog, or a build that analyzes differently), every file's is computed again, and every file's `seq` moves on, so
/// the thumbnail pass looks at all of them once. A newer one is refused before this runs ([`newer_mark`]). Checked and
/// set in one write transaction.
pub(crate) fn set_import_version(conn: &Connection, current: i64) -> Result<MarkChange> {
    set_mark_i64(conn, "import_version", current, |tx| {
        Ok(tx.execute_batch(concat!(
            "UPDATE files SET pending = ",
            stale!(),
            "; ",
            next_seq!(),
            "; UPDATE files SET seq = ",
            seq!(),
            ";"
        ))?)
    })
}

/// Set the mark `key` of `meta` to `current`, and when it was another, make the rows follow it (`recompute`): checked
/// before and again inside one immediate write transaction, so two processes opening the catalog at once do it once.
fn set_mark<T>(
    conn: &Connection,
    key: &str,
    current: &T,
    recompute: impl FnOnce(&Transaction) -> Result<()>,
) -> Result<()>
where
    T: rusqlite::types::FromSql + rusqlite::ToSql + PartialEq,
{
    // A mark that is not there (a catalog from before it existed) is another value: it is set, and the rows follow.
    let stored = |c: &Connection| -> Result<Option<T>> {
        match c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)) {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    };
    if stored(conn)?.as_ref() == Some(current) {
        return Ok(());
    }
    let tx = write_transaction(conn)?;
    if stored(&tx)?.as_ref() != Some(current) {
        tx.execute(
            "INSERT INTO meta(key, value) VALUES (?2, ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![current, key],
        )?;
        recompute(&tx)?;
    }
    Ok(tx.commit()?)
}

/// The value of the `meta` row `key`, or `None` when it is not there (or the table is not readable).
pub(crate) fn meta_text(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).ok()
}

/// A folder row as the key recomputation reads it: its id, its path, and its stored path key and match key.
type StoredFolder = (i64, Vec<u8>, Vec<u8>, Vec<u8>);

/// A folder with the keys it is recomputed to: its id, its new and its stored path key, and its new and its stored match
/// key (see [`recompute_keys`]).
type SettledFolder = (i64, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

/// The placeholder a row's key holds while [`recompute_keys`] moves the keys of the rows that change: unique to the row
/// (its id, in a namespace of its own), so no two rows hold it at once and the unique key can't be violated mid-pass.
fn placeholder(namespace: u8, id: i64) -> Vec<u8> {
    std::iter::once(namespace).chain(id.to_be_bytes()).collect()
}

/// Make the stored keys of the folders, the folder ledger and the files follow this build's key algorithm (the
/// `key_version` mark changed, or a carried catalog's rows are made): each folder's `path_key` and `match_key` from its
/// path, then folders that now share a `path_key` merged ([`merge_folder`]: the lowest id is kept), the folder ledger's
/// keys likewise (a folder's failure is one row, the lowest id kept), and each file's order key from its folder's
/// `match_key` and its name, in pages of files by id. Only the rows whose key changed are written. A row whose path
/// doesn't parse under the catalog's rules keeps its keys, which is logged, not an error.
///
/// Paths stored outside the folder rows are not re-keyed: a queued write's temp file (`file_writes.temp`), the paths in
/// a notice's JSON (`notices.data`), and the `folder` column of the `thumb_sources` view, which names the survivor's
/// spelling for a merged folder's files. Nothing looks a folder up by one of them: a file's folder is its `folder_id`.
pub(crate) fn recompute_keys(tx: &Transaction<'_>, rules: PathRules) -> Result<()> {
    let folders: Vec<StoredFolder> = tx
        .prepare("SELECT id, path, path_key, match_key FROM folders ORDER BY id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    // The new keys, by path key: the folders that share one are one folder.
    let mut by_key: BTreeMap<Vec<u8>, Vec<i64>> = BTreeMap::new();
    let mut settled: Vec<SettledFolder> = Vec::with_capacity(folders.len());
    for (id, path, stored_path_key, stored_match) in folders {
        let (path_key, match_key) = match rules.parse_bytes(&path) {
            Some(parsed) => (rules.path_key(&parsed), rules.folder_key(&parsed.parts)),
            None => {
                pg_log::warn(
                    "catalog",
                    &format!("folder {id}: its path doesn't parse under the path rules; its keys are kept"),
                );
                (stored_path_key.clone(), stored_match.clone())
            }
        };
        by_key.entry(path_key.clone()).or_default().push(id);
        settled.push((id, path_key, stored_path_key, match_key, stored_match));
    }
    let mut merged = 0_u64;
    for ids in by_key.values().filter(|ids| ids.len() > 1) {
        let Some((keep, rest)) = ids.split_first() else { continue };
        for &drop in rest {
            merged += merge_folder(tx, *keep, drop)?;
            pg_log::warn(
                "catalog",
                &format!("folder {drop}: the same folder as {keep} under another case; merged into it"),
            );
        }
    }
    if merged > 0 {
        pg_log::warn(
            "catalog",
            &format!("{merged} file(s) moved into a folder they now share a name with, kept as rows"),
        );
    }
    // Two passes, so no row takes a key another row still holds: the changed keys go to placeholders, then to the keys.
    for (id, path_key, stored_path_key, match_key, stored_match) in &settled {
        if by_key.get(path_key).and_then(|ids| ids.first()) != Some(id) {
            continue;
        }
        if path_key != stored_path_key || match_key != stored_match {
            tx.execute(
                "UPDATE folders SET path_key = ?1 WHERE id = ?2",
                rusqlite::params![placeholder(0xFE, *id), id],
            )?;
        }
    }
    for (id, path_key, stored_path_key, match_key, stored_match) in &settled {
        if by_key.get(path_key).and_then(|ids| ids.first()) != Some(id) {
            continue;
        }
        if path_key != stored_path_key || match_key != stored_match {
            tx.execute(
                "UPDATE folders SET path_key = ?1, match_key = ?2 WHERE id = ?3",
                rusqlite::params![path_key, match_key, id],
            )?;
        }
    }
    recompute_failure_keys(tx, rules)?;
    let folder_keys: BTreeMap<i64, Vec<u8>> = tx
        .prepare("SELECT id, match_key FROM folders")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut after = 0_i64;
    loop {
        let page: Vec<(i64, i64, Vec<u8>, Vec<u8>)> = tx
            .prepare("SELECT id, folder_id, name, order_key FROM files WHERE id > ?1 ORDER BY id LIMIT 4096")?
            .query_map([after], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let Some(last) = page.last() else { break };
        after = last.0;
        for (id, folder, name, stored) in page {
            let Some(folder_key) = folder_keys.get(&folder) else { continue };
            let key = rules.order_key(folder_key, &Name::from_raw(&name));
            if key != stored {
                tx.execute("UPDATE files SET order_key = ?1 WHERE id = ?2", rusqlite::params![key, id])?;
            }
        }
    }
    Ok(())
}

/// Merge the folder `drop` into `keep`, the same folder: its files are re-pointed to `keep`, and its row is deleted. The
/// files keep their rows and their names, so two files of one name may now share a folder (no key forbids it, and each
/// file has its own identity). Returns the files moved.
pub(crate) fn merge_folder(tx: &Transaction<'_>, keep: i64, drop: i64) -> Result<u64> {
    let moved = tx.execute("UPDATE files SET folder_id = ?1 WHERE folder_id = ?2", rusqlite::params![keep, drop])?;
    tx.execute("DELETE FROM folders WHERE id = ?1", [drop])?;
    Ok(u64::try_from(moved).unwrap_or(u64::MAX))
}

/// The folder ledger's keys (`folder_failures.path_key`) follow the folders' paths, the same way the folders' do: one
/// row per folder, the lowest id kept, and a row whose path doesn't parse keeps its key.
fn recompute_failure_keys(tx: &Transaction<'_>, rules: PathRules) -> Result<()> {
    let rows: Vec<(i64, Vec<u8>, Vec<u8>)> = tx
        .prepare("SELECT id, path, path_key FROM folder_failures ORDER BY id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut keys: Vec<(i64, Vec<u8>, Vec<u8>)> = Vec::with_capacity(rows.len());
    let mut seen = std::collections::BTreeSet::new();
    for (id, path, stored) in rows {
        let key = rules.parse_bytes(&path).map_or(stored.clone(), |parsed| rules.path_key(&parsed));
        if !seen.insert(key.clone()) {
            tx.execute("DELETE FROM folder_failures WHERE id = ?1", [id])?;
            continue;
        }
        keys.push((id, key, stored));
    }
    for (id, key, stored) in &keys {
        if key != stored {
            tx.execute(
                "UPDATE folder_failures SET path_key = ?1 WHERE id = ?2",
                rusqlite::params![placeholder(0xFD, *id), id],
            )?;
        }
    }
    for (id, key, stored) in &keys {
        if key != stored {
            tx.execute("UPDATE folder_failures SET path_key = ?1 WHERE id = ?2", rusqlite::params![key, id])?;
        }
    }
    Ok(())
}

/// Set the catalog's `key_version` mark to this build's, and recompute the keys when it was another ([`recompute_keys`]).
pub(crate) fn set_key_version(conn: &Connection, rules: PathRules) -> Result<()> {
    set_mark(conn, "key_version", &key_version_text(), |tx| recompute_keys(tx, rules))
}

/// The free space a migration needs, in multiples of the catalog file's size: the copy aside (`rebuild::back_up`) and
/// the migration's WAL, which rewrites at most every page once (it is checkpointed after the commit, before a `VACUUM`).
const MIGRATION_ROOM: u64 = 2;
/// The free space a `VACUUM` needs, in multiples of the file's size: it writes the whole new file through the WAL.
const VACUUM_ROOM: u64 = 1;

/// Whether `free` bytes leave room for `times` the catalog file's `len`; an unknown free space (`None`) goes ahead, as a
/// write that then fails fails as any other does.
fn has_room(len: u64, free: Option<u64>, times: u64) -> bool {
    free.is_none_or(|free| free >= len.saturating_mul(times))
}

/// The folder whose volume holds the catalog file at `path`.
fn folder_of(path: &Path) -> &Path {
    path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// The catalog file's size and the free space of its volume (`None` when it can't be read).
fn size_and_free(path: &Path) -> (u64, Option<u64>) {
    let len = std::fs::metadata(path).map_or(0, |m| m.len());
    (len, pg_platform::volume::space(folder_of(path)).map(|s| s.free))
}

/// Refuse a migration of the catalog at `path` when its volume has no room for it ([`MIGRATION_ROOM`]): before the
/// copy aside, so the catalog is left as it is, and the error says how much is needed and where. `free_space`, when
/// given, reports the volume's free bytes in its place (a test's room).
pub(crate) fn ensure_room_to_migrate(path: &Path, free_space: Option<&FreeSpace<'_>>) -> Result<()> {
    let (len, free) = match free_space {
        None => size_and_free(path),
        Some(space) => (size_and_free(path).0, space(folder_of(path))),
    };
    if has_room(len, free, MIGRATION_ROOM) {
        return Ok(());
    }
    let mb = |bytes: u64| bytes.div_ceil(1_000_000);
    Err(Error::MigrationRoom {
        path: path.display().to_string(),
        needed: mb(len.saturating_mul(MIGRATION_ROOM)),
        free: mb(free.unwrap_or(0)),
    })
}

/// What the warnings of a `VACUUM` that didn't run say about its retry.
const RETRY_AT_START: &str = "it is tried again at the next start";

/// The `VACUUM` a migration left owed (`meta.vacuum_owed`, schema version 13's), after its commit: the WAL is merged
/// first, the file is rewritten without the dropped columns' pages, and the WAL is merged again, then the mark is
/// cleared. Not run without room for it ([`VACUUM_ROOM`]), and a `VACUUM` that fails (busy, the disk filled) fails
/// nothing: the catalog is whole either way, so it is logged and the mark kept, for the next start. `path` is `None`
/// for an in-memory catalog.
pub(crate) fn vacuum_if_owed(conn: &Connection, path: Option<&Path>, exclusive: bool) -> Result<()> {
    let owed: i64 = conn.query_row("SELECT value FROM meta WHERE key = 'vacuum_owed'", [], |r| r.get(0))?;
    if owed == 0 {
        return Ok(());
    }
    // Without the exclusive hold the mark stays, for the start that has it: the `VACUUM` writes the whole file.
    if !exclusive {
        return Ok(());
    }
    if let Some(path) = path {
        let (len, free) = size_and_free(path);
        if !has_room(len, free, VACUUM_ROOM) {
            pg_log::warn(
                "catalog",
                &format!("catalog {}: no room to compact it after its migration; {RETRY_AT_START}", path.display()),
            );
            return Ok(());
        }
    }
    // Incremental auto-vacuum: set first, a `VACUUM` is what makes it take effect on a file that has pages.
    match conn.execute_batch(
        "PRAGMA auto_vacuum = INCREMENTAL; PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);",
    ) {
        Ok(()) => {
            conn.execute("UPDATE meta SET value = 0 WHERE key = 'vacuum_owed'", [])?;
            Ok(())
        }
        Err(e) => {
            let name = path.map_or_else(|| ":memory:".to_string(), |p| p.display().to_string());
            pg_log::warn(
                "catalog",
                &format!("catalog {name}: compacting it after its migration failed ({e:#}); {RETRY_AT_START}"),
            );
            Ok(())
        }
    }
}

/// Mark missing the present files whose extension this build doesn't hold (`media::Ext::of`; a build that held more
/// recorded them): the number marked. The scan records only the extensions it holds, so they never come back, and
/// every read of the catalog (the grid, an entry, the tree, the facets, the status) leaves them out as it leaves out
/// any missing file, where one such row would fail the grid's list. A queued write on one is dropped as any write on
/// a missing file is. At `Catalog::prepare`, and only when `meta.held_exts` is another build's set: like
/// [`set_import_version`], one [`set_mark`], so a start of the same build reads no file. A scan's writes stamp the set
/// of the build that made them ([`stamp_held_exts`]), so the rows a build with more extensions records after this one
/// started are looked at again at this build's next start.
pub(crate) fn mark_unheld_missing(conn: &Connection) -> Result<()> {
    set_mark(conn, "held_exts", &pg_model::media::held_exts(), |tx| {
        let exts: Vec<String> = tx
            .prepare("SELECT DISTINCT ext FROM files WHERE missing = 0")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut mark = tx.prepare("UPDATE files SET missing = 1 WHERE ext = ?1 AND missing = 0")?;
        for ext in exts.iter().filter(|ext| Ext::of(ext.as_str()).is_none()) {
            mark.execute([ext])?;
        }
        Ok(())
    })
}

/// Stamp `meta.held_exts` with this build's set, in the transaction of a write that records or revives files (the
/// scan's [`crate::writers::ScanWriter`], the pairing after a scan): the rows it writes are of the extensions this build holds,
/// so a build that holds fewer looks for them at its next start ([`mark_unheld_missing`]). Writes nothing when the set
/// is this build's already.
pub(crate) fn stamp_held_exts(tx: &Connection) -> Result<()> {
    tx.prepare_cached("UPDATE meta SET value = ?1 WHERE key = 'held_exts' AND value IS NOT ?1")?
        .execute([pg_model::media::held_exts()])?;
    Ok(())
}

/// Make `analysis.borders_due` mean this build's detector: when the catalog's `meta.borders_version` is another,
/// every analysis that can have borders is due. Like [`set_import_version`], one [`set_mark`].
pub(crate) fn set_borders_version(conn: &Connection, current: i64) -> Result<MarkChange> {
    // A change recomputes: a borders detection of another version is not this build's (a newer one is refused before this).
    set_mark_i64(conn, "borders_version", current, |tx| {
        tx.execute_batch(concat!("UPDATE analysis SET borders_due = coalesce(", has_borders!(), ", 0)"))?;
        // Every file is looked at again by the new detector: what the old one failed on is not known to fail.
        tx.execute("DELETE FROM file_failures WHERE pass = 'borders'", [])?;
        Ok(())
    })
}

/// Make the pairs this build's: when the catalog's `meta.pairing_version` is another, every eligible analysis is due again
/// (the pairing pass makes all of them) and the pairs go, with the changes stamped so the views read them again. A newer
pub(crate) fn set_pairing_version(conn: &Connection, current: i64) -> Result<MarkChange> {
    set_mark_i64(conn, "pairing_version", current, |tx| owe_pairs_again(tx).map(|_| ()))
}

/// Every eligible analysis (orientation hashes and a reliable quality) owes its pairs again, and the pairs go, with the
/// change stamped so the views read them again. Returns how many analyses owe their pairs now. The pairing version's
/// change makes it (`set_pairing_version`), and `gallery pair --again` does too.
pub(crate) fn owe_pairs_again(tx: &Transaction) -> Result<u64> {
    // Only the rows not owed already are written: an analysis that owes its pairs keeps its row.
    tx.execute(
        "INSERT OR IGNORE INTO pairs_owed(file_id)
         SELECT file_id FROM analysis WHERE pdq_dihedral IS NOT NULL AND pdq_quality >= ?1",
        [pg_model::pdq::MIN_QUALITY],
    )?;
    tx.execute_batch("DELETE FROM pdq_pairs;")?;
    tx.execute_batch(next_seq!())?;
    Ok(tx.query_row("SELECT count(*) FROM pairs_owed", [], |r| r.get(0))?)
}

/// Make the thumbnails' failure ledger this build's: when the catalog's `meta.thumbs_version` is another
/// (`pg_model::versions::THUMBS_VERSION`, a decoder that may take what the old one failed on), the files whose thumbnail panicked
/// are listed again for the pass (the others are, by the pass's own rule), and the rows go: the next pass records what
/// still fails. The cache's checkpoint holds the panicked files back too, and keeps a mark of its own
/// (`pg_thumbcache::ThumbCache::open`). Like [`set_import_version`], one [`set_mark`]. `relist` does the listing: the
/// caller passes `ledger::failure::relist_thumbs_panics`, so this module does not depend on the ledger.
pub(crate) fn set_thumbs_version(
    conn: &Connection,
    current: i64,
    relist: impl FnOnce(&Transaction) -> Result<()>,
) -> Result<MarkChange> {
    set_mark_i64(conn, "thumbs_version", current, relist)
}

/// Make `analysis.taken` follow this system's zone: when `meta.taken_zone` is another fingerprint (the zone changed,
/// or the catalog was made under another), every UTC-sourced `taken` is computed again from its `taken_utc` instant.
/// Like [`set_import_version`], one [`set_mark`]. The stored instants are what make this possible: the wall
/// clock alone would lose the instant of a repeated hour.
pub(crate) fn set_taken_zone(conn: &Connection) -> Result<()> {
    let current: String = conn.query_row(TAKEN_ZONE_QUERY, [], |r| r.get(0))?;
    set_mark(conn, "taken_zone", &current, |tx| {
        Ok(tx.execute_batch(
            "UPDATE analysis SET taken = unixepoch(taken_utc, 'unixepoch', 'localtime') WHERE taken_utc IS NOT NULL",
        )?)
    })
}

#[cfg(test)]
mod key_version_tests {
    use pg_model::ids::{FileId, FolderId};
    use pg_model::name::Name;
    use pg_platform::native::PATH_RULES;
    use rusqlite::{Connection, params};

    use crate::Catalog;
    use crate::fixture::{File, insert, state};
    use crate::test_support::versions;
    use crate::types::key_version_text;

    /// Seeds the `key_version` mark into a file a test made by hand, as an earlier layout (its rows are the test's): a file
    /// without the mark is refused as one that predates it, so the test that means to carry its rows seeds it. The
    /// mark is this build's, so the carry copies it and no key is recomputed.
    pub(crate) fn seed_key_version(path: &std::path::Path) {
        Connection::open(path)
            .unwrap()
            .execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('key_version', ?1)", [key_version_text()])
            .unwrap();
    }

    /// The stored keys: the folder's, then each file's order key, by id.
    fn keys_of(c: &Catalog) -> Vec<Vec<u8>> {
        let folders: Vec<Vec<u8>> = c
            .conn
            .prepare("SELECT match_key FROM folders ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let files: Vec<Vec<u8>> = c
            .conn
            .prepare("SELECT order_key FROM files ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        folders.into_iter().chain(files).collect()
    }

    /// A catalog at `name` with one folder and two files, and the keys they were stored with.
    fn catalog_with_files(name: &str) -> (std::path::PathBuf, Vec<Vec<u8>>) {
        let dir = pg_testkit::run_folder("scratch_catalog", name).unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), PATH_RULES).unwrap();
        state::add_scan(&c, 1, "D:\\p").unwrap();
        state::add_folder(&c, 1, "D:\\p").unwrap();
        insert(
            &c,
            &[
                File::new(FileId::new(1), FolderId::new(1), "a.jpg", 1, 5),
                File::new(FileId::new(2), FolderId::new(1), "b.jpg", 1, 5),
            ],
        )
        .unwrap();
        let keys = keys_of(&c);
        drop(c);
        (path, keys)
    }

    /// A later build's integer mark is refused by this build, as a newer schema is: the catalog is not changed, and the
    /// mark stays the later build's.
    #[test]
    fn a_newer_import_mark_is_refused_and_the_catalog_is_left_as_it_was() {
        use crate::error::Refusal;
        use pg_model::versions::IMPORT_VERSION;
        let dir = pg_testkit::run_folder("scratch_catalog", "newer-import").unwrap();
        let path = dir.join("catalog.sqlite");
        let newer = crate::Versions { import: IMPORT_VERSION + 1, ..versions() };
        drop(Catalog::prepare(&path, newer, PATH_RULES).unwrap());
        let err = Catalog::prepare(&path, versions(), PATH_RULES).err().expect("refused");
        let (_, reason) = err.refusal().expect("a typed refusal");
        assert!(matches!(reason, Refusal::NewerMark { mark: "import_version", .. }), "{reason:?}");
        let raw = Connection::open(&path).unwrap();
        let mark: i64 = raw.query_row("SELECT value FROM meta WHERE key = 'import_version'", [], |r| r.get(0)).unwrap();
        assert_eq!(mark, IMPORT_VERSION + 1, "the later build's mark is kept");
    }

    /// A later build's mark for a pass refuses this build's records of that pass, and not another pass's.
    #[test]
    fn a_later_pass_mark_refuses_this_builds_records_of_that_pass() {
        use pg_model::notice::FilePass;
        use pg_model::versions::{BORDERS_VERSION, THUMBS_VERSION};
        let (path, _) = catalog_with_files("pass-marks");
        let raw = Connection::open(&path).unwrap();
        raw.execute("UPDATE meta SET value = ?1 WHERE key = 'thumbs_version'", [THUMBS_VERSION + 1]).unwrap();
        assert!(!crate::writers::pass_mark_is_ours(&raw, FilePass::Thumbs).unwrap());
        assert!(crate::writers::pass_mark_is_ours(&raw, FilePass::Borders).unwrap());
        raw.execute("UPDATE meta SET value = ?1 WHERE key = 'borders_version'", [BORDERS_VERSION + 1]).unwrap();
        assert!(!crate::writers::pass_mark_is_ours(&raw, FilePass::Borders).unwrap());
    }

    /// Keys stored by another algorithm version (or toolchain) are recomputed when the catalog is prepared, and the mark
    /// is this build's afterwards.
    #[test]
    fn stale_keys_are_recomputed_when_the_mark_is_another() {
        let (path, keys) = catalog_with_files("another-mark");
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch(
            "UPDATE folders SET match_key = x'00'; UPDATE files SET order_key = x'00';
             UPDATE meta SET value = 'an older key algorithm' WHERE key = 'key_version';",
        )
        .unwrap();
        drop(raw);
        let c = Catalog::prepare(&path, versions(), PATH_RULES).unwrap();
        assert_eq!(keys_of(&c), keys, "the keys are this build's again");
        assert_eq!(super::meta_text(&c.conn, "key_version").as_deref(), Some(key_version_text().as_str()));
    }

    /// A rebuild carries the rows, and the carry needs the mark to say which keys they have: a catalog without the mark
    /// whose schema differs from this build's is refused, with the file untouched.
    #[test]
    fn a_rebuild_of_a_catalog_without_the_mark_is_refused() {
        let (path, _) = catalog_with_files("refused");
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("DROP INDEX analysis_failed; DELETE FROM meta WHERE key = 'key_version';").unwrap();
        drop(raw);
        let refused = Catalog::prepare(&path, versions(), PATH_RULES);
        let message = refused.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(message.contains("predates key_version"), "{message}");
        let raw = Connection::open(&path).unwrap();
        let indexes: i64 = raw
            .query_row("SELECT count(*) FROM sqlite_master WHERE name = 'analysis_failed'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(indexes, 0, "the file is as the refusal left it");
    }

    /// A large catalog with half its order keys stale: the recompute makes every key this build's, and writes the rows
    /// that changed (one transaction, paged by id). The wall clock is not asserted; the profile's budget gates it.
    #[test]
    fn a_large_catalog_has_its_stale_keys_recomputed() {
        const FILES: i64 = 200_000;
        let dir = pg_testkit::run_folder("scratch_catalog", "large").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), PATH_RULES).unwrap();
        state::add_scan(&c, 1, "D:\\p").unwrap();
        state::add_folder(&c, 1, "D:\\p").unwrap();
        let folder_key: Vec<u8> =
            c.conn.query_row("SELECT match_key FROM folders WHERE id = 1", [], |r| r.get(0)).unwrap();
        let tx = c.conn.unchecked_transaction().unwrap();
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO files(volume, file_ref, folder_id, name, ext, kind, size, mtime, order_key)
                     VALUES (1, ?1, 1, ?2, 'jpg', 'image', 1, 1, ?3)",
                )
                .unwrap();
            for n in 0..FILES {
                let name = format!("f{n:06}.jpg");
                // Every other file's key is the stale one.
                let key =
                    if n % 2 == 0 { PATH_RULES.order_key(&folder_key, &Name::from(name.as_str())) } else { vec![0] };
                insert.execute(params![n + 1, name.as_bytes(), key]).unwrap();
            }
        }
        tx.commit().unwrap();
        c.conn.execute("UPDATE meta SET value = 'an older key algorithm' WHERE key = 'key_version'", []).unwrap();
        drop(c);
        let c = Catalog::prepare(&path, versions(), PATH_RULES).unwrap();
        let mut wrong = 0_i64;
        let mut stmt = c.conn.prepare("SELECT name, order_key FROM files").unwrap();
        for row in stmt.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))).unwrap() {
            let (name, key) = row.unwrap();
            if key != PATH_RULES.order_key(&folder_key, &Name::from_raw(&name)) {
                wrong += 1;
            }
        }
        assert_eq!(wrong, 0, "every key is this build's after the recompute");
    }
}

#[cfg(test)]
mod room_tests {
    use super::has_room;

    #[test]
    fn room_is_a_multiple_of_the_file_and_unknown_room_goes_ahead() {
        assert!(has_room(100, Some(200), 2));
        assert!(!has_room(100, Some(199), 2));
        assert!(has_room(100, None, 2), "an unknown free space goes ahead");
        assert!(has_room(0, Some(0), 2), "a new file needs nothing");
        assert!(!has_room(u64::MAX, Some(u64::MAX - 1), 1));
    }
}

#[cfg(test)]
mod tests {
    use super::set_taken_zone;
    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::{self, raw};
    use crate::schema::{
        BASELINE, Hooks, MIGRATIONS, SchemaSet, bring_to, current_expected, current_version, mismatch,
    };
    use crate::test_support::*;
    use crate::types::TAKEN_ZONE_QUERY;
    use crate::types::VolumeSet;
    use crate::types::rules_name;
    use crate::work::pending_import_sql;
    use pg_model::notice::Carried;

    use crate::{Catalog, Result, ScannedFile};
    use pg_model::analysis::Taken;
    use pg_model::ids::{FileId, FileIdentity, FolderId};
    use pg_model::media::Ext;
    use pg_model::name::Name;
    use pg_model::rules::PathRules;
    use pg_model::versions::IMPORT_VERSION;
    use pg_testkit::run_folder;
    use rusqlite::Connection;
    use std::path::Path;

    /// Where this module's tests write (`PG_TEST_DIR` when it is set).
    const SCRATCH: &str = "scratch_catalog";

    /// The layout before the order key (2026-10-31), as the catalogs made with it have it.
    const BEFORE_ORDER_KEY: &str = r"
        PRAGMA journal_mode = WAL;
        CREATE TABLE meta(key TEXT PRIMARY KEY, value ANY NOT NULL);
        INSERT INTO meta(key, value) VALUES ('path_rules', 'windows');
        CREATE TABLE scans(id INTEGER PRIMARY KEY, root TEXT NOT NULL, started INTEGER NOT NULL, finished INTEGER);
        CREATE TABLE folders(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE);
        CREATE TABLE files(id INTEGER PRIMARY KEY, volume INTEGER NOT NULL, file_ref INTEGER NOT NULL,
            folder_id INTEGER NOT NULL, name TEXT NOT NULL, ext TEXT NOT NULL, kind TEXT NOT NULL,
            size INTEGER NOT NULL, mtime INTEGER NOT NULL, last_scan INTEGER NOT NULL,
            missing INTEGER NOT NULL DEFAULT 0, rotation INTEGER NOT NULL DEFAULT 0, UNIQUE(volume, file_ref));
        CREATE TABLE analysis(file_id INTEGER PRIMARY KEY, version INTEGER NOT NULL, size INTEGER NOT NULL,
            mtime INTEGER NOT NULL, analyzed INTEGER NOT NULL, error TEXT, file_hash BLOB, pixel_hash BLOB,
            format TEXT, orientation INTEGER, width INTEGER, height INTEGER, pdq BLOB, pdq_quality INTEGER,
            thumbhash BLOB, frames INTEGER, taken INTEGER, duration_ms INTEGER, poster_ms INTEGER);
        CREATE TABLE pdq_pairs(a INTEGER NOT NULL, b INTEGER NOT NULL, distance INTEGER NOT NULL,
            variant INTEGER NOT NULL, PRIMARY KEY(a, b)) WITHOUT ROWID;
        CREATE TABLE roots(path TEXT PRIMARY KEY, added INTEGER NOT NULL);
        CREATE TABLE ratings(file_id INTEGER PRIMARY KEY, rating INTEGER NOT NULL, updated INTEGER NOT NULL);
        CREATE TABLE file_ratings(file_id INTEGER PRIMARY KEY, value INTEGER NOT NULL);
        CREATE TABLE file_writes(file_id INTEGER NOT NULL, kind TEXT NOT NULL, value INTEGER NOT NULL,
            queued INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, error TEXT, temp TEXT, temp_value INTEGER,
            PRIMARY KEY(file_id, kind));
        CREATE TABLE views(name TEXT PRIMARY KEY, folder TEXT NOT NULL, recursive INTEGER NOT NULL,
            query TEXT NOT NULL, sort TEXT NOT NULL);
        CREATE TABLE session(key TEXT PRIMARY KEY, value TEXT NOT NULL);
        PRAGMA user_version = 1;";

    /// `pairs_again` merges the WAL it writes into the file, so the pass that follows does not merge it: the `-wal`
    /// is empty once it returns.
    #[test]
    fn pairs_again_merges_its_wal_into_the_file() {
        let dir = run_folder(SCRATCH, "pairs-again-wal").unwrap();
        let path = dir.join("catalog.sqlite");
        let mut c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        c.pairs_again().unwrap();
        let wal = std::fs::metadata(dir.join("catalog.sqlite-wal")).map(|m| m.len()).unwrap_or(0);
        assert_eq!(wal, 0, "the WAL `pairs_again` wrote is still in the -wal file");
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The layout tests below read the file on disk through their own connection (`raw::on_disk`): what they check
    // is the layout, not the catalog's tables.
    /// A catalog that isn't there is an error to open, and opening it makes no marker beside where it would be.
    #[test]
    fn opening_a_catalog_that_is_not_there_makes_no_marker() {
        let dir = run_folder(SCRATCH, "not-there").unwrap();
        let path = dir.join("catalog.sqlite");
        assert!(Catalog::open(&path, pg_platform::native::PATH_RULES).is_err());
        assert!(!dir.join("catalog.sqlite.live").exists(), "no marker for a catalog that isn't there");
    }

    /// A catalog another connection has open is not moved aside: its rebuild is refused, with the file as it was, and it
    /// is rebuilt once that connection has closed. The edit to the file is made on a raw connection, which holds nothing.
    #[test]
    fn a_catalog_another_connection_has_open_is_not_moved_aside() {
        let dir = run_folder(SCRATCH, "open-elsewhere").unwrap();
        let path = dir.join("catalog.sqlite");
        let open = Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap();
        let edited = Connection::open(&path).unwrap();
        edited.execute_batch("CREATE INDEX extra_names ON files(name);").unwrap();
        drop(edited);

        let err = Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).err().expect("refused");
        assert!(format!("{err:#}").contains("has it open"), "{err:#}");
        assert!(path.exists(), "the catalog is where it was");
        assert!(!dir.join("catalog.sqlite.replaced").exists(), "nothing was moved aside");
        drop(open);

        let c = Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap();
        assert!(c.rebuilt().is_some(), "moved aside once nothing has it open");
    }

    // Windows-only: the earlier layout is a Windows catalog, its rows are `D:\` paths recorded as `windows`.
    #[test]
    fn an_earlier_layout_is_moved_aside_and_a_new_catalog_is_made() {
        let dir = run_folder(SCRATCH, "move-aside").unwrap();
        let path = dir.join("catalog.sqlite");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(BEFORE_ORDER_KEY).unwrap();
        old.execute_batch(&format!(
            r"INSERT INTO scans VALUES (1, 'D:\p', 10, 20);
              INSERT INTO folders VALUES (1, 'D:\p'), (2, 'D:\p\Sub');
              INSERT INTO files VALUES (11, 7, 101, 1, 'B.jpg', 'jpg', 'image', 100, 5, 1, 0, 0),
                                       (12, 7, 102, 2, 'a.png', 'png', 'image', 200, 6, 1, 0, 1),
                                       (13, 7, 103, 1, 'new.jpg', 'jpg', 'image', 300, 7, 1, 0, 0),
                                       (14, 7, 104, 1, 'changed.jpg', 'jpg', 'image', 400, 8, 1, 0, 0);
              INSERT INTO analysis(file_id, version, size, mtime, analyzed, width, height)
                  VALUES (11, {IMPORT_VERSION}, 100, 5, 30, 640, 480), (12, {IMPORT_VERSION}, 200, 6, 30, 10, 20),
                         (14, {IMPORT_VERSION}, 399, 8, 30, 1, 1);
              INSERT INTO pdq_pairs VALUES (11, 12, 3, 0);
              INSERT INTO roots VALUES ('D:\p', 1);
              INSERT INTO ratings VALUES (11, 4, 40), (13, 1, 41);
              INSERT INTO file_ratings VALUES (11, 4);
              INSERT INTO file_writes(file_id, kind, value, queued) VALUES (12, 'rotation', 1, 50);
              INSERT INTO views VALUES ('best', 'p', 1, '{{}}', '{{}}');
              INSERT INTO session VALUES ('folder', 'p');"
        ))
        .unwrap();
        drop(old);

        super::key_version_tests::seed_key_version(&path);
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        let disk = raw::on_disk(&path);
        assert!(is_current(&disk).unwrap());
        let count = |conn: &Connection, table: &str| -> i64 {
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
        };
        // The old layout lacks a column the new one needs as NOT NULL (`files.order_key`; the folders' keys are made from
        // their paths by the carry): the copy fails as a whole, so nothing of the old rows is in the new catalog (not even
        // its folders), and the reason is said. The earlier one is kept whole, rows and all, beside it.
        let r = c.rebuilt().expect("moved aside");
        assert!(
            matches!(&r.carried, Carried::Nothing { reason: why } if why.starts_with("NOT NULL constraint failed")),
            "{:?}",
            r.carried
        );
        assert_eq!(count(&disk, "files"), 0);
        assert_eq!(count(&disk, "roots"), 0);
        assert!(c.roots().unwrap().is_empty());
        let moved = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(count(&moved, "files"), 4);
        assert_eq!(count(&moved, "roots"), 1);
        drop(moved);
        drop(disk);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A catalog that differs from this build's schema only by an index it shouldn't have is rebuilt with its rows: the
    /// same files under the same ids, and the library folders.
    fn a_catalog_that_differs_keeps_its_rows(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("keeps-rows-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        crate::fixture::state::add_scan(&c, 1, &spelled(rules, r"D:\p")).unwrap();
        crate::fixture::state::add_folder(&c, 1, &spelled(rules, r"D:\p")).unwrap();
        crate::fixture::state::add_folder(&c, 2, &spelled(rules, r"D:\p\Sub")).unwrap();
        fixture::insert(
            &c,
            &[
                fixture::File::new(FileId::new(11), FolderId::new(1), "B.jpg", 100, 5),
                fixture::File::new(FileId::new(12), FolderId::new(2), "a.png", 200, 6),
            ],
        )
        .unwrap();
        assert!(c.add_root(&p(rules, r"D:\p")).unwrap());
        drop(c);
        let edited = Connection::open(&path).unwrap();
        edited.execute_batch("CREATE INDEX extra_names ON files(name);").unwrap();
        drop(edited);

        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let r = c.rebuilt().expect("moved aside");
        assert!(r.why.contains("extra_names"), "{}", r.why);
        assert_eq!(r.carried, Carried::Rows { files: 2, folders: 1 });
        let disk = raw::on_disk(&path);
        assert!(is_current(&disk).unwrap());
        let names: Vec<(i64, String)> = disk
            .prepare("SELECT id, CAST(name AS TEXT) FROM files ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(names, [(11, "B.jpg".to_string()), (12, "a.png".to_string())]);
        assert_eq!(c.roots().unwrap().len(), 1);
        drop(disk);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        a_catalog_that_differs_keeps_its_rows,
        a_catalog_that_differs_keeps_its_rows_windows,
        a_catalog_that_differs_keeps_its_rows_posix
    );

    // Unix: a catalog in a folder whose name has no UTF-8 form is rebuilt with its rows, as the old file is attached
    // by its exact bytes.
    #[cfg(unix)]
    #[test]
    fn a_catalog_in_a_folder_with_a_name_that_is_not_utf8_keeps_its_rows_on_unix() {
        use std::os::unix::ffi::OsStringExt;
        let base = run_folder(SCRATCH, "odd-folder-name").unwrap();
        let dir = base.join(std::ffi::OsString::from_vec(b"odd-\xFF".to_vec()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("catalog.sqlite");
        let rules = PathRules::Posix;
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        crate::fixture::state::add_scan(&c, 1, "/p").unwrap();
        crate::fixture::state::add_folder(&c, 1, "/p").unwrap();
        fixture::insert(&c, &[fixture::File::new(FileId::new(11), FolderId::new(1), "B.jpg", 100, 5)]).unwrap();
        assert!(c.add_root(&p(rules, "/p")).unwrap());
        drop(c);
        let edited = Connection::open(&path).unwrap();
        edited.execute_batch("CREATE INDEX extra_names ON files(name);").unwrap();
        drop(edited);

        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let r = c.rebuilt().expect("moved aside");
        assert_eq!(r.carried, Carried::Rows { files: 1, folders: 1 });
    }

    /// On Windows, a catalog in a folder whose name has no UTF-8 form is opened, and rebuilt with its rows carried (the
    /// carry attaches the old file by name), through the alias its caller opens it by (`pg_platform::sqlite::openable_folder`).
    #[cfg(windows)]
    #[test]
    fn a_catalog_in_a_folder_without_a_utf8_name_is_opened_and_carried_through_an_alias_on_windows() {
        let dir = run_folder(SCRATCH, "odd-folder").unwrap();
        let folder = dir.join(pg_platform::names::os_name(&pg_model::name::Name::from_raw(b"cat\xED\xA0\x80")));
        std::fs::create_dir_all(&folder).unwrap();
        let alias = pg_platform::sqlite::openable_folder(&folder, &dir.join("aliases")).unwrap();
        assert_ne!(alias, folder, "SQLite can't name the folder: it is opened through an alias");
        let path = alias.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap());
        drop(Catalog::open(&path, pg_platform::native::PATH_RULES).unwrap());
        // The layout of before the queued temps were bytes: the rebuild carries the old file's rows, attached by name.
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            r"DROP TABLE file_writes;
              CREATE TABLE file_writes(
                  file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                  kind     TEXT NOT NULL CHECK (kind IN ('rating', 'rotation')),
                  value    INTEGER NOT NULL,
                  queued   INTEGER NOT NULL,
                  attempts INTEGER NOT NULL DEFAULT 0,
                  error    TEXT,
                  temp       TEXT,
                  temp_value INTEGER,
                  PRIMARY KEY(file_id, kind));",
        )
        .unwrap();
        drop(old);
        let c = Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap();
        assert!(c.rebuilt().is_some(), "moved aside, and its rows carried");
        drop(c);
        assert!(folder.join("catalog.sqlite").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rows stored under other rules are not carried into a catalog of these: their paths don't read as its own.
    #[test]
    fn rows_of_other_rules_are_not_carried_over() {
        let dir = run_folder(SCRATCH, "folders-rules").unwrap();
        let path = dir.join("catalog.sqlite");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(BEFORE_ORDER_KEY).unwrap();
        old.execute_batch(r"INSERT INTO roots VALUES ('D:\p', 1);").unwrap();
        drop(old);
        super::key_version_tests::seed_key_version(&path);
        let c = Catalog::prepare(&path, versions(), PathRules::Posix).unwrap();
        let r = c.rebuilt().expect("moved aside");
        assert_eq!(r.carried, Carried::Nothing { reason: "made with the windows path rules".to_string() });
        assert!(c.roots().unwrap().is_empty());
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("layout_source-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        let fresh = raw::on_disk(&path);
        assert!(is_current(&fresh).unwrap());
        // The same names and counts, the text before the size was added: the layout check must refuse it.
        let text = |name: &str| -> String {
            fresh.query_row("SELECT sql FROM sqlite_schema WHERE name = ?1", [name], |r| r.get(0)).unwrap()
        };
        let (trigger, view) = (text("files_source_changed"), text("thumb_sources"));
        let old_trigger = trigger.replace(", size ON files", " ON files").replace(
            "
    OR old.size IS NOT new.size",
            "",
        );
        let old_view = view.replace("f.size, f.seq", "f.seq");
        assert!(old_trigger != trigger && old_view != view);
        fresh
            .execute_batch(&format!(
                "DROP TRIGGER files_source_changed; {old_trigger}; DROP VIEW thumb_sources; {old_view};"
            ))
            .unwrap();
        drop(fresh);
        let earlier = raw::on_disk(&path);
        assert!(!is_current(&earlier).unwrap());
        drop(earlier);

        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let disk = raw::on_disk(&path);
        assert!(is_current(&disk).unwrap());
        let current: String = disk
            .query_row(
                "SELECT group_concat(sql, ' ') FROM sqlite_schema WHERE name IN ('files_source_changed', 'thumb_sources')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(current.contains("old.size IS NOT new.size") && current.contains("f.size, f.seq"), "{current}");
        assert!(dir.join("catalog.sqlite.replaced").exists());
        drop(disk);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside,
        a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside_windows,
        a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside_posix
    );

    fn a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("move-aside-twice-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // Files without the columns the current layout needs.
        let old_layout = |path: &Path| {
            let conn = Connection::open(path).unwrap();
            // The key mark is seeded: a file without it is refused as one that predates it (the carry needs the mark).
            conn.execute_batch(&format!(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value ANY NOT NULL); INSERT INTO meta VALUES ('path_rules', '{}');
                 INSERT INTO meta VALUES ('key_version', '{}');
                 CREATE TABLE files(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO files VALUES (1, 'x');
                 PRAGMA user_version = 1;",
                rules_name(rules),
                crate::types::key_version_text()
            ))
            .unwrap();
        };
        old_layout(&path);
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        let moved = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(moved.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(moved);
        // Another one moved aside later keeps the first: the new catalog made in the first one's place goes first.
        for part in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(dir.join(format!("catalog.sqlite{part}")));
        }
        old_layout(&path);
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        assert!(dir.join("catalog.sqlite.replaced.1").exists());
        assert!(dir.join("catalog.sqlite.replaced").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name,
        a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name_windows,
        a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name_posix
    );

    fn an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("move-aside-no-rules-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // An older build's catalog: no `path_rules` row, and the files without the current layout's columns.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value ANY NOT NULL);
                 CREATE TABLE files(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO files VALUES (1, 'x');
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        super::key_version_tests::seed_key_version(&path);
        let c = Catalog::prepare(&path, versions(), rules).expect("rebuilt, not refused");
        drop(c);
        let moved = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(moved.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(moved);
        // The new catalog records its rules, so it opens again.
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused,
        an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused_windows,
        an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused_posix
    );

    fn catalogs_have_their_own_id(rules: PathRules) {
        let (a, b) = (
            Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap(),
            Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap(),
        );
        assert_eq!(a.catalog_id().unwrap().len(), 32);
        assert_ne!(a.catalog_id().unwrap(), b.catalog_id().unwrap());
    }

    under_both_rules!(catalogs_have_their_own_id, catalogs_have_their_own_id_windows, catalogs_have_their_own_id_posix);

    #[test]
    fn a_catalog_made_under_other_rules_is_refused() {
        let dir = run_folder(SCRATCH, "path-rules").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PathRules::Windows).unwrap());
        // Another build's rules: its paths would read as other paths. The catalog is valid, so it is refused, not moved.
        let e = Catalog::open(&path, PathRules::Posix).err().expect("refused");
        assert!(matches!(e.refusal(), Some((_, crate::error::Refusal::PathRules { .. }))), "{e:?}");
        assert!(e.to_string().contains("on the platform that made it"), "{e}");
        let again = Catalog::prepare(&path, versions(), PathRules::Posix).err().expect("refused");
        assert!(matches!(again.refusal(), Some((_, crate::error::Refusal::PathRules { .. }))), "{again:?}");
        assert!(!dir.join("catalog.sqlite.replaced").exists());
        // Its rules row gone is not this build's schema: it is moved aside, and the new catalog is made.
        raw::on_disk(&path).execute("DELETE FROM meta WHERE key = 'path_rules'", []).unwrap();
        assert!(Catalog::open(&path, PathRules::Windows).is_err());
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        assert!(c.rebuilt().is_some());
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn posix_paths_are_bytes_under_case_sensitive_rules() {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), PathRules::Posix).unwrap();
        let rules = c.rules();
        crate::fixture::state::add_scan(&c, 1, "/photos").unwrap();
        crate::fixture::state::add_folder(&c, 1, "/photos").unwrap();
        crate::fixture::state::add_folder(&c, 2, "/photos/Pics").unwrap();
        crate::fixture::state::add_folder(&c, 3, "/photos/pics").unwrap();
        fixture::insert(
            &c,
            &[
                fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100),
                fixture::File::new(FileId::new(2), FolderId::new(2), "b.jpg", 10, 100),
                fixture::File::new(FileId::new(3), FolderId::new(3), "b.jpg", 10, 100),
            ],
        )
        .unwrap();
        let root = |text: &str| rules.parse(text).expect("a test path");
        assert!(c.add_root(&root("/photos")).unwrap());
        assert!(!c.add_root(&root("/photos/Pics")).unwrap(), "inside a root");
        assert!(c.add_root(&root("/Photos/x")).unwrap(), "case is significant on POSIX");
        assert_eq!(rules.display(&c.file_path(FileId::new(2)).unwrap().unwrap()), "/photos/Pics/b.jpg");
        // Folders that differ only in case are two folders, and their files are two files.
        assert_eq!(c.file_by_path(&root("/photos/Pics/b.jpg")).unwrap(), Some(FileId::new(2)));
        assert_eq!(c.file_by_path(&root("/photos/pics/b.jpg")).unwrap(), Some(FileId::new(3)));
        assert_eq!(c.files_under(&root("/photos/Pics")).unwrap().len(), 1);
    }

    /// Whether the catalog at `conn` is this build's schema at its version: the check [`Catalog::prepare`] makes.
    fn is_current(conn: &Connection) -> Result<bool> {
        Ok(mismatch(conn, current_expected()?)?.is_none())
    }

    /// Why the preparation moved the old catalog aside (`None` when it kept it).
    fn rebuilt_why(c: &Catalog) -> Option<String> {
        c.rebuilt().map(|r| r.why.clone())
    }

    /// A catalog prepared at `path`, then changed by `sql`, which is run on it as the file is (closed again).
    fn edited(rules: PathRules, path: &Path, sql: &str) {
        drop(Catalog::prepare(path, versions(), rules).unwrap());
        raw::on_disk(path).execute_batch(sql).unwrap();
    }

    fn preparing_a_catalog_again_rebuilds_nothing(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("prepare-twice-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        let again = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(again.rebuilt().is_none());
        drop(again);
        assert!(!dir.join("catalog.sqlite.replaced").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        preparing_a_catalog_again_rebuilds_nothing,
        preparing_a_catalog_again_rebuilds_nothing_windows,
        preparing_a_catalog_again_rebuilds_nothing_posix
    );

    fn a_changed_definition_with_the_same_names_is_moved_aside(rules: PathRules) {
        // The same object names: an added column, an index on other columns, an object this build doesn't make.
        for (name, sql, named) in [
            ("edited-column", "ALTER TABLE files ADD COLUMN extra INTEGER;", "files"),
            ("edited-index", "DROP INDEX files_changed; CREATE INDEX files_changed ON files(name);", "files_changed"),
            ("edited-table", "CREATE TABLE extra(x INTEGER);", "extra"),
        ] {
            let dir = run_folder(SCRATCH, &format!("{name}-{}", rules_name(rules))).unwrap();
            let path = dir.join("catalog.sqlite");
            edited(rules, &path, sql);
            let c = Catalog::prepare(&path, versions(), rules).unwrap();
            let why = rebuilt_why(&c).unwrap_or_else(|| panic!("{name}: not moved aside"));
            assert!(why.contains(named), "{name}: {why}");
            assert!(is_current(&raw::on_disk(&path)).unwrap());
            drop(c);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    under_both_rules!(
        a_changed_definition_with_the_same_names_is_moved_aside,
        a_changed_definition_with_the_same_names_is_moved_aside_windows,
        a_changed_definition_with_the_same_names_is_moved_aside_posix
    );

    fn a_comment_in_a_stored_definition_is_not_a_schema_change(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("comment-edit-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // A catalog whose `files` table was written with a comment in it: the same schema.
        let commented =
            BASELINE.replacen("CREATE TABLE \"files\"(", "CREATE TABLE \"files\"( -- an added comment\n", 1);
        assert_ne!(commented, BASELINE);
        let conn = Connection::open(&path).unwrap();
        let set = SchemaSet { baseline: &commented, migrations: MIGRATIONS };
        bring_to(&conn, &set, rules, current_version().unwrap(), &Hooks::none()).unwrap();
        drop(conn);
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(c.rebuilt().is_none(), "a comment moved the catalog aside");
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        a_comment_in_a_stored_definition_is_not_a_schema_change,
        a_comment_in_a_stored_definition_is_not_a_schema_change_windows,
        a_comment_in_a_stored_definition_is_not_a_schema_change_posix
    );

    /// A file whose extension this build doesn't hold (a build that held more recorded it) is marked missing when the
    /// catalog is prepared, so the grid lists the rest, and every count leaves it out.
    fn a_file_of_an_extension_not_held_is_missing_after_prepare(rules: PathRules) {
        use pg_model::query::{Query, Sort};

        let dir = run_folder(SCRATCH, &format!("unheld-ext-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        {
            let mut c = Catalog::prepare(&path, versions(), rules).unwrap();
            c.add_root(&p(rules, r"D:\x")).unwrap();
            let scan = begin(&mut c, &p(rules, r"D:\x"));
            for (n, name) in ["a.jpg", "b.jpg"].into_iter().enumerate() {
                let tx = c.tx().unwrap();
                let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
                let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
                let file = ScannedFile {
                    identity: FileIdentity::new(1, 10 + u64::try_from(n).unwrap()),
                    name: &Name::from(name),
                    ext: Ext::of_name(name).unwrap(),
                    size: 10,
                    mtime: 100,
                    created: None,
                    prior: None,
                };
                writer.record_file(&folder, &file).unwrap();
                drop(writer);
                tx.commit().unwrap();
            }
            // What a build that holds `xyz` leaves: its row, and its set in the mark (its scan stamped it).
            c.conn.execute("UPDATE files SET ext = 'xyz' WHERE name = ?1", [b"b.jpg".to_vec()]).unwrap();
            c.conn.execute("UPDATE meta SET value = 'jpg,xyz;mp4' WHERE key = 'held_exts'", []).unwrap();
        }
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let missing: i64 = c.conn.query_row("SELECT missing FROM files WHERE ext = 'xyz'", [], |r| r.get(0)).unwrap();
        assert_eq!(missing, 1);
        let mark: String =
            c.conn.query_row("SELECT value FROM meta WHERE key = 'held_exts'", [], |r| r.get(0)).unwrap();
        assert_eq!(mark, pg_model::media::held_exts());
        let all = pg_model::rules::Merged::default();
        assert_eq!(c.list_grid(&all, true, &Query::default(), Sort::default()).unwrap().len(), 1);
        let status = c.status().unwrap();
        assert_eq!((status.files, status.missing), (1, 1));
        assert_eq!(c.folder_tree().unwrap().total(), 1);
        let counts = c.facets(&all, true, &Query::default()).unwrap();
        assert_eq!(counts.total, 1);
        assert!(counts.exts.iter().all(|(ext, _)| ext != "xyz"), "{:?}", counts.exts);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        a_file_of_an_extension_not_held_is_missing_after_prepare,
        a_file_of_an_extension_not_held_is_missing_after_prepare_windows,
        a_file_of_an_extension_not_held_is_missing_after_prepare_posix
    );

    /// `prepare` looks for unheld extensions only when the mark is another build's set, and a scan's writes (its
    /// writer, and the pairing after it) stamp this build's: so a row another build records after this one started is
    /// looked at again at this build's next start, and a start after this build's own scan reads no file.
    fn the_unheld_extensions_are_looked_for_when_another_build_wrote(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("held-exts-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        let mark = |c: &Catalog| -> String {
            c.conn.query_row("SELECT value FROM meta WHERE key = 'held_exts'", [], |r| r.get(0)).unwrap()
        };
        let present = |c: &Catalog, name: &str| -> bool {
            let missing: i64 = c
                .conn
                .query_row("SELECT missing FROM files WHERE name = ?1", [name.as_bytes().to_vec()], |r| r.get(0))
                .unwrap();
            missing == 0
        };
        let other = "jpg,xyz;mp4";
        let mut c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert_eq!(mark(&c), pg_model::media::held_exts(), "a new catalog's first start looks, and marks");
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let scan = begin(&mut c, &p(rules, r"D:\x"));
        for (n, name) in ["a.jpg", "b.jpg"].into_iter().enumerate() {
            // Each batch of the scan runs while the other build's set is the mark: its writer stamps this build's.
            c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'held_exts'", [other]).unwrap();
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
            let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
            let file = ScannedFile {
                identity: FileIdentity::new(1, 10 + u64::try_from(n).unwrap()),
                name: &Name::from(name),
                ext: Ext::of_name(name).unwrap(),
                size: 10,
                mtime: 100,
                created: None,
                prior: None,
            };
            writer.record_file(&folder, &file).unwrap();
            drop(writer);
            tx.commit().unwrap();
            assert_eq!(mark(&c), pg_model::media::held_exts(), "the scan's writer stamps the mark");
        }
        // The pairing after the scan stamps it too (its rebinds revive rows).
        c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'held_exts'", [other]).unwrap();
        let coverage = crate::pair::Coverage { complete: true, skipped: &[] };
        lease(&mut c, scan, &p(rules, r"D:\x")).pair_after_scan(coverage).unwrap();
        assert_eq!(mark(&c), pg_model::media::held_exts(), "the pairing after the scan stamps the mark");
        // The other build records a row of an extension this one doesn't hold, and stamps its set.
        c.conn.execute("UPDATE files SET ext = 'xyz' WHERE name = ?1", [b"b.jpg".to_vec()]).unwrap();
        c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'held_exts'", [other]).unwrap();
        drop(c);
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(!present(&c, "b.jpg") && present(&c, "a.jpg"), "another build's set: looked for, and marked missing");
        assert_eq!(mark(&c), pg_model::media::held_exts());
        // With this build's set as the mark, a start reads no file: a row it doesn't hold is left as it is.
        c.conn.execute("UPDATE files SET ext = 'xyz', missing = 0 WHERE name = ?1", [b"a.jpg".to_vec()]).unwrap();
        drop(c);
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(present(&c, "a.jpg"), "this build's set: not looked for");
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        the_unheld_extensions_are_looked_for_when_another_build_wrote,
        the_unheld_extensions_are_looked_for_when_another_build_wrote_windows,
        the_unheld_extensions_are_looked_for_when_another_build_wrote_posix
    );

    fn a_database_that_is_not_a_catalog_is_moved_aside_not_built_on(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("foreign-database-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE notes(x INTEGER); INSERT INTO notes VALUES (5);").unwrap();
        }
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let why = rebuilt_why(&c).expect("moved aside");
        assert!(why.contains("notes"), "{why}");
        let kept = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(kept.query_row("SELECT x FROM notes", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        drop(kept);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        a_database_that_is_not_a_catalog_is_moved_aside_not_built_on,
        a_database_that_is_not_a_catalog_is_moved_aside_not_built_on_windows,
        a_database_that_is_not_a_catalog_is_moved_aside_not_built_on_posix
    );

    fn a_file_that_is_not_a_database_is_moved_aside_unread(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("not-a-database-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        std::fs::write(&path, vec![b'x'; 8192]).unwrap();
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert_eq!(rebuilt_why(&c).as_deref(), Some("not a catalog"));
        assert_eq!(std::fs::read(dir.join("catalog.sqlite.replaced")).unwrap(), vec![b'x'; 8192]);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        a_file_that_is_not_a_database_is_moved_aside_unread,
        a_file_that_is_not_a_database_is_moved_aside_unread_windows,
        a_file_that_is_not_a_database_is_moved_aside_unread_posix
    );

    fn the_same_schema_under_another_application_id_is_moved_aside(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("application-id-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        raw::on_disk(&path).pragma_update(None, "application_id", 7).unwrap();
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert_eq!(rebuilt_why(&c).as_deref(), Some("application id 7, not a catalog's"));
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        the_same_schema_under_another_application_id_is_moved_aside,
        the_same_schema_under_another_application_id_is_moved_aside_windows,
        the_same_schema_under_another_application_id_is_moved_aside_posix
    );

    fn open_refuses_a_catalog_changed_after_it_was_prepared(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("changed-after-prepare-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        raw::on_disk(&path).execute_batch("CREATE TABLE extra(x INTEGER);").unwrap();
        let e = Catalog::open(&path, rules).err().expect("refused");
        assert!(e.to_string().contains("extra"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        open_refuses_a_catalog_changed_after_it_was_prepared,
        open_refuses_a_catalog_changed_after_it_was_prepared_windows,
        open_refuses_a_catalog_changed_after_it_was_prepared_posix
    );

    fn two_preparations_of_one_catalog_move_it_aside_once(rules: PathRules) {
        let dir = run_folder(SCRATCH, &format!("concurrent-prepare-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        std::fs::write(&path, vec![b'x'; 8192]).unwrap();
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || Catalog::prepare(&path, versions(), rules).map(|c| c.rebuilt().is_some()))
            })
            .collect();
        let moved: Vec<bool> = threads.into_iter().map(|t| t.join().unwrap().unwrap()).collect();
        assert_eq!(moved.iter().filter(|m| **m).count(), 1, "{moved:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    under_both_rules!(
        two_preparations_of_one_catalog_move_it_aside_once,
        two_preparations_of_one_catalog_move_it_aside_once_windows,
        two_preparations_of_one_catalog_move_it_aside_once_posix
    );

    fn a_memory_catalog_takes_no_lock_file(rules: PathRules) {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        assert!(c.rebuilt().is_none());
        assert!(!Path::new(":memory:.lock").exists());
    }

    under_both_rules!(
        a_memory_catalog_takes_no_lock_file,
        a_memory_catalog_takes_no_lock_file_windows,
        a_memory_catalog_takes_no_lock_file_posix
    );

    /// A fingerprint in the real format (`TAKEN_ZONE_QUERY`: four offsets in seconds) that no zone has: every offset is past
    /// the widest real one, 14 hours (50400 s).
    const NO_ZONE: &str = "-99999,-99999,-99999,-99999";

    fn a_changed_zone_computes_utc_taken_times_again_from_their_instants(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        let instant = 1_714_557_600;
        store_taken(&mut c, &item, Some(Taken::Utc(instant)));

        // A `taken` left from another zone, and a fingerprint no zone has (a UTC system's own is "0,0,0,0", so a
        // numeric stand-in could match it and hide the change): the next prepare recomputes it.
        c.conn.execute("UPDATE analysis SET taken = 0 WHERE file_id = 1", []).unwrap();
        c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'taken_zone'", [NO_ZONE]).unwrap();
        set_taken_zone(&c.conn).unwrap();
        assert_eq!(taken_columns(&c), (Some(local_of(&c, instant)), Some(instant)));
        let current: String = c.conn.query_row(TAKEN_ZONE_QUERY, [], |r| r.get(0)).unwrap();
        let stored: String =
            c.conn.query_row("SELECT value FROM meta WHERE key = 'taken_zone'", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, current);
        assert_eq!(current.split(',').count(), 4);
    }

    under_both_rules!(
        a_changed_zone_computes_utc_taken_times_again_from_their_instants,
        a_changed_zone_computes_utc_taken_times_again_from_their_instants_windows,
        a_changed_zone_computes_utc_taken_times_again_from_their_instants_posix
    );

    fn a_camera_clock_is_not_changed_by_a_changed_zone(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        store_taken(&mut c, &item, Some(Taken::Wall(1_700_000_000)));
        c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'taken_zone'", [NO_ZONE]).unwrap();
        set_taken_zone(&c.conn).unwrap();
        assert_eq!(taken_columns(&c), (Some(1_700_000_000), None));
    }

    under_both_rules!(
        a_camera_clock_is_not_changed_by_a_changed_zone,
        a_camera_clock_is_not_changed_by_a_changed_zone_windows,
        a_camera_clock_is_not_changed_by_a_changed_zone_posix
    );

    fn prepare_records_the_zone_its_utc_times_were_computed_with(rules: PathRules) {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let current: String = c.conn.query_row(TAKEN_ZONE_QUERY, [], |r| r.get(0)).unwrap();
        let stored: String =
            c.conn.query_row("SELECT value FROM meta WHERE key = 'taken_zone'", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, current);
    }

    under_both_rules!(
        prepare_records_the_zone_its_utc_times_were_computed_with,
        prepare_records_the_zone_its_utc_times_were_computed_with_windows,
        prepare_records_the_zone_its_utc_times_were_computed_with_posix
    );

    fn a_new_import_version_redoes_everything(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        analyzed(&mut c, &item);
        // As an older build left it: its import version, and analyses made by it.
        raw::older_import_version(&c);
        let (pending, seq) = state(&c, item.id);
        assert!(!pending, "current for that build");
        raw::start_import_version(&c);
        let (pending, after) = state(&c, item.id);
        assert!(pending && after > seq);
        // Done once.
        raw::start_import_version(&c);
        assert_eq!(raw::file_seq(&c, item.id), after);
    }

    under_both_rules!(
        a_new_import_version_redoes_everything,
        a_new_import_version_redoes_everything_windows,
        a_new_import_version_redoes_everything_posix
    );

    fn a_round_with_nothing_to_import_reads_one_index(rules: PathRules) {
        let (c, _) = one_file(rules);
        let plan = raw::query_plan(&c, &pending_import_sql(false, &VolumeSet::All), []);
        assert!(plan.contains("USING INDEX files_to_import") && !plan.contains("TEMP B-TREE"), "{plan}");
        let retry = raw::query_plan(&c, &pending_import_sql(true, &VolumeSet::All), []);
        assert!(retry.contains("files_to_import") && retry.contains("analysis_failed"), "{retry}");
    }

    under_both_rules!(
        a_round_with_nothing_to_import_reads_one_index,
        a_round_with_nothing_to_import_reads_one_index_windows,
        a_round_with_nothing_to_import_reads_one_index_posix
    );

    /// A `VACUUM` that can't run (another connection reads) fails nothing: the mark stays, and the next start runs it.
    #[test]
    fn a_vacuum_that_fails_leaves_the_mark_for_the_next_start() {
        let rules = pg_platform::native::PATH_RULES;
        let dir = run_folder(SCRATCH, "vacuum-fails").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        let conn = raw::on_disk(&path);
        conn.execute_batch("PRAGMA busy_timeout = 0; UPDATE meta SET value = 1 WHERE key = 'vacuum_owed';").unwrap();
        let reader = raw::on_disk(&path);
        reader.execute_batch("BEGIN; SELECT count(*) FROM files;").unwrap();
        // A write lock the VACUUM can't get while the reader's snapshot is open.
        conn.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let other = raw::on_disk(&path);
        other.execute_batch("PRAGMA busy_timeout = 0;").unwrap();
        super::vacuum_if_owed(&other, Some(&path), true).unwrap();
        let owed: i64 = other.query_row("SELECT value FROM meta WHERE key = 'vacuum_owed'", [], |r| r.get(0)).unwrap();
        assert_eq!(owed, 1, "kept for the next start");
        conn.execute_batch("COMMIT;").unwrap();
        reader.execute_batch("COMMIT;").unwrap();
        drop((conn, reader));
        super::vacuum_if_owed(&other, Some(&path), true).unwrap();
        let owed: i64 = other.query_row("SELECT value FROM meta WHERE key = 'vacuum_owed'", [], |r| r.get(0)).unwrap();
        assert_eq!(owed, 0, "done once nothing holds the file");
        drop(other);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
