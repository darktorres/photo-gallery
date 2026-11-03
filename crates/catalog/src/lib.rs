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
pub mod query;
pub mod writes;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, anyhow};

mod error;
pub use error::{Error, Result};
use rusqlite::functions::FunctionFlags;
use rusqlite::types::Type;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Statement, Transaction, TransactionBehavior, params};

use pg_model::analysis::Analyzed;
use pg_model::detection::Detection;
use pg_model::format::Format;
use pg_model::ids::{FileId, FileIdentity, FolderId, RootId};
use pg_model::media::{Ext, Kind};
use pg_model::orientation::Orientation;
use pg_model::path::is_under;
use pg_model::path::{FilePath, PathArena, PathRef, WorkList};
use pg_model::rating::Rating;
use pg_model::turns::Turns;

/// Migrations (`PRAGMA user_version` counts them). A new catalog gets [`BASELINE`] at version [`BASELINE_VERSION`],
/// then every migration in turn; `MIGRATIONS[i]` takes version `BASELINE_VERSION + i` to the next.
///
/// Until there is a catalog worth keeping (the full-library run at the latest, PLAN M4), schema changes edit
/// the baseline in place, without a new version, and a catalog made before the edit is copied into a new one when
/// it is opened ([`rebuild`]). From then on the baseline is frozen and every change is a migration.
const BASELINE_VERSION: i64 = 1;
const MIGRATIONS: &[&str] = &[];

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

/// Folders equal to root ?1 or below it: path in [root\, root]), as ']' is the character after '\'.
macro_rules! under_root {
    () => {
        "SELECT id FROM folders WHERE path = ?1 OR (path >= ?1 || '\\' AND path < ?1 || ']')"
    };
}

const BASELINE: &str = concat!(
    r"
CREATE TABLE scans(
    id       INTEGER PRIMARY KEY,
    root     TEXT NOT NULL,
    started  INTEGER NOT NULL,   -- Unix seconds
    finished INTEGER
);
CREATE TABLE folders(
    id   INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE    -- absolute, no trailing backslash
);
CREATE TABLE files(
    id        INTEGER PRIMARY KEY,
    volume    INTEGER NOT NULL,  -- volume serial number
    file_ref  INTEGER NOT NULL,  -- NTFS file ID; the low 48 bits are the MFT record number
    folder_id INTEGER NOT NULL REFERENCES folders(id),
    name      TEXT NOT NULL,
    ext       TEXT NOT NULL,     -- the name's last extension, lower case (media::Ext)
    kind      TEXT NOT NULL,     -- media::Kind::key, from the extension: 'image' | 'video'
    size      INTEGER NOT NULL,
    mtime     INTEGER NOT NULL,  -- FILETIME: 100 ns since 1601 UTC
    last_scan INTEGER NOT NULL REFERENCES scans(id),
    missing   INTEGER NOT NULL DEFAULT 0,
    rotation  INTEGER NOT NULL DEFAULT 0,  -- quarter turns clockwise kept here, for files that can't be rotated
                                           -- in place (pg_metadata::rotation_method); applied after EXIF orientation
    order_key BLOB NOT NULL,     -- where it sorts by name, all drives merged (path::order_key)
    pending   INTEGER NOT NULL DEFAULT 1,  -- the import has to analyze it (again): kept by the triggers below
    seq       INTEGER NOT NULL DEFAULT 0,  -- meta.seq when its thumbnails' source last changed: kept by triggers
    changed   INTEGER NOT NULL DEFAULT 0,  -- meta.seq at its last user-visible change (changes_since): kept by triggers
    UNIQUE(volume, file_ref)
);
CREATE INDEX files_folder ON files(folder_id, name);
-- The grid's name order and every folder's files (a range of keys: browse::read::scope).
CREATE INDEX files_order ON files(order_key) WHERE missing = 0;
-- The folder tree's counts, from the index alone.
CREATE INDEX files_present ON files(folder_id) WHERE missing = 0;
-- The import's work in on-disk order (Catalog::pending_import): a round with nothing to import is one probe.
CREATE INDEX files_to_import ON files(volume, file_ref & 0xFFFFFFFFFFFF) WHERE missing = 0 AND pending = 1;
-- The files changed since the thumbnail pass last looked (thumbs::pending_in).
CREATE INDEX files_seq ON files(seq);
-- The files changed since a change number (browse::read::changes_since).
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
    taken       INTEGER,           -- when it was taken, as wall-clock time (no time zone) in Unix seconds
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
-- analysis, and their folder's path. One table and no join, with the analysis and the folder as lookups by key: SQLite
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
    path  TEXT NOT NULL UNIQUE,  -- absolute, no trailing backslash
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
-- Changes waiting to be written into the files (ingest::writeback), so a write survives quitting. `value` is
-- the rating, or the quarter turns clockwise still to apply. `temp` is set once the new content is complete
-- beside the original, carrying `temp_value`; a row with it set is finished by copying it in (the file keeps its
-- NTFS identity). The user may change `value` meanwhile: what is left after `temp_value` is written stays queued.
CREATE TABLE file_writes(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL CHECK (kind IN ('rating', 'rotation')),
    value    INTEGER NOT NULL,
    queued   INTEGER NOT NULL,  -- Unix seconds
    attempts INTEGER NOT NULL DEFAULT 0,
    error    TEXT,
    temp       TEXT,
    temp_value INTEGER,
    PRIMARY KEY(file_id, kind)
);

-- Saved views (ImageRater's bookmarks): a folder, the filters and an order.
CREATE TABLE views(
    name      TEXT PRIMARY KEY,
    folder    TEXT NOT NULL,     -- merged path (path::merged_path); '' = all folders
    recursive INTEGER NOT NULL,
    query     TEXT NOT NULL,     -- query::Query as JSON
    sort      TEXT NOT NULL      -- query::Sort as JSON
);
-- Where the user was (the open view and the selected file), restored at start.
CREATE TABLE session(
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- About the catalog itself. `catalog_id` is made with it: a cache keyed by file ids (the thumbnails) checks it,
-- since a rebuilt catalog numbers its files anew. `import_version` is the IMPORT_VERSION `files.pending` was
-- computed for (Catalog::prepare recomputes it when the build's differs). `seq` is the change sequence's last number.
-- `borders_version` is the BORDERS_VERSION `analysis.borders_due` was computed for.
CREATE TABLE meta(
    key   TEXT PRIMARY KEY,
    value ANY NOT NULL
);
INSERT INTO meta(key, value) VALUES
    ('catalog_id', lower(hex(randomblob(16)))), ('import_version', 0), ('seq', 0), ('borders_version', 0);

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
    "CREATE TRIGGER ratings_stamp_insert AFTER INSERT ON ratings BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE files SET changed = ",
    seq!(),
    " WHERE id = new.file_id;\nEND;\n",
    "CREATE TRIGGER ratings_stamp_update AFTER UPDATE ON ratings BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE files SET changed = ",
    seq!(),
    " WHERE id = new.file_id;\nEND;\n",
    "CREATE TRIGGER ratings_stamp_delete AFTER DELETE ON ratings BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE files SET changed = ",
    seq!(),
    " WHERE id = old.file_id;\nEND;\n",
    "CREATE TRIGGER border_marks_stamp_insert AFTER INSERT ON border_marks BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE files SET changed = ",
    seq!(),
    " WHERE id = new.file_id;\nEND;\n",
    "CREATE TRIGGER border_marks_stamp_delete AFTER DELETE ON border_marks BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE files SET changed = ",
    seq!(),
    " WHERE id = old.file_id;\nEND;\n",
    "CREATE TRIGGER file_writes_stamp_insert AFTER INSERT ON file_writes BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE files SET changed = ",
    seq!(),
    " WHERE id = new.file_id;\nEND;\n",
    "CREATE TRIGGER file_writes_stamp_update AFTER UPDATE ON file_writes BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE files SET changed = ",
    seq!(),
    " WHERE id = new.file_id;\nEND;\n",
    "CREATE TRIGGER file_writes_stamp_delete AFTER DELETE ON file_writes BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE files SET changed = ",
    seq!(),
    " WHERE id = old.file_id;\nEND;\n",
);

/// A catalog made before the current baseline (pre-release, the baseline is edited in place) lacks some of these
/// `LAYOUT_PARTS`. The last two are the text of a trigger and a view that were edited in place (the size joined what a
/// thumbnail is made from): an earlier catalog has the same names, so only their text tells it apart.
const LAYOUT_CHECK: &str = "SELECT (SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'meta')
     + (SELECT count(*) FROM pragma_table_info('files') WHERE name IN ('order_key', 'pending', 'seq', 'changed'))
     + (SELECT count(*) FROM pragma_table_info('analysis') WHERE name IN ('rotation', 'kind', 'borders_due', 'pdq_dihedral'))
     + (SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('border_sides', 'border_marks'))
     + (SELECT count(*) FROM pragma_table_info('roots') WHERE name = 'id' AND pk = 1)
     + (SELECT count(*) FROM sqlite_schema WHERE type = 'view' AND name IN ('thumb_sources', 'thumb_clock'))
     + (SELECT count(*) FROM sqlite_schema WHERE type = 'view' AND name = 'thumb_sources' AND sql LIKE '%f.size, f.missing%')
     + (SELECT count(*) FROM sqlite_schema WHERE type = 'trigger' AND name = 'files_source_changed'
        AND sql LIKE '%old.size IS NOT new.size%')
     + (SELECT count(*) FROM sqlite_schema WHERE type = 'index' AND name = 'files_changed')
     + (SELECT count(*) FROM sqlite_schema WHERE type = 'trigger' AND name IN ('ratings_stamp_insert',
        'ratings_stamp_update', 'ratings_stamp_delete', 'border_marks_stamp_insert', 'border_marks_stamp_delete',
        'file_writes_stamp_insert', 'file_writes_stamp_update', 'file_writes_stamp_delete'))";
const LAYOUT_PARTS: i64 = 25;

/// The tables [`rebuild`] copies, each after those it refers to.
const CARRIED: &[&str] = &[
    "meta",
    "scans",
    "folders",
    "files",
    "analysis",
    "border_sides",
    "border_marks",
    "pdq_pairs",
    "roots",
    "ratings",
    "file_ratings",
    "file_writes",
    "views",
    "session",
];

/// Columns an earlier layout may lack, and what [`rebuild`] makes them from (`o` is the old row). An analysis of
/// the current version was made with the file's rotation as it is: a turn used to zero the version.
const FILLED: &[(&str, &str, &str)] = &[
    ("files", "order_key", "pg_order_key((SELECT path FROM old.folders WHERE id = o.folder_id), o.name)"),
    ("analysis", "rotation", "(SELECT rotation FROM old.files WHERE id = o.file_id)"),
    ("analysis", "kind", "(SELECT kind FROM old.files WHERE id = o.file_id)"),
    // A root kept its row when the earlier layout had no id: the row number stands for it.
    ("roots", "id", "o.rowid"),
];

/// A library folder: the user added it, and a rescan walks it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub id: RootId,
    /// Absolute, with no trailing backslash.
    pub path: String,
}

pub struct Catalog {
    conn: Connection,
}

/// The versions of the analyses the build makes, which `files.pending` and `analysis.borders_due` are kept against
/// (`analyze::IMPORT_VERSION`, `borders::BORDERS_VERSION`). The library passes them to [`Catalog::prepare`]: the
/// catalog doesn't depend on the analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Versions {
    pub import: i64,
    pub borders: i64,
}

/// Bring a catalog to the current schema: a new one gets the baseline, then each migration it lacks runs in its
/// own transaction.
fn migrate(conn: &Connection, baseline: &str, migrations: &[&str]) -> Result<()> {
    // Each step is all or nothing: the transaction rolls back when dropped on an error.
    let step = |sql: &str, to: i64| -> Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", to)?;
        Ok(tx.commit()?)
    };
    let current = BASELINE_VERSION + migrations.len() as i64;
    let mut version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == 0 {
        step(baseline, BASELINE_VERSION).context("creating the catalog")?;
        version = BASELINE_VERSION;
    }
    if version > current {
        return Err(Error::Other(anyhow!(
            "schema version {version}, which this build doesn't know (it has {current}): made by another build"
        )));
    }
    for (i, sql) in migrations.iter().enumerate().skip((version - BASELINE_VERSION) as usize) {
        let to = BASELINE_VERSION + i as i64 + 1;
        step(sql, to).with_context(|| format!("migration to schema version {to}"))?;
    }
    Ok(())
}

/// Every connection's settings.
const SETTINGS: &str = "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
     PRAGMA cache_size = -262144; PRAGMA temp_store = MEMORY;";

/// A connection to the catalog at `path`, created with the baseline when new.
fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SETTINGS)?;
    migrate(&conn, BASELINE, MIGRATIONS).with_context(|| format!("catalog {}", path.display()))?;
    Ok(conn)
}

/// Whether the catalog has every part of the current layout (a catalog of an earlier one lacks some).
fn current_layout(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(LAYOUT_CHECK, [], |r| r.get::<_, i64>(0))? == LAYOUT_PARTS)
}

/// Replace the catalog at `path`, made with an earlier layout, by a copy in the current one: every row of every
/// table, with the same ids (so the thumbnail cache still matches), and the columns it lacks made from the others
/// ([`FILLED`]). The triggers and the import version check then mark what the import has to do again. Built
/// beside it and moved over it once complete, so a failure leaves the old catalog's rows as they were (it is left
/// out of WAL mode). No connection to it may be open, here or in another process.
fn rebuild(path: &Path) -> Result<()> {
    let with_suffix = |suffix: &str| {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        std::path::PathBuf::from(p)
    };
    const IN_USE: &str = "another program has it open: close it and start again";
    // Out of WAL mode, which takes every other connection being closed, and leaves it one file.
    {
        let old = Connection::open(path)?;
        old.busy_timeout(std::time::Duration::from_secs(1))?;
        let mode: String = old.query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0)).context(IN_USE)?;
        if !mode.eq_ignore_ascii_case("delete") {
            return Err(Error::Other(anyhow!(IN_USE)));
        }
    }
    let new_path = with_suffix(".rebuild");
    for leftover in [new_path.clone(), with_suffix(".rebuild-journal")] {
        if leftover.exists() {
            std::fs::remove_file(&leftover).with_context(|| format!("deleting {}", leftover.display()))?;
        }
    }
    let built = build_copy(path, &new_path);
    if built.is_err() {
        let _ = std::fs::remove_file(&new_path);
    }
    built?;
    // A WAL beside the new file would be read into it. Any is the old catalog's, left behind when it was closed.
    for side in [with_suffix("-wal"), with_suffix("-shm")] {
        match std::fs::remove_file(&side) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(anyhow::Error::from(e).context(format!("deleting {}: {IN_USE}", side.display())).into());
            }
        }
    }
    // Windows refuses to move over a file another program has open.
    std::fs::rename(&new_path, path).with_context(|| format!("moving {} over it; {IN_USE}", new_path.display()))?;
    Ok(())
}

/// [`rebuild`]'s copy of the catalog at `path`, made at `new_path`.
fn build_copy(path: &Path, new_path: &Path) -> Result<()> {
    let new = Connection::open(new_path)?;
    migrate(&new, BASELINE, MIGRATIONS)?;
    new.create_scalar_function(
        "pg_order_key",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let text = |i| ctx.get_raw(i).as_str().map_err(|e| rusqlite::Error::UserFunctionError(e.into()));
            Ok(pg_model::path::order_key_of(text(0)?, text(1)?))
        },
    )?;
    new.execute("ATTACH DATABASE ?1 AS old", [path.to_string_lossy()])?;
    let tx = new.unchecked_transaction()?;
    for table in CARRIED {
        copy_table(&tx, table)?;
    }
    tx.commit()?;
    new.execute("DETACH DATABASE old", [])?;
    Ok(())
}

/// Copy the attached `old` catalog's `table` into the same table of this one, if it has it, and check that every
/// row came.
fn copy_table(tx: &Transaction, table: &str) -> Result<()> {
    // (name, NOT NULL without a default) of each column.
    let columns = |schema: &str| -> Result<Vec<(String, bool)>> {
        let mut stmt = tx.prepare(r#"SELECT name, "notnull" AND dflt_value IS NULL FROM pragma_table_info(?1, ?2)"#)?;
        let rows = stmt.query_map([table, schema], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    };
    let old = columns("old")?;
    if old.is_empty() {
        return Ok(());
    }
    let (mut into, mut from) = (Vec::new(), Vec::new());
    for (column, required) in columns("main")? {
        let source = if old.iter().any(|(c, _)| *c == column) {
            format!(r#"o."{column}""#)
        } else if let Some((_, _, made)) = FILLED.iter().find(|(t, c, _)| *t == table && *c == column) {
            (*made).to_string()
        } else if required {
            return Err(Error::Other(anyhow!("{table}.{column}: not in this catalog, and nothing to make it from")));
        } else {
            continue;
        };
        into.push(format!(r#""{column}""#));
        from.push(source);
    }
    // The baseline's own meta rows give way to the catalog's (its id first of all).
    let verb = if table == "meta" { "INSERT OR REPLACE" } else { "INSERT" };
    tx.execute(
        &format!(r#"{verb} INTO main."{table}" ({}) SELECT {} FROM old."{table}" o"#, into.join(", "), from.join(", ")),
        [],
    )
    .with_context(|| format!("copying {table}"))?;
    let count = |schema: &str| -> Result<i64> {
        Ok(tx.query_row(&format!(r#"SELECT count(*) FROM {schema}."{table}""#), [], |r| r.get(0))?)
    };
    if table != "meta" && count("main")? != count("old")? {
        return Err(Error::Other(anyhow!("copying {table}: {} rows of {}", count("main")?, count("old")?)));
    }
    Ok(())
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
        let path = list.file(r.get(1)?, r.get_ref(2)?.as_str()?, |arena| folder_path(folder, r, arena))?;
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
    folder.query_row([id], |f| Ok(arena.push(f.get_ref(0)?.as_str()?)))
}

/// A file as the scanner saw it.
pub struct ScannedFile<'a> {
    pub identity: FileIdentity,
    pub name: &'a str,
    /// Its last extension (`Ext::of_name`), which also says its kind.
    pub ext: Ext,
    pub size: u64,
    pub mtime: i64,
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

    /// Prepare the catalog at `path` for this build, creating it when new: one made with an earlier layout is copied
    /// into the current one first (`rebuild`, seconds for 190k files), and its version marks are set to `versions`.
    /// The library calls this once, before anything else opens the catalog.
    pub fn prepare(path: &Path, versions: Versions) -> Result<Catalog> {
        let mut conn = connect(path)?;
        if !current_layout(&conn)? {
            drop(conn);
            rebuild(path).with_context(|| format!("catalog {}: copying it into the current layout", path.display()))?;
            conn = connect(path)?;
            if !current_layout(&conn)? {
                return Err(Error::Other(anyhow!(
                    "catalog {}: still not in the current layout after copying it",
                    path.display()
                )));
            }
        }
        set_import_version(&conn, versions.import).with_context(|| format!("catalog {}", path.display()))?;
        set_borders_version(&conn, versions.borders).with_context(|| format!("catalog {}", path.display()))?;
        crate::query::register_functions(&conn)?;
        Ok(Catalog { conn })
    }

    /// Open a catalog this process has prepared ([`Catalog::prepare`], which makes it, brings it to the current layout
    /// and sets the build's version marks): the settings and the SQL functions only. Every connection to it (the
    /// app's threads, the background job, the writer), which would otherwise check the layout and the marks again
    /// each time. An error if the catalog isn't there, or its schema isn't this build's (another program changed it
    /// since).
    pub fn open(path: &Path) -> Result<Catalog> {
        let flags = OpenFlags::default() - OpenFlags::SQLITE_OPEN_CREATE;
        let conn = Connection::open_with_flags(path, flags).with_context(|| format!("catalog {}", path.display()))?;
        conn.execute_batch(SETTINGS)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let current = BASELINE_VERSION + MIGRATIONS.len() as i64;
        if version != current {
            return Err(Error::Other(anyhow!(
                "catalog {}: schema version {version}, not this build's {current}",
                path.display()
            )));
        }
        crate::query::register_functions(&conn)?;
        Ok(Catalog { conn })
    }

    /// Whether the file can't be written: it is missing from the catalog, or no longer listed.
    pub fn file_gone(&self, file_id: FileId) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT coalesce((SELECT missing FROM files WHERE id = ?1), 1)",
            [file_id.get()],
            |r| r.get(0),
        )?)
    }

    /// The folder the file is in, as the catalog has it; `None` if there is no such file.
    pub fn file_folder(&self, file_id: FileId) -> Option<String> {
        self.conn
            .query_row(
                "SELECT fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
                [file_id.get()],
                |r| r.get(0),
            )
            .ok()
    }

    /// What a queued write needs of its file: its path, its analyzed format, and its kind (`files.kind`).
    pub fn write_target(&self, file_id: FileId) -> Result<(String, Option<String>, String)> {
        Ok(self.conn.query_row(
            "SELECT fo.path || '\\' || f.name, a.format, f.kind FROM files f JOIN folders fo ON fo.id = f.folder_id
             LEFT JOIN analysis a ON a.file_id = f.id WHERE f.id = ?1",
            [file_id.get()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }

    /// The id made with the catalog (`meta.catalog_id`).
    pub fn catalog_id(&self) -> Result<String> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = 'catalog_id'", [], |r| r.get(0))?)
    }

    /// Stop the statements on this connection while `cancel` is set (from any thread): each fails with an error for
    /// which [`Error::Interrupted`] is returned, including those started after it was set, until it is cleared. Unlike
    /// SQLite's interrupt, which only reaches the statements running at that moment, it can't fall between two
    /// statements of a read.
    pub fn cancel_when(&self, cancel: Arc<AtomicBool>) {
        // Checked every 1000 virtual-machine steps: microseconds.
        self.conn.progress_handler(1000, Some(move || cancel.load(Ordering::Relaxed)));
    }

    pub fn begin_scan(&self, root: &str) -> Result<i64> {
        self.conn.execute("INSERT INTO scans(root, started) VALUES (?1, unixepoch())", [root])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_scan(&self, scan: i64) -> Result<()> {
        self.conn.execute("UPDATE scans SET finished = unixepoch() WHERE id = ?1", [scan])?;
        Ok(())
    }

    /// Mark files under `root` that this scan didn't see as missing. Their rows stay (ratings, decisions). Files in
    /// `unreadable` folders (and their subfolders) are left as they are: the scan couldn't list them, so it saw
    /// nothing there to compare with (R-45).
    pub fn mark_missing(&self, root: &str, scan: i64, unreadable: &[String]) -> Result<usize> {
        let under = |path: &str| -> Result<Vec<i64>> {
            let mut stmt = self.conn.prepare(under_root!())?;
            Ok(stmt.query_map([path], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
        };
        let mut skipped = std::collections::BTreeSet::new();
        for path in unreadable {
            skipped.extend(under(path)?);
        }
        let mut mark = self
            .conn
            .prepare("UPDATE files SET missing = 1 WHERE folder_id = ?1 AND missing = 0 AND last_scan < ?2")?;
        let mut marked = 0;
        for folder in under(root)? {
            if !skipped.contains(&folder) {
                marked += mark.execute(params![folder, scan])?;
            }
        }
        Ok(marked)
    }

    /// The library folders, by path, with their ids.
    pub fn roots(&self) -> Result<Vec<Root>> {
        let mut stmt = self.conn.prepare("SELECT id, path FROM roots ORDER BY path")?;
        let rows = stmt.query_map([], |r| Ok(Root { id: RootId::new(r.get(0)?), path: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The id of a library folder, by its path; `None` when the path is not one.
    pub fn root_id(&self, path: &str) -> Result<Option<RootId>> {
        let id: Option<i64> =
            self.conn.query_row("SELECT id FROM roots WHERE path = ?1", [path], |r| r.get(0)).optional()?;
        Ok(id.map(RootId::new))
    }

    /// Add a folder to the library. A folder already inside the library adds nothing (returns false); one that
    /// contains library folders replaces them.
    pub fn add_root(&self, root: &str) -> Result<bool> {
        let roots = self.roots()?;
        if roots.iter().any(|r| is_under(root, &r.path)) {
            return Ok(false);
        }
        for r in roots.iter().filter(|r| is_under(&r.path, root)) {
            self.conn.execute("DELETE FROM roots WHERE id = ?1", [r.id.get()])?;
        }
        self.conn.execute("INSERT INTO roots(path, added) VALUES (?1, unixepoch())", [root])?;
        Ok(true)
    }

    /// Take a folder out of the library. Its files are marked missing, not deleted: adding it back (or the drive
    /// returning) brings them back with their analysis and ratings. Returns how many files it hid; `None`, with
    /// nothing changed, when `root` is not a library folder.
    pub fn remove_root(&self, root: RootId) -> Result<Option<usize>> {
        let tx = self.conn.unchecked_transaction()?;
        let path: Option<String> =
            tx.query_row("SELECT path FROM roots WHERE id = ?1", [root.get()], |r| r.get(0)).optional()?;
        let Some(path) = path else { return Ok(None) };
        tx.execute("DELETE FROM roots WHERE id = ?1", [root.get()])?;
        let n = tx.execute(
            concat!("UPDATE files SET missing = 1 WHERE missing = 0 AND folder_id IN (", under_root!(), ")"),
            [path.as_str()],
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
    pub fn file_by_path(&self, path: &str) -> Result<Option<FileId>> {
        let Some((folder, name)) = path.rsplit_once('\\') else { return Ok(None) };
        Ok(self
            .conn
            .prepare_cached(
                "SELECT f.id FROM files f JOIN folders d ON d.id = f.folder_id
                 WHERE lower(d.path) = lower(?1) AND lower(f.name) = lower(?2) AND f.missing = 0",
            )?
            .query_row([folder, name], |r| r.get::<_, i64>(0))
            .optional()?
            .map(FileId::new))
    }

    /// The present files in a folder and its subfolders (an absolute path in the catalog's case, without a
    /// trailing backslash).
    pub fn files_under(&self, folder: &str) -> Result<Vec<FileId>> {
        let mut stmt = self.conn.prepare(concat!(
            "SELECT id FROM files WHERE missing = 0 AND folder_id IN (",
            under_root!(),
            ")"
        ))?;
        let ids: Vec<i64> = stmt.query_map([folder], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids.into_iter().map(FileId::new).collect())
    }

    /// Make these files due for the import again (`files.pending`), whatever their analysis: after a fix to what
    /// the import reads, which comes without a new IMPORT_VERSION before the first release. Their next analysis
    /// clears it, as any does (`analysis_added`). Missing files are left as they are. Returns how many it marked.
    pub fn mark_pending(&self, ids: &[FileId]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
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
                "SELECT (SELECT path FROM folders WHERE id = f.folder_id) || '\\' || f.name, f.size, f.mtime,
                        a.format, a.orientation, a.width, a.height, a.cut_left, a.cut_top, a.cut_right, a.cut_bottom
                 FROM files f JOIN analysis a ON a.file_id = f.id
                 WHERE f.id = ?1 AND f.missing = 0 AND f.pending = 0 AND a.borders_due = 0
                   AND a.size = f.size AND a.mtime = f.mtime
                   AND a.cut_left + a.cut_top + a.cut_right + a.cut_bottom > 0
                   AND NOT EXISTS(SELECT 1 FROM border_marks WHERE file_id = f.id)",
            )?
            .query_row([file_id.get()], |r| {
                let (orientation, w, h): (Orientation, u32, u32) =
                    (Orientation::from_tag(r.get(4)?), r.get(5)?, r.get(6)?);
                let (width, height) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
                let format = r.get_ref(3)?.as_str()?;
                let format = Format::from_name(format).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(3, Type::Text, format!("unknown format {format}").into())
                })?;
                Ok(CropSource {
                    path: paths.push(r.get_ref(0)?.as_str()?),
                    size: r.get(1)?,
                    mtime: r.get(2)?,
                    format,
                    width,
                    height,
                    cut: [r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?],
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
    pub fn file_path(&self, file_id: FileId) -> Result<String> {
        Ok(self.conn.query_row(
            "SELECT fo.path || '\\' || f.name FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
            [file_id.get()],
            |r| r.get(0),
        )?)
    }

    /// Give back this connection's page cache (the reader does it after each list it made).
    pub fn release_memory(&self) -> rusqlite::Result<()> {
        self.conn.release_memory()
    }

    /// Each count from its own index where there is one (`files_present`, `files_to_import`, `analysis_failed`).
    pub fn status(&self) -> Result<Status> {
        let one = |sql: &str| -> Result<i64> { Ok(self.conn.query_row(sql, [], |r| r.get(0))?) };
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
}

impl Catalog {
    /// Begin a transaction on the catalog (see [`Tx`]).
    pub fn tx(&mut self) -> Result<Tx<'_>> {
        Ok(Tx { inner: self.conn.transaction()? })
    }
}

impl Tx<'_> {
    /// The writer of the scan `scan`, in this transaction.
    pub fn scan_writer(&self, scan: i64) -> Result<ScanWriter<'_>> {
        ScanWriter::new(&self.inner, scan)
    }

    /// The writer of the import's analyses, in this transaction.
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

/// The scan's writes.
pub struct ScanWriter<'tx> {
    scan: i64,
    folder_id: Statement<'tx>,
    new_folder: Statement<'tx>,
    known: Statement<'tx>,
    seen: Statement<'tx>,
    file: Statement<'tx>,
    /// The order key of the file being recorded.
    key: Vec<u8>,
}

impl<'tx> ScanWriter<'tx> {
    pub fn new(tx: &'tx Transaction, scan: i64) -> Result<ScanWriter<'tx>> {
        Ok(ScanWriter {
            scan,
            folder_id: tx.prepare("SELECT id FROM folders WHERE path = ?1")?,
            new_folder: tx.prepare("INSERT INTO folders(path) VALUES (?1) RETURNING id")?,
            known: tx.prepare(
                "SELECT id, folder_id, name, ext, kind, size, mtime, missing, order_key FROM files
                 WHERE volume = ?1 AND file_ref = ?2",
            )?,
            seen: tx.prepare("UPDATE files SET last_scan = ?2 WHERE id = ?1")?,
            file: tx.prepare(
                "INSERT INTO files(volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, order_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(volume, file_ref) DO UPDATE SET
                    folder_id = excluded.folder_id, name = excluded.name, ext = excluded.ext, kind = excluded.kind,
                    size = excluded.size, mtime = excluded.mtime, last_scan = excluded.last_scan, missing = 0,
                    order_key = excluded.order_key",
            )?,
            key: Vec::new(),
        })
    }

    /// The folder's row, made if it's new.
    pub fn folder(&mut self, path: &str) -> Result<Folder> {
        let key = pg_model::path::folder_key(pg_model::path::merged_path(path));
        let id: i64 = match self.folder_id.query_row([path], |r| r.get(0)).optional()? {
            Some(id) => id,
            None => self.new_folder.query_row([path], |r| r.get(0))?,
        };
        Ok(Folder { id: FolderId::new(id), key })
    }

    /// Record a file the scan found, by identity: a moved or renamed file keeps its row (and its analysis).
    /// Whether the catalog changed: a new file, or one moved, renamed, rewritten or back from missing. A file
    /// recorded as it is is only marked seen by this scan.
    pub fn upsert_file(&mut self, folder: &Folder, f: &ScannedFile) -> Result<bool> {
        let ScanWriter { scan, known, seen, file, key, .. } = self;
        key.clear();
        key.extend_from_slice(&folder.key);
        pg_model::path::push_name_key(key, f.name);
        let (volume, file_ref, size) = (f.identity.volume, f.identity.file_index as i64, f.size as i64);
        let (ext, kind) = (f.ext.name(), f.ext.kind().key());
        let row = known
            .query_row(params![volume, file_ref], |r| {
                let same = r.get::<_, i64>(1)? == folder.id.get()
                    && r.get_ref(2)?.as_str()? == f.name
                    && r.get_ref(3)?.as_str()? == ext
                    && r.get_ref(4)?.as_str()? == kind
                    && r.get::<_, i64>(5)? == size
                    && r.get::<_, i64>(6)? == f.mtime
                    && r.get::<_, i64>(7)? == 0
                    && r.get_ref(8)?.as_blob()? == &key[..];
                Ok((r.get::<_, i64>(0)?, same))
            })
            .optional()?;
        if let Some((id, true)) = row {
            seen.execute(params![id, *scan])?;
            return Ok(false);
        }
        // `pending` and `seq` follow from the triggers.
        file.execute(params![volume, file_ref, folder.id.get(), f.name, ext, kind, size, f.mtime, *scan, &key[..]])?;
        Ok(true)
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
                    taken, duration_ms, poster_ms, borders_due, pdq_dihedral)
                 VALUES (?1, ?2, ?3, ?4, ?19, ?20, unixepoch(), NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                         CASE WHEN ?16 THEN unixepoch(?15, 'unixepoch', 'localtime') ELSE ?15 END, ?17, ?18, 0, ?21)",
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
            let color = s.color.map(|[r, g, b]| ((r as i64) << 16) | ((g as i64) << 8) | b as i64);
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
    use pg_analysis::analyze::IMPORT_VERSION;
    use pg_analysis::borders::BORDERS_VERSION;
    use pg_model::analysis::{Analysis, ImageFacts, Media, PixelHashes, ThumbHash};
    use pg_testkit::test_folder;

    /// The versions this build's analyses are stamped with, as the library passes them to [`Catalog::prepare`].
    fn versions() -> Versions {
        Versions { import: IMPORT_VERSION, borders: BORDERS_VERSION }
    }

    /// Where this module's tests write (`PG_TEST_DIR` when it is set).
    const SCRATCH: &str = r"D:\PhotoGallery\scratch_catalog";

    /// The layout before the order key (2026-10-31), as the catalogs made with it have it.
    const BEFORE_ORDER_KEY: &str = r"
        PRAGMA journal_mode = WAL;
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

    #[test]
    fn an_earlier_layout_is_copied_into_the_current_one() {
        let dir = test_folder(SCRATCH, "rebuild");
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

        let c = Catalog::prepare(&path, versions()).unwrap();
        assert!(current_layout(&c.conn).unwrap());
        assert!(!dir.join("catalog.sqlite.rebuild").exists());
        let rows = |sql: &str| -> Vec<String> {
            let mut stmt = c.conn.prepare(sql).unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        // The user's own: the library, ratings, the queued turn, views and session, with the same file ids.
        assert_eq!(rows("SELECT path FROM roots"), [r"D:\p"]);
        // The row number of the old layout is the root's id now.
        assert_eq!(rows("SELECT id || ' ' || path FROM roots"), [r"1 D:\p"]);
        assert_eq!(rows("SELECT file_id || ':' || rating FROM ratings ORDER BY 1"), ["11:4", "13:1"]);
        assert_eq!(rows("SELECT file_id || ':' || value FROM file_ratings"), ["11:4"]);
        assert_eq!(rows("SELECT file_id || kind || value FROM file_writes"), ["12rotation1"]);
        assert_eq!(rows("SELECT name FROM views"), ["best"]);
        assert_eq!(rows("SELECT value FROM session"), ["p"]);
        assert_eq!(rows("SELECT a || '-' || b FROM pdq_pairs"), ["11-12"]);
        // What the layout added: the order keys as the scanner writes them, and each analysis's rotation and kind.
        let keys: Vec<(Vec<u8>, String, String)> = {
            let mut stmt = c
                .conn
                .prepare("SELECT f.order_key, fo.path, f.name FROM files f JOIN folders fo ON fo.id = f.folder_id")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect()
        };
        for (key, folder, name) in keys {
            assert_eq!(key, pg_model::path::order_key_of(&folder, &name), "{name}");
        }
        assert_eq!(
            rows("SELECT file_id || ':' || rotation || kind FROM analysis ORDER BY 1"),
            ["11:0image", "12:1image", "14:0image"]
        );
        // Analyses made from the file as it is stand; the file without one and the one changed since are pending.
        assert_eq!(rows("SELECT id || ':' || pending FROM files ORDER BY id"), ["11:0", "12:0", "13:1", "14:1"]);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_catalog_with_the_earlier_source_trigger_or_view_is_copied_into_the_current_layout() {
        let dir = test_folder(SCRATCH, "layout_source");
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions()).unwrap());
        let fresh = Connection::open(&path).unwrap();
        assert!(current_layout(&fresh).unwrap());
        // The same names and counts, the text before the size was added: the layout check must refuse it.
        let text = |name: &str| -> String {
            fresh.query_row("SELECT sql FROM sqlite_schema WHERE name = ?1", [name], |r| r.get(0)).unwrap()
        };
        let (trigger, view) = (text("files_source_changed"), text("thumb_sources"));
        let old_trigger =
            trigger.replace(", size ON files", " ON files").replace("\n    OR old.size IS NOT new.size", "");
        let old_view = view.replace("f.size, f.missing", "f.missing");
        assert!(old_trigger != trigger && old_view != view);
        fresh
            .execute_batch(&format!(
                "DROP TRIGGER files_source_changed; {old_trigger}; DROP VIEW thumb_sources; {old_view};"
            ))
            .unwrap();
        drop(fresh);
        let earlier = Connection::open(&path).unwrap();
        assert!(!current_layout(&earlier).unwrap());
        drop(earlier);

        let c = Catalog::prepare(&path, versions()).unwrap();
        assert!(current_layout(&c.conn).unwrap());
        let copied: String = c
            .conn
            .query_row(
                "SELECT group_concat(sql, ' ') FROM sqlite_schema WHERE name IN ('files_source_changed', 'thumb_sources')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(copied.contains("old.size IS NOT new.size") && copied.contains("f.size, f.missing"), "{copied}");
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_catalog_that_cant_be_copied_is_left_as_it_was() {
        let dir = test_folder(SCRATCH, "rebuild-fails");
        let path = dir.join("catalog.sqlite");
        // Files without the columns the current layout needs, and no way to make them.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE files(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO files VALUES (1, 'x');
             PRAGMA user_version = 1;",
        )
        .unwrap();
        drop(conn);
        let e = Catalog::prepare(&path, versions()).err().expect("refused");
        assert!(format!("{e:#}").contains("files.volume"), "{e:#}");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(conn.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(conn);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_catalog_open_elsewhere_isnt_copied() {
        let dir = test_folder(SCRATCH, "rebuild-open");
        let path = dir.join("catalog.sqlite");
        let other = Connection::open(&path).unwrap();
        other.execute_batch(BEFORE_ORDER_KEY).unwrap();
        let e = Catalog::prepare(&path, versions()).err().expect("refused");
        assert!(format!("{e:#}").contains("another program has it open"), "{e:#}");
        drop(other);
        assert!(Catalog::prepare(&path, versions()).is_ok(), "copied once nothing else has it open");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn catalogs_have_their_own_id() {
        let (a, b) = (
            Catalog::prepare(Path::new(":memory:"), versions()).unwrap(),
            Catalog::prepare(Path::new(":memory:"), versions()).unwrap(),
        );
        assert_eq!(a.catalog_id().unwrap().len(), 32);
        assert_ne!(a.catalog_id().unwrap(), b.catalog_id().unwrap());
    }

    #[test]
    fn roots_never_nest() {
        let c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        assert!(c.add_root(r"D:\a\b").unwrap());
        assert!(c.add_root(r"D:\a\bc").unwrap());
        assert!(!c.add_root(r"d:\A\b\c").unwrap(), "inside an existing root");
        assert!(c.add_root(r"D:\a").unwrap(), "replaces both");
        let paths = |c: &Catalog| c.roots().unwrap().into_iter().map(|r| r.path).collect::<Vec<_>>();
        assert_eq!(paths(&c), [r"D:\a"]);
        let id = c.root_id(r"D:\a").unwrap().expect("a library folder has an id");
        assert_eq!(c.remove_root(id).unwrap(), Some(0));
        assert!(paths(&c).is_empty());
        assert_eq!(c.remove_root(id).unwrap(), None, "not a library folder any more");
    }

    #[test]
    fn cancelled_until_cleared() {
        let c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        c.cancel_when(cancel.clone());
        let long =
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000) SELECT count(*) FROM n";
        let run = || c.conn.query_row(long, [], |r| r.get::<_, i64>(0)).map_err(Error::from);
        assert_eq!(run().unwrap(), 100_000);
        // Set between statements (SQLite's own interrupt would be lost there): every statement fails.
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(run().unwrap(), 100_000);
    }

    fn version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn migrations_apply_once_in_order() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn, BASELINE, &[]).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION);
        // A later build adds two migrations: both run, in order, once.
        let later = ["CREATE TABLE extra(x INTEGER);", "INSERT INTO extra VALUES (7);"];
        migrate(&conn, BASELINE, &later).unwrap();
        migrate(&conn, BASELINE, &later).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION + 2);
        assert_eq!(conn.query_row("SELECT count(*) FROM extra", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        // An older build refuses the newer catalog.
        assert!(migrate(&conn, BASELINE, &later[..1]).is_err());
        // A failed migration leaves the catalog at the last good version.
        let broken = ["CREATE TABLE extra(x INTEGER);", "INSERT INTO extra VALUES (7);", "NOT SQL"];
        assert!(migrate(&conn, BASELINE, &broken).is_err());
        assert_eq!(version(&conn), BASELINE_VERSION + 2);
    }

    /// A catalog with one file in it.
    fn one_file() -> (Catalog, i64) {
        let c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        c.conn
            .execute_batch(
                "INSERT INTO scans(id, root, started) VALUES (1, 'D:', 0);
                 INSERT INTO folders(id, path) VALUES (1, 'D:\\x');",
            )
            .unwrap();
        fixture::insert(&c, &[fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]);
        (c, 1)
    }

    /// Whether the import would analyze file `id`, and its change number (`files.seq`).
    /// A turn the file can't hold, written by the writer: the catalog keeps it.
    fn turn_kept(c: &Catalog, id: FileId, turns: i64) {
        let job =
            writes::Job { file_id: id, kind: pg_model::write::WriteKind::Turns, value: turns, temp: None, attempts: 0 };
        writes::done(c, &job, writes::WriteEffect::KeptInCatalog(Turns::new(turns as u8))).unwrap();
    }

    fn state(c: &Catalog, id: FileId) -> (bool, i64) {
        c.conn
            .query_row("SELECT pending, seq FROM files WHERE id = ?1", [id.get()], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
    }

    /// The scanner finding file 7 on volume 1 in D:\x like this; whether that changed the catalog.
    fn scanned(c: &mut Catalog, scan: i64, name: &str, size: u64, mtime: i64) -> bool {
        let tx = c.conn.transaction().unwrap();
        let mut writer = ScanWriter::new(&tx, scan).unwrap();
        let folder = writer.folder(r"D:\x").unwrap();
        let ext = Ext::of_name(name).unwrap();
        let changed = writer
            .upsert_file(&folder, &ScannedFile { identity: FileIdentity::new(1, 7), name, ext, size, mtime })
            .unwrap();
        drop(writer);
        tx.commit().unwrap();
        changed
    }

    #[test]
    fn a_scan_tells_a_file_found_as_it_was_from_one_that_changed() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        c.add_root(r"D:\x").unwrap();
        let row = |c: &Catalog| -> (i64, String, i64, i64, i64, Vec<u8>) {
            c.conn
                .query_row("SELECT last_scan, name, size, mtime, missing, order_key FROM files", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
                })
                .unwrap()
        };
        let key = |name| pg_model::path::order_key_of(r"D:\x", name);
        let scans: Vec<i64> = (0..9).map(|_| c.begin_scan(r"D:\x").unwrap()).collect();
        assert!(scanned(&mut c, scans[0], "a.jpg", 10, 100), "new");
        // As it was: only marked seen by the scan.
        assert!(!scanned(&mut c, scans[1], "a.jpg", 10, 100));
        assert_eq!(row(&c), (scans[1], "a.jpg".into(), 10, 100, 0, key("a.jpg")));
        for (scan, name, size, mtime) in [
            (scans[2], "A.jpg", 10, 100),
            (scans[3], "b.jpg", 10, 100),
            (scans[4], "b.jpg", 11, 100),
            (scans[5], "b.jpg", 11, 101),
        ] {
            assert!(scanned(&mut c, scan, name, size, mtime), "{name} {size} {mtime}");
            assert_eq!(row(&c), (scan, name.into(), size as i64, mtime, 0, key(name)));
        }
        // Back from missing.
        c.conn.execute("UPDATE files SET missing = 1", []).unwrap();
        assert!(scanned(&mut c, scans[6], "b.jpg", 11, 101));
        assert_eq!(row(&c).4, 0);
        // An order key made by an earlier rule is made again.
        c.conn.execute("UPDATE files SET order_key = x'00'", []).unwrap();
        assert!(scanned(&mut c, scans[7], "b.jpg", 11, 101));
        assert_eq!(row(&c).5, key("b.jpg"));
        assert!(!scanned(&mut c, scans[8], "b.jpg", 11, 101));
    }

    #[test]
    fn what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        c.add_root(r"D:\x").unwrap();
        let scan = c.begin_scan(r"D:\x").unwrap();
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let stored = |c: &mut Catalog, item: &WorkItem| {
            let tx = c.conn.transaction().unwrap();
            let stored =
                AnalysisWriter::new(&tx, versions().import).unwrap().store_failure(item, "a stand-in").unwrap();
            tx.commit().unwrap();
            stored
        };
        let item = next_work(&c);
        // A write of the app's into it in progress: it may have been read part-written.
        c.conn
            .execute(
                r"INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value)
                  VALUES (?1, 'rating', 3, 0, 'D:\x\a.jpg.tmp', 3)",
                [item.id.get()],
            )
            .unwrap();
        assert!(!stored(&mut c, &item));
        assert!(state(&c, item.id).0, "still due");
        c.conn.execute("DELETE FROM file_writes", []).unwrap();
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
        assert!(!state(&c, item.id).0);
    }

    #[test]
    fn a_size_change_alone_moves_the_file_in_the_thumbnail_source() {
        // The scanner decides a file changed by its size and mtime, so a size change with the same mtime must reach
        // the thumbnail pass (through files.seq), or a thumbnail made from the old file would be kept.
        use fixture::{File, insert};
        let c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        c.conn
            .execute_batch("INSERT INTO scans(id, root, started) VALUES (1, 'D:\\x', 0); INSERT INTO folders(id, path) VALUES (1, 'D:\\x');")
            .unwrap();
        insert(&c, &[File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]);
        c.conn
            .execute_batch("INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed) VALUES (1, 1, 10, 100, 0, 'image', 0);")
            .unwrap();
        let seq =
            || -> i64 { c.conn.query_row("SELECT seq FROM thumb_sources WHERE id = 1", [], |r| r.get(0)).unwrap() };
        let before = seq();
        c.conn.execute("UPDATE files SET size = 11 WHERE id = 1", []).unwrap();
        assert!(seq() > before);
    }

    #[test]
    fn thumb_sources_is_the_analysed_files_with_their_folders() {
        use fixture::{File, insert};
        let c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        c.conn
            .execute_batch(
                "INSERT INTO scans(id, root, started) VALUES (1, 'D:\\x', 0);
                 INSERT INTO folders(id, path) VALUES (1, 'D:\\x'), (2, 'D:\\x\\sub');",
            )
            .unwrap();
        insert(
            &c,
            &[
                File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100),
                File::new(FileId::new(2), FolderId::new(2), "b.jpg", 10, 100),
                File::new(FileId::new(3), FolderId::new(1), "c.jpg", 10, 100),
            ],
        );
        // File 1 analyzed, file 2 failed (still a source: the pass leaves it out), file 3 not analyzed (not a source).
        c.conn
            .execute_batch(
                "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error) VALUES
                    (1, 1, 10, 100, 0, 'image', 0, NULL), (2, 1, 10, 100, 0, 'image', 0, 'unreadable');",
            )
            .unwrap();
        let rows: Vec<(i64, String)> = c
            .conn
            .prepare("SELECT id, folder FROM thumb_sources ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows, [(1, r"D:\x".to_string()), (2, r"D:\x\sub".to_string())]);
        // The clock is the catalog's change number, as the pass reads it.
        let clock: i64 = c.conn.query_row("SELECT seq FROM thumb_clock", [], |r| r.get(0)).unwrap();
        let meta: i64 = c.conn.query_row("SELECT value FROM meta WHERE key = 'seq'", [], |r| r.get(0)).unwrap();
        assert_eq!(clock, meta);
        // The pass's shape over it, with a TEMP stub of the thumbnail cache's entries: a change number's range still
        // reads files_seq, and a file with no thumbnail yet is the one listed.
        c.conn
            .execute_batch(
                "CREATE TEMP TABLE entries(file_id INTEGER, height INTEGER, mtime INTEGER, rotation INTEGER, UNIQUE(file_id, height));",
            )
            .unwrap();
        let sql = "SELECT f.id FROM thumb_sources f LEFT JOIN entries t ON t.file_id = f.id AND t.height = ?1
                   WHERE f.seq > ?2 AND f.missing = 0 AND f.error IS NULL AND f.analyzed_mtime = f.mtime
                     AND (t.file_id IS NULL OR t.mtime != f.mtime OR t.rotation != f.rotation)
                   ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF";
        let plan: Vec<String> = c
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map([100, -1], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(plan.join("\n").contains("files_seq"), "{plan:?}");
        let listed: Vec<i64> =
            c.conn.prepare(sql).unwrap().query_map([100, -1], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
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
        let tx = c.conn.transaction().unwrap();
        AnalysisWriter::new(&tx, versions().import).unwrap().store_failure(item, "a stand-in for an analysis").unwrap();
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

        let mut c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        c.add_root(r"D:\x").unwrap();
        let scan = c.begin_scan(r"D:\x").unwrap();
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
        let tx = c.conn.transaction().unwrap();
        AnalysisWriter::new(&tx, versions().import)
            .unwrap()
            .store_analysis(&item, &stored_analysis(&analysis))
            .unwrap();
        tx.commit().unwrap();
        let cut = |c: &Catalog| -> [Option<i64>; 4] {
            c.conn
                .query_row("SELECT cut_left, cut_top, cut_right, cut_bottom FROM analysis", [], |r| {
                    Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?])
                })
                .unwrap()
        };
        // The import leaves them to the borders pass.
        assert_eq!(cut(&c), [None; 4]);
        let due = c.pending_borders().unwrap();
        assert_eq!(due.items.iter().map(|d| d.id).collect::<Vec<_>>(), [item.id]);
        let tx = c.conn.transaction().unwrap();
        BordersWriter::new(&tx).unwrap().store_borders(&due.items[0], Some(&found)).unwrap();
        tx.commit().unwrap();
        assert_eq!(cut(&c), [Some(0), Some(41), Some(0), Some(41)]);
        assert_eq!(browse::read::border_sides(&c, item.id).unwrap(), found.sides);
        assert!(c.pending_borders().unwrap().is_empty());

        // The grid's entry, the filters and the sort.
        let list = |c: &Catalog, q: Query| -> Vec<browse::read::Entry> {
            browse::read::list_grid(c, "", true, &q, Sort::default()).unwrap()
        };
        let entry = list(&c, Query::default()).remove(0);
        assert_eq!(entry.borders, browse::read::Borders::Found);
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
        assert_eq!(list(&c, Query::default())[0].borders, browse::read::Borders::Marked);
        assert!(list(&c, only(vec![BorderFacet::Black])).is_empty());
        assert_eq!(list(&c, only(vec![BorderFacet::None])).len(), 1);
        assert_eq!(list(&c, Query { border_marked: Some(true), ..Query::default() }).len(), 1);

        // A new detector: due again, without a new import; the mark stays.
        c.conn.execute("UPDATE meta SET value = 0 WHERE key = 'borders_version'", []).unwrap();
        set_borders_version(&c.conn, BORDERS_VERSION).unwrap();
        assert!(c.pending_import(false).unwrap().is_empty());
        let due = c.pending_borders().unwrap();
        assert_eq!(due.len(), 1);
        let tx = c.conn.transaction().unwrap();
        BordersWriter::new(&tx).unwrap().store_borders(&due.items[0], Some(&Detection::none(400, 300))).unwrap();
        tx.commit().unwrap();
        assert_eq!(cut(&c), [Some(0); 4]);
        assert!(browse::read::border_sides(&c, item.id).unwrap().is_empty());
        assert!(c.pending_borders().unwrap().is_empty());
        assert_eq!(list(&c, Query::default())[0].borders, browse::read::Borders::None);
        let marks: i64 = c.conn.query_row("SELECT count(*) FROM border_marks", [], |r| r.get(0)).unwrap();
        assert_eq!(marks, 1);

        // Analyzed again (the file changed): the old borders go, the new ones are due.
        let tx = c.conn.transaction().unwrap();
        BordersWriter::new(&tx).unwrap().store_borders(&due.items[0], Some(&found)).unwrap();
        AnalysisWriter::new(&tx, versions().import)
            .unwrap()
            .store_analysis(&item, &stored_analysis(&analysis))
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(cut(&c), [None; 4]);
        assert!(browse::read::border_sides(&c, item.id).unwrap().is_empty());
        assert_eq!(c.pending_borders().unwrap().len(), 1);
    }

    #[test]
    fn border_depth_sorts_by_the_largest_share_of_a_side_cut() {
        use crate::browse;
        use pg_model::query::{Query, Sort, SortKey};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        c.add_root(r"D:\x").unwrap();
        let scan = c.begin_scan(r"D:\x").unwrap();
        // 400 × 300 stored. b: none; c: 30 of 300 at the top (10%); d: 60 of 400 at the left (15%), but turned a
        // quarter (orientation 6), so 60 of the 400 upright rows; e: 90 at the top (30%), marked not a border.
        let files = [("b.jpg", 1, None), ("c.jpg", 1, Some([0, 30, 0, 0])), ("d.jpg", 6, Some([60, 0, 0, 0]))];
        let files = files.into_iter().chain([("e.jpg", 1, Some([0, 90, 0, 0]))]);
        for (n, (name, orientation, cut)) in files.enumerate() {
            let tx = c.conn.transaction().unwrap();
            let mut writer = ScanWriter::new(&tx, scan).unwrap();
            let folder = writer.folder(r"D:\x").unwrap();
            let ext = Ext::of_name(name).unwrap();
            let file = ScannedFile { identity: FileIdentity::new(1, 10 + n as u64), name, ext, size: 10, mtime: 100 };
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
                file_hash: [n as u8; 32],
                orientation: Orientation::from_tag(orientation),
                width,
                height,
                thumbhash: ThumbHash::default(),
                frames: 1,
                taken: None,
                media: a_jpeg(),
            };
            let tx = c.conn.transaction().unwrap();
            AnalysisWriter::new(&tx, versions().import)
                .unwrap()
                .store_analysis(&item, &stored_analysis(&analysis))
                .unwrap();
            BordersWriter::new(&tx).unwrap().store_borders(&item, Some(&borders)).unwrap();
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

    #[test]
    fn pending_follows_what_the_analysis_was_made_from() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        let scan = c.begin_scan(r"D:\x").unwrap();
        // New: pending.
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let first = next_work(&c);
        let id = first.id;
        assert!(state(&c, id).0);
        // Analyzed as it is: done. A failure counts as done; only --retry redoes it.
        analyzed(&mut c, &first);
        assert!(!state(&c, id).0);
        assert!(c.pending_import(false).unwrap().is_empty());
        assert_eq!(c.pending_import(true).unwrap().len(), 1);
        // A rescan that changes nothing changes nothing, for the import or the thumbnails.
        let seq = state(&c, id).1;
        scanned(&mut c, scan, "a.jpg", 10, 100);
        assert_eq!(state(&c, id), (false, seq));
        // Changed while it was being analyzed: what's stored is of the old file.
        let old = c.pending_import(true).unwrap().items.remove(0);
        scanned(&mut c, scan, "a.jpg", 11, 101);
        analyzed(&mut c, &old);
        assert!(state(&c, id).0);
        let current = next_work(&c);
        analyzed(&mut c, &current);
        assert!(!state(&c, id).0);
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
        assert!(!state(&c, id).0);
        rewritten(&c, (12, 102), (13, 103), None);
        assert!(state(&c, id).0);
        let rewritten = next_work(&c);
        analyzed(&mut c, &rewritten);
        // Renamed to another kind: analyzed again as that.
        scanned(&mut c, scan, "a.mp4", 13, 103);
        let renamed = next_work(&c);
        assert_eq!(renamed.kind, Kind::Video);
        analyzed(&mut c, &renamed);
        assert!(!state(&c, id).0);
        // Gone, then back: nothing to import, but the thumbnails look at it both times.
        let seq = state(&c, id).1;
        let next = c.begin_scan(r"D:\x").unwrap();
        c.mark_missing(r"D:\x", next, &[]).unwrap();
        let gone = state(&c, id).1;
        scanned(&mut c, next, "a.mp4", 13, 103);
        assert_eq!(c.pending_import(false).unwrap().len(), 0);
        assert!(seq < gone && gone < state(&c, id).1);
    }

    #[test]
    fn a_folder_the_scan_could_not_list_keeps_its_files() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        let put = |c: &mut Catalog, scan: i64, folder: &str, name: &str, file_ref: u64| {
            let tx = c.conn.transaction().unwrap();
            let mut writer = ScanWriter::new(&tx, scan).unwrap();
            let folder = writer.folder(folder).unwrap();
            let ext = Ext::of_name(name).unwrap();
            let identity = FileIdentity::new(1, file_ref);
            writer.upsert_file(&folder, &ScannedFile { identity, name, ext, size: 1, mtime: 1 }).unwrap();
            drop(writer);
            tx.commit().unwrap();
        };
        let missing = |c: &Catalog| -> i64 {
            c.conn.query_row("SELECT count(*) FROM files WHERE missing = 1", [], |r| r.get(0)).unwrap()
        };
        let first = c.begin_scan(r"D:\x").unwrap();
        put(&mut c, first, r"D:\x\ok", "a.jpg", 1);
        put(&mut c, first, r"D:\x\locked", "b.jpg", 2);
        put(&mut c, first, r"D:\x\locked\deep", "c.jpg", 3);

        // The next scan lists only D:\x\ok: D:\x\locked couldn't be read, so its files weren't seen, and aren't missing.
        let next = c.begin_scan(r"D:\x").unwrap();
        put(&mut c, next, r"D:\x\ok", "a.jpg", 1);
        assert_eq!(c.mark_missing(r"D:\x", next, &[r"D:\x\locked".to_string()]).unwrap(), 0);
        assert_eq!(missing(&c), 0);

        // Once it can be listed again, a scan that doesn't see its files marks them.
        let later = c.begin_scan(r"D:\x").unwrap();
        put(&mut c, later, r"D:\x\ok", "a.jpg", 1);
        assert_eq!(c.mark_missing(r"D:\x", later, &[]).unwrap(), 2);
        assert_eq!(missing(&c), 2);
    }

    #[test]
    fn files_marked_are_imported_again_once() {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions()).unwrap();
        c.conn
            .execute_batch(
                "INSERT INTO scans(id, root, started) VALUES (1, 'D:', 0);
                 INSERT INTO folders(id, path) VALUES (1, 'D:\\x'), (2, 'D:\\x\\y'), (3, 'D:\\xy'), (4, 'D:\\x\\y\\z');",
            )
            .unwrap();
        let files = [(1, 1, "a.jpg"), (2, 2, "b.jpg"), (3, 3, "c.jpg"), (4, 4, "d.jpg"), (5, 4, "e.jpg")];
        let files =
            files.map(|(id, folder, name)| fixture::File::new(FileId::new(id), FolderId::new(folder), name, 10, 100));
        fixture::insert(&c, &files);
        for item in c.pending_import(false).unwrap().items {
            analyzed(&mut c, &item);
        }
        // Analyzed, then gone.
        c.conn.execute("UPDATE files SET missing = 1 WHERE id = 5", []).unwrap();
        assert!(c.pending_import(false).unwrap().is_empty());

        // A folder holds the files of its subfolders, not those of a folder named like it; missing ones aren't there.
        let mut under = c.files_under(r"D:\x\y").unwrap();
        under.sort();
        assert_eq!(under, [FileId::new(2), FileId::new(4)]);
        assert_eq!(c.files_under(r"D:\x").unwrap().len(), 3);
        assert!(c.files_under(r"D:\q").unwrap().is_empty());

        // Marked: due, whatever their analysis, the missing one left; the change number doesn't move.
        let seq = state(&c, FileId::new(2)).1;
        assert_eq!(c.mark_pending(&[FileId::new(2), FileId::new(4), FileId::new(5)]).unwrap(), 2);
        assert_eq!(state(&c, FileId::new(2)), (true, seq));
        assert!(state(&c, FileId::new(4)).0 && !state(&c, FileId::new(5)).0 && !state(&c, FileId::new(1)).0);
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
        c.conn
            .execute_batch(&format!(
                "UPDATE meta SET value = {0} WHERE key = 'import_version';
                 UPDATE analysis SET version = {0};",
                IMPORT_VERSION - 1
            ))
            .unwrap();
        let (pending, seq) = state(&c, item.id);
        assert!(!pending, "current for that build");
        set_import_version(&c.conn, IMPORT_VERSION).unwrap();
        let (pending, after) = state(&c, item.id);
        assert!(pending && after > seq);
        // Done once.
        set_import_version(&c.conn, IMPORT_VERSION).unwrap();
        assert_eq!(state(&c, item.id).1, after);
    }

    #[test]
    fn a_round_with_nothing_to_import_reads_one_index() {
        let (c, _) = one_file();
        let plan: Vec<String> = c
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {}", pending_import_sql(false)))
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let plan = plan.join("\n");
        assert!(plan.contains("USING INDEX files_to_import") && !plan.contains("TEMP B-TREE"), "{plan}");
        let retry: Vec<String> = c
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {}", pending_import_sql(true)))
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let retry = retry.join("\n");
        assert!(retry.contains("files_to_import") && retry.contains("analysis_failed"), "{retry}");
    }

    fn rating(c: &Catalog, id: i64) -> Option<u8> {
        c.conn.query_row("SELECT rating FROM ratings WHERE file_id = ?1", [id], |r| r.get(0)).optional().unwrap()
    }

    fn set_rating(c: &Catalog, id: i64, n: u8) {
        let stars = Rating::new(n).filter(|r| r.get() > 0);
        writes::set_rating(c, FileId::new(id), stars, false).unwrap();
    }

    #[test]
    fn three_way_rating_sync() {
        let (c, id) = one_file();
        let sync = |in_file| {
            let tx = c.conn.unchecked_transaction().unwrap();
            AnalysisWriter::new(&tx, versions().import).unwrap().sync_file_rating(FileId::new(id), in_file).unwrap();
            tx.commit().unwrap();
        };
        // First import of a file rated elsewhere (ImageRater, Explorer): adopted.
        sync(Rating::new(4));
        assert_eq!(rating(&c, id), Some(4));
        // Rated in the app, not written back (or the format can't hold it): the file still says 4, as known, so
        // re-imports keep the catalog's 2.
        set_rating(&c, id, 2);
        sync(Rating::new(4));
        assert_eq!(rating(&c, id), Some(2));
        // Another program changes the file's rating: that is newer, and adopted.
        sync(Rating::new(5));
        assert_eq!(rating(&c, id), Some(5));
        // Another program clears it (writes 0, or removes it): the catalog follows.
        sync(Rating::new(0));
        assert_eq!(rating(&c, id), None);
        sync(None);
        assert_eq!(rating(&c, id), None);
        // A write of the app's own still queued wins over the file.
        set_rating(&c, id, 3);
        c.conn
            .execute("INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rating', 3, 0)", [id])
            .unwrap();
        sync(Rating::new(1));
        assert_eq!(rating(&c, id), Some(3));
    }
}
