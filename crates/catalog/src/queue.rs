//! The file writes queue (`file_writes`): what the user changed that the files themselves should carry, and what
//! the writer does with it. A rating or a turn is queued here in the same transaction that changes the catalog
//! (the producers: `set_rating`, `queue_rotation`). The writer takes the oldest settled job with `next_write`, records
//! the temp file it made with `record_write_temp` once the original is open for the copy (a recorded temp means the copy
//! into the original may have started), and ends the job with `write_done` (with what the write changed in the catalog, a
//! `WriteEffect`), `write_failed`, or `drop_gone_write`. A failure never discards a recorded temp: a copy that keeps failing
//! stays recorded, and `retry_write_copies` lets each start of the writer try it again. `drop_gone_write` is the one
//! exception: it removes the row of a write whose original is gone under a removal, temp included, and the temp file is
//! left in place for the writer to name to the user. Each call is one transaction or one statement, so no reader sees a
//! write both done and still queued.

use std::rc::Rc;

use crate::core::Catalog;
use crate::error::Result;
use rusqlite::types::{Type, Value};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use pg_model::format::Format;
use pg_model::ids::FileId;
use pg_model::media::Kind;
use pg_model::num::count_u64;
use pg_model::orientation::Orientation;
use pg_model::rating::Rating;
use pg_model::rules::Path as RulePath;
use pg_model::turns::Turns;
use pg_model::write::{Write, WriteKind};
use pg_model::{write_rating, write_rotation};

use crate::types::{DELETE_FILE_RATING, DELETE_RATING, SET_FILE_RATING, SET_RATING, bad_value};

mod job;
pub use job::{Job, Rewrite, WriteEffect};

/// Failed writes are retried at most this many times (offline drives retry without counting).
pub const MAX_ATTEMPTS: i64 = 5;

impl Catalog {
    /// Rate a file: 1–5, or `None` to clear. The catalog changes at once. With `write_back`, the rating is also queued
    /// for the file itself (`pg_writeback`) when it may hold one (`pg_model::format::rating_in_file_possible`: an image whose format can
    /// hold one, or that isn't analyzed yet, but not one whose analysis failed) and it differs from what the file holds;
    /// clearing writes 0 only into a file that holds a rating.
    pub fn set_rating(&self, file_id: FileId, rating: Option<Rating>, write_back: bool) -> Result<()> {
        let conn = &self.conn;
        let tx = crate::transaction::write_transaction(conn)?;
        let file_id = file_id.get();
        match rating {
            Some(rating) => tx.execute(SET_RATING, params![file_id, rating.get()])?,
            None => tx.execute(DELETE_RATING, [file_id])?,
        };
        // The file's kind, its analyzed format, and whether its analysis failed (a file the import couldn't read isn't
        // written into).
        let target: Option<(Kind, Option<Format>, bool)> = tx
            .query_row(
                "SELECT f.kind, a.format, a.error IS NOT NULL FROM files f LEFT JOIN analysis a ON a.file_id = f.id
                 WHERE f.id = ?1",
                [file_id],
                |r| Ok((crate::types::kind_at(r, 0)?, crate::types::format_at(r, 1)?, r.get(2)?)),
            )
            .optional()?;
        let can_hold = target
            .is_some_and(|(kind, format, failed)| !failed && pg_model::format::rating_in_file_possible(kind, format));
        // The file's rating as it holds it (no `file_ratings` row is unrated), and the rating it should hold. Unrated is
        // written as 0, the job's wire value.
        let in_file: Option<u8> =
            tx.query_row("SELECT value FROM file_ratings WHERE file_id = ?1", [file_id], |r| r.get(0)).optional()?;
        let wanted: Option<u8> = rating.map(Rating::get);
        if write_back && can_hold && wanted != in_file {
            tx.execute(
                concat!(
                    "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, '",
                    write_rating!(),
                    "', ?2, unixepoch())
                 ON CONFLICT(file_id, kind) DO UPDATE SET value = excluded.value, queued = excluded.queued,
                    attempts = 0, error = NULL"
                ),
                params![file_id, wanted.unwrap_or(0)],
            )?;
        } else {
            // Back to what the file holds (or not written back): nothing left to write. A write already being
            // copied in finishes, and then this value follows it; its attempts start again, so a copy that had failed
            // up to the cap is tried again for the change.
            tx.execute(
                concat!(
                    "DELETE FROM file_writes WHERE file_id = ?1 AND kind = '",
                    write_rating!(),
                    "' AND temp IS NULL"
                ),
                [file_id],
            )?;
            tx.execute(
                concat!(
                    "UPDATE file_writes SET value = ?2, attempts = 0 WHERE file_id = ?1 AND kind = '",
                    write_rating!(),
                    "'"
                ),
                params![file_id, wanted.unwrap_or(0)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Turn a file `turns` quarter turns clockwise: queued for the writer, which turns the file itself losslessly where
    /// it can and keeps the turn in the catalog otherwise. Turns add up until written. Returns the file's queued turns
    /// now, the total the catalog holds (what a reader sees as its pending turns), so a caller sets its picture of them
    /// to this rather than adding `turns` to it; and the file's `turns_queued` counter as this same transaction left it
    /// (`Entry::turns_queued`). The counter adds the quarter turns queued and no write lowers it, so a caller that
    /// counts its own presses against it counts each press once, whenever it reads.
    pub fn queue_rotation(&self, file_id: FileId, turns: Turns) -> Result<(Turns, u64)> {
        let conn = &self.conn;
        let tx = crate::transaction::write_transaction(conn)?;
        let file_id = file_id.get();
        // The sum is taken as a `Turns` here, not by SQL's modulus: the queued turns, with these added, and the write is
        // the one row of the file's queued turns.
        let queued: Option<i64> = tx
            .query_row(
                concat!("SELECT value FROM file_writes WHERE file_id = ?1 AND kind = '", write_rotation!(), "'"),
                [file_id],
                |r| r.get(0),
            )
            .optional()?;
        let total = Turns::of(queued.unwrap_or(0)).plus(turns);
        tx.execute(
            concat!(
                "INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, '",
                write_rotation!(),
                "', ?2, unixepoch())
             ON CONFLICT(file_id, kind) DO UPDATE SET value = excluded.value, queued = excluded.queued,
                attempts = 0, error = NULL"
            ),
            params![file_id, total.get()],
        )?;
        tx.execute(
            concat!(
                "DELETE FROM file_writes WHERE file_id = ?1 AND kind = '",
                write_rotation!(),
                "' AND value = 0 AND temp IS NULL"
            ),
            [file_id],
        )?;
        // The counter moves with the queue, in the same transaction: a read that sees the queued turns sees the count.
        tx.execute(
            "UPDATE files SET turns_queued = turns_queued + ?2 WHERE id = ?1",
            params![file_id, i64::from(turns.get())],
        )?;
        let turns_queued: u64 =
            tx.query_row("SELECT turns_queued FROM files WHERE id = ?1", [file_id], |r| r.get(0))?;
        tx.commit()?;
        Ok((total, turns_queued))
    }

    /// The oldest job the writer can try now: an interrupted copy first (its temp is complete), then the oldest change
    /// that has settled for `settle_secs`. A failed job waits `settle_secs` longer for each attempt so far.
    pub fn next_write(&self, settle_secs: i64) -> Result<Option<Job>> {
        self.next_write_skipping(settle_secs, &[])
    }

    /// [`Catalog::next_write`] without the files in `skip`: the writer's drain passes the files whose drive is away, so
    /// their writes wait for the drive's own recheck rather than come round at each settle time.
    pub fn next_write_skipping(&self, settle_secs: i64, skip: &[FileId]) -> Result<Option<Job>> {
        let conn = &self.conn;
        let skip = Rc::new(skip.iter().map(|id| Value::Integer(id.get())).collect::<Vec<_>>());
        let job = conn
            .query_row(NEXT_READY, params![MAX_ATTEMPTS, settle_secs, skip], |r| {
                let kind: String = r.get(1)?;
                let kind =
                    WriteKind::from_key(&kind).ok_or_else(|| bad_value(1, Type::Text, "file_writes.kind", &kind))?;
                // The value read with its kind: one out of the kind's range is a damaged row (the schema's CHECK).
                let write_at = |i: usize, column: &'static str| -> rusqlite::Result<Option<Write>> {
                    let Some(value) = r.get::<_, Option<i64>>(i)? else { return Ok(None) };
                    Write::from_stored(kind, value)
                        .map(Some)
                        .ok_or_else(|| bad_value(i, Type::Integer, column, &value.to_string()))
                };
                let temp: Option<Vec<u8>> = r.get(3)?;
                let temp_value = write_at(4, "file_writes.temp_value")?;
                // The temp's path is the rules' bytes (`record_temp`); one that doesn't parse is a damaged row.
                let temp = match temp {
                    Some(bytes) => Some(self.rules().parse_bytes(&bytes).ok_or_else(|| {
                        bad_value(3, Type::Blob, "file_writes.temp", &String::from_utf8_lossy(&bytes))
                    })?),
                    None => None,
                };
                Ok(Job {
                    file_id: FileId::new(r.get(0)?),
                    write: write_at(2, "file_writes.value")?
                        .ok_or_else(|| bad_value(2, Type::Null, "file_writes.value", "NULL"))?,
                    temp: temp.zip(temp_value),
                    attempts: r.get(5)?,
                })
            })
            .optional()?;
        Ok(job)
    }

    /// Whether a job is left that may still be tried (the writer waits for it to settle, rather than stopping).
    pub fn writes_attemptable(&self) -> Result<bool> {
        self.writes_attemptable_skipping(&[])
    }

    /// [`Catalog::writes_attemptable`] without the files in `skip` (see [`Catalog::next_write_skipping`]).
    pub fn writes_attemptable_skipping(&self, skip: &[FileId]) -> Result<bool> {
        let conn = &self.conn;
        let skip = Rc::new(skip.iter().map(|id| Value::Integer(id.get())).collect::<Vec<_>>());
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM file_writes WHERE attempts < ?1 AND file_id NOT IN rarray(?2))",
            params![MAX_ATTEMPTS, skip],
            |r| r.get(0),
        )?)
    }

    /// Every queued write, failing or not.
    pub fn writes_remaining(&self) -> Result<u64> {
        let conn = &self.conn;
        Ok(conn.query_row("SELECT count(*) FROM file_writes", [], |r| r.get(0))?)
    }

    /// The queued writes still to make now: those that have retries left, and aren't failing without a temp to finish.
    pub fn left_to_write(&self) -> Result<u64> {
        let conn = &self.conn;
        Ok(conn.query_row(
            "SELECT count(*) FROM file_writes WHERE attempts < ?1 AND (error IS NULL OR temp IS NOT NULL)",
            [MAX_ATTEMPTS],
            |r| r.get(0),
        )?)
    }

    /// The latest errors of the queue, for the command line: newest first. `error` is set only by [`Catalog::write_failed`], which
    /// stamps `queued` with the time of the failure, so `queued` orders them; the file and the kind break a tie within a
    /// second, so the rows shown under the limit are always the same.
    pub fn recent_write_errors(&self, limit: usize) -> Result<Vec<String>> {
        let conn = &self.conn;
        let mut stmt = conn.prepare(
            "SELECT error FROM file_writes WHERE error IS NOT NULL ORDER BY queued DESC, file_id DESC, kind DESC LIMIT ?1",
        )?;
        let errors = stmt
            .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(errors)
    }

    /// The new content of a job is complete and durable at `temp`, carrying `write`, and the original is open for the copy
    /// into it, which starts next: from here the original may be part-written. Until `done`, readers see the file as being
    /// written (`pg_model::state::FileState::writing`), and the writer finishes the copy from `temp` before anything else.
    ///
    /// `false` when the job's row is no longer there (it was removed after the writer read it, so nothing is recorded): the
    /// copy must not start then. The caller leaves the original as it is.
    pub fn record_write_temp(&self, job: &Job, temp: &RulePath, write: Write) -> Result<bool> {
        let conn = &self.conn;
        let bytes = self.rules().join(temp);
        // Not for a file hidden since the job was picked (a removal is under way): its temp would then be the only copy
        // of the edit with nothing that finishes it.
        let recorded = conn.execute(
            "UPDATE file_writes SET temp = ?3, temp_value = ?4 WHERE file_id = ?1 AND kind = ?2 AND \
             EXISTS (SELECT 1 FROM files WHERE id = ?1 AND missing = 0)",
            params![job.file_id.get(), job.kind().key(), bytes, write.stored()],
        )?;
        Ok(recorded == 1)
    }

    /// The job is written: its value (a rating, or the turns) is now in the file or the catalog. `effect` changes the
    /// catalog in the same transaction that takes the job off the queue (a rating or turns changed again meanwhile stays
    /// queued for the rest).
    pub fn write_done(&self, job: &Job, effect: WriteEffect) -> Result<()> {
        let conn = &self.conn;
        let (file_id, kind) = (job.file_id.get(), job.kind().key());
        // What the queue records as written: the job's rating, or its turns.
        let written = job.write.stored();
        let tx = crate::transaction::write_transaction(conn)?;
        match effect {
            WriteEffect::FileRating { rewrite, rating } => {
                file_rewritten(&tx, job.file_id, rewrite)?;
                set_file_rating(&tx, job.file_id, rating)?;
            }
            WriteEffect::TurnedBack { rewrite, turns, exif } => {
                file_rewritten(&tx, job.file_id, rewrite)?;
                analysis_turned_back(&tx, job.file_id, turns, exif)?;
            }
            WriteEffect::KeptInCatalog(turns) => add_catalog_rotation(&tx, job.file_id, turns)?,
            WriteEffect::Nothing => {}
        }
        match job.write {
            Write::Rotation(turns) => {
                // The queued turns less the ones written, as a `Turns` (what is left is the turns still to write).
                let queued: Option<i64> = tx
                    .query_row(
                        concat!(
                            "SELECT value FROM file_writes WHERE file_id = ?1 AND kind = '",
                            write_rotation!(),
                            "'"
                        ),
                        [file_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(queued) = queued {
                    tx.execute(
                        concat!(
                            "UPDATE file_writes SET value = ?2 WHERE file_id = ?1 AND kind = '",
                            write_rotation!(),
                            "'"
                        ),
                        params![file_id, Turns::of(queued - written).get()],
                    )?;
                }
                tx.execute(
                    concat!(
                        "DELETE FROM file_writes WHERE file_id = ?1 AND kind = '",
                        write_rotation!(),
                        "' AND value = 0"
                    ),
                    [file_id],
                )?;
                // The upright size at once: the grid lays the file out turned before its analysis is redone.
                if turns.swaps_axes() {
                    tx.execute("UPDATE analysis SET width = height, height = width WHERE file_id = ?1", [file_id])?;
                }
            }
            Write::Rating(_) => {
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
    pub fn write_failed(&self, job: &Job, message: &str, offline: bool) -> Result<()> {
        let conn = &self.conn;
        conn.execute(
            "UPDATE file_writes SET attempts = attempts + ?3, error = ?4, queued = unixepoch()
             WHERE file_id = ?1 AND kind = ?2",
            params![job.file_id.get(), job.kind().key(), if offline { 0 } else { 1 }, message],
        )?;
        Ok(())
    }

    /// The write is dropped with its row, which holds the temp's path when one is recorded: the caller leaves the temp file
    /// in place (see the module doc). Used for a file that is gone, or whose original is gone under a removal.
    pub fn drop_gone_write(&self, job: &Job) -> Result<()> {
        let conn = &self.conn;
        conn.execute(
            "DELETE FROM file_writes WHERE file_id = ?1 AND kind = ?2",
            params![job.file_id.get(), job.kind().key()],
        )?;
        Ok(())
    }

    /// The copies into the originals that failed up to the cap get their attempts back: the writer calls it when it starts,
    /// so each start tries a stuck copy again. Its temp is the only whole copy of the new content while the original may be
    /// part-written, so it is never given up. The number of rows reset.
    pub fn retry_write_copies(&self) -> Result<u64> {
        let conn = &self.conn;
        Ok(count_u64(conn.execute("UPDATE file_writes SET attempts = 0 WHERE temp IS NOT NULL AND attempts > 0", [])?))
    }
}

/// [`Catalog::next_write`]'s statement. Its order is the queue index's (`file_writes_next`): no sort of the queue.
const NEXT_READY: &str = "SELECT file_id, kind, value, temp, temp_value, attempts FROM file_writes
     WHERE attempts < ?1 AND queued <= unixepoch() - ?2 * (attempts + 1) AND file_id NOT IN rarray(?3)
     ORDER BY temp IS NULL, queued LIMIT 1";

/// Record what the file holds now (the rating it was last written with, or the import's reads).
pub(crate) fn set_file_rating(conn: &Connection, file_id: FileId, value: Option<Rating>) -> Result<()> {
    match value {
        Some(v) => conn.prepare_cached(SET_FILE_RATING)?.execute(params![file_id.get(), v.get()])?,
        None => conn.prepare_cached(DELETE_FILE_RATING)?.execute([file_id.get()])?,
    };
    Ok(())
}

/// The file was rewritten in place: record its new size and mtime, so the scanner and the import see it as the file
/// it is. `before` is the (size, mtime) the file had on disk just before the write, when it is known. After a
/// metadata-only change (`metadata_only` = the new BLAKE3) an analysis made from exactly those bytes stays valid and
/// follows the file: nothing is re-imported. After a pixel change, or from a file whose state before isn't known, it is
/// left stale, so the import redoes it (new upright size, hashes, PDQ).
fn file_rewritten(tx: &Transaction, file_id: FileId, rewrite: Rewrite) -> Result<()> {
    let Rewrite { before, after, metadata_only } = rewrite;
    tx.execute("UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1", params![file_id.get(), after.0, after.1])?;
    if let (Some(file_hash), Some(before)) = (metadata_only, before) {
        // An analysis stored without its duplicate hashes (`hashes_owed`) follows the file's size and mtime, but takes no
        // hash here: a file hash alone would make it look hashed while its pixel hashes are missing. The Hashes pass makes
        // all of them, from the file as it now is.
        tx.execute(
            "UPDATE analysis SET size = ?2, mtime = ?3,
                file_hash = CASE WHEN EXISTS (SELECT 1 FROM hashes_owed WHERE file_id = ?1) THEN file_hash ELSE ?4 END
             WHERE file_id = ?1 AND size = ?5 AND mtime = ?6",
            params![file_id.get(), after.0, after.1, &file_hash.0[..], before.0, before.1],
        )?;
        // The placeholder of the same pixels follows the file too: a rating written into the file is not a reason to
        // make it again (its identity is the file's size and mtime, `placeholders`).
        tx.execute(
            "UPDATE placeholders SET size = ?2, mtime = ?3 WHERE file_id = ?1 AND size = ?4 AND mtime = ?5",
            params![file_id.get(), after.0, after.1, before.0, before.1],
        )?;
    }
    Ok(())
}

/// A turn written into the file (its pixels or its orientation tag): the analysis, and the placeholder, made before are of
/// the file as it is now turned back by them, so they are recorded as made at the catalog rotation that many turns back.
/// Both are stale anyway (the mtime changed): until the import redoes the analysis and the Placeholders pass redoes the
/// placeholder (the identity no longer matches), the placeholder shows the file as it is now.
fn analysis_turned_back(tx: &Transaction, file_id: FileId, turns: Turns, exif: Orientation) -> Result<()> {
    let back = |stored: Turns| Turns::of(i64::from(stored.get()) - i64::from(turns.get()));
    change_turns(tx, ("analysis", "file_id", "rotation"), file_id.get(), back)?;
    // The file's tag is what the write left in it: exact, where the rest of the analysis is only stale.
    tx.execute("UPDATE analysis SET exif_orientation = ?2 WHERE file_id = ?1", (file_id.get(), exif.get()))?;
    change_turns(tx, ("placeholders", "file_id", "rotation"), file_id.get(), back)
}

/// The turn is kept in the catalog (the file can't hold it): the analysis, made without it, is stale (`files.pending`),
/// and the thumbnails no longer match (thumbs::Source).
fn add_catalog_rotation(tx: &Transaction, file_id: FileId, turns: Turns) -> Result<()> {
    change_turns(tx, ("files", "id", "turns"), file_id.get(), |stored| stored.plus(turns))
}

/// Sets the quarter turns of the row `key` = `id` of `table` (its `column`, a `Turns`) to `change` of what it holds, when the
/// row is there. The arithmetic is `Turns`'s, so the modulus is in one place and not in SQL.
fn change_turns(
    tx: &Transaction,
    (table, key, column): (&'static str, &'static str, &'static str),
    id: i64,
    change: impl FnOnce(Turns) -> Turns,
) -> Result<()> {
    let stored: Option<i64> =
        tx.query_row(&format!("SELECT {column} FROM {table} WHERE {key} = ?1"), [id], |r| r.get(0)).optional()?;
    if let Some(stored) = stored {
        let changed = change(Turns::of(stored));
        tx.execute(&format!("UPDATE {table} SET {column} = ?2 WHERE {key} = ?1"), params![id, changed.get()])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::{library, under_both_rules};
    use crate::fixture::raw;
    use pg_model::rules::PathRules;

    under_both_rules!(
        ratings_and_turns_queue_for_the_file,
        ratings_and_turns_queue_for_the_file_windows,
        ratings_and_turns_queue_for_the_file_posix
    );

    fn ratings_and_turns_queue_for_the_file(rules: PathRules) {
        let c = library(rules);
        c.set_rating(FileId::new(2), Rating::new(3), true).unwrap();
        assert_eq!(raw::queued(&c, 2, "rating"), Some(3));
        // Back to what the file holds (nothing): nothing to write.
        c.set_rating(FileId::new(2), None, true).unwrap();
        assert_eq!(raw::queued(&c, 2, "rating"), None);
        // A file that holds 4: clearing writes 0.
        raw::holds_rating(&c, FileId::new(1), Rating::new(4));
        c.set_rating(FileId::new(1), None, true).unwrap();
        assert_eq!(raw::queued(&c, 1, "rating"), Some(0));
        // BMP can't hold one; write-back off writes nothing.
        c.set_rating(FileId::new(4), Rating::new(5), true).unwrap();
        c.set_rating(FileId::new(3), Rating::new(5), false).unwrap();
        assert_eq!((raw::queued(&c, 4, "rating"), raw::queued(&c, 3, "rating")), (None, None));
        // Turns add up modulo 4; a full turn leaves nothing to do.
        c.queue_rotation(FileId::new(1), Turns::new(1)).unwrap();
        c.queue_rotation(FileId::new(1), Turns::new(1)).unwrap();
        assert_eq!(raw::queued(&c, 1, "rotation"), Some(2));
        c.queue_rotation(FileId::new(1), Turns::new(2)).unwrap();
        assert_eq!(raw::queued(&c, 1, "rotation"), None);
    }

    /// The counter adds the quarter turns each press queues (a counterclockwise press is three), and the write of them
    /// leaves it where it is.
    #[test]
    fn the_turns_queued_counter_adds_quarter_turns_and_no_write_lowers_it() {
        let c = library(PathRules::Windows);
        assert_eq!(c.queue_rotation(FileId::new(1), Turns::new(1)).unwrap(), (Turns::new(1), 1));
        assert_eq!(c.queue_rotation(FileId::new(1), Turns::new(3)).unwrap(), (Turns::new(0), 4));
        assert_eq!(c.queue_rotation(FileId::new(1), Turns::new(2)).unwrap(), (Turns::new(2), 6));
        let job = c.next_write(0).unwrap().unwrap();
        c.write_done(&job, WriteEffect::KeptInCatalog(Turns::new(2))).unwrap();
        assert_eq!(c.grid_entry(FileId::new(1)).unwrap().unwrap().turns_queued, 6);
    }

    #[test]
    fn a_job_is_taken_once_it_settles_and_done_applies_its_effect() {
        let c = library(PathRules::Windows);
        c.set_rating(FileId::new(1), Rating::new(3), true).unwrap();
        c.queue_rotation(FileId::new(2), Turns::new(1)).unwrap();
        // Both are new: nothing is ready until they settle (a settle time of 0 takes them now).
        let first = c.next_write(0).unwrap().unwrap();
        assert_eq!((first.file_id, first.write), (FileId::new(1), Write::Rating(Rating::new(3))));
        // The rating is in the file: the catalog records it, and the job leaves the queue.
        let after = (11, 101);
        let rewrite =
            Rewrite { before: Some((10, 1)), after, metadata_only: Some(pg_model::analysis::FileHash([7; 32])) };
        c.write_done(&first, WriteEffect::FileRating { rewrite, rating: Rating::new(3) }).unwrap();
        assert_eq!(raw::queued(&c, 1, "rating"), None);
        assert_eq!(raw::file_size(&c, FileId::new(1)), 11);
        // The turn can't go in the file: the catalog keeps it, and the upright size swaps at once.
        let turn = c.next_write(0).unwrap().unwrap();
        assert_eq!((turn.file_id, turn.write), (FileId::new(2), Write::Rotation(Turns::new(1))));
        c.write_done(&turn, WriteEffect::KeptInCatalog(Turns::new(1))).unwrap();
        assert_eq!(raw::queued(&c, 2, "rotation"), None);
        assert_eq!(raw::rotation_and_width(&c, FileId::new(2)), (1, 3));
        assert!(c.next_write(0).unwrap().is_none());
    }

    /// A file rewritten in place whose analysis is owed its duplicate hashes (`hashes_owed`) takes its new size and mtime,
    /// and no file hash: a hash alone would make the analysis look hashed while its pixel hashes are missing.
    #[test]
    fn a_rewrite_of_an_unhashed_file_follows_its_size_and_takes_no_hash() {
        let c = library(PathRules::Windows);
        // The analysis was made from the file as it was before the write: its size and mtime are the rewrite's `before`.
        c.conn
            .execute(
                "UPDATE analysis SET file_hash = NULL, pixel_hash = NULL, size = 10, mtime = 1 WHERE file_id = 1",
                [],
            )
            .unwrap();
        c.conn.execute("INSERT INTO hashes_owed(file_id) VALUES (1)", []).unwrap();
        c.set_rating(FileId::new(1), Rating::new(3), true).unwrap();
        let job = c.next_write(0).unwrap().unwrap();
        let after = (11, 101);
        let rewrite =
            Rewrite { before: Some((10, 1)), after, metadata_only: Some(pg_model::analysis::FileHash([7; 32])) };
        c.write_done(&job, WriteEffect::FileRating { rewrite, rating: Rating::new(3) }).unwrap();
        assert_eq!(raw::file_size(&c, FileId::new(1)), 11);
        let (size, file_hash): (i64, Option<Vec<u8>>) = c
            .conn
            .query_row("SELECT size, file_hash FROM analysis WHERE file_id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((size, file_hash), (11, None));
        // The file is not pending: a guarded size and mtime would leave the analysis stale, and every rating write would
        // then force a full re-import.
        let pending: i64 = c.conn.query_row("SELECT pending FROM files WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(pending, 0);
        assert_eq!(c.owed_hashes().unwrap(), 1, "still owed: the Hashes pass makes them from the file as it is");
    }

    #[test]
    fn a_failure_waits_longer_and_an_offline_drive_does_not_count() {
        let c = library(PathRules::Windows);
        c.set_rating(FileId::new(1), Rating::new(3), true).unwrap();
        let job = c.next_write(0).unwrap().unwrap();
        c.write_failed(&job, "denied", false).unwrap();
        // One attempt in: due two settle times later, not now.
        assert!(c.next_write(1).unwrap().is_none());
        assert!(c.writes_attemptable().unwrap());
        let job = c.next_write(0).unwrap().unwrap();
        assert_eq!(job.attempts, 1);
        c.write_failed(&job, "offline", true).unwrap();
        let job = c.next_write(0).unwrap().unwrap();
        assert_eq!(job.attempts, 1, "an offline failure is not an attempt");
        assert_eq!(c.recent_write_errors(5).unwrap(), ["offline"]);
        c.drop_gone_write(&job).unwrap();
        assert_eq!(c.writes_remaining().unwrap(), 0);
    }

    /// The files in the skip set are not offered, the others are; with every queued file skipped there is no job and
    /// nothing to wait for. The drain relies on this to leave an away drive's writes for its recheck.
    #[test]
    fn a_skipped_file_is_not_offered_and_the_others_are() {
        let c = library(PathRules::Windows);
        c.set_rating(FileId::new(1), Rating::new(3), true).unwrap();
        c.set_rating(FileId::new(2), Rating::new(3), true).unwrap();
        let one = [FileId::new(1)];
        let both = [FileId::new(1), FileId::new(2)];
        assert_eq!(c.next_write_skipping(0, &one).unwrap().unwrap().file_id, FileId::new(2));
        assert!(c.next_write_skipping(0, &both).unwrap().is_none());
        assert!(c.writes_attemptable_skipping(&one).unwrap());
        assert!(!c.writes_attemptable_skipping(&both).unwrap());
        assert!(c.writes_attemptable().unwrap(), "the unskipped entry point still sees both");
    }

    /// The latest errors come first, a tie within a second broken by the file and the kind, and the limit keeps the
    /// newest.
    #[test]
    fn recent_errors_are_the_newest_in_a_fixed_order() {
        let c = library(PathRules::Windows);
        c.set_rating(FileId::new(1), Rating::new(3), true).unwrap();
        c.set_rating(FileId::new(2), Rating::new(3), true).unwrap();
        c.queue_rotation(FileId::new(1), Turns::new(1)).unwrap();
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
        assert_eq!(c.recent_write_errors(5).unwrap(), ["tied, higher file", "tied, lower file", "oldest"]);
        assert_eq!(c.recent_write_errors(2).unwrap(), ["tied, higher file", "tied, lower file"]);
        // A tie on the file too: the kind decides.
        fail(1, "rating", 200, "tied, rating");
        assert_eq!(c.recent_write_errors(3).unwrap(), ["tied, higher file", "tied, lower file", "tied, rating"]);
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
        c.queue_rotation(FileId::new(1), Turns::new(3)).unwrap();
        let job = c.next_write(0).unwrap().unwrap();
        let temp = PathRules::Windows.parse(r"D:\p\.b.jpg.rotation.pg-write").unwrap();
        let three = Write::Rotation(Turns::new(3));
        c.record_write_temp(&job, &temp, three).unwrap();
        c.queue_rotation(FileId::new(1), Turns::new(1)).unwrap();
        assert_eq!(raw::queued(&c, 1, "rotation"), Some(0), "kept while its temp is recorded");
        let job = c.next_write(0).unwrap().unwrap();
        assert_eq!(job.temp, Some((temp.clone(), three)));
        c.write_failed(&job, "locked", false).unwrap();
        let job = c.next_write(0).unwrap().unwrap();
        let rewrite = Rewrite { before: None, after: (31, 7), metadata_only: None };
        c.write_done(
            &Job { write: three, ..job.clone() },
            WriteEffect::TurnedBack { rewrite, turns: Turns::new(3), exif: Orientation::new(6).unwrap() },
        )
        .unwrap();
        // Three turns are in the file: one is left to make the full circle. The file's tag is what the write left in it.
        assert_eq!(raw::queued(&c, 1, "rotation"), Some(1));
        let tag = raw::rows_of(&c, "SELECT exif_orientation FROM analysis WHERE file_id = 1", [], |r| {
            r.get::<_, Option<i64>>(0)
        });
        assert_eq!(tag, [Some(6)], "the analysis holds the tag the write left in the file");
        assert_eq!(attempts(&c, 1, "rotation"), 0);
        let rest = c.next_write(0).unwrap().unwrap();
        assert_eq!(rest.temp, None);
        c.drop_gone_write(&rest).unwrap();

        // A rating of 3 recorded in a temp, then 5: the copy of 3 is done, and 5 stays queued.
        c.set_rating(FileId::new(2), Rating::new(3), true).unwrap();
        let job = c.next_write(0).unwrap().unwrap();
        assert_eq!((job.file_id, job.kind()), (FileId::new(2), WriteKind::Rating));
        let temp = PathRules::Windows.parse(r"E:\p\.a.png.rating.pg-write").unwrap();
        let (three, five) = (Rating::new(3).unwrap(), Rating::new(5).unwrap());
        c.record_write_temp(&job, &temp, Write::Rating(Some(three))).unwrap();
        c.set_rating(FileId::new(2), Some(five), true).unwrap();
        let job = c.next_write(0).unwrap().unwrap();
        assert_eq!(
            (job.file_id, job.write, job.temp.as_ref().map(|t| t.1)),
            (FileId::new(2), Write::Rating(Some(five)), Some(Write::Rating(Some(three))))
        );
        let rewrite = Rewrite { before: None, after: (11, 9), metadata_only: None };
        c.write_done(
            &Job { write: Write::Rating(Some(three)), ..job.clone() },
            WriteEffect::FileRating { rewrite, rating: Some(three) },
        )
        .unwrap();
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
        c.set_rating(FileId::new(2), Rating::new(3), true).unwrap();
        let job = c.next_write(0).unwrap().unwrap();
        let temp = PathRules::Windows.parse(r"E:\p\.a.png.rating.pg-write").unwrap();
        let three = Write::Rating(Rating::new(3));
        c.record_write_temp(&job, &temp, three).unwrap();
        let stuck = |c: &Catalog| {
            for _ in 0..MAX_ATTEMPTS {
                c.write_failed(&job, "locked", false).unwrap();
            }
            assert_eq!(attempts(c, 2, "rating"), MAX_ATTEMPTS);
            assert!(c.next_write(0).unwrap().is_none(), "at the cap");
        };
        stuck(&c);
        // Back to what the file holds (none): the row stays for its copy, with its attempts back.
        c.set_rating(FileId::new(2), None, true).unwrap();
        assert_eq!(attempts(&c, 2, "rating"), 0);
        assert_eq!(c.next_write(0).unwrap().unwrap().temp, Some((temp, three)));
        stuck(&c);
        assert_eq!(c.retry_write_copies().unwrap(), 1);
        assert_eq!(attempts(&c, 2, "rating"), 0);
        // A failing write without a temp is not a copy: it keeps its attempts.
        c.queue_rotation(FileId::new(1), Turns::new(1)).unwrap();
        let turn = Job { file_id: FileId::new(1), write: Write::Rotation(Turns::new(1)), temp: None, attempts: 0 };
        c.write_failed(&turn, "locked", false).unwrap();
        assert_eq!(c.retry_write_copies().unwrap(), 0);
        assert_eq!(attempts(&c, 1, "rotation"), 1);
    }

    /// A rating is queued for an image whose format can hold one or isn't known yet (not analyzed), and not for a
    /// video, a format that can't hold one, or a file whose analysis failed.
    #[test]
    fn a_rating_is_queued_for_the_files_that_may_hold_one() {
        let c = crate::browse::fixtures::mixed_library(PathRules::Windows);
        // 5: a video; 7: its analysis failed; 8: not analyzed yet; 4: a BMP.
        for id in [4, 5, 7, 8] {
            c.set_rating(FileId::new(id), Rating::new(4), true).unwrap();
        }
        let queued: Vec<Option<i64>> = [4, 5, 7, 8].iter().map(|&id| raw::queued(&c, id, "rating")).collect();
        assert_eq!(queued, [None, None, None, Some(4)]);
        assert!(pg_model::format::rating_in_file_possible(Kind::Image, None));
        assert!(pg_model::format::rating_in_file_possible(Kind::Image, Some(Format::Jpeg)));
        assert!(!pg_model::format::rating_in_file_possible(Kind::Image, Some(Format::Bmp)));
        assert!(!pg_model::format::rating_in_file_possible(Kind::Video, None));
    }

    #[test]
    fn the_next_job_comes_off_the_queue_index() {
        let c = library(PathRules::Windows);
        let plan = raw::query_plan(&c, NEXT_READY, params![MAX_ATTEMPTS, 0, Rc::new(Vec::<Value>::new())]);
        assert!(plan.contains("file_writes_next") && !plan.contains("TEMP B-TREE"), "{plan}");
    }

    #[test]
    fn a_file_with_no_row_has_no_folder() {
        let c = library(PathRules::Windows);
        assert!(c.file_folder(FileId::new(1)).unwrap().is_some());
        assert!(c.file_folder(FileId::new(999)).unwrap().is_none());
    }
}
