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

use std::path::Path;
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

/// Migrations (`PRAGMA user_version` counts them). A new catalog gets [`BASELINE`] at version [`BASELINE_VERSION`],
/// then every migration in turn; `MIGRATIONS[i]` takes version `BASELINE_VERSION + i` to the next.
///
/// Until there is a catalog worth keeping (the full-library run at the latest, PLAN M4), schema changes edit
/// the baseline in place, without a new version, and a catalog made before the edit is moved aside when it is
/// opened ([`move_aside`]; a scan rebuilds it). From then on the baseline is frozen and every change is a migration.
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
    let sep = rules.sep() as u8;
    let bytes = rules.join(root);
    let (lo, hi) = if bytes.last() == Some(&sep) {
        let mut hi = bytes[..bytes.len() - 1].to_vec();
        hi.push(sep + 1);
        (bytes.clone(), hi)
    } else {
        let mut lo = bytes.clone();
        lo.push(sep);
        let mut hi = bytes.clone();
        hi.push(sep + 1);
        (lo, hi)
    };
    [bytes, lo, hi]
}

mod pair;
pub use pair::Pairing;

const BASELINE: &str = concat!(
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
                                           -- in place (pg_metadata::rotation_method); applied after EXIF orientation
    order_key BLOB NOT NULL,     -- where it sorts by name, all drives merged (path::order_key)
    pending   INTEGER NOT NULL DEFAULT 1,  -- the import has to analyze it (again): kept by the triggers below
    seq       INTEGER NOT NULL DEFAULT 0,  -- meta.seq when its thumbnails' source last changed: kept by triggers
    changed   INTEGER NOT NULL DEFAULT 0,  -- meta.seq at its last user-visible change (changes_since): kept by triggers
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
     + (SELECT count(*) = 2 FROM pragma_table_info('files') WHERE name IN ('created', 'unpaired'))
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
/// The parts [`LAYOUT_CHECK`] counts when the catalog is in the current layout (26: `files.created` and
/// `files.unpaired` with the nullable identity; the S-17.7 layout, which the earlier catalogs can't be converted to).
const LAYOUT_PARTS: i64 = 26;

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
}

/// The `meta.path_rules` value of a rule set.
fn rules_name(rules: PathRules) -> &'static str {
    match rules {
        PathRules::Windows => "windows",
        PathRules::Posix => "posix",
    }
}

/// Refuse a catalog whose paths were stored under other rules, or before the rules were recorded (no row): its
/// bytes don't read under these. Its way forward is to be rebuilt: deleted and scanned again.
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

/// Record the rules a new catalog's paths are stored under (`meta.path_rules`).
fn record_rules(conn: &Connection, rules: PathRules) -> Result<()> {
    conn.execute("INSERT INTO meta(key, value) VALUES ('path_rules', ?1)", [rules_name(rules)])?;
    Ok(())
}

/// Move the catalog at `path`, with its `-wal` and `-shm` files, to `*.pre-s17-7` beside it (a number follows that
/// suffix when it is taken). Nothing is deleted: the user removes the moved files when they are no longer wanted. No
/// connection to the catalog may be open. The thumbnail cache needs no move: it is reset by the new catalog's id.
fn move_aside(path: &Path) -> Result<()> {
    const PARTS: [&str; 3] = ["", "-wal", "-shm"];
    let with = |suffix: &str| -> std::path::PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        std::path::PathBuf::from(name)
    };
    let mut n = 0u32;
    let base = loop {
        let base = if n == 0 { ".pre-s17-7".to_string() } else { format!(".pre-s17-7.{n}") };
        if PARTS.iter().all(|part| !with(&format!("{base}{part}")).exists()) {
            break base;
        }
        n += 1;
    };
    for part in PARTS {
        let from = with(part);
        if from.exists() {
            std::fs::rename(&from, with(&format!("{base}{part}")))
                .with_context(|| format!("can't move {} aside", from.display()))?;
        }
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

    /// Prepare the catalog at `path` for this build, creating it when new. One made with an earlier layout is moved
    /// aside (`move_aside`) and a new one is made in its place, which the next scan fills: its identities can't be
    /// converted (S-17.7 decision 1). Its version marks are set to `versions`. The library calls this once, before
    /// anything else opens the catalog.
    pub fn prepare(path: &Path, versions: Versions, rules: PathRules) -> Result<Catalog> {
        // A new catalog records its rules; an existing one must have been made under them.
        let created = !path.exists();
        let mut conn = connect(path)?;
        if created {
            record_rules(&conn, rules)?;
        }
        check_rules(&conn, path, rules)?;
        if !current_layout(&conn)? {
            drop(conn);
            move_aside(path).with_context(|| format!("catalog {}: moving it aside", path.display()))?;
            conn = connect(path)?;
            record_rules(&conn, rules)?;
            if !current_layout(&conn)? {
                return Err(Error::Other(anyhow!(
                    "catalog {}: the new catalog is not in the current layout",
                    path.display()
                )));
            }
        }
        set_import_version(&conn, versions.import).with_context(|| format!("catalog {}", path.display()))?;
        set_borders_version(&conn, versions.borders).with_context(|| format!("catalog {}", path.display()))?;
        crate::query::register_functions(&conn)?;
        Ok(Catalog { conn, rules })
    }

    /// Open a catalog this process has prepared ([`Catalog::prepare`], which makes it, brings it to the current layout
    /// and sets the build's version marks): the settings and the SQL functions only. Every connection to it (the
    /// app's threads, the background job, the writer), which would otherwise check the layout and the marks again
    /// each time. An error if the catalog isn't there, or its schema isn't this build's (another program changed it
    /// since).
    pub fn open(path: &Path, rules: PathRules) -> Result<Catalog> {
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
        check_rules(&conn, path, rules)?;
        crate::query::register_functions(&conn)?;
        Ok(Catalog { conn, rules })
    }

    /// The path rules the catalog's paths are stored under.
    pub fn rules(&self) -> PathRules {
        self.rules
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
    pub fn file_folder(&self, file_id: FileId) -> Option<RulePath> {
        let bytes: Vec<u8> = self
            .conn
            .query_row(
                "SELECT fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
                [file_id.get()],
                |r| r.get(0),
            )
            .ok()?;
        self.rules.parse_bytes(&bytes)
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
    pub fn cancel_when(&self, cancel: Arc<AtomicBool>) {
        // Checked every 1000 virtual-machine steps: microseconds.
        self.conn.progress_handler(1000, Some(move || cancel.load(Ordering::Relaxed)));
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
    /// nothing there to compare with (R-45).
    pub fn mark_missing(&self, root: &RulePath, scan: i64, unreadable: &[RulePath]) -> Result<usize> {
        let under = |path: &RulePath| -> Result<Vec<i64>> {
            let mut stmt = self.conn.prepare(under_root!())?;
            let [p1, p2, p3] = under_params(self.rules, path);
            Ok(stmt.query_map(params![p1, p2, p3], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
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
        let roots = self.roots()?;
        if roots.iter().any(|r| self.rules.is_under(root, &r.path)) {
            return Ok(false);
        }
        for r in roots.iter().filter(|r| self.rules.is_under(&r.path, root)) {
            self.conn.execute("DELETE FROM roots WHERE id = ?1", [r.id.get()])?;
        }
        self.conn.execute("INSERT INTO roots(path, added) VALUES (?1, unixepoch())", [self.rules.join(root)])?;
        Ok(true)
    }

    /// Take a folder out of the library. Its files are marked missing, not deleted: adding it back (or the drive
    /// returning) brings them back with their analysis and ratings. Returns how many files it hid; `None`, with
    /// nothing changed, when `root` is not a library folder.
    pub fn remove_root(&self, root: RootId) -> Result<Option<usize>> {
        let tx = self.conn.unchecked_transaction()?;
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
                let path = self.rules.sep() as u8;
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
    rules: PathRules,
}

impl Catalog {
    /// Begin a transaction on the catalog (see [`Tx`]).
    pub fn tx(&mut self) -> Result<Tx<'_>> {
        Ok(Tx { inner: self.conn.transaction()?, rules: self.rules })
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
    let (volume, file_ref) = (f.identity.volume as i64, f.identity.file_index as i64);
    let (ext, kind) = (f.ext.name(), f.ext.kind().key());
    Ok(file.execute(params![
        volume,
        file_ref,
        f.created,
        folder.id.get(),
        f.name.as_bytes(),
        ext,
        kind,
        f.size as i64,
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
        let (volume, file_ref) = (f.identity.volume as i64, f.identity.file_index as i64);
        let (ext, kind, size) = (f.ext.name(), f.ext.kind().key(), f.size as i64);
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
        self.seen_identity.execute(params![identity.volume as i64, identity.file_index as i64, self.scan])?;
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
    use crate::fixture::raw;
    use pg_analysis::analyze::IMPORT_VERSION;
    use pg_analysis::borders::BORDERS_VERSION;
    use pg_model::analysis::{Analysis, ImageFacts, Media, PixelHashes, ThumbHash};
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
    const SCRATCH: &str = r"D:\PhotoGallery\scratch_catalog";

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
        let dir = test_folder(SCRATCH, "move-aside");
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
        assert!(current_layout(&disk).unwrap());
        let count = |conn: &Connection, table: &str| -> i64 {
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
        };
        // The new catalog is empty: the scan fills it. The earlier one is kept whole, rows and all, beside it.
        assert_eq!(count(&disk, "files"), 0);
        assert_eq!(count(&disk, "roots"), 0);
        let moved = raw::on_disk(&dir.join("catalog.sqlite.pre-s17-7"));
        assert_eq!(count(&moved, "files"), 4);
        assert_eq!(count(&moved, "roots"), 1);
        drop(moved);
        drop(disk);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside() {
        let dir = test_folder(SCRATCH, "layout_source");
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), RULES).unwrap());
        let fresh = raw::on_disk(&path);
        assert!(current_layout(&fresh).unwrap());
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
        assert!(!current_layout(&earlier).unwrap());
        drop(earlier);

        let c = Catalog::prepare(&path, versions(), RULES).unwrap();
        let disk = raw::on_disk(&path);
        assert!(current_layout(&disk).unwrap());
        let current: String = disk
            .query_row(
                "SELECT group_concat(sql, ' ') FROM sqlite_schema WHERE name IN ('files_source_changed', 'thumb_sources')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(current.contains("old.size IS NOT new.size") && current.contains("f.size, f.missing"), "{current}");
        assert!(dir.join("catalog.sqlite.pre-s17-7").exists());
        drop(disk);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name() {
        let dir = test_folder(SCRATCH, "move-aside-twice");
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
        let moved = raw::on_disk(&dir.join("catalog.sqlite.pre-s17-7"));
        assert_eq!(moved.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(moved);
        // Another one moved aside later keeps the first: the new catalog made in the first one's place goes first.
        for part in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(dir.join(format!("catalog.sqlite{part}")));
        }
        old_layout(&path);
        drop(Catalog::prepare(&path, versions(), RULES).unwrap());
        assert!(dir.join("catalog.sqlite.pre-s17-7.1").exists());
        assert!(dir.join("catalog.sqlite.pre-s17-7").exists());
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
    fn a_catalog_made_under_other_rules_or_with_none_recorded_is_refused() {
        let dir = test_folder(SCRATCH, "path-rules");
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PathRules::Windows).unwrap());
        // Another build's rules: its paths would read as other paths.
        let e = Catalog::open(&path, PathRules::Posix).err().expect("refused");
        assert!(matches!(e, Error::PathRules { .. }), "{e:?}");
        assert!(e.to_string().contains("rebuild it"), "{e}");
        assert!(Catalog::prepare(&path, versions(), PathRules::Posix).is_err());
        // A catalog whose rules were never recorded.
        raw::on_disk(&path).execute("DELETE FROM meta WHERE key = 'path_rules'", []).unwrap();
        assert!(matches!(Catalog::open(&path, PathRules::Windows), Err(Error::PathRules { .. })));
        assert!(matches!(Catalog::prepare(&path, versions(), PathRules::Windows), Err(Error::PathRules { .. })));
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
        );
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
        let c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        c.cancel_when(cancel.clone());
        let run = || raw::slow_count(&c).map_err(Error::from);
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
        let c = Catalog::prepare(Path::new(":memory:"), versions(), RULES).unwrap();
        raw::add_scan(&c, 1, "D:");
        raw::add_folder(&c, 1, r"D:\x");
        fixture::insert(&c, &[fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]);
        (c, 1)
    }

    /// A turn the file can't hold, written by the writer: the catalog keeps it.
    fn turn_kept(c: &Catalog, id: FileId, turns: i64) {
        let job =
            writes::Job { file_id: id, kind: pg_model::write::WriteKind::Turns, value: turns, temp: None, attempts: 0 };
        writes::done(c, &job, writes::WriteEffect::KeptInCatalog(Turns::new(turns as u8))).unwrap();
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
            assert_eq!(raw::file_row(&c), (scan, name.into(), size as i64, mtime, 0, key(name)));
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
        assert_eq!((stored.0 as u64, stored.1 as u64), (volume, file));
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
        insert(&c, &[File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]);
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
        );
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
                identity: FileIdentity::new(1, 10 + n as u64),
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
                file_hash: [n as u8; 32],
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
        fixture::insert(&c, &files);
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
