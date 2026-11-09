//! The file writes queue (`file_writes`): what the user changed that the files themselves should carry, and what
//! the writer does with it. A rating or a turn is queued here in the same transaction that changes the catalog
//! (the producers: `set_rating`, `queue_rotation`). The writer takes the oldest settled job with `next_ready`, records
//! the temp file it made with `record_temp` once the original is open for the copy (a recorded temp means the copy
//! into the original may have started), and ends the job with `done` (with what the write changed in the catalog, a
//! `WriteEffect`), `failed`, or `drop_gone`. A recorded temp is never discarded: a copy that keeps failing stays
//! recorded, and `retry_copies` lets each start of the writer try it again. Each call is one transaction or one
//! statement, so no reader sees a write both done and still queued.

use crate::Catalog;
use crate::error::Result;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use pg_model::format::Format;
use pg_model::ids::FileId;
use pg_model::media::Kind;
use pg_model::orientation::turns_swap;
use pg_model::rating::Rating;
use pg_model::rules::Path as RulePath;
use pg_model::turns::Turns;
use pg_model::write::WriteKind;

use super::{DELETE_FILE_RATING, SET_FILE_RATING};

/// Failed writes are retried at most this many times (offline drives retry without counting).
pub const MAX_ATTEMPTS: i64 = 5;

/// A queued write, as the writer takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub file_id: FileId,
    pub kind: WriteKind,
    /// The rating, or the quarter turns still to write (0 to 3).
    pub value: i64,
    /// Set once the copy of the new content into the original may have started (the temp is complete, and the
    /// original is open for it): the temp's path, and the value it carries.
    pub temp: Option<(RulePath, i64)>,
    /// Failed attempts so far.
    pub attempts: i64,
}

/// A file a write rewrote: its (size, mtime) on disk before and after (`before` is (-1, -1) when it isn't known), and
/// the BLAKE3 of the new bytes when only metadata changed (the analysis stays valid, and follows the file). `None` when
/// the pixels did, so the analysis is redone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rewrite {
    pub before: (i64, i64),
    pub after: (i64, i64),
    pub metadata_only: Option<[u8; 32]>,
}

/// What a finished write changed in the catalog. Applied in the transaction that takes the job off the queue
/// (`done`), so the catalog and the queue never disagree. The effect's value is the job's: `done` checks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteEffect {
    /// The rating is in the file, which was rewritten to hold it (`rating`, 0 to 5).
    FileRating { rewrite: Rewrite, rating: u8 },
    /// The turns are in the file, which was rewritten to show them: the analysis is turned back to match.
    TurnedBack { rewrite: Rewrite, turns: Turns },
    /// The file can't hold the turns: the catalog keeps them (`files.rotation`).
    KeptInCatalog(Turns),
    /// Nothing to record: the write is done, and the catalog already says so.
    Nothing,
}

/// Whether a file of `kind` (`files.kind`) and analyzed `format` (`None` when it isn't analyzed yet) may hold a rating
/// of its own: an image (a video keeps its rating in the catalog, as rewriting it would copy GBs), whose format can hold
/// one or isn't known yet. The writer decides for an unknown format once it has read the file (`pg_writeback`).
pub fn rating_in_file_possible(kind: &str, format: Option<&str>) -> bool {
    kind == Kind::Image.key() && format.and_then(Format::from_name).is_none_or(pg_metadata::can_hold_rating)
}

/// Rate a file: 1–5, or `None` to clear. The catalog changes at once. With `write_back`, the rating is also queued
/// for the file itself (`pg_writeback`) when it may hold one ([`rating_in_file_possible`]: an image whose format can
/// hold one, or that isn't analyzed yet, but not one whose analysis failed) and it differs from what the file holds;
/// clearing writes 0 only into a file that holds a rating.
pub fn set_rating(catalog: &Catalog, file_id: FileId, rating: Option<Rating>, write_back: bool) -> Result<()> {
    let conn = &catalog.conn;
    let tx = crate::schema::write_transaction(conn)?;
    let file_id = file_id.get();
    match rating {
        Some(rating) => tx.execute(
            "INSERT INTO ratings(file_id, rating, updated) VALUES (?1, ?2, unixepoch())
             ON CONFLICT(file_id) DO UPDATE SET rating = excluded.rating, updated = excluded.updated",
            params![file_id, rating.get()],
        )?,
        None => tx.execute("DELETE FROM ratings WHERE file_id = ?1", [file_id])?,
    };
    // The file's kind, its analyzed format, and whether its analysis failed (a file the import couldn't read isn't
    // written into).
    let target: Option<(String, Option<String>, bool)> = tx
        .query_row(
            "SELECT f.kind, a.format, a.error IS NOT NULL FROM files f LEFT JOIN analysis a ON a.file_id = f.id
             WHERE f.id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let can_hold =
        target.is_some_and(|(kind, format, failed)| !failed && rating_in_file_possible(&kind, format.as_deref()));
    let in_file: Option<u8> =
        tx.query_row("SELECT value FROM file_ratings WHERE file_id = ?1", [file_id], |r| r.get(0)).optional()?;
    // What the file should hold: the rating, or none (0) if it holds one and the rating is cleared.
    let wanted = match rating {
        Some(rating) => Some(rating.get()),
        None if in_file.is_some_and(|v| v > 0) => Some(0),
        None => in_file,
    };
    if write_back && can_hold && wanted != in_file {
        tx.execute(
            "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rating', ?2, unixepoch())
             ON CONFLICT(file_id, kind) DO UPDATE SET value = excluded.value, queued = excluded.queued,
                attempts = 0, error = NULL",
            params![file_id, wanted],
        )?;
    } else {
        // Back to what the file holds (or not written back): nothing left to write. A write already being
        // copied in finishes, and then this value follows it; its attempts start again, so a copy that had failed
        // up to the cap is tried again for the change.
        tx.execute("DELETE FROM file_writes WHERE file_id = ?1 AND kind = 'rating' AND temp IS NULL", [file_id])?;
        tx.execute(
            "UPDATE file_writes SET value = ?2, attempts = 0 WHERE file_id = ?1 AND kind = 'rating'",
            params![file_id, wanted.unwrap_or(0)],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Turn a file `turns` quarter turns clockwise: queued for the writer, which turns the file itself losslessly where
/// it can and keeps the turn in the catalog otherwise. Turns add up until written.
pub fn queue_rotation(catalog: &Catalog, file_id: FileId, turns: Turns) -> Result<()> {
    let conn = &catalog.conn;
    let tx = crate::schema::write_transaction(conn)?;
    let (file_id, turns) = (file_id.get(), turns.get());
    tx.execute(
        "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rotation', ?2 % 4, unixepoch())
         ON CONFLICT(file_id, kind) DO UPDATE SET value = (value + excluded.value) % 4, queued = excluded.queued,
            attempts = 0, error = NULL",
        params![file_id, turns],
    )?;
    tx.execute(
        "DELETE FROM file_writes WHERE file_id = ?1 AND kind = 'rotation' AND value = 0 AND temp IS NULL",
        [file_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// [`next_ready`]'s statement. Its order is the queue index's (`file_writes_next`): no sort of the queue.
const NEXT_READY: &str = "SELECT file_id, kind, value, temp, temp_value, attempts FROM file_writes
     WHERE attempts < ?1 AND queued <= unixepoch() - ?2 * (attempts + 1)
     ORDER BY temp IS NULL, queued LIMIT 1";

/// The oldest job the writer can try now: an interrupted copy first (its temp is complete), then the oldest change
/// that has settled for `settle_secs`. A failed job waits `settle_secs` longer for each attempt so far.
pub fn next_ready(catalog: &Catalog, settle_secs: i64) -> Result<Option<Job>> {
    let conn = &catalog.conn;
    let job = conn
        .query_row(NEXT_READY, params![MAX_ATTEMPTS, settle_secs], |r| {
            let kind: String = r.get(1)?;
            let temp: Option<Vec<u8>> = r.get(3)?;
            let temp_value: Option<i64> = r.get(4)?;
            // The temp's path is the rules' bytes (`record_temp`); one that doesn't parse is a damaged row.
            let temp = match temp {
                Some(bytes) => Some(
                    catalog
                        .rules()
                        .parse_bytes(&bytes)
                        .ok_or_else(|| rusqlite::Error::InvalidColumnType(3, "temp".into(), Type::Blob))?,
                ),
                None => None,
            };
            Ok(Job {
                file_id: FileId::new(r.get(0)?),
                kind: WriteKind::from_key(&kind)
                    .ok_or_else(|| rusqlite::Error::InvalidColumnType(1, kind.clone(), Type::Text))?,
                value: r.get(2)?,
                temp: temp.zip(temp_value),
                attempts: r.get(5)?,
            })
        })
        .optional()?;
    Ok(job)
}

/// Whether a job is left that may still be tried (the writer waits for it to settle, rather than stopping).
pub fn attemptable(catalog: &Catalog) -> Result<bool> {
    let conn = &catalog.conn;
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM file_writes WHERE attempts < ?1)", [MAX_ATTEMPTS], |r| r.get(0))?)
}

/// Every queued write, failing or not.
pub fn remaining(catalog: &Catalog) -> Result<usize> {
    let conn = &catalog.conn;
    Ok(conn.query_row("SELECT count(*) FROM file_writes", [], |r| r.get(0))?)
}

/// The queued writes still to make now: those that have retries left, and aren't failing without a temp to finish.
pub fn left_to_write(catalog: &Catalog) -> Result<i64> {
    let conn = &catalog.conn;
    Ok(conn.query_row(
        "SELECT count(*) FROM file_writes WHERE attempts < ?1 AND (error IS NULL OR temp IS NOT NULL)",
        [MAX_ATTEMPTS],
        |r| r.get(0),
    )?)
}

/// The latest errors of the queue, for the command line: newest first. `error` is set only by [`failed`], which
/// stamps `queued` with the time of the failure, so `queued` orders them; the file and the kind break a tie within a
/// second, so the rows shown under the limit are always the same.
pub fn recent_errors(catalog: &Catalog, limit: usize) -> Result<Vec<String>> {
    let conn = &catalog.conn;
    let mut stmt = conn.prepare(
        "SELECT error FROM file_writes WHERE error IS NOT NULL ORDER BY queued DESC, file_id DESC, kind DESC LIMIT ?1",
    )?;
    let errors = stmt
        .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(errors)
}

/// The new content of a job is complete and durable at `temp`, carrying `value`, and the original is open for the copy
/// into it, which starts next: from here the original may be part-written. Until `done`, readers see the file as being
/// written (`pg_model::state::FileState::writing`), and the writer finishes the copy from `temp` before anything else.
///
/// `false` when the job's row is no longer there (it was removed after the writer read it, so nothing is recorded): the
/// copy must not start then. The caller leaves the original as it is.
pub fn record_temp(catalog: &Catalog, job: &Job, temp: &RulePath, value: i64) -> Result<bool> {
    let conn = &catalog.conn;
    let bytes = catalog.rules().join(temp);
    let recorded = conn.execute(
        "UPDATE file_writes SET temp = ?3, temp_value = ?4 WHERE file_id = ?1 AND kind = ?2",
        params![job.file_id.get(), job.kind.key(), bytes, value],
    )?;
    Ok(recorded == 1)
}

/// The job is written: its value (a rating, or the turns) is now in the file or the catalog. `effect` changes the
/// catalog in the same transaction that takes the job off the queue (a rating or turns changed again meanwhile stays
/// queued for the rest).
pub fn done(catalog: &Catalog, job: &Job, effect: WriteEffect) -> Result<()> {
    let conn = &catalog.conn;
    let (file_id, kind) = (job.file_id.get(), job.kind.key());
    // What the queue records as written: the job's rating, or its turns.
    let written = job.value;
    let tx = crate::schema::write_transaction(conn)?;
    match effect {
        WriteEffect::FileRating { rewrite, rating } => {
            debug_assert!(job.kind == WriteKind::Rating && i64::from(rating) == written);
            file_rewritten(&tx, job.file_id, rewrite.before, rewrite.after, rewrite.metadata_only)?;
            set_file_rating(&tx, job.file_id, Rating::new(rating))?;
        }
        WriteEffect::TurnedBack { rewrite, turns } => {
            debug_assert!(job.kind == WriteKind::Turns && i64::from(turns.get()) == written.rem_euclid(4));
            file_rewritten(&tx, job.file_id, rewrite.before, rewrite.after, rewrite.metadata_only)?;
            analysis_turned_back(&tx, job.file_id, turns)?;
        }
        WriteEffect::KeptInCatalog(turns) => {
            debug_assert!(job.kind == WriteKind::Turns && i64::from(turns.get()) == written.rem_euclid(4));
            add_catalog_rotation(&tx, job.file_id, turns)?;
        }
        WriteEffect::Nothing => {}
    }
    match job.kind {
        WriteKind::Turns => {
            tx.execute(
                "UPDATE file_writes SET value = (value - ?2 + 4) % 4 WHERE file_id = ?1 AND kind = 'rotation'",
                params![file_id, written],
            )?;
            tx.execute("DELETE FROM file_writes WHERE file_id = ?1 AND kind = 'rotation' AND value = 0", [file_id])?;
            // The upright size at once: the grid lays the file out turned before its analysis is redone.
            if turns_swap(Turns::new(u8::try_from(written.rem_euclid(4)).unwrap_or(u8::MAX))) {
                tx.execute("UPDATE analysis SET width = height, height = width WHERE file_id = ?1", [file_id])?;
            }
        }
        WriteKind::Rating => {
            tx.execute(
                "DELETE FROM file_writes WHERE file_id = ?1 AND kind = ?2 AND value = ?3",
                params![file_id, kind, written],
            )?;
        }
    }
    tx.execute(
        "UPDATE file_writes SET temp = NULL, temp_value = NULL, attempts = 0, error = NULL
         WHERE file_id = ?1 AND kind = ?2",
        params![file_id, kind],
    )?;
    tx.commit()?;
    Ok(())
}

/// The write failed. An offline drive's failure doesn't count as an attempt. The message is the status line's until a
/// success.
pub fn failed(catalog: &Catalog, job: &Job, message: &str, offline: bool) -> Result<()> {
    let conn = &catalog.conn;
    conn.execute(
        "UPDATE file_writes SET attempts = attempts + ?3, error = ?4, queued = unixepoch()
         WHERE file_id = ?1 AND kind = ?2",
        params![job.file_id.get(), job.kind.key(), if offline { 0 } else { 1 }, message],
    )?;
    Ok(())
}

/// The file is no longer in the library (missing, or unlisted): the write is dropped.
pub fn drop_gone(catalog: &Catalog, job: &Job) -> Result<()> {
    let conn = &catalog.conn;
    conn.execute(
        "DELETE FROM file_writes WHERE file_id = ?1 AND kind = ?2",
        params![job.file_id.get(), job.kind.key()],
    )?;
    Ok(())
}

/// The copies into the originals that failed up to the cap get their attempts back: the writer calls it when it starts,
/// so each start tries a stuck copy again. Its temp is the only whole copy of the new content while the original may be
/// part-written, so it is never given up. The number of rows reset.
pub fn retry_copies(catalog: &Catalog) -> Result<usize> {
    let conn = &catalog.conn;
    Ok(conn.execute("UPDATE file_writes SET attempts = 0 WHERE temp IS NOT NULL AND attempts > 0", [])?)
}

/// Record what the file holds now (the rating it was last written with, or the import's reads).
pub(crate) fn set_file_rating(conn: &Connection, file_id: FileId, value: Option<Rating>) -> Result<()> {
    match value {
        Some(v) => conn.prepare_cached(SET_FILE_RATING)?.execute(params![file_id.get(), v.get()])?,
        None => conn.prepare_cached(DELETE_FILE_RATING)?.execute([file_id.get()])?,
    };
    Ok(())
}

/// The file was rewritten in place: record its new size and mtime, so the scanner and the import see it as the file
/// it is. `before` is the (size, mtime) the file had on disk just before the write. After a metadata-only change
/// (`metadata_only` = the new BLAKE3) an analysis made from exactly those bytes stays valid and follows the file:
/// nothing is re-imported. After a pixel change it is left stale, so the import redoes it (new upright size, hashes,
/// PDQ).
fn file_rewritten(
    tx: &Transaction,
    file_id: FileId,
    before: (i64, i64),
    after: (i64, i64),
    metadata_only: Option<[u8; 32]>,
) -> Result<()> {
    tx.execute("UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1", params![file_id.get(), after.0, after.1])?;
    if let Some(file_hash) = metadata_only {
        tx.execute(
            "UPDATE analysis SET size = ?2, mtime = ?3, file_hash = ?4
             WHERE file_id = ?1 AND size = ?5 AND mtime = ?6",
            params![file_id.get(), after.0, after.1, &file_hash[..], before.0, before.1],
        )?;
    }
    Ok(())
}

/// A turn written into the file (its pixels or its orientation tag): the analysis made before is of the file as it is
/// now turned back by them, so it is recorded as made at the catalog rotation that many turns back. It is stale
/// anyway (the mtime changed); until the import redoes it, the placeholder made from it shows the file as it is now.
fn analysis_turned_back(tx: &Transaction, file_id: FileId, turns: Turns) -> Result<()> {
    tx.execute(
        "UPDATE analysis SET rotation = (rotation + 4 - ?2 % 4) % 4 WHERE file_id = ?1",
        params![file_id.get(), turns.get()],
    )?;
    Ok(())
}

/// The turn is kept in the catalog (the file can't hold it): the analysis, made without it, is stale (`files.pending`),
/// and the thumbnails no longer match (thumbs::Source).
fn add_catalog_rotation(tx: &Transaction, file_id: FileId, turns: Turns) -> Result<()> {
    tx.execute("UPDATE files SET rotation = (rotation + ?2) % 4 WHERE id = ?1", params![file_id.get(), turns.get()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::library;
    use crate::fixture::raw;
    use pg_model::rules::PathRules;

    #[test]
    fn ratings_and_turns_queue_for_the_file() {
        let c = library(PathRules::Windows);
        set_rating(&c, FileId::new(2), Rating::new(3), true).unwrap();
        assert_eq!(raw::queued(&c, 2, "rating"), Some(3));
        // Back to what the file holds (nothing): nothing to write.
        set_rating(&c, FileId::new(2), None, true).unwrap();
        assert_eq!(raw::queued(&c, 2, "rating"), None);
        // A file that holds 4: clearing writes 0.
        raw::holds_rating(&c, FileId::new(1), Rating::new(4));
        set_rating(&c, FileId::new(1), None, true).unwrap();
        assert_eq!(raw::queued(&c, 1, "rating"), Some(0));
        // BMP can't hold one; write-back off writes nothing.
        set_rating(&c, FileId::new(4), Rating::new(5), true).unwrap();
        set_rating(&c, FileId::new(3), Rating::new(5), false).unwrap();
        assert_eq!((raw::queued(&c, 4, "rating"), raw::queued(&c, 3, "rating")), (None, None));
        // Turns add up modulo 4; a full turn leaves nothing to do.
        queue_rotation(&c, FileId::new(1), Turns::new(1)).unwrap();
        queue_rotation(&c, FileId::new(1), Turns::new(1)).unwrap();
        assert_eq!(raw::queued(&c, 1, "rotation"), Some(2));
        queue_rotation(&c, FileId::new(1), Turns::new(2)).unwrap();
        assert_eq!(raw::queued(&c, 1, "rotation"), None);
    }

    #[test]
    fn a_job_is_taken_once_it_settles_and_done_applies_its_effect() {
        let c = library(PathRules::Windows);
        set_rating(&c, FileId::new(1), Rating::new(3), true).unwrap();
        queue_rotation(&c, FileId::new(2), Turns::new(1)).unwrap();
        // Both are new: nothing is ready until they settle (a settle time of 0 takes them now).
        let first = next_ready(&c, 0).unwrap().unwrap();
        assert_eq!((first.file_id, first.kind, first.value), (FileId::new(1), WriteKind::Rating, 3));
        // The rating is in the file: the catalog records it, and the job leaves the queue.
        let after = (11, 101);
        let rewrite = Rewrite { before: (10, 1), after, metadata_only: Some([7; 32]) };
        done(&c, &first, WriteEffect::FileRating { rewrite, rating: 3 }).unwrap();
        assert_eq!(raw::queued(&c, 1, "rating"), None);
        assert_eq!(raw::file_size(&c, FileId::new(1)), 11);
        // The turn can't go in the file: the catalog keeps it, and the upright size swaps at once.
        let turn = next_ready(&c, 0).unwrap().unwrap();
        assert_eq!((turn.file_id, turn.kind, turn.value), (FileId::new(2), WriteKind::Turns, 1));
        done(&c, &turn, WriteEffect::KeptInCatalog(Turns::new(1))).unwrap();
        assert_eq!(raw::queued(&c, 2, "rotation"), None);
        assert_eq!(raw::rotation_and_width(&c, FileId::new(2)), (1, 3));
        assert!(next_ready(&c, 0).unwrap().is_none());
    }

    #[test]
    fn a_failure_waits_longer_and_an_offline_drive_does_not_count() {
        let c = library(PathRules::Windows);
        set_rating(&c, FileId::new(1), Rating::new(3), true).unwrap();
        let job = next_ready(&c, 0).unwrap().unwrap();
        failed(&c, &job, "denied", false).unwrap();
        // One attempt in: due two settle times later, not now.
        assert!(next_ready(&c, 1).unwrap().is_none());
        assert!(attemptable(&c).unwrap());
        let job = next_ready(&c, 0).unwrap().unwrap();
        assert_eq!(job.attempts, 1);
        failed(&c, &job, "offline", true).unwrap();
        let job = next_ready(&c, 0).unwrap().unwrap();
        assert_eq!(job.attempts, 1, "an offline failure is not an attempt");
        assert_eq!(recent_errors(&c, 5).unwrap(), ["offline"]);
        drop_gone(&c, &job).unwrap();
        assert_eq!(remaining(&c).unwrap(), 0);
    }

    /// The latest errors come first, a tie within a second broken by the file and the kind, and the limit keeps the
    /// newest.
    #[test]
    fn recent_errors_are_the_newest_in_a_fixed_order() {
        let c = library(PathRules::Windows);
        set_rating(&c, FileId::new(1), Rating::new(3), true).unwrap();
        set_rating(&c, FileId::new(2), Rating::new(3), true).unwrap();
        queue_rotation(&c, FileId::new(1), Turns::new(1)).unwrap();
        let fail = |id: i64, kind: &str, queued: i64, error: &str| {
            c.conn
                .execute(
                    "UPDATE file_writes SET error = ?3, queued = ?4 WHERE file_id = ?1 AND kind = ?2",
                    params![id, kind, error, queued],
                )
                .unwrap();
        };
        fail(1, "rating", 100, "oldest");
        fail(1, "rotation", 200, "tied, lower file");
        fail(2, "rating", 200, "tied, higher file");
        assert_eq!(recent_errors(&c, 5).unwrap(), ["tied, higher file", "tied, lower file", "oldest"]);
        assert_eq!(recent_errors(&c, 2).unwrap(), ["tied, higher file", "tied, lower file"]);
        // A tie on the file too: the kind decides.
        fail(1, "rating", 200, "tied, rating");
        assert_eq!(recent_errors(&c, 3).unwrap(), ["tied, higher file", "tied, lower file", "tied, rating"]);
    }

    fn attempts(c: &Catalog, id: i64, kind: &str) -> i64 {
        c.conn
            .query_row("SELECT attempts FROM file_writes WHERE file_id = ?1 AND kind = ?2", params![id, kind], |r| {
                r.get(0)
            })
            .unwrap()
    }

    /// A finishing copy is done with the value its temp carries: a turn or a rating made after the temp was recorded
    /// stays queued for the rest, with its attempts back to 0.
    #[test]
    fn a_finished_copy_leaves_a_later_change_queued() {
        let c = library(PathRules::Windows);
        // Three turns recorded in a temp; then one more turn (a full circle in all), and a failed copy.
        queue_rotation(&c, FileId::new(1), Turns::new(3)).unwrap();
        let job = next_ready(&c, 0).unwrap().unwrap();
        let temp = PathRules::Windows.parse(r"D:\p\.b.jpg.turns.pg-write").unwrap();
        record_temp(&c, &job, &temp, 3).unwrap();
        queue_rotation(&c, FileId::new(1), Turns::new(1)).unwrap();
        assert_eq!(raw::queued(&c, 1, "rotation"), Some(0), "kept while its temp is recorded");
        let job = next_ready(&c, 0).unwrap().unwrap();
        assert_eq!(job.temp, Some((temp.clone(), 3)));
        failed(&c, &job, "locked", false).unwrap();
        let job = next_ready(&c, 0).unwrap().unwrap();
        let rewrite = Rewrite { before: (-1, -1), after: (31, 7), metadata_only: None };
        done(&c, &Job { value: 3, ..job.clone() }, WriteEffect::TurnedBack { rewrite, turns: Turns::new(3) }).unwrap();
        // Three turns are in the file: one is left to make the full circle.
        assert_eq!(raw::queued(&c, 1, "rotation"), Some(1));
        assert_eq!(attempts(&c, 1, "rotation"), 0);
        let rest = next_ready(&c, 0).unwrap().unwrap();
        assert_eq!(rest.temp, None);
        drop_gone(&c, &rest).unwrap();

        // A rating of 3 recorded in a temp, then 5: the copy of 3 is done, and 5 stays queued.
        set_rating(&c, FileId::new(2), Rating::new(3), true).unwrap();
        let job = next_ready(&c, 0).unwrap().unwrap();
        assert_eq!((job.file_id, job.kind), (FileId::new(2), WriteKind::Rating));
        let temp = PathRules::Windows.parse(r"E:\p\.a.png.rating.pg-write").unwrap();
        record_temp(&c, &job, &temp, 3).unwrap();
        set_rating(&c, FileId::new(2), Rating::new(5), true).unwrap();
        let job = next_ready(&c, 0).unwrap().unwrap();
        assert_eq!((job.file_id, job.value, job.temp.as_ref().map(|t| t.1)), (FileId::new(2), 5, Some(3)));
        let rewrite = Rewrite { before: (-1, -1), after: (11, 9), metadata_only: None };
        done(&c, &Job { value: 3, ..job.clone() }, WriteEffect::FileRating { rewrite, rating: 3 }).unwrap();
        assert_eq!(raw::queued(&c, 2, "rating"), Some(5));
        let in_file: u8 =
            c.conn.query_row("SELECT value FROM file_ratings WHERE file_id = 2", [], |r| r.get(0)).unwrap();
        assert_eq!(in_file, 3, "the file holds what the temp carried");
    }

    /// A copy that failed up to the cap keeps its temp, and is tried again: after a change to the file (even one that
    /// only goes back to what the file holds), and at each start of the writer.
    #[test]
    fn a_stuck_copy_is_tried_again_after_a_change_and_at_a_start() {
        let c = library(PathRules::Windows);
        set_rating(&c, FileId::new(2), Rating::new(3), true).unwrap();
        let job = next_ready(&c, 0).unwrap().unwrap();
        let temp = PathRules::Windows.parse(r"E:\p\.a.png.rating.pg-write").unwrap();
        record_temp(&c, &job, &temp, 3).unwrap();
        let stuck = |c: &Catalog| {
            for _ in 0..MAX_ATTEMPTS {
                failed(c, &job, "locked", false).unwrap();
            }
            assert_eq!(attempts(c, 2, "rating"), MAX_ATTEMPTS);
            assert!(next_ready(c, 0).unwrap().is_none(), "at the cap");
        };
        stuck(&c);
        // Back to what the file holds (none): the row stays for its copy, with its attempts back.
        set_rating(&c, FileId::new(2), None, true).unwrap();
        assert_eq!(attempts(&c, 2, "rating"), 0);
        assert_eq!(next_ready(&c, 0).unwrap().unwrap().temp, Some((temp, 3)));
        stuck(&c);
        assert_eq!(retry_copies(&c).unwrap(), 1);
        assert_eq!(attempts(&c, 2, "rating"), 0);
        // A failing write without a temp is not a copy: it keeps its attempts.
        queue_rotation(&c, FileId::new(1), Turns::new(1)).unwrap();
        let turn = Job { file_id: FileId::new(1), kind: WriteKind::Turns, value: 1, temp: None, attempts: 0 };
        failed(&c, &turn, "locked", false).unwrap();
        assert_eq!(retry_copies(&c).unwrap(), 0);
        assert_eq!(attempts(&c, 1, "rotation"), 1);
    }

    /// A rating is queued for an image whose format can hold one or isn't known yet (not analyzed), and not for a
    /// video, a format that can't hold one, or a file whose analysis failed.
    #[test]
    fn a_rating_is_queued_for_the_files_that_may_hold_one() {
        let c = crate::browse::fixtures::mixed_library(PathRules::Windows);
        // 5: a video; 7: its analysis failed; 8: not analyzed yet; 4: a BMP.
        for id in [4, 5, 7, 8] {
            set_rating(&c, FileId::new(id), Rating::new(4), true).unwrap();
        }
        let queued: Vec<Option<i64>> = [4, 5, 7, 8].iter().map(|&id| raw::queued(&c, id, "rating")).collect();
        assert_eq!(queued, [None, None, None, Some(4)]);
        assert!(rating_in_file_possible("image", None));
        assert!(rating_in_file_possible("image", Some("jpeg")));
        assert!(!rating_in_file_possible("image", Some("bmp")));
        assert!(!rating_in_file_possible("video", None));
    }

    #[test]
    fn the_next_job_comes_off_the_queue_index() {
        let c = library(PathRules::Windows);
        let plan = raw::query_plan(&c, NEXT_READY, [MAX_ATTEMPTS, 0]);
        assert!(plan.contains("file_writes_next") && !plan.contains("TEMP B-TREE"), "{plan}");
    }

    #[test]
    fn a_file_with_no_row_has_no_folder() {
        let c = library(PathRules::Windows);
        assert!(c.file_folder(FileId::new(1)).unwrap().is_some());
        assert!(c.file_folder(FileId::new(999)).unwrap().is_none());
    }
}
