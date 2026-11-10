//! The cache file's layout and how it is opened: the schema this build makes, the judgement of a file that exists
//! (kept, started over, or moved aside as damaged), and the writer's connection to it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use pg_platform::sqlite::sqlite_path;
use pg_schema::Schema;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::config::{CACHE_VERSION, MAX_DEFAULT_CAP_BYTES, cache_version, default_cap_bytes};
use crate::maintenance::{Drift, heal_if_due};
use crate::types::StartOverReason;

/// A new cache file's tables, at [`CACHE_VERSION`].
const SCHEMA: &str = concat!(
    "
CREATE TABLE entries(
    id        INTEGER PRIMARY KEY,
    file_id   INTEGER NOT NULL,  -- catalog files.id
    height    INTEGER NOT NULL,  -- requested grid height (the image may be smaller)
    mtime     INTEGER NOT NULL,  -- Source: stale when either differs from the file's
    rotation  INTEGER NOT NULL,
    format    TEXT NOT NULL,
    bytes     INTEGER NOT NULL,
    last_used INTEGER NOT NULL,  -- Unix seconds, to the hour (TOUCH_AGE_S)
    UNIQUE(file_id, height));
-- Eviction reads only this index.
CREATE INDEX entries_lru ON entries(last_used, height, bytes);
-- The JPEG of entry `id`.
CREATE TABLE data(
    id   INTEGER PRIMARY KEY,
    jpeg BLOB NOT NULL);
-- Sum and count of entries, kept by the triggers in the statements that change them.
CREATE TABLE totals(
    one   INTEGER PRIMARY KEY CHECK (one = 1),
    bytes INTEGER NOT NULL,
    count INTEGER NOT NULL);
INSERT INTO totals VALUES (1, 0, 0);
-- The size limit, in bytes, for every process that writes the file (the row is made with the file).
CREATE TABLE limits(
    one INTEGER PRIMARY KEY CHECK (one = 1),
    cap INTEGER NOT NULL);
-- About the cache: `catalog_id`, the catalog's (its meta.catalog_id) whose file ids the entries use.
CREATE TABLE meta(
    key   TEXT PRIMARY KEY,
    value ANY NOT NULL);
-- Per height: the catalog's change number (files.seq) up to which every file was checked for a current entry
-- (NULL: not known), and how many times entries of that height were evicted.
CREATE TABLE checked(
    height    INTEGER PRIMARY KEY,
    seq       INTEGER,
    evictions INTEGER NOT NULL);
CREATE TRIGGER entries_added AFTER INSERT ON entries BEGIN
    UPDATE totals SET bytes = bytes + new.bytes, count = count + 1;
END;
CREATE TRIGGER entries_removed AFTER DELETE ON entries BEGIN
    UPDATE totals SET bytes = bytes - old.bytes, count = count - 1;
    DELETE FROM data WHERE id = old.id;
END;
CREATE TRIGGER entries_resized AFTER UPDATE OF bytes ON entries BEGIN
    UPDATE totals SET bytes = bytes - old.bytes + new.bytes;
END; PRAGMA user_version = ",
    cache_version!(),
    ";"
);

/// The `application_id` header field of a cache file (`PGTC`), set when the file is made: with [`SEEDS`] and the schema,
/// what tells a cache from another SQLite file.
pub(crate) const APPLICATION_ID: i64 = 0x5047_5443;

/// The rows that creation seeds and that a cache must have the keys of: `limits` and `totals`. Their values are not
/// compared: the cap is the user's. Not `meta`: the cache writes more keys there while it runs (`healed`), and its
/// `catalog_id` is checked by value on its own.
const SEEDS: &[(&str, &str)] = &[("limits", "one"), ("totals", "one")];

/// Settings for every connection to a cache file. Recursive triggers make a REPLACE fire the delete trigger
/// (this code never replaces entries; anything else writing the file might).
pub(crate) const CONNECTION_PRAGMAS: &str =
    "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA recursive_triggers = ON;";

/// How long a statement waits for another connection's lock before it fails (`busy_timeout`, on every connection to
/// the cache): the writer's transactions and the readers' pass both wait for each other, rather than failing at once.
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a file is started over, inside the cache: the reason, and the technical account of it ([`Event::StartedOver`](crate::Event::StartedOver)).
pub(crate) struct Why {
    pub(crate) reason: StartOverReason,
    pub(crate) detail: String,
}

/// What a cache file is, for a writer of catalog `catalog_id`, before anything is written to it.
enum Found {
    /// This build's cache, of this catalog: kept.
    Usable,
    /// No file, or an empty one: made.
    New,
    /// A cache this build can't use (of another catalog, another layout, or an earlier build's): started over, with
    /// the reason and the cap it had.
    Stale { why: Why, cap: Option<i64> },
    /// Not a database, or one SQLite finds damaged (`NotADatabase`, `DatabaseCorrupt`): moved aside to
    /// `<name>.bad` and made anew. Nothing in it can be trusted, its cap included, and it held derived data only.
    Damaged { why: Why },
    /// A database that is not a thumbnail cache: refused, and left as it is.
    Foreign { why: String },
}

/// Why `e` says the file is damaged, if its code is one of SQLite's for that: not a database at all
/// (`NotADatabase`), or a database whose pages don't hold together (`DatabaseCorrupt`). Any other failure (busy, no
/// permission, a full disk) is not the file's fault, and stays an error.
fn damage(e: &rusqlite::Error) -> Option<Why> {
    match e.sqlite_error_code() {
        Some(rusqlite::ErrorCode::NotADatabase) => {
            Some(Why { reason: StartOverReason::NotADatabase, detail: "it is not a database".to_string() })
        }
        Some(rusqlite::ErrorCode::DatabaseCorrupt) => {
            Some(Why { reason: StartOverReason::Damaged, detail: format!("it is damaged ({e})") })
        }
        _ => None,
    }
}

/// [`damage`] for the first SQLite error in `e`'s chain.
fn damage_in(e: &anyhow::Error) -> Option<Why> {
    e.chain().filter_map(|cause| cause.downcast_ref::<rusqlite::Error>()).find_map(damage)
}

/// The cache's schema as this build makes it, for the comparison in [`judge`]. Made in memory, once per process: the
/// catalog's id is not part of the schema (only the keys of its rows are compared, so any id will do here).
fn expected() -> Result<&'static Schema> {
    static EXPECTED: OnceLock<Schema> = OnceLock::new();
    if let Some(schema) = EXPECTED.get() {
        return Ok(schema);
    }
    let conn = Connection::open_in_memory()?;
    make_layout(&conn, "", MAX_DEFAULT_CAP_BYTES)?;
    let schema = Schema::read(&conn, SEEDS)?;
    Ok(EXPECTED.get_or_init(|| schema))
}

/// Make the cache's layout in `conn`, an empty database: its tables ([`SCHEMA`]), its application id, the cap row and the
/// catalog's id. Creating a file and making the expected schema both do this, so they can't disagree.
fn make_layout(conn: &Connection, catalog_id: &str, cap_bytes: i64) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "application_id", APPLICATION_ID)?;
    conn.execute("INSERT INTO limits VALUES (1, ?1)", [cap_bytes])?;
    conn.execute("INSERT INTO meta VALUES ('catalog_id', ?1)", [catalog_id])?;
    Ok(())
}

/// What the file at `path` is, for a writer of catalog `catalog_id`. It reads only: a file that is not ours gets no
/// pragma and no write, so it is left as it was. A read SQLite fails as damage ([`damage`]) is [`Found::Damaged`].
fn judge(path: &Path, catalog_id: &str) -> Result<Found> {
    if !path.exists() {
        return Ok(Found::New);
    }
    let expected = expected()?;
    let conn = Connection::open(sqlite_path(path)?)?;
    match read_found(&conn, catalog_id, expected) {
        Ok(found) => Ok(found),
        Err(e) => match damage(&e) {
            Some(why) => Ok(Found::Damaged { why }),
            None => Err(e.into()),
        },
    }
}

/// [`judge`]'s reads, on the file's connection `conn`: its schema compared with this build's (`expected`), its header
/// fields and its owner.
fn read_found(conn: &Connection, catalog_id: &str, expected: &Schema) -> rusqlite::Result<Found> {
    let schema = Schema::read(conn, SEEDS)?;
    if schema.is_empty() {
        return Ok(Found::New);
    }
    let id = pg_schema::application_id(conn)?;
    // A cache of an earlier build has no application id, but it has the two tables every cache has.
    let ours = id == APPLICATION_ID || (id == 0 && schema.has_table("entries") && schema.has_table("limits"));
    if !ours {
        return Ok(Found::Foreign { why: "it is not a thumbnail cache".to_string() });
    }
    // The cap a stale file had: none when an earlier layout's `limits` has no `cap` column (or no table, or no row).
    // Any other failure to read it (busy, no permission, damage) is an error, so the cap is never replaced by the
    // default because of one.
    let has_cap: i64 =
        conn.query_row("SELECT count(*) FROM pragma_table_info('limits') WHERE name = 'cap'", [], |r| r.get(0))?;
    let cap = if has_cap == 0 {
        None
    } else {
        conn.query_row("SELECT cap FROM limits", [], |r| r.get::<_, i64>(0)).optional()?
    };
    let version = pg_schema::user_version(conn)?;
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    // The owner is read last: an earlier layout may have no `meta` table at all.
    let other_build = |detail: String| Some(Why { reason: StartOverReason::OtherBuild, detail });
    let difference = if version != CACHE_VERSION {
        other_build(format!("layout version {version}, this build's is {CACHE_VERSION}"))
    } else if let Some(difference) = schema.difference(expected) {
        other_build(difference.to_string())
    } else if id != APPLICATION_ID {
        other_build(format!("application id {id}, not a cache's"))
    } else if auto_vacuum != 2 {
        other_build("auto_vacuum is not incremental, so its free pages can't be given back".to_string())
    } else {
        let owner: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key = 'catalog_id'", [], |r| r.get(0)).optional()?;
        (owner.as_deref() != Some(catalog_id))
            .then(|| Why { reason: StartOverReason::OtherCatalog, detail: "made for another catalog".to_string() })
    };
    Ok(match difference {
        None => Found::Usable,
        Some(why) => Found::Stale { why, cap },
    })
}

/// What [`open_writer`] gives the cache: the writer's connection, why the file was started over and the path of
/// [`Event::StartedOver`](crate::Event::StartedOver) (`None`: it was kept, or it is new), what the day's check of the totals fixed
/// (`heal_if_due`), and the shared hold on the file (`pg_schema::Live`), kept for as long as the connection is.
pub(crate) struct Writing {
    pub(crate) conn: Connection,
    pub(crate) started_over: Option<(Why, PathBuf)>,
    pub(crate) drift: Option<Drift>,
    pub(crate) live: pg_schema::Live,
}

/// One try at the file ([`open_once`]): opened, or found damaged (the reason), with nothing of it held.
enum Attempt {
    Opened(Writing),
    Damaged(Why),
}

/// The writer's connection to a cache of catalog `catalog_id`, with the cap set if one is given ([`Writing`]). A file
/// this build can't use (of another catalog or layout) is deleted with its sidecars and made anew, keeping its cap:
/// that needs the exclusive hold, so no other program or connection has the file open (a start over is refused
/// otherwise, and nothing is deleted). A file that is not a database, or one SQLite finds damaged when it is judged,
/// opened or has its totals checked (`heal_if_due`), is moved aside to `<name>.bad` with its sidecars
/// (`pg_schema::move_aside`, which overwrites nothing) and made anew once; it holds derived data only. A database
/// that is not a thumbnail cache is refused and left alone, and any other failure (busy, no permission, a full disk)
/// stays an error. The lock beside the file is held while it is judged, checked and changed, so another process
/// opening it finds the result.
pub(crate) fn open_writer(path: &Path, catalog_id: &str, cap_bytes: Option<i64>) -> Result<Writing> {
    let _lock = pg_schema::Lock::take(path)
        .with_context(|| format!("{}: its lock file", path.display()))?
        .ok_or_else(|| anyhow!("{}: another process is changing it", path.display()))?;
    let why = match open_once(path, catalog_id, cap_bytes)? {
        Attempt::Opened(writing) => return Ok(writing),
        Attempt::Damaged(why) => why,
    };
    let kept = move_damaged(path, &why.detail)?;
    match open_once(path, catalog_id, cap_bytes)? {
        Attempt::Opened(mut writing) => {
            let detail = format!("{}; the old file is kept as {}", why.detail, kept.display());
            writing.started_over = Some((Why { reason: why.reason, detail }, kept));
            Ok(writing)
        }
        Attempt::Damaged(again) => bail!("{}: {}, even when made anew", path.display(), again.detail),
    }
}

/// One try of [`open_writer`], under its lock: the file judged, started over if this build can't use it, opened, and
/// its totals checked. Damage found on the way is [`Attempt::Damaged`], with every connection to the file closed and
/// every hold on it let go, so the file can be moved aside.
fn open_once(path: &Path, catalog_id: &str, cap_bytes: Option<i64>) -> Result<Attempt> {
    let (usable, why, cap) = match judge(path, catalog_id)? {
        Found::Usable => (true, None, None),
        Found::New => (false, None, None),
        Found::Stale { why, cap } => (false, Some(why), cap),
        Found::Damaged { why } => return Ok(Attempt::Damaged(why)),
        Found::Foreign { why } => bail!("{}: {why}, so it is left alone", path.display()),
    };
    // Taken before anything is removed, and kept until the new file is made: the exclusive hold says nothing else has
    // the file open. Without it, the file is not touched.
    let exclusive = if why.is_some() {
        let held = pg_schema::Live::exclusive(path)
            .with_context(|| format!("{}: its open marker", path.display()))?
            .ok_or_else(|| {
                anyhow!(
                    "{}: made for another catalog or layout, and can't be started over while another program has it \
                     open",
                    path.display()
                )
            })?;
        Some(held)
    } else {
        None
    };
    if why.is_some() {
        // Sidecars first: a `-wal` left beside a new file would be replayed into it.
        for suffix in ["-wal", "-shm", "-journal", ""] {
            // The sidecar's name is the file's own units with the suffix (not its display form, which is lossy).
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            let file = PathBuf::from(name);
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => bail!(
                    "{}: made for another catalog or layout, and can't be started over while another program has it \
                     open: {e}",
                    file.display()
                ),
            }
        }
    }
    let (conn, drift) = match ready(path, catalog_id, cap_bytes.or(cap), !usable) {
        Ok(ready) => ready,
        // The connection is closed (`ready` dropped it) and the exclusive hold goes with this return.
        Err(e) => return damage_in(&e).map(Attempt::Damaged).ok_or(e),
    };
    // The file is made: the exclusive hold gives way to the shared one the connection keeps.
    drop(exclusive);
    let live = hold_shared(path)?;
    let started_over = why.map(|why| (why, path.to_path_buf()));
    Ok(Attempt::Opened(Writing { conn, started_over, drift, live }))
}

/// The writer's connection to the judged file ([`open_file`]), and what the day's check of its totals fixed. A file
/// made here (`made`) is judged again first, and must be this build's.
fn ready(path: &Path, catalog_id: &str, cap_bytes: Option<i64>, made: bool) -> Result<(Connection, Option<Drift>)> {
    let mut conn = open_file(path, catalog_id, cap_bytes)?;
    if made && !matches!(judge(path, catalog_id)?, Found::Usable) {
        bail!("{}: not usable even when made anew", path.display());
    }
    let drift = heal_if_due(&mut conn)?;
    // After the day's check, not in `open_file`: a write before it would commit to a damaged file, which would then be
    // moved aside changed. This write's failures are judged like the check's: damage moves the file aside
    // (`open_once`), any other failure is an error. The row is there: a file judged usable has it, and a stale one was
    // made anew above.
    if let Some(cap) = cap_bytes {
        conn.execute("UPDATE limits SET cap = ?1", [cap])?;
    }
    Ok((conn, drift))
}

/// Move the damaged cache at `path` aside, with its sidecars, to `<name>.bad` (or `.bad.1`, and so on): its new path.
/// Only with the exclusive hold, so no other program or connection has it open; refused otherwise, with nothing moved.
fn move_damaged(path: &Path, why: &str) -> Result<PathBuf> {
    let _exclusive =
        pg_schema::Live::exclusive(path).with_context(|| format!("{}: its open marker", path.display()))?.ok_or_else(
            || anyhow!("{}: {why}, and can't be moved aside while another program has it open", path.display()),
        )?;
    pg_schema::move_aside(path, ".bad").with_context(|| format!("{}: {why}, and can't be moved aside", path.display()))
}

/// The shared hold a cache keeps while it is open (`pg_schema::Live`): it waits for a start over in progress to finish.
pub(crate) fn hold_shared(path: &Path) -> Result<pg_schema::Live> {
    pg_schema::Live::shared(path)
        .with_context(|| format!("{}: its open marker", path.display()))?
        .ok_or_else(|| anyhow!("{}: another process is starting it over", path.display()))
}

/// A connection to the cache file, made with its layout if it is new (for catalog `catalog_id`), and the cap set if one
/// is given and the file has the table for it. The file was judged by [`judge`] first, so only a new one is made here.
fn open_file(path: &Path, catalog_id: &str, cap_bytes: Option<i64>) -> Result<Connection> {
    let mut conn = Connection::open(sqlite_path(path)?)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    // auto_vacuum must be set before the first table exists. Setting it writes page 1 even when the file already has
    // it, so it is set only when it differs: a cache judged usable has it, and its open must write nothing before the
    // day's check (which may find the file damaged and move it aside as it was).
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    if auto_vacuum != 2 {
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    }
    conn.execute_batch(CONNECTION_PRAGMAS)?;
    // The writer's pages: a 200-thumbnail batch fits in them, so its transaction doesn't go to the file for each.
    conn.execute_batch("PRAGMA cache_size = -16384;")?;
    let empty = |c: &Connection| -> Result<bool> { Ok(Schema::read(c, &[])?.is_empty()) };
    if empty(&conn)? {
        // The setting takes effect only in a file SQLite has not written: an empty file that was written (a
        // user_version) needs a VACUUM to take it.
        let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
        if auto_vacuum != 2 {
            conn.execute_batch("VACUUM;")?;
        }
        // Another process may be creating it too: decide inside the write lock.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if empty(&tx)? {
            let dir = path.parent().unwrap_or(path);
            make_layout(&tx, catalog_id, cap_bytes.unwrap_or_else(|| default_cap_bytes(dir)))?;
        }
        tx.commit()?;
    }
    Ok(conn)
}
