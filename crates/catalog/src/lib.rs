//! The catalog (PLAN §6): SQLite in WAL mode, the source of truth for files, analyses, pairs and ratings.
//!
//! File identity is (volume serial, NTFS file ID), so moves and renames need no re-analysis (PLAN §4). An analysis
//! row records what it was computed from: the size, mtime, catalog rotation and kind, and the pipeline version.
//! When any differ, the file is pending again (`files.pending`, kept by triggers). That is all the resumability
//! there is: every batch is one transaction, and the work list is an index.
//!
//! The thumbnails' work is tracked the same way: whatever a thumbnail depends on changing gives the file the next
//! number of a change sequence (`files.seq`, from `meta.seq`), so the thumbnail pass looks only at files changed
//! since it last looked (`thumbs::pending_in`).
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod browse;
pub mod dups;
#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub use pg_query as query;
pub mod writes;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, anyhow};

mod error;
pub use error::{Error, Result};
use rusqlite::types::Type;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Statement, Transaction, TransactionBehavior, params};

use pg_model::analysis::Analyzed;
use pg_model::detection::Detection;
use pg_model::format::Format;
use pg_model::ids::{FileId, FileIdentity, FolderId, RootId};
use pg_model::media::{Ext, Kind};
use pg_model::name::Name;
use pg_model::orientation::Orientation;
use pg_model::path::{FilePath, PathArena, PathRef, WorkList};
use pg_model::rating::Rating;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_model::turns::Turns;

// SQL used in more than one statement is a macro, so every statement is a constant (`concat!`), none made at run
// time.

/// Whether the `files` row being written needs its analysis (re)done: no analysis made from it as it is now (at
/// `meta.import_version` or later, from the same size, mtime, catalog rotation and kind). A failure counts as made:
/// `--retry` redoes those. Inside `UPDATE files … WHERE id = …`.
macro_rules! stale {
    () => {
        "NOT EXISTS (SELECT 1 FROM analysis a WHERE a.file_id = files.id
        AND a.version >= (SELECT value FROM meta WHERE key = 'import_version')
        AND a.size = files.size AND a.mtime = files.mtime AND a.rotation = files.rotation AND a.kind = files.kind)"
    };
}

/// The next number of the change sequence, for `files.seq`: `next_seq!()` first, then `seq = seq!()`.
macro_rules! next_seq {
    () => {
        "UPDATE meta SET value = value + 1 WHERE key = 'seq'"
    };
}
macro_rules! seq {
    () => {
        "(SELECT value FROM meta WHERE key = 'seq')"
    };
}

/// The analyses borders are looked for in: still images that decoded and are big enough (detection::MIN_SIDE).
macro_rules! has_borders {
    () => {
        concat!("error IS NULL AND kind = 'image' AND frames <= 1 AND min(width, height) >= ", pg_model::min_side!())
    };
}

/// Folders equal to the root (?1) or below it: the paths from ?2 (the root and a separator) up to ?3 (the root and
/// the separator's successor, so nothing sorts past the subtree). The bounds are made by [`under_params`].
macro_rules! under_root {
    () => {
        "SELECT id FROM folders WHERE path = ?1 OR (path >= ?2 AND path < ?3)"
    };
}

/// The data a row can hold that a pairing would lose (S-17.7 decision 4): a rotation, a rating, the file's own rating,
/// an analysis or a queued write. Inside a statement over `files`.
macro_rules! has_children {
    () => {
        "(files.rotation <> 0 OR EXISTS (SELECT 1 FROM ratings WHERE file_id = files.id)
          OR EXISTS (SELECT 1 FROM file_ratings WHERE file_id = files.id)
          OR EXISTS (SELECT 1 FROM analysis WHERE file_id = files.id)
          OR EXISTS (SELECT 1 FROM file_writes WHERE file_id = files.id))"
    };
}

/// The parameters of [`under_root`] for `root` under `rules`: the root's bytes, and the bounds of its subtree. A
/// drive's root already ends with a separator, so its subtree starts there.
fn under_params(rules: PathRules, root: &pg_model::rules::Path) -> [Vec<u8>; 3] {
    let sep = rules.sep_byte();
    let bytes = rules.join(root);
    let (lo, hi) = match bytes.split_last() {
        Some((&last, init)) if last == sep => {
            let mut hi = init.to_vec();
            hi.push(sep + 1);
            (bytes.clone(), hi)
        }
        _ => {
            let mut lo = bytes.clone();
            lo.push(sep);
            let mut hi = bytes.clone();
            hi.push(sep + 1);
            (lo, hi)
        }
    };
    [bytes, lo, hi]
}

mod pair;
pub use pair::Pairing;
mod schema;
use schema::{
    BASELINE, MIGRATIONS, SETTINGS, bring_to, carry_rows, current_expected, current_version, expected, mismatch,
    version_of,
};

/// A library folder: the user added it, and a rescan walks it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub id: RootId,
    /// Absolute, with no trailing separator.
    pub path: RulePath,
}

pub struct Catalog {
    conn: Connection,
    /// The path rules the catalog's paths are stored under (`meta.path_rules`, checked when it opens).
    rules: PathRules,
    /// The catalog this build moved aside to make this one, when [`Catalog::prepare`] made one.
    rebuilt: Option<Rebuilt>,
}

/// A catalog that [`Catalog::prepare`] moved aside, and why: its schema differed from this build's, or it was not a
/// database. The new catalog is made from this build's schema and takes the old one's rows ([`Carried`]), so the
/// library stays as it was; the old file is kept beside it, whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rebuilt {
    /// Where the old catalog is now (beside the new one, with its `-wal`, `-shm` and `-journal` moved with it).
    pub kept: PathBuf,
    /// The first difference from this build's schema, as [`pg_schema::Difference`] words it.
    pub why: String,
    /// What the new catalog took from the old one.
    pub carried: Carried,
}

/// The rows a rebuilt catalog took from the old one, with their ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Carried {
    /// The old catalog's rows, copied as they were: its files and its library folders (`roots`).
    Rows { files: i64, folders: i64 },
    /// No rows were copied (the old catalog was not one this build can read, or the copy failed): the reason. The new
    /// catalog is empty, and the next scan fills it.
    Nothing(String),
}

impl Rebuilt {
    /// What was carried, for a note. It starts with `"; "`, so it follows the note's text.
    pub fn carried_note(&self) -> String {
        let count = |n: i64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        match &self.carried {
            Carried::Rows { files: 0, folders: 0 } => String::new(),
            Carried::Rows { files, folders } => format!(
                "; {} and {} carried over",
                count(*files, "file", "files"),
                count(*folders, "library folder", "library folders")
            ),
            Carried::Nothing(reason) => format!("; nothing carried over ({reason}): scan the library again"),
        }
    }
}

/// The `meta.path_rules` value of a rule set.
pub(crate) fn rules_name(rules: PathRules) -> &'static str {
    match rules {
        PathRules::Windows => "windows",
        PathRules::Posix => "posix",
    }
}

/// Whether the rows of the catalog `old` may be carried into a catalog under `rules`: they were stored under those rules
/// (`meta.path_rules`). Paths stored under other rules don't read as paths under these, so none of them is carried; the
/// reason says why, for the note.
fn stored_under(old: &Connection, rules: PathRules) -> std::result::Result<(), String> {
    let recorded: Option<String> = old
        .query_row("SELECT value FROM meta WHERE key = 'path_rules'", [], |r| r.get(0))
        .optional()
        .map_err(|e| format!("its path rules can't be read: {e}"))?;
    match recorded.as_deref() {
        Some(name) if name == rules_name(rules) => Ok(()),
        Some(name) => Err(format!("made with the {name} path rules")),
        None => Err("made before its path rules were recorded".to_string()),
    }
}

/// `path` with `suffix` added to its name: the catalog's `-wal` and so on, or a file beside it.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Where the replacement for the catalog at `path` is made before it takes the name ([`rebuild`]).
fn building_path(path: &Path) -> PathBuf {
    beside(path, ".building")
}

/// Make the catalog `old` (at `path`) into a new one in this build's layout, carrying its rows ([`rebuild`]): the old file
/// is moved aside, and the new one takes its name. `old` is the connection the change was found on; it is closed here.
fn rebuild(old: Connection, path: &Path, rules: PathRules, why: String, readable: bool) -> Result<Rebuilt> {
    let source = if readable {
        // The changes still in the old file's `-wal` are checkpointed into it, so the copy reads all of them.
        old.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        stored_under(&old, rules)
    } else {
        Err("the old file holds no catalog".to_string())
    };
    drop(old);
    let replacement = building_path(path);
    remove_database(&replacement)?;
    let carried = match source {
        Ok(()) => match make_replacement(&replacement, rules, Some(path)) {
            Ok((files, folders)) => Carried::Rows { files, folders },
            Err(e) => {
                // The copy is one transaction, so an error leaves no rows; the replacement is made again without them.
                remove_database(&replacement)?;
                make_replacement(&replacement, rules, None)?;
                Carried::Nothing(format!("{e:#}"))
            }
        },
        Err(reason) => {
            make_replacement(&replacement, rules, None)?;
            Carried::Nothing(reason)
        }
    };
    let kept = move_aside(path).with_context(|| format!("catalog {}: moving it aside ({why})", path.display()))?;
    std::fs::rename(&replacement, path).with_context(|| {
        format!("catalog {}: its new file {} can't take the name", path.display(), replacement.display())
    })?;
    Ok(Rebuilt { kept, why, carried })
}

/// Make the catalog file `path` in this build's layout, with the rows of `from` copied into it when given
/// ([`carry_rows`]). Returns its file and library folder counts (zero without `from`). The file is closed, with its
/// changes in the main file.
fn make_replacement(path: &Path, rules: PathRules, from: Option<&Path>) -> Result<(i64, i64)> {
    let mut conn = Connection::open(path).with_context(|| format!("catalog {}", path.display()))?;
    conn.execute_batch(SETTINGS)?;
    bring_to(&conn, BASELINE, MIGRATIONS, rules, current_version()?)
        .with_context(|| format!("catalog {}", path.display()))?;
    let counts = match from {
        Some(old) => {
            carry_rows(&mut conn, old)?;
            let count = |table: &str| -> rusqlite::Result<i64> {
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            };
            (count("files")?, count("roots")?)
        }
        None => (0, 0),
    };
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    conn.close().map_err(|(_, e)| e)?;
    Ok(counts)
}

/// Delete the database at `path`, with its `-wal`, `-shm` and `-journal`, if they are there. Only ever called on a file
/// this build made (a replacement left by an interrupted rebuild), never on the catalog itself.
fn remove_database(path: &Path) -> Result<()> {
    for part in ["-wal", "-shm", "-journal", ""] {
        let file = beside(path, part);
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::Other(anyhow!("can't remove {}: {e}", file.display()))),
        }
    }
    Ok(())
}

/// Refuse a catalog whose paths were stored under other rules than these (`meta.path_rules`): its bytes don't read as
/// paths under these. [`Catalog::prepare`] has already made sure the schema is this build's, which includes the rules
/// row, so a missing row can only be refused here, as a file this build can't read.
fn check_rules(conn: &Connection, path: &Path, rules: PathRules) -> Result<()> {
    let found: Option<String> =
        conn.query_row("SELECT value FROM meta WHERE key = 'path_rules'", [], |r| r.get(0)).optional()?;
    if found.as_deref() == Some(rules_name(rules)) {
        return Ok(());
    }
    let detail = match &found {
        Some(f) => format!("made with the {f} path rules"),
        None => "made before its path rules were recorded".to_string(),
    };
    Err(Error::PathRules { path: path.display().to_string(), detail, wanted: rules_name(rules) })
}

/// The lock [`Catalog::prepare`] holds on the catalog (`pg_schema::Lock`): two processes preparing one catalog do it one
/// after the other, and the second finds the first's catalog current. It is not taken for `:memory:`.
fn take_lock(path: &Path) -> Result<pg_schema::Lock> {
    pg_schema::Lock::take(path)
        .with_context(|| format!("catalog {}: its lock file", path.display()))?
        .ok_or_else(|| Error::Other(anyhow!("catalog {}: another process is preparing it", path.display())))
}

/// Move the catalog at `path`, with its `-wal`, `-shm` and `-journal` files, aside to the first free name beside it:
/// `catalog.sqlite.replaced`, then `.replaced.1`, and so on. Returns the main file's new path. Nothing is deleted: the
/// user removes the moved files when they are no longer wanted. The sidecars move first, so the main file is never
/// left at `path` without the changes its sidecars held. No connection to the catalog may be open.
fn move_aside(path: &Path) -> Result<PathBuf> {
    const PARTS: [&str; 4] = ["-wal", "-shm", "-journal", ""];
    let mut n = 0u32;
    let base = loop {
        let base = if n == 0 { ".replaced".to_string() } else { format!(".replaced.{n}") };
        if PARTS.iter().all(|part| !beside(path, &format!("{base}{part}")).exists()) {
            break base;
        }
        n += 1;
    };
    for part in PARTS {
        let from = beside(path, part);
        if from.exists() {
            let to = beside(path, &format!("{base}{part}"));
            std::fs::rename(&from, &to).with_context(|| format!("can't move {} aside", from.display()))?;
        }
    }
    Ok(beside(path, &base))
}

/// The versions of the analyses the build makes, which `files.pending` and `analysis.borders_due` are kept against
/// (`analyze::IMPORT_VERSION`, `borders::BORDERS_VERSION`). The library passes them to [`Catalog::prepare`]: the
/// catalog doesn't depend on the analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Versions {
    pub import: i64,
    pub borders: i64,
}

/// A transaction that writes: it takes the write lock when it begins, so a lock held by another connection is waited
/// for (`busy_timeout`) before anything is read. A deferred one that reads first and then writes can instead fail at
/// once when another connection has committed in between (SQLITE_BUSY_SNAPSHOT), which no wait fixes.
pub(crate) fn write_transaction(conn: &Connection) -> rusqlite::Result<Transaction<'_>> {
    Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
}

/// Make `files.pending` mean this build's IMPORT_VERSION: when the catalog's `meta.import_version` is another
/// (a new catalog, or a build that analyzes differently), every file's is computed again, and every file's `seq`
/// moves on, so the thumbnail pass looks at all of them once. Checked and set in one write transaction, so two
/// processes opening the catalog at once do it once.
fn set_import_version(conn: &Connection, current: i64) -> Result<()> {
    let version = |c: &Connection| -> Result<i64> {
        Ok(c.query_row("SELECT value FROM meta WHERE key = 'import_version'", [], |r| r.get(0))?)
    };
    if version(conn)? == current {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if version(&tx)? != current {
        tx.execute("UPDATE meta SET value = ?1 WHERE key = 'import_version'", [current])?;
        tx.execute_batch(concat!(
            "UPDATE files SET pending = ",
            stale!(),
            "; ",
            next_seq!(),
            "; UPDATE files SET seq = ",
            seq!(),
            ";"
        ))?;
    }
    Ok(tx.commit()?)
}

/// Make `analysis.borders_due` mean this build's detector: when the catalog's `meta.borders_version` is another,
/// every analysis that can have borders is due. Like [`set_import_version`], in one write transaction.
fn set_borders_version(conn: &Connection, current: i64) -> Result<()> {
    let version = |c: &Connection| -> Result<i64> {
        Ok(c.query_row("SELECT value FROM meta WHERE key = 'borders_version'", [], |r| r.get(0))?)
    };
    if version(conn)? == current {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if version(&tx)? != current {
        tx.execute("UPDATE meta SET value = ?1 WHERE key = 'borders_version'", [current])?;
        tx.execute_batch(concat!("UPDATE analysis SET borders_due = coalesce(", has_borders!(), ", 0)"))?;
    }
    Ok(tx.commit()?)
}

/// The local zone's UTC offsets, in seconds, on January 1 and July 1 of 2000 and of 2030: the zone's fingerprint, as
/// `a,b,c,d`. A UTC-sourced `analysis.taken` depends on the zone only through these offsets, so a change to any of
/// them is a change of zone (or of its daylight-saving rules as they apply to these dates).
const TAKEN_ZONE_QUERY: &str = "SELECT (unixepoch('2000-01-01', 'localtime') - unixepoch('2000-01-01')) || ',' || \
    (unixepoch('2000-07-01', 'localtime') - unixepoch('2000-07-01')) || ',' || \
    (unixepoch('2030-01-01', 'localtime') - unixepoch('2030-01-01')) || ',' || \
    (unixepoch('2030-07-01', 'localtime') - unixepoch('2030-07-01'))";

/// Make `analysis.taken` follow this system's zone: when `meta.taken_zone` is another fingerprint (the zone changed,
/// or the catalog was made under another), every UTC-sourced `taken` is computed again from its `taken_utc` instant.
/// Like [`set_import_version`], in one write transaction. The stored instants are what make this possible: the wall
/// clock alone would lose the instant of a repeated hour.
fn set_taken_zone(conn: &Connection) -> Result<()> {
    let stored = |c: &Connection| -> Result<String> {
        Ok(c.query_row("SELECT value FROM meta WHERE key = 'taken_zone'", [], |r| r.get(0))?)
    };
    let current: String = conn.query_row(TAKEN_ZONE_QUERY, [], |r| r.get(0))?;
    if stored(conn)? == current {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if stored(&tx)? != current {
        tx.execute("UPDATE meta SET value = ?1 WHERE key = 'taken_zone'", [&current])?;
        tx.execute_batch(
            "UPDATE analysis SET taken = unixepoch(taken_utc, 'unixepoch', 'localtime') WHERE taken_utc IS NOT NULL",
        )?;
    }
    Ok(tx.commit()?)
}

/// What a [`WorkItem`] is read from, with `f` the file: its folder's id and its name, which [`work_list`] makes
/// its path from.
macro_rules! work_columns {
    () => {
        "f.id, f.folder_id, f.name, f.size, f.mtime, f.rotation, f.kind"
    };
}

/// `Catalog::pending_import`'s query, with the volume and the record number for its order.
macro_rules! pending_files {
    () => {
        // `missing = 0 AND pending = 1` written out: the index's condition.
        concat!(
            "SELECT ",
            work_columns!(),
            ", f.volume AS volume, f.file_ref & 0xFFFFFFFFFFFF AS record
             FROM files f WHERE f.missing = 0 AND f.pending = 1"
        )
    };
}

/// `Catalog::pending_import`'s query.
fn pending_import_sql(retry_errors: bool) -> &'static str {
    if retry_errors {
        // A UNION, as an OR would scan every file; the failures are read from their index first.
        concat!(
            pending_files!(),
            " UNION SELECT ",
            work_columns!(),
            ", f.volume AS volume, f.file_ref & 0xFFFFFFFFFFFF AS record
             FROM analysis a CROSS JOIN files f
             WHERE a.error IS NOT NULL AND f.id = a.file_id AND f.missing = 0 ORDER BY volume, record"
        )
    } else {
        concat!(pending_files!(), " ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF")
    }
}

/// The work list of a statement whose rows start with `work_columns!()` and number at most `bound`, read in
/// `conn`'s transaction: the items in a `Vec` of that capacity, each folder's path read once.
fn work_list(conn: &Connection, stmt: &mut rusqlite::Statement, bound: usize) -> Result<WorkList<WorkItem>> {
    let mut rows = stmt.query([])?;
    let (mut list, mut folder) = (WorkList::default(), None);
    while let Some(r) = rows.next()? {
        if folder.is_none() {
            // Only once there is work: a round with nothing to do reads nothing more.
            list = WorkList::with_capacity(bound, max_folder_id(conn, "")?);
            folder = Some(conn.prepare_cached(FOLDER_PATH)?);
        }
        let Some(folder) = folder.as_mut() else { unreachable!("made above") };
        let path = list.file(r.get(1)?, r.get_ref(2)?.as_blob()?, |arena| folder_path(folder, r, arena))?;
        list.items.push(WorkItem {
            id: FileId::new(r.get(0)?),
            path,
            size: r.get(3)?,
            mtime: r.get(4)?,
            rotation: Turns::new(r.get(5)?),
            kind: Kind::from_key(r.get_ref(6)?.as_str()?).unwrap_or(Kind::Image),
        });
    }
    Ok(list)
}

/// A folder's path by its id.
const FOLDER_PATH: &str = "SELECT path FROM folders WHERE id = ?1";

/// The largest folder id of the catalog (`schema`: `""`, or `"cat."` where it is attached under that name): an
/// O(1) read of the table's last row.
pub fn max_folder_id(conn: &Connection, schema: &str) -> Result<i64> {
    let sql = format!("SELECT coalesce(max(id), -1) FROM {schema}folders");
    Ok(conn.query_row(&sql, [], |r| r.get(0))?)
}

/// The path of the folder of row `r` (its column 1), read with `folder` ([`FOLDER_PATH`]) into `arena`.
pub(crate) fn folder_path(
    folder: &mut rusqlite::Statement,
    r: &rusqlite::Row,
    arena: &mut PathArena,
) -> rusqlite::Result<PathRef> {
    let id: i64 = r.get(1)?;
    folder.query_row([id], |f| Ok(arena.push(f.get_ref(0)?.as_blob()?)))
}

/// A file as the scanner saw it.
pub struct ScannedFile<'a> {
    pub identity: FileIdentity,
    pub name: &'a Name,
    /// Its last extension (`Ext::of_name`), which also says its kind.
    pub ext: Ext,
    pub size: u64,
    pub mtime: i64,
    /// Its creation time (FILETIME ticks), where the listing gives one (S-17.7 decision 2).
    pub created: Option<i64>,
}

/// A folder's row, with its part of its files' order key (`path::folder_key`).
pub struct Folder {
    pub id: FolderId,
    pub key: Vec<u8>,
}

/// A file the import (or the borders pass) still has to analyze.
#[derive(Clone, Copy, Debug)]
pub struct WorkItem {
    pub id: FileId,
    /// In its list.
    pub path: FilePath,
    pub size: i64,
    pub mtime: i64,
    /// Catalog rotation (`files.rotation`), applied when decoding.
    pub rotation: Turns,
    pub kind: Kind,
}

/// A file with borders to crop off ([`Catalog::crop_source`]).
pub struct CropSource {
    /// In the arena `crop_source` was given.
    pub path: PathRef,
    /// The file as analyzed.
    pub size: i64,
    pub mtime: i64,
    pub format: Format,
    /// The stored size.
    pub width: u32,
    pub height: u32,
    /// Left, top, right, bottom, stored pixels.
    pub cut: [u32; 4],
}

pub struct Status {
    pub files: i64,
    pub missing: i64,
    pub analyzed: i64,
    pub failed: i64,
    pub pending: i64,
    pub pdq_pairs: i64,
    /// Ratings and turns not yet written into the files, and those that keep failing.
    pub queued_writes: i64,
    pub failed_writes: i64,
}

impl Catalog {
    /// Merge the WAL into the file, then close the connection (on drop, too, without the merge). The last connection
    /// to a file to close deletes its WAL; the merge keeps the file whole if the process ends before the others close.
    pub fn close(self) -> Result<()> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    /// Prepare the catalog at `path` for this build, creating it when new. A file that is not this build's catalog is
    /// judged before anything is written to it: its schema (every object, and the keys its creation seeds) must be the
    /// one this build makes for its version. One that isn't (an earlier layout, another build's, or not a database at
    /// all) is rebuilt: a new catalog in this build's layout is made beside it, the old rows are copied into it with
    /// their ids (`carry_rows`), and the old file is moved aside (`move_aside`; see [`Catalog::rebuilt`]). A row the
    /// copy can't take (a column the new layout needs that the old one lacks) fails the copy, and the new catalog is
    /// made empty, with the reason said. A catalog of this build's schema at an earlier version is migrated forward.
    /// Its version marks are set to `versions`. The library calls this once, before anything else opens the catalog.
    pub fn prepare(path: &Path, versions: Versions, rules: PathRules) -> Result<Catalog> {
        let _lock = if path == Path::new(":memory:") { None } else { Some(take_lock(path)?) };
        let current = current_version()?;
        let mut conn = Connection::open(path).with_context(|| format!("catalog {}", path.display()))?;
        let version = version_of(&conn)?;
        let why = match version {
            None => Some("not a database".to_string()),
            Some(found) if found > current => {
                return Err(Error::Other(anyhow!(
                    "catalog {}: schema version {found}, which this build doesn't know (it has {current}): made by \
                     another build",
                    path.display()
                )));
            }
            Some(found) => mismatch(&conn, found, &expected(found)?)?,
        };
        let mut rebuilt = None;
        if let Some(why) = why {
            rebuilt = Some(rebuild(conn, path, rules, why, version.is_some())?);
            conn = Connection::open(path).with_context(|| format!("catalog {}", path.display()))?;
        }
        conn.execute_batch(SETTINGS)?;
        bring_to(&conn, BASELINE, MIGRATIONS, rules, current).with_context(|| format!("catalog {}", path.display()))?;
        if let Some(why) = mismatch(&conn, current, current_expected()?)? {
            return Err(Error::Other(anyhow!(
                "catalog {}: the catalog made is not this build's: {why}",
                path.display()
            )));
        }
        check_rules(&conn, path, rules)?;
        set_import_version(&conn, versions.import).with_context(|| format!("catalog {}", path.display()))?;
        set_borders_version(&conn, versions.borders).with_context(|| format!("catalog {}", path.display()))?;
        set_taken_zone(&conn).with_context(|| format!("catalog {}", path.display()))?;
        crate::query::register_functions(&conn)?;
        Ok(Catalog { conn, rules, rebuilt })
    }

    /// Open a catalog this process has prepared ([`Catalog::prepare`], which makes it, brings it to this build's schema
    /// and sets the build's version marks): the settings and the SQL functions. Every connection to it (the app's
    /// threads, the background job, the writer) runs this, so the check is the schema's itself, against the cached
    /// expected schema, and is not repeated for the marks. An error if the catalog isn't there, or its schema isn't this
    /// build's (another program changed it since it was prepared).
    pub fn open(path: &Path, rules: PathRules) -> Result<Catalog> {
        let flags = OpenFlags::default() - OpenFlags::SQLITE_OPEN_CREATE;
        let conn = Connection::open_with_flags(path, flags).with_context(|| format!("catalog {}", path.display()))?;
        conn.execute_batch(SETTINGS)?;
        let current = current_version()?;
        let version = pg_schema::user_version(&conn)?;
        if version != current {
            return Err(Error::Other(anyhow!(
                "catalog {}: schema version {version}, not this build's {current}",
                path.display()
            )));
        }
        if let Some(why) = mismatch(&conn, version, current_expected()?)? {
            return Err(Error::Other(anyhow!("catalog {}: {why}: it changed after it was prepared", path.display())));
        }
        check_rules(&conn, path, rules)?;
        crate::query::register_functions(&conn)?;
        Ok(Catalog { conn, rules, rebuilt: None })
    }

    /// The path rules the catalog's paths are stored under.
    pub fn rules(&self) -> PathRules {
        self.rules
    }

    /// The catalog this build moved aside to make this one, when [`Catalog::prepare`] did; `None` when the catalog was
    /// kept (or opened with [`Catalog::open`]).
    pub fn rebuilt(&self) -> Option<&Rebuilt> {
        self.rebuilt.as_ref()
    }

    /// Whether the file can't be written: it is missing from the catalog, or no longer listed.
    pub fn file_gone(&self, file_id: FileId) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT coalesce((SELECT missing FROM files WHERE id = ?1), 1)",
            [file_id.get()],
            |r| r.get(0),
        )?)
    }

    /// The folder the file is in, as the catalog has it; `None` if there is no such file. A failed query is an error.
    pub fn file_folder(&self, file_id: FileId) -> Result<Option<RulePath>> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
                [file_id.get()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(bytes.and_then(|bytes| self.rules.parse_bytes(&bytes)))
    }

    /// What a queued write needs of its file: its path, its analyzed format, and its kind (`files.kind`).
    pub fn write_target(&self, file_id: FileId) -> Result<(RulePath, Option<String>, String)> {
        let (folder, name, format, kind): (Vec<u8>, Vec<u8>, Option<String>, String) = self.conn.query_row(
            "SELECT fo.path, f.name, a.format, f.kind FROM files f JOIN folders fo ON fo.id = f.folder_id
             LEFT JOIN analysis a ON a.file_id = f.id WHERE f.id = ?1",
            [file_id.get()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        Ok((self.file_path_of(&folder, &name)?, format, kind))
    }

    /// A file's path from its folder's stored bytes and its name's: the folder, the separator, the name.
    fn file_path_of(&self, folder: &[u8], name: &[u8]) -> Result<RulePath> {
        let mut parts = self.rules.parse_bytes(folder).ok_or_else(|| anyhow!("a stored folder path has a NUL"))?;
        parts.parts.push(Name::from_raw(name));
        Ok(parts)
    }

    /// The id made with the catalog (`meta.catalog_id`).
    pub fn catalog_id(&self) -> Result<String> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = 'catalog_id'", [], |r| r.get(0))?)
    }

    /// Stop the statements on this connection while `cancel` is set (from any thread): each fails with an error for
    /// which [`Error::Interrupted`] is returned, including those started after it was set, until it is cleared. Unlike
    /// SQLite's interrupt, which only reaches the statements running at that moment, it can't fall between two
    /// statements of a read.
    pub fn cancel_when(&mut self, cancel: Arc<AtomicBool>) -> Result<()> {
        // Checked every 1000 virtual-machine steps: microseconds.
        Ok(self.conn.progress_handler(1000, Some(move || cancel.load(Ordering::Relaxed)))?)
    }

    pub fn begin_scan(&self, root: &RulePath) -> Result<i64> {
        self.conn.execute("INSERT INTO scans(root, started) VALUES (?1, unixepoch())", [self.rules.join(root)])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_scan(&self, scan: i64) -> Result<()> {
        self.conn.execute("UPDATE scans SET finished = unixepoch() WHERE id = ?1", [scan])?;
        Ok(())
    }

    /// Mark files under `root` that this scan didn't see as missing. Their rows stay (ratings, decisions). Files in
    /// `unreadable` folders (and their subfolders) are left as they are: the scan couldn't list them, so it saw
    /// nothing there to compare with.
    pub fn mark_missing(&self, root: &RulePath, scan: i64, unreadable: &[RulePath]) -> Result<usize> {
        // One transaction: all of the folders are marked, or none, and the commit is one for the whole pass.
        let tx = write_transaction(&self.conn)?;
        let under = |path: &RulePath| -> Result<Vec<i64>> {
            let mut stmt = tx.prepare(under_root!())?;
            let [p1, p2, p3] = under_params(self.rules, path);
            Ok(stmt.query_map(params![p1, p2, p3], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
        };
        let mut skipped = std::collections::BTreeSet::new();
        for path in unreadable {
            skipped.extend(under(path)?);
        }
        let mut marked = 0;
        {
            let mut mark =
                tx.prepare("UPDATE files SET missing = 1 WHERE folder_id = ?1 AND missing = 0 AND last_scan < ?2")?;
            for folder in under(root)? {
                if !skipped.contains(&folder) {
                    marked += mark.execute(params![folder, scan])?;
                }
            }
        }
        tx.commit()?;
        Ok(marked)
    }

    /// The library folders, by path, with their ids.
    pub fn roots(&self) -> Result<Vec<Root>> {
        self.roots_on(&self.conn)
    }

    /// [`Catalog::roots`] on `conn`, which may be a transaction's.
    fn roots_on(&self, conn: &Connection) -> Result<Vec<Root>> {
        let mut stmt = conn.prepare("SELECT id, path FROM roots ORDER BY path")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        let mut roots = Vec::new();
        for row in rows {
            let (id, bytes) = row?;
            let path = self.rules.parse_bytes(&bytes).ok_or_else(|| anyhow!("a stored library path has a NUL"))?;
            roots.push(Root { id: RootId::new(id), path });
        }
        Ok(roots)
    }

    /// The id of a library folder, by its path; `None` when the path is not one.
    pub fn root_id(&self, path: &RulePath) -> Result<Option<RootId>> {
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM roots WHERE path = ?1", [self.rules.join(path)], |r| r.get(0))
            .optional()?;
        Ok(id.map(RootId::new))
    }

    /// Add a folder to the library. A folder already inside the library adds nothing (returns false); one that
    /// contains library folders replaces them.
    pub fn add_root(&self, root: &RulePath) -> Result<bool> {
        // One transaction: the roots it replaces go only with the insert that replaces them (a crash between the two
        // would lose them), and the check reads what the replacement will see.
        let tx = write_transaction(&self.conn)?;
        let roots = self.roots_on(&tx)?;
        if roots.iter().any(|r| self.rules.is_under(root, &r.path)) {
            return Ok(false);
        }
        for r in roots.iter().filter(|r| self.rules.is_under(&r.path, root)) {
            tx.execute("DELETE FROM roots WHERE id = ?1", [r.id.get()])?;
        }
        tx.execute("INSERT INTO roots(path, added) VALUES (?1, unixepoch())", [self.rules.join(root)])?;
        tx.commit()?;
        Ok(true)
    }

    /// Take a folder out of the library. Its files are marked missing, not deleted: adding it back (or the drive
    /// returning) brings them back with their analysis and ratings. Returns how many files it hid; `None`, with
    /// nothing changed, when `root` is not a library folder.
    pub fn remove_root(&self, root: RootId) -> Result<Option<usize>> {
        let tx = write_transaction(&self.conn)?;
        let path: Option<Vec<u8>> =
            tx.query_row("SELECT path FROM roots WHERE id = ?1", [root.get()], |r| r.get(0)).optional()?;
        let Some(path) = path else { return Ok(None) };
        let path = self.rules.parse_bytes(&path).ok_or_else(|| anyhow!("a stored library path has a NUL"))?;
        tx.execute("DELETE FROM roots WHERE id = ?1", [root.get()])?;
        let [p1, p2, p3] = under_params(self.rules, &path);
        let n = tx.execute(
            concat!("UPDATE files SET missing = 1 WHERE missing = 0 AND folder_id IN (", under_root!(), ")"),
            params![p1, p2, p3],
        )?;
        tx.commit()?;
        Ok(Some(n))
    }

    /// Files whose analysis is absent or stale (`files.pending`), and with `retry_errors` those that failed, in
    /// on-disk order (volume, then MFT record number), which reads ~3× faster than path order on an HDD (FINDINGS
    /// "I/O"). Without `retry_errors` it's read in that order from `files_to_import`, so nothing pending is one probe.
    pub fn pending_import(&self, retry_errors: bool) -> Result<WorkList<WorkItem>> {
        // Counted in the same snapshot: an upper bound (a failure may also be pending), each count read from its
        // index alone.
        let tx = self.conn.unchecked_transaction()?;
        let count = |sql: &str| -> Result<usize> { Ok(tx.query_row(sql, [], |r| r.get(0))?) };
        let mut bound = count("SELECT count(*) FROM files WHERE missing = 0 AND pending = 1")?;
        if retry_errors {
            bound += count("SELECT count(*) FROM analysis WHERE error IS NOT NULL")?;
        }
        let list = work_list(&tx, &mut tx.prepare(pending_import_sql(retry_errors))?, bound)?;
        tx.commit()?;
        Ok(list)
    }

    /// The reliable PDQ hashes of current analyses (PLAN §6), as ids and hash words: `ids[i]`'s hash is `hashes[i]`, and
    /// a file contributes its 8 orientations. The tier-1 index (`pg_dedup::pdq_index::PdqIndex::build`) is made from them for each import.
    pub fn pdq_hashes(&self) -> Result<(Vec<FileId>, Vec<[u64; 4]>)> {
        // Sized by the present files (an upper bound, counted from `files_present` alone): an exact count of the
        // join below costs a quarter of reading it.
        let present: usize = self.conn.query_row("SELECT count(*) FROM files WHERE missing = 0", [], |r| r.get(0))?;
        // An analysis made before the orientations were stored has its main hash but not these: its pairs would
        // be missing one way round. Those files are re-analyzed (`import --again`), not matched with what they lack.
        let without: i64 = self.conn.query_row(
            "SELECT count(*) FROM analysis a JOIN files f ON f.id = a.file_id
             WHERE f.missing = 0 AND f.pending = 0 AND a.pdq_dihedral IS NULL AND a.pdq_quality >= ?1",
            [pg_model::pdq::MIN_QUALITY],
            |r| r.get(0),
        )?;
        if without > 0 {
            return Err(Error::Other(anyhow!(
                "{without} analyses have no orientation hashes (made by an older build): re-import them"
            )));
        }
        let (mut ids, mut hashes) = (Vec::with_capacity(present * 8), Vec::with_capacity(present * 8));
        let mut stmt = self.conn.prepare(
            "SELECT a.file_id, a.pdq_dihedral FROM analysis a JOIN files f ON f.id = a.file_id
             WHERE f.missing = 0 AND f.pending = 0 AND a.pdq_dihedral IS NOT NULL AND a.pdq_quality >= ?1",
        )?;
        let mut rows = stmt.query([pg_model::pdq::MIN_QUALITY])?;
        while let Some(r) = rows.next()? {
            let id = FileId::new(r.get(0)?);
            let blob: Vec<u8> = r.get(1)?;
            let (orientations, _) = blob.as_chunks::<{ pg_model::pdq::HASH_BYTES }>();
            for orientation in orientations {
                ids.push(id);
                hashes.push(pg_model::pdq::Hash(*orientation).words());
            }
        }
        Ok((ids, hashes))
    }

    /// Analyses whose borders are due (new ones, and all after a new detector: `set_borders_version`), of
    /// files the import has nothing to do for, in on-disk order.
    pub fn pending_borders(&self) -> Result<WorkList<WorkItem>> {
        // An upper bound, from `analysis_borders_due` alone.
        let tx = self.conn.unchecked_transaction()?;
        let bound: usize = tx.query_row("SELECT count(*) FROM analysis WHERE borders_due = 1", [], |r| r.get(0))?;
        let mut stmt = tx.prepare(concat!(
            "SELECT ",
            work_columns!(),
            " FROM analysis a CROSS JOIN files f
             WHERE a.borders_due = 1 AND f.id = a.file_id AND f.missing = 0 AND f.pending = 0
             ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF"
        ))?;
        let list = work_list(&tx, &mut stmt, bound)?;
        drop(stmt);
        tx.commit()?;
        Ok(list)
    }

    /// The present file at an absolute path (Windows paths: any case).
    pub fn file_by_path(&self, path: &RulePath) -> Result<Option<FileId>> {
        let Some((folder, name)) = self.rules.split_file_name(path) else { return Ok(None) };
        let key = self.rules.folder_key(&folder.parts);
        let name_key = name.name_key();
        // The files of the folder's key (every drive's copy of it), then the one at this path by the rules: the same
        // drive, and a name equal by key, which is case-insensitive where the rules are (SQLite's lower() is ASCII).
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.id, f.name, fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id
             WHERE fo.key = ?1 AND f.missing = 0",
        )?;
        let rows =
            stmt.query_map([key], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?)))?;
        for row in rows {
            let (id, stored_name, stored_folder) = row?;
            let same_folder = self.rules.parse_bytes(&stored_folder).is_some_and(|f| self.rules.same(&f, &folder));
            let mut stored_key = Vec::new();
            Name::from_raw(&stored_name).push_name_key(&mut stored_key);
            if same_folder && stored_key == name_key {
                return Ok(Some(FileId::new(id)));
            }
        }
        Ok(None)
    }

    /// The present files in a folder and its subfolders (a path in the catalog's case, without a trailing
    /// separator).
    pub fn files_under(&self, folder: &RulePath) -> Result<Vec<FileId>> {
        let mut stmt = self.conn.prepare(concat!(
            "SELECT id FROM files WHERE missing = 0 AND folder_id IN (",
            under_root!(),
            ")"
        ))?;
        let [p1, p2, p3] = under_params(self.rules, folder);
        let ids: Vec<i64> = stmt.query_map(params![p1, p2, p3], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids.into_iter().map(FileId::new).collect())
    }

    /// Make these files due for the import again (`files.pending`), whatever their analysis: after a fix to what
    /// the import reads, which does not bump IMPORT_VERSION. Their next analysis
    /// clears it, as any does (`analysis_added`). Missing files are left as they are. Returns how many it marked.
    pub fn mark_pending(&self, ids: &[FileId]) -> Result<usize> {
        let tx = write_transaction(&self.conn)?;
        let mut marked = 0;
        {
            let mut mark = tx.prepare("UPDATE files SET pending = 1 WHERE id = ?1 AND missing = 0")?;
            for id in ids {
                marked += mark.execute([id.get()])?;
            }
        }
        tx.commit()?;
        Ok(marked)
    }

    /// What cropping a file's borders off needs: its borders as found in the file as it was analyzed (`size`,
    /// `mtime`: the caller checks the file is still that one), its path added to `paths`. `None` when it has none to
    /// crop: none found, marked as not a border, or not looked for in the file as it is.
    pub fn crop_source(&self, file_id: FileId, paths: &mut PathArena) -> Result<Option<CropSource>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT (SELECT path FROM folders WHERE id = f.folder_id), f.name, f.size, f.mtime,
                        a.format, a.orientation, a.width, a.height, a.cut_left, a.cut_top, a.cut_right, a.cut_bottom
                 FROM files f JOIN analysis a ON a.file_id = f.id
                 WHERE f.id = ?1 AND f.missing = 0 AND f.pending = 0 AND a.borders_due = 0
                   AND a.size = f.size AND a.mtime = f.mtime
                   AND a.cut_left + a.cut_top + a.cut_right + a.cut_bottom > 0
                   AND NOT EXISTS(SELECT 1 FROM border_marks WHERE file_id = f.id)",
            )?
            .query_row([file_id.get()], |r| {
                let (orientation, w, h): (Orientation, u32, u32) =
                    (Orientation::from_tag(r.get(5)?), r.get(6)?, r.get(7)?);
                let (width, height) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
                let format = r.get_ref(4)?.as_str()?;
                let format = Format::from_name(format).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(4, Type::Text, format!("unknown format {format}").into())
                })?;
                let (folder, name) = (r.get_ref(0)?.as_blob()?, r.get_ref(1)?.as_blob()?);
                let path = self.rules.sep_byte();
                Ok(CropSource {
                    path: paths.push_parts(&[folder, &[path], name]),
                    size: r.get(2)?,
                    mtime: r.get(3)?,
                    format,
                    width,
                    height,
                    cut: [r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?],
                })
            })
            .optional()?)
    }

    /// Mark a file's borders as not a border, or take the mark back (the review).
    pub fn set_border_mark(&self, file_id: FileId, marked: bool) -> Result<()> {
        if marked {
            self.conn
                .prepare_cached("INSERT OR REPLACE INTO border_marks(file_id, marked) VALUES (?1, unixepoch())")?
                .execute([file_id.get()])?;
        } else {
            self.conn.prepare_cached("DELETE FROM border_marks WHERE file_id = ?1")?.execute([file_id.get()])?;
        }
        Ok(())
    }

    /// The file's state as the readers check it (`pg_model::state::FileState`), `None` when it isn't in the catalog.
    pub fn file_state(&self, file_id: FileId) -> Result<Option<pg_model::state::FileState>> {
        browse::read::file_state(self, file_id)
    }

    /// Where a file is: its folder and name, as Windows spells it.
    pub fn file_path(&self, file_id: FileId) -> Result<RulePath> {
        let (folder, name): (Vec<u8>, Vec<u8>) = self.conn.query_row(
            "SELECT fo.path, f.name FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
            [file_id.get()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        self.file_path_of(&folder, &name)
    }

    /// Give back this connection's page cache (the reader does it after each list it made).
    pub fn release_memory(&self) -> rusqlite::Result<()> {
        self.conn.release_memory()
    }

    /// Each count from its own index where there is one (`files_present`, `files_to_import`, `analysis_failed`).
    pub fn status(&self) -> Result<Status> {
        // One read snapshot: the counts agree with each other while a scan writes.
        let snapshot = self.conn.unchecked_transaction()?;
        let one = |sql: &str| -> Result<i64> { Ok(snapshot.query_row(sql, [], |r| r.get(0))?) };
        let files = one("SELECT count(*) FROM files WHERE missing = 0")?;
        let failed = one("SELECT count(*) FROM analysis a CROSS JOIN files f
             WHERE a.error IS NOT NULL AND f.id = a.file_id AND f.missing = 0")?;
        Ok(Status {
            files,
            missing: one("SELECT count(*) FROM files")? - files,
            // Present files with an analysis that isn't a failure, current or not: the one count that reads them all.
            analyzed: one(
                "SELECT count(*) FROM analysis a JOIN files f ON f.id = a.file_id WHERE f.missing = 0 AND a.error IS NULL",
            )?,
            failed,
            pending: one("SELECT count(*) FROM files WHERE missing = 0 AND pending = 1")?,
            pdq_pairs: one("SELECT count(*) FROM present_pdq_pairs")?,
            queued_writes: one("SELECT count(*) FROM file_writes")?,
            failed_writes: one("SELECT count(*) FROM file_writes WHERE error IS NOT NULL")?,
        })
    }
}

/// No write of the app's into file `?1` is in progress (`file_writes.temp`, set before the file is rewritten in
/// place and cleared in the transaction that records the result): what was read from it is what the catalog
/// says, not a part-written file.
macro_rules! not_being_written {
    () => {
        "NOT EXISTS(SELECT 1 FROM file_writes WHERE file_id = ?1 AND temp IS NOT NULL)"
    };
}

// The passes' writes: one writer per transaction, each statement prepared once for it rather than looked up in
// rusqlite's statement cache per row (16 entries, which the import's statements alone would crowd). A writer
// borrows its transaction, so it is dropped before the transaction commits.

const DELETE_SIDES: &str = "DELETE FROM border_sides WHERE file_id = ?1";
const SET_FILE_RATING: &str = "INSERT INTO file_ratings(file_id, value) VALUES (?1, ?2)
     ON CONFLICT(file_id) DO UPDATE SET value = excluded.value";
const DELETE_FILE_RATING: &str = "DELETE FROM file_ratings WHERE file_id = ?1";

/// A transaction on the catalog. It hands out the writers of the scan, the import and the borders, so a batch of
/// their writes commits together, with `commit`.
pub struct Tx<'c> {
    inner: Transaction<'c>,
    rules: PathRules,
}

impl Catalog {
    /// Begin a transaction on the catalog (see [`Tx`]).
    pub fn tx(&mut self) -> Result<Tx<'_>> {
        Ok(Tx { inner: write_transaction(&self.conn)?, rules: self.rules })
    }
}

impl Tx<'_> {
    /// The writer of the scan `scan` of `root`, in this transaction.
    pub fn scan_writer(&self, scan: i64, root: &RulePath) -> Result<ScanWriter<'_>> {
        ScanWriter::new(&self.inner, scan, self.rules, root)
    }

    /// The writer of the analyses, in this transaction. `import` is the build's IMPORT_VERSION (`Versions::import`),
    /// which the failures it stores are made against.
    pub fn analysis_writer(&self, import: i64) -> Result<AnalysisWriter<'_>> {
        AnalysisWriter::new(&self.inner, import)
    }

    /// The writer of the borders, in this transaction.
    pub fn borders_writer(&self) -> Result<BordersWriter<'_>> {
        BordersWriter::new(&self.inner)
    }

    /// Commit what was written through this transaction's writers.
    pub fn commit(self) -> Result<()> {
        Ok(self.inner.commit()?)
    }
}

/// What recording a file did ([`ScanWriter::upsert_file`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// Recorded as it was: only marked seen by this scan.
    Same,
    /// A new file, moved, renamed, rewritten, back from missing, or the file that now has an identity another row had.
    Changed,
    /// The identity is a row of another root, which this scan doesn't write (S-17.7 decision 5).
    Foreign,
}

/// The row a file's identity has, as the scan needs it.
struct Hit {
    id: i64,
    folder_id: i64,
    /// The same folder, name, kind, size, mtime, order key, and not missing: nothing to write but the scan's mark.
    same: bool,
    created: Option<i64>,
}

/// Write a file as a new row (or, on its identity, the same file's changed fields): unpaired, as `ScanWriter::file` says.
fn write_file(file: &mut Statement<'_>, f: &ScannedFile, folder: &Folder, scan: i64, key: &[u8]) -> Result<usize> {
    let (volume, file_ref) = (f.identity.volume.cast_signed(), f.identity.file_index.cast_signed());
    let size = i64::try_from(f.size).context("a file's size")?;
    let (ext, kind) = (f.ext.name(), f.ext.kind().key());
    Ok(file.execute(params![
        volume,
        file_ref,
        f.created,
        folder.id.get(),
        f.name.as_bytes(),
        ext,
        kind,
        size,
        f.mtime,
        scan,
        key
    ])?)
}

/// The scan's writes.
pub struct ScanWriter<'tx> {
    scan: i64,
    rules: PathRules,
    /// The scanned root's subtree (`under_params`): a row's folder is in it when it is `?1` or between `?2` and `?3`.
    under: [Vec<u8>; 3],
    folder_id: Statement<'tx>,
    new_folder: Statement<'tx>,
    known: Statement<'tx>,
    in_root: Statement<'tx>,
    seen: Statement<'tx>,
    seen_identity: Statement<'tx>,
    backfill: Statement<'tx>,
    displace: Statement<'tx>,
    file: Statement<'tx>,
    /// The order key of the file being recorded.
    key: Vec<u8>,
}

impl<'tx> ScanWriter<'tx> {
    pub fn new(tx: &'tx Transaction, scan: i64, rules: PathRules, root: &RulePath) -> Result<ScanWriter<'tx>> {
        Ok(ScanWriter {
            scan,
            rules,
            under: under_params(rules, root),
            folder_id: tx.prepare("SELECT id FROM folders WHERE path = ?1")?,
            new_folder: tx.prepare("INSERT INTO folders(path, key) VALUES (?1, ?2) RETURNING id")?,
            known: tx.prepare(
                "SELECT id, folder_id, name, ext, kind, size, mtime, missing, order_key, created FROM files
                 WHERE volume = ?1 AND file_ref = ?2",
            )?,
            in_root: tx.prepare(
                "SELECT EXISTS(SELECT 1 FROM folders WHERE id = ?4 AND (path = ?1 OR (path >= ?2 AND path < ?3)))",
            )?,
            seen: tx.prepare("UPDATE files SET last_scan = ?2 WHERE id = ?1")?,
            seen_identity: tx.prepare("UPDATE files SET last_scan = ?3 WHERE volume = ?1 AND file_ref = ?2")?,
            backfill: tx.prepare("UPDATE files SET created = ?2 WHERE id = ?1 AND created IS NULL")?,
            displace: tx
                .prepare("UPDATE files SET volume = NULL, file_ref = NULL, missing = 1, unpaired = 1 WHERE id = ?1")?,
            // A new row is unpaired: the pairing at the end of the scan decides what it is (S-17.7 decision 4).
            file: tx.prepare(
                "INSERT INTO files(volume, file_ref, created, folder_id, name, ext, kind, size, mtime, last_scan,
                    order_key, unpaired)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1)
                 ON CONFLICT(volume, file_ref) DO UPDATE SET
                    folder_id = excluded.folder_id, name = excluded.name, ext = excluded.ext, kind = excluded.kind,
                    size = excluded.size, mtime = excluded.mtime, last_scan = excluded.last_scan, missing = 0,
                    order_key = excluded.order_key, created = coalesce(files.created, excluded.created)",
            )?,
            key: Vec::new(),
        })
    }

    /// The folder's row, made if it's new.
    pub fn folder(&mut self, path: &RulePath) -> Result<Folder> {
        let key = self.rules.folder_key(&path.parts);
        let bytes = self.rules.join(path);
        let id: i64 = match self.folder_id.query_row([&bytes], |r| r.get(0)).optional()? {
            Some(id) => id,
            None => self.new_folder.query_row(params![bytes, key], |r| r.get(0))?,
        };
        Ok(Folder { id: FolderId::new(id), key })
    }

    /// Record a file the scan found, by identity (S-17.7 decision 2). A file moved or renamed keeps its row, and so
    /// does one whose `created` is unknown on either side. A hit whose `created` differs is a different file: the row
    /// is displaced (it keeps its id, folder, name, ratings and content, and loses its identity), and a new row is
    /// made for the identity. A new row is `unpaired` until the pairing at the end of the scan.
    pub fn upsert_file(&mut self, folder: &Folder, f: &ScannedFile) -> Result<Recorded> {
        let ScanWriter { scan, under, known, in_root, seen, backfill, displace, file, key, .. } = self;
        key.clear();
        key.extend_from_slice(&folder.key);
        f.name.push_name_key(key);
        let (volume, file_ref) = (f.identity.volume.cast_signed(), f.identity.file_index.cast_signed());
        let (ext, kind, size) = (f.ext.name(), f.ext.kind().key(), i64::try_from(f.size).context("a file's size")?);
        let hit = known
            .query_row(params![volume, file_ref], |r| {
                let same = r.get::<_, i64>(1)? == folder.id.get()
                    && r.get_ref(2)?.as_blob()? == f.name.as_bytes()
                    && r.get_ref(3)?.as_str()? == ext
                    && r.get_ref(4)?.as_str()? == kind
                    && r.get::<_, i64>(5)? == size
                    && r.get::<_, i64>(6)? == f.mtime
                    && r.get::<_, i64>(7)? == 0
                    && r.get_ref(8)?.as_blob()? == &key[..];
                Ok(Hit { id: r.get(0)?, folder_id: r.get(1)?, same, created: r.get(9)? })
            })
            .optional()?;
        let Some(hit) = hit else {
            write_file(file, f, folder, *scan, &key[..])?;
            return Ok(Recorded::Changed);
        };
        if !in_root.query_row(params![under[0], under[1], under[2], hit.folder_id], |r| r.get::<_, bool>(0))? {
            return Ok(Recorded::Foreign);
        }
        if matches!((hit.created, f.created), (Some(a), Some(b)) if a != b) {
            displace.execute(params![hit.id])?;
            write_file(file, f, folder, *scan, &key[..])?;
            return Ok(Recorded::Changed);
        }
        if hit.created.is_none() && f.created.is_some() {
            backfill.execute(params![hit.id, f.created])?;
        }
        if hit.same {
            seen.execute(params![hit.id, *scan])?;
            return Ok(Recorded::Same);
        }
        write_file(file, f, folder, *scan, &key[..])?;
        Ok(Recorded::Changed)
    }

    /// Mark the row with this identity as seen by this scan, changing nothing else. A reparse point is recorded this
    /// way: its listing gives no content to analyze, and a row that is there is kept (S-17.7 decision 9).
    pub fn seen_identity(&mut self, identity: FileIdentity) -> Result<()> {
        self.seen_identity.execute(params![
            identity.volume.cast_signed(),
            identity.file_index.cast_signed(),
            self.scan
        ])?;
        Ok(())
    }
}

/// The import's writes: analyses, failures, the file ratings they carry, and tier-1 pairs.
pub struct AnalysisWriter<'tx> {
    /// The build's IMPORT_VERSION, which a stored failure is made against.
    import: i64,
    current: Statement<'tx>,
    delete_pairs: Statement<'tx>,
    analysis: Statement<'tx>,
    failure: Statement<'tx>,
    borders_due: Statement<'tx>,
    delete_sides: Statement<'tx>,
    known_rating: Statement<'tx>,
    queued_rating: Statement<'tx>,
    set_rating: Statement<'tx>,
    delete_rating: Statement<'tx>,
    set_file_rating: Statement<'tx>,
    delete_file_rating: Statement<'tx>,
    pair: Statement<'tx>,
}

impl<'tx> AnalysisWriter<'tx> {
    pub fn new(tx: &'tx Transaction, import: i64) -> Result<AnalysisWriter<'tx>> {
        Ok(AnalysisWriter {
            import,
            current: tx.prepare(concat!(
                "SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1 AND size = ?2 AND mtime = ?3 AND rotation = ?4) AND ",
                not_being_written!()
            ))?,
            delete_pairs: tx.prepare("DELETE FROM pdq_pairs WHERE a = ?1 OR b = ?1")?,
            analysis: tx.prepare(
                "INSERT OR REPLACE INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error,
                    file_hash, pixel_hash, format, orientation, width, height, pdq, pdq_quality, thumbhash, frames,
                    taken, taken_utc, duration_ms, poster_ms, borders_due, pdq_dihedral)
                 VALUES (?1, ?2, ?3, ?4, ?19, ?20, unixepoch(), NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                         CASE WHEN ?16 THEN unixepoch(?15, 'unixepoch', 'localtime') ELSE ?15 END,
                         CASE WHEN ?16 THEN ?15 END, ?17, ?18, 0, ?21)",
            )?,
            failure: tx.prepare(
                "INSERT OR REPLACE INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error,
                    borders_due)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch(), ?7, 0)",
            )?,
            borders_due: tx.prepare(concat!(
                "UPDATE analysis SET borders_due = coalesce(",
                has_borders!(),
                ", 0) WHERE file_id = ?1"
            ))?,
            delete_sides: tx.prepare(DELETE_SIDES)?,
            known_rating: tx.prepare("SELECT value FROM file_ratings WHERE file_id = ?1")?,
            queued_rating: tx
                .prepare("SELECT EXISTS(SELECT 1 FROM file_writes WHERE file_id = ?1 AND kind = 'rating')")?,
            set_rating: tx.prepare(
                "INSERT INTO ratings(file_id, rating, updated) VALUES (?1, ?2, unixepoch())
                 ON CONFLICT(file_id) DO UPDATE SET rating = excluded.rating, updated = excluded.updated",
            )?,
            delete_rating: tx.prepare("DELETE FROM ratings WHERE file_id = ?1")?,
            set_file_rating: tx.prepare(SET_FILE_RATING)?,
            delete_file_rating: tx.prepare(DELETE_FILE_RATING)?,
            pair: tx.prepare(
                "INSERT INTO pdq_pairs(a, b, distance, variant) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(a, b) DO UPDATE SET distance = excluded.distance, variant = excluded.variant
                 WHERE excluded.distance < pdq_pairs.distance",
            )?,
        })
    }

    /// Whether the file is still as it was listed (`item`), with no write of the app's into it in progress: only
    /// then is what was read from it stored. Otherwise it is left as it is, due again (a changed file is stale; a
    /// file whose metadata was rewritten keeps the analysis made before, which the writer carried over).
    fn current(&mut self, item: &WorkItem) -> Result<bool> {
        Ok(self.current.query_row(params![item.id.get(), item.size, item.mtime, item.rotation.get()], |r| r.get(0))?)
    }

    /// Store an analysis, unless the file changed since it was listed (see `current`). Whether it was stored.
    pub fn store_analysis(&mut self, item: &WorkItem, analyzed: &Analyzed) -> Result<bool> {
        if !self.current(item)? {
            return Ok(false);
        }
        let a = &analyzed.record;
        // A re-analysis invalidates the file's tier-1 pairs; the caller re-queries it.
        self.delete_pairs.execute([item.id.get()])?;
        let (taken, taken_utc) = a.taken.map_or((None, false), |t| {
            let (seconds, utc) = t.parts();
            (Some(seconds), utc)
        });
        let (image, video) = (a.image(), a.video());
        self.analysis.execute(params![
            item.id.get(),
            a.version,
            item.size,
            item.mtime,
            &a.file_hash[..],
            image.map(|i| &i.pixels.pixel_hash[..]),
            a.format(),
            a.orientation.get(),
            a.width,
            a.height,
            image.map(|i| &i.pixels.pdq.hash.0[..]),
            image.map(|i| i.pixels.pdq.quality),
            a.thumbhash.as_slice(),
            a.frames,
            taken,
            taken_utc,
            video.map(|v| v.duration_ms),
            video.map(|v| v.poster_ms),
            item.rotation.get(),
            item.kind.key(),
            analyzed.pdq_dihedral.map(|d| d.iter().flat_map(|h| h.0).collect::<Vec<u8>>()),
        ])?;
        // The borders of what was there before are gone; the borders pass finds the new ones.
        self.borders_due.execute([item.id.get()])?;
        self.delete_sides.execute([item.id.get()])?;
        self.sync_file_rating(item.id, image.and_then(|i| i.file_rating))?;
        Ok(true)
    }

    /// Store a failure, unless the file changed since it was listed (see `current`). Whether it was stored.
    pub fn store_failure(&mut self, item: &WorkItem, error: &str) -> Result<bool> {
        if !self.current(item)? {
            return Ok(false);
        }
        self.delete_pairs.execute([item.id.get()])?;
        self.failure.execute(params![
            item.id.get(),
            self.import,
            item.size,
            item.mtime,
            item.rotation.get(),
            item.kind.key(),
            error
        ])?;
        self.delete_sides.execute([item.id.get()])?;
        Ok(true)
    }

    pub fn store_pdq_pair(&mut self, a: FileId, b: FileId, distance: u32, variant: usize) -> Result<()> {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        self.pair.execute(params![a.get(), b.get(), distance, variant])?;
        Ok(())
    }

    /// Three-way rating sync. `file_ratings` holds what the file was last known to hold; a file rating that
    /// differs from it was set by another program after that, so it becomes the catalog rating. Otherwise the
    /// catalog's stands (it may be newer and not written back yet, or the file can't hold one). A write of our own
    /// still queued is newer than anything in the file: nothing is adopted then.
    fn sync_file_rating(&mut self, file_id: FileId, in_file: Option<Rating>) -> Result<()> {
        let known: Option<Rating> =
            self.known_rating.query_row([file_id.get()], |r| r.get::<_, u8>(0)).optional()?.and_then(Rating::new);
        let queued: bool = self.queued_rating.query_row([file_id.get()], |r| r.get(0))?;
        if in_file == known || queued {
            return Ok(());
        }
        match in_file {
            // The user's ratings are 1 to 5: a file that holds none (0) clears the catalog's.
            Some(v) if v.get() >= 1 => self.set_rating.execute(params![file_id.get(), v.get()])?,
            _ => self.delete_rating.execute([file_id.get()])?,
        };
        match in_file {
            Some(v) => self.set_file_rating.execute(params![file_id.get(), v.get()])?,
            None => self.delete_file_rating.execute([file_id.get()])?,
        };
        Ok(())
    }
}

/// The borders pass's writes.
pub struct BordersWriter<'tx> {
    current: Statement<'tx>,
    cut: Statement<'tx>,
    delete_sides: Statement<'tx>,
    side: Statement<'tx>,
}

impl<'tx> BordersWriter<'tx> {
    pub fn new(tx: &'tx Transaction) -> Result<BordersWriter<'tx>> {
        Ok(BordersWriter {
            current: tx.prepare(concat!(
                "SELECT EXISTS(SELECT 1 FROM analysis WHERE file_id = ?1 AND size = ?2 AND mtime = ?3) AND ",
                not_being_written!()
            ))?,
            cut: tx.prepare(
                "UPDATE analysis SET cut_left = ?2, cut_top = ?3, cut_right = ?4, cut_bottom = ?5, borders_due = 0
                 WHERE file_id = ?1",
            )?,
            delete_sides: tx.prepare(DELETE_SIDES)?,
            side: tx.prepare(
                "INSERT INTO border_sides(file_id, pass, edge, kind, class, depth, color, straight, dev, drift,
                    rejected)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?,
        })
    }

    /// Record borders looked for ([`Catalog::pending_borders`]; `None`: not looked for), replacing what was there,
    /// unless the file changed meanwhile (the import then analyzes it again, and the borders are due again) or a
    /// write of the app's into it is in progress (they stay due).
    pub fn store_borders(&mut self, item: &WorkItem, borders: Option<&Detection>) -> Result<()> {
        let current: bool = self.current.query_row(params![item.id.get(), item.size, item.mtime], |r| r.get(0))?;
        if !current {
            return Ok(());
        }
        let cut = borders.map(Detection::cut);
        self.cut.execute(params![
            item.id.get(),
            cut.map(|c| c[0]),
            cut.map(|c| c[1]),
            cut.map(|c| c[2]),
            cut.map(|c| c[3])
        ])?;
        self.delete_sides.execute([item.id.get()])?;
        for s in borders.map_or(&[][..], |b| &b.sides) {
            let color = s.color.map(|[r, g, b]| (i64::from(r) << 16) | (i64::from(g) << 8) | i64::from(b));
            self.side.execute(params![
                item.id.get(),
                s.pass,
                s.edge.key(),
                s.kind.key(),
                s.class().key(),
                s.depth,
                color,
                s.straight,
                s.dev,
                s.drift,
                s.rejected.map(|r| r.key()),
            ])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::raw;
    use pg_analysis::analyze::IMPORT_VERSION;
    use pg_analysis::borders::BORDERS_VERSION;
    use pg_model::analysis::{Analysis, ImageFacts, Media, PixelHashes, Taken, ThumbHash};
    use pg_testkit::test_folder;

    /// The versions this build's analyses are stamped with, as the library passes them to [`Catalog::prepare`].
    fn versions() -> Versions {
        Versions { import: IMPORT_VERSION, borders: BORDERS_VERSION }
    }

    /// A path under the tests' rules.
    fn p(text: &str) -> RulePath {
        RULES.parse(text).expect("a test path")
    }

    /// The rules the tests of this module open their catalogs under: the Windows ones, as the app has them.
    const RULES: PathRules = PathRules::Windows;

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

    // The layout tests below read the file on disk through their own connection (`raw::on_disk`): what they check
    // is the layout, not the catalog's tables.

    #[test]
    fn an_earlier_layout_is_moved_aside_and_a_new_catalog_is_made() {
        let dir = test_folder(SCRATCH, "move-aside").unwrap();
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

        let c = Catalog::prepare(&path, versions(), RULES).unwrap();
        let disk = raw::on_disk(&path);
        assert!(is_current(&disk).unwrap());
        let count = |conn: &Connection, table: &str| -> i64 {
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
        };
        // The old layout lacks columns the new one needs as NOT NULL (`folders.key`, then `files.order_key`): the copy
        // fails as a whole, so nothing of the old rows is in the new catalog (not even its folders), and the reason is
        // said. The earlier one is kept whole, rows and all, beside it.
        let r = c.rebuilt().expect("moved aside");
        assert!(
            matches!(&r.carried, Carried::Nothing(why) if why.starts_with("NOT NULL constraint failed")),
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
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A catalog that differs from this build's schema only by an index it shouldn't have is rebuilt with its rows: the
    /// same files under the same ids, and the library folders.
    #[test]
    fn a_catalog_that_differs_keeps_its_rows() {
        let dir = test_folder(SCRATCH, "keeps-rows").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), RULES).unwrap();
        raw::add_scan(&c, 1, r"D:\p");
        raw::add_folder(&c, 1, r"D:\p");
        raw::add_folder(&c, 2, r"D:\p\Sub");
        fixture::insert(
            &c,
            &[
                fixture::File::new(FileId::new(11), FolderId::new(1), "B.jpg", 100, 5),
                fixture::File::new(FileId::new(12), FolderId::new(2), "a.png", 200, 6),
            ],
        )
        .unwrap();
        assert!(c.add_root(&RULES.parse(r"D:\p").expect("a test path")).unwrap());
        drop(c);
        let edited = Connection::open(&path).unwrap();
        edited.execute_batch("CREATE INDEX extra_names ON files(name);").unwrap();
        drop(edited);

        let c = Catalog::prepare(&path, versions(), RULES).unwrap();
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
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Rows stored under other rules are not carried into a catalog of these: their paths don't read as its own.
    #[test]
    fn rows_of_other_rules_are_not_carried_over() {
        let dir = test_folder(SCRATCH, "folders-rules").unwrap();
        let path = dir.join("catalog.sqlite");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(BEFORE_ORDER_KEY).unwrap();
        old.execute_batch(r"INSERT INTO roots VALUES ('D:\p', 1);").unwrap();
        drop(old);
        let c = Catalog::prepare(&path, versions(), PathRules::Posix).unwrap();
        let r = c.rebuilt().expect("moved aside");
        assert_eq!(r.carried, Carried::Nothing("made with the windows path rules".to_string()));
        assert!(c.roots().unwrap().is_empty());
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside() {
        let dir = test_folder(SCRATCH, "layout_source").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), RULES).unwrap());
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
        let old_view = view.replace("f.size, f.missing", "f.missing");
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

        let c = Catalog::prepare(&path, versions(), RULES).unwrap();
        let disk = raw::on_disk(&path);
        assert!(is_current(&disk).unwrap());
        let current: String = disk
            .query_row(
                "SELECT group_concat(sql, ' ') FROM sqlite_schema WHERE name IN ('files_source_changed', 'thumb_sources')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(current.contains("old.size IS NOT new.size") && current.contains("f.size, f.missing"), "{current}");
        assert!(dir.join("catalog.sqlite.replaced").exists());
        drop(disk);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name() {
        let dir = test_folder(SCRATCH, "move-aside-twice").unwrap();
        let path = dir.join("catalog.sqlite");
        // Files without the columns the current layout needs.
        let old_layout = |path: &Path| {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value ANY NOT NULL); INSERT INTO meta VALUES ('path_rules', 'windows');
                 CREATE TABLE files(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO files VALUES (1, 'x');
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        };
        old_layout(&path);
        drop(Catalog::prepare(&path, versions(), RULES).unwrap());
        let moved = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(moved.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(moved);
        // Another one moved aside later keeps the first: the new catalog made in the first one's place goes first.
        for part in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(dir.join(format!("catalog.sqlite{part}")));
        }
        old_layout(&path);
        drop(Catalog::prepare(&path, versions(), RULES).unwrap());
        assert!(dir.join("catalog.sqlite.replaced.1").exists());
        assert!(dir.join("catalog.sqlite.replaced").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused() {
        let dir = test_folder(SCRATCH, "move-aside-no-rules").unwrap();
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
        let c = Catalog::prepare(&path, versions(), RULES).expect("rebuilt, not refused");
        drop(c);
        let moved = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(moved.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(moved);
        // The new catalog records its rules, so it opens again.
        drop(Catalog::prepare(&path, versions(), RULES).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn catalogs_have_their_own_id() {
        let (a, b) = (
            Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap(),
            Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap(),
        );
        assert_eq!(a.catalog_id().unwrap().len(), 32);
        assert_ne!(a.catalog_id().unwrap(), b.catalog_id().unwrap());
    }

    #[test]
    fn a_catalog_made_under_other_rules_is_refused() {
        let dir = test_folder(SCRATCH, "path-rules").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PathRules::Windows).unwrap());
        // Another build's rules: its paths would read as other paths. The catalog is valid, so it is refused, not moved.
        let e = Catalog::open(&path, PathRules::Posix).err().expect("refused");
        assert!(matches!(e, Error::PathRules { .. }), "{e:?}");
        assert!(e.to_string().contains("on the platform that made it"), "{e}");
        assert!(matches!(Catalog::prepare(&path, versions(), PathRules::Posix), Err(Error::PathRules { .. })));
        assert!(!dir.join("catalog.sqlite.replaced").exists());
        // Its rules row gone is not this build's schema: it is moved aside, and the new catalog is made.
        raw::on_disk(&path).execute("DELETE FROM meta WHERE key = 'path_rules'", []).unwrap();
        assert!(Catalog::open(&path, PathRules::Windows).is_err());
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        assert!(c.rebuilt().is_some());
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn posix_paths_are_bytes_under_case_sensitive_rules() {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), PathRules::Posix).unwrap();
        let rules = c.rules();
        raw::add_scan(&c, 1, "/photos");
        raw::add_folder(&c, 1, "/photos");
        raw::add_folder(&c, 2, "/photos/Pics");
        raw::add_folder(&c, 3, "/photos/pics");
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
        assert_eq!(rules.display(&c.file_path(FileId::new(2)).unwrap()), "/photos/Pics/b.jpg");
        // Folders that differ only in case are two folders, and their files are two files.
        assert_eq!(c.file_by_path(&root("/photos/Pics/b.jpg")).unwrap(), Some(FileId::new(2)));
        assert_eq!(c.file_by_path(&root("/photos/pics/b.jpg")).unwrap(), Some(FileId::new(3)));
        assert_eq!(c.files_under(&root("/photos/Pics")).unwrap().len(), 1);
    }

    #[test]
    fn roots_never_nest() {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        assert!(c.add_root(&p(r"D:\a\b")).unwrap());
        assert!(c.add_root(&p(r"D:\a\bc")).unwrap());
        assert!(!c.add_root(&p(r"d:\A\b\c")).unwrap(), "inside an existing root");
        assert!(c.add_root(&p(r"D:\a")).unwrap(), "replaces both");
        let paths =
            |c: &Catalog| c.roots().unwrap().into_iter().map(|r| c.rules().display(&r.path)).collect::<Vec<_>>();
        assert_eq!(paths(&c), [r"D:\a"]);
        let id = c.root_id(&p(r"D:\a")).unwrap().expect("a library folder has an id");
        assert_eq!(c.remove_root(id).unwrap(), Some(0));
        assert!(paths(&c).is_empty());
        assert_eq!(c.remove_root(id).unwrap(), None, "not a library folder any more");
    }

    #[test]
    fn cancelled_until_cleared() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        c.cancel_when(cancel.clone()).unwrap();
        let run = || raw::slow_count(&c).map_err(Error::from);
        assert_eq!(run().unwrap(), 100_000);
        // Set between statements (SQLite's own interrupt would be lost there): every statement fails.
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(run().unwrap(), 100_000);
    }

    /// Whether the catalog at `conn` is this build's schema at its version: the check [`Catalog::prepare`] makes.
    fn is_current(conn: &Connection) -> Result<bool> {
        Ok(mismatch(conn, current_version()?, current_expected()?)?.is_none())
    }

    /// Why the preparation moved the old catalog aside (`None` when it kept it).
    fn rebuilt_why(c: &Catalog) -> Option<String> {
        c.rebuilt().map(|r| r.why.clone())
    }

    /// A catalog prepared at `path`, then changed by `sql`, which is run on it as the file is (closed again).
    fn edited(path: &Path, sql: &str) {
        drop(Catalog::prepare(path, versions(), RULES).unwrap());
        raw::on_disk(path).execute_batch(sql).unwrap();
    }

    #[test]
    fn preparing_a_catalog_again_rebuilds_nothing() {
        let dir = test_folder(SCRATCH, "prepare-twice").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), RULES).unwrap());
        let again = Catalog::prepare(&path, versions(), RULES).unwrap();
        assert!(again.rebuilt().is_none());
        drop(again);
        assert!(!dir.join("catalog.sqlite.replaced").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_changed_definition_with_the_same_names_is_moved_aside() {
        // The same object names: an added column, an index on other columns, an object this build doesn't make.
        for (name, sql, named) in [
            ("edited-column", "ALTER TABLE files ADD COLUMN extra INTEGER;", "files"),
            ("edited-index", "DROP INDEX files_changed; CREATE INDEX files_changed ON files(name);", "files_changed"),
            ("edited-table", "CREATE TABLE extra(x INTEGER);", "extra"),
        ] {
            let dir = test_folder(SCRATCH, name).unwrap();
            let path = dir.join("catalog.sqlite");
            edited(&path, sql);
            let c = Catalog::prepare(&path, versions(), RULES).unwrap();
            let why = rebuilt_why(&c).unwrap_or_else(|| panic!("{name}: not moved aside"));
            assert!(why.contains(named), "{name}: {why}");
            assert!(is_current(&raw::on_disk(&path)).unwrap());
            drop(c);
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_comment_in_a_stored_definition_is_not_a_schema_change() {
        let dir = test_folder(SCRATCH, "comment-edit").unwrap();
        let path = dir.join("catalog.sqlite");
        // A catalog whose `files` table was written with a comment in it: the same schema.
        let commented = BASELINE.replacen("CREATE TABLE files(", "CREATE TABLE files( -- an added comment\n", 1);
        assert_ne!(commented, BASELINE);
        let conn = Connection::open(&path).unwrap();
        bring_to(&conn, &commented, MIGRATIONS, RULES, current_version().unwrap()).unwrap();
        drop(conn);
        let c = Catalog::prepare(&path, versions(), RULES).unwrap();
        assert!(c.rebuilt().is_none(), "a comment moved the catalog aside");
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_database_that_is_not_a_catalog_is_moved_aside_not_built_on() {
        let dir = test_folder(SCRATCH, "foreign-database").unwrap();
        let path = dir.join("catalog.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE notes(x INTEGER); INSERT INTO notes VALUES (5);").unwrap();
        }
        let c = Catalog::prepare(&path, versions(), RULES).unwrap();
        let why = rebuilt_why(&c).expect("moved aside");
        assert!(why.contains("notes"), "{why}");
        let kept = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(kept.query_row("SELECT x FROM notes", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        drop(kept);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_that_is_not_a_database_is_moved_aside_unread() {
        let dir = test_folder(SCRATCH, "not-a-database").unwrap();
        let path = dir.join("catalog.sqlite");
        std::fs::write(&path, vec![b'x'; 8192]).unwrap();
        let c = Catalog::prepare(&path, versions(), RULES).unwrap();
        assert_eq!(rebuilt_why(&c).as_deref(), Some("not a database"));
        assert_eq!(std::fs::read(dir.join("catalog.sqlite.replaced")).unwrap(), vec![b'x'; 8192]);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_same_schema_under_another_application_id_is_moved_aside() {
        let dir = test_folder(SCRATCH, "application-id").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), RULES).unwrap());
        raw::on_disk(&path).pragma_update(None, "application_id", 7).unwrap();
        let c = Catalog::prepare(&path, versions(), RULES).unwrap();
        assert_eq!(rebuilt_why(&c).as_deref(), Some("application id 7, not a catalog's"));
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn open_refuses_a_catalog_changed_after_it_was_prepared() {
        let dir = test_folder(SCRATCH, "changed-after-prepare").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), RULES).unwrap());
        raw::on_disk(&path).execute_batch("CREATE TABLE extra(x INTEGER);").unwrap();
        let e = Catalog::open(&path, RULES).err().expect("refused");
        assert!(e.to_string().contains("extra"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_preparations_of_one_catalog_move_it_aside_once() {
        let dir = test_folder(SCRATCH, "concurrent-prepare").unwrap();
        let path = dir.join("catalog.sqlite");
        std::fs::write(&path, vec![b'x'; 8192]).unwrap();
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || Catalog::prepare(&path, versions(), RULES).map(|c| c.rebuilt().is_some()))
            })
            .collect();
        let moved: Vec<bool> = threads.into_iter().map(|t| t.join().unwrap().unwrap()).collect();
        assert_eq!(moved.iter().filter(|m| **m).count(), 1, "{moved:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_memory_catalog_takes_no_lock_file() {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        assert!(c.rebuilt().is_none());
        assert!(!Path::new(":memory:.lock").exists());
    }

    /// A catalog with one file in it.
    fn one_file() -> (Catalog, i64) {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        raw::add_scan(&c, 1, "D:");
        raw::add_folder(&c, 1, r"D:\x");
        fixture::insert(&c, &[fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]).unwrap();
        (c, 1)
    }

    /// A turn the file can't hold, written by the writer: the catalog keeps it.
    fn turn_kept(c: &Catalog, id: FileId, turns: i64) {
        let job =
            writes::Job { file_id: id, kind: pg_model::write::WriteKind::Turns, value: turns, temp: None, attempts: 0 };
        writes::done(c, &job, writes::WriteEffect::KeptInCatalog(Turns::new(u8::try_from(turns).unwrap()))).unwrap();
    }

    /// Whether the import would analyze file `id` now.
    fn due(c: &Catalog, id: FileId) -> bool {
        c.pending_import(false).unwrap().items.iter().any(|i| i.id == id)
    }

    /// Whether file `id` is due for the import, and its change number (`files.seq`).
    fn state(c: &Catalog, id: FileId) -> (bool, i64) {
        (due(c, id), raw::file_seq(c, id))
    }

    /// The scanner finding file 7 on volume 1 in D:\x like this; whether that changed the catalog.
    fn scanned(c: &mut Catalog, scan: i64, name: &str, size: u64, mtime: i64) -> bool {
        let tx = c.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &p(r"D:\x")).unwrap();
        let folder = writer.folder(&p(r"D:\x")).unwrap();
        let ext = Ext::of_name(name).unwrap();
        let file =
            ScannedFile { identity: FileIdentity::new(1, 7), name: &Name::from(name), ext, size, mtime, created: None };
        let changed = writer.upsert_file(&folder, &file).unwrap() == Recorded::Changed;
        drop(writer);
        tx.commit().unwrap();
        changed
    }

    #[test]
    fn a_scan_tells_a_file_found_as_it_was_from_one_that_changed() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        c.add_root(&p(r"D:\x")).unwrap();
        let key = |name| pg_model::path::order_key_of(r"D:\x", name);
        let scans: Vec<i64> = (0..9).map(|_| c.begin_scan(&p(r"D:\x")).unwrap()).collect();
        assert!(scanned(&mut c, scans[0], "a.jpg", 10, 100), "new");
        // As it was: only marked seen by the scan.
        assert!(!scanned(&mut c, scans[1], "a.jpg", 10, 100));
        assert_eq!(raw::file_row(&c), (scans[1], "a.jpg".into(), 10, 100, 0, key("a.jpg")));
        for (scan, name, size, mtime) in [
            (scans[2], "A.jpg", 10, 100),
            (scans[3], "b.jpg", 10, 100),
            (scans[4], "b.jpg", 11, 100),
            (scans[5], "b.jpg", 11, 101),
        ] {
            assert!(scanned(&mut c, scan, name, size, mtime), "{name} {size} {mtime}");
            assert_eq!(raw::file_row(&c), (scan, name.into(), i64::try_from(size).unwrap(), mtime, 0, key(name)));
        }
        // Back from missing.
        raw::set_missing(&c, FileId::new(1));
        assert!(scanned(&mut c, scans[6], "b.jpg", 11, 101));
        assert_eq!(raw::file_row(&c).4, 0);
        // An order key made by an earlier rule is made again.
        raw::set_order_key(&c, FileId::new(1), &[0]);
        assert!(scanned(&mut c, scans[7], "b.jpg", 11, 101));
        assert_eq!(raw::file_row(&c).5, key("b.jpg"));
        assert!(!scanned(&mut c, scans[8], "b.jpg", 11, 101));
    }

    /// The scanner finding a file in D:\x with this identity and creation time (a.jpg, 10 bytes, mtime 100).
    fn found_as(c: &mut Catalog, scan: i64, identity: FileIdentity, created: Option<i64>) -> Recorded {
        let tx = c.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &p(r"D:\x")).unwrap();
        let folder = writer.folder(&p(r"D:\x")).unwrap();
        let file = ScannedFile {
            identity,
            name: &Name::from("a.jpg"),
            ext: Ext::of_name("a.jpg").unwrap(),
            size: 10,
            mtime: 100,
            created,
        };
        let recorded = writer.upsert_file(&folder, &file).unwrap();
        drop(writer);
        tx.commit().unwrap();
        recorded
    }

    #[test]
    fn a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        let scan = c.begin_scan(&p(r"D:\x")).unwrap();
        let (volume, file) = (u64::MAX - 1, u64::MAX);
        assert_eq!(found_as(&mut c, scan, FileIdentity::new(volume, file), None), Recorded::Changed);
        let stored: (i64, i64) =
            c.conn.query_row("SELECT volume, file_ref FROM files", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((stored.0.cast_unsigned(), stored.1.cast_unsigned()), (volume, file));
        // The same identity read back from the catalog is the same file: no new row.
        assert_eq!(found_as(&mut c, scan, FileIdentity::new(volume, file), None), Recorded::Same);
    }

    #[test]
    fn a_creation_time_fills_in_once_and_a_different_one_is_a_different_file() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        let scan = c.begin_scan(&p(r"D:\x")).unwrap();
        let identity = FileIdentity::new(1, 7);
        // Unknown on the first scan (a file system that doesn't give it), then known: it is filled in.
        assert_eq!(found_as(&mut c, scan, identity, None), Recorded::Changed);
        assert_eq!(found_as(&mut c, scan, identity, Some(5)), Recorded::Same);
        let created: Option<i64> =
            c.conn.query_row("SELECT created FROM files WHERE volume = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(created, Some(5));
        // A different creation time is a different file: the row is displaced, and a new row takes the identity.
        assert_eq!(found_as(&mut c, scan, identity, Some(6)), Recorded::Changed);
        let rows: i64 = c.conn.query_row("SELECT count(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 2);
        let displaced: bool = c
            .conn
            .query_row(
                "SELECT volume IS NULL AND unpaired = 1 AND created = 5 FROM files WHERE volume IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(displaced);
    }

    #[test]
    fn what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        c.add_root(&p(r"D:\x")).unwrap();
        let scan = c.begin_scan(&p(r"D:\x")).unwrap();
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let stored = |c: &mut Catalog, item: &WorkItem| {
            let tx = c.tx().unwrap();
            let stored = tx.analysis_writer(versions().import).unwrap().store_failure(item, "a stand-in").unwrap();
            tx.commit().unwrap();
            stored
        };
        let item = next_work(&c);
        // A write of the app's into it in progress: it may have been read part-written.
        writes::queue_rotation(&c, item.id, Turns::new(3)).unwrap();
        let job = writes::next_ready(&c, 0).unwrap().unwrap();
        writes::record_temp(&c, &job, r"D:\x\a.jpg.tmp", 3).unwrap();
        assert!(!stored(&mut c, &item));
        assert!(due(&c, item.id), "still due");
        writes::done(&c, &job, writes::WriteEffect::Nothing).unwrap();
        // Changed since it was listed.
        scanned(&mut c, scan, "a.jpg", 11, 101);
        assert!(!stored(&mut c, &item));
        // Turned in the catalog since.
        turn_kept(&c, item.id, 1);
        let listed = next_work(&c);
        assert_eq!(listed.rotation, Turns::new(1));
        let unturned = WorkItem { rotation: Turns::new(0), ..listed };
        assert!(!stored(&mut c, &unturned));
        // As listed.
        assert!(stored(&mut c, &listed));
        assert!(!due(&c, item.id));
    }

    #[test]
    fn a_size_change_alone_moves_the_file_in_the_thumbnail_source() {
        // The scanner decides a file changed by its size and mtime, so a size change with the same mtime must reach
        // the thumbnail pass (through files.seq), or a thumbnail made from the old file would be kept.
        use fixture::{File, insert};
        let c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        raw::add_scan(&c, 1, r"D:\x");
        raw::add_folder(&c, 1, r"D:\x");
        insert(&c, &[File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]).unwrap();
        raw::insert_analysis(&c, &raw::AnalysisRow::image(1, 10, 100));
        let before = raw::thumb_source_seq(&c, FileId::new(1));
        raw::set_size(&c, FileId::new(1), 11);
        assert!(raw::thumb_source_seq(&c, FileId::new(1)) > before);
    }

    #[test]
    fn thumb_sources_is_the_analysed_files_with_their_folders() {
        use fixture::{File, insert};
        let c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        raw::add_scan(&c, 1, r"D:\x");
        raw::add_folder(&c, 1, r"D:\x");
        raw::add_folder(&c, 2, r"D:\x\sub");
        insert(
            &c,
            &[
                File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100),
                File::new(FileId::new(2), FolderId::new(2), "b.jpg", 10, 100),
                File::new(FileId::new(3), FolderId::new(1), "c.jpg", 10, 100),
            ],
        )
        .unwrap();
        // File 1 analyzed, file 2 failed (still a source: the pass leaves it out), file 3 not analyzed (not a source).
        raw::insert_analysis(&c, &raw::AnalysisRow::image(1, 10, 100));
        raw::insert_analysis(
            &c,
            &raw::AnalysisRow { error: Some("unreadable"), ..raw::AnalysisRow::image(2, 10, 100) },
        );
        assert_eq!(raw::thumb_sources(&c), [(1, r"D:\x".to_string()), (2, r"D:\x\sub".to_string())]);
        // The clock is the catalog's change number, as the pass reads it.
        assert_eq!(raw::thumb_clock(&c), raw::meta_seq(&c));
        // The pass's shape over it, with a TEMP stub of the thumbnail cache's entries: a change number's range still
        // reads files_seq, and a file with no thumbnail yet is the one listed.
        raw::stub_thumb_entries(&c);
        let sql = "SELECT f.id FROM thumb_sources f LEFT JOIN entries t ON t.file_id = f.id AND t.height = ?1
                   WHERE f.seq > ?2 AND f.missing = 0 AND f.error IS NULL AND f.analyzed_mtime = f.mtime
                     AND (t.file_id IS NULL OR t.mtime != f.mtime OR t.rotation != f.rotation)
                   ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF";
        let plan = raw::query_plan(&c, sql, [100, -1]);
        assert!(plan.contains("files_seq"), "{plan:?}");
        let listed = raw::rows_of(&c, sql, [100, -1], |r| r.get::<_, i64>(0));
        assert_eq!(listed, [1]);
    }

    /// The facts of a JPEG with no rating and stand-in hashes, for the analyses the tests store.
    fn a_jpeg() -> Media {
        let pdq = pg_model::pdq::Pdq { hash: pg_model::pdq::Hash([0; 32]), quality: 0 };
        let pixels = PixelHashes { pixel_hash: [0; 16], pdq };
        Media::Image(ImageFacts { format: Format::Jpeg, file_rating: None, pixels })
    }

    /// An analysis as the import holds it: an image's dihedral hashes are stand-ins (`a_jpeg`'s), a video's are none.
    fn stored_analysis(record: &Analysis) -> Analyzed {
        let pdq_dihedral = record.image().map(|_| [pg_model::pdq::Hash([0; 32]); 8]);
        Analyzed { record: record.clone(), pdq_dihedral }
    }

    /// The import storing what it made of `item` (a failure: it counts as made, and needs no Analysis).
    fn analyzed(c: &mut Catalog, item: &WorkItem) {
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().store_failure(item, "a stand-in for an analysis").unwrap();
        tx.commit().unwrap();
    }

    fn next_work(c: &Catalog) -> WorkItem {
        let mut work = c.pending_import(false).unwrap();
        assert_eq!(work.len(), 1);
        work.items.remove(0)
    }

    #[test]
    fn borders_are_looked_for_after_the_import_and_again_after_a_new_detector() {
        use crate::browse;
        use pg_model::detection::{Edge, Kind as SideKind, Reason, Side};
        use pg_model::query::{BorderFacet, Query, Sort};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        c.add_root(&p(r"D:\x")).unwrap();
        let scan = c.begin_scan(&p(r"D:\x")).unwrap();
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let item = next_work(&c);
        let solid = |edge| Side {
            pass: 0,
            edge,
            kind: SideKind::Solid,
            depth: 41,
            color: Some([0, 0, 0]),
            straight: 1.0,
            dev: Some(0.0),
            drift: Some(0.5),
            rejected: None,
        };
        let sky = Side {
            pass: 0,
            edge: Edge::Left,
            kind: SideKind::Blur,
            depth: 30,
            color: None,
            straight: 0.75,
            dev: None,
            drift: None,
            rejected: Some(Reason::UnpairedBlur),
        };
        let found = Detection {
            width: 400,
            height: 300,
            content: [0, 41, 400, 259],
            sides: [solid(Edge::Top), solid(Edge::Bottom), sky].into_iter().collect(),
        };
        let analysis = Analysis {
            version: IMPORT_VERSION,
            file_hash: [1; 32],
            orientation: Orientation::UPRIGHT,
            width: 400,
            height: 300,
            thumbhash: ThumbHash::default(),
            frames: 1,
            taken: None,
            media: a_jpeg(),
        };
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
        tx.commit().unwrap();
        // The import leaves them to the borders pass.
        assert_eq!(raw::cut_sides(&c), [None; 4]);
        let due = c.pending_borders().unwrap();
        assert_eq!(due.items.iter().map(|d| d.id).collect::<Vec<_>>(), [item.id]);
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&due.items[0], Some(&found)).unwrap();
        tx.commit().unwrap();
        assert_eq!(raw::cut_sides(&c), [Some(0), Some(41), Some(0), Some(41)]);
        assert_eq!(browse::read::border_sides(&c, item.id).unwrap(), found.sides);
        assert!(c.pending_borders().unwrap().is_empty());

        // The grid's entry, the filters and the sort.
        let list = |c: &Catalog, q: Query| -> Vec<pg_model::grid::Entry> {
            browse::read::list_grid(c, "", true, &q, Sort::default()).unwrap()
        };
        let entry = list(&c, Query::default()).remove(0);
        assert_eq!(entry.borders, pg_model::grid::Borders::Found);
        let [x, y, w, h] = entry.content().unwrap();
        assert!(x == 0.0 && w == 1.0 && (y - 41.0 / 300.0).abs() < 1e-9 && (h - 218.0 / 300.0).abs() < 1e-9);
        let only = |b: Vec<BorderFacet>| Query { borders: b, ..Query::default() };
        assert_eq!(list(&c, only(vec![BorderFacet::Black])).len(), 1);
        assert!(list(&c, only(vec![BorderFacet::Blur, BorderFacet::None])).is_empty());
        let counts = browse::read::facets(&c, "", true, &Query::default()).unwrap().borders;
        assert_eq!(counts.iter().find(|(b, _)| *b == BorderFacet::Black).unwrap().1, 1);
        assert_eq!(counts.iter().find(|(b, _)| *b == BorderFacet::Blur).unwrap().1, 0);

        // Marked as not a border: no border to the filters, the mark to the review.
        c.set_border_mark(item.id, true).unwrap();
        assert_eq!(list(&c, Query::default())[0].borders, pg_model::grid::Borders::Marked);
        assert!(list(&c, only(vec![BorderFacet::Black])).is_empty());
        assert_eq!(list(&c, only(vec![BorderFacet::None])).len(), 1);
        assert_eq!(list(&c, Query { border_marked: Some(true), ..Query::default() }).len(), 1);

        // A new detector: due again, without a new import; the mark stays.
        raw::older_borders_version(&c);
        raw::start_borders_version(&c);
        assert!(c.pending_import(false).unwrap().is_empty());
        let due = c.pending_borders().unwrap();
        assert_eq!(due.len(), 1);
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&due.items[0], Some(&Detection::none(400, 300))).unwrap();
        tx.commit().unwrap();
        assert_eq!(raw::cut_sides(&c), [Some(0); 4]);
        assert!(browse::read::border_sides(&c, item.id).unwrap().is_empty());
        assert!(c.pending_borders().unwrap().is_empty());
        assert_eq!(list(&c, Query::default())[0].borders, pg_model::grid::Borders::None);
        assert_eq!(raw::border_marks(&c), 1);

        // Analyzed again (the file changed): the old borders go, the new ones are due.
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&due.items[0], Some(&found)).unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
        tx.commit().unwrap();
        assert_eq!(raw::cut_sides(&c), [None; 4]);
        assert!(browse::read::border_sides(&c, item.id).unwrap().is_empty());
        assert_eq!(c.pending_borders().unwrap().len(), 1);
    }

    #[test]
    fn border_depth_sorts_by_the_largest_share_of_a_side_cut() {
        use crate::browse;
        use pg_model::query::{Query, Sort, SortKey};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        c.add_root(&p(r"D:\x")).unwrap();
        let scan = c.begin_scan(&p(r"D:\x")).unwrap();
        // 400 × 300 stored. b: none; c: 30 of 300 at the top (10%); d: 60 of 400 at the left (15%), but turned a
        // quarter (orientation 6), so 60 of the 400 upright rows; e: 90 at the top (30%), marked not a border.
        let files = [("b.jpg", 1, None), ("c.jpg", 1, Some([0, 30, 0, 0])), ("d.jpg", 6, Some([60, 0, 0, 0]))];
        let files = files.into_iter().chain([("e.jpg", 1, Some([0, 90, 0, 0]))]);
        for (n, (name, orientation, cut)) in files.enumerate() {
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &p(r"D:\x")).unwrap();
            let folder = writer.folder(&p(r"D:\x")).unwrap();
            let ext = Ext::of_name(name).unwrap();
            let file = ScannedFile {
                identity: FileIdentity::new(1, 10 + u64::try_from(n).unwrap()),
                name: &Name::from(name),
                ext,
                size: 10,
                mtime: 100,
                created: None,
            };
            writer.upsert_file(&folder, &file).unwrap();
            drop(writer);
            tx.commit().unwrap();
            let item = c.pending_import(false).unwrap().items.remove(0);
            let [l, t, r, b] = cut.unwrap_or([0; 4]);
            let mut borders = Detection::none(400, 300);
            borders.content = [l, t, 400 - r, 300 - b];
            let (width, height) = if Orientation::from_tag(orientation).swaps_axes() { (300, 400) } else { (400, 300) };
            let analysis = Analysis {
                version: IMPORT_VERSION,
                file_hash: [u8::try_from(n).unwrap(); 32],
                orientation: Orientation::from_tag(orientation),
                width,
                height,
                thumbhash: ThumbHash::default(),
                frames: 1,
                taken: None,
                media: a_jpeg(),
            };
            let tx = c.tx().unwrap();
            tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
            tx.borders_writer().unwrap().store_borders(&item, Some(&borders)).unwrap();
            tx.commit().unwrap();
            if name == "e.jpg" {
                c.set_border_mark(item.id, true).unwrap();
            }
        }
        let names = |descending: bool| -> Vec<String> {
            let sort = Sort { key: SortKey::Borders, descending };
            let entries = browse::read::list_grid(&c, "", true, &Query::default(), sort).unwrap();
            let path = |id| browse::read::details(&c, id).unwrap().unwrap().path;
            entries.iter().map(|e| path(e.id).rsplit('\\').next().unwrap().to_string()).collect()
        };
        // Without a border (none found, or marked) last, by name, either way.
        assert_eq!(names(false), ["c.jpg", "d.jpg", "b.jpg", "e.jpg"]);
        assert_eq!(names(true), ["d.jpg", "c.jpg", "b.jpg", "e.jpg"]);
    }

    /// Stores an import of `item` with `taken` as its time, as the import stores it.
    fn store_taken(c: &mut Catalog, item: &WorkItem, taken: Option<Taken>) {
        let analysis = Analysis {
            version: IMPORT_VERSION,
            file_hash: [1; 32],
            orientation: Orientation::UPRIGHT,
            width: 400,
            height: 300,
            thumbhash: ThumbHash::default(),
            frames: 1,
            taken,
            media: a_jpeg(),
        };
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(item, &stored_analysis(&analysis)).unwrap();
        tx.commit().unwrap();
    }

    /// `analysis.taken` and `analysis.taken_utc` of file 1.
    fn taken_columns(c: &Catalog) -> (Option<i64>, Option<i64>) {
        c.conn
            .query_row("SELECT taken, taken_utc FROM analysis WHERE file_id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
    }

    /// The local wall clock of a UTC instant in this system's zone, as SQLite converts it.
    fn local_of(c: &Catalog, instant: i64) -> i64 {
        c.conn.query_row("SELECT unixepoch(?1, 'unixepoch', 'localtime')", [instant], |r| r.get(0)).unwrap()
    }

    #[test]
    fn a_camera_clock_is_kept_as_it_is() {
        let (mut c, _) = one_file();
        let item = next_work(&c);
        store_taken(&mut c, &item, Some(Taken::Wall(1_700_000_000)));
        assert_eq!(taken_columns(&c), (Some(1_700_000_000), None));
    }

    #[test]
    fn a_utc_instant_is_converted_to_the_local_wall_clock_and_kept_as_well() {
        let (mut c, _) = one_file();
        let item = next_work(&c);
        let instant = 1_714_557_600;
        store_taken(&mut c, &item, Some(Taken::Utc(instant)));
        assert_eq!(taken_columns(&c), (Some(local_of(&c, instant)), Some(instant)));
    }

    #[test]
    fn a_changed_zone_computes_utc_taken_times_again_from_their_instants() {
        let (mut c, _) = one_file();
        let item = next_work(&c);
        let instant = 1_714_557_600;
        store_taken(&mut c, &item, Some(Taken::Utc(instant)));

        // A `taken` left from another zone, and a fingerprint no zone has (a UTC system's own is "0,0,0,0", so a
        // numeric stand-in could match it and hide the change): the next prepare recomputes it.
        c.conn.execute("UPDATE analysis SET taken = 0 WHERE file_id = 1", []).unwrap();
        c.conn.execute("UPDATE meta SET value = 'another-zone' WHERE key = 'taken_zone'", []).unwrap();
        set_taken_zone(&c.conn).unwrap();
        assert_eq!(taken_columns(&c), (Some(local_of(&c, instant)), Some(instant)));
        let current: String = c.conn.query_row(TAKEN_ZONE_QUERY, [], |r| r.get(0)).unwrap();
        let stored: String =
            c.conn.query_row("SELECT value FROM meta WHERE key = 'taken_zone'", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, current);
        assert_eq!(current.split(',').count(), 4);
    }

    #[test]
    fn a_camera_clock_is_not_changed_by_a_changed_zone() {
        let (mut c, _) = one_file();
        let item = next_work(&c);
        store_taken(&mut c, &item, Some(Taken::Wall(1_700_000_000)));
        c.conn.execute("UPDATE meta SET value = 'another-zone' WHERE key = 'taken_zone'", []).unwrap();
        set_taken_zone(&c.conn).unwrap();
        assert_eq!(taken_columns(&c), (Some(1_700_000_000), None));
    }

    #[test]
    fn prepare_records_the_zone_its_utc_times_were_computed_with() {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        let current: String = c.conn.query_row(TAKEN_ZONE_QUERY, [], |r| r.get(0)).unwrap();
        let stored: String =
            c.conn.query_row("SELECT value FROM meta WHERE key = 'taken_zone'", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, current);
    }

    #[test]
    fn pending_follows_what_the_analysis_was_made_from() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        let scan = c.begin_scan(&p(r"D:\x")).unwrap();
        // New: pending.
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let first = next_work(&c);
        let id = first.id;
        assert!(due(&c, id));
        // Analyzed as it is: done. A failure counts as done; only --retry redoes it.
        analyzed(&mut c, &first);
        assert!(!due(&c, id));
        assert!(c.pending_import(false).unwrap().is_empty());
        assert_eq!(c.pending_import(true).unwrap().len(), 1);
        // A rescan that changes nothing changes nothing, for the import or the thumbnails.
        let seq = raw::file_seq(&c, id);
        scanned(&mut c, scan, "a.jpg", 10, 100);
        assert_eq!(state(&c, id), (false, seq));
        // Changed while it was being analyzed: what's stored is of the old file.
        let old = c.pending_import(true).unwrap().items.remove(0);
        scanned(&mut c, scan, "a.jpg", 11, 101);
        analyzed(&mut c, &old);
        assert!(due(&c, id));
        let current = next_work(&c);
        analyzed(&mut c, &current);
        assert!(!due(&c, id));
        // Turned in the catalog: analyzed again with the turn, and the thumbnails' source moved on.
        turn_kept(&c, id, 1);
        let (pending, after) = state(&c, id);
        assert!(pending && after > seq);
        let turned = next_work(&c);
        assert_eq!(turned.rotation, Turns::new(1));
        analyzed(&mut c, &turned);
        // Rewritten by the app: a metadata write carries the analysis along; a pixel write leaves it stale.
        let rewritten = |c: &Catalog, before, after, metadata_only| {
            // A write that leaves what the file holds as it was (no turn), so only its size and mtime change.
            let job =
                writes::Job { file_id: id, kind: pg_model::write::WriteKind::Turns, value: 0, temp: None, attempts: 0 };
            let rewrite = writes::Rewrite { before, after, metadata_only };
            writes::done(c, &job, writes::WriteEffect::TurnedBack { rewrite, turns: Turns::NONE }).unwrap();
        };
        rewritten(&c, (11, 101), (12, 102), Some([0; 32]));
        assert!(!due(&c, id));
        rewritten(&c, (12, 102), (13, 103), None);
        assert!(due(&c, id));
        let rewritten = next_work(&c);
        analyzed(&mut c, &rewritten);
        // Renamed to another kind: analyzed again as that.
        scanned(&mut c, scan, "a.mp4", 13, 103);
        let renamed = next_work(&c);
        assert_eq!(renamed.kind, Kind::Video);
        analyzed(&mut c, &renamed);
        assert!(!due(&c, id));
        // Gone, then back: nothing to import, but the thumbnails look at it both times.
        let seq = raw::file_seq(&c, id);
        let next = c.begin_scan(&p(r"D:\x")).unwrap();
        c.mark_missing(&p(r"D:\x"), next, &[]).unwrap();
        let gone = raw::file_seq(&c, id);
        scanned(&mut c, next, "a.mp4", 13, 103);
        assert_eq!(c.pending_import(false).unwrap().len(), 0);
        assert!(seq < gone && gone < raw::file_seq(&c, id));
    }

    #[test]
    fn a_folder_the_scan_could_not_list_keeps_its_files() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        let put = |c: &mut Catalog, scan: i64, folder: &str, name: &str, file_ref: u64| {
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &p(r"D:\x")).unwrap();
            let folder = writer.folder(&p(folder)).unwrap();
            let ext = Ext::of_name(name).unwrap();
            let identity = FileIdentity::new(1, file_ref);
            writer
                .upsert_file(
                    &folder,
                    &ScannedFile { identity, name: &Name::from(name), ext, size: 1, mtime: 1, created: None },
                )
                .unwrap();
            drop(writer);
            tx.commit().unwrap();
        };
        let first = c.begin_scan(&p(r"D:\x")).unwrap();
        put(&mut c, first, r"D:\x\ok", "a.jpg", 1);
        put(&mut c, first, r"D:\x\locked", "b.jpg", 2);
        put(&mut c, first, r"D:\x\locked\deep", "c.jpg", 3);

        // The next scan lists only D:\x\ok: D:\x\locked couldn't be read, so its files weren't seen, and aren't missing.
        let next = c.begin_scan(&p(r"D:\x")).unwrap();
        put(&mut c, next, r"D:\x\ok", "a.jpg", 1);
        assert_eq!(c.mark_missing(&p(r"D:\x"), next, &[p(r"D:\x\locked")]).unwrap(), 0);
        assert_eq!(raw::missing_count(&c), 0);

        // Once it can be listed again, a scan that doesn't see its files marks them.
        let later = c.begin_scan(&p(r"D:\x")).unwrap();
        put(&mut c, later, r"D:\x\ok", "a.jpg", 1);
        assert_eq!(c.mark_missing(&p(r"D:\x"), later, &[]).unwrap(), 2);
        assert_eq!(raw::missing_count(&c), 2);
    }

    #[test]
    fn files_marked_are_imported_again_once() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        raw::add_scan(&c, 1, "D:");
        raw::add_folder(&c, 1, r"D:\x");
        raw::add_folder(&c, 2, r"D:\x\y");
        raw::add_folder(&c, 3, r"D:\xy");
        raw::add_folder(&c, 4, r"D:\x\y\z");
        let files = [(1, 1, "a.jpg"), (2, 2, "b.jpg"), (3, 3, "c.jpg"), (4, 4, "d.jpg"), (5, 4, "e.jpg")];
        let files =
            files.map(|(id, folder, name)| fixture::File::new(FileId::new(id), FolderId::new(folder), name, 10, 100));
        fixture::insert(&c, &files).unwrap();
        for item in c.pending_import(false).unwrap().items {
            analyzed(&mut c, &item);
        }
        // Analyzed, then gone.
        raw::set_missing(&c, FileId::new(5));
        assert!(c.pending_import(false).unwrap().is_empty());

        // A folder holds the files of its subfolders, not those of a folder named like it; missing ones aren't there.
        let mut under = c.files_under(&p(r"D:\x\y")).unwrap();
        under.sort();
        assert_eq!(under, [FileId::new(2), FileId::new(4)]);
        assert_eq!(c.files_under(&p(r"D:\x")).unwrap().len(), 3);
        assert!(c.files_under(&p(r"D:\q")).unwrap().is_empty());

        // Marked: due, whatever their analysis, the missing one left; the change number doesn't move.
        let seq = raw::file_seq(&c, FileId::new(2));
        assert_eq!(c.mark_pending(&[FileId::new(2), FileId::new(4), FileId::new(5)]).unwrap(), 2);
        assert_eq!(state(&c, FileId::new(2)), (true, seq));
        assert!(due(&c, FileId::new(4)) && !due(&c, FileId::new(5)) && !due(&c, FileId::new(1)));
        let work = c.pending_import(false).unwrap();
        assert_eq!(work.items.iter().map(|i| i.id).collect::<Vec<_>>(), [FileId::new(2), FileId::new(4)]);
        // Analyzed again, as they are: done.
        for item in work.items {
            analyzed(&mut c, &item);
        }
        assert!(c.pending_import(false).unwrap().is_empty());
    }

    #[test]
    fn a_new_import_version_redoes_everything() {
        let (mut c, _) = one_file();
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

    #[test]
    fn a_round_with_nothing_to_import_reads_one_index() {
        let (c, _) = one_file();
        let plan = raw::query_plan(&c, pending_import_sql(false), []);
        assert!(plan.contains("USING INDEX files_to_import") && !plan.contains("TEMP B-TREE"), "{plan}");
        let retry = raw::query_plan(&c, pending_import_sql(true), []);
        assert!(retry.contains("files_to_import") && retry.contains("analysis_failed"), "{retry}");
    }

    fn set_rating(c: &Catalog, id: i64, n: u8) {
        let stars = Rating::new(n).filter(|r| r.get() > 0);
        writes::set_rating(c, FileId::new(id), stars, false).unwrap();
    }

    /// The import's sync of the file's rating, in a transaction of its own.
    fn sync_file(c: &mut Catalog, id: i64, in_file: Option<Rating>) {
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().sync_file_rating(FileId::new(id), in_file).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn three_way_rating_sync() {
        let (mut c, id) = one_file();
        // First import of a file rated elsewhere (ImageRater, Explorer): adopted.
        sync_file(&mut c, id, Rating::new(4));
        assert_eq!(raw::rating(&c, id), Some(4));
        // Rated in the app, not written back (or the format can't hold it): the file still says 4, as known, so
        // re-imports keep the catalog's 2.
        set_rating(&c, id, 2);
        sync_file(&mut c, id, Rating::new(4));
        assert_eq!(raw::rating(&c, id), Some(2));
        // Another program changes the file's rating: that is newer, and adopted.
        sync_file(&mut c, id, Rating::new(5));
        assert_eq!(raw::rating(&c, id), Some(5));
        // Another program clears it (writes 0, or removes it): the catalog follows.
        sync_file(&mut c, id, Rating::new(0));
        assert_eq!(raw::rating(&c, id), None);
        sync_file(&mut c, id, None);
        assert_eq!(raw::rating(&c, id), None);
        // A write of the app's own still queued wins over the file: a JPEG the app rates 3, queued for the file.
        raw::insert_analysis(&c, &raw::AnalysisRow { format: Some("jpeg"), ..raw::AnalysisRow::image(id, 10, 100) });
        writes::set_rating(&c, FileId::new(id), Rating::new(3), true).unwrap();
        sync_file(&mut c, id, Rating::new(1));
        assert_eq!(raw::rating(&c, id), Some(3));
    }
}
