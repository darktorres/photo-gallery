//! The state the tests of the crates around the catalog set up or check, by name. Test-only (this module is behind the
//! `fixture` feature): each statement is a function here, so the SQL is in one place and a test says what it sets up
//! or checks. Production code reads through the catalog's functions. Each returns the statement's error, so a test
//! fails with it.

use crate::core::Catalog;
use crate::queue::MAX_ATTEMPTS;
use anyhow::Result;
use pg_model::ids::FileId;
use rusqlite::{OptionalExtension, params};

/// A scan row. Ids are the test's own: [`super::insert`] needs scan 1, and the folders its files name.
pub fn add_scan(c: &Catalog, id: i64, root: &str) -> Result<()> {
    c.conn.execute("INSERT INTO scans(id, root, started) VALUES (?1, ?2, 0)", params![id, root])?;
    Ok(())
}

/// A folder row (see [`add_scan`]).
pub fn add_folder(c: &Catalog, id: i64, path: &str) -> Result<()> {
    let (bytes, key) = super::folder_row(c, path)?;
    c.conn.execute("INSERT INTO folders(id, path, key) VALUES (?1, ?2, ?3)", params![id, bytes, key])?;
    Ok(())
}

/// An analysis row at version 1 for every file, with no pixel data: a stub for the tests that only need the row
/// (the thumbnail pass reads its size, mtime, rotation and kind).
pub fn stub_analysis_of_all(c: &Catalog) -> Result<()> {
    c.conn.execute_batch(
        "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed)
             SELECT id, 1, size, mtime, rotation, kind, 0 FROM files;",
    )?;
    let ids: Vec<i64> =
        c.conn.prepare("SELECT id FROM files")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for id in ids {
        crate::writers::touch_analysis(&c.conn, id)?;
    }
    Ok(())
}

/// The catalog's own change number, as a restored copy of an older catalog would have it.
pub fn set_meta_seq(c: &Catalog, value: i64) -> Result<()> {
    c.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'seq'", [value])?;
    Ok(())
}

/// Time passes for every scan lease: each heartbeat is `secs` older (a walk that has not renewed since, as a listing that
/// outlasts the lease does not). Past `SCAN_LEASE_SECS`, the lease is one a crashed walk left, and the next walk takes it.
pub fn age_leases(c: &Catalog, secs: i64) -> Result<()> {
    c.conn.execute("UPDATE scan_leases SET heartbeat = heartbeat - ?1", [secs])?;
    Ok(())
}

/// A file changed on disk: its mtime is `mtime`, and its analysis follows, as a metadata write's does.
pub fn change_mtime(c: &Catalog, id: FileId, mtime: i64) -> Result<()> {
    c.conn.execute("UPDATE files SET mtime = ?2 WHERE id = ?1", params![id.get(), mtime])?;
    c.conn.execute("UPDATE analysis SET mtime = ?2 WHERE file_id = ?1", params![id.get(), mtime])?;
    Ok(())
}

/// The files the catalog lists and has not found missing, by name.
pub fn present_files(c: &Catalog) -> Result<Vec<(FileId, String)>> {
    let mut stmt = c.conn.prepare("SELECT id, name FROM files WHERE missing = 0 ORDER BY name")?;
    // A name with no UTF-8 form is shown lossily (its display form), so the listing never fails on one.
    let rows = stmt.query_map([], |r| {
        let name: Vec<u8> = r.get(1)?;
        Ok((FileId::new(r.get(0)?), pg_model::name::Name::from_raw(&name).display().into_owned()))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The id of the file called `name`.
pub fn file_id(c: &Catalog, name: &str) -> Result<FileId> {
    Ok(FileId::new(c.conn.query_row("SELECT id FROM files WHERE name = ?1", [name.as_bytes()], |r| r.get(0))?))
}

/// The id and the full path the database has for the file called `name` (its folder's path, the separator, the
/// name, as the catalog's rules display it).
pub fn file_and_path(c: &Catalog, name: &str) -> Result<(FileId, pg_model::rules::Path)> {
    let id: i64 = c.conn.query_row("SELECT id FROM files WHERE name = ?1", [name.as_bytes()], |r| r.get(0))?;
    let path = c.file_path(FileId::new(id))?.ok_or_else(|| anyhow::anyhow!("file {id} is not in the catalog"))?;
    Ok((FileId::new(id), path))
}

/// The turn the file holds in the catalog (`files.rotation`), by name.
pub fn file_rotation(c: &Catalog, name: &str) -> Result<u8> {
    Ok(c.conn.query_row("SELECT rotation FROM files WHERE name = ?1", [name.as_bytes()], |r| r.get(0))?)
}

/// Marks the file missing, as a scan that doesn't find it does.
pub fn mark_missing(c: &Catalog, id: FileId) -> Result<()> {
    c.conn.execute("UPDATE files SET missing = 1 WHERE id = ?1", [id.get()])?;
    Ok(())
}

/// The rating the catalog holds for the file (`ratings`), unrated as `None`.
pub fn rating_of(c: &Catalog, id: FileId) -> Result<Option<u8>> {
    Ok(c.conn.query_row("SELECT rating FROM ratings WHERE file_id = ?1", [id.get()], |r| r.get(0)).optional()?)
}

/// The rating the file itself holds (`file_ratings`, as it was written into the file or read from it), unrated as `None`.
pub fn file_rating_of(c: &Catalog, id: FileId) -> Result<Option<u8>> {
    Ok(c.conn.query_row("SELECT value FROM file_ratings WHERE file_id = ?1", [id.get()], |r| r.get(0)).optional()?)
}

/// The `borders_due` flag of the analysis of the file called `name`: 1 while the borders pass still has to read it.
pub fn borders_due(c: &Catalog, name: &str) -> Result<i64> {
    Ok(c.conn.query_row(
        "SELECT a.borders_due FROM analysis a JOIN files f ON f.id = a.file_id WHERE f.name = ?1",
        [name.as_bytes()],
        |r| r.get(0),
    )?)
}

/// Whether the file called `name` owes its pairs (a `pairs_owed` row): true until the pairing pass has paired it.
pub fn owes_pairs(c: &Catalog, name: &str) -> Result<bool> {
    Ok(c.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pairs_owed o JOIN files f ON f.id = o.file_id WHERE f.name = ?1)",
        [name.as_bytes()],
        |r| r.get(0),
    )?)
}

/// The page cache of the catalog's connection, in KiB (`PRAGMA cache_size`, which the catalog sets in KiB).
pub fn cache_kib(c: &Catalog) -> Result<i64> {
    Ok(-c.conn.query_row("PRAGMA cache_size", [], |r| r.get::<_, i64>(0))?)
}

/// Every pair stored, as its two files' ids (`a` below `b`) and its distance, in order.
pub fn pairs(c: &Catalog) -> Result<Vec<(i64, i64, i64)>> {
    let mut stmt = c.conn.prepare("SELECT a, b, distance FROM pdq_pairs ORDER BY a, b")?;
    Ok(stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?)
}

/// The pairs shown (`present_pdq_pairs`: both files present), as [`pairs`] lists them.
pub fn shown_pairs(c: &Catalog) -> Result<Vec<(i64, i64, i64)>> {
    let mut stmt = c.conn.prepare("SELECT a, b, distance FROM present_pdq_pairs ORDER BY a, b")?;
    Ok(stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?)
}

/// The file's volume went away and came back renumbered, and it was not there meanwhile: its row is missing, on the
/// volume `volume` (one no scan sees), under the name `name`, as a scan of the old volume left it.
pub fn on_lost_volume(c: &Catalog, id: FileId, volume: i64, name: &str) -> Result<()> {
    c.conn.execute(
        "UPDATE files SET volume = ?2, name = ?3, missing = 1 WHERE id = ?1",
        params![id.get(), volume, name.as_bytes()],
    )?;
    Ok(())
}

/// The file's analysis was made by a build of import version `version` (an older one, say).
pub fn set_analysis_version(c: &Catalog, id: FileId, version: i64) -> Result<()> {
    c.conn.execute("UPDATE analysis SET version = ?2 WHERE file_id = ?1", params![id.get(), version])?;
    Ok(())
}

/// Makes every pair write fail, until [`allow_pair_writes`]: a pass that writes a pair then fails as a write does.
pub fn fail_pair_writes(c: &Catalog) -> Result<()> {
    Ok(c.conn.execute_batch(
        "CREATE TRIGGER pairs_fault BEFORE INSERT ON pdq_pairs BEGIN SELECT RAISE(ABORT, 'a test fault'); END;",
    )?)
}

/// Lets the pair writes through again (see [`fail_pair_writes`]).
pub fn allow_pair_writes(c: &Catalog) -> Result<()> {
    Ok(c.conn.execute_batch("DROP TRIGGER pairs_fault;")?)
}

/// Make the borders of the file called `name` due again, its file unchanged (as an analysis made again does).
pub fn make_borders_due(c: &Catalog, name: &str) -> Result<()> {
    c.conn.execute(
        "UPDATE analysis SET borders_due = 1 WHERE file_id = (SELECT id FROM files WHERE name = ?1)",
        [name.as_bytes()],
    )?;
    Ok(())
}

/// The width and height the analysis holds for each present file, by name.
pub fn analysis_sizes(c: &Catalog) -> Result<Vec<(u32, u32)>> {
    let mut stmt =
        c.conn.prepare("SELECT a.width, a.height FROM analysis a JOIN files f ON f.id = a.file_id ORDER BY f.name")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The turn the analysis holds for the file (its pending turn, when a turn is queued and not yet analysed).
pub fn analysis_rotation(c: &Catalog, id: FileId) -> Result<i64> {
    Ok(c.conn.query_row("SELECT rotation FROM analysis WHERE file_id = ?1", [id.get()], |r| r.get(0))?)
}

/// Sets the turn the analysis holds for the file.
pub fn set_analysis_rotation(c: &Catalog, id: FileId, turns: u8) -> Result<()> {
    c.conn.execute("UPDATE analysis SET rotation = ?2 WHERE file_id = ?1", params![id.get(), turns])?;
    Ok(())
}

/// One row per present file that has a video analysis, by name: the name, the kind, the width, the height, the
/// duration and the poster time in milliseconds, and whether the pixel hash is missing (1) or present (0).
pub type MediaRow = (String, String, i64, i64, i64, i64, Option<i64>);

/// A present file's stored hashes: its name, its file hash and its pixel hash (`None` while not made).
pub type StoredHashes = (String, Option<Vec<u8>>, Option<Vec<u8>>);

/// Every present file's stored hashes, by name.
pub fn stored_hashes(c: &Catalog) -> Result<Vec<StoredHashes>> {
    let mut stmt = c.conn.prepare(
        "SELECT CAST(f.name AS TEXT), a.file_hash, a.pixel_hash
             FROM files f JOIN analysis a ON a.file_id = f.id ORDER BY f.name",
    )?;
    Ok(stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?)
}

/// The media rows of the present files (see [`MediaRow`]).
pub fn media_rows(c: &Catalog) -> Result<Vec<MediaRow>> {
    let mut stmt = c.conn.prepare(
        "SELECT CAST(f.name AS TEXT), f.kind, a.width, a.height, a.duration_ms, a.poster_ms, a.pixel_hash IS NULL
             FROM files f JOIN analysis a ON a.file_id = f.id ORDER BY f.name",
    )?;
    let rows =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The capture time the analysis holds for the file called `name`, if it has one.
pub fn taken_of(c: &Catalog, name: &str) -> Result<Option<i64>> {
    Ok(c.conn.query_row(
        "SELECT taken FROM analysis a JOIN files f ON f.id = a.file_id WHERE f.name = ?1",
        [name.as_bytes()],
        |r| r.get(0),
    )?)
}

/// The Unix time at which the local clock shows `wall` seconds (as SQLite reads the local time zone).
pub fn local_epoch(c: &Catalog, wall: i64) -> Result<i64> {
    Ok(c.conn.query_row("SELECT unixepoch(?1, 'unixepoch', 'localtime')", [wall], |r| r.get(0))?)
}

/// Writes still to be tried: their attempts are below the queue's limit. (The queue's own count, `Catalog::left_to_write`,
/// leaves out a failed write with no temp file, which is not tried again until its time comes.)
pub fn writes_still_tried(c: &Catalog) -> Result<i64> {
    Ok(c.conn.query_row("SELECT count(*) FROM file_writes WHERE attempts < ?1", [MAX_ATTEMPTS], |r| r.get(0))?)
}

/// The write rows the queue holds for the file, whatever their kind.
pub fn writes_queued(c: &Catalog, id: FileId) -> Result<i64> {
    Ok(c.conn.query_row("SELECT count(*) FROM file_writes WHERE file_id = ?1", [id.get()], |r| r.get(0))?)
}

/// The state of the file's write row in the queue: the tries so far, the temp file a finishing copy left, and the
/// last error.
pub struct WriteRow {
    pub attempts: i64,
    pub temp: Option<pg_model::rules::Path>,
    pub error: Option<String>,
}

/// The file's write row (it must be in the queue: an error otherwise).
pub fn write_row(c: &Catalog, id: FileId) -> Result<WriteRow> {
    let (attempts, temp, error): (i64, Option<Vec<u8>>, Option<String>) =
        c.conn.query_row("SELECT attempts, temp, error FROM file_writes WHERE file_id = ?1", [id.get()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    let temp = temp
        .map(|bytes| c.rules().parse_bytes(&bytes).ok_or_else(|| anyhow::anyhow!("a temp path that doesn't parse")))
        .transpose()?;
    Ok(WriteRow { attempts, temp, error })
}

/// The file's write row of `kind` (`rating` or `rotation`), for a file with both queued (it must be in the queue: an
/// error otherwise).
pub fn write_row_of(c: &Catalog, id: FileId, kind: &str) -> Result<WriteRow> {
    let (attempts, temp, error): (i64, Option<Vec<u8>>, Option<String>) = c.conn.query_row(
        "SELECT attempts, temp, error FROM file_writes WHERE file_id = ?1 AND kind = ?2",
        params![id.get(), kind],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let temp = temp
        .map(|bytes| c.rules().parse_bytes(&bytes).ok_or_else(|| anyhow::anyhow!("a temp path that doesn't parse")))
        .transpose()?;
    Ok(WriteRow { attempts, temp, error })
}

/// Queues a write of `kind` (`rating` or `rotation`) with `value` for the file, as a change made by hand.
pub fn queue_write(c: &Catalog, id: FileId, kind: &str, value: i64) -> Result<()> {
    c.conn.execute(
        "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, ?2, ?3, 0)",
        params![id.get(), kind, value],
    )?;
    Ok(())
}

/// Queues a write whose copy was finished when the app stopped: its temp file `temp` is beside the original.
pub fn queue_finished_copy(
    c: &Catalog,
    id: FileId,
    kind: &str,
    value: i64,
    temp: &pg_model::rules::Path,
) -> Result<()> {
    c.conn.execute(
        "INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value) VALUES (?1, ?2, ?3, 0, ?4, ?3)",
        params![id.get(), kind, value, c.rules().join(temp)],
    )?;
    Ok(())
}

/// Makes the catalog at `path` the layout before the temp was bytes: `file_writes` with `temp` as text, holding one
/// queued rating for `id` whose temp is `temp_text`. The next open carries that row (its temp as bytes). It takes the
/// path, not a `Catalog`: the test has closed its catalog, and the statements are the old layout's.
pub fn set_text_temp_layout(path: &std::path::Path, id: FileId, temp_text: &str) -> Result<()> {
    let old = rusqlite::Connection::open(path)?;
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
    )?;
    old.execute(
        "INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value) VALUES (?1, 'rating', 3, 0, ?2, 3)",
        params![id.get(), temp_text],
    )?;
    Ok(())
}

/// The stored notice rows of kind `kind` are rewritten to kind `to_kind` with `data` as their payload: a row that won't
/// read, as an older build or a damaged file leaves one. Returns how many rows changed.
pub fn rewrite_notices(c: &Catalog, kind: &str, to_kind: &str, data: &str) -> Result<usize> {
    Ok(c.conn.execute("UPDATE notices SET kind = ?2, data = ?3 WHERE kind = ?1", params![kind, to_kind, data])?)
}
