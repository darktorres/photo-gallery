//! The state the tests of the crates around the catalog set up or check, by name. Test-only (this module is behind the
//! `fixture` feature): each statement is a function here, so the SQL is in one place and a test says what it sets up
//! or checks. Production code reads through the catalog's functions.

use crate::Catalog;
use crate::writes::MAX_ATTEMPTS;
use pg_model::ids::FileId;
use rusqlite::{OptionalExtension, params};

/// A scan row. Ids are the test's own: [`super::insert`] needs scan 1, and the folders its files name.
pub fn add_scan(c: &Catalog, id: i64, root: &str) {
    c.conn.execute("INSERT INTO scans(id, root, started) VALUES (?1, ?2, 0)", params![id, root]).unwrap();
}

/// A folder row (see [`add_scan`]).
pub fn add_folder(c: &Catalog, id: i64, path: &str) {
    c.conn.execute("INSERT INTO folders(id, path) VALUES (?1, ?2)", params![id, path]).unwrap();
}

/// An analysis row at version 1 for every file, with no pixel data: a stub for the tests that only need the row
/// (the thumbnail pass reads its size, mtime, rotation and kind).
pub fn stub_analysis_of_all(c: &Catalog) {
    c.conn
        .execute_batch(
            "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed)
             SELECT id, 1, size, mtime, rotation, kind, 0 FROM files;",
        )
        .unwrap();
}

/// The catalog's own change number, as a restored copy of an older catalog would have it.
pub fn set_meta_seq(c: &Catalog, value: i64) {
    c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'seq'", [value]).unwrap();
}

/// A file changed on disk: its mtime is `mtime`, and its analysis follows, as a metadata write's does.
pub fn change_mtime(c: &Catalog, id: FileId, mtime: i64) {
    c.conn.execute("UPDATE files SET mtime = ?2 WHERE id = ?1", params![id.get(), mtime]).unwrap();
    c.conn.execute("UPDATE analysis SET mtime = ?2 WHERE file_id = ?1", params![id.get(), mtime]).unwrap();
}

/// The files the catalog lists and has not found missing, by name.
pub fn present_files(c: &Catalog) -> Vec<(FileId, String)> {
    let mut stmt = c.conn.prepare("SELECT id, name FROM files WHERE missing = 0 ORDER BY name").unwrap();
    stmt.query_map([], |r| Ok((FileId::new(r.get(0)?), r.get(1)?))).unwrap().map(Result::unwrap).collect()
}

/// The id of the file called `name`.
pub fn file_id(c: &Catalog, name: &str) -> FileId {
    FileId::new(c.conn.query_row("SELECT id FROM files WHERE name = ?1", [name], |r| r.get(0)).unwrap())
}

/// The id and the full path the database has for the file called `name`: its folder and its name, joined by a
/// backslash.
pub fn file_and_path(c: &Catalog, name: &str) -> (FileId, String) {
    let (id, path): (i64, String) = c
        .conn
        .query_row(
            "SELECT f.id, fo.path || '\\' || f.name FROM files f JOIN folders fo ON fo.id = f.folder_id
             WHERE f.name = ?1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    (FileId::new(id), path)
}

/// The turn the file holds in the catalog (`files.rotation`), by name.
pub fn file_rotation(c: &Catalog, name: &str) -> u8 {
    c.conn.query_row("SELECT rotation FROM files WHERE name = ?1", [name], |r| r.get(0)).unwrap()
}

/// Marks the file missing, as a scan that doesn't find it does.
pub fn mark_missing(c: &Catalog, id: FileId) {
    c.conn.execute("UPDATE files SET missing = 1 WHERE id = ?1", [id.get()]).unwrap();
}

/// The rating the catalog holds for the file (`ratings`), unrated as `None`.
pub fn rating_of(c: &Catalog, id: FileId) -> Option<u8> {
    c.conn.query_row("SELECT rating FROM ratings WHERE file_id = ?1", [id.get()], |r| r.get(0)).optional().unwrap()
}

/// The `borders_due` flag of the analysis of the file called `name`: 1 while the borders pass still has to read it.
pub fn borders_due(c: &Catalog, name: &str) -> i64 {
    c.conn
        .query_row(
            "SELECT a.borders_due FROM analysis a JOIN files f ON f.id = a.file_id WHERE f.name = ?1",
            [name],
            |r| r.get(0),
        )
        .unwrap()
}

/// The width and height the analysis holds for each present file, by name.
pub fn analysis_sizes(c: &Catalog) -> Vec<(u32, u32)> {
    let mut stmt = c
        .conn
        .prepare("SELECT a.width, a.height FROM analysis a JOIN files f ON f.id = a.file_id ORDER BY f.name")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
}

/// The turn the analysis holds for the file (its pending turn, when a turn is queued and not yet analysed).
pub fn analysis_rotation(c: &Catalog, id: FileId) -> i64 {
    c.conn.query_row("SELECT rotation FROM analysis WHERE file_id = ?1", [id.get()], |r| r.get(0)).unwrap()
}

/// Sets the turn the analysis holds for the file.
pub fn set_analysis_rotation(c: &Catalog, id: FileId, turns: u8) {
    c.conn.execute("UPDATE analysis SET rotation = ?2 WHERE file_id = ?1", params![id.get(), turns]).unwrap();
}

/// One row per present file that has a video analysis, by name: the name, the kind, the width, the height, the
/// duration and the poster time in milliseconds, and whether the pixel hash is missing (1) or present (0).
pub type MediaRow = (String, String, i64, i64, i64, i64, Option<i64>);

/// The media rows of the present files (see [`MediaRow`]).
pub fn media_rows(c: &Catalog) -> Vec<MediaRow> {
    let mut stmt = c
        .conn
        .prepare(
            "SELECT f.name, f.kind, a.width, a.height, a.duration_ms, a.poster_ms, a.pixel_hash IS NULL
             FROM files f JOIN analysis a ON a.file_id = f.id ORDER BY f.name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The capture time the analysis holds for the file called `name`, if it has one.
pub fn taken_of(c: &Catalog, name: &str) -> Option<i64> {
    c.conn
        .query_row("SELECT taken FROM analysis a JOIN files f ON f.id = a.file_id WHERE f.name = ?1", [name], |r| {
            r.get(0)
        })
        .unwrap()
}

/// The Unix time at which the local clock shows `wall` seconds (as SQLite reads the local time zone).
pub fn local_epoch(c: &Catalog, wall: i64) -> i64 {
    c.conn.query_row("SELECT unixepoch(?1, 'unixepoch', 'localtime')", [wall], |r| r.get(0)).unwrap()
}

/// Writes still to be tried: their attempts are below the queue's limit. (The queue's own count, `writes::left_to_write`,
/// leaves out a failed write with no temp file, which is not tried again until its time comes.)
pub fn writes_still_tried(c: &Catalog) -> i64 {
    c.conn.query_row("SELECT count(*) FROM file_writes WHERE attempts < ?1", [MAX_ATTEMPTS], |r| r.get(0)).unwrap()
}

/// The write rows the queue holds for the file, whatever their kind.
pub fn writes_queued(c: &Catalog, id: FileId) -> i64 {
    c.conn.query_row("SELECT count(*) FROM file_writes WHERE file_id = ?1", [id.get()], |r| r.get(0)).unwrap()
}

/// The state of the file's write row in the queue: the tries so far, the temp file a finishing copy left, and the
/// last error.
pub struct WriteRow {
    pub attempts: i64,
    pub temp: Option<String>,
    pub error: Option<String>,
}

/// The file's write row (it must be in the queue).
pub fn write_row(c: &Catalog, id: FileId) -> WriteRow {
    c.conn
        .query_row("SELECT attempts, temp, error FROM file_writes WHERE file_id = ?1", [id.get()], |r| {
            Ok(WriteRow { attempts: r.get(0)?, temp: r.get(1)?, error: r.get(2)? })
        })
        .unwrap()
}

/// Queues a write of `kind` (`rating` or `rotation`) with `value` for the file, as a change made by hand.
pub fn queue_write(c: &Catalog, id: FileId, kind: &str, value: i64) {
    c.conn
        .execute(
            "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, ?2, ?3, 0)",
            params![id.get(), kind, value],
        )
        .unwrap();
}

/// Queues a write whose copy was finished when the app stopped: its temp file `temp` is beside the original.
pub fn queue_finished_copy(c: &Catalog, id: FileId, kind: &str, value: i64, temp: &str) {
    c.conn
        .execute(
            "INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value) VALUES (?1, ?2, ?3, 0, ?4, ?3)",
            params![id.get(), kind, value, temp],
        )
        .unwrap();
}
