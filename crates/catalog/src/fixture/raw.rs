//! The raw state the catalog's own tests set up or check, by name. Test-only.
//!
//! A test sets up or checks common state through the functions here, so each shared statement is in one place and the
//! test says what it sets up or checks. Two kinds of test use the connection directly: a test whose subject is a
//! statement (a writer, a query, a migration) runs it on the catalog's own connection, and the layout tests (the rebuild
//! and the migrations) open the file on disk themselves with [`on_disk`], since what they check is the file.

// Its helpers unwrap: a failed statement here is the test's failure. They are test code under `cfg(test)` (exempt by
// clippy.toml), but other crates' tests build this module through the `fixture` feature, where the lint applies.
#![cfg_attr(not(test), expect(clippy::unwrap_used, reason = "a fixture's statement that fails is the test's failure"))]

use std::path::Path;

use crate::core::Catalog;
use crate::lifecycle::{set_borders_version, set_import_version, set_thumbs_version};
use crate::queue;
use pg_model::analysis::PLACEHOLDER_VERSION;
use pg_model::ids::FileId;
use pg_model::rating::Rating;
use pg_model::rules::PathRules;
use pg_model::turns::Turns;
use pg_model::versions::{BORDERS_VERSION, IMPORT_VERSION, THUMBS_VERSION};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Params, Row, params};

/// A connection of the test's own to the catalog file at `path`.
pub(crate) fn on_disk(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    // The catalog's functions, under the rules the file records (when it records them): a write of a file row, and a
    // `VACUUM`, evaluate the name index's.
    let rules: Option<String> =
        conn.query_row("SELECT value FROM meta WHERE key = 'path_rules'", [], |r| r.get(0)).ok();
    if let Some(rules) = [PathRules::Windows, PathRules::Posix]
        .into_iter()
        .find(|&r| rules.as_deref() == Some(crate::types::rules_name(r)))
    {
        crate::schema::register(&conn, rules).unwrap();
    }
    conn
}

/// The path rules a catalog file was made under, as its `meta.path_rules` row says, read through a read-only
/// connection of the test's own (the way the real catalogs are read).
pub(crate) fn recorded_rules(path: &Path) -> PathRules {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let name: String = conn.query_row("SELECT value FROM meta WHERE key = 'path_rules'", [], |r| r.get(0)).unwrap();
    [PathRules::Windows, PathRules::Posix]
        .into_iter()
        .find(|&rules| crate::types::rules_name(rules) == name)
        .unwrap_or_else(|| panic!("an unknown path rules name: {name:?}"))
}

/// Run `sql`, which the catalog itself runs, and map its rows: to check what the catalog's statements return.
pub(crate) fn rows_of<T, P: Params>(
    c: &Catalog,
    sql: &str,
    params: P,
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Vec<T> {
    let mut stmt = c.conn.prepare(sql).unwrap();
    stmt.query_map(params, map).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

/// The query plan of `sql` (the catalog's statement, with `params`): one line per step.
pub(crate) fn query_plan<P: Params>(c: &Catalog, sql: &str, params: P) -> String {
    rows_of(c, &format!("EXPLAIN QUERY PLAN {sql}"), params, |r| r.get::<_, String>(3)).join("\n")
}

/// The catalog's own statement with a long recursion, for the cancel test: it runs as long as it is not interrupted.
pub(crate) fn slow_count(c: &Catalog) -> rusqlite::Result<i64> {
    let long =
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000) SELECT count(*) FROM n";
    c.conn.query_row(long, [], |r| r.get(0))
}

pub(crate) fn set_missing(c: &Catalog, id: FileId) {
    c.conn.execute("UPDATE files SET missing = 1 WHERE id = ?1", [id.get()]).unwrap();
}

/// A file's volume by its serial as the catalog stores it (`None` for a displaced row), interned as a writer interns it:
/// the fixture inserts every file on volume 1, so a test that spreads files over volumes moves them here.
pub(crate) fn set_volume(c: &Catalog, id: FileId, volume: Option<i64>) {
    if let Some(serial) = volume {
        c.conn.execute("INSERT OR IGNORE INTO volumes(serial) VALUES (?1)", [serial]).unwrap();
    }
    c.conn
        .execute(
            "UPDATE files SET volume = (SELECT id FROM volumes WHERE serial = ?2) WHERE id = ?1",
            params![id.get(), volume],
        )
        .unwrap();
}

/// The serial of a file's volume, as the catalog stores it (`None` for a displaced row).
pub(crate) fn volume_of(c: &Catalog, id: FileId) -> Option<i64> {
    c.conn
        .query_row(
            "SELECT v.serial FROM files f LEFT JOIN volumes v ON v.id = f.volume WHERE f.id = ?1",
            [id.get()],
            |r| r.get(0),
        )
        .unwrap()
}

/// A file's size changed in the catalog alone (the triggers move it in the thumbnail source).
pub(crate) fn set_size(c: &Catalog, id: FileId, size: i64) {
    c.conn.execute("UPDATE files SET size = ?2 WHERE id = ?1", params![id.get(), size]).unwrap();
}

/// The turns the catalog keeps for a file (its `files.turns`).
pub(crate) fn set_rotation(c: &Catalog, id: FileId, turns: Turns) {
    c.conn.execute("UPDATE files SET turns = ?2 WHERE id = ?1", params![id.get(), turns.get()]).unwrap();
}

pub(crate) fn file_size(c: &Catalog, id: FileId) -> i64 {
    c.conn.query_row("SELECT size FROM files WHERE id = ?1", [id.get()], |r| r.get(0)).unwrap()
}

/// The file's kept turns and the width of its analysis.
pub(crate) fn rotation_and_width(c: &Catalog, id: FileId) -> (i64, i64) {
    c.conn
        .query_row(
            "SELECT f.turns, a.width FROM files f JOIN analysis a ON a.file_id = f.id WHERE f.id = ?1",
            [id.get()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

/// The only file's row as the scan leaves it: name, size, mtime, missing, and its merged folder's key.
pub(crate) fn file_row(c: &Catalog) -> (String, i64, i64, i64, Vec<u8>) {
    c.conn
        .query_row(
            "SELECT CAST(f.name AS TEXT), f.size, f.mtime, f.missing, m.key FROM files f JOIN mfolders m ON m.id = f.mid",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap()
}

pub(crate) fn missing_count(c: &Catalog) -> i64 {
    c.conn.query_row("SELECT count(*) FROM files WHERE missing = 1", [], |r| r.get(0)).unwrap()
}

/// The file's change number, which the thumbnail pass reads (`files.seq`).
pub(crate) fn file_seq(c: &Catalog, id: FileId) -> i64 {
    c.conn.query_row("SELECT seq FROM files WHERE id = ?1", [id.get()], |r| r.get(0)).unwrap()
}

pub(crate) fn thumb_source_seq(c: &Catalog, id: FileId) -> i64 {
    c.conn.query_row("SELECT seq FROM thumb_sources WHERE id = ?1", [id.get()], |r| r.get(0)).unwrap()
}

/// The files the thumbnail pass sees, with their folders.
pub(crate) fn thumb_sources(c: &Catalog) -> Vec<(i64, String)> {
    rows_of(c, "SELECT id, CAST(folder AS TEXT) FROM thumb_sources ORDER BY id", [], |r| Ok((r.get(0)?, r.get(1)?)))
}

pub(crate) fn thumb_clock(c: &Catalog) -> i64 {
    c.conn.query_row("SELECT seq FROM thumb_clock", [], |r| r.get(0)).unwrap()
}

pub(crate) fn meta_seq(c: &Catalog) -> i64 {
    c.conn.query_row("SELECT value FROM meta WHERE key = 'seq'", [], |r| r.get(0)).unwrap()
}

/// The borders the analysis holds: the content's left, top, right and bottom cut, for the only analysis.
pub(crate) fn cut_sides(c: &Catalog) -> [Option<i64>; 4] {
    c.conn
        .query_row("SELECT cut_left, cut_top, cut_right, cut_bottom FROM analysis", [], |r| {
            Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?])
        })
        .unwrap()
}

pub(crate) fn border_marks(c: &Catalog) -> i64 {
    c.conn.query_row("SELECT count(*) FROM border_marks", [], |r| r.get(0)).unwrap()
}

/// A placeholder for a file, made at its identity and the current version, as the Placeholders pass stores it.
pub(crate) fn set_placeholder(c: &Catalog, id: FileId, bytes: &[u8]) {
    c.conn
        .execute(
            "INSERT OR REPLACE INTO placeholders(file_id, size, mtime, rotation, version, hash)
             SELECT id, size, mtime, turns, ?2, ?3 FROM files WHERE id = ?1",
            params![id.get(), PLACEHOLDER_VERSION, bytes],
        )
        .unwrap();
}

pub(crate) fn set_file_hash(c: &Catalog, ids: &[i64], bytes: &[u8]) {
    for id in ids {
        c.conn.execute("UPDATE analysis SET file_hash = ?2 WHERE file_id = ?1", params![id, bytes]).unwrap();
    }
}

pub(crate) fn set_pixel_hash(c: &Catalog, ids: &[i64], bytes: &[u8]) {
    for id in ids {
        c.conn.execute("UPDATE analysis SET pixel_hash = ?2 WHERE file_id = ?1", params![id, bytes]).unwrap();
    }
}

pub(crate) fn set_taken(c: &Catalog, id: i64, seconds: i64) {
    c.conn.execute("UPDATE analysis SET taken = ?2 WHERE file_id = ?1", params![id, seconds]).unwrap();
}

/// A pair of files close enough to list as duplicates (the dedup's tier 1). What their analyses owe (`pairs_owed`) stays
/// as it is: the test seeds the pair as the pairing pass would have stored it.
pub(crate) fn add_pdq_pair(c: &Catalog, a: i64, b: i64, distance: i64) {
    c.conn.execute("INSERT INTO pdq_pairs(a, b, distance) VALUES (?1, ?2, ?3)", params![a, b, distance]).unwrap();
}

/// An analysis row as a test sets it up: the columns the browse tests read, at the import version
/// (`pg_model::versions::IMPORT_VERSION`). The kind and the format are their names (`"video"`, `"jpeg"`), stored as their
/// codes.
pub(crate) struct AnalysisRow {
    pub file: i64,
    pub size: i64,
    pub mtime: i64,
    pub kind: &'static str,
    pub format: Option<&'static str>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub frames: Option<i64>,
    pub duration_ms: Option<i64>,
    pub error: Option<&'static str>,
}

impl AnalysisRow {
    /// An image of `size` and `mtime` with no format or dimensions: set the fields a test needs.
    pub(crate) fn image(file: i64, size: i64, mtime: i64) -> AnalysisRow {
        AnalysisRow {
            file,
            size,
            mtime,
            kind: "image",
            format: None,
            width: None,
            height: None,
            frames: None,
            duration_ms: None,
            error: None,
        }
    }
}

/// The stored code of a kind's name (`"image"`, `"video"`), as a test names it.
pub(crate) fn kind_code(name: &str) -> i64 {
    match name {
        "image" => pg_model::media::Kind::Image.code(),
        "video" => pg_model::media::Kind::Video.code(),
        other => panic!("no kind is named {other:?}"),
    }
}

pub(crate) fn insert_analysis(c: &Catalog, row: &AnalysisRow) {
    let format = row
        .format
        .map(|f| pg_model::format::Format::from_key(f).unwrap_or_else(|| panic!("no format is named {f:?}")).code());
    c.conn
        .execute(
            "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, format, width, height, frames,
                                  duration_ms, error)
             VALUES (?1, ?11, ?2, ?3, 0, ?4, 0, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                row.file,
                row.size,
                row.mtime,
                kind_code(row.kind),
                format,
                row.width,
                row.height,
                row.frames,
                row.duration_ms,
                row.error,
                IMPORT_VERSION
            ],
        )
        .unwrap();
    crate::writers::touch_analysis(&c.conn, row.file).unwrap();
}

/// The import version as an older build left it: the stored version and the analyses made by it.
pub(crate) fn older_import_version(c: &Catalog) {
    let older = IMPORT_VERSION - 1;
    c.conn
        .execute_batch(&format!(
            "UPDATE meta SET value = {older} WHERE key = 'import_version';
             UPDATE analysis SET version = {older};"
        ))
        .unwrap();
}

/// This build starts on the catalog: its import version is taken, and the files it changes are due again.
pub(crate) fn start_import_version(c: &Catalog) {
    set_import_version(&c.conn, IMPORT_VERSION).unwrap();
}

/// The stored borders version, older than this build's detector. 0 is the sentinel for an older build's mark: the
/// registry starts at 1, so no build writes 0.
pub(crate) fn older_borders_version(c: &Catalog) {
    c.conn.execute("UPDATE meta SET value = 0 WHERE key = 'borders_version'", []).unwrap();
}

/// This build starts on the catalog: its detector's version is taken.
pub(crate) fn start_borders_version(c: &Catalog) {
    set_borders_version(&c.conn, BORDERS_VERSION).unwrap();
}

/// A later build's detector has taken the catalog's borders mark: this build's detections are refused.
pub(crate) fn newer_borders_version(c: &Catalog) {
    c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'borders_version'", [BORDERS_VERSION + 1]).unwrap();
}

/// The thumbnails' version as an older build left it: the stored mark (`meta.thumbs_version`), older than this build's
/// thumbnailer.
pub(crate) fn older_thumbs_version(c: &Catalog) {
    c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'thumbs_version'", [THUMBS_VERSION - 1]).unwrap();
}

/// This build starts on the catalog: its thumbnailer's version is taken, and a failure of an older one is listed again.
pub(crate) fn start_thumbs_version(c: &Catalog) {
    set_thumbs_version(&c.conn, THUMBS_VERSION, crate::ledger::failure::relist_thumbs_panics).unwrap();
}

/// The rating the catalog holds for a file (`ratings`), unrated as `None`.
pub(crate) fn rating(c: &Catalog, id: i64) -> Option<u8> {
    super::state::rating_of(c, FileId::new(id)).unwrap()
}

/// What the file holds, as the import read it (`file_ratings`): the writer's own record of it.
pub(crate) fn holds_rating(c: &Catalog, id: FileId, value: Option<Rating>) {
    queue::set_file_rating(&c.conn, id, value).unwrap();
}

/// The value queued for a file's write of `kind` (`rating` or `rotation`), if one is queued.
pub(crate) fn queued(c: &Catalog, id: i64, kind: &str) -> Option<i64> {
    c.conn
        .query_row("SELECT value FROM file_writes WHERE file_id = ?1 AND kind = ?2", params![id, kind], |r| r.get(0))
        .optional()
        .unwrap()
}
