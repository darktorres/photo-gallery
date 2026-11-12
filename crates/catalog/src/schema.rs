//! The catalog's schema: the SQL that makes a new catalog, the version a file is at, and how a file is brought to a
//! version. A file is compared with [`expected`], the schema this build makes in memory from the same SQL, so the
//! layout checked is the layout created.

use anyhow::{Context, anyhow};
use pg_dbfile::{Damage, Expected, Schema};
use pg_model::rules::PathRules;
use rusqlite::{Connection, Transaction};

use crate::convert::{Source, convert_notify, convert_session};
use crate::error::Result;
use crate::transaction::write_transaction;

/// One step of a migration: SQL that changes the layout, or a conversion of rows (`crate::convert`), which reads the
/// old shape from its [`Source`] and is shaped-guarded and idempotent. Every data conversion is a `Rust` step, so the
/// rebuild's copy runs the same code (`rebuild::copy_tables` runs every `Rust` step with `Source::Carried`). `PathRules`:
/// the rules the catalog's paths are stored under.
#[derive(Clone, Copy)]
pub(crate) enum Step {
    Sql(&'static str),
    Rust(fn(&Transaction<'_>, Source, PathRules) -> Result<()>),
}

/// Migrations (`PRAGMA user_version` counts them). A new catalog gets [`BASELINE`] at version [`BASELINE_VERSION`],
/// then every migration in turn; `MIGRATIONS[i]` takes version `BASELINE_VERSION + i` to the next, its steps in order,
/// in one transaction with the version.
///
/// The baseline is frozen: since the first migration (version 2), every schema change is a migration, which converts
/// the rows whose meaning changed instead of copying them as they are, and the baseline takes comment edits only (a
/// catalog at version 1 is compared with it). A catalog whose schema differs from this build's at its version is
/// rebuilt when it is prepared (`Catalog::prepare`): a new catalog is made from the baseline and the
/// migrations and takes the old rows with their ids (the rebuild's `carry_rows`, which runs the migrations' `Rust`
/// steps on them), and the old file is moved aside. A catalog of this build's schema at an earlier version is migrated
/// forward, keeping its rows.
pub(crate) const BASELINE_VERSION: i64 = 1;
pub(crate) const MIGRATIONS: &[&[Step]] = &[
    &[Step::Sql(TO_2_CHANGE_STAMPS)],
    &[Step::Sql(TO_3_NOTICES)],
    &[Step::Sql(TO_4_FAILURES)],
    TO_5_SESSION,
    &[Step::Sql(TO_6_THUMB_VIEWS)],
    &[Step::Sql(TO_7_WRITE_CHECKS)],
    TO_8_NOTIFY,
    &[Step::Sql(TO_9_CHEAPER_WRITES)],
];

/// The version [`TO_8_NOTIFY`] takes a catalog from: the tests' "as that build left one".
#[cfg(test)]
pub(crate) const BEFORE_NOTIFY: i64 = 7;

/// Version 2: `files.changed` (what a client that kept its list fetches again by) is stamped for what the grid shows of a file's borders,
/// which the baseline's triggers missed. The borders pass writes the cut into the analysis after the import, and sets
/// its sides (`Catalog::store_borders`); a pairing's replacement deletes them, and the analysis with them. `files.seq`
/// is not touched: the thumbnails are not made from the borders.
const TO_2_CHANGE_STAMPS: &str = concat!(
    stamp!(
        "analysis_cut_stamp",
        "UPDATE OF cut_left, cut_top, cut_right, cut_bottom",
        "analysis",
        new,
        when "old.cut_left IS NOT new.cut_left OR old.cut_top IS NOT new.cut_top OR old.cut_right IS NOT new.cut_right\n    OR old.cut_bottom IS NOT new.cut_bottom"
    ),
    stamp!("analysis_removed_stamp", "DELETE", "analysis", old),
    stamp!("border_sides_stamp_insert", "INSERT", "border_sides", new),
    stamp!("border_sides_stamp_delete", "DELETE", "border_sides", old),
);

/// Version 3: the inbox (docs/NOTIFICATIONS.md), what the app told the user, kept until it is dismissed.
///
/// A row is a `pg_model::notice::Notice` as JSON (`data`), with what the inbox sorts and filters by beside it. A kept
/// state (`key`, its kind's code, folder and part) is one row that the next post of its key replaces and counts; an event
/// has no key, and each post is a row. `root` is the library folder it is about: removing the folder removes its rows
/// (the folders' ids are reused, so a row must not outlive its folder). `uid` is `meta.seq` when the row was made: with
/// the id (reused too) it names one row, so a mark or a dismissal the window sends for a row since gone does nothing.
/// Times are Unix seconds, as `scans`'.
const TO_3_NOTICES: &str = concat!(
    "CREATE TABLE notices(\n",
    "    id        INTEGER PRIMARY KEY,\n",
    "    uid       INTEGER NOT NULL DEFAULT 0,\n",
    "    key       TEXT UNIQUE,\n",
    "    part      TEXT,\n",
    "    root      INTEGER REFERENCES roots(id) ON DELETE CASCADE,\n",
    "    level     INTEGER NOT NULL,   -- 0 info, 1 warning, 2 error\n",
    "    source    TEXT NOT NULL,      -- Source::code\n",
    "    kind      TEXT NOT NULL,      -- Notice::code\n",
    "    data      TEXT NOT NULL,      -- the notice, JSON\n",
    "    count     INTEGER NOT NULL DEFAULT 1,\n",
    "    created   INTEGER NOT NULL,\n",
    "    updated   INTEGER NOT NULL,\n",
    "    read      INTEGER,\n",
    "    dismissed INTEGER\n",
    ");\n",
    "CREATE INDEX notices_live ON notices(dismissed, updated DESC);\n",
    "CREATE INDEX notices_root ON notices(root);\n",
    "CREATE TRIGGER notices_uid AFTER INSERT ON notices BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE notices SET uid = ",
    seq!(),
    " WHERE id = new.id;\nEND;\n",
);

/// Version 4: the failure ledger (`crate::ledger`): the files a pass couldn't make its result of, one row per file and
/// pass, for the state of the file it failed on (`mtime`, `size`, `rotation`). A change to that state, or the file going
/// missing, removes the row (`files_failures_stale`): the next pass tries the file as it is now. A pass's failure ledger
/// is one inbox entry, read from these rows; `ledger_ack` is what the user has seen of each entry (`seen_to`: the newest
/// row they were shown, by `serial`, which is `meta.seq` when the row was made) and whether they read or dismissed it.
/// `retry`: the pass tries the file again (1), or not until it changes, a retry or a new `THUMBS_VERSION` (0: it
/// panicked). `thumbs_version` is the `pg_render::THUMBS_VERSION` the thumbnails' rows were recorded under
/// (`set_thumbs_version`).
const TO_4_FAILURES: &str = concat!(
    "CREATE TABLE file_failures(\n",
    "    id         INTEGER PRIMARY KEY,\n",
    "    file_id    INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,\n",
    "    pass       TEXT NOT NULL,      -- FilePass::code: 'thumbs' | 'borders'\n",
    "    mtime      INTEGER NOT NULL,   -- the file as it failed\n",
    "    size       INTEGER NOT NULL,\n",
    "    rotation   INTEGER NOT NULL,\n",
    "    error      TEXT NOT NULL,\n",
    "    retry      INTEGER NOT NULL,\n",
    "    serial     INTEGER NOT NULL DEFAULT 0,\n",
    "    first_seen INTEGER NOT NULL,   -- Unix seconds\n",
    "    UNIQUE(file_id, pass)\n",
    ");\n",
    "CREATE INDEX file_failures_serial ON file_failures(pass, serial);\n",
    "CREATE TABLE ledger_ack(\n",
    "    kind      TEXT PRIMARY KEY,   -- FilePass::code: 'thumbs' | 'borders' | 'import'\n",
    "    seen_to   INTEGER NOT NULL,   -- a serial (the import's: files.seq)\n",
    "    read      INTEGER,\n",
    "    dismissed INTEGER\n",
    ");\n",
    "CREATE TRIGGER file_failures_stamp AFTER INSERT ON file_failures BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE file_failures SET serial = ",
    seq!(),
    " WHERE id = new.id;\nEND;\n",
    "CREATE TRIGGER files_failures_stale AFTER UPDATE OF mtime, size, rotation, missing ON files\n",
    "WHEN old.mtime IS NOT new.mtime OR old.size IS NOT new.size OR old.rotation IS NOT new.rotation\n",
    "    OR old.missing IS NOT new.missing\n",
    "BEGIN\n    DELETE FROM file_failures WHERE file_id = new.id;\nEND;\n",
    "INSERT INTO meta(key, value) VALUES ('thumbs_version', 0);\n",
);

/// Version 5: the session is a listing (`pg_model::view::Listing`) in one row, with the saved views' columns, which the
/// views' row code reads and writes (`listing::listing_at`, `listing_values`). It was a store of text keys; the row is
/// made from them once, by `convert::convert_session`, between the new table's creation and the old one's drop. The
/// steps' SQL is the text version 5 was first made with, so the layout it leaves is the one catalogs already have.
const TO_5_SESSION: &[Step] = &[Step::Sql(TO_5_SESSION_ROW), Step::Rust(convert_session), Step::Sql(TO_5_SESSION_SWAP)];

const TO_5_SESSION_ROW: &str = r"CREATE TABLE session_row(
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    folder    BLOB NOT NULL,     -- as views.folder
    recursive INTEGER NOT NULL,
    query     TEXT NOT NULL,     -- query::Query as JSON
    sort      TEXT NOT NULL      -- query::Sort as JSON
);
";

const TO_5_SESSION_SWAP: &str = "DROP TABLE session;
ALTER TABLE session_row RENAME TO session;
";

/// Version 6: the thumbnail cache reads the catalog through its views alone. `thumb_sources` decides which files can
/// have a thumbnail (present, and analyzed without an error for the file as it is now), and gives a video's poster
/// time resolved (0 where the analysis has none; NULL for an image), so the cache's query only compares the rows with
/// its own entries. It is still one table with lookups by key, which SQLite flattens into the pass's query (the
/// `files_seq` index is kept). `thumb_folders` is the largest folder id (-1 for none): it sizes the pass's work list,
/// and is a view of its own, since `thumb_clock` is the change number the pass takes its snapshot at.
const TO_6_THUMB_VIEWS: &str = concat!(
    "DROP VIEW thumb_sources;
CREATE VIEW thumb_sources AS
    SELECT f.id, f.volume, f.file_ref, f.folder_id,
           (SELECT path FROM folders WHERE id = f.folder_id) AS folder,
           f.name, f.mtime, f.rotation, f.size, f.seq,
           CASE WHEN f.kind = '",
    pg_model::kind_video!(),
    "' THEN coalesce((SELECT poster_ms FROM analysis WHERE file_id = f.id), 0)
           END AS poster_ms
    FROM files f
    WHERE f.missing = 0
      AND EXISTS (SELECT 1 FROM analysis WHERE file_id = f.id AND error IS NULL AND mtime = f.mtime);
CREATE VIEW thumb_folders AS SELECT coalesce(max(id), -1) AS max_id FROM folders;
"
);

/// The rule a queued write's value follows (`pg_model::write::Write::from_stored`): a rating's stars 0 to 5, or 0 to 3
/// quarter turns, in the value and in the temp's.
macro_rules! write_value_ok {
    () => {
        concat!(
            "CASE kind WHEN '",
            pg_model::write_rating!(),
            "' THEN value BETWEEN 0 AND 5 AND coalesce(temp_value BETWEEN 0 AND 5, 1)
                ELSE value BETWEEN 0 AND 3 AND coalesce(temp_value BETWEEN 0 AND 3, 1) END"
        )
    };
}

/// Version 7: the write queue's value is tied to its kind by a `CHECK`, as `ratings.rating` is: the rule the rows are
/// read with (`write_value_ok!`). SQLite adds no `CHECK` in place, so the table is made again with it and takes the
/// rows that keep it: one that breaks it was never a valid write, and is dropped (the write is lost). The index and
/// the stamps go with the old table, and are made again as they were.
const TO_7_WRITE_CHECKS: &str = concat!(
    "CREATE TABLE file_writes_checked(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL CHECK (kind IN ('",
    pg_model::write_rating!(),
    "', '",
    pg_model::write_rotation!(),
    "')),
    value    INTEGER NOT NULL,  -- write::Write::stored: a rating's stars, or quarter turns
    queued   INTEGER NOT NULL,  -- Unix seconds
    attempts INTEGER NOT NULL DEFAULT 0,
    error    TEXT,
    temp       BLOB,
    temp_value INTEGER,
    PRIMARY KEY(file_id, kind),
    CHECK (",
    write_value_ok!(),
    ")
);
INSERT INTO file_writes_checked(file_id, kind, value, queued, attempts, error, temp, temp_value)
    SELECT file_id, kind, value, queued, attempts, error, temp, temp_value FROM file_writes WHERE ",
    write_value_ok!(),
    ";
DROP TABLE file_writes;
ALTER TABLE file_writes_checked RENAME TO file_writes;
CREATE INDEX file_writes_next ON file_writes((temp IS NULL), queued);
",
    stamp!("file_writes_stamp_insert", "INSERT", "file_writes", new),
    stamp!("file_writes_stamp_update", "UPDATE", "file_writes", new),
    stamp!("file_writes_stamp_delete", "DELETE", "file_writes", old),
);

/// Version 8: the inbox's storage as docs/NOTIFICATIONS.md describes it. The layout is added first, the rows are
/// converted (`convert::convert_notify`), and the column they were converted from is dropped.
const TO_8_NOTIFY: &[Step] = &[Step::Sql(TO_8_NOTIFY_ADD), Step::Rust(convert_notify), Step::Sql(TO_8_NOTIFY_DROP)];

/// - **A ledger's dismissal is a serial** (`ledger_ack.dismissed_to`): the entry is shown while it has live rows and
///   none was dismissed, or one came after the dismissal. Reading it later no longer hides it.
/// - **A notice's revision** (`notices.rev`): it rises when a state's data changes, and a mark names the revision it
///   was made for, so a mark sent for one never lands on the next.
/// - **Events are bounded by `notices_uid`**: past `kept_events!()` keyless rows, each insert deletes the oldest by
///   insertion (`uid`), read ones first, never the row inserted and never a state (bounded by its keys).
/// - **`notices(kind, root)`**: what a clear reads to find whether it has anything to remove.
/// - **The import's ledger is rows** (`pass = 'import'`), kept by the triggers on `analysis`: they mirror the failed
///   analysis, with the state of the bytes it read, so a re-analysis of the same bytes keeps its serial and a changed
///   file that fails again is new. `files_failures_stale` no longer touches them, nor any row when a file goes missing.
/// - **`file_failures.hidden`**: the file is missing (or, for the import, pending), kept by `files_failures_hidden`; the
///   inbox reads the rest through the partial index `file_failures_live`.
/// - **The folders a walk couldn't read** (`folder_failures`): one row per folder, by its exact path, tied to the library
///   folder that holds it; `walk` is the `scans` row of the walk that last saw it, which a later complete walk of a folder
///   above it sweeps by (`Catalog::mark_missing`). `reason`: `unreadable` | `mount-refused` | `refused`.
/// - **The scan leases** (`scan_leases`, transient: a rebuild carries none): one row per walk running (`id` its `scans`
///   row), so two walks of overlapping folders never interleave (`Catalog::begin_scan`).
const TO_8_NOTIFY_ADD: &str = concat!(
    "ALTER TABLE ledger_ack ADD COLUMN dismissed_to INTEGER;
ALTER TABLE notices ADD COLUMN rev INTEGER NOT NULL DEFAULT 0;
CREATE INDEX notices_kind_root ON notices(kind, root);
DROP TRIGGER notices_uid;
CREATE TRIGGER notices_uid AFTER INSERT ON notices BEGIN
    ",
    next_seq!(),
    ";
    UPDATE notices SET uid = ",
    seq!(),
    " WHERE id = new.id;
    DELETE FROM notices WHERE new.key IS NULL AND id IN (
        SELECT id FROM notices WHERE key IS NULL AND id <> new.id ORDER BY read IS NULL, uid
        LIMIT max(0, (SELECT count(*) FROM notices WHERE key IS NULL) - ",
    kept_events!(),
    "));
END;
ALTER TABLE file_failures ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
CREATE INDEX file_failures_live ON file_failures(pass, serial) WHERE ",
    live_failures!(),
    ";
DROP TRIGGER files_failures_stale;
CREATE TRIGGER files_failures_stale AFTER UPDATE OF mtime, size, rotation ON files
WHEN old.mtime IS NOT new.mtime OR old.size IS NOT new.size OR old.rotation IS NOT new.rotation
BEGIN
    DELETE FROM file_failures WHERE file_id = new.id AND pass <> 'import';
END;
CREATE TRIGGER files_failures_hidden AFTER UPDATE OF pending, missing ON files
WHEN old.pending IS NOT new.pending OR old.missing IS NOT new.missing
BEGIN
    UPDATE file_failures SET hidden = (new.missing OR (pass = 'import' AND new.pending))
    WHERE file_id = new.id AND hidden IS NOT (new.missing OR (pass = 'import' AND new.pending));
END;
CREATE TRIGGER analysis_import_failed AFTER INSERT ON analysis WHEN new.error IS NOT NULL BEGIN
    DELETE FROM file_failures WHERE file_id = new.file_id AND pass = 'import'
        AND (mtime IS NOT new.mtime OR size IS NOT new.size OR rotation IS NOT new.rotation);
    INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
        SELECT new.file_id, 'import', new.mtime, new.size, new.rotation, new.error, 0, new.analyzed, f.missing
        FROM files f WHERE f.id = new.file_id
        ON CONFLICT(file_id, pass) DO UPDATE SET error = excluded.error
        WHERE file_failures.error IS NOT excluded.error;
END;
CREATE TRIGGER analysis_import_fixed AFTER INSERT ON analysis WHEN new.error IS NULL BEGIN
    DELETE FROM file_failures WHERE file_id = new.file_id AND pass = 'import';
END;
CREATE TRIGGER analysis_import_rewritten AFTER UPDATE OF size, mtime, rotation ON analysis WHEN new.error IS NOT NULL
BEGIN
    UPDATE file_failures SET size = new.size, mtime = new.mtime, rotation = new.rotation
    WHERE file_id = new.file_id AND pass = 'import';
END;
CREATE TRIGGER analysis_import_removed AFTER DELETE ON analysis BEGIN
    DELETE FROM file_failures WHERE file_id = old.file_id AND pass = 'import';
END;
CREATE TABLE folder_failures(
    id         INTEGER PRIMARY KEY,
    root       INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    path       BLOB NOT NULL UNIQUE,   -- exact bytes, as folders.path
    reason     TEXT NOT NULL,          -- 'unreadable' | 'mount-refused' | 'refused'
    error      TEXT NOT NULL,
    walk       INTEGER NOT NULL,       -- the scans row of the walk that last saw it
    serial     INTEGER NOT NULL DEFAULT 0,
    first_seen INTEGER NOT NULL        -- Unix seconds
);
CREATE INDEX folder_failures_serial ON folder_failures(serial);
CREATE INDEX folder_failures_root ON folder_failures(root);
CREATE TRIGGER folder_failures_stamp AFTER INSERT ON folder_failures BEGIN
    ",
    next_seq!(),
    ";
    UPDATE folder_failures SET serial = ",
    seq!(),
    " WHERE id = new.id;
END;
CREATE TABLE scan_leases(
    id        INTEGER PRIMARY KEY,     -- the walk's scans row
    root      BLOB NOT NULL,           -- the folder it walks, as scans.root
    holder    TEXT NOT NULL,           -- who walks it, for the refusal's words
    heartbeat INTEGER NOT NULL         -- Unix seconds: renewed with each batch the walk writes
);
"
);

/// [`TO_8_NOTIFY`]'s last step: the acknowledgement's old dismissal time, converted.
const TO_8_NOTIFY_DROP: &str = "ALTER TABLE ledger_ack DROP COLUMN dismissed;\n";

/// Version 9: what version 8 made a write cost, measured (`just perf --budgets`), brought back to what the work needs.
/// - **`files_fresh`**: the pairing's fresh rows (`pair::fresh_rows`, unpaired with an identity, by the scan that saw
///   them), in id order, so each batch of them is a seek from the last one's id. Read through the root's folders, every
///   batch scanned all of the root's rows again: quadratic in a first scan's rows.
/// - **`analysis_import`**: the import ledger's one trigger on a new analysis, in place of `analysis_import_failed` and
///   `analysis_import_fixed`. Its `WHEN` is cheap for the common case, a file analysed without an error that has no
///   ledger row, which then writes nothing; a delete run for every analysis that succeeded cost the import a fifth of
///   its batch. A failure still writes its row, as before: the row is removed when the bytes changed, and kept (with
///   the new error) when they didn't.
const TO_9_CHEAPER_WRITES: &str =
    "CREATE INDEX files_fresh ON files(last_scan) WHERE unpaired = 1 AND volume IS NOT NULL;
DROP TRIGGER analysis_import_failed;
DROP TRIGGER analysis_import_fixed;
CREATE TRIGGER analysis_import AFTER INSERT ON analysis
WHEN new.error IS NOT NULL OR EXISTS (SELECT 1 FROM file_failures WHERE file_id = new.file_id AND pass = 'import')
BEGIN
    DELETE FROM file_failures WHERE file_id = new.file_id AND pass = 'import'
        AND (new.error IS NULL OR mtime IS NOT new.mtime OR size IS NOT new.size OR rotation IS NOT new.rotation);
    INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
        SELECT new.file_id, 'import', new.mtime, new.size, new.rotation, new.error, 0, new.analyzed, f.missing
        FROM files f WHERE new.error IS NOT NULL AND f.id = new.file_id
        ON CONFLICT(file_id, pass) DO UPDATE SET error = excluded.error
        WHERE file_failures.error IS NOT excluded.error;
END;
";

/// The `application_id` header field of a catalog file (`PGCT`). Set when the file is made, and checked with its
/// schema, so a SQLite file that merely has the same tables is not taken for a catalog.
pub(crate) const APPLICATION_ID: i64 = 0x5047_4354;

/// The rows that creation seeds and that a catalog must have the keys of: `meta`'s. Their values differ per catalog
/// (its id, the marks it was made with), so only the keys are compared.
const SEEDS: &[(&str, &str)] = &[("meta", "key")];

/// Every connection's settings. `busy_timeout`: a statement that finds the catalog locked by another connection (the
/// background job's writer, a scan) waits up to this long for it, instead of failing at once with "database is locked".
pub(crate) const SETTINGS: &str = "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
     PRAGMA cache_size = -262144; PRAGMA temp_store = MEMORY; PRAGMA busy_timeout = 5000;";

/// The version this build's catalogs are at: the baseline, then each migration.
pub(crate) fn current_version() -> Result<i64> {
    let migrations = i64::try_from(MIGRATIONS.len()).context("too many migrations")?;
    Ok(BASELINE_VERSION + migrations)
}

/// The version of the database at `conn`, or `None` when the file is not a database (SQLite can't read its header).
pub(crate) fn version_of(conn: &Connection) -> Result<Option<i64>> {
    match pg_dbfile::user_version(conn) {
        Ok(version) => Ok(Some(version)),
        Err(e) if Damage::of(&e) == Some(Damage::NotADatabase) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Bring the database at `conn` up to `target`, each step in its own write transaction. A new database (version 0)
/// gets `baseline`, with its `path_rules` row for `rules`, the `application_id` and [`BASELINE_VERSION`]; a database at
/// version `v` gets `migrations[v - BASELINE_VERSION]`, which takes it to `v + 1`: its steps in order (a `Rust` step with
/// [`Source::Main`]), with the version, in one transaction. Each migration reads the version inside its transaction, so
/// one another connection already made is not made again. A database at or past `target` is left alone; a failed step
/// rolls back its migration and leaves the database at the last good version.
pub(crate) fn bring_to(
    conn: &Connection,
    baseline: &str,
    migrations: &[&[Step]],
    rules: PathRules,
    target: i64,
) -> Result<()> {
    loop {
        let tx = write_transaction(conn)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version >= target {
            return Ok(());
        }
        if version == 0 {
            tx.execute_batch(baseline).context("creating the catalog")?;
            tx.execute("INSERT INTO meta(key, value) VALUES ('path_rules', ?1)", [crate::types::rules_name(rules)])?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", BASELINE_VERSION)?;
        } else {
            let index = usize::try_from(version - BASELINE_VERSION)
                .with_context(|| format!("schema version {version}, older than the baseline {BASELINE_VERSION}"))?;
            let steps =
                migrations.get(index).ok_or_else(|| anyhow!("schema version {version}: no migration takes it on"))?;
            let context = || format!("migration to schema version {}", version + 1);
            for step in steps.iter() {
                match *step {
                    Step::Sql(sql) => tx.execute_batch(sql).with_context(context)?,
                    Step::Rust(convert) => convert(&tx, Source::Main, rules).with_context(context)?,
                }
            }
            tx.pragma_update(None, "user_version", version + 1)?;
        }
        tx.commit()?;
    }
}

/// The schema a catalog at `version` has, as this build makes it in memory: the baseline and the migrations up to
/// `version`. Version 0 (no catalog) is the empty database.
pub(crate) fn expected(version: i64) -> Result<Schema> {
    expected_for(BASELINE, MIGRATIONS, version)
}

/// [`expected`] for the given baseline and migrations, made on an empty database: the `Sql` steps make the layout, and
/// the `Rust` steps find no rows to convert.
pub(crate) fn expected_for(baseline: &str, migrations: &[&[Step]], version: i64) -> Result<Schema> {
    let conn = Connection::open_in_memory()?;
    bring_to(&conn, baseline, migrations, pg_platform::native::PATH_RULES, version)?;
    Ok(Schema::read(&conn, SEEDS)?)
}

/// The [`expected`] schema of the current version, built once per process (every connection's `Catalog::open` checks
/// its file against it).
pub(crate) fn current_expected() -> Result<&'static Schema> {
    Ok(CURRENT.schema()?)
}

/// This build's layout at its current version, as `pg_dbfile::Expected` makes it.
fn current_layout(conn: &Connection) -> rusqlite::Result<()> {
    let version = current_version().map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
    bring_to(conn, BASELINE, MIGRATIONS, pg_platform::native::PATH_RULES, version)
        .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))
}

/// The current schema, made once per process.
static CURRENT: Expected = Expected::new(current_layout, SEEDS);

/// Why the file at `conn` is not this build's catalog at `version`, given `want` (its [`expected`] schema): `None` when
/// it is. It reads only. A file that is not a database is not a catalog, and neither is one whose `application_id` is
/// another's.
pub(crate) fn mismatch(conn: &Connection, version: i64, want: &Schema) -> Result<Option<String>> {
    let found = match Schema::read(conn, SEEDS) {
        Ok(found) => found,
        Err(e) if Damage::of(&e) == Some(Damage::NotADatabase) => return Ok(Some("not a database".to_string())),
        Err(e) => return Err(e.into()),
    };
    if let Some(difference) = found.difference(want) {
        return Ok(Some(difference.to_string()));
    }
    if version > 0 {
        let id = pg_dbfile::application_id(conn)?;
        if id != APPLICATION_ID {
            return Ok(Some(format!("application id {id}, not a catalog's")));
        }
    }
    Ok(None)
}

pub(crate) const BASELINE: &str = concat!(
    r"
CREATE TABLE scans(
    id       INTEGER PRIMARY KEY,
    root     BLOB NOT NULL,      -- the folder as stored (Name bytes, PathRules::join)
    started  INTEGER NOT NULL,   -- Unix seconds
    finished INTEGER
);
CREATE TABLE folders(
    id   INTEGER PRIMARY KEY,
    path BLOB NOT NULL UNIQUE,   -- absolute, no trailing separator (PathRules::join)
    key  BLOB NOT NULL           -- the folder's key (PathRules::folder_key): the lookup by folder
);
CREATE TABLE files(
    id        INTEGER PRIMARY KEY,
    -- The identity: the volume's 64-bit serial (Windows) or device number (Unix), and the file ID. NULL for a
    -- displaced row (its file is another one now, or it is gone): it keeps its id, folder, name, ratings and content,
    -- and waits to be paired (`unpaired`). Two NULLs are not equal, so the UNIQUE holds for any number of them.
    volume    INTEGER,
    file_ref  INTEGER,           -- NTFS file ID; the low 48 bits are the MFT record number
    -- The creation time (FILETIME ticks), where the file system gives one; NULL where it doesn't (drvfs). Two
    -- values agree when either is NULL or they are equal: see the scanner's identity rules (S-17.7 decision 2).
    created   INTEGER,
    folder_id INTEGER NOT NULL REFERENCES folders(id),
    name      BLOB NOT NULL,     -- the file name as the OS has it (Name bytes)
    ext       TEXT NOT NULL,     -- the name's last extension, lower case (media::Ext)
    kind      TEXT NOT NULL,     -- media::Kind::key, from the extension: 'image' | 'video'
    size      INTEGER NOT NULL,
    mtime     INTEGER NOT NULL,  -- FILETIME: 100 ns since 1601 UTC
    last_scan INTEGER NOT NULL REFERENCES scans(id),
    missing   INTEGER NOT NULL DEFAULT 0,
    rotation  INTEGER NOT NULL DEFAULT 0,  -- quarter turns clockwise kept here, for files that can't be rotated
                                           -- in place (`pg_edit::rotate`); applied after EXIF orientation
    order_key BLOB NOT NULL,     -- where it sorts by name, all drives merged (PathRules::order_key)
    pending   INTEGER NOT NULL DEFAULT 1,  -- the import has to analyze it (again): kept by the triggers below
    seq       INTEGER NOT NULL DEFAULT 0,  -- meta.seq when its thumbnails' source last changed: kept by triggers
    changed   INTEGER NOT NULL DEFAULT 0,  -- meta.seq at its last user-visible change (a client's resync): kept by triggers
    -- Not yet resolved by the scanner's pairing: a row made for a file it has not seen before, or a displaced row. The
    -- pairing runs at the end of every scan of its root and clears the flag from the rows it leaves as new files.
    unpaired  INTEGER NOT NULL DEFAULT 0,
    UNIQUE(volume, file_ref)
);
CREATE INDEX files_folder ON files(folder_id, name);
-- A file's identity from its file ID alone: the pairing's candidates on another volume (decision 4, tier 2).
CREATE INDEX files_ref ON files(file_ref);
CREATE INDEX folders_key ON folders(key);
-- The grid's name order and every folder's files (a range of keys: browse::read::scope).
CREATE INDEX files_order ON files(order_key) WHERE missing = 0;
-- The folder tree's counts, from the index alone.
CREATE INDEX files_present ON files(folder_id) WHERE missing = 0;
-- The import's work in on-disk order (Catalog::pending_import): a round with nothing to import is one probe.
CREATE INDEX files_to_import ON files(volume, file_ref & 0xFFFFFFFFFFFF) WHERE missing = 0 AND pending = 1;
-- The files changed since the thumbnail pass last looked (thumbs::pending_in).
CREATE INDEX files_seq ON files(seq);
-- The files changed since a change number (a client's resync).
CREATE INDEX files_changed ON files(changed);
CREATE TABLE analysis(
    file_id     INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,  -- analyze::IMPORT_VERSION
    size        INTEGER NOT NULL,  -- the file as analyzed
    mtime       INTEGER NOT NULL,
    rotation    INTEGER NOT NULL,  -- files.rotation it was decoded with
    kind        TEXT NOT NULL,     -- files.kind it was analyzed as
    analyzed    INTEGER NOT NULL,  -- Unix seconds
    error       TEXT,              -- set when the file couldn't be read or decoded; the rest is then NULL
    file_hash   BLOB,              -- tier 0a: BLAKE3 of the bytes
    pixel_hash  BLOB,              -- tier 0b: XXH3-128 of the upright decoded pixels
    format      TEXT,
    orientation INTEGER,
    width       INTEGER,           -- upright, full size
    height      INTEGER,
    pdq         BLOB,              -- tier 1: 32 bytes, Meta's byte order
    pdq_quality INTEGER,
    pdq_dihedral BLOB,             -- tier 1: the 8 orientations' hashes, 32 bytes each (the first is pdq's), for pairs
    thumbhash   BLOB,
    frames      INTEGER,           -- frames in the file: > 1 is an animated image (or a video)
    -- When it was taken, as wall-clock time (no time zone) in Unix seconds: the frame every date filter and sort
    -- compares in. A camera's own clock is taken as it is; a UTC instant (a video's creation time) is converted to
    -- the local wall clock of this system's zone (`taken_zone`), so it is also kept below, as the instant.
    taken       INTEGER,
    -- The UTC instant `taken` was converted from (a video's creation time), NULL for a camera's wall clock. `taken` is
    -- computed from it again when the system's zone changes (`set_taken_zone`), so no zone is lost at import.
    taken_utc   INTEGER,
    duration_ms INTEGER,           -- videos
    poster_ms   INTEGER,           -- videos: where the frame for the ThumbHash and the thumbnails is
    -- Borders (borders::detect) on the stored pixels, found after the import by a pass of their own: what a crop
    -- cuts from the left, top, right and bottom (detection::Detection::cut), all 0 for none. NULL until looked for,
    -- and where they aren't (videos, animated and tiny images, failures). `borders_due`: to be looked for, by this
    -- build's detector (the import sets it; Catalog::prepare too when meta.borders_version differs).
    cut_left    INTEGER,
    cut_top     INTEGER,
    cut_right   INTEGER,
    cut_bottom  INTEGER,
    borders_due INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX analysis_file_hash ON analysis(file_hash);
CREATE INDEX analysis_pixel_hash ON analysis(pixel_hash);
-- The files to retry (`gallery import --retry`) and the failure count.
CREATE INDEX analysis_failed ON analysis(file_id) WHERE error IS NOT NULL;
-- The borders to look for (Catalog::pending_borders).
CREATE INDEX analysis_borders_due ON analysis(file_id) WHERE borders_due = 1;
-- Every side borders::detect found: the accepted ones (which make the cut) and the rejected candidates.
CREATE TABLE border_sides(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    pass     INTEGER NOT NULL,  -- the peeling round (nested borders come later)
    edge     TEXT NOT NULL CHECK (edge IN ('top', 'bottom', 'left', 'right')),
    kind     TEXT NOT NULL CHECK (kind IN ('solid', 'blur')),
    class    TEXT NOT NULL CHECK (class IN ('black', 'white', 'color', 'blur')),
    depth    INTEGER NOT NULL,  -- stored pixels from the edge of what earlier rounds left
    color    INTEGER,           -- solid: 0xRRGGBB at the edge
    straight REAL NOT NULL,
    dev      REAL,              -- solid and measured: flatness
    drift    REAL,
    rejected TEXT,              -- detection::Reason key; NULL: accepted
    PRIMARY KEY(file_id, pass, edge)
) WITHOUT ROWID;
-- The class facet (query::BorderClass).
CREATE INDEX border_sides_class ON border_sides(class, file_id) WHERE rejected IS NULL;
-- Files whose borders the user marked as not a border (the review). Kept when the borders are looked for again.
CREATE TABLE border_marks(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    marked  INTEGER NOT NULL  -- Unix seconds
);
-- Tier 1 candidates: PDQ distance <= pg_model::pdq::MATCH_DISTANCE between reliable hashes, any of the 8 orientations.
CREATE TABLE pdq_pairs(
    a        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,  -- a < b
    b        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    distance INTEGER NOT NULL,
    variant  INTEGER NOT NULL,  -- dihedral variant of the newer file that matched (pdq::hash_rgb_dihedral)
    PRIMARY KEY(a, b)
) WITHOUT ROWID;
CREATE INDEX pdq_pairs_b ON pdq_pairs(b);
-- Pairs whose files are both present. A missing file keeps its pairs, in case it comes back.
CREATE VIEW present_pdq_pairs AS
    SELECT p.* FROM pdq_pairs p JOIN files fa ON fa.id = p.a JOIN files fb ON fb.id = p.b
    WHERE fa.missing = 0 AND fb.missing = 0;
-- What the thumbnail cache's pass reads (its own file, so it knows the catalog only through this): the files with an
-- analysis, and their folder's path. Version 6 remakes it (TO_6_THUMB_VIEWS). One table and no join, with the analysis and the folder as lookups by key: SQLite
-- flattens it into the pass's query only then (two joins never flatten together), which keeps the files_seq index.
CREATE VIEW thumb_sources AS
    SELECT f.id, f.volume, f.file_ref, f.folder_id,
           (SELECT path FROM folders WHERE id = f.folder_id) AS folder,
           f.name, f.kind, f.mtime, f.rotation, f.size, f.missing, f.seq,
           (SELECT error FROM analysis WHERE file_id = f.id) AS error,
           (SELECT mtime FROM analysis WHERE file_id = f.id) AS analyzed_mtime,
           (SELECT poster_ms FROM analysis WHERE file_id = f.id) AS poster_ms
    FROM files f WHERE EXISTS (SELECT 1 FROM analysis WHERE file_id = f.id);

-- The change number now, which the thumbnail pass takes its snapshot at: one row, the view the pass reads it through.
CREATE VIEW thumb_clock AS SELECT value AS seq FROM meta WHERE key = 'seq';

-- The library: folders the user added, which a rescan walks again. None is inside another.
CREATE TABLE roots(
    id    INTEGER PRIMARY KEY,
    path  BLOB NOT NULL UNIQUE,  -- absolute, no trailing separator (PathRules::join)
    added INTEGER NOT NULL       -- Unix seconds
);

-- Ratings: the catalog is the source of truth, for every format (PLAN §2). Unrated files have no row.
CREATE TABLE ratings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    rating  INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
    updated INTEGER NOT NULL  -- Unix seconds
);
-- The rating the file itself holds, as last read by the import or written by the app (0–5; no row: none).
-- A file rating that differs from this was changed by another program, and is adopted (store_analysis).
CREATE TABLE file_ratings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    value   INTEGER NOT NULL CHECK (value BETWEEN 0 AND 5)
);
-- Changes waiting to be written into the files (the writeback crate), so a write survives quitting. `value` is
-- the rating, or the quarter turns clockwise still to apply. `temp` is set once the copy of the complete new
-- content beside the original, carrying `temp_value`, into the original starts (the original may be part-written
-- from then); a row with it set is finished by copying it in (the file keeps its NTFS identity), and is never given
-- up. The user may change `value` meanwhile: what is left after `temp_value` is written stays queued.
-- `temp` is the temp file's path as the rules join it (the bytes, like `files`' folder), not text.
CREATE TABLE file_writes(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL CHECK (kind IN ('rating', 'rotation')),
    value    INTEGER NOT NULL,
    queued   INTEGER NOT NULL,  -- Unix seconds
    attempts INTEGER NOT NULL DEFAULT 0,
    error    TEXT,
    temp       BLOB,
    temp_value INTEGER,
    PRIMARY KEY(file_id, kind)
);
-- The writer's next job (Catalog::next_write orders by these): the interrupted copies first, then the oldest, read off
-- the index in order, without sorting the whole queue for each job.
CREATE INDEX file_writes_next ON file_writes((temp IS NULL), queued);

-- Saved views (ImageRater's bookmarks): a folder, the filters and an order.
CREATE TABLE views(
    name      TEXT PRIMARY KEY,
    folder    BLOB NOT NULL,     -- the merged folder's parts, joined (PathRules::join_merged); empty = all folders
    recursive INTEGER NOT NULL,
    query     TEXT NOT NULL,     -- query::Query as JSON
    sort      TEXT NOT NULL      -- query::Sort as JSON
);
-- The session as text keys, until version 5 makes it one row (TO_5_SESSION, where it is described).
CREATE TABLE session(
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- About the catalog itself. `catalog_id` is made with it: a cache keyed by file ids (the thumbnails) checks it,
-- since a rebuilt catalog numbers its files anew. `import_version` is the IMPORT_VERSION `files.pending` was
-- computed for (Catalog::prepare recomputes it when the build's differs). `seq` is the change sequence's last number.
-- `borders_version` is the BORDERS_VERSION `analysis.borders_due` was computed for. `taken_zone` is the system zone's
-- offsets the UTC-sourced `analysis.taken` values were computed with (`set_taken_zone`).
CREATE TABLE meta(
    key   TEXT PRIMARY KEY,
    value ANY NOT NULL
);
INSERT INTO meta(key, value) VALUES
    ('catalog_id', lower(hex(randomblob(16)))), ('import_version', 0), ('seq', 0), ('borders_version', 0),
    ('taken_zone', '');

-- files.pending and files.seq, kept in the statements that change what they depend on, whoever writes them. The
-- WHEN clauses keep a rescan that changes nothing from writing anything.
CREATE TRIGGER files_added AFTER INSERT ON files BEGIN
    ",
    next_seq!(),
    ";
    UPDATE files SET pending = ",
    stale!(),
    ", seq = ",
    seq!(),
    ", changed = ",
    seq!(),
    r" WHERE id = new.id;
END;
CREATE TRIGGER files_changed AFTER UPDATE OF size, mtime, rotation, kind ON files
WHEN old.size IS NOT new.size OR old.mtime IS NOT new.mtime OR old.rotation IS NOT new.rotation
    OR old.kind IS NOT new.kind
BEGIN
    UPDATE files SET pending = ",
    stale!(),
    r" WHERE id = new.id;
END;
-- What a thumbnail is made from (thumbs::Source), and whether the file is there to show.
CREATE TRIGGER files_source_changed AFTER UPDATE OF mtime, rotation, missing, size ON files
WHEN old.mtime IS NOT new.mtime OR old.rotation IS NOT new.rotation OR old.missing IS NOT new.missing
    OR old.size IS NOT new.size
BEGIN
    ",
    next_seq!(),
    ";
    UPDATE files SET seq = ",
    seq!(),
    ", changed = ",
    seq!(),
    r" WHERE id = new.id;
END;
-- A REPLACE fires only this one (recursive triggers are off): its delete half fires no trigger.
CREATE TRIGGER analysis_added AFTER INSERT ON analysis BEGIN
    ",
    next_seq!(),
    ";
    UPDATE files SET pending = ",
    stale!(),
    ", seq = ",
    seq!(),
    ", changed = ",
    seq!(),
    r" WHERE id = new.file_id;
END;
-- Besides what pending depends on: whether it decoded (a thumbnail needs that) and a video's poster frame.
CREATE TRIGGER analysis_changed AFTER UPDATE OF version, size, mtime, rotation, kind, error, poster_ms ON analysis
WHEN old.version IS NOT new.version OR old.size IS NOT new.size OR old.mtime IS NOT new.mtime
    OR old.rotation IS NOT new.rotation OR old.kind IS NOT new.kind OR old.error IS NOT new.error
    OR old.poster_ms IS NOT new.poster_ms
BEGIN
    ",
    next_seq!(),
    ";
    UPDATE files SET pending = ",
    stale!(),
    ", seq = ",
    seq!(),
    ", changed = ",
    seq!(),
    r" WHERE id = new.file_id;
END;
CREATE TRIGGER analysis_removed AFTER DELETE ON analysis BEGIN
    UPDATE files SET pending = 1 WHERE id = old.file_id;
END;
",
    stamp!("ratings_stamp_insert", "INSERT", "ratings", new),
    stamp!("ratings_stamp_update", "UPDATE", "ratings", new),
    stamp!("ratings_stamp_delete", "DELETE", "ratings", old),
    stamp!("border_marks_stamp_insert", "INSERT", "border_marks", new),
    stamp!("border_marks_stamp_delete", "DELETE", "border_marks", old),
    stamp!("file_writes_stamp_insert", "INSERT", "file_writes", new),
    stamp!("file_writes_stamp_update", "UPDATE", "file_writes", new),
    stamp!("file_writes_stamp_delete", "DELETE", "file_writes", old),
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::library;
    use crate::browse::fixtures::under_both_rules;
    use crate::test_support::changed_since;
    use pg_dbfile::Difference;
    use pg_model::ids::FileId;
    use pg_model::rating::Rating;
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;

    fn version(conn: &Connection) -> i64 {
        pg_dbfile::user_version(conn).unwrap()
    }

    /// A rebuild copies every table of the current schema but the transient ones, so a table a migration adds has to be
    /// listed in `CARRIED` or `TRANSIENT`, and in one only (or the rebuild would leave its rows behind without a word).
    #[test]
    fn carried_is_every_table_of_the_current_schema() {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, BASELINE, MIGRATIONS, pg_platform::native::PATH_RULES, current_version().unwrap()).unwrap();
        let mut tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let (carried, transient) = (crate::rebuild::CARRIED, crate::rebuild::TRANSIENT);
        assert!(carried.iter().all(|t| !transient.contains(t)), "carried and transient are disjoint");
        let mut both: Vec<String> = carried.iter().chain(transient).map(|t| t.to_string()).collect();
        both.sort();
        tables.sort();
        assert_eq!(both, tables);
    }

    /// One migration of SQL steps.
    const EXTRA: &[Step] = &[Step::Sql("CREATE TABLE extra(x INTEGER);")];
    const SEVEN: &[Step] = &[Step::Sql("INSERT INTO extra VALUES (7);")];
    const NOT_SQL: &[Step] = &[Step::Sql("INSERT INTO extra VALUES (8);"), Step::Sql("NOT SQL")];

    fn migrations_apply_once_in_order_and_a_failed_one_rolls_back(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, BASELINE, &[], rules, BASELINE_VERSION).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION);
        // A later build adds two migrations: both run, in order, once.
        let later = [EXTRA, SEVEN];
        bring_to(&conn, BASELINE, &later, rules, BASELINE_VERSION + 2).unwrap();
        bring_to(&conn, BASELINE, &later, rules, BASELINE_VERSION + 2).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION + 2);
        assert_eq!(conn.query_row("SELECT count(*) FROM extra", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        // A failed migration leaves the catalog at the last good version, none of its steps made.
        let broken = [EXTRA, SEVEN, NOT_SQL];
        let fresh = Connection::open_in_memory().unwrap();
        assert!(bring_to(&fresh, BASELINE, &broken, rules, BASELINE_VERSION + 3).is_err());
        assert_eq!(version(&fresh), BASELINE_VERSION + 2);
        assert_eq!(fresh.query_row("SELECT count(*) FROM extra", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    under_both_rules!(
        migrations_apply_once_in_order_and_a_failed_one_rolls_back,
        migrations_apply_once_in_order_and_a_failed_one_rolls_back_windows,
        migrations_apply_once_in_order_and_a_failed_one_rolls_back_posix
    );

    fn expected_schemas_follow_the_version(rules: PathRules) {
        let later = [EXTRA];
        let at_one = expected_for(BASELINE, &later, BASELINE_VERSION).unwrap();
        let at_two = expected_for(BASELINE, &later, BASELINE_VERSION + 1).unwrap();
        assert!(matches!(at_one.difference(&at_two), Some(Difference::Missing { .. })));
        assert!(matches!(at_two.difference(&at_one), Some(Difference::Unexpected { .. })));
        assert!(expected_for(BASELINE, &later, 0).unwrap().is_empty());
        // A database at a version matches the expected schema of that version.
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, BASELINE, &later, rules, BASELINE_VERSION).unwrap();
        assert_eq!(Schema::read(&conn, SEEDS).unwrap().difference(&at_one), None);
    }

    under_both_rules!(
        expected_schemas_follow_the_version,
        expected_schemas_follow_the_version_windows,
        expected_schemas_follow_the_version_posix
    );

    /// Version 5 makes the session's text keys its one row: the folder's parts (one not UTF-8) joined by the rules'
    /// separator, as a view's folder is stored, and the other keys as they were. A session never saved stays empty.
    fn the_session_keys_become_its_row(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, BASELINE, MIGRATIONS, rules, BASELINE_VERSION + 3).unwrap();
        conn.execute_batch(
            r#"INSERT INTO session(key, value) VALUES
                 ('folder', '["pinterest",{"b":"78ff"}]'), ('recursive', 'false'), ('sort', '{"key":"size"}');"#,
        )
        .unwrap();
        bring_to(&conn, BASELINE, MIGRATIONS, rules, current_version().unwrap()).unwrap();
        let row: (i64, Vec<u8>, bool, String, String) = conn
            .query_row("SELECT id, folder, recursive, query, sort FROM session", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap();
        let folder = pg_model::rules::Merged::from(vec![
            pg_model::name::Name::from("pinterest"),
            pg_model::name::Name::from_raw(b"x\xFF"),
        ]);
        assert_eq!((row.0, row.1, row.2), (1, rules.join_merged(&folder), false));
        // The query never saved reads as the default; the sort as it was saved.
        assert_eq!(pg_query::from_json::<pg_model::query::Query>(&row.3).unwrap(), pg_model::query::Query::default());
        let sort: pg_model::query::Sort = pg_query::from_json(&row.4).unwrap();
        assert_eq!(sort, pg_model::query::Sort { key: pg_model::query::SortKey::Size, descending: false });

        let empty = Connection::open_in_memory().unwrap();
        bring_to(&empty, BASELINE, MIGRATIONS, rules, current_version().unwrap()).unwrap();
        assert_eq!(empty.query_row("SELECT count(*) FROM session", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    under_both_rules!(
        the_session_keys_become_its_row,
        the_session_keys_become_its_row_windows,
        the_session_keys_become_its_row_posix
    );

    /// Version 7 keeps the queued writes whose value fits their kind and drops the others; from then on the CHECK
    /// refuses one, and the stamps still fire.
    fn a_write_out_of_its_kinds_range_is_dropped_and_refused(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, BASELINE, MIGRATIONS, rules, BASELINE_VERSION + 5).unwrap();
        conn.execute_batch(
            "INSERT INTO scans(id, root, started) VALUES (1, x'00', 0);
             INSERT INTO folders(id, path, key) VALUES (1, x'01', x'01');
             INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, order_key)
                 VALUES (1, 1, 1, 1, x'61', 'jpg', 'image', 1, 1, 1, x'61'), (2, 1, 2, 1, x'62', 'jpg', 'image', 1, 1, 1, x'62');
             INSERT INTO file_writes(file_id, kind, value, queued) VALUES (1, 'rating', 4, 0), (1, 'rotation', 7, 0),
                 (2, 'rating', 9, 0), (2, 'rotation', 3, 0);",
        )
        .unwrap();
        bring_to(&conn, BASELINE, MIGRATIONS, rules, current_version().unwrap()).unwrap();
        let kept: Vec<(i64, String, i64)> = conn
            .prepare("SELECT file_id, kind, value FROM file_writes ORDER BY file_id, kind")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(kept, [(1, "rating".to_string(), 4), (2, "rotation".to_string(), 3)]);
        assert!(conn.execute("UPDATE file_writes SET value = 6 WHERE file_id = 1", []).is_err(), "the CHECK holds");
        assert!(
            conn.execute("UPDATE file_writes SET temp_value = 4 WHERE file_id = 2", []).is_err(),
            "and for the temp's value"
        );
        let stamps = triggers(&conn);
        assert!(stamps.iter().any(|t| t == "file_writes_stamp_update"), "{stamps:?}");
    }

    under_both_rules!(
        a_write_out_of_its_kinds_range_is_dropped_and_refused,
        a_write_out_of_its_kinds_range_is_dropped_and_refused_windows,
        a_write_out_of_its_kinds_range_is_dropped_and_refused_posix
    );

    fn triggers(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT name FROM sqlite_schema WHERE type = 'trigger' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// The stamps of version 2 are its migration's, not the baseline's (which is frozen): a version-1 catalog taken
    /// forward by the migrations has this build's schema.
    fn the_baseline_and_the_migration_make_the_current_schema(rules: PathRules) {
        assert_eq!(current_version().unwrap(), BASELINE_VERSION + i64::try_from(MIGRATIONS.len()).unwrap());
        let stamps =
            ["analysis_cut_stamp", "analysis_removed_stamp", "border_sides_stamp_delete", "border_sides_stamp_insert"];
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, BASELINE, MIGRATIONS, rules, BASELINE_VERSION).unwrap();
        let before = triggers(&conn);
        assert!(stamps.iter().all(|s| !before.iter().any(|t| t == s)), "{before:?}");
        assert_eq!(Schema::read(&conn, SEEDS).unwrap().difference(&expected(BASELINE_VERSION).unwrap()), None);
        bring_to(&conn, BASELINE, MIGRATIONS, rules, current_version().unwrap()).unwrap();
        assert_eq!(version(&conn), current_version().unwrap());
        let after = triggers(&conn);
        assert!(stamps.iter().all(|s| after.iter().any(|t| t == s)), "{after:?}");
        assert!(after.iter().any(|t| t == "notices_uid"), "{after:?}");
        assert_eq!(Schema::read(&conn, SEEDS).unwrap().difference(current_expected().unwrap()), None);
    }

    under_both_rules!(
        the_baseline_and_the_migration_make_the_current_schema,
        the_baseline_and_the_migration_make_the_current_schema_windows,
        the_baseline_and_the_migration_make_the_current_schema_posix
    );

    /// The tests' version before the inbox's conversion is the one `TO_8_NOTIFY` takes a catalog from.
    #[test]
    fn before_notify_is_the_inbox_conversions_source() {
        let at = usize::try_from(BEFORE_NOTIFY - BASELINE_VERSION).unwrap();
        assert!(std::ptr::eq(MIGRATIONS[at], TO_8_NOTIFY));
    }

    /// Every user-visible change stamps its file (`files.changed`), in the same statement as the change: the file
    /// itself, its analysis, its rating, its border mark and its queued write. A view is not a file's change.
    #[test]
    fn each_user_visible_change_names_its_file() {
        let c = library(PathRules::Windows);
        let (all, now) = changed_since(&c, 0);
        assert!(all.len() >= 4, "every file the library made is a change: {all:?}");
        // Nothing has changed since `now`.
        assert_eq!(changed_since(&c, now), (vec![], now));

        c.set_rating(FileId::new(1), Rating::new(3), false).unwrap();
        c.set_border_mark(FileId::new(2), true).unwrap();
        c.queue_rotation(FileId::new(3), Turns::new(1)).unwrap();
        let (changed, after) = changed_since(&c, now);
        assert_eq!(changed, [FileId::new(1), FileId::new(2), FileId::new(3)]);
        assert!(after > now);
        // What was seen at `after` is not reported again.
        assert_eq!(changed_since(&c, after), (vec![], after));

        // A view is not a file's change.
        c.save_view(&pg_model::view::SavedView {
            name: "v".into(),
            listing: pg_model::view::Listing {
                folder: pg_model::rules::Merged::from(vec![pg_model::name::Name::from("p")]),
                ..Default::default()
            },
        })
        .unwrap();
        assert_eq!(changed_since(&c, after), (vec![], after));
    }
}
