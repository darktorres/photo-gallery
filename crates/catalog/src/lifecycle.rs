//! The catalog file's lifecycle: making, moving aside and rebuilding it, its locks and open holds, the version stamps
//! that `Catalog::prepare` checks and sets, and the work lists the passes read. Uses `types`, `schema` and `error`; its
//! SQL macros are from `sql`.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use pg_platform::sqlite::sqlite_path;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};

use pg_model::ids::FileId;
use pg_model::media::{Ext, Kind};
use pg_model::path::WorkList;
use pg_model::rules::PathRules;
use pg_model::turns::Turns;

use crate::error::{Error, Result};
use crate::schema::{BASELINE, MIGRATIONS, SETTINGS, bring_to, carry_rows, current_version, write_transaction};
use crate::types::{FOLDER_PATH, TAKEN_ZONE_QUERY, Carried, Rebuilt, WorkItem, folder_path, rules_name};

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

/// The largest folder id of the catalog (`schema`: `""`, or `"cat."` where it is attached under that name): an
/// O(1) read of the table's last row.
pub fn max_folder_id(conn: &Connection, schema: &str) -> Result<i64> {
    let sql = format!("SELECT coalesce(max(id), -1) FROM {schema}folders");
    Ok(conn.query_row(&sql, [], |r| r.get(0))?)
}
