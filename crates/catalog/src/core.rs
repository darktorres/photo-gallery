//! The catalog's lifecycle: `Catalog` prepared, opened and closed, its transaction `Tx` and the writers it hands out,
//! and a walk's lease (`ScanLease`, which `Catalog::begin_scan` takes). Uses `lifecycle`, `rebuild`, `writers`,
//! `lease`, `progress`, `classify`, `schema`, `transaction`, `types` and `error`. Every other concern's methods are in its own module's `impl Catalog`
//! (`roots`, `lookup`, `work`, `borders`, `browse`, `queue`, `dups`, `pair`, `inbox`, `ledger`); this calls none of
//! them.

use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use pg_dbfile::Schema;
use pg_model::ids::ScanId;
use pg_model::rules::{Path as RulePath, PathRules};

use crate::classify::{Action, Facts, LONGEST_NAME_SUFFIX, RELEASED, classify};
use crate::error::{Error, Result};
use crate::lease::HeldLease;
use crate::lifecycle::{
    check_rules, ensure_room_to_migrate, in_use, mark_unheld_missing, meta_text, newer_mark, refuse_read_only,
    set_borders_version, set_import_version, set_key_version, set_pairing_version, set_taken_zone, set_thumbs_version,
    take_lock, take_shared, try_exclusive, vacuum_if_owed,
};
use crate::progress::{FreeSpace, NO_PROGRESS, Phase, Progress};
use crate::rebuild::{back_up, backup_path, db_file, rebuild};
use crate::schema::{
    Hooks, SCHEMA, SchemaSet, apply_settings, bring_to, current_expected, current_version, expected_for, mismatch,
    version_in, version_of,
};
use crate::transaction::{WriteTx, write_transaction};
use crate::types::{PREPARE_CACHE_KIB, Rebuilt, Versions, key_version_text};
use crate::writers::{AnalysisWriter, BordersWriter, LedgerWriter, PairsWriter, PlaceholderWriter, ScanWriter};
use pg_model::refusal::CatalogRefusal as Refusal;

/// An open catalog: its connection, the path rules its paths are stored under, and the hold on the file while it is open.
pub struct Catalog {
    pub(crate) conn: Connection,
    /// The path rules the catalog's paths are stored under (`meta.path_rules`, checked when it opens).
    pub(crate) rules: PathRules,
    /// The catalog this build moved aside to make this one, when [`Catalog::prepare`] made one.
    rebuilt: Option<Rebuilt>,
    /// The shared hold on the file (`pg_dbfile::Live`) for as long as the catalog is open: a rebuild elsewhere waits
    /// for it to go. `None` for `:memory:`. Declared after `conn`, so it is released after the connection closes.
    _live: Option<pg_dbfile::Live>,
    /// How many PASSIVE checkpoints this catalog has run ([`Catalog::checkpoint_passive`]).
    passive: AtomicU64,
}

impl Catalog {
    /// A number that changes when another connection (in this process or another) commits a change to the catalog:
    /// what tells a poll that something it reads may have changed. Read outside a transaction.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }

    /// Merge the WAL into the file, then close the connection (on drop, too, without the merge). The last connection
    /// to a file to close deletes its WAL; the merge keeps the file whole if the process ends before the others close.
    /// A merge that another connection's pages kept from finishing is an error ([`Error::WalNotMerged`]).
    pub fn close(self) -> Result<()> {
        checkpoint_truncate(&self.conn)
    }

    /// Move the WAL's finished pages into the file, without waiting for readers (PASSIVE). The thread that wrote the WAL
    /// calls this at the end of its work (a lane's round, a removal's deletes, the writeback's drain), so no writer's commit
    /// pays for the checkpoint (`SETTINGS` turns the automatic one off). A reader in the way is not an error: the pages it
    /// holds are moved by a later checkpoint.
    pub fn checkpoint_passive(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)")?;
        self.passive.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// How many PASSIVE checkpoints this catalog has run. The writers' call sites are checked through it: a lane's round,
    /// a removal's deletes and a writeback drain each run one.
    pub fn passive_checkpoints(&self) -> u64 {
        self.passive.load(Ordering::SeqCst)
    }

    /// The WAL file's size in bytes: `0` for an in-memory catalog, which has none.
    pub fn wal_bytes(&self) -> Result<u64> {
        let Some(path) = self.conn.path().filter(|p| !p.is_empty()) else { return Ok(0) };
        Ok(std::fs::metadata(format!("{path}-wal")).map(|m| m.len()).unwrap_or(0))
    }

    /// A PASSIVE checkpoint when the WAL is over `bytes`; `true` when it ran. For a writer that is not one of the three
    /// threads above and so has no round or drain to checkpoint at its end (the session's saves): its writes are bounded
    /// by this, and the checkpoint is not a commit's.
    pub fn checkpoint_if_wal_over(&self, bytes: u64) -> Result<bool> {
        if self.wal_bytes()? <= bytes {
            return Ok(false);
        }
        self.checkpoint_passive()?;
        Ok(true)
    }

    /// Prepare the catalog at `path` for this build, creating it when new. A file that is not this build's catalog is
    /// judged before anything is written to it: its schema (every object, and the keys its creation seeds) must be the
    /// one this build makes for its version. One that isn't (an earlier layout, another build's, or not a database at
    /// all) is rebuilt: a new catalog in this build's layout is made beside it, the old rows are copied into it with
    /// their ids (`carry_rows`), and the old file is moved aside (`pg_dbfile::move_aside`; see [`Catalog::rebuilt`]). A
    /// row the copy can't take (a column the new layout needs that the old one lacks) fails the copy, and the new catalog
    /// is made empty, with the reason said. A catalog at a version after this build's is refused (made by another build),
    /// and so is one whose integer mark (`import_version`, `borders_version`, `pairing_version`, `thumbs_version`) is above
    /// this build's: a later build touched it, and an older one does not set its marks back. Both refusals are made before
    /// anything is written. So a downgrade is a backup restored by hand, the only way back to an older build's catalog.
    /// Its version marks are set to `versions`. The library calls this once, before anything else opens the catalog. A
    /// change to the file (a rebuild, or a migration) needs no other process or connection to have it open: otherwise
    /// it fails with the catalog untouched, and the one that has it open is left as it is.
    pub fn prepare(path: &Path, versions: Versions, rules: PathRules) -> Result<Catalog> {
        Catalog::preparing(path, versions, rules).run()
    }

    /// A preparation of the catalog at `path`, to be configured and then run ([`Prepare::run`]): the progress it reports,
    /// the schema it is brought to, and the free space its volume is taken to have. [`Catalog::prepare`] is this with the
    /// defaults.
    pub fn preparing(path: &Path, versions: Versions, rules: PathRules) -> Prepare<'_> {
        Prepare { path, versions, rules, schemas: None, progress: &NO_PROGRESS, free_space: None }
    }
}

/// Merges the whole write-ahead log of `conn` into its file and empties the log (`TRUNCATE`). SQLite reports a merge that
/// a reader's pages kept from finishing as a result row, not an error, so the row is read: a merge that did not finish is
/// [`Error::WalNotMerged`], and the file is whole regardless (its changes stay in the log).
pub(crate) fn checkpoint_truncate(conn: &Connection) -> Result<()> {
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    if busy != 0 {
        let path = conn.path().unwrap_or_default().to_string();
        return Err(Error::WalNotMerged { path });
    }
    Ok(())
}

/// A preparation of a catalog ([`Catalog::preparing`]): the work [`Catalog::prepare`] does, with its options.
pub struct Prepare<'a> {
    path: &'a Path,
    versions: Versions,
    rules: PathRules,
    /// The schema the catalog is brought to; the build's own when `None`.
    schemas: Option<&'a SchemaSet<'a>>,
    progress: &'a dyn Progress,
    free_space: Option<&'a FreeSpace<'a>>,
}

impl<'a> Prepare<'a> {
    /// Reports the preparation's progress to `progress` (report only: see [`Progress`]).
    pub fn progress(mut self, progress: &'a dyn Progress) -> Self {
        self.progress = progress;
        self
    }

    /// Takes the free bytes of the catalog's volume from `space`, in place of the volume's own report. A test's room for a
    /// migration.
    pub fn free_space(mut self, space: &'a FreeSpace<'a>) -> Self {
        self.free_space = Some(space);
        self
    }

    /// Brings the catalog to `set`, rather than the build's own schema. Test-only until the crash tests need it
    /// from outside the crate, which is when `SchemaSet` becomes public behind the `fixture` feature.
    #[cfg(test)]
    pub(crate) fn schemas(mut self, set: &'a SchemaSet<'a>) -> Self {
        self.schemas = Some(set);
        self
    }

    /// Prepares the catalog as [`Catalog::prepare`] describes.
    pub fn run(self) -> Result<Catalog> {
        let Prepare { path, versions, rules, schemas, progress, free_space } = self;
        progress.phase(Phase::Check);
        let set = schemas.unwrap_or(&SCHEMA);
        let memory = path == Path::new(":memory:");
        // A path too long for its files is refused before the lock file or the database is named: SQLite would fail on it
        // with no words for why. It is a fact of the path, not of the file, so it is not `classify`'s.
        if !memory && !pg_platform::sqlite::fits_beside(path, LONGEST_NAME_SUFFIX) {
            return Err(Error::Refused { path: path.display().to_string(), reason: Box::new(Refusal::PathTooLong) });
        }
        let _lock = if memory { None } else { Some(take_lock(path)?) };
        let current = version_in(set)?;
        let mut conn = Connection::open(db_file(path)?)
            .map_err(|e| Error::step(format!("catalog {}: opening it", path.display()), e))?;
        // The functions before anything reads or writes the files: the name index (`files_mn`) is on one, so the schema's
        // creation, a migration, a key recompute and every write of a file row call it.
        register(&conn, rules)?;
        let version = version_of(&conn)?;
        // The build's current schema is built once per process; a custom one, and an earlier version, are built here.
        let custom;
        let current_schema: &Schema = match schemas {
            None => current_expected()?,
            Some(_) => {
                custom = expected_for(set, current)?;
                &custom
            }
        };
        // How the file's schema compares with the one this build expects for its version. A later build's schema has none
        // here, and a file that is not a database has no version: `classify` decides those, not this comparison.
        let drift = match version {
            None => None,
            Some(found) if found > current => None,
            Some(found) if found == current => mismatch(&conn, current_schema)?,
            Some(found) => mismatch(&conn, &expected_for(set, found)?)?,
        }
        .map(|m| m.to_string());
        // The lock order of a change: the exclusive hold first, kept through the change. Without it a change is refused
        // before anything is moved; a catalog that needs no change is held shared, below, as any open catalog is.
        // A catalog whose `key_version` mark is another's (or missing) has its keys recomputed under the lock: a change.
        let key_text = key_version_text();
        let stored_key_version = meta_text(&conn, "key_version");
        // What `classify` decides from the file's facts. A refusal is made before anything is written; a drift is carried
        // only while nothing is released, and only from a catalog whose `key_version` says which keys its rows have.
        let facts = Facts {
            version,
            current,
            drift: drift.as_deref(),
            has_key_version: stored_key_version.is_some(),
            newer_mark: newer_mark(&conn, versions),
        };
        let action = match classify(&facts, RELEASED) {
            Action::Refuse(refusal) => return Err(refused(path, &conn, refusal)),
            action => action,
        };
        // A catalog of this build's schema whose paths were stored under other rules is refused before anything is
        // written to it: its migrations would convert its rows under these rules (a rebuild checks them itself).
        if matches!(action, Action::Open | Action::Upgrade { .. }) {
            check_rules(&conn, path, rules)?;
        }
        let why = match &action {
            Action::MoveAsideThenCreate => Some("not a catalog".to_string()),
            Action::RebuildCarry { why } => Some(why.clone()),
            _ => None,
        };
        let key_stale =
            version.is_some_and(|found| found > 0) && stored_key_version.as_deref() != Some(key_text.as_str());
        let changes = why.is_some() || key_stale || version.is_some_and(|found| found != current);
        // A catalog marked read-only is not changed: refused by name before the hold is taken (the lock file may be made
        // beside it, but nothing of the catalog is).
        if changes && !memory && std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
            return Err(refuse_read_only(path));
        }
        let exclusive = if memory { None } else { try_exclusive(path)? };
        if changes && !memory && exclusive.is_none() {
            return Err(in_use(path));
        }
        let mut rebuilt = None;
        if let Some(why) = why {
            // The replacement is written beside the catalog before the old file is moved aside: a volume without the room
            // for it is refused first, so the catalog is left as it is.
            ensure_room_to_migrate(path, free_space)?;
            rebuilt = Some(rebuild(conn, path, set, rules, why, version.is_some())?);
            conn = Connection::open(db_file(path)?)
                .map_err(|e| Error::step(format!("catalog {}: reopening it after its rebuild", path.display()), e))?;
            register(&conn, rules)?;
        }
        // An empty file takes incremental auto-vacuum before its first table (where it is a page-1 change, at once).
        if version == Some(0) && rebuilt.is_none() {
            conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        }
        apply_settings(&conn)?;
        // What follows is one-off work over whole tables (the copy aside, a migration, its `VACUUM`, a mark's
        // recompute), which reads each page once or twice: on a small cache, so it does not fill the cache with the whole
        // file (a migration's peak commit was 438 MiB with it). `SETTINGS` restores the connection's at the end.
        conn.pragma_update(None, "cache_size", -PREPARE_CACHE_KIB)?;
        // A catalog of this build's schema at an earlier version is copied aside before it is migrated (a rebuilt one
        // was: the old file is kept as it was).
        let mut upgraded_from = None;
        let upgrade = match action {
            Action::Upgrade { from } if !memory => Some(from),
            _ => None,
        };
        if let Some(found) = upgrade {
            ensure_room_to_migrate(path, free_space)?;
            progress.phase(Phase::Backup);
            back_up(&conn, path, found)?;
            pg_log::info("catalog", &upgrade_notice(found, current, &backup_path(path, found)));
            upgraded_from = Some(found);
        }
        bring_to(&conn, set, rules, current, &Hooks::reporting(progress)).map_err(|e| {
            let what = match upgraded_from {
                Some(found) => format!(
                    "catalog {}: the upgrade from version {found} failed; the catalog as it was is kept at {}",
                    path.display(),
                    backup_path(path, found).display()
                ),
                None => format!("catalog {}: bringing it to this build's schema", path.display()),
            };
            Error::step(what, e)
        })?;
        if let Some(why) = mismatch(&conn, current_schema)? {
            return Err(refused(path, &conn, Refusal::Drift { found: current, why: why.to_string() }));
        }
        check_rules(&conn, path, rules)?;
        // The keys follow this build's key algorithm and toolchain (a copy carried the old file's mark; a change of it
        // recomputes them, under the exclusive hold taken above).
        set_key_version(&conn, rules)
            .map_err(|e| Error::step(format!("catalog {}: its key version", path.display()), e))?;
        // The one-time compaction (incremental auto-vacuum's conversion) only under the exclusive hold: it writes the whole
        // file, and a start that holds it shared must not make every other process wait for it.
        vacuum_if_owed(&conn, (!memory).then_some(path), exclusive.is_some() || memory)?;
        mark_unheld_missing(&conn)
            .map_err(|e| Error::step(format!("catalog {}: its missing files", path.display()), e))?;
        // The marks are this build's or older (a later build's were refused above, before anything was written).
        let context = || format!("catalog {}: its version marks", path.display());
        set_import_version(&conn, versions.import).map_err(|e| Error::step(context(), e))?;
        set_borders_version(&conn, versions.borders).map_err(|e| Error::step(context(), e))?;
        set_pairing_version(&conn, versions.pairing).map_err(|e| Error::step(context(), e))?;
        set_thumbs_version(&conn, versions.thumbs, crate::ledger::failure::relist_thumbs_panics)
            .map_err(|e| Error::step(context(), e))?;
        set_taken_zone(&conn).map_err(|e| Error::step(format!("catalog {}: its taken zone", path.display()), e))?;
        // The build that last changed the catalog is named by it, so a later refusal can say which build to open it with.
        if changes {
            conn.execute("UPDATE meta SET value = ?1 WHERE key = 'written_by'", [env!("CARGO_PKG_VERSION")]).map_err(
                |e| Error::step(format!("catalog {}: recording the build that changed it", path.display()), e),
            )?;
        }
        apply_settings(&conn)?;
        conn.release_memory()?;
        // The change is made: the catalog's own shared hold takes the exclusive one's place (no other holder can have
        // come in meanwhile, since the preparer's lock is still held).
        drop(exclusive);
        let live = if memory { None } else { Some(take_shared(path)?) };
        Ok(Catalog { conn, rules, rebuilt, _live: live, passive: AtomicU64::new(0) })
    }
}

/// The error for a file `prepare` refuses (`classify`'s reason). A newer schema or mark names the build that last changed
/// the file (`meta.written_by`), so the user knows which build to open it with; nothing suggests deleting the file.
fn refused(path: &Path, conn: &Connection, refusal: Refusal) -> Error {
    let refusal = match refusal {
        Refusal::NewerSchema { found, current, .. } => {
            Refusal::NewerSchema { found, current, written_by: meta_text(conn, "written_by") }
        }
        Refusal::NewerMark { mark, found, current, .. } => {
            Refusal::NewerMark { mark, found, current, written_by: meta_text(conn, "written_by") }
        }
        other => other,
    };
    Error::Refused { path: path.display().to_string(), reason: Box::new(refusal) }
}

impl Catalog {
    /// Open a catalog this process has prepared ([`Catalog::prepare`], which makes it, brings it to this build's schema
    /// and sets the build's version marks): the settings and the SQL functions. Every connection to it (the app's
    /// threads, the background job, the writer) runs this, so the check is the schema's itself, against the cached
    /// expected schema, and is not repeated for the marks. An error if the catalog isn't there, or its schema isn't this
    /// build's (another program changed it since it was prepared).
    pub fn open(path: &Path, rules: PathRules) -> Result<Catalog> {
        // Held before the file is looked at: a rebuild in progress is waited for, not read half-made.
        // A catalog that isn't there is the open's error below, and no marker is made for it.
        let live = if path == Path::new(":memory:") || !path.exists() { None } else { Some(take_shared(path)?) };
        let flags = OpenFlags::default() - OpenFlags::SQLITE_OPEN_CREATE;
        let conn = Connection::open_with_flags(db_file(path)?, flags)
            .map_err(|e| Error::step(format!("catalog {}: opening it", path.display()), e))?;
        apply_settings(&conn)?;
        let current = current_version()?;
        let version = pg_dbfile::user_version(&conn)?;
        // The conditions `prepare` refuses by, each with its one text (`Refusal`): this build's schema is opened as it is.
        if version > current {
            return Err(refused(path, &conn, Refusal::NewerSchema { found: version, current, written_by: None }));
        }
        if version < current {
            return Err(refused(path, &conn, Refusal::OlderSchema { found: version, current }));
        }
        if let Some(why) = mismatch(&conn, current_expected()?)? {
            return Err(refused(path, &conn, Refusal::Drift { found: version, why: why.to_string() }));
        }
        check_rules(&conn, path, rules)?;
        register(&conn, rules)?;
        Ok(Catalog { conn, rules, rebuilt: None, _live: live, passive: AtomicU64::new(0) })
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
        Ok(self.conn.progress_handler(1000, Some(move || cancel.load(Ordering::SeqCst)))?)
    }

    /// Give back this connection's page cache (the reader does it after each list it made).
    pub fn release_memory(&self) -> Result<()> {
        Ok(self.conn.release_memory()?)
    }

    /// Set this connection's page cache to `kib` KiB. Every connection starts with a small one (`SETTINGS`); the grid
    /// reader's raises it, since its reads touch most of the catalog, and it gives the pages back after each read.
    pub fn set_cache_kib(&self, kib: u32) -> Result<()> {
        Ok(self.conn.pragma_update(None, "cache_size", -i64::from(kib))?)
    }

    /// How long a statement on this connection waits for a lock another connection holds before it fails with
    /// `SQLITE_BUSY` ([`Error::is_busy`]). Every connection starts at 5 s (`crate::schema::BUSY_TIMEOUT_MS`); a writer
    /// that retries on its own, and checks for a stop between tries, sets a shorter one.
    pub fn set_busy_timeout(&self, timeout: Duration) -> Result<()> {
        Ok(self.conn.busy_timeout(timeout)?)
    }

    /// A connection of its own for one long read ([`Reader`]), whose page cache is at most `cache_kib` KiB and goes when
    /// it is dropped: a read of a whole table (the pairing pass's) leaves neither its pages in this connection's cache
    /// (the catalog's own, `SETTINGS`) nor the cache in the process. It opens the same file read-only, so it needs no
    /// check of the schema (this connection's open made it), and this catalog's shared hold on the file keeps a rebuild
    /// out. An in-memory catalog (the tests') has no file another connection could open: its reads run on this one.
    pub fn reader(&self, cache_kib: u32) -> Result<Reader<'_>> {
        let Some(file) = self.file() else {
            return Ok(Reader { catalog: self, own: None, cache_kib });
        };
        Ok(Reader { catalog: self, own: Some(read_only(file, self.rules, cache_kib)?), cache_kib })
    }

    /// The catalog's file, `None` in memory: what a [`crate::FolderRowReader`] opens.
    pub fn file(&self) -> Option<&str> {
        self.conn.path().filter(|f| !f.is_empty())
    }

    /// Bound this connection's page cache to `kib` KiB until the guard is dropped, which restores the cache's size and
    /// gives back the pages over it (`sqlite3_db_release_memory`), however the holder leaves (an error, a stop, a panic).
    /// The pairing pass's writes read every owed analysis through this connection, which would otherwise fill its cache
    /// and keep it filled after the pass (the app keeps its catalog open across passes).
    pub fn bounded_cache(&mut self, kib: u32) -> Result<BoundedCache<'_>> {
        let app: i64 = self.conn.query_row("PRAGMA cache_size", [], |r| r.get(0))?;
        self.conn.pragma_update(None, "cache_size", -i64::from(kib))?;
        Ok(BoundedCache { catalog: self, app })
    }
}

/// What the log says of a one-time upgrade: the versions, and the copy of the old file that is kept.
pub(crate) fn upgrade_notice(from: i64, to: i64, copy: &Path) -> String {
    format!("Updating the catalog from version {from} to {to}, one time. The old file is kept at {}.", copy.display())
}

/// What every connection of a catalog has besides its settings: the SQL functions the queries call (the name key is the
/// catalog's `rules`'), and `rarray`, which binds a list of ids as one parameter (the pairing pass's writes,
/// `PairsWriter`).
pub(crate) fn register(conn: &Connection, rules: PathRules) -> Result<()> {
    pg_query::register_functions(conn, rules)?;
    rusqlite::vtab::array::load_module(conn)?;
    // The scan's writer and the import's statements are prepared once and reused across batches (`prepare_cached`), so
    // the cache holds all of them with room to spare (rusqlite's default is 16).
    conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE);
    Ok(())
}

/// The prepared statements a connection keeps for reuse (`prepare_cached`).
const STATEMENT_CACHE: usize = 64;

/// A read-only connection to the catalog's `file` with a cache of `cache_kib` KiB: a [`Reader`]'s. It has the catalog's
/// functions (its reads seek and order by the name key), under the catalog's `rules`.
pub(crate) fn read_only(file: &str, rules: PathRules, cache_kib: u32) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn =
        Connection::open_with_flags(file, flags).map_err(|e| Error::step(format!("catalog {file}: opening it"), e))?;
    conn.busy_timeout(Duration::from_millis(crate::schema::BUSY_TIMEOUT_MS))?;
    conn.pragma_update(None, "cache_size", -i64::from(cache_kib))?;
    register(&conn, rules)?;
    Ok(conn)
}

/// A read of the catalog on a connection of its own ([`Catalog::reader`]), or on the catalog's when it is in memory.
pub struct Reader<'c> {
    catalog: &'c Catalog,
    /// The read-only connection; `None` for an in-memory catalog, whose reads run on its own connection.
    own: Option<Connection>,
    /// Its cache, in KiB: the budget a read on several connections shares ([`Reader::opener`]).
    cache_kib: u32,
}

impl Reader<'_> {
    /// The connection the reads run on.
    pub(crate) fn conn(&self) -> &Connection {
        self.own.as_ref().unwrap_or(&self.catalog.conn)
    }

    /// What opens more read-only connections to the file, for a read split over `n`: each gets an `n`th of this
    /// reader's cache, so the read's connections share its budget. `None` in memory, where there is one connection.
    pub(crate) fn opener(&self, n: usize) -> Option<ReadOnly> {
        let file = self.own.as_ref().and(self.catalog.file())?;
        let share = self.cache_kib / u32::try_from(n.max(1)).unwrap_or(u32::MAX);
        Some(ReadOnly { file: file.to_owned(), rules: self.catalog.rules, cache_kib: share.max(1) })
    }
}

/// Opens read-only connections to a catalog's file ([`Reader::opener`]), on any thread.
pub(crate) struct ReadOnly {
    file: String,
    rules: PathRules,
    cache_kib: u32,
}

impl ReadOnly {
    pub(crate) fn open(&self) -> Result<Connection> {
        read_only(&self.file, self.rules, self.cache_kib)
    }
}

/// The catalog with its connection's page cache bounded ([`Catalog::bounded_cache`]); used as the catalog, through
/// `Deref`. Dropping it restores the cache.
pub struct BoundedCache<'c> {
    catalog: &'c mut Catalog,
    /// The cache's size before (`PRAGMA cache_size`: pages, or KiB when negative).
    app: i64,
}

impl Deref for BoundedCache<'_> {
    type Target = Catalog;

    fn deref(&self) -> &Catalog {
        &*self.catalog
    }
}

impl DerefMut for BoundedCache<'_> {
    fn deref_mut(&mut self) -> &mut Catalog {
        &mut *self.catalog
    }
}

impl Drop for BoundedCache<'_> {
    /// Nothing to report from a drop: a pragma that fails leaves the cache bounded, which costs only speed.
    fn drop(&mut self) {
        let _ = self.catalog.conn.pragma_update(None, "cache_size", self.app);
        let _ = self.catalog.conn.release_memory();
    }
}

/// A transaction on the catalog. It hands out the writers of the scan, the import, the borders and the failure
/// ledger, so a batch of their writes commits together, with `commit`.
pub struct Tx<'c> {
    inner: WriteTx<'c>,
    rules: PathRules,
}

impl Catalog {
    /// Begin a transaction on the catalog (see [`Tx`]).
    pub fn tx(&mut self) -> Result<Tx<'_>> {
        Ok(Tx { inner: write_transaction(&self.conn)?, rules: self.rules })
    }
}

impl Tx<'_> {
    /// The writer of the scan `scan` of `root`, in this transaction. Making it renews the walk's lease (the heartbeat
    /// of each batch the walk writes), which must still be the walk's ([`Error::RootRemoved`], [`Error::LeaseLost`]),
    /// and resolves the library folder that holds `root`, once, for the folder ledger.
    pub fn scan_writer(&self, scan: ScanId, root: &RulePath) -> Result<ScanWriter<'_>> {
        ScanWriter::new(&self.inner, scan, self.rules, root)
    }

    /// The writer of the failure ledger, in this transaction: what a batch failed on is stored with what it made, or
    /// neither.
    pub fn ledger_writer(&self) -> LedgerWriter<'_> {
        LedgerWriter::new(&self.inner)
    }

    /// The writer of the analyses, in this transaction. `import` is the build's IMPORT_VERSION (`Versions::import`),
    /// which the failures it stores are made against.
    pub fn analysis_writer(&self, import: i64) -> Result<AnalysisWriter<'_>> {
        AnalysisWriter::new(&self.inner, import)
    }

    /// The writer of the placeholders, in this transaction.
    pub fn placeholder_writer(&self) -> Result<PlaceholderWriter<'_>> {
        PlaceholderWriter::new(&self.inner)
    }

    /// The writer of the borders, in this transaction.
    pub fn borders_writer(&self) -> Result<BordersWriter<'_>> {
        BordersWriter::new(&self.inner)
    }

    /// The writer of the pairing pass, in this transaction.
    pub fn pairs_writer(&self) -> Result<PairsWriter<'_>> {
        Ok(PairsWriter::new(&self.inner))
    }

    /// Commit what was written through this transaction's writers.
    pub fn commit(self) -> Result<()> {
        Ok(self.inner.commit()?)
    }
}

/// A walk's lease of the folder it walks, which [`Catalog::begin_scan`] takes: while it holds, no other walk of a
/// folder that overlaps it begins (one is inside the other), in this process or another, so two walks never interleave
/// their writes. The walk renews it with each write transaction (`Tx::scan_writer`, the pairing's batches and the
/// missing marks', [`ScanLease::pair_after_scan`] and [`ScanLease::mark_missing`]), and with [`ScanLease::renew`] while
/// it waits; a lease with no heartbeat for `SCAN_LEASE_SECS` is taken to be a crashed walk's, and expires. Removing
/// the library folder takes the leases of the walks under it (`Catalog::remove_root`). Each of the walk's write
/// transactions begins with the heartbeat, which must find the lease: one that doesn't writes nothing, and fails as
/// [`Error::RootRemoved`] or [`Error::LeaseLost`].
///
/// It is released at the end of a complete walk, in [`ScanLease::mark_missing`]'s last transaction, or by
/// [`ScanLease::release`], and in any case when it is dropped (a stop, an error or a panic leaves through here). It
/// borrows the catalog and is used as it: the walk reads and writes through the lease.
pub struct ScanLease<'c> {
    catalog: &'c mut Catalog,
    scan: ScanId,
    /// The folder walked.
    root: RulePath,
}

impl<'c> ScanLease<'c> {
    pub(crate) fn new(catalog: &'c mut Catalog, scan: ScanId, root: RulePath) -> ScanLease<'c> {
        ScanLease { catalog, scan, root }
    }

    /// The walk's `scans` row, which its writes are stamped with.
    pub fn id(&self) -> ScanId {
        self.scan
    }

    /// The folder the walk walks.
    pub fn root(&self) -> &RulePath {
        &self.root
    }

    /// The walk is alive: its heartbeat is now. One short write, outside any transaction of the walk's. A lease that
    /// is gone (its folder removed from the library, or expired and taken by another walk) is
    /// [`Error::RootRemoved`] or [`Error::LeaseLost`]: the walk can no longer tell that it is alone.
    pub fn renew(&self) -> Result<()> {
        self.held().hold(&self.catalog.conn)
    }

    /// The check of this lease that each of the walk's write transactions begins with ([`HeldLease::hold`]).
    pub(crate) fn held(&self) -> HeldLease<'_> {
        HeldLease { scan: self.scan, rules: self.catalog.rules, root: &self.root }
    }

    /// Give the lease back now, with the error if it can't be (dropping it gives it back too, but says nothing).
    pub fn release(self) -> Result<()> {
        release_lease(&self.catalog.conn, self.scan)
    }
}

impl Deref for ScanLease<'_> {
    type Target = Catalog;

    fn deref(&self) -> &Catalog {
        &*self.catalog
    }
}

impl DerefMut for ScanLease<'_> {
    fn deref_mut(&mut self) -> &mut Catalog {
        &mut *self.catalog
    }
}

impl Drop for ScanLease<'_> {
    /// A lease that can't be given back now (the catalog locked past the busy timeout) expires in `SCAN_LEASE_SECS`.
    fn drop(&mut self) {
        let _ = release_lease(&self.catalog.conn, self.scan);
    }
}

/// Give back the lease of walk `scan`, on `conn`; nothing is written when it is gone already (a complete walk gave it
/// back in its last transaction).
pub(crate) fn release_lease(conn: &Connection, scan: ScanId) -> Result<()> {
    // The walk's rows seen go with its lease: the next walk starts with none.
    conn.execute("DELETE FROM temp.scan_seen", [])?;
    let held: bool = conn
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM scan_leases WHERE id = ?1)")?
        .query_row([scan.get()], |r| r.get(0))?;
    if held {
        conn.prepare_cached("DELETE FROM scan_leases WHERE id = ?1")?.execute([scan.get()])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Catalog;
    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::{self, raw};
    use crate::test_support::*;
    use crate::{Error, ScannedFile, queue};
    use pg_model::ids::{FileId, FileIdentity, FolderId};
    use pg_model::media::{Ext, Kind};
    use pg_model::name::Name;
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    /// The one-time upgrade's log line says both versions and where the old file is kept.
    #[test]
    fn the_upgrade_notice_names_both_versions_and_the_copy() {
        let text = super::upgrade_notice(18, 20, Path::new(r"D:\c\catalog.sqlite.pre-v18.bak"));
        assert!(text.contains("from version 18 to 20"), "{text}");
        assert!(text.contains("one time"), "{text}");
        assert!(text.contains("catalog.sqlite.pre-v18.bak"), "{text}");
    }

    /// The bundled SQLite is built with the photo gallery's defaults (the fork's `photo-gallery-defaults` feature): no
    /// process-wide memory statistics, and a lookaside of 200 slots of 1200 bytes.
    #[test]
    fn sqlite_is_built_with_the_photo_gallery_defaults() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let options: Vec<String> = conn
            .prepare("PRAGMA compile_options")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(options.iter().any(|o| o == "DEFAULT_MEMSTATUS=0"), "{options:?}");
        // SQLite prints this option's value with a trailing comma (its `CTIMEOPT_VAL` for a value with a comma).
        assert!(options.iter().any(|o| o.trim_end_matches(',') == "DEFAULT_LOOKASIDE=1200,200"), "{options:?}");
    }

    fn roots_never_nest(rules: PathRules) {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        assert!(c.add_root(&p(rules, r"D:\a\b")).unwrap());
        assert!(c.add_root(&p(rules, r"D:\a\bc")).unwrap());
        // The same folder spelled another way: on Windows another drive letter and case, which the root covers. POSIX
        // names are case-sensitive, so there the spelling is the same path.
        let same_folder = match rules {
            PathRules::Windows => r"d:\A\b\c",
            PathRules::Posix => "/a/b/c",
        };
        assert!(!c.add_root(&p(rules, same_folder)).unwrap(), "inside an existing root");
        assert!(c.add_root(&p(rules, r"D:\a")).unwrap(), "replaces both");
        let paths =
            |c: &Catalog| c.roots().unwrap().into_iter().map(|r| c.rules().display(&r.path)).collect::<Vec<_>>();
        assert_eq!(paths(&c), [spelled(rules, r"D:\a")]);
        let id = c.root_id(&p(rules, r"D:\a")).unwrap().expect("a library folder has an id");
        assert_eq!(c.remove_root(id).unwrap(), Some(0));
        assert!(paths(&c).is_empty());
        assert_eq!(c.remove_root(id).unwrap(), None, "not a library folder any more");
    }

    under_both_rules!(roots_never_nest, roots_never_nest_windows, roots_never_nest_posix);

    fn cancelled_until_cleared(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        c.cancel_when(cancel.clone()).unwrap();
        let run = || raw::slow_count(&c).map_err(Error::from);
        assert_eq!(run().unwrap(), 100_000);
        // Set between statements (SQLite's own interrupt would be lost there): every statement fails.
        cancel.store(true, Ordering::SeqCst);
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        cancel.store(false, Ordering::SeqCst);
        assert_eq!(run().unwrap(), 100_000);
    }

    under_both_rules!(cancelled_until_cleared, cancelled_until_cleared_windows, cancelled_until_cleared_posix);

    fn pending_follows_what_the_analysis_was_made_from(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let scan = begin(&mut c, &p(rules, r"D:\x"));
        // New: pending.
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let first = next_work(&c);
        let id = first.id;
        assert!(due(&c, id));
        // Analyzed as it is: done. A failure counts as done; only the user's Retry redoes it.
        analyzed(&mut c, &first);
        assert!(!due(&c, id));
        assert!(c.pending_import(&crate::VolumeSet::All).unwrap().is_empty());
        // A rescan that changes nothing changes nothing, for the import or the thumbnails.
        let seq = raw::file_seq(&c, id);
        scanned(&mut c, scan, "a.jpg", 10, 100);
        assert_eq!(state(&c, id), (false, seq));
        // Changed while it was being analyzed: what's stored is of the old file.
        let old = crate::work::work_item(&c.conn, id).unwrap().unwrap();
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
        assert_eq!(turned.turns, Turns::new(1));
        analyzed(&mut c, &turned);
        // Rewritten by the app: a metadata write carries the analysis along; a pixel write leaves it stale.
        let rewritten = |c: &Catalog, before, after, metadata_only| {
            // A write that leaves what the file holds as it was (no turn), so only its size and mtime change.
            let write = pg_model::write::Write::Rotation(Turns::NONE);
            let job = queue::Job { file_id: id, write, temp: None, attempts: 0 };
            let rewrite = queue::Rewrite { before: Some(before), after, metadata_only };
            c.write_done(&job, queue::WriteEffect::TurnedBack { rewrite, turns: Turns::NONE }).unwrap();
        };
        rewritten(&c, (11, 101), (12, 102), Some(pg_model::analysis::FileHash([0; 32])));
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
        // Gone (a complete scan that didn't see it), then back (the next scan): nothing to import, but the thumbnails
        // look at it both times.
        let seq = raw::file_seq(&c, id);
        let next = begin(&mut c, &p(rules, r"D:\x"));
        lease(&mut c, next, &p(rules, r"D:\x")).mark_missing(&[]).unwrap();
        let gone = raw::file_seq(&c, id);
        let back = begin(&mut c, &p(rules, r"D:\x"));
        scanned(&mut c, back, "a.mp4", 13, 103);
        assert_eq!(c.pending_import(&crate::VolumeSet::All).unwrap().len(), 0);
        assert!(seq < gone && gone < raw::file_seq(&c, id));
    }

    under_both_rules!(
        pending_follows_what_the_analysis_was_made_from,
        pending_follows_what_the_analysis_was_made_from_windows,
        pending_follows_what_the_analysis_was_made_from_posix
    );

    fn a_folder_the_scan_could_not_list_keeps_its_files(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let put = |c: &mut Catalog, scan: pg_model::ids::ScanId, folder: &str, name: &str, file_ref: u64| {
            let identity = FileIdentity::new(1, file_ref);
            let prior = listed(c, &p(rules, folder), &[identity]).remove(0);
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
            let folder = writer.folder(&p(rules, folder)).unwrap();
            let ext = Ext::of_name(name).unwrap();
            let file = ScannedFile { identity, name: &Name::from(name), ext, size: 1, mtime: 1, created: None, prior };
            writer.record_file(&folder, &file).unwrap();
            drop(writer);
            tx.commit().unwrap();
        };
        let first = begin(&mut c, &p(rules, r"D:\x"));
        put(&mut c, first, r"D:\x\ok", "a.jpg", 1);
        put(&mut c, first, r"D:\x\locked", "b.jpg", 2);
        put(&mut c, first, r"D:\x\locked\deep", "c.jpg", 3);

        // The next scan lists only D:\x\ok: D:\x\locked couldn't be read, so its files weren't seen, and aren't missing.
        let next = begin(&mut c, &p(rules, r"D:\x"));
        put(&mut c, next, r"D:\x\ok", "a.jpg", 1);
        let locked = crate::Skip { path: p(rules, r"D:\x\locked"), reason: crate::SkipReason::Unreadable };
        assert_eq!(lease(&mut c, next, &p(rules, r"D:\x")).mark_missing(&[locked]).unwrap(), 0);
        assert_eq!(raw::missing_count(&c), 0);

        // Once it can be listed again, a scan that doesn't see its files marks them.
        let later = begin(&mut c, &p(rules, r"D:\x"));
        put(&mut c, later, r"D:\x\ok", "a.jpg", 1);
        assert_eq!(lease(&mut c, later, &p(rules, r"D:\x")).mark_missing(&[]).unwrap(), 2);
        assert_eq!(raw::missing_count(&c), 2);
    }

    under_both_rules!(
        a_folder_the_scan_could_not_list_keeps_its_files,
        a_folder_the_scan_could_not_list_keeps_its_files_windows,
        a_folder_the_scan_could_not_list_keeps_its_files_posix
    );

    fn files_marked_are_imported_again_once(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        crate::fixture::state::add_scan(&c, 1, &spelled(rules, "D:")).unwrap();
        crate::fixture::state::add_folder(&c, 1, &spelled(rules, r"D:\x")).unwrap();
        crate::fixture::state::add_folder(&c, 2, &spelled(rules, r"D:\x\y")).unwrap();
        crate::fixture::state::add_folder(&c, 3, &spelled(rules, r"D:\xy")).unwrap();
        crate::fixture::state::add_folder(&c, 4, &spelled(rules, r"D:\x\y\z")).unwrap();
        let files = [(1, 1, "a.jpg"), (2, 2, "b.jpg"), (3, 3, "c.jpg"), (4, 4, "d.jpg"), (5, 4, "e.jpg")];
        let files =
            files.map(|(id, folder, name)| fixture::File::new(FileId::new(id), FolderId::new(folder), name, 10, 100));
        fixture::insert(&c, &files).unwrap();
        for item in c.pending_import(&crate::VolumeSet::All).unwrap().items {
            analyzed(&mut c, &item);
        }
        // Analyzed, then gone.
        raw::set_missing(&c, FileId::new(5));
        assert!(c.pending_import(&crate::VolumeSet::All).unwrap().is_empty());

        // A folder holds the files of its subfolders, not those of a folder named like it; missing ones aren't there.
        let mut under = c.files_under(&p(rules, r"D:\x\y")).unwrap();
        under.sort();
        assert_eq!(under, [FileId::new(2), FileId::new(4)]);
        assert_eq!(c.files_under(&p(rules, r"D:\x")).unwrap().len(), 3);
        assert!(c.files_under(&p(rules, r"D:\q")).unwrap().is_empty());

        // Marked: due, whatever their analysis, the missing one left; the change number doesn't move.
        let seq = raw::file_seq(&c, FileId::new(2));
        assert_eq!(c.mark_pending(&[FileId::new(2), FileId::new(4), FileId::new(5)]).unwrap(), 2);
        assert_eq!(state(&c, FileId::new(2)), (true, seq));
        assert!(due(&c, FileId::new(4)) && !due(&c, FileId::new(5)) && !due(&c, FileId::new(1)));
        let work = c.pending_import(&crate::VolumeSet::All).unwrap();
        assert_eq!(work.items.iter().map(|i| i.id).collect::<Vec<_>>(), [FileId::new(2), FileId::new(4)]);
        // Analyzed again, as they are: done.
        for item in work.items {
            analyzed(&mut c, &item);
        }
        assert!(c.pending_import(&crate::VolumeSet::All).unwrap().is_empty());
    }

    under_both_rules!(
        files_marked_are_imported_again_once,
        files_marked_are_imported_again_once_windows,
        files_marked_are_imported_again_once_posix
    );

    /// The cache size of a connection, in KiB (`PRAGMA cache_size` as the catalog sets it: negative).
    fn cache_kib(conn: &rusqlite::Connection) -> i64 {
        -conn.query_row("PRAGMA cache_size", [], |r| r.get::<_, i64>(0)).unwrap()
    }

    /// A PASSIVE checkpoint leaves the frames a reader's snapshot still needs in the WAL, and is not an error: the frames
    /// written after the snapshot stay until the reader ends, and the next checkpoint moves them.
    #[test]
    fn a_passive_checkpoint_leaves_the_frames_a_reader_holds() {
        let dir = pg_testkit::run_folder("scratch_catalog", "passive-reader-holds").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap();
        let reader = rusqlite::Connection::open(&path).unwrap();
        reader.execute_batch("BEGIN; SELECT count(*) FROM meta").unwrap();
        for i in 0..200 {
            c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'taken_zone'", [format!("zone {i}")]).unwrap();
        }
        c.checkpoint_passive().unwrap();
        let (log, done): (i64, i64) =
            c.conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| Ok((r.get(1)?, r.get(2)?))).unwrap();
        assert!(log > 0, "the writes are in the WAL");
        assert!(done < log, "the frames after the reader's snapshot stay in the WAL: {done} of {log}");
        reader.execute_batch("COMMIT").unwrap();
        drop(reader);
        c.checkpoint_passive().unwrap();
        assert_eq!(c.passive_checkpoints(), 2, "each checkpoint call is counted");
        let (log, done): (i64, i64) =
            c.conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| Ok((r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(done, log, "with the reader gone, every frame moves");
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A close with a reader holding pages of the file does not merge them all, and says so: the merge is an error, and
    /// the changes stay in the WAL.
    #[test]
    fn a_close_with_a_reader_in_the_way_is_an_error_not_a_merge() {
        let dir = pg_testkit::run_folder("scratch_catalog", "close-reader-in-the-way").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap();
        c.set_busy_timeout(Duration::ZERO).unwrap();
        let reader = rusqlite::Connection::open(&path).unwrap();
        reader.execute_batch("BEGIN; SELECT count(*) FROM meta").unwrap();
        for i in 0..200 {
            c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'taken_zone'", [format!("zone {i}")]).unwrap();
        }
        let err = c.close().unwrap_err();
        assert!(matches!(err, Error::WalNotMerged { .. }), "{err}");
        reader.execute_batch("COMMIT").unwrap();
        drop(reader);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A writer with no round or drain checkpoints its WAL only when it is over a bound (`checkpoint_if_wal_over`): under
    /// it nothing runs, over it a PASSIVE checkpoint runs and moves the frames.
    #[test]
    fn a_wal_over_the_bound_is_checkpointed_and_one_under_it_is_not() {
        let dir = pg_testkit::run_folder("scratch_catalog", "wal-bound").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap();
        assert_eq!(
            Catalog::prepare(Path::new(":memory:"), versions(), pg_platform::native::PATH_RULES)
                .unwrap()
                .wal_bytes()
                .unwrap(),
            0
        );
        for i in 0..200 {
            c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'taken_zone'", [format!("zone {i}")]).unwrap();
        }
        assert!(c.wal_bytes().unwrap() > 0, "the writes are in the WAL");
        assert!(!c.checkpoint_if_wal_over(u64::MAX).unwrap(), "under the bound: nothing runs");
        assert_eq!(c.passive_checkpoints(), 0);
        assert!(c.checkpoint_if_wal_over(0).unwrap(), "over the bound: a checkpoint runs");
        assert_eq!(c.passive_checkpoints(), 1);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The last connection to a catalog file to close leaves no WAL behind, even with no checkpoint point reached: SQLite
    /// checkpoints the file and deletes the WAL when it closes. So a writer that never reaches a checkpoint point (its
    /// round, its drain) leaves nothing after the process closes the catalog (section 11).
    #[test]
    fn the_last_connection_to_close_leaves_no_wal() {
        let dir = pg_testkit::run_folder("scratch_catalog", "close-leaves-no-wal").unwrap();
        let path = dir.join("catalog.sqlite");
        let wal = dir.join("catalog.sqlite-wal");
        let c = Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap();
        for i in 0..200 {
            c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'taken_zone'", [format!("zone {i}")]).unwrap();
        }
        let before = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert!(before > 0, "the writes are in the WAL");
        drop(c);
        let after = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert_eq!(after, 0, "closing the last connection removed the WAL ({before} bytes before)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A catalog connection does not checkpoint on a commit (`SETTINGS`): the WAL grows until the thread that wrote it
    /// checkpoints at the end of its work.
    #[test]
    fn a_catalog_connection_does_not_checkpoint_on_a_commit() {
        let dir = pg_testkit::run_folder("scratch_catalog", "no-autocheckpoint").unwrap();
        let c = Catalog::prepare(&dir.join("catalog.sqlite"), versions(), pg_platform::native::PATH_RULES).unwrap();
        let auto: i64 = c.conn.query_row("PRAGMA wal_autocheckpoint", [], |r| r.get(0)).unwrap();
        assert_eq!(auto, 0);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A PASSIVE checkpoint moves the frames no reader holds, and a reader in the way is not an error.
    #[test]
    fn a_passive_checkpoint_moves_the_wal_and_a_reader_in_the_way_is_no_error() {
        let dir = pg_testkit::run_folder("scratch_catalog", "passive-checkpoint").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), pg_platform::native::PATH_RULES).unwrap();
        for i in 0..200 {
            c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'taken_zone'", [format!("zone {i}")]).unwrap();
        }
        let reader = rusqlite::Connection::open(&path).unwrap();
        reader.execute_batch("BEGIN; SELECT count(*) FROM meta").unwrap();
        c.checkpoint_passive().unwrap();
        reader.execute_batch("COMMIT").unwrap();
        drop(reader);
        c.checkpoint_passive().unwrap();
        // Still in the WAL until the next write resets it, so a further PASSIVE checkpoint reports them all as done.
        let (busy, log, done): (i64, i64, i64) =
            c.conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(busy, 0);
        assert!(log > 0, "the writes are in the WAL");
        assert_eq!(done, log, "the checkpoint moved every frame");
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file catalog's reader is a connection of its own: read-only, its cache bounded, and it sees what was committed
    /// before it opened. An in-memory catalog's reads run on its own connection (another one would be another, empty
    /// database).
    #[test]
    fn a_reader_reads_the_catalog_file_on_a_bounded_read_only_connection() {
        let rules = pg_platform::native::PATH_RULES;
        let dir = pg_testkit::run_folder("scratch_catalog", "reader").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        c.conn.execute("UPDATE meta SET value = 'just before' WHERE key = 'taken_zone'", []).unwrap();
        let reader = c.reader(1024).unwrap();
        assert!(reader.own.is_some());
        let zone: String =
            reader.conn().query_row("SELECT value FROM meta WHERE key = 'taken_zone'", [], |r| r.get(0)).unwrap();
        assert_eq!(zone, "just before");
        assert_eq!(cache_kib(reader.conn()), 1024);
        assert_eq!(cache_kib(&c.conn), 8_192, "the catalog's own cache is left as it is: a writer's, small");
        c.set_cache_kib(262_144).unwrap();
        assert_eq!(cache_kib(&c.conn), 262_144, "the grid reader's is set by the holder");
        assert!(
            reader.conn().execute("UPDATE meta SET value = 'x' WHERE key = 'taken_zone'", []).is_err(),
            "read-only"
        );
        drop(reader);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);

        let memory = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let reader = memory.reader(1024).unwrap();
        assert!(reader.own.is_none());
        assert!(std::ptr::eq(reader.conn(), &memory.conn));
    }

    /// The bound holds while the guard does, and the app's cache comes back however its holder leaves.
    #[test]
    fn a_bounded_cache_is_restored_when_its_guard_goes() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), pg_platform::native::PATH_RULES).unwrap();
        let fails = |c: &mut Catalog| -> crate::Result<()> {
            let bounded = c.bounded_cache(2048)?;
            assert_eq!(cache_kib(&bounded.conn), 2048);
            bounded.conn.execute("SELECT * FROM no_such_table", [])?;
            Ok(())
        };
        assert!(fails(&mut c).is_err());
        assert_eq!(cache_kib(&c.conn), 8_192, "restored after an error");
        let panics = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _bounded = c.bounded_cache(2048).unwrap();
            panic!("the holder panics");
        }));
        assert!(panics.is_err());
        assert_eq!(cache_kib(&c.conn), 8_192, "restored after a panic");
    }
}
