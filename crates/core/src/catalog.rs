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

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::analyze::{Analysis, IMPORT_VERSION};
use crate::borders::{BORDERS_VERSION, Detection};
use crate::media::{self, Kind};
use crate::pdq;
use crate::pdq_index::PdqIndex;

/// Migrations (`PRAGMA user_version` counts them). A new catalog gets [`baseline`] at version [`BASELINE_VERSION`],
/// then every migration in turn; `MIGRATIONS[i]` takes version `BASELINE_VERSION + i` to the next.
///
/// Until there is a catalog worth keeping (the full-library run at the latest, PLAN M4), schema changes edit
/// the baseline in place, without a new version, and a catalog made before the edit is copied into a new one when
/// it is opened ([`rebuild`]). From then on the baseline is frozen and every change is a migration.
const BASELINE_VERSION: i64 = 1;
const MIGRATIONS: &[&str] = &[];

/// Whether the `files` row being written needs its analysis (re)done: no analysis made from it as it is now (at
/// `meta.import_version` or later, from the same size, mtime, catalog rotation and kind). A failure counts as made:
/// `--retry` redoes those. Inside `UPDATE files … WHERE id = …`.
const STALE: &str = "NOT EXISTS (SELECT 1 FROM analysis a WHERE a.file_id = files.id
        AND a.version >= (SELECT value FROM meta WHERE key = 'import_version')
        AND a.size = files.size AND a.mtime = files.mtime AND a.rotation = files.rotation AND a.kind = files.kind)";

/// The next number of the change sequence, for `files.seq`: `{NEXT_SEQ}` first, then `seq = {SEQ}`.
const NEXT_SEQ: &str = "UPDATE meta SET value = value + 1 WHERE key = 'seq'";
const SEQ: &str = "(SELECT value FROM meta WHERE key = 'seq')";

fn baseline() -> String {
    BASELINE.replace("{STALE}", STALE).replace("{NEXT_SEQ}", NEXT_SEQ).replace("{SEQ}", SEQ)
}

const BASELINE: &str = r"
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
    ext       TEXT NOT NULL,     -- the name's last extension, lower case (media::extension)
    kind      TEXT NOT NULL,     -- media::Kind::key, from the extension: 'image' | 'video'
    size      INTEGER NOT NULL,
    mtime     INTEGER NOT NULL,  -- FILETIME: 100 ns since 1601 UTC
    last_scan INTEGER NOT NULL REFERENCES scans(id),
    missing   INTEGER NOT NULL DEFAULT 0,
    rotation  INTEGER NOT NULL DEFAULT 0,  -- quarter turns clockwise kept here, for files that can't be rotated
                                           -- in place (metadata::rotation_method); applied after EXIF orientation
    order_key BLOB NOT NULL,     -- where it sorts by name, all drives merged (browse::order_key)
    pending   INTEGER NOT NULL DEFAULT 1,  -- the import has to analyze it (again): kept by the triggers below
    seq       INTEGER NOT NULL DEFAULT 0,  -- meta.seq when its thumbnails' source last changed: kept by triggers
    UNIQUE(volume, file_ref)
);
CREATE INDEX files_folder ON files(folder_id, name);
-- The grid's name order and every folder's files (a range of keys: browse::scope).
CREATE INDEX files_order ON files(order_key) WHERE missing = 0;
-- The folder tree's counts, from the index alone.
CREATE INDEX files_present ON files(folder_id) WHERE missing = 0;
-- The import's work in on-disk order (Catalog::pending_import): a round with nothing to import is one probe.
CREATE INDEX files_to_import ON files(volume, file_ref & 0xFFFFFFFFFFFF) WHERE missing = 0 AND pending = 1;
-- The files changed since the thumbnail pass last looked (thumbs::pending_in).
CREATE INDEX files_seq ON files(seq);
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
    thumbhash   BLOB,
    frames      INTEGER,           -- frames in the file: > 1 is an animated image (or a video)
    taken       INTEGER,           -- when it was taken, as wall-clock time (no time zone) in Unix seconds
    duration_ms INTEGER,           -- videos
    poster_ms   INTEGER,           -- videos: where the frame for the ThumbHash and the thumbnails is
    -- Borders (borders::detect) on the stored pixels, found after the import by a pass of their own: what a crop
    -- cuts from the left, top, right and bottom (borders::Detection::cut), all 0 for none. NULL until looked for,
    -- and where they aren't (videos, animated and tiny images, failures). `borders_due`: to be looked for, by this
    -- build's detector (the import sets it; Catalog::open too when meta.borders_version differs).
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
    rejected TEXT,              -- borders::Reason key; NULL: accepted
    PRIMARY KEY(file_id, pass, edge)
) WITHOUT ROWID;
-- The class facet (query::BorderClass).
CREATE INDEX border_sides_class ON border_sides(class, file_id) WHERE rejected IS NULL;
-- Files whose borders the user marked as not a border (the review). Kept when the borders are looked for again.
CREATE TABLE border_marks(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    marked  INTEGER NOT NULL  -- Unix seconds
);
-- Tier 1 candidates: PDQ distance <= pdq::MATCH_DISTANCE between reliable hashes, any of the 8 orientations.
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

-- The library: folders the user added, which a rescan walks again. None is inside another.
CREATE TABLE roots(
    path  TEXT PRIMARY KEY,  -- absolute, no trailing backslash
    added INTEGER NOT NULL   -- Unix seconds
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
    folder    TEXT NOT NULL,     -- merged path (browse::merged_path); '' = all folders
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
-- computed for (Catalog::open recomputes it when the build's differs). `seq` is the change sequence's last number.
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
    {NEXT_SEQ};
    UPDATE files SET pending = {STALE}, seq = {SEQ} WHERE id = new.id;
END;
CREATE TRIGGER files_changed AFTER UPDATE OF size, mtime, rotation, kind ON files
WHEN old.size IS NOT new.size OR old.mtime IS NOT new.mtime OR old.rotation IS NOT new.rotation
    OR old.kind IS NOT new.kind
BEGIN
    UPDATE files SET pending = {STALE} WHERE id = new.id;
END;
-- What a thumbnail is made from (thumbs::Source), and whether the file is there to show.
CREATE TRIGGER files_source_changed AFTER UPDATE OF mtime, rotation, missing ON files
WHEN old.mtime IS NOT new.mtime OR old.rotation IS NOT new.rotation OR old.missing IS NOT new.missing
BEGIN
    {NEXT_SEQ};
    UPDATE files SET seq = {SEQ} WHERE id = new.id;
END;
-- A REPLACE fires only this one (recursive triggers are off): its delete half fires no trigger.
CREATE TRIGGER analysis_added AFTER INSERT ON analysis BEGIN
    {NEXT_SEQ};
    UPDATE files SET pending = {STALE}, seq = {SEQ} WHERE id = new.file_id;
END;
-- Besides what pending depends on: whether it decoded (a thumbnail needs that) and a video's poster frame.
CREATE TRIGGER analysis_changed AFTER UPDATE OF version, size, mtime, rotation, kind, error, poster_ms ON analysis
WHEN old.version IS NOT new.version OR old.size IS NOT new.size OR old.mtime IS NOT new.mtime
    OR old.rotation IS NOT new.rotation OR old.kind IS NOT new.kind OR old.error IS NOT new.error
    OR old.poster_ms IS NOT new.poster_ms
BEGIN
    {NEXT_SEQ};
    UPDATE files SET pending = {STALE}, seq = {SEQ} WHERE id = new.file_id;
END;
CREATE TRIGGER analysis_removed AFTER DELETE ON analysis BEGIN
    UPDATE files SET pending = 1 WHERE id = old.file_id;
END;
";

/// A catalog made before the current baseline (pre-release, the baseline is edited in place) lacks some of these
/// `LAYOUT_PARTS`.
const LAYOUT_CHECK: &str = "SELECT (SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'meta')
     + (SELECT count(*) FROM pragma_table_info('files') WHERE name IN ('order_key', 'pending', 'seq'))
     + (SELECT count(*) FROM pragma_table_info('analysis') WHERE name IN ('rotation', 'kind', 'borders_due'))
     + (SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('border_sides', 'border_marks'))";
const LAYOUT_PARTS: i64 = 9;

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
];

/// Folders equal to root ?1 or below it: path in [root\, root]), as ']' is the character after '\'.
const UNDER_ROOT: &str = "SELECT id FROM folders WHERE path = ?1 OR (path >= ?1 || '\\' AND path < ?1 || ']')";

/// Whether `path` is `root` or inside it (Windows paths: case-insensitive).
pub fn is_under(path: &str, root: &str) -> bool {
    let (p, r) = (path.to_lowercase(), root.to_lowercase());
    p == r || p.strip_prefix(&r).is_some_and(|rest| rest.starts_with('\\'))
}

pub struct Catalog {
    pub conn: Connection,
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
        bail!("schema version {version}, which this build doesn't know (it has {current}): made by another build");
    }
    for (i, sql) in migrations.iter().enumerate().skip((version - BASELINE_VERSION) as usize) {
        let to = BASELINE_VERSION + i as i64 + 1;
        step(sql, to).with_context(|| format!("migration to schema version {to}"))?;
    }
    Ok(())
}

/// A connection to the catalog at `path`, created with the baseline when new.
fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
         PRAGMA cache_size = -262144; PRAGMA temp_store = MEMORY;",
    )?;
    migrate(&conn, &baseline(), MIGRATIONS).with_context(|| format!("catalog {}", path.display()))?;
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
            bail!(IN_USE);
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
            Err(e) => return Err(anyhow::Error::from(e).context(format!("deleting {}: {IN_USE}", side.display()))),
        }
    }
    // Windows refuses to move over a file another program has open.
    std::fs::rename(&new_path, path).with_context(|| format!("moving {} over it; {IN_USE}", new_path.display()))?;
    Ok(())
}

/// [`rebuild`]'s copy of the catalog at `path`, made at `new_path`.
fn build_copy(path: &Path, new_path: &Path) -> Result<()> {
    let new = Connection::open(new_path)?;
    migrate(&new, &baseline(), MIGRATIONS)?;
    new.create_scalar_function(
        "pg_order_key",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let text = |i| ctx.get_raw(i).as_str().map_err(|e| rusqlite::Error::UserFunctionError(e.into()));
            Ok(crate::browse::order_key(text(0)?, text(1)?))
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
            bail!("{table}.{column}: not in this catalog, and nothing to make it from");
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
        bail!("copying {table}: {} rows of {}", count("main")?, count("old")?);
    }
    Ok(())
}

/// Make `files.pending` mean this build's IMPORT_VERSION: when the catalog's `meta.import_version` is another
/// (a new catalog, or a build that analyzes differently), every file's is computed again, and every file's `seq`
/// moves on, so the thumbnail pass looks at all of them once. Checked and set in one write transaction, so two
/// processes opening the catalog at once do it once.
fn set_import_version(conn: &Connection) -> Result<()> {
    let version = |c: &Connection| -> Result<i64> {
        Ok(c.query_row("SELECT value FROM meta WHERE key = 'import_version'", [], |r| r.get(0))?)
    };
    if version(conn)? == IMPORT_VERSION {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if version(&tx)? != IMPORT_VERSION {
        tx.execute("UPDATE meta SET value = ?1 WHERE key = 'import_version'", [IMPORT_VERSION])?;
        tx.execute_batch(&format!("UPDATE files SET pending = {STALE}; {NEXT_SEQ}; UPDATE files SET seq = {SEQ};"))?;
    }
    Ok(tx.commit()?)
}

/// The analyses borders are looked for in: still images that decoded and are big enough (borders::MIN_SIDE).
const HAS_BORDERS: &str = "error IS NULL AND kind = 'image' AND frames <= 1 AND min(width, height) >= 64";

/// Make `analysis.borders_due` mean this build's detector: when the catalog's `meta.borders_version` is another,
/// every analysis that can have borders is due. Like [`set_import_version`], in one write transaction.
fn set_borders_version(conn: &Connection) -> Result<()> {
    let version = |c: &Connection| -> Result<i64> {
        Ok(c.query_row("SELECT value FROM meta WHERE key = 'borders_version'", [], |r| r.get(0))?)
    };
    if version(conn)? == BORDERS_VERSION {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if version(&tx)? != BORDERS_VERSION {
        tx.execute("UPDATE meta SET value = ?1 WHERE key = 'borders_version'", [BORDERS_VERSION])?;
        tx.execute_batch(&format!("UPDATE analysis SET borders_due = coalesce({HAS_BORDERS}, 0)"))?;
    }
    Ok(tx.commit()?)
}

/// `Catalog::pending_import`'s query.
fn pending_import_sql(retry_errors: bool) -> String {
    const COLUMNS: &str = "f.id, (SELECT path FROM folders WHERE id = f.folder_id) || '\\' || f.name, f.size,
        f.mtime, f.rotation, f.kind, f.volume AS volume, f.file_ref & 0xFFFFFFFFFFFF AS record";
    // `missing = 0 AND pending = 1` written out: the index's condition.
    let pending = format!("SELECT {COLUMNS} FROM files f WHERE f.missing = 0 AND f.pending = 1");
    if retry_errors {
        // A UNION, as an OR would scan every file; the failures are read from their index first.
        format!(
            "{pending} UNION SELECT {COLUMNS} FROM analysis a CROSS JOIN files f
             WHERE a.error IS NOT NULL AND f.id = a.file_id AND f.missing = 0 ORDER BY volume, record"
        )
    } else {
        format!("{pending} ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF")
    }
}

/// A file as the scanner saw it.
pub struct ScannedFile<'a> {
    pub volume: u32,
    pub file_ref: u64,
    pub name: &'a str,
    pub kind: Kind,
    pub size: u64,
    pub mtime: i64,
}

/// A folder's row, with its part of its files' order key (`browse::folder_key`).
pub struct Folder {
    pub id: i64,
    pub key: Vec<u8>,
}

/// A file the import still has to analyze.
pub struct WorkItem {
    pub id: i64,
    pub path: String,
    pub size: i64,
    pub mtime: i64,
    /// Catalog rotation (`files.rotation`), applied when decoding.
    pub rotation: u8,
    pub kind: Kind,
}

/// A file with borders to crop off ([`Catalog::crop_source`]).
pub struct CropSource {
    pub path: String,
    /// The file as analyzed.
    pub size: i64,
    pub mtime: i64,
    /// `decode::Format::name`.
    pub format: String,
    /// The stored size.
    pub width: u32,
    pub height: u32,
    /// Left, top, right, bottom, stored pixels.
    pub cut: [u32; 4],
}

/// Whether an error is a statement stopped by [`Catalog::cancel_when`]'s flag.
pub fn interrupted(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<rusqlite::Error>()
            .and_then(rusqlite::Error::sqlite_error_code)
            .is_some_and(|code| code == rusqlite::ErrorCode::OperationInterrupted)
    })
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
    /// Open the catalog at `path`, creating it when new. One made with an earlier layout is copied into the current
    /// one first ([`rebuild`]): seconds for 190k files.
    pub fn open(path: &Path) -> Result<Catalog> {
        let mut conn = connect(path)?;
        if !current_layout(&conn)? {
            drop(conn);
            rebuild(path).with_context(|| format!("catalog {}: copying it into the current layout", path.display()))?;
            conn = connect(path)?;
            if !current_layout(&conn)? {
                bail!("catalog {}: still not in the current layout after copying it", path.display());
            }
        }
        set_import_version(&conn).with_context(|| format!("catalog {}", path.display()))?;
        set_borders_version(&conn).with_context(|| format!("catalog {}", path.display()))?;
        crate::query::register_functions(&conn)?;
        Ok(Catalog { conn })
    }

    /// The id made with the catalog (`meta.catalog_id`).
    pub fn catalog_id(&self) -> Result<String> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = 'catalog_id'", [], |r| r.get(0))?)
    }

    /// Stop the statements on this connection while `cancel` is set (from any thread): each fails with an error for
    /// which [`interrupted`] is true, including those started after it was set, until it is cleared. Unlike
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

    /// The folder's row, made if it's new.
    pub fn folder(tx: &Transaction, path: &str) -> Result<Folder> {
        let key = crate::browse::folder_key(crate::browse::merged_path(path));
        let mut get = tx.prepare_cached("SELECT id FROM folders WHERE path = ?1")?;
        if let Some(id) = get.query_row([path], |r| r.get(0)).optional()? {
            return Ok(Folder { id, key });
        }
        tx.prepare_cached("INSERT INTO folders(path) VALUES (?1)")?.execute([path])?;
        Ok(Folder { id: tx.last_insert_rowid(), key })
    }

    /// Insert or update by file identity; a moved or renamed file keeps its row (and its analysis).
    pub fn upsert_file(tx: &Transaction, scan: i64, folder: &Folder, f: &ScannedFile) -> Result<()> {
        // `pending` and `seq` follow from the triggers.
        let order_key = [folder.key.as_slice(), &crate::browse::name_key(f.name)].concat();
        tx.prepare_cached(
            "INSERT INTO files(volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, order_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(volume, file_ref) DO UPDATE SET
                folder_id = excluded.folder_id, name = excluded.name, ext = excluded.ext, kind = excluded.kind,
                size = excluded.size, mtime = excluded.mtime, last_scan = excluded.last_scan, missing = 0,
                order_key = excluded.order_key",
        )?
        .execute(params![
            f.volume,
            f.file_ref as i64,
            folder.id,
            f.name,
            media::extension(f.name),
            f.kind.key(),
            f.size as i64,
            f.mtime,
            scan,
            order_key
        ])?;
        Ok(())
    }

    /// Mark files under `root` that this scan didn't see as missing. Their rows stay (ratings, decisions).
    pub fn mark_missing(&self, root: &str, scan: i64) -> Result<usize> {
        Ok(self.conn.execute(
            &format!(
                "UPDATE files SET missing = 1 WHERE missing = 0 AND last_scan < ?2 AND folder_id IN ({UNDER_ROOT})"
            ),
            params![root, scan],
        )?)
    }

    /// The library folders, by path.
    pub fn roots(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM roots ORDER BY path")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Add a folder to the library. A folder already inside the library adds nothing (returns false); one that
    /// contains library folders replaces them.
    pub fn add_root(&self, root: &str) -> Result<bool> {
        let roots = self.roots()?;
        if roots.iter().any(|r| is_under(root, r)) {
            return Ok(false);
        }
        for r in roots.iter().filter(|r| is_under(r, root)) {
            self.conn.execute("DELETE FROM roots WHERE path = ?1", [r])?;
        }
        self.conn.execute("INSERT INTO roots(path, added) VALUES (?1, unixepoch())", [root])?;
        Ok(true)
    }

    /// Take a folder out of the library. Its files are marked missing, not deleted: adding it back (or the drive
    /// returning) brings them back with their analysis and ratings. Returns how many files it hid.
    pub fn remove_root(&self, root: &str) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM roots WHERE path = ?1", [root])?;
        let n = tx.execute(
            &format!("UPDATE files SET missing = 1 WHERE missing = 0 AND folder_id IN ({UNDER_ROOT})"),
            [root],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// Files whose analysis is absent or stale (`files.pending`), and with `retry_errors` those that failed, in
    /// on-disk order (volume, then MFT record number), which reads ~3× faster than path order on an HDD (FINDINGS
    /// "I/O"). Without `retry_errors` it's read in that order from `files_to_import`, so nothing pending is one probe.
    pub fn pending_import(&self, retry_errors: bool) -> Result<Vec<WorkItem>> {
        let mut stmt = self.conn.prepare(&pending_import_sql(retry_errors))?;
        let rows = stmt.query_map([], |r| {
            Ok(WorkItem {
                id: r.get(0)?,
                path: r.get(1)?,
                size: r.get(2)?,
                mtime: r.get(3)?,
                rotation: r.get(4)?,
                kind: Kind::from_key(&r.get::<_, String>(5)?).unwrap_or(Kind::Image),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The tier-1 index of the reliable PDQ hashes of current analyses (PLAN §6), rebuilt for each import.
    pub fn pdq_index(&self) -> Result<PdqIndex> {
        // Sized by the present files (an upper bound, counted from `files_present` alone): an exact count of the
        // join below costs a quarter of reading it.
        let present: usize = self.conn.query_row("SELECT count(*) FROM files WHERE missing = 0", [], |r| r.get(0))?;
        let (mut ids, mut hashes) = (Vec::with_capacity(present), Vec::with_capacity(present));
        let mut stmt = self.conn.prepare(
            "SELECT a.file_id, a.pdq FROM analysis a JOIN files f ON f.id = a.file_id
             WHERE f.missing = 0 AND f.pending = 0 AND a.pdq IS NOT NULL AND a.pdq_quality >= ?1",
        )?;
        let mut rows = stmt.query([pdq::MIN_QUALITY])?;
        while let Some(r) = rows.next()? {
            ids.push(r.get(0)?);
            hashes.push(pdq::Hash(r.get::<_, [u8; pdq::HASH_BYTES]>(1)?).words());
        }
        Ok(PdqIndex::build(ids, hashes))
    }

    pub fn store_analysis(tx: &Transaction, item: &WorkItem, a: &Analysis) -> Result<()> {
        // A re-analysis invalidates the file's tier-1 pairs; the caller re-queries it.
        tx.prepare_cached("DELETE FROM pdq_pairs WHERE a = ?1 OR b = ?1")?.execute([item.id])?;
        tx.prepare_cached(
            "INSERT OR REPLACE INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error,
                file_hash, pixel_hash, format, orientation, width, height, pdq, pdq_quality, thumbhash, frames,
                taken, duration_ms, poster_ms, borders_due)
             VALUES (?1, ?2, ?3, ?4, ?19, ?20, unixepoch(), NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     CASE WHEN ?16 THEN unixepoch(?15, 'unixepoch', 'localtime') ELSE ?15 END, ?17, ?18, 0)",
        )?
        .execute(params![
            item.id,
            IMPORT_VERSION,
            item.size,
            item.mtime,
            &a.file_hash[..],
            a.pixels.as_ref().map(|p| p.pixel_hash.to_vec()),
            a.format,
            a.orientation,
            a.width,
            a.height,
            a.pixels.as_ref().map(|p| p.pdq.hash.0.to_vec()),
            a.pixels.as_ref().map(|p| p.pdq.quality),
            &a.thumbhash,
            a.frames,
            a.taken,
            a.taken_utc,
            a.duration_ms,
            a.poster_ms,
            item.rotation,
            item.kind.key(),
        ])?;
        // The borders of what was there before are gone; the borders pass finds the new ones.
        tx.prepare_cached(&format!("UPDATE analysis SET borders_due = coalesce({HAS_BORDERS}, 0) WHERE file_id = ?1"))?
            .execute([item.id])?;
        tx.prepare_cached("DELETE FROM border_sides WHERE file_id = ?1")?.execute([item.id])?;
        Self::sync_file_rating(tx, item.id, a.file_rating)
    }

    /// Record the borders found in a file (`None`: not looked for), replacing what was there.
    fn write_borders(tx: &Transaction, file_id: i64, borders: Option<&Detection>) -> Result<()> {
        let cut = borders.map(Detection::cut);
        tx.prepare_cached(
            "UPDATE analysis SET cut_left = ?2, cut_top = ?3, cut_right = ?4, cut_bottom = ?5, borders_due = 0
             WHERE file_id = ?1",
        )?
        .execute(params![
            file_id,
            cut.map(|c| c[0]),
            cut.map(|c| c[1]),
            cut.map(|c| c[2]),
            cut.map(|c| c[3])
        ])?;
        tx.prepare_cached("DELETE FROM border_sides WHERE file_id = ?1")?.execute([file_id])?;
        let mut insert = tx.prepare_cached(
            "INSERT INTO border_sides(file_id, pass, edge, kind, class, depth, color, straight, dev, drift, rejected)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for s in borders.map_or(&[][..], |b| &b.sides) {
            let color = s.color.map(|[r, g, b]| ((r as i64) << 16) | ((g as i64) << 8) | b as i64);
            insert.execute(params![
                file_id,
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

    /// Analyses whose borders are due (new ones, and all after a new detector: [`set_borders_version`]), of
    /// files the import has nothing to do for, in on-disk order.
    pub fn pending_borders(&self) -> Result<Vec<WorkItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.id, (SELECT path FROM folders WHERE id = f.folder_id) || '\\' || f.name, f.size, f.mtime,
                    f.rotation, f.kind
             FROM analysis a CROSS JOIN files f
             WHERE a.borders_due = 1 AND f.id = a.file_id AND f.missing = 0 AND f.pending = 0
             ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(WorkItem {
                id: r.get(0)?,
                path: r.get(1)?,
                size: r.get(2)?,
                mtime: r.get(3)?,
                rotation: r.get(4)?,
                kind: Kind::from_key(&r.get::<_, String>(5)?).unwrap_or(Kind::Image),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Record borders looked for ([`Catalog::pending_borders`]), unless the file changed meanwhile (the import
    /// then analyzes it again, and the borders are due again).
    pub fn store_borders(tx: &Transaction, item: &WorkItem, borders: Option<&Detection>) -> Result<()> {
        let current: bool = tx
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM analysis WHERE file_id = ?1 AND size = ?2 AND mtime = ?3)")?
            .query_row(params![item.id, item.size, item.mtime], |r| r.get(0))?;
        if current {
            Self::write_borders(tx, item.id, borders)?;
        }
        Ok(())
    }

    /// The present file at an absolute path (Windows paths: any case).
    pub fn file_by_path(&self, path: &str) -> Result<Option<i64>> {
        let Some((folder, name)) = path.rsplit_once('\\') else { return Ok(None) };
        Ok(self
            .conn
            .prepare_cached(
                "SELECT f.id FROM files f JOIN folders d ON d.id = f.folder_id
                 WHERE lower(d.path) = lower(?1) AND lower(f.name) = lower(?2) AND f.missing = 0",
            )?
            .query_row([folder, name], |r| r.get(0))
            .optional()?)
    }

    /// What cropping a file's borders off needs: its borders as found in the file as it was analyzed (`size`,
    /// `mtime`: the caller checks the file is still that one). `None` when it has none to crop: none found, marked
    /// as not a border, or not looked for in the file as it is.
    pub fn crop_source(&self, file_id: i64) -> Result<Option<CropSource>> {
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
            .query_row([file_id], |r| {
                let (orientation, w, h): (u8, u32, u32) = (r.get(4)?, r.get(5)?, r.get(6)?);
                let (width, height) = if orientation >= 5 { (h, w) } else { (w, h) };
                Ok(CropSource {
                    path: r.get(0)?,
                    size: r.get(1)?,
                    mtime: r.get(2)?,
                    format: r.get(3)?,
                    width,
                    height,
                    cut: [r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?],
                })
            })
            .optional()?)
    }

    /// Mark a file's borders as not a border, or take the mark back (the review).
    pub fn set_border_mark(conn: &Connection, file_id: i64, marked: bool) -> Result<()> {
        if marked {
            conn.prepare_cached("INSERT OR REPLACE INTO border_marks(file_id, marked) VALUES (?1, unixepoch())")?
                .execute([file_id])?;
        } else {
            conn.prepare_cached("DELETE FROM border_marks WHERE file_id = ?1")?.execute([file_id])?;
        }
        Ok(())
    }

    /// Three-way rating sync. `file_ratings` holds what the file was last known to hold; a file rating that
    /// differs from it was set by another program after that, so it becomes the catalog rating. Otherwise the
    /// catalog's stands (it may be newer and not written back yet, or the file can't hold one). A write of our own
    /// still queued is newer than anything in the file: nothing is adopted then.
    fn sync_file_rating(tx: &Transaction, file_id: i64, in_file: Option<u8>) -> Result<()> {
        let known: Option<u8> = tx
            .prepare_cached("SELECT value FROM file_ratings WHERE file_id = ?1")?
            .query_row([file_id], |r| r.get(0))
            .optional()?;
        let queued: bool = tx
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM file_writes WHERE file_id = ?1 AND kind = 'rating')")?
            .query_row([file_id], |r| r.get(0))?;
        if in_file == known || queued {
            return Ok(());
        }
        match in_file {
            Some(n @ 1..=5) => tx
                .prepare_cached(
                    "INSERT INTO ratings(file_id, rating, updated) VALUES (?1, ?2, unixepoch())
                     ON CONFLICT(file_id) DO UPDATE SET rating = excluded.rating, updated = excluded.updated",
                )?
                .execute(params![file_id, n])?,
            _ => tx.prepare_cached("DELETE FROM ratings WHERE file_id = ?1")?.execute([file_id])?,
        };
        Self::set_file_rating(tx, file_id, in_file)
    }

    /// Record what the file holds now (read by the import, or just written by the app).
    pub fn set_file_rating(conn: &Connection, file_id: i64, value: Option<u8>) -> Result<()> {
        match value {
            Some(v) => conn
                .prepare_cached(
                    "INSERT INTO file_ratings(file_id, value) VALUES (?1, ?2)
                     ON CONFLICT(file_id) DO UPDATE SET value = excluded.value",
                )?
                .execute(params![file_id, v.min(5)])?,
            None => conn.prepare_cached("DELETE FROM file_ratings WHERE file_id = ?1")?.execute([file_id])?,
        };
        Ok(())
    }

    /// The app rewrote a file in place (ingest::writeback): record its new size and mtime, so the scanner and the
    /// import see it as the file it is. `before` is the (size, mtime) the file had on disk just before the write.
    /// After a metadata-only change (`metadata_only` = the new BLAKE3) an analysis made from exactly those bytes
    /// stays valid and follows the file: nothing is re-imported. After a pixel change it is left stale, so the
    /// import redoes it (new upright size, hashes, PDQ).
    pub fn file_rewritten(
        conn: &Connection,
        file_id: i64,
        before: (i64, i64),
        after: (i64, i64),
        metadata_only: Option<[u8; 32]>,
    ) -> Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute("UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1", params![file_id, after.0, after.1])?;
        if let Some(file_hash) = metadata_only {
            tx.execute(
                "UPDATE analysis SET size = ?2, mtime = ?3, file_hash = ?4
                 WHERE file_id = ?1 AND size = ?5 AND mtime = ?6",
                params![file_id, after.0, after.1, &file_hash[..], before.0, before.1],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The turn is kept in the catalog (`rotate::Outcome::Catalog`): the analysis, made without it, is stale
    /// (`files.pending`), and the thumbnails no longer match (thumbs::Source).
    pub fn add_catalog_rotation(conn: &Connection, file_id: i64, turns: u8) -> Result<()> {
        conn.execute("UPDATE files SET rotation = (rotation + ?2) % 4 WHERE id = ?1", params![file_id, turns])?;
        Ok(())
    }

    pub fn store_failure(tx: &Transaction, item: &WorkItem, error: &str) -> Result<()> {
        tx.prepare_cached("DELETE FROM pdq_pairs WHERE a = ?1 OR b = ?1")?.execute([item.id])?;
        tx.prepare_cached(
            "INSERT OR REPLACE INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error, borders_due)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch(), ?7, 0)",
        )?
        .execute(params![
            item.id,
            IMPORT_VERSION,
            item.size,
            item.mtime,
            item.rotation,
            item.kind.key(),
            error
        ])?;
        tx.prepare_cached("DELETE FROM border_sides WHERE file_id = ?1")?.execute([item.id])?;
        Ok(())
    }

    pub fn store_pdq_pair(tx: &Transaction, a: i64, b: i64, distance: u32, variant: usize) -> Result<()> {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        tx.prepare_cached(
            "INSERT INTO pdq_pairs(a, b, distance, variant) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(a, b) DO UPDATE SET distance = excluded.distance, variant = excluded.variant
             WHERE excluded.distance < pdq_pairs.distance",
        )?
        .execute(params![a, b, distance, variant])?;
        Ok(())
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

/// Test catalogs: files inserted as the scanner would, with their order keys.
#[cfg(test)]
pub(crate) mod fixture {
    use rusqlite::{Connection, params};

    /// A file row; `folder` must exist in `folders`.
    pub struct File {
        pub id: i64,
        pub folder: i64,
        pub name: &'static str,
        pub kind: &'static str,
        pub size: i64,
        pub mtime: i64,
        pub file_ref: i64,
        pub missing: bool,
        pub rotation: u8,
    }

    impl File {
        /// An image present on volume 1, its file reference its id.
        pub fn new(id: i64, folder: i64, name: &'static str, size: i64, mtime: i64) -> File {
            File { id, folder, name, kind: "image", size, mtime, file_ref: id, missing: false, rotation: 0 }
        }
    }

    /// Insert the files (scan 1 must exist).
    pub fn insert(conn: &Connection, files: &[File]) {
        for f in files {
            let folder: String =
                conn.query_row("SELECT path FROM folders WHERE id = ?1", [f.folder], |r| r.get(0)).unwrap();
            conn.execute(
                "INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, missing,
                                   rotation, order_key)
                 VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?11)",
                params![
                    f.id,
                    f.file_ref,
                    f.folder,
                    f.name,
                    crate::media::extension(f.name),
                    f.kind,
                    f.size,
                    f.mtime,
                    f.missing,
                    f.rotation,
                    crate::browse::order_key(&folder, f.name)
                ],
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A fresh folder for one test's catalog file.
    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_earlier_layout_is_copied_into_the_current_one() {
        let dir = test_dir("rebuild");
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

        let c = Catalog::open(&path).unwrap();
        assert!(current_layout(&c.conn).unwrap());
        assert!(!dir.join("catalog.sqlite.rebuild").exists());
        let rows = |sql: &str| -> Vec<String> {
            let mut stmt = c.conn.prepare(sql).unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        // The user's own: the library, ratings, the queued turn, views and session, with the same file ids.
        assert_eq!(rows("SELECT path FROM roots"), [r"D:\p"]);
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
            assert_eq!(key, crate::browse::order_key(&folder, &name), "{name}");
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
    fn a_catalog_that_cant_be_copied_is_left_as_it_was() {
        let dir = test_dir("rebuild-fails");
        let path = dir.join("catalog.sqlite");
        // Files without the columns the current layout needs, and no way to make them.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE files(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO files VALUES (1, 'x');
             PRAGMA user_version = 1;",
        )
        .unwrap();
        drop(conn);
        let e = Catalog::open(&path).err().expect("refused");
        assert!(format!("{e:#}").contains("files.volume"), "{e:#}");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(conn.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(conn);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_catalog_open_elsewhere_isnt_copied() {
        let dir = test_dir("rebuild-open");
        let path = dir.join("catalog.sqlite");
        let other = Connection::open(&path).unwrap();
        other.execute_batch(BEFORE_ORDER_KEY).unwrap();
        let e = Catalog::open(&path).err().expect("refused");
        assert!(format!("{e:#}").contains("another program has it open"), "{e:#}");
        drop(other);
        assert!(Catalog::open(&path).is_ok(), "copied once nothing else has it open");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn catalogs_have_their_own_id() {
        let (a, b) = (Catalog::open(Path::new(":memory:")).unwrap(), Catalog::open(Path::new(":memory:")).unwrap());
        assert_eq!(a.catalog_id().unwrap().len(), 32);
        assert_ne!(a.catalog_id().unwrap(), b.catalog_id().unwrap());
    }

    #[test]
    fn roots_never_nest() {
        let c = Catalog::open(Path::new(":memory:")).unwrap();
        assert!(c.add_root(r"D:\a\b").unwrap());
        assert!(c.add_root(r"D:\a\bc").unwrap());
        assert!(!c.add_root(r"d:\A\b\c").unwrap(), "inside an existing root");
        assert!(c.add_root(r"D:\a").unwrap(), "replaces both");
        assert_eq!(c.roots().unwrap(), [r"D:\a"]);
        c.remove_root(r"D:\a").unwrap();
        assert!(c.roots().unwrap().is_empty());
    }

    #[test]
    fn cancelled_until_cleared() {
        let c = Catalog::open(Path::new(":memory:")).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        c.cancel_when(cancel.clone());
        let long =
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000) SELECT count(*) FROM n";
        let run = || c.conn.query_row(long, [], |r| r.get::<_, i64>(0)).map_err(anyhow::Error::from);
        assert_eq!(run().unwrap(), 100_000);
        // Set between statements (SQLite's own interrupt would be lost there): every statement fails.
        cancel.store(true, Ordering::Relaxed);
        assert!(interrupted(&run().unwrap_err()));
        assert!(interrupted(&run().unwrap_err()));
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(run().unwrap(), 100_000);
    }

    fn version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn migrations_apply_once_in_order() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn, &baseline(), &[]).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION);
        // A later build adds two migrations: both run, in order, once.
        let later = ["CREATE TABLE extra(x INTEGER);", "INSERT INTO extra VALUES (7);"];
        migrate(&conn, &baseline(), &later).unwrap();
        migrate(&conn, &baseline(), &later).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION + 2);
        assert_eq!(conn.query_row("SELECT count(*) FROM extra", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        // An older build refuses the newer catalog.
        assert!(migrate(&conn, &baseline(), &later[..1]).is_err());
        // A failed migration leaves the catalog at the last good version.
        let broken = ["CREATE TABLE extra(x INTEGER);", "INSERT INTO extra VALUES (7);", "NOT SQL"];
        assert!(migrate(&conn, &baseline(), &broken).is_err());
        assert_eq!(version(&conn), BASELINE_VERSION + 2);
    }

    /// A catalog with one file in it.
    fn one_file() -> (Catalog, i64) {
        let c = Catalog::open(Path::new(":memory:")).unwrap();
        c.conn
            .execute_batch(
                "INSERT INTO scans(id, root, started) VALUES (1, 'D:', 0);
                 INSERT INTO folders(id, path) VALUES (1, 'D:\\x');",
            )
            .unwrap();
        fixture::insert(&c.conn, &[fixture::File::new(1, 1, "a.jpg", 10, 100)]);
        (c, 1)
    }

    /// Whether the import would analyze file `id`, and its change number (`files.seq`).
    fn state(c: &Catalog, id: i64) -> (bool, i64) {
        c.conn.query_row("SELECT pending, seq FROM files WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
    }

    /// The scanner finding file 7 on volume 1 in D:\x like this.
    fn scanned(c: &mut Catalog, scan: i64, name: &str, size: u64, mtime: i64) {
        let tx = c.conn.transaction().unwrap();
        let folder = Catalog::folder(&tx, r"D:\x").unwrap();
        let kind = media::kind_of(name).unwrap();
        Catalog::upsert_file(&tx, scan, &folder, &ScannedFile { volume: 1, file_ref: 7, name, kind, size, mtime })
            .unwrap();
        tx.commit().unwrap();
    }

    /// The import storing what it made of `item` (a failure: it counts as made, and needs no Analysis).
    fn analyzed(c: &mut Catalog, item: &WorkItem) {
        let tx = c.conn.transaction().unwrap();
        Catalog::store_failure(&tx, item, "a stand-in for an analysis").unwrap();
        tx.commit().unwrap();
    }

    fn next_work(c: &Catalog) -> WorkItem {
        let mut work = c.pending_import(false).unwrap();
        assert_eq!(work.len(), 1);
        work.remove(0)
    }

    #[test]
    fn borders_are_looked_for_after_the_import_and_again_after_a_new_detector() {
        use crate::borders::{Edge, Kind as SideKind, Reason, Side};
        use crate::browse;
        use crate::query::{BorderFacet, Query, Sort};

        let mut c = Catalog::open(Path::new(":memory:")).unwrap();
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
            sides: vec![solid(Edge::Top), solid(Edge::Bottom), sky],
        };
        let analysis = Analysis {
            file_hash: [1; 32],
            format: "jpeg",
            orientation: 1,
            width: 400,
            height: 300,
            thumbhash: Vec::new(),
            file_rating: None,
            frames: 1,
            taken: None,
            taken_utc: false,
            pixels: None,
            duration_ms: None,
            poster_ms: None,
        };
        let tx = c.conn.transaction().unwrap();
        Catalog::store_analysis(&tx, &item, &analysis).unwrap();
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
        assert_eq!(due.iter().map(|d| d.id).collect::<Vec<_>>(), [item.id]);
        let tx = c.conn.transaction().unwrap();
        Catalog::store_borders(&tx, &due[0], Some(&found)).unwrap();
        tx.commit().unwrap();
        assert_eq!(cut(&c), [Some(0), Some(41), Some(0), Some(41)]);
        assert_eq!(browse::border_sides(&c.conn, item.id).unwrap(), found.sides);
        assert!(c.pending_borders().unwrap().is_empty());

        // The grid's entry, the filters and the sort.
        let list = |c: &Catalog, q: Query| -> Vec<browse::Entry> {
            browse::list_grid(&c.conn, "", true, &q, Sort::default()).unwrap()
        };
        let entry = list(&c, Query::default()).remove(0);
        assert_eq!(entry.borders, browse::Borders::Found);
        let [x, y, w, h] = entry.content().unwrap();
        assert!(x == 0.0 && w == 1.0 && (y - 41.0 / 300.0).abs() < 1e-9 && (h - 218.0 / 300.0).abs() < 1e-9);
        let only = |b: Vec<BorderFacet>| Query { borders: b, ..Query::default() };
        assert_eq!(list(&c, only(vec![BorderFacet::Black])).len(), 1);
        assert!(list(&c, only(vec![BorderFacet::Blur, BorderFacet::None])).is_empty());
        let counts = browse::facets(&c.conn, "", true, &Query::default()).unwrap().borders;
        assert_eq!(counts.iter().find(|(b, _)| *b == BorderFacet::Black).unwrap().1, 1);
        assert_eq!(counts.iter().find(|(b, _)| *b == BorderFacet::Blur).unwrap().1, 0);

        // Marked as not a border: no border to the filters, the mark to the review.
        Catalog::set_border_mark(&c.conn, item.id, true).unwrap();
        assert_eq!(list(&c, Query::default())[0].borders, browse::Borders::Marked);
        assert!(list(&c, only(vec![BorderFacet::Black])).is_empty());
        assert_eq!(list(&c, only(vec![BorderFacet::None])).len(), 1);
        assert_eq!(list(&c, Query { border_marked: Some(true), ..Query::default() }).len(), 1);

        // A new detector: due again, without a new import; the mark stays.
        c.conn.execute("UPDATE meta SET value = 0 WHERE key = 'borders_version'", []).unwrap();
        set_borders_version(&c.conn).unwrap();
        assert!(c.pending_import(false).unwrap().is_empty());
        let due = c.pending_borders().unwrap();
        assert_eq!(due.len(), 1);
        let tx = c.conn.transaction().unwrap();
        Catalog::store_borders(&tx, &due[0], Some(&Detection::none(400, 300))).unwrap();
        tx.commit().unwrap();
        assert_eq!(cut(&c), [Some(0); 4]);
        assert!(browse::border_sides(&c.conn, item.id).unwrap().is_empty());
        assert!(c.pending_borders().unwrap().is_empty());
        assert_eq!(list(&c, Query::default())[0].borders, browse::Borders::None);
        let marks: i64 = c.conn.query_row("SELECT count(*) FROM border_marks", [], |r| r.get(0)).unwrap();
        assert_eq!(marks, 1);

        // Analyzed again (the file changed): the old borders go, the new ones are due.
        let tx = c.conn.transaction().unwrap();
        Catalog::store_borders(&tx, &due[0], Some(&found)).unwrap();
        Catalog::store_analysis(&tx, &item, &analysis).unwrap();
        tx.commit().unwrap();
        assert_eq!(cut(&c), [None; 4]);
        assert!(browse::border_sides(&c.conn, item.id).unwrap().is_empty());
        assert_eq!(c.pending_borders().unwrap().len(), 1);
    }

    #[test]
    fn border_depth_sorts_by_the_largest_share_of_a_side_cut() {
        use crate::browse;
        use crate::query::{Query, Sort, SortKey};

        let mut c = Catalog::open(Path::new(":memory:")).unwrap();
        c.add_root(r"D:\x").unwrap();
        let scan = c.begin_scan(r"D:\x").unwrap();
        // 400 × 300 stored. b: none; c: 30 of 300 at the top (10%); d: 60 of 400 at the left (15%), but turned a
        // quarter (orientation 6), so 60 of the 400 upright rows; e: 90 at the top (30%), marked not a border.
        let files = [("b.jpg", 1, None), ("c.jpg", 1, Some([0, 30, 0, 0])), ("d.jpg", 6, Some([60, 0, 0, 0]))];
        let files = files.into_iter().chain([("e.jpg", 1, Some([0, 90, 0, 0]))]);
        for (n, (name, orientation, cut)) in files.enumerate() {
            let tx = c.conn.transaction().unwrap();
            let folder = Catalog::folder(&tx, r"D:\x").unwrap();
            let kind = media::kind_of(name).unwrap();
            let file = ScannedFile { volume: 1, file_ref: 10 + n as u64, name, kind, size: 10, mtime: 100 };
            Catalog::upsert_file(&tx, scan, &folder, &file).unwrap();
            tx.commit().unwrap();
            let item = c.pending_import(false).unwrap().remove(0);
            let [l, t, r, b] = cut.unwrap_or([0; 4]);
            let mut borders = Detection::none(400, 300);
            borders.content = [l, t, 400 - r, 300 - b];
            let (width, height) = if orientation >= 5 { (300, 400) } else { (400, 300) };
            let analysis = Analysis {
                file_hash: [n as u8; 32],
                format: "jpeg",
                orientation,
                width,
                height,
                thumbhash: Vec::new(),
                file_rating: None,
                frames: 1,
                taken: None,
                taken_utc: false,
                pixels: None,
                duration_ms: None,
                poster_ms: None,
            };
            let tx = c.conn.transaction().unwrap();
            Catalog::store_analysis(&tx, &item, &analysis).unwrap();
            Catalog::store_borders(&tx, &item, Some(&borders)).unwrap();
            tx.commit().unwrap();
            if name == "e.jpg" {
                Catalog::set_border_mark(&c.conn, item.id, true).unwrap();
            }
        }
        let names = |descending: bool| -> Vec<String> {
            let sort = Sort { key: SortKey::Borders, descending };
            let entries = browse::list_grid(&c.conn, "", true, &Query::default(), sort).unwrap();
            let path = |id| browse::details(&c.conn, id).unwrap().unwrap().path;
            entries.iter().map(|e| path(e.id).rsplit('\\').next().unwrap().to_string()).collect()
        };
        // Without a border (none found, or marked) last, by name, either way.
        assert_eq!(names(false), ["c.jpg", "d.jpg", "b.jpg", "e.jpg"]);
        assert_eq!(names(true), ["d.jpg", "c.jpg", "b.jpg", "e.jpg"]);
    }

    #[test]
    fn pending_follows_what_the_analysis_was_made_from() {
        let mut c = Catalog::open(Path::new(":memory:")).unwrap();
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
        let old = c.pending_import(true).unwrap().remove(0);
        scanned(&mut c, scan, "a.jpg", 11, 101);
        analyzed(&mut c, &old);
        assert!(state(&c, id).0);
        let current = next_work(&c);
        analyzed(&mut c, &current);
        assert!(!state(&c, id).0);
        // Turned in the catalog: analyzed again with the turn, and the thumbnails' source moved on.
        Catalog::add_catalog_rotation(&c.conn, id, 1).unwrap();
        let (pending, after) = state(&c, id);
        assert!(pending && after > seq);
        let turned = next_work(&c);
        assert_eq!(turned.rotation, 1);
        analyzed(&mut c, &turned);
        // Rewritten by the app: a metadata write carries the analysis along; a pixel write leaves it stale.
        Catalog::file_rewritten(&c.conn, id, (11, 101), (12, 102), Some([0; 32])).unwrap();
        assert!(!state(&c, id).0);
        Catalog::file_rewritten(&c.conn, id, (12, 102), (13, 103), None).unwrap();
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
        c.mark_missing(r"D:\x", next).unwrap();
        let gone = state(&c, id).1;
        scanned(&mut c, next, "a.mp4", 13, 103);
        assert_eq!(c.pending_import(false).unwrap().len(), 0);
        assert!(seq < gone && gone < state(&c, id).1);
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
        set_import_version(&c.conn).unwrap();
        let (pending, after) = state(&c, item.id);
        assert!(pending && after > seq);
        // Done once.
        set_import_version(&c.conn).unwrap();
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
        crate::browse::set_rating(&c.conn, id, n, false).unwrap();
    }

    #[test]
    fn three_way_rating_sync() {
        let (c, id) = one_file();
        let sync = |in_file| {
            let tx = c.conn.unchecked_transaction().unwrap();
            Catalog::sync_file_rating(&tx, id, in_file).unwrap();
            tx.commit().unwrap();
        };
        // First import of a file rated elsewhere (ImageRater, Explorer): adopted.
        sync(Some(4));
        assert_eq!(rating(&c, id), Some(4));
        // Rated in the app, not written back (or the format can't hold it): the file still says 4, as known, so
        // re-imports keep the catalog's 2.
        set_rating(&c, id, 2);
        sync(Some(4));
        assert_eq!(rating(&c, id), Some(2));
        // Another program changes the file's rating: that is newer, and adopted.
        sync(Some(5));
        assert_eq!(rating(&c, id), Some(5));
        // Another program clears it (writes 0, or removes it): the catalog follows.
        sync(Some(0));
        assert_eq!(rating(&c, id), None);
        sync(None);
        assert_eq!(rating(&c, id), None);
        // A write of the app's own still queued wins over the file.
        set_rating(&c, id, 3);
        c.conn
            .execute("INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rating', 3, 0)", [id])
            .unwrap();
        sync(Some(1));
        assert_eq!(rating(&c, id), Some(3));
    }
}
