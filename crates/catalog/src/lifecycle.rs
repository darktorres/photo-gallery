//! The catalog file's lifecycle: making, moving aside and rebuilding it, its locks and open holds, the version stamps
//! that `Catalog::prepare` checks and sets, and the work lists the passes read. Uses `types`, `schema` and `error`; its
//! SQL macros are from `sql`.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use pg_platform::sqlite::sqlite_path;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};

use pg_model::ids::FileId;
use pg_model::media::{Ext, Kind};
use pg_model::notice::Carried;
use pg_model::path::WorkList;
use pg_model::rules::PathRules;
use pg_model::turns::Turns;

use crate::error::{Error, Result};
use crate::schema::{BASELINE, MIGRATIONS, SETTINGS, bring_to, carry_rows, current_version, write_transaction};
use crate::types::{FOLDER_PATH, Rebuilt, TAKEN_ZONE_QUERY, WorkItem, folder_path, rules_name};

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

/// A count of rows the copy made (never negative), as the notice of the rebuild holds it.
fn count(rows: i64) -> u64 {
    u64::try_from(rows).unwrap_or(0)
}

/// Where the replacement for the catalog at `path` is made before it takes the name ([`rebuild`]).
fn building_path(path: &Path) -> PathBuf {
    beside(path, ".building")
}

/// Where the copy of the catalog at `path` made before it is migrated from `version` is kept.
pub(crate) fn backup_path(path: &Path, version: i64) -> PathBuf {
    beside(path, &format!(".pre-v{version}.bak"))
}

/// Copy the catalog at `path`, at `version`, aside before it is migrated forward (the migrations can't be undone, and an
/// older build refuses a catalog of a later version: going back to it is putting the copy back). Its `-wal` is merged
/// into the file first, so the copy is whole; the caller holds the catalog exclusively. A copy already there (from an
/// earlier try of the same migration) is kept: it is the older one.
pub(crate) fn back_up(conn: &Connection, path: &Path, version: i64) -> Result<()> {
    let backup = backup_path(path, version);
    if backup.exists() {
        return Ok(());
    }
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    let partial = beside(&backup, ".partial");
    std::fs::copy(db_file(path)?, &partial)
        .and_then(|_| std::fs::rename(&partial, &backup))
        .with_context(|| format!("copying the catalog to {} before it is migrated", backup.display()))?;
    Ok(())
}

/// Make the catalog `old` (at `path`) into a new one in this build's layout, carrying its rows ([`rebuild`]): the old file
/// is moved aside, and the new one takes its name. `old` is the connection the change was found on; it is closed here.
pub(crate) fn rebuild(old: Connection, path: &Path, rules: PathRules, why: String, readable: bool) -> Result<Rebuilt> {
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
            Ok((files, folders)) => Carried::Rows { files: count(files), folders: count(folders) },
            Err(e) => {
                // The copy is one transaction, so an error leaves no rows; the replacement is made again without them.
                remove_database(&replacement)?;
                make_replacement(&replacement, rules, None)?;
                Carried::Nothing { reason: format!("{e:#}") }
            }
        },
        Err(reason) => {
            make_replacement(&replacement, rules, None)?;
            Carried::Nothing { reason }
        }
    };
    // To `catalog.sqlite.replaced`, then `.replaced.1`, and so on: kept whole, with its sidecars, for the user to
    // remove.
    let kept = pg_schema::move_aside(path, ".replaced")
        .with_context(|| format!("catalog {}: moving it aside ({why})", path.display()))?;
    std::fs::rename(&replacement, path).with_context(|| {
        format!("catalog {}: its new file {} can't take the name", path.display(), replacement.display())
    })?;
    Ok(Rebuilt { kept, why, carried })
}

/// The file SQLite opens for the catalog at `path`: the path itself, or on Windows an alias for a folder whose name has
/// no UTF-8 form (`pg_platform::sqlite`).
pub(crate) fn db_file(path: &Path) -> anyhow::Result<PathBuf> {
    sqlite_path(path).with_context(|| format!("catalog {}", path.display()))
}

/// Make the catalog file `path` in this build's layout, with the rows of `from` copied into it when given
/// ([`carry_rows`]). Returns its file and library folder counts (zero without `from`). The file is closed, with its
/// changes in the main file.
fn make_replacement(path: &Path, rules: PathRules, from: Option<&Path>) -> Result<(i64, i64)> {
    let mut conn = Connection::open(db_file(path)?).with_context(|| format!("catalog {}", path.display()))?;
    conn.execute_batch(SETTINGS)?;
    bring_to(&conn, BASELINE, MIGRATIONS, rules, current_version()?)
        .with_context(|| format!("catalog {}", path.display()))?;
    let counts = match from {
        Some(old) => {
            carry_rows(&mut conn, old, rules)?;
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
/// paths under these. `Catalog::prepare` has already made sure the schema is this build's,
/// which includes the rules row, so a missing row can only be refused here, as a file this build can't read.
pub(crate) fn check_rules(conn: &Connection, path: &Path, rules: PathRules) -> Result<()> {
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

/// The lock `Catalog::prepare` holds on the catalog (`pg_schema::Lock`): two processes
/// preparing one catalog do it one after the other, and the second finds the first's catalog current. It is not taken
/// for `:memory:`.
pub(crate) fn take_lock(path: &Path) -> Result<pg_schema::Lock> {
    pg_schema::Lock::take(path)
        .with_context(|| format!("catalog {}: its lock file", path.display()))?
        .ok_or_else(|| Error::Other(anyhow!("catalog {}: another process is preparing it", path.display())))
}

/// The exclusive hold on the catalog file (`pg_schema::Live`), when no other process or connection holds it. It doesn't
/// wait: `None` is the answer when one does.
pub(crate) fn try_exclusive(path: &Path) -> Result<Option<pg_schema::Live>> {
    Ok(pg_schema::Live::exclusive(path).with_context(|| format!("catalog {}: its open marker", path.display()))?)
}

/// The refusal of a change to a catalog that another program or connection has open. Nothing has been moved.
pub(crate) fn in_use(path: &Path) -> Error {
    Error::Other(anyhow!(
        "catalog {}: another program or connection has it open, so it can't be changed until it is closed",
        path.display()
    ))
}

/// The shared hold a catalog keeps while it is open (`pg_schema::Live`). It waits for a change in progress to finish.
pub(crate) fn take_shared(path: &Path) -> Result<pg_schema::Live> {
    pg_schema::Live::shared(path)
        .with_context(|| format!("catalog {}: its open marker", path.display()))?
        .ok_or_else(|| Error::Other(anyhow!("catalog {}: another program is changing it", path.display())))
}

/// Make `files.pending` mean this build's IMPORT_VERSION: when the catalog's `meta.import_version` is another
/// (a new catalog, or a build that analyzes differently), every file's is computed again, and every file's `seq`
/// moves on, so the thumbnail pass looks at all of them once. Checked and set in one write transaction, so two
/// processes opening the catalog at once do it once.
pub(crate) fn set_import_version(conn: &Connection, current: i64) -> Result<()> {
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

/// Mark missing the present files whose extension this build doesn't hold (`media::Ext::of`; a build that held more
/// recorded them): the number marked. The scan records only the extensions it holds, so they never come back, and
/// every read of the catalog (the grid, an entry, the tree, the facets, the status) leaves them out as it leaves out
/// any missing file, where one such row would fail the grid's list. A queued write on one is dropped as any write on
/// a missing file is. In one write transaction, at `Catalog::prepare`.
pub(crate) fn mark_unheld_missing(conn: &Connection) -> Result<usize> {
    let tx = write_transaction(conn)?;
    let exts: Vec<String> = tx
        .prepare("SELECT DISTINCT ext FROM files WHERE missing = 0")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut marked = 0;
    {
        let mut mark = tx.prepare("UPDATE files SET missing = 1 WHERE ext = ?1 AND missing = 0")?;
        for ext in exts.iter().filter(|ext| Ext::of(ext.as_str()).is_none()) {
            marked += mark.execute([ext])?;
        }
    }
    tx.commit()?;
    Ok(marked)
}

/// Make `analysis.borders_due` mean this build's detector: when the catalog's `meta.borders_version` is another,
/// every analysis that can have borders is due. Like [`set_import_version`], in one write transaction.
pub(crate) fn set_borders_version(conn: &Connection, current: i64) -> Result<()> {
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
        // Every file is looked at again by the new detector: what the old one failed on is not known to fail.
        tx.execute("DELETE FROM file_failures WHERE pass = 'borders'", [])?;
    }
    Ok(tx.commit()?)
}

/// Make the thumbnails' failure ledger this build's: when the catalog's `meta.thumbs_version` is another
/// (`pg_render::THUMBS_VERSION`, a decoder that may take what the old one failed on), the files whose thumbnail panicked
/// are listed again for the pass (the others are, by the pass's own rule), and the rows go: the next pass records what
/// still fails. The cache's checkpoint holds the panicked files back too, and keeps a mark of its own
/// (`pg_thumbcache::ThumbCache::open`). Like [`set_import_version`], in one write transaction.
pub(crate) fn set_thumbs_version(conn: &Connection, current: i64) -> Result<()> {
    let version = |c: &Connection| -> Result<i64> {
        Ok(c.query_row("SELECT value FROM meta WHERE key = 'thumbs_version'", [], |r| r.get(0))?)
    };
    if version(conn)? == current {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if version(&tx)? != current {
        tx.execute("UPDATE meta SET value = ?1 WHERE key = 'thumbs_version'", [current])?;
        tx.execute_batch(next_seq!())?;
        tx.execute(failures_relisted!(" AND retry = 0"), ["thumbs"])?;
        tx.execute("DELETE FROM file_failures WHERE pass = 'thumbs'", [])?;
    }
    Ok(tx.commit()?)
}

/// Make `analysis.taken` follow this system's zone: when `meta.taken_zone` is another fingerprint (the zone changed,
/// or the catalog was made under another), every UTC-sourced `taken` is computed again from its `taken_utc` instant.
/// Like [`set_import_version`], in one write transaction. The stored instants are what make this possible: the wall
/// clock alone would lose the instant of a repeated hour.
pub(crate) fn set_taken_zone(conn: &Connection) -> Result<()> {
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

/// `Catalog::pending_import`'s query.
pub(crate) fn pending_import_sql(retry_errors: bool) -> &'static str {
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
pub(crate) fn work_list(conn: &Connection, stmt: &mut rusqlite::Statement, bound: usize) -> Result<WorkList<WorkItem>> {
    let mut rows = stmt.query([])?;
    let (mut list, mut folder) = (WorkList::default(), None);
    while let Some(r) = rows.next()? {
        if folder.is_none() {
            // Only once there is work: a round with nothing to do reads nothing more.
            list = WorkList::with_capacity(bound, max_folder_id(conn)?);
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
            kind: crate::types::kind_at(r, 6)?,
        });
    }
    Ok(list)
}

/// The largest folder id of the catalog (-1 for none): an O(1) read of the table's last row. The thumbnail cache reads
/// the same through the `thumb_folders` view.
fn max_folder_id(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT coalesce(max(id), -1) FROM folders", [], |r| r.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::{backup_path, pending_import_sql, set_taken_zone};
    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::{self, raw};
    use crate::schema::{BASELINE, MIGRATIONS, bring_to, current_expected, current_version, mismatch};
    use crate::test_support::*;
    use crate::types::TAKEN_ZONE_QUERY;
    use crate::types::rules_name;
    use pg_model::notice::Carried;

    use crate::{Catalog, Error, Result, ScannedFile};
    use pg_analysis::analyze::IMPORT_VERSION;
    use pg_model::analysis::Taken;
    use pg_model::ids::{FileId, FileIdentity, FolderId, RootId};
    use pg_model::media::Ext;
    use pg_model::name::Name;
    use pg_model::rules::PathRules;
    use pg_testkit::test_folder;
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

    // The layout tests below read the file on disk through their own connection (`raw::on_disk`): what they check
    // is the layout, not the catalog's tables.
    /// A catalog that isn't there is an error to open, and opening it makes no marker beside where it would be.
    #[test]
    fn opening_a_catalog_that_is_not_there_makes_no_marker() {
        let dir = test_folder(SCRATCH, "not-there").unwrap();
        let path = dir.join("catalog.sqlite");
        assert!(Catalog::open(&path, pg_platform::native::PATH_RULES).is_err());
        assert!(!dir.join("catalog.sqlite.live").exists(), "no marker for a catalog that isn't there");
    }

    /// A catalog another connection has open is not moved aside: its rebuild is refused, with the file as it was, and it
    /// is rebuilt once that connection has closed. The edit to the file is made on a raw connection, which holds nothing.
    #[test]
    fn a_catalog_another_connection_has_open_is_not_moved_aside() {
        let dir = test_folder(SCRATCH, "open-elsewhere").unwrap();
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

        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
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
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A catalog that differs from this build's schema only by an index it shouldn't have is rebuilt with its rows: the
    /// same files under the same ids, and the library folders.
    fn a_catalog_that_differs_keeps_its_rows(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("keeps-rows-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        raw::add_scan(&c, 1, &spelled(rules, r"D:\p"));
        raw::add_folder(&c, 1, &spelled(rules, r"D:\p"));
        raw::add_folder(&c, 2, &spelled(rules, r"D:\p\Sub"));
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
        std::fs::remove_dir_all(&dir).unwrap();
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
    fn a_catalog_in_a_folder_with_a_name_that_is_not_utf8_keeps_its_rows() {
        use std::os::unix::ffi::OsStringExt;
        let base = test_folder(SCRATCH, "odd-folder-name").unwrap();
        let dir = base.join(std::ffi::OsString::from_vec(b"odd-\xFF".to_vec()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("catalog.sqlite");
        let rules = PathRules::Posix;
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        raw::add_scan(&c, 1, "/p");
        raw::add_folder(&c, 1, "/p");
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

    /// A catalog from before the folders were typed kept its saved views' folders and the session's as text, and its
    /// session as text keys. They are carried typed (a view's stored bytes; the session's one row), so they read as
    /// they were saved.
    // Windows-only: before the folders were typed, the views and the session held backslash-separated Windows text,
    // and that text is what this carries.
    #[test]
    fn text_folders_of_views_and_session_are_carried_typed() {
        let dir = test_folder(SCRATCH, "text-folders").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        drop(c);
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            r"DROP TABLE views;
              CREATE TABLE views(name TEXT PRIMARY KEY, folder TEXT NOT NULL, recursive INTEGER NOT NULL,
                                 query TEXT NOT NULL, sort TEXT NOT NULL);
              INSERT INTO views VALUES ('old', 'pinterest\a', 1, '{}', '{}');
              DROP TABLE session;
              CREATE TABLE session(key TEXT PRIMARY KEY, value TEXT NOT NULL);
              INSERT INTO session VALUES ('folder', 'pinterest\a'), ('recursive', 'false');",
        )
        .unwrap();
        drop(old);
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        let r = c.rebuilt().expect("moved aside");
        assert_eq!(r.carried, Carried::Rows { files: 0, folders: 0 });
        let saved = crate::browse::read::views(&c).unwrap();
        assert_eq!(saved.len(), 1);
        let folder = pg_model::rules::Merged::from(vec![Name::from("pinterest"), Name::from("a")]);
        assert_eq!(saved[0].listing.folder, folder);
        let session = crate::browse::read::session_listing(&c).unwrap().expect("carried");
        assert_eq!((session.folder, session.recursive), (folder, false));
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A catalog from before the queued writes' temps were bytes keeps them: a copy that was finished when the app
    /// stopped is still the job it was, with its temp at the same path.
    #[test]
    fn a_queued_temp_from_before_it_was_bytes_is_carried_and_read_back() {
        let dir = test_folder(SCRATCH, "text-temps").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        raw::add_scan(&c, 1, r"D:\p");
        raw::add_folder(&c, 1, r"D:\p");
        fixture::insert(&c, &[fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 1, 5)]).unwrap();
        drop(c);
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
                  PRIMARY KEY(file_id, kind));
              INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value)
                  VALUES (1, 'rating', 3, 0, 'D:\p\.a.jpg.pg-write', 3);",
        )
        .unwrap();
        drop(old);
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        assert!(c.rebuilt().is_some(), "moved aside");
        let job = crate::writes::next_ready(&c, 0).unwrap().expect("the queued copy");
        let temp = PathRules::Windows.parse(r"D:\p\.a.jpg.pg-write").unwrap();
        assert_eq!(job.temp, Some((temp, 3)));
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// On Windows, a catalog in a folder whose name has no UTF-8 form is opened, and rebuilt with its rows carried (the
    /// carry attaches the old file by name): every SQLite call goes through the folder's alias (`pg_platform::sqlite`).
    #[cfg(windows)]
    #[test]
    fn a_catalog_in_a_folder_without_a_utf8_name_is_opened_and_carried_through_an_alias() {
        let dir = test_folder(SCRATCH, "odd-folder").unwrap();
        let folder = dir.join(pg_platform::names::os_name(&pg_model::name::Name::from_raw(b"cat\xED\xA0\x80")));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap());
        drop(Catalog::open(&path, pg_platform::native::PATH_RULES).unwrap());
        // The layout of before the queued temps were bytes: the rebuild carries the old file's rows, attached by name.
        let old = Connection::open(pg_platform::sqlite::sqlite_path(&path).unwrap()).unwrap();
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
        assert_eq!(r.carried, Carried::Nothing { reason: "made with the windows path rules".to_string() });
        assert!(c.roots().unwrap().is_empty());
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("layout_source-{}", rules_name(rules))).unwrap();
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
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside,
        a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside_windows,
        a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside_posix
    );

    fn a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("move-aside-twice-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // Files without the columns the current layout needs.
        let old_layout = |path: &Path| {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch(&format!(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value ANY NOT NULL); INSERT INTO meta VALUES ('path_rules', '{}');
                 CREATE TABLE files(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO files VALUES (1, 'x');
                 PRAGMA user_version = 1;",
                rules_name(rules)
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
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name,
        a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name_windows,
        a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name_posix
    );

    fn an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("move-aside-no-rules-{}", rules_name(rules))).unwrap();
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
        let c = Catalog::prepare(&path, versions(), rules).expect("rebuilt, not refused");
        drop(c);
        let moved = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(moved.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(moved);
        // The new catalog records its rules, so it opens again.
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
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
        assert_eq!(rules.display(&c.file_path(FileId::new(2)).unwrap().unwrap()), "/photos/Pics/b.jpg");
        // Folders that differ only in case are two folders, and their files are two files.
        assert_eq!(c.file_by_path(&root("/photos/Pics/b.jpg")).unwrap(), Some(FileId::new(2)));
        assert_eq!(c.file_by_path(&root("/photos/pics/b.jpg")).unwrap(), Some(FileId::new(3)));
        assert_eq!(c.files_under(&root("/photos/Pics")).unwrap().len(), 1);
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
    fn edited(rules: PathRules, path: &Path, sql: &str) {
        drop(Catalog::prepare(path, versions(), rules).unwrap());
        raw::on_disk(path).execute_batch(sql).unwrap();
    }

    fn preparing_a_catalog_again_rebuilds_nothing(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("prepare-twice-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        let again = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(again.rebuilt().is_none());
        drop(again);
        assert!(!dir.join("catalog.sqlite.replaced").exists());
        std::fs::remove_dir_all(&dir).unwrap();
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
            let dir = test_folder(SCRATCH, &format!("{name}-{}", rules_name(rules))).unwrap();
            let path = dir.join("catalog.sqlite");
            edited(rules, &path, sql);
            let c = Catalog::prepare(&path, versions(), rules).unwrap();
            let why = rebuilt_why(&c).unwrap_or_else(|| panic!("{name}: not moved aside"));
            assert!(why.contains(named), "{name}: {why}");
            assert!(is_current(&raw::on_disk(&path)).unwrap());
            drop(c);
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    under_both_rules!(
        a_changed_definition_with_the_same_names_is_moved_aside,
        a_changed_definition_with_the_same_names_is_moved_aside_windows,
        a_changed_definition_with_the_same_names_is_moved_aside_posix
    );

    fn a_comment_in_a_stored_definition_is_not_a_schema_change(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("comment-edit-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // A catalog whose `files` table was written with a comment in it: the same schema.
        let commented = BASELINE.replacen("CREATE TABLE files(", "CREATE TABLE files( -- an added comment\n", 1);
        assert_ne!(commented, BASELINE);
        let conn = Connection::open(&path).unwrap();
        bring_to(&conn, &commented, MIGRATIONS, rules, current_version().unwrap()).unwrap();
        drop(conn);
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(c.rebuilt().is_none(), "a comment moved the catalog aside");
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_comment_in_a_stored_definition_is_not_a_schema_change,
        a_comment_in_a_stored_definition_is_not_a_schema_change_windows,
        a_comment_in_a_stored_definition_is_not_a_schema_change_posix
    );

    fn a_catalog_at_version_one_is_migrated_forward_and_keeps_its_rows(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("migrate-v1-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // A catalog as the build before the first migration made it: the baseline, with a library folder.
        {
            let conn = Connection::open(&path).unwrap();
            bring_to(&conn, BASELINE, MIGRATIONS, rules, crate::schema::BASELINE_VERSION).unwrap();
            let root = rules.join(&p(rules, r"D:\kept"));
            conn.execute("INSERT INTO roots(id, path, added) VALUES (7, ?1, 1)", [root]).unwrap();
        }
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(c.rebuilt().is_none(), "migrated, not rebuilt");
        assert!(!dir.join("catalog.sqlite.replaced").exists());
        assert_eq!(pg_schema::user_version(&c.conn).unwrap(), current_version().unwrap());
        assert!(is_current(&c.conn).unwrap());
        let roots = c.roots().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!((roots[0].id, &roots[0].path), (RootId::new(7), &p(rules, r"D:\kept")));
        // The file as it was before the migration is kept beside it, once: an older build can be gone back to.
        let backup = backup_path(&path, crate::schema::BASELINE_VERSION);
        let old = Connection::open(&backup).unwrap();
        assert_eq!(pg_schema::user_version(&old).unwrap(), crate::schema::BASELINE_VERSION);
        assert_eq!(old.query_row("SELECT count(*) FROM roots", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        drop(old);
        drop(c);
        let kept = std::fs::metadata(&backup).unwrap().modified().unwrap();
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        assert_eq!(std::fs::metadata(&backup).unwrap().modified().unwrap(), kept);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file whose extension this build doesn't hold (a build that held more recorded it) is marked missing when the
    /// catalog is prepared, so the grid lists the rest, and every count leaves it out.
    fn a_file_of_an_extension_not_held_is_missing_after_prepare(rules: PathRules) {
        use crate::browse::read::{facets, folder_tree, list_grid};
        use pg_model::query::{Query, Sort};

        let dir = test_folder(SCRATCH, &format!("unheld-ext-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        {
            let mut c = Catalog::prepare(&path, versions(), rules).unwrap();
            c.add_root(&p(rules, r"D:\x")).unwrap();
            let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
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
                };
                writer.upsert_file(&folder, &file).unwrap();
                drop(writer);
                tx.commit().unwrap();
            }
            c.conn.execute("UPDATE files SET ext = 'xyz' WHERE name = ?1", [b"b.jpg".to_vec()]).unwrap();
        }
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let missing: i64 = c.conn.query_row("SELECT missing FROM files WHERE ext = 'xyz'", [], |r| r.get(0)).unwrap();
        assert_eq!(missing, 1);
        let all = pg_model::rules::Merged::default();
        assert_eq!(list_grid(&c, &all, true, &Query::default(), Sort::default()).unwrap().len(), 1);
        let status = c.status().unwrap();
        assert_eq!((status.files, status.missing), (1, 1));
        assert_eq!(folder_tree(&c).unwrap().total(), 1);
        let counts = facets(&c, &all, true, &Query::default()).unwrap();
        assert_eq!(counts.total, 1);
        assert!(counts.exts.iter().all(|(ext, _)| ext != "xyz"), "{:?}", counts.exts);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_file_of_an_extension_not_held_is_missing_after_prepare,
        a_file_of_an_extension_not_held_is_missing_after_prepare_windows,
        a_file_of_an_extension_not_held_is_missing_after_prepare_posix
    );

    under_both_rules!(
        a_catalog_at_version_one_is_migrated_forward_and_keeps_its_rows,
        a_catalog_at_version_one_is_migrated_forward_and_keeps_its_rows_windows,
        a_catalog_at_version_one_is_migrated_forward_and_keeps_its_rows_posix
    );

    fn a_database_that_is_not_a_catalog_is_moved_aside_not_built_on(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("foreign-database-{}", rules_name(rules))).unwrap();
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
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_database_that_is_not_a_catalog_is_moved_aside_not_built_on,
        a_database_that_is_not_a_catalog_is_moved_aside_not_built_on_windows,
        a_database_that_is_not_a_catalog_is_moved_aside_not_built_on_posix
    );

    fn a_file_that_is_not_a_database_is_moved_aside_unread(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("not-a-database-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        std::fs::write(&path, vec![b'x'; 8192]).unwrap();
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert_eq!(rebuilt_why(&c).as_deref(), Some("not a database"));
        assert_eq!(std::fs::read(dir.join("catalog.sqlite.replaced")).unwrap(), vec![b'x'; 8192]);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_file_that_is_not_a_database_is_moved_aside_unread,
        a_file_that_is_not_a_database_is_moved_aside_unread_windows,
        a_file_that_is_not_a_database_is_moved_aside_unread_posix
    );

    fn the_same_schema_under_another_application_id_is_moved_aside(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("application-id-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        raw::on_disk(&path).pragma_update(None, "application_id", 7).unwrap();
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert_eq!(rebuilt_why(&c).as_deref(), Some("application id 7, not a catalog's"));
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        the_same_schema_under_another_application_id_is_moved_aside,
        the_same_schema_under_another_application_id_is_moved_aside_windows,
        the_same_schema_under_another_application_id_is_moved_aside_posix
    );

    fn open_refuses_a_catalog_changed_after_it_was_prepared(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("changed-after-prepare-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        raw::on_disk(&path).execute_batch("CREATE TABLE extra(x INTEGER);").unwrap();
        let e = Catalog::open(&path, rules).err().expect("refused");
        assert!(e.to_string().contains("extra"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        open_refuses_a_catalog_changed_after_it_was_prepared,
        open_refuses_a_catalog_changed_after_it_was_prepared_windows,
        open_refuses_a_catalog_changed_after_it_was_prepared_posix
    );

    fn two_preparations_of_one_catalog_move_it_aside_once(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("concurrent-prepare-{}", rules_name(rules))).unwrap();
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
        std::fs::remove_dir_all(&dir).unwrap();
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

    fn a_changed_zone_computes_utc_taken_times_again_from_their_instants(rules: PathRules) {
        let (mut c, _) = one_file(rules);
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

    under_both_rules!(
        a_changed_zone_computes_utc_taken_times_again_from_their_instants,
        a_changed_zone_computes_utc_taken_times_again_from_their_instants_windows,
        a_changed_zone_computes_utc_taken_times_again_from_their_instants_posix
    );

    fn a_camera_clock_is_not_changed_by_a_changed_zone(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        store_taken(&mut c, &item, Some(Taken::Wall(1_700_000_000)));
        c.conn.execute("UPDATE meta SET value = 'another-zone' WHERE key = 'taken_zone'", []).unwrap();
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
        let plan = raw::query_plan(&c, pending_import_sql(false), []);
        assert!(plan.contains("USING INDEX files_to_import") && !plan.contains("TEMP B-TREE"), "{plan}");
        let retry = raw::query_plan(&c, pending_import_sql(true), []);
        assert!(retry.contains("files_to_import") && retry.contains("analysis_failed"), "{retry}");
    }

    under_both_rules!(
        a_round_with_nothing_to_import_reads_one_index,
        a_round_with_nothing_to_import_reads_one_index_windows,
        a_round_with_nothing_to_import_reads_one_index_posix
    );
}
