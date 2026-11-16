//! The writes of the scan, of the import (analyses, failures and the ratings they carry), of the borders pass and of
//! the passes' failure ledger. Each writer borrows the transaction it is made in, so it is dropped before that commits.
//! The ledger's set-based writes are functions over a connection ([`record_failures_on`], [`clear_failures_on`]), which
//! `Catalog::record_failures`/`clear_failures` run in a transaction of their own and the [`LedgerWriter`] in its
//! caller's. Uses `types`, `ledger::failure` and `error`; its SQL macros are from `sql`.

use anyhow::{Context, anyhow};
use std::rc::Rc;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Statement, Transaction, params};

use pg_model::analysis::Analysis;
use pg_model::detection::Detection;
use pg_model::ids::{FileId, FileIdentity, FolderId, ScanId};
use pg_model::notice::FilePass;
use pg_model::rating::Rating;
use pg_model::rules::{Path as RulePath, PathRules};

use crate::error::{Error, Result};
use crate::ledger::failure::{Failure, tidy};
use crate::types::{
    DELETE_FILE_RATING, DELETE_RATING, DELETE_SIDES, FOLDER_FAILED, Folder, FolderFailure, Hit, Recorded,
    SET_FILE_RATING, SET_RATING, ScannedFile, WorkItem, covering_root, under_params,
};
use crate::work::PairingWork;

// The passes' writes: one writer per transaction, each statement prepared once for it rather than looked up in
// rusqlite's statement cache per row (16 entries, which the import's statements alone would crowd). A writer
// borrows its transaction, so it is dropped before the transaction commits.

/// Write a file as a new row (or, on its identity, the same file's changed fields, while the row is as the walk read it
/// when it listed the folder: `f.prior`): unpaired, as `ScanWriter::file` says. The rows written: 0 when the row
/// changed since the listing.
fn write_file(file: &mut Statement<'_>, f: &ScannedFile, folder: &Folder, scan: i64, key: &[u8]) -> Result<usize> {
    let (volume, file_ref) = (f.identity.volume.cast_signed(), f.identity.file_index.cast_signed());
    let size = i64::try_from(f.size).context("a file's size")?;
    let (ext, kind) = (f.ext.name(), f.ext.kind().key());
    Ok(file.execute(params![
        volume,
        file_ref,
        f.created,
        folder.id.get(),
        f.name.as_bytes(),
        ext,
        kind,
        size,
        f.mtime,
        scan,
        key,
        f.prior.map(|p| p.size),
        f.prior.map(|p| p.mtime)
    ])?)
}

/// The scan's writes.
pub struct ScanWriter<'tx> {
    scan: i64,
    rules: PathRules,
    /// The scanned root's subtree (`under_params`): a row's folder is in it when it is `?1` or between `?2` and `?3`.
    under: [Vec<u8>; 3],
    /// The library folder that holds the walked one (its id and its stored path), resolved once when the writer is
    /// made; `None` when no library folder holds it (the folder ledger then records nothing).
    covering: Option<(i64, Vec<u8>)>,
    folder_id: Statement<'tx>,
    new_folder: Statement<'tx>,
    known: Statement<'tx>,
    in_root: Statement<'tx>,
    seen: Statement<'tx>,
    seen_identity: Statement<'tx>,
    backfill: Statement<'tx>,
    displace: Statement<'tx>,
    file: Statement<'tx>,
    folder_failed: Statement<'tx>,
    /// The order key of the file being recorded.
    key: Vec<u8>,
}

impl<'tx> ScanWriter<'tx> {
    /// The writer of the walk `scan` of `root`, in `tx`: the walk's lease is renewed (the heartbeat of each batch), and
    /// must still be the walk's. One that is gone (the folder removed from the library, which took it, or expired) is
    /// [`crate::Error::RootRemoved`] or [`crate::Error::LeaseLost`], with nothing written: the batch would bring a
    /// removed folder's files back as present, or interleave with another walk's.
    pub(crate) fn new(
        tx: &'tx Transaction,
        scan: ScanId,
        rules: PathRules,
        root: &RulePath,
    ) -> Result<ScanWriter<'tx>> {
        crate::core::HeldLease { scan, rules, root }.hold(tx)?;
        crate::lifecycle::stamp_held_exts(tx)?;
        let covering = covering_root(tx, rules, root)?.map(|r| (r.id.get(), rules.join(&r.path)));
        Ok(ScanWriter {
            scan: scan.get(),
            rules,
            under: under_params(rules, root),
            covering,
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
            // A new row is unpaired: the pairing at the end of the scan decides what it is. A row
            // is updated only while it is as the walk read it when it listed the folder (`?12`, `?13`: NULL for none),
            // and no write of the app's into it is in progress: `current::IS_CURRENT`'s size and mtime rule (a listing
            // has no rotation). The values are compared, not a stamp: `files.seq` moves with imports and retries,
            // which change nothing on disk.
            file: tx.prepare(concat!(
                "INSERT INTO files(volume, file_ref, created, folder_id, name, ext, kind, size, mtime, last_scan,
                    order_key, unpaired)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1)
                 ON CONFLICT(volume, file_ref) DO UPDATE SET
                    folder_id = excluded.folder_id, name = excluded.name, ext = excluded.ext, kind = excluded.kind,
                    size = excluded.size, mtime = excluded.mtime, last_scan = excluded.last_scan, missing = 0,
                    order_key = excluded.order_key, created = coalesce(files.created, excluded.created)
                 WHERE files.size IS ?12 AND files.mtime IS ?13 AND ",
                not_being_written!(files)
            ))?,
            folder_failed: tx.prepare(FOLDER_FAILED)?,
            key: Vec::new(),
        })
    }

    /// Record a folder the walk couldn't take (`failure`: unreadable, or a mount it refuses) with its error, in the
    /// folder ledger, as seen by this walk: the end of a complete walk of a folder above it sweeps the rows this walk
    /// didn't see (`ScanLease::mark_missing`). One already recorded keeps its serial, so it is not news again. Nothing is
    /// written when no library folder holds the walked one, or it was removed since the writer was made. Whether a row
    /// was written.
    pub fn folder_failed(&mut self, path: &RulePath, failure: FolderFailure, error: &str) -> Result<bool> {
        let Some((root, root_bytes)) = &self.covering else { return Ok(false) };
        let written = self.folder_failed.execute(params![
            root,
            root_bytes,
            self.rules.join(path),
            failure.code(),
            tidy(error),
            self.scan
        ])?;
        Ok(written > 0)
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

    /// Record a file the scan found, by identity. A file moved or renamed keeps its row, and so
    /// does one whose `created` is unknown on either side; a missing row of another root counts as moved here;
    /// a present one is [`Recorded::Foreign`]. A hit whose `created` differs is a different file: the row
    /// is displaced (it keeps its id, folder, name, ratings and content, and loses its identity), and a new row is
    /// made for the identity. A new row is `unpaired` until the pairing at the end of the scan. A row that changed
    /// since the folder was listed (`f.prior`), or that the app is writing into, is only marked seen
    /// ([`Recorded::Same`]): the listing is older than the row.
    pub fn upsert_file(&mut self, folder: &Folder, f: &ScannedFile) -> Result<Recorded> {
        let ScanWriter { scan, under, known, in_root, seen, backfill, displace, file, key, .. } = self;
        key.clear();
        key.extend_from_slice(&folder.key);
        f.name.push_name_key(key);
        let (volume, file_ref) = (f.identity.volume.cast_signed(), f.identity.file_index.cast_signed());
        let (ext, kind, size) = (f.ext.name(), f.ext.kind().key(), i64::try_from(f.size).context("a file's size")?);
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
                Ok(Hit {
                    id: r.get(0)?,
                    folder_id: r.get(1)?,
                    same,
                    created: r.get(9)?,
                    missing: r.get::<_, i64>(7)? != 0,
                })
            })
            .optional()?;
        let Some(hit) = hit else {
            write_file(file, f, folder, *scan, &key[..])?;
            return Ok(Recorded::Changed);
        };
        // A present row of another root is that root's (a second path to its file). A missing one does not keep the
        // identity from this root: its file is here now, so the row is this root's hit, moved here with its data as a
        // move within the root would be (or displaced, if the creation times say it is another file).
        if !hit.missing
            && !in_root.query_row(params![under[0], under[1], under[2], hit.folder_id], |r| r.get::<_, bool>(0))?
        {
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
        if write_file(file, f, folder, *scan, &key[..])? == 0 {
            seen.execute(params![hit.id, *scan])?;
            return Ok(Recorded::Same);
        }
        Ok(Recorded::Changed)
    }

    /// Mark the row with this identity as seen by this scan, changing nothing else. A reparse point is recorded this
    /// way: its listing gives no content to analyze, and a row that is there is kept.
    pub fn seen_identity(&mut self, identity: FileIdentity) -> Result<()> {
        self.seen_identity.execute(params![
            identity.volume.cast_signed(),
            identity.file_index.cast_signed(),
            self.scan
        ])?;
        Ok(())
    }
}

/// The import's writes: analyses, failures, and the file ratings they carry. The pairs are the pairing pass's
/// ([`PairsWriter`]); an analysis that owes them has a `pairs_owed` row.
pub struct AnalysisWriter<'tx> {
    /// The build's IMPORT_VERSION, which a stored failure is made against.
    import: i64,
    current: Statement<'tx>,
    delete_pairs: Statement<'tx>,
    analysis: Statement<'tx>,
    failure: Statement<'tx>,
    borders_due: Statement<'tx>,
    delete_sides: Statement<'tx>,
    owe_pairs: Statement<'tx>,
    known_rating: Statement<'tx>,
    queued_rating: Statement<'tx>,
    set_rating: Statement<'tx>,
    delete_rating: Statement<'tx>,
    set_file_rating: Statement<'tx>,
    delete_file_rating: Statement<'tx>,
}

impl<'tx> AnalysisWriter<'tx> {
    pub(crate) fn new(tx: &'tx Transaction, import: i64) -> Result<AnalysisWriter<'tx>> {
        Ok(AnalysisWriter {
            import,
            current: tx.prepare(crate::current::IS_CURRENT)?,
            delete_pairs: tx.prepare("DELETE FROM pdq_pairs WHERE a = ?1 OR b = ?1")?,
            analysis: tx.prepare(
                "INSERT OR REPLACE INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error,
                    file_hash, pixel_hash, format, orientation, width, height, pdq_dihedral, pdq_quality, thumbhash,
                    frames, taken, taken_utc, duration_ms, poster_ms, borders_due)
                 VALUES (?1, ?2, ?3, ?4, ?19, ?20, unixepoch(), NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                         CASE WHEN ?16 THEN unixepoch(?15, 'unixepoch', 'localtime') ELSE ?15 END,
                         CASE WHEN ?16 THEN ?15 END, ?17, ?18, 0)",
            )?,
            owe_pairs: tx.prepare("INSERT OR IGNORE INTO pairs_owed(file_id) VALUES (?1)")?,
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
            queued_rating: tx.prepare(concat!(
                "SELECT EXISTS(SELECT 1 FROM file_writes WHERE file_id = ?1 AND kind = '",
                pg_model::write_rating!(),
                "')"
            ))?,
            set_rating: tx.prepare(SET_RATING)?,
            delete_rating: tx.prepare(DELETE_RATING)?,
            set_file_rating: tx.prepare(SET_FILE_RATING)?,
            delete_file_rating: tx.prepare(DELETE_FILE_RATING)?,
        })
    }

    /// Whether the file is still as it was listed (`item`), with no write of the app's into it in progress: only
    /// then is what was read from it stored. Otherwise it is left as it is, due again (a changed file is stale; a
    /// file whose metadata was rewritten keeps the analysis made before, which the writer carried over).
    fn current(&mut self, item: &WorkItem) -> Result<bool> {
        crate::current::query_current(&mut self.current, item.id, &item.listed())
    }

    /// Store an analysis, unless the file changed since it was listed (see `current`). Whether it was stored.
    pub fn store_analysis(&mut self, item: &WorkItem, analyzed: &Analysis) -> Result<bool> {
        if !self.current(item)? {
            return Ok(false);
        }
        let a = analyzed;
        // A re-analysis invalidates the file's tier-1 pairs: it owes new ones (`pairs_owed`), which the pairing pass
        // makes. The REPLACE below deletes the old analysis and, through the foreign key, its `pairs_owed` row, so a
        // picture owes its pairs exactly when the row is put back below.
        self.delete_pairs.execute([item.id.get()])?;
        let (taken, taken_utc) = a.taken.map_or((None, false), |t| {
            let (seconds, utc) = t.parts();
            (Some(seconds), utc)
        });
        let (image, video) = (a.image(), a.video());
        // A picture of a reliable quality owes its pairs (what the pass pairs, `pairs.rs`).
        let owes_pairs = image.is_some_and(|i| i.pixels.pdq.quality >= pg_model::pdq::MIN_QUALITY);
        self.analysis.execute(params![
            item.id.get(),
            a.version,
            item.size,
            item.mtime,
            &a.file_hash.0[..],
            image.map(|i| &i.pixels.pixel_hash[..]),
            a.format(),
            a.orientation.get(),
            a.width,
            a.height,
            image.map(|i| pg_model::pdq::to_blob(&i.pixels.pdq.orientations)),
            image.map(|i| i.pixels.pdq.quality),
            a.thumbhash.as_slice(),
            a.frames,
            taken,
            taken_utc,
            video.map(|v| v.duration_ms),
            video.map(|v| v.poster_ms),
            item.rotation.get(),
            item.kind.key(),
        ])?;
        if owes_pairs {
            self.owe_pairs.execute([item.id.get()])?;
        }
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

/// The pairing pass's writes: one batch per write transaction ([`PairsWriter::write`]). A pair is stored only while both
/// pictures still hold the analyses it was made from (their stored hashes and quality): a file analyzed again in
/// between (by an import, here or in another process) owes its own pairs again (its pass makes them), and a stale pair
/// must never be stored beside them. A picture is marked done only while it holds the analysis its pairs were made
/// from, so its owed state is kept by a change in the meantime. Its statements are prepared once per pass (the
/// connection's statement cache: the pass's connection runs only these while it writes).
pub struct PairsWriter<'tx> {
    conn: &'tx Connection,
}

/// One batch of the pairing pass's writes, in pictures of its load (indices into [`PairingWork`]'s columns).
#[derive(Clone, Copy, Debug)]
pub struct PairsBatch<'a> {
    /// The pictures marked done: every pair each one owns is in `pairs`.
    pub owners: &'a [usize],
    /// The pairs, `(a, b, distance)`, each once.
    pub pairs: &'a [(usize, usize, u32)],
}

/// What [`PairsWriter::write`] wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PairsWritten {
    /// The pairs stored or lowered (one already stored at a lower distance stays).
    pub pairs: usize,
    /// The owners marked done.
    pub done: usize,
    /// Whether the guard read the batch's analyses: something they depend on changed since the load
    /// (`meta.pairing_epoch`).
    pub guarded: bool,
}

/// [`PairsWriter`]'s statements.
const PAIR: &str = "INSERT INTO pdq_pairs(a, b, distance) VALUES (?1, ?2, ?3)
     ON CONFLICT(a, b) DO UPDATE SET distance = excluded.distance WHERE excluded.distance < pdq_pairs.distance";
const DONE: &str = "DELETE FROM pairs_owed WHERE file_id IN rarray(?1)";
const GUARD: &str = "SELECT file_id, pdq_dihedral, pdq_quality FROM analysis WHERE file_id IN rarray(?1)";

impl<'tx> PairsWriter<'tx> {
    pub(crate) fn new(tx: &'tx Transaction) -> PairsWriter<'tx> {
        PairsWriter { conn: tx }
    }

    /// Write `batch`, paired from `work`: its pairs whose two pictures pass, and its owners that pass marked done. A
    /// picture passes while its analysis is the one the load read: it is there, with the stored hashes `hashes(i)`
    /// returns (as `pg_model::pdq::to_blob` stores them) and the load's quality. When the catalog's pairing epoch is the
    /// load's, nothing they depend on changed, and every picture passes unread; otherwise the batch's analyses are read,
    /// once each, by exact ids (`hashes` is asked only then). `None`, and nothing written, when the catalog's pairing
    /// version is not the load's: another build's pairing owes every pair again, and these are not its pairs.
    pub fn write(
        &mut self,
        work: &PairingWork,
        batch: PairsBatch<'_>,
        hashes: &dyn Fn(usize) -> Option<[u8; pg_model::pdq::BLOB_BYTES]>,
    ) -> Result<Option<PairsWritten>> {
        let marks = crate::work::pairing_marks(self.conn)?;
        if marks.version != work.marks.version {
            return Ok(None);
        }
        let guarded = marks.epoch != work.marks.epoch;
        // Picture indices, sorted: those that pass, when the guard ran.
        let passing = if guarded { Some(self.passing(work, batch, hashes)?) } else { None };
        let passes = |i: usize| passing.as_ref().is_none_or(|p: &Vec<usize>| p.binary_search(&i).is_ok());
        let id = |i: usize| work.ids.get(i).map(|id| id.get());
        let mut written = PairsWritten { guarded, ..PairsWritten::default() };
        let mut pair = self.conn.prepare_cached(PAIR)?;
        for &(a, b, distance) in batch.pairs {
            if let (true, Some(ia), Some(ib)) = (passes(a) && passes(b), id(a), id(b)) {
                written.pairs += pair.execute(params![ia.min(ib), ia.max(ib), distance])?;
            }
        }
        let owners: Vec<Value> =
            batch.owners.iter().filter(|&&i| passes(i)).filter_map(|&i| id(i)).map(Value::Integer).collect();
        written.done = self.conn.prepare_cached(DONE)?.execute([Rc::new(owners)])?;
        Ok(Some(written))
    }

    /// The batch's pictures (owners and partners) whose analysis is the one the load read, by index, sorted. An
    /// analysis that is gone (its file rebound or replaced), a failure in its place (no hashes), other hashes or another
    /// quality do not pass.
    fn passing(
        &self,
        work: &PairingWork,
        batch: PairsBatch<'_>,
        hashes: &dyn Fn(usize) -> Option<[u8; pg_model::pdq::BLOB_BYTES]>,
    ) -> Result<Vec<usize>> {
        let mut pictures: Vec<usize> = batch.owners.to_vec();
        pictures.extend(batch.pairs.iter().flat_map(|&(a, b, _)| [a, b]));
        pictures.sort_unstable();
        pictures.dedup();
        let ids: Vec<Value> =
            pictures.iter().filter_map(|&i| work.ids.get(i)).map(|id| Value::Integer(id.get())).collect();
        let mut guard = self.conn.prepare_cached(GUARD)?;
        let mut rows = guard.query([Rc::new(ids)])?;
        let mut passing = Vec::with_capacity(pictures.len());
        while let Some(r) = rows.next()? {
            let id = FileId::new(r.get(0)?);
            // The ids are the pictures' (in file-id order, as the indices are), so each row is one of them.
            let Some(&i) = pictures
                .binary_search_by_key(&Some(id), |&i| work.ids.get(i).copied())
                .ok()
                .and_then(|at| pictures.get(at))
            else {
                continue;
            };
            let stored = r.get_ref(1)?.as_blob_or_null()?;
            let quality: Option<u8> = r.get(2)?;
            if stored.is_some()
                && stored == hashes(i).as_ref().map(|h| &h[..])
                && quality == work.quality.get(i).copied()
            {
                passing.push(i);
            }
        }
        passing.sort_unstable();
        Ok(passing)
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
    pub(crate) fn new(tx: &'tx Transaction) -> Result<BordersWriter<'tx>> {
        Ok(BordersWriter {
            current: tx.prepare(crate::current::IS_CURRENT)?,
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

    /// Record borders looked for (`Catalog::pending_borders`; `None`: not looked
    /// for), replacing what was there, unless the file changed meanwhile (the import then analyzes it again, and the
    /// borders are due again) or a write of the app's into it is in progress (they stay due).
    pub fn store_borders(&mut self, item: &WorkItem, borders: Option<&Detection>) -> Result<()> {
        if !crate::current::query_current(&mut self.current, item.id, &item.listed())? {
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
            let color = s.packed_color();
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

/// The failure ledger's writes in a transaction of the caller's: the borders pass records and clears its rows in the
/// transaction that stores the borders, so a failure to write the ledger stores nothing of the batch.
pub struct LedgerWriter<'tx> {
    conn: &'tx Connection,
}

impl<'tx> LedgerWriter<'tx> {
    pub(crate) fn new(tx: &'tx Transaction) -> LedgerWriter<'tx> {
        LedgerWriter { conn: tx }
    }

    /// `record_failures_on`: whether a row changed.
    pub fn record(&mut self, pass: FilePass, failures: &[Failure], now: i64) -> Result<bool> {
        record_failures_on(self.conn, pass, failures, now)
    }

    /// `clear_failures_on`: whether a row went.
    pub fn clear(&mut self, pass: FilePass, files: &[FileId]) -> Result<bool> {
        clear_failures_on(self.conn, pass, files)
    }
}

/// The import's ledger is the triggers' (`analysis_import_*`, mirroring the failed analyses): no pass writes it.
fn written_by_its_pass(pass: FilePass) -> Result<()> {
    match pass {
        FilePass::Thumbs | FilePass::Borders => Ok(()),
        FilePass::Import => Err(Error::Other(anyhow!("the import's failures are kept by its analyses, not written"))),
    }
}

/// Whether the test fixture makes the ledgers' writes fail ([`crate::fixture::LedgerWritesFail`]): the other crates'
/// tests of a ledger that can't be written, in every process-wide connection at once.
#[cfg(any(test, feature = "fixture"))]
pub(crate) static LEDGER_FAULT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(any(test, feature = "fixture"))]
fn injected_ledger_fault() -> Result<()> {
    if LEDGER_FAULT.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(Error::Other(anyhow!("the test fixture fails the ledger's writes")));
    }
    Ok(())
}

#[cfg(not(any(test, feature = "fixture")))]
#[expect(clippy::unnecessary_wraps, reason = "the fixture's build can fail here")]
fn injected_ledger_fault() -> Result<()> {
    Ok(())
}

/// Record `failures` of `pass` (the thumbnails' or the borders'), found at `now`, on `conn`: one row per file, kept
/// only while the catalog has the file as the pass read it (one changed or gone since is not recorded). A failure
/// recorded already is updated only if its error or its retry changed, and keeps its serial. One statement for them
/// all. Whether a row changed (`changes()`): a failure that holds writes nothing.
pub(crate) fn record_failures_on(conn: &Connection, pass: FilePass, failures: &[Failure], now: i64) -> Result<bool> {
    written_by_its_pass(pass)?;
    injected_ledger_fault()?;
    if failures.is_empty() {
        return Ok(false);
    }
    let rows: Vec<serde_json::Value> = failures
        .iter()
        .map(|f| {
            serde_json::json!([f.file_id.get(), f.mtime, f.size, f.rotation.get(), tidy(&f.error), i64::from(f.retry)])
        })
        .collect();
    let rows = serde_json::to_string(&rows).map_err(anyhow::Error::from)?;
    let changed = conn
        .prepare_cached(
            "INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen)
         SELECT f.id, ?1, f.mtime, f.size, f.rotation, j.value ->> 4, j.value ->> 5, ?3
         FROM json_each(?2) j CROSS JOIN files f ON f.id = j.value ->> 0
         WHERE f.mtime = j.value ->> 1 AND f.size = j.value ->> 2 AND f.rotation = j.value ->> 3 AND f.missing = 0
         ON CONFLICT(file_id, pass) DO UPDATE SET error = excluded.error, retry = excluded.retry
         WHERE file_failures.error IS NOT excluded.error OR file_failures.retry IS NOT excluded.retry",
        )?
        .execute(params![pass.code(), rows, now])?;
    Ok(changed > 0)
}

/// The results of `files` were made by `pass` (the thumbnails' or the borders'): their rows go, on `conn`. Whether a
/// row went.
pub(crate) fn clear_failures_on(conn: &Connection, pass: FilePass, files: &[FileId]) -> Result<bool> {
    written_by_its_pass(pass)?;
    injected_ledger_fault()?;
    if files.is_empty() {
        return Ok(false);
    }
    let gone = conn
        .prepare_cached("DELETE FROM file_failures WHERE pass = ?1 AND file_id IN (SELECT value FROM json_each(?2))")?
        .execute(params![pass.code(), file_ids_json(files)?])?;
    Ok(gone > 0)
}

/// Whether `pass`'s ledger holds any of `files`, on `conn`: a read, so a clear of what isn't there takes no write lock.
pub(crate) fn failures_held(conn: &Connection, pass: FilePass, files: &[FileId]) -> Result<bool> {
    if files.is_empty() {
        return Ok(false);
    }
    Ok(conn
        .prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM file_failures WHERE pass = ?1 AND file_id IN (SELECT value FROM json_each(?2)))",
        )?
        .query_row(params![pass.code(), file_ids_json(files)?], |r| r.get(0))?)
}

/// The ids of `files`, as the JSON array the ledger's statements read with `json_each`.
fn file_ids_json(files: &[FileId]) -> Result<String> {
    Ok(serde_json::to_string(&files.iter().map(|f| f.get()).collect::<Vec<_>>()).map_err(anyhow::Error::from)?)
}

#[cfg(test)]
mod tests {
    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::{self, raw};
    use crate::test_support::*;
    use crate::{Catalog, PairingWork, PairsBatch, PairsWritten, Recorded, ScannedFile, WorkItem, queue};
    use pg_analysis::analyze::IMPORT_VERSION;
    use pg_model::analysis::{Analysis, FileHash, ImageFacts, Media, PixelHashes, Taken, ThumbHash};
    use pg_model::detection::Detection;
    use pg_model::format::Format;
    use pg_model::ids::{FileId, FileIdentity, FolderId, ScanId};
    use pg_model::media::Ext;
    use pg_model::name::Name;
    use pg_model::orientation::Orientation;
    use pg_model::rating::Rating;
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;
    use rusqlite::params;
    use std::path::Path;

    fn a_scan_tells_a_file_found_as_it_was_from_one_that_changed(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let key = |name: &str| {
            let folder = rules.merge(&p(rules, r"D:\x"));
            rules.order_key(&rules.merged_key(&folder), &Name::from(name))
        };
        let scans: Vec<ScanId> = (0..9).map(|_| begin(&mut c, &p(rules, r"D:\x"))).collect();
        assert!(scanned(&mut c, scans[0], "a.jpg", 10, 100), "new");
        // As it was: only marked seen by the scan.
        assert!(!scanned(&mut c, scans[1], "a.jpg", 10, 100));
        assert_eq!(raw::file_row(&c), (scans[1].get(), "a.jpg".into(), 10, 100, 0, key("a.jpg")));
        for (scan, name, size, mtime) in [
            (scans[2], "A.jpg", 10, 100),
            (scans[3], "b.jpg", 10, 100),
            (scans[4], "b.jpg", 11, 100),
            (scans[5], "b.jpg", 11, 101),
        ] {
            assert!(scanned(&mut c, scan, name, size, mtime), "{name} {size} {mtime}");
            assert_eq!(raw::file_row(&c), (scan.get(), name.into(), i64::try_from(size).unwrap(), mtime, 0, key(name)));
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

    under_both_rules!(
        a_scan_tells_a_file_found_as_it_was_from_one_that_changed,
        a_scan_tells_a_file_found_as_it_was_from_one_that_changed_windows,
        a_scan_tells_a_file_found_as_it_was_from_one_that_changed_posix
    );

    /// The scanner finding a file in D:\x with this identity and creation time (a.jpg, 10 bytes, mtime 100).
    fn found_as(c: &mut Catalog, scan: ScanId, identity: FileIdentity, created: Option<i64>) -> Recorded {
        let rules = c.rules();
        let prior = listed(c, &p(rules, r"D:\x"), &[identity]).remove(0);
        let tx = c.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
        let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
        let file = ScannedFile {
            identity,
            name: &Name::from("a.jpg"),
            ext: Ext::of_name("a.jpg").unwrap(),
            size: 10,
            mtime: 100,
            created,
            prior,
        };
        let recorded = writer.upsert_file(&folder, &file).unwrap();
        drop(writer);
        tx.commit().unwrap();
        recorded
    }

    fn a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let scan = begin(&mut c, &p(rules, r"D:\x"));
        let (volume, file) = (u64::MAX - 1, u64::MAX);
        assert_eq!(found_as(&mut c, scan, FileIdentity::new(volume, file), None), Recorded::Changed);
        let stored: (i64, i64) =
            c.conn.query_row("SELECT volume, file_ref FROM files", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((stored.0.cast_unsigned(), stored.1.cast_unsigned()), (volume, file));
        // The same identity read back from the catalog is the same file: no new row.
        assert_eq!(found_as(&mut c, scan, FileIdentity::new(volume, file), None), Recorded::Same);
    }

    under_both_rules!(
        a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again,
        a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again_windows,
        a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again_posix
    );

    fn a_creation_time_fills_in_once_and_a_different_one_is_a_different_file(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let scan = begin(&mut c, &p(rules, r"D:\x"));
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

    under_both_rules!(
        a_creation_time_fills_in_once_and_a_different_one_is_a_different_file,
        a_creation_time_fills_in_once_and_a_different_one_is_a_different_file_windows,
        a_creation_time_fills_in_once_and_a_different_one_is_a_different_file_posix
    );

    fn what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let scan = begin(&mut c, &p(rules, r"D:\x"));
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let stored = |c: &mut Catalog, item: &WorkItem| {
            let tx = c.tx().unwrap();
            let stored = tx.analysis_writer(versions().import).unwrap().store_failure(item, "a stand-in").unwrap();
            tx.commit().unwrap();
            stored
        };
        let item = next_work(&c);
        // A write of the app's into it in progress: it may have been read part-written.
        c.queue_rotation(item.id, Turns::new(3)).unwrap();
        let job = c.next_write(0).unwrap().unwrap();
        let temp = rules.parse(&spelled(rules, r"D:\x\a.jpg.tmp")).unwrap();
        c.record_write_temp(&job, &temp, pg_model::write::Write::Turns(Turns::new(3))).unwrap();
        assert!(!stored(&mut c, &item));
        assert!(due(&c, item.id), "still due");
        c.write_done(&job, queue::WriteEffect::Nothing).unwrap();
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

    under_both_rules!(
        what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed,
        what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed_windows,
        what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed_posix
    );

    fn a_size_change_alone_moves_the_file_in_the_thumbnail_source(rules: PathRules) {
        // The scanner decides a file changed by its size and mtime, so a size change with the same mtime must reach
        // the thumbnail pass (through files.seq), or a thumbnail made from the old file would be kept.
        use fixture::{File, insert};
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        crate::fixture::state::add_scan(&c, 1, &spelled(rules, r"D:\x")).unwrap();
        crate::fixture::state::add_folder(&c, 1, &spelled(rules, r"D:\x")).unwrap();
        insert(&c, &[File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]).unwrap();
        raw::insert_analysis(&c, &raw::AnalysisRow::image(1, 10, 100));
        let before = raw::thumb_source_seq(&c, FileId::new(1));
        raw::set_size(&c, FileId::new(1), 11);
        assert!(raw::thumb_source_seq(&c, FileId::new(1)) > before);
    }

    under_both_rules!(
        a_size_change_alone_moves_the_file_in_the_thumbnail_source,
        a_size_change_alone_moves_the_file_in_the_thumbnail_source_windows,
        a_size_change_alone_moves_the_file_in_the_thumbnail_source_posix
    );

    fn thumb_sources_is_the_analysed_files_with_their_folders(rules: PathRules) {
        use fixture::{File, insert};
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        crate::fixture::state::add_scan(&c, 1, &spelled(rules, r"D:\x")).unwrap();
        crate::fixture::state::add_folder(&c, 1, &spelled(rules, r"D:\x")).unwrap();
        crate::fixture::state::add_folder(&c, 2, &spelled(rules, r"D:\x\sub")).unwrap();
        insert(
            &c,
            &[
                File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100),
                File::new(FileId::new(2), FolderId::new(2), "b.jpg", 10, 100),
                File::new(FileId::new(3), FolderId::new(1), "c.jpg", 10, 100),
            ],
        )
        .unwrap();
        // File 1 analyzed (a source), file 2 failed and file 3 not analyzed (neither can have a thumbnail).
        raw::insert_analysis(&c, &raw::AnalysisRow::image(1, 10, 100));
        raw::insert_analysis(
            &c,
            &raw::AnalysisRow { error: Some("unreadable"), ..raw::AnalysisRow::image(2, 10, 100) },
        );
        assert_eq!(raw::thumb_sources(&c), [(1, spelled(rules, r"D:\x"))]);
        // An image has no poster time.
        let poster: Option<i64> = c.conn.query_row("SELECT poster_ms FROM thumb_sources", [], |r| r.get(0)).unwrap();
        assert_eq!(poster, None);
        // The clock is the catalog's change number, and the folders' bound the largest folder id, as the pass reads them.
        assert_eq!(raw::thumb_clock(&c), raw::meta_seq(&c));
        assert_eq!(c.conn.query_row("SELECT max_id FROM thumb_folders", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        // A change number's range of the view still reads files_seq (the cache's join over it is tested there).
        let plan = raw::query_plan(&c, "SELECT id FROM thumb_sources WHERE seq > ?1", [-1]);
        assert!(plan.contains("files_seq"), "{plan:?}");
    }

    under_both_rules!(
        thumb_sources_is_the_analysed_files_with_their_folders,
        thumb_sources_is_the_analysed_files_with_their_folders_windows,
        thumb_sources_is_the_analysed_files_with_their_folders_posix
    );

    fn borders_are_looked_for_after_the_import_and_again_after_a_new_detector(rules: PathRules) {
        use pg_model::detection::{Edge, Kind as SideKind, Reason, Side};
        use pg_model::query::{BorderFacet, Query, Sort};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let scan = begin(&mut c, &p(rules, r"D:\x"));
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
            file_hash: pg_model::analysis::FileHash([1; 32]),
            orientation: Orientation::UPRIGHT,
            width: 400,
            height: 300,
            thumbhash: ThumbHash::default(),
            frames: 1,
            taken: None,
            media: a_jpeg(),
        };
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &analysis).unwrap();
        tx.commit().unwrap();
        // The import leaves them to the borders pass.
        assert_eq!(raw::cut_sides(&c), [None; 4]);
        let due = c.pending_borders().unwrap();
        assert_eq!(due.items.iter().map(|d| d.id).collect::<Vec<_>>(), [item.id]);
        let (_, since) = changed_since(&c, 0);
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&due.items[0], Some(&found)).unwrap();
        tx.commit().unwrap();
        assert_eq!(raw::cut_sides(&c), [Some(0), Some(41), Some(0), Some(41)]);
        // The cut and its sides (`analysis_cut_stamp`, `border_sides_stamp_insert`) name the file to a client that keeps
        // its list.
        assert_eq!(changed_since(&c, since).0, [item.id], "the borders name their file");
        assert_eq!(c.border_sides(item.id).unwrap(), found.sides);
        assert!(c.pending_borders().unwrap().is_empty());

        // The grid's entry, the filters and the sort.
        let list = |c: &Catalog, q: Query| -> Vec<pg_model::grid::Entry> {
            c.list_grid(&pg_model::rules::Merged::default(), true, &q, Sort::default()).unwrap()
        };
        let entry = list(&c, Query::default()).remove(0);
        assert_eq!(entry.borders, pg_model::grid::Borders::Found);
        let [x, y, w, h] = entry.content().unwrap();
        assert!(x == 0.0 && w == 1.0 && (y - 41.0 / 300.0).abs() < 1e-9 && (h - 218.0 / 300.0).abs() < 1e-9);
        let only = |b: Vec<BorderFacet>| Query { borders: b, ..Query::default() };
        assert_eq!(list(&c, only(vec![BorderFacet::Black])).len(), 1);
        assert!(list(&c, only(vec![BorderFacet::Blur, BorderFacet::None])).is_empty());
        let counts = c.facets(&pg_model::rules::Merged::default(), true, &Query::default()).unwrap().borders;
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
        assert!(c.border_sides(item.id).unwrap().is_empty());
        assert!(c.pending_borders().unwrap().is_empty());
        assert_eq!(list(&c, Query::default())[0].borders, pg_model::grid::Borders::None);
        assert_eq!(raw::border_marks(&c), 1);

        // Analyzed again (the file changed): the old borders go, the new ones are due.
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&due.items[0], Some(&found)).unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &analysis).unwrap();
        tx.commit().unwrap();
        assert_eq!(raw::cut_sides(&c), [None; 4]);
        assert!(c.border_sides(item.id).unwrap().is_empty());
        assert_eq!(c.pending_borders().unwrap().len(), 1);
    }

    under_both_rules!(
        borders_are_looked_for_after_the_import_and_again_after_a_new_detector,
        borders_are_looked_for_after_the_import_and_again_after_a_new_detector_windows,
        borders_are_looked_for_after_the_import_and_again_after_a_new_detector_posix
    );

    /// The borders the grid shows are a change of the file (`files.changed`): storing them, a changed cut, removed
    /// sides and a removed analysis each stamp it. Its thumbnails' source (`files.seq`) is not touched.
    fn borders_stamp_the_file_as_changed_and_not_its_thumbnails(rules: PathRules) {
        use pg_model::detection::{Edge, Kind as SideKind, Side};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let scan = begin(&mut c, &p(rules, r"D:\x"));
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let item = next_work(&c);
        let analysis = Analysis {
            version: IMPORT_VERSION,
            file_hash: pg_model::analysis::FileHash([1; 32]),
            orientation: Orientation::UPRIGHT,
            width: 400,
            height: 300,
            thumbhash: ThumbHash::default(),
            frames: 1,
            taken: None,
            media: a_jpeg(),
        };
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &analysis).unwrap();
        tx.commit().unwrap();
        let seq = |c: &Catalog| -> i64 {
            c.conn.query_row("SELECT seq FROM files WHERE id = ?1", [item.id.get()], |r| r.get(0)).unwrap()
        };
        let thumbs = seq(&c);
        // Each step is a change of the file, after the change number before it.
        let stamped = |c: &Catalog, since: i64| -> i64 {
            let (changed, now) = changed_since(c, since);
            assert_eq!(changed, [item.id]);
            assert_eq!(seq(c), thumbs, "the thumbnails' source is not changed");
            now
        };
        let (_, before) = changed_since(&c, 0);

        let top = Side {
            pass: 0,
            edge: Edge::Top,
            kind: SideKind::Solid,
            depth: 41,
            color: Some([0, 0, 0]),
            straight: 1.0,
            dev: Some(0.0),
            drift: Some(0.5),
            rejected: None,
        };
        let sides = [top].into_iter().collect();
        let found = Detection { width: 400, height: 300, content: [0, 41, 400, 300], sides };
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&item, Some(&found)).unwrap();
        tx.commit().unwrap();
        let now = stamped(&c, before);

        // The same cut written again changes nothing.
        c.conn.execute("UPDATE analysis SET cut_top = cut_top WHERE file_id = ?1", [item.id.get()]).unwrap();
        assert_eq!(changed_since(&c, now), (vec![], now));
        c.conn.execute("UPDATE analysis SET cut_top = 7 WHERE file_id = ?1", [item.id.get()]).unwrap();
        let now = stamped(&c, now);
        c.conn.execute("DELETE FROM border_sides WHERE file_id = ?1", [item.id.get()]).unwrap();
        let now = stamped(&c, now);
        c.conn.execute("DELETE FROM analysis WHERE file_id = ?1", [item.id.get()]).unwrap();
        // `analysis_removed`: a file with no analysis is pending again.
        let pending: i64 =
            c.conn.query_row("SELECT pending FROM files WHERE id = ?1", [item.id.get()], |r| r.get(0)).unwrap();
        assert_eq!(pending, 1, "a deleted analysis leaves its file pending");
        stamped(&c, now);
    }

    under_both_rules!(
        borders_stamp_the_file_as_changed_and_not_its_thumbnails,
        borders_stamp_the_file_as_changed_and_not_its_thumbnails_windows,
        borders_stamp_the_file_as_changed_and_not_its_thumbnails_posix
    );

    fn border_depth_sorts_by_the_largest_share_of_a_side_cut(rules: PathRules) {
        use pg_model::query::{Query, Sort, SortKey};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let scan = begin(&mut c, &p(rules, r"D:\x"));
        // 400 × 300 stored. b: none; c: 30 of 300 at the top (10%); d: 60 of 400 at the left (15%), but turned a
        // quarter (orientation 6), so 60 of the 400 upright rows; e: 90 at the top (30%), marked not a border.
        let files = [("b.jpg", 1, None), ("c.jpg", 1, Some([0, 30, 0, 0])), ("d.jpg", 6, Some([60, 0, 0, 0]))];
        let files = files.into_iter().chain([("e.jpg", 1, Some([0, 90, 0, 0]))]);
        for (n, (name, orientation, cut)) in files.enumerate() {
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
            let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
            let ext = Ext::of_name(name).unwrap();
            let file = ScannedFile {
                identity: FileIdentity::new(1, 10 + u64::try_from(n).unwrap()),
                name: &Name::from(name),
                ext,
                size: 10,
                mtime: 100,
                created: None,
                prior: None,
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
                file_hash: pg_model::analysis::FileHash([u8::try_from(n).unwrap(); 32]),
                orientation: Orientation::from_tag(orientation),
                width,
                height,
                thumbhash: ThumbHash::default(),
                frames: 1,
                taken: None,
                media: a_jpeg(),
            };
            let tx = c.tx().unwrap();
            tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &analysis).unwrap();
            tx.borders_writer().unwrap().store_borders(&item, Some(&borders)).unwrap();
            tx.commit().unwrap();
            if name == "e.jpg" {
                c.set_border_mark(item.id, true).unwrap();
            }
        }
        let names = |descending: bool| -> Vec<String> {
            let sort = Sort { key: SortKey::Borders, descending };
            let entries = c.list_grid(&pg_model::rules::Merged::default(), true, &Query::default(), sort).unwrap();
            let name = |id| c.details(id).unwrap().unwrap().path.parts.last().map(|n| n.display().into_owned());
            entries.iter().map(|e| name(e.id).unwrap_or_default()).collect()
        };
        // Without a border (none found, or marked) last, by name, either way.
        assert_eq!(names(false), ["c.jpg", "d.jpg", "b.jpg", "e.jpg"]);
        assert_eq!(names(true), ["d.jpg", "c.jpg", "b.jpg", "e.jpg"]);
    }

    under_both_rules!(
        border_depth_sorts_by_the_largest_share_of_a_side_cut,
        border_depth_sorts_by_the_largest_share_of_a_side_cut_windows,
        border_depth_sorts_by_the_largest_share_of_a_side_cut_posix
    );

    fn a_camera_clock_is_kept_as_it_is(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        store_taken(&mut c, &item, Some(Taken::Wall(1_700_000_000)));
        assert_eq!(taken_columns(&c), (Some(1_700_000_000), None));
    }

    under_both_rules!(
        a_camera_clock_is_kept_as_it_is,
        a_camera_clock_is_kept_as_it_is_windows,
        a_camera_clock_is_kept_as_it_is_posix
    );

    fn a_utc_instant_is_converted_to_the_local_wall_clock_and_kept_as_well(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        let instant = 1_714_557_600;
        store_taken(&mut c, &item, Some(Taken::Utc(instant)));
        assert_eq!(taken_columns(&c), (Some(local_of(&c, instant)), Some(instant)));
    }

    under_both_rules!(
        a_utc_instant_is_converted_to_the_local_wall_clock_and_kept_as_well,
        a_utc_instant_is_converted_to_the_local_wall_clock_and_kept_as_well_windows,
        a_utc_instant_is_converted_to_the_local_wall_clock_and_kept_as_well_posix
    );

    /// A catalog with files 1 and 2 in D:\x, both pending: the import has not analyzed them yet.
    fn two_files(rules: PathRules) -> Catalog {
        let (c, _) = one_file(rules);
        add_file(&c, 2, "b.jpg");
        c
    }

    /// Adds file `id` (pending) to the scan and folder of [`one_file`].
    fn add_file(c: &Catalog, id: i64, name: &'static str) {
        fixture::insert(c, &[fixture::File::new(FileId::new(id), FolderId::new(1), name, 10, 100)]).unwrap();
    }

    /// An image's analysis as the import stores it: its PDQ hash `hash` at `quality`, in all four kept orientations (the
    /// writer does not check them).
    fn picture(hash: [u8; 32], quality: u8) -> Analysis {
        let pdq =
            pg_model::pdq::Pdq { orientations: [pg_model::pdq::Hash(hash); pg_model::pdq::ORIENTATIONS], quality };
        let pixels = PixelHashes { pixel_hash: [0; 16], pdq };
        Analysis {
            version: IMPORT_VERSION,
            file_hash: FileHash([1; 32]),
            orientation: Orientation::UPRIGHT,
            width: 400,
            height: 300,
            thumbhash: ThumbHash::default(),
            frames: 1,
            taken: None,
            media: Media::Image(ImageFacts { format: Format::Jpeg, file_rating: None, pixels }),
        }
    }

    /// File `id`'s work item as the work lists give it now, pending or not.
    fn item_of(c: &Catalog, id: i64) -> WorkItem {
        crate::work::work_item(&c.conn, FileId::new(id)).unwrap().unwrap()
    }

    /// Stores `stored` as the import of file `id` stores it.
    fn store_for(c: &mut Catalog, id: i64, stored: &Analysis) {
        let item = item_of(c, id);
        let tx = c.tx().unwrap();
        assert!(tx.analysis_writer(versions().import).unwrap().store_analysis(&item, stored).unwrap());
        tx.commit().unwrap();
    }

    /// The files with a `pairs_owed` row, by id.
    fn state_owed(c: &Catalog) -> Vec<i64> {
        raw::rows_of(c, "SELECT file_id FROM pairs_owed ORDER BY file_id", [], |r| r.get(0))
    }

    /// Whether file `id` owes its pairs (a `pairs_owed` row).
    fn owes(c: &Catalog, id: i64) -> bool {
        c.conn.query_row("SELECT EXISTS(SELECT 1 FROM pairs_owed WHERE file_id = ?1)", [id], |r| r.get(0)).unwrap()
    }

    /// The stored distance of the pair `a`, `b` (`a < b`), if there is one.
    fn pair_distance(c: &Catalog, a: i64, b: i64) -> Option<i64> {
        c.conn.query_row("SELECT distance FROM pdq_pairs WHERE a = ?1 AND b = ?2", [a, b], |r| r.get(0)).ok()
    }

    fn an_analysis_owes_its_pairs_only_with_hashes_of_a_reliable_quality(rules: PathRules) {
        let mut c = two_files(rules);
        store_for(&mut c, 1, &picture([1; 32], pg_model::pdq::MIN_QUALITY));
        store_for(&mut c, 2, &picture([2; 32], pg_model::pdq::MIN_QUALITY - 1));
        assert!(owes(&c, 1));
        assert!(!owes(&c, 2), "below the quality it owes nothing");
        // Analyzed again, it owes again: the REPLACE's delete takes the row through the foreign key, and the writer
        // puts it back.
        store_for(&mut c, 1, &picture([3; 32], 100));
        assert!(owes(&c, 1));
        // A failure owes nothing: its REPLACE takes the row, and nothing puts it back.
        let item = item_of(&c, 1);
        analyzed(&mut c, &item);
        assert!(!owes(&c, 1));
        // An analysis that goes (a path taken over by another file, `pair.rs`) takes its row with it.
        store_for(&mut c, 1, &picture([4; 32], 100));
        assert!(owes(&c, 1));
        c.conn.execute("DELETE FROM analysis WHERE file_id = 1", []).unwrap();
        assert!(!owes(&c, 1));
        // A row needs its analysis.
        assert!(c.conn.execute("INSERT INTO pairs_owed(file_id) VALUES (2)", []).is_ok());
        assert!(c.conn.execute("INSERT INTO pairs_owed(file_id) VALUES (99)", []).is_err(), "no analysis 99");
    }

    under_both_rules!(
        an_analysis_owes_its_pairs_only_with_hashes_of_a_reliable_quality,
        an_analysis_owes_its_pairs_only_with_hashes_of_a_reliable_quality_windows,
        an_analysis_owes_its_pairs_only_with_hashes_of_a_reliable_quality_posix
    );

    /// One batch through [`PairsWriter::write`], in a transaction of its own, guarded by the load's hashes.
    fn write_batch(
        c: &mut Catalog,
        work: &PairingWork,
        owners: &[usize],
        pairs: &[(usize, usize, u32)],
    ) -> Option<PairsWritten> {
        let tx = c.tx().unwrap();
        let hashes = |i: usize| work.hashes.get(i).map(pg_model::pdq::to_blob);
        let written = tx.pairs_writer().unwrap().write(work, PairsBatch { owners, pairs }, &hashes).unwrap();
        tx.commit().unwrap();
        written
    }

    /// The catalog's pairing marks now.
    fn marks(c: &Catalog) -> crate::PairingMarks {
        crate::work::pairing_marks(&c.conn).unwrap()
    }

    /// The names of [`pictures`]' files.
    const NAMES: [&str; 13] = [
        "a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg", "f.jpg", "g.jpg", "h.jpg", "i.jpg", "j.jpg", "k.jpg", "l.jpg",
        "m.jpg",
    ];

    /// Files 1 to `n` (at most 12) in D:\x, each analysed as [`picture`]`([id; 32], 100)`: picture `id - 1` of the load.
    fn pictures(rules: PathRules, n: u8) -> Catalog {
        let (mut c, _) = one_file(rules);
        for (id, name) in (2..=i64::from(n)).zip(NAMES.iter().skip(1)) {
            add_file(&c, id, name);
        }
        for id in 1..=n {
            store_for(&mut c, i64::from(id), &picture([id; 32], 100));
        }
        c
    }

    fn a_pair_is_stored_only_while_both_pictures_hold_the_analyses_it_was_made_from(rules: PathRules) {
        let mut c = pictures(rules, 6);
        let work = c.pairing_work().unwrap();
        // Nothing changed since the load: written unread.
        assert_eq!(
            write_batch(&mut c, &work, &[1], &[(1, 0, 5)]),
            Some(PairsWritten { pairs: 1, done: 1, guarded: false }),
            "either order of the pair's pictures"
        );
        assert_eq!(write_batch(&mut c, &work, &[], &[(0, 1, 9)]).unwrap().pairs, 0, "a higher distance stays out");
        assert_eq!(write_batch(&mut c, &work, &[], &[(0, 1, 4)]).unwrap().pairs, 1, "a lower one replaces it");
        assert_eq!(pair_distance(&c, 1, 2), Some(4));
        // Since the load: file 3 analysed again with other hashes, 4 with the same ones, 5 at another quality; 6 failed.
        store_for(&mut c, 3, &picture([9; 32], 100));
        store_for(&mut c, 4, &picture([4; 32], 100));
        store_for(&mut c, 5, &picture([5; 32], 90));
        let failed = item_of(&c, 6);
        analyzed(&mut c, &failed);
        let pairs = [(0, 2, 1), (0, 3, 1), (0, 4, 1), (0, 5, 1)];
        assert_eq!(
            write_batch(&mut c, &work, &[0, 2, 3, 4, 5], &pairs),
            Some(PairsWritten { pairs: 1, done: 2, guarded: true }),
            "only files 1 and 4 hold the analyses the load read"
        );
        assert_eq!(pair_distance(&c, 1, 4), Some(1));
        for b in [3, 5, 6] {
            assert_eq!(pair_distance(&c, 1, b), None);
        }
        // 3 and 5 owe their new analyses' pairs; 6 failed, and owes none.
        assert_eq!(state_owed(&c), vec![3, 5]);
        // An analysis that is gone (a rebind or a replacement deletes it), with its file or alone: its pairs are left
        // out, with no foreign-key failure.
        c.conn.execute("DELETE FROM analysis WHERE file_id = 2", []).unwrap();
        c.conn.execute("DELETE FROM files WHERE id = 6", []).unwrap();
        assert_eq!(write_batch(&mut c, &work, &[], &[(1, 3, 2), (3, 5, 2)]).unwrap().pairs, 0);
    }

    under_both_rules!(
        a_pair_is_stored_only_while_both_pictures_hold_the_analyses_it_was_made_from,
        a_pair_is_stored_only_while_both_pictures_hold_the_analyses_it_was_made_from_windows,
        a_pair_is_stored_only_while_both_pictures_hold_the_analyses_it_was_made_from_posix
    );

    /// Another build's `Catalog::prepare` changed the pairing since the load: the batch writes nothing, and what was owed
    /// stays owed.
    fn a_batch_under_another_pairing_version_writes_nothing(rules: PathRules) {
        let mut c = pictures(rules, 2);
        let work = c.pairing_work().unwrap();
        crate::lifecycle::set_pairing_version(&c.conn, pg_model::pdq::PAIRING_VERSION + 1).unwrap();
        assert_eq!(write_batch(&mut c, &work, &[0, 1], &[(0, 1, 3)]), None);
        assert_eq!(pair_distance(&c, 1, 2), None);
        assert_eq!(state_owed(&c), vec![1, 2]);
    }

    under_both_rules!(
        a_batch_under_another_pairing_version_writes_nothing,
        a_batch_under_another_pairing_version_writes_nothing_windows,
        a_batch_under_another_pairing_version_writes_nothing_posix
    );

    fn a_new_analysis_drops_the_pairs_it_made_and_owes_them_again(rules: PathRules) {
        let mut c = pictures(rules, 2);
        let work = c.pairing_work().unwrap();
        assert_eq!(write_batch(&mut c, &work, &[0, 1], &[(0, 1, 5)]).unwrap().pairs, 1);
        assert_eq!(pair_distance(&c, 1, 2), Some(5));
        assert!(state_owed(&c).is_empty());
        // File 2 changed: its pairs go with the old analysis, and it owes them again.
        store_for(&mut c, 2, &picture([3; 32], 100));
        assert_eq!(pair_distance(&c, 1, 2), None);
        assert_eq!((owes(&c, 1), owes(&c, 2)), (false, true));
    }

    under_both_rules!(
        a_new_analysis_drops_the_pairs_it_made_and_owes_them_again,
        a_new_analysis_drops_the_pairs_it_made_and_owes_them_again_windows,
        a_new_analysis_drops_the_pairs_it_made_and_owes_them_again_posix
    );

    /// Whether `change` moves the catalog's pairing epoch on (which only grows).
    fn moves(c: &mut Catalog, change: &dyn Fn(&mut Catalog)) -> bool {
        let before = marks(c).epoch;
        change(c);
        let after = marks(c).epoch;
        assert!(after >= before, "the epoch only grows");
        after > before
    }

    /// A change that is one statement.
    fn sql(statement: &'static str) -> impl Fn(&mut Catalog) {
        move |c: &mut Catalog| {
            c.conn.execute(statement, []).unwrap();
        }
    }

    /// Migration 14's triggers: `meta.pairing_epoch` moves on each change to an analysis the pairs are guarded by, and on
    /// nothing else: not on owed rows, nor on the pass's own writes.
    fn the_pairing_epoch_moves_with_what_the_pairs_are_made_from(rules: PathRules) {
        let mut c = pictures(rules, 4);
        add_file(&c, 5, "e.jpg");
        let failed = item_of(&c, 5);
        analyzed(&mut c, &failed);
        // A new analysis over a picture (a REPLACE, whose own delete fires no trigger), with hashes or failed.
        assert!(moves(&mut c, &|c| store_for(c, 1, &picture([7; 32], 100))));
        assert!(moves(&mut c, &|c| {
            let item = item_of(c, 1);
            analyzed(c, &item);
        }));
        // An analysis's hashes or quality changed in place; the same values written again move nothing.
        assert!(moves(&mut c, &sql("UPDATE analysis SET pdq_quality = 80 WHERE file_id = 2")));
        assert!(moves(&mut c, &sql("UPDATE analysis SET pdq_dihedral = zeroblob(128) WHERE file_id = 2")));
        assert!(!moves(&mut c, &sql("UPDATE analysis SET pdq_quality = 80 WHERE file_id = 2")));
        // An analysis moved to another file (no writer does it; the guard would find the row gone).
        c.conn.execute("DELETE FROM pairs_owed WHERE file_id = 4", []).unwrap();
        add_file(&c, 6, "f.jpg");
        assert!(moves(&mut c, &sql("UPDATE analysis SET file_id = 6 WHERE file_id = 4")));
        assert!(moves(&mut c, &sql("UPDATE analysis SET file_id = 4 WHERE file_id = 6")));
        // A picture's analysis deleted, alone or with its file (the cascade's delete fires the trigger); a failure's
        // moves nothing (no pair could hold it).
        assert!(moves(&mut c, &sql("DELETE FROM analysis WHERE file_id = 2")));
        assert!(moves(&mut c, &sql("DELETE FROM files WHERE id = 3")));
        assert!(!moves(&mut c, &sql("DELETE FROM analysis WHERE file_id = 5")));
        // A picture owed again (`owe_pairs_again`), or done (the pass's delete): the guard never reads owed rows.
        assert!(!moves(&mut c, &sql("INSERT INTO pairs_owed(file_id) VALUES (4)")));
        assert!(!moves(&mut c, &sql("DELETE FROM pairs_owed WHERE file_id = 4")));
        // What the pairs are not made from: the analysis's borders and turn, the file's state, a rating.
        assert!(!moves(&mut c, &sql("UPDATE analysis SET borders_due = 1, rotation = 1, width = 7 WHERE file_id = 4")));
        assert!(!moves(&mut c, &sql("UPDATE files SET missing = 1, mtime = mtime + 1 WHERE id = 4")));
        assert!(!moves(&mut c, &|c| c.set_rating(FileId::new(4), Rating::new(3), false).unwrap()));
        // The pass's own writes.
        let mut c = pictures(rules, 2);
        let work = c.pairing_work().unwrap();
        assert!(!moves(&mut c, &|c| assert!(write_batch(c, &work, &[0, 1], &[(0, 1, 2)]).is_some())));
    }

    under_both_rules!(
        the_pairing_epoch_moves_with_what_the_pairs_are_made_from,
        the_pairing_epoch_moves_with_what_the_pairs_are_made_from_windows,
        the_pairing_epoch_moves_with_what_the_pairs_are_made_from_posix
    );

    /// A small deterministic generator for the differential test (SplitMix64).
    struct Mix(u64);

    impl Mix {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            usize::try_from((z ^ (z >> 31)) % u64::try_from(n.max(1)).unwrap()).unwrap()
        }
    }

    /// `PairsWriter`'s writes before the set-based guard, as the oracle: each pair and each done one statement, guarded
    /// by the stored hashes of its analyses alone.
    fn oracle_write(c: &mut Catalog, work: &PairingWork, owners: &[usize], pairs: &[(usize, usize, u32)]) {
        let tx = crate::transaction::write_transaction(&c.conn).unwrap();
        let mut pair = tx
            .prepare(
                "INSERT INTO pdq_pairs(a, b, distance)
                 SELECT ?1, ?2, ?3
                 WHERE (SELECT pdq_dihedral FROM analysis WHERE file_id = ?1) IS ?4
                   AND (SELECT pdq_dihedral FROM analysis WHERE file_id = ?2) IS ?5
                 ON CONFLICT(a, b) DO UPDATE SET distance = excluded.distance WHERE excluded.distance < pdq_pairs.distance",
            )
            .unwrap();
        let mut done = tx
            .prepare(
                "DELETE FROM pairs_owed
                 WHERE file_id = ?1 AND (SELECT pdq_dihedral FROM analysis WHERE file_id = ?1) IS ?2",
            )
            .unwrap();
        let id = |i: usize| work.ids.get(i).unwrap().get();
        let blob = |i: usize| pg_model::pdq::to_blob(work.hashes.get(i).unwrap());
        for &(a, b, distance) in pairs {
            let (lo, hi) = if id(a) < id(b) { (a, b) } else { (b, a) };
            pair.execute(params![id(lo), id(hi), distance, &blob(lo)[..], &blob(hi)[..]]).unwrap();
        }
        for &i in owners {
            done.execute(params![id(i), &blob(i)[..]]).unwrap();
        }
        drop((pair, done));
        tx.commit().unwrap();
    }

    /// The changes a picture's file and analysis go through between the load and the write, as the differential test
    /// makes them ([`change`]).
    const CHANGES: usize = 9;

    /// Change `which` (below [`CHANGES`]) to file `id`, if it is still there.
    fn change(c: &mut Catalog, id: i64, which: usize) {
        let exists = |c: &Catalog, sql: &str| -> bool { c.conn.query_row(sql, [id], |r| r.get(0)).unwrap() };
        if !exists(c, "SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1)") {
            return;
        }
        match which {
            // Analysed again: other hashes; the same hashes at the same quality.
            0 => store_for(c, id, &picture([200; 32], 100)),
            1 => store_for(c, id, &picture([u8::try_from(id).unwrap(); 32], 100)),
            // Failed.
            2 => {
                let item = item_of(c, id);
                analyzed(c, &item);
            }
            // Its analysis gone, alone or with its file.
            3 => {
                c.conn.execute("DELETE FROM analysis WHERE file_id = ?1", [id]).unwrap();
            }
            4 => {
                c.conn.execute("DELETE FROM files WHERE id = ?1", [id]).unwrap();
            }
            // Its file missing, or pending.
            5 => {
                c.conn.execute("UPDATE files SET missing = 1 WHERE id = ?1", [id]).unwrap();
            }
            6 => {
                c.mark_pending(&[FileId::new(id)]).unwrap();
            }
            // Owed again (a row needs its analysis).
            7 => {
                if exists(c, "SELECT EXISTS(SELECT 1 FROM analysis WHERE file_id = ?1)") {
                    c.conn.execute("INSERT OR IGNORE INTO pairs_owed(file_id) VALUES (?1)", [id]).unwrap();
                }
            }
            // Something the pairs are not made from: its rating.
            _ => c.set_rating(FileId::new(id), Rating::new(2), false).unwrap(),
        }
    }

    /// The batch writer stores and marks done what the row-by-row guard did (the oracle, [`oracle_write`]), whatever
    /// changed between the load and the write: on 64 random batches and changes, each written both ways on two copies
    /// of one catalog. Half the rounds move the epoch by hand, so the guard also runs where nothing changed. (A change of
    /// quality alone, which no writer makes, is the one the oracle let through: the test above has it.)
    fn the_batch_writer_writes_what_the_row_by_row_guard_did(rules: PathRules) {
        for seed in 0..64 {
            let mut mix = Mix(seed);
            let made = || {
                let mut c = pictures(rules, 12);
                // Half the pictures owe nothing (they are partners), and 13's analysis is below the quality and owed (a
                // malformed row: no picture's, so no batch's).
                for id in (2..=12).step_by(2) {
                    c.conn.execute("DELETE FROM pairs_owed WHERE file_id = ?1", [id]).unwrap();
                }
                add_file(&c, 13, "m.jpg");
                store_for(&mut c, 13, &picture([13; 32], pg_model::pdq::MIN_QUALITY - 1));
                c.conn.execute("INSERT INTO pairs_owed(file_id) VALUES (13)", []).unwrap();
                c
            };
            let (mut new, mut old) = (made(), made());
            let work = new.pairing_work().unwrap();
            let owners: Vec<usize> = (0..work.owed.len()).filter(|&i| work.owed.get(i) == Some(&true)).collect();
            // Repeated partners, partners that are owners, repeated pairs at other distances.
            let n = work.ids.len();
            let pairs: Vec<(usize, usize, u32)> = (0..mix.below(24))
                .map(|_| {
                    let a = mix.below(n);
                    let b = (a + 1 + mix.below(n - 1)) % n;
                    (a, b, u32::try_from(mix.below(32)).unwrap())
                })
                .collect();
            let changes: Vec<(i64, usize)> =
                (0..mix.below(5)).map(|_| (i64::try_from(mix.below(13) + 1).unwrap(), mix.below(CHANGES))).collect();
            let by_hand = mix.below(2) == 0;
            for c in [&mut new, &mut old] {
                for &(id, which) in &changes {
                    change(c, id, which);
                }
                if by_hand {
                    c.conn.execute("UPDATE meta SET value = value + 1 WHERE key = 'pairing_epoch'", []).unwrap();
                }
            }
            write_batch(&mut new, &work, &owners, &pairs).unwrap();
            oracle_write(&mut old, &work, &owners, &pairs);
            let all_pairs = |c: &Catalog| -> Vec<(i64, i64, i64)> {
                raw::rows_of(c, "SELECT a, b, distance FROM pdq_pairs ORDER BY a, b", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
            };
            let context = format!("seed {seed}: changes {changes:?}, pairs {pairs:?}");
            assert_eq!(all_pairs(&new), all_pairs(&old), "{context}");
            assert_eq!(state_owed(&new), state_owed(&old), "{context}");
        }
    }

    under_both_rules!(
        the_batch_writer_writes_what_the_row_by_row_guard_did,
        the_batch_writer_writes_what_the_row_by_row_guard_did_windows,
        the_batch_writer_writes_what_the_row_by_row_guard_did_posix
    );

    fn a_new_pairing_version_owes_the_eligible_analyses_again_and_drops_the_pairs(rules: PathRules) {
        let mut c = two_files(rules);
        store_for(&mut c, 1, &picture([1; 32], 100));
        store_for(&mut c, 2, &picture([2; 32], pg_model::pdq::MIN_QUALITY - 1));
        c.conn.execute("DELETE FROM pairs_owed", []).unwrap();
        raw::add_pdq_pair(&c, 1, 2, 5);
        // The same version: nothing changes.
        crate::lifecycle::set_pairing_version(&c.conn, pg_model::pdq::PAIRING_VERSION).unwrap();
        assert_eq!(pair_distance(&c, 1, 2), Some(5));
        // Another version: the eligible analysis owes its pairs, the ineligible one owes nothing, and the pairs go.
        crate::lifecycle::set_pairing_version(&c.conn, pg_model::pdq::PAIRING_VERSION + 1).unwrap();
        assert_eq!(pair_distance(&c, 1, 2), None);
        assert_eq!((owes(&c, 1), owes(&c, 2)), (true, false));
    }

    under_both_rules!(
        a_new_pairing_version_owes_the_eligible_analyses_again_and_drops_the_pairs,
        a_new_pairing_version_owes_the_eligible_analyses_again_and_drops_the_pairs_windows,
        a_new_pairing_version_owes_the_eligible_analyses_again_and_drops_the_pairs_posix
    );

    /// [`PairingWork`]'s columns, to compare two reads.
    fn columns(w: &crate::PairingWork) -> (Vec<FileId>, Vec<bool>, Vec<[pg_model::pdq::Hash; 4]>, usize) {
        (w.ids.clone(), w.owed.clone(), w.hashes.clone(), w.legacy)
    }

    /// In memory, and on a file, where the pass reads on a connection of its own (`Catalog::reader`): the same rows,
    /// including the commits made just before it opened.
    fn pairing_work_lists_the_hashed_files_with_an_identity_in_one_read(rules: PathRules) {
        for on_file in [false, true] {
            let dir = on_file.then(|| {
                pg_testkit::run_folder("scratch_catalog", &format!("pairing-work-{}", crate::types::rules_name(rules)))
                    .unwrap()
            });
            let path = dir.as_ref().map_or_else(|| std::path::PathBuf::from(":memory:"), |d| d.join("catalog.sqlite"));
            pairing_work_at(rules, &path);
            if let Some(dir) = dir {
                std::fs::remove_dir_all(&dir).unwrap();
            }
        }
    }

    fn pairing_work_at(rules: PathRules, path: &std::path::Path) {
        let (mut c, _) = crate::test_support::one_file_at(rules, path);
        add_file(&c, 2, "b.jpg");
        add_file(&c, 3, "c.jpg");
        add_file(&c, 4, "d.jpg");
        add_file(&c, 5, "e.jpg");
        add_file(&c, 6, "f.jpg");
        add_file(&c, 7, "g.jpg");
        add_file(&c, 8, "h.jpg");
        add_file(&c, 9, "i.jpg");
        store_for(&mut c, 1, &picture([1; 32], 100));
        store_for(&mut c, 2, &picture([2; 32], 100));
        c.conn.execute("DELETE FROM pairs_owed WHERE file_id = 2", []).unwrap();
        // A failure: no hashes at all.
        let failed = item_of(&c, 3);
        analyzed(&mut c, &failed);
        // Legacy: a reliable quality but no stored hashes, and so no pairs owed (an older build's analysis, or one
        // whose hashes failed the migration's check).
        store_for(&mut c, 4, &picture([4; 32], 100));
        c.conn.execute("UPDATE analysis SET pdq_dihedral = NULL WHERE file_id = 4", []).unwrap();
        c.conn.execute("DELETE FROM pairs_owed WHERE file_id = 4", []).unwrap();
        // Missing, and pending: partners like any other (their analyses are stored), though they owe their pairs too.
        store_for(&mut c, 5, &picture([5; 32], 100));
        c.conn.execute("UPDATE files SET missing = 1 WHERE id = 5", []).unwrap();
        store_for(&mut c, 6, &picture([6; 32], 100));
        c.mark_pending(&[FileId::new(6)]).unwrap();
        // Displaced (no identity, as `ScanWriter::upsert_file` leaves a row whose identity is another file's): left out.
        store_for(&mut c, 7, &picture([7; 32], 100));
        c.conn
            .execute("UPDATE files SET volume = NULL, file_ref = NULL, missing = 1, unpaired = 1 WHERE id = 7", [])
            .unwrap();
        // Legacy rows of a missing and a pending file: not counted, as `gallery import --again` does not reach them.
        for id in [8, 9] {
            store_for(&mut c, id, &picture([8; 32], 100));
            c.conn.execute("UPDATE analysis SET pdq_dihedral = NULL WHERE file_id = ?1", [id]).unwrap();
            c.conn.execute("DELETE FROM pairs_owed WHERE file_id = ?1", [id]).unwrap();
        }
        c.conn.execute("UPDATE files SET missing = 1 WHERE id = 8", []).unwrap();
        c.mark_pending(&[FileId::new(9)]).unwrap();
        let work = c.pairing_work().unwrap();
        let read = c.reader(1024).unwrap().pairing_work().unwrap();
        assert_eq!(columns(&read), columns(&work), "the reader reads what the catalog's connection does");
        let ids: Vec<i64> = work.ids.iter().map(|id| id.get()).collect();
        assert_eq!(ids, vec![1, 2, 5, 6]);
        // Files 5 and 6 owe theirs, but one is missing and the other pending: only partners until they are back.
        assert_eq!(work.owed, vec![true, false, false, false]);
        let hashes = [1, 2, 5, 6].map(|b| [pg_model::pdq::Hash([b; 32]); 4]);
        assert_eq!(work.hashes, hashes);
        assert_eq!(work.legacy, 1);
        // Only the owed pictures the pass can make (those `owed` marks) are counted and open the pass; the others' rows
        // stay for their return.
        assert_eq!(c.owed_pairs().unwrap(), work.owed.iter().filter(|&&o| o).count());
        assert_eq!(c.owed_pairs().unwrap(), 1);
        assert!(c.has_owed_pairs().unwrap());
        assert_eq!(state_owed(&c), vec![1, 5, 6, 7]);
        c.conn.execute("DELETE FROM pairs_owed WHERE file_id = 1", []).unwrap();
        assert_eq!(c.owed_pairs().unwrap(), 0);
        assert!(!c.has_owed_pairs().unwrap());
    }

    under_both_rules!(
        pairing_work_lists_the_hashed_files_with_an_identity_in_one_read,
        pairing_work_lists_the_hashed_files_with_an_identity_in_one_read_windows,
        pairing_work_lists_the_hashed_files_with_an_identity_in_one_read_posix
    );

    /// The load as it was before it read its id sets (the pairing plan's second round, step 8): one statement joining
    /// each analysis to its file and its owed row. The load must read what this does.
    const JOINED: &str = "SELECT a.file_id, o.file_id IS NOT NULL AND f.missing = 0 AND f.pending = 0, a.pdq_dihedral,
             f.missing = 0 AND f.pending = 0, a.pdq_quality
         FROM analysis a CROSS JOIN files f LEFT JOIN pairs_owed o ON o.file_id = a.file_id
         WHERE f.id = a.file_id AND f.volume IS NOT NULL AND a.pdq_quality >= ?1
         ORDER BY a.file_id";

    /// The load's columns, its qualities and its legacy count.
    type Loaded = (Vec<FileId>, Vec<bool>, Vec<[pg_model::pdq::Hash; 4]>, Vec<u8>, usize);

    fn loaded(w: &PairingWork) -> Loaded {
        (w.ids.clone(), w.owed.clone(), w.hashes.clone(), w.quality.clone(), w.legacy)
    }

    /// [`JOINED`]'s rows as the load makes its columns of them.
    fn joined(c: &Catalog) -> Loaded {
        let mut out: Loaded = Default::default();
        let mut stmt = c.conn.prepare(JOINED).unwrap();
        let mut rows = stmt.query([pg_model::pdq::MIN_QUALITY]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let Some(blob) = r.get_ref(2).unwrap().as_blob_or_null().unwrap() else {
                if r.get::<_, bool>(3).unwrap() {
                    out.4 += 1;
                }
                continue;
            };
            out.0.push(FileId::new(r.get(0).unwrap()));
            out.1.push(r.get(1).unwrap());
            out.2.push(pg_model::pdq::from_blob(blob).unwrap());
            out.3.push(r.get(4).unwrap());
        }
        out
    }

    /// Random catalogs: sparse ids and two far past the others (the id sets are sorted lists, sized by their rows and
    /// not by the largest id); analyses that are pictures, below the quality, failures, legacy rows, or none; files
    /// present, missing, pending (present or missing) or displaced; owed rows kept or not, some of them of analyses
    /// that are no picture; and an analysis with no file at all (made with the foreign keys off), which neither
    /// read includes. In memory, and on a file, where the reader's load is also split into parts on connections of
    /// their own (`Reader::pairing_work_split`): one part, a few, and more than there are pictures.
    fn the_load_reads_what_the_joined_statement_did(rules: PathRules) {
        for seed in 0..32 {
            let (c, nofile) = random_catalog(rules, seed, std::path::Path::new(":memory:"));
            let work = c.pairing_work().unwrap();
            assert_eq!(loaded(&work), joined(&c), "seed {seed}");
            assert!(!work.ids.contains(&FileId::new(nofile)), "seed {seed}: an analysis with no file is no picture");
        }
        let dir = pg_testkit::run_folder("scratch_catalog", &format!("load-split-{}", crate::types::rules_name(rules)))
            .unwrap();
        for seed in 0..8 {
            let path = dir.join(format!("catalog-{seed}.sqlite"));
            let (c, _) = random_catalog(rules, seed, &path);
            let expected = joined(&c);
            for threads in [1, 2, 3, 5, 64] {
                let work = c.reader(1024).unwrap().pairing_work_split(threads, &|| {}).unwrap();
                assert_eq!(loaded(&work), expected, "seed {seed}, {threads} threads");
            }
            drop(c);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A random catalog of [`the_load_reads_what_the_joined_statement_did`] at `path`, and the id of its analysis with no
    /// file.
    fn random_catalog(rules: PathRules, seed: u64, path: &std::path::Path) -> (Catalog, i64) {
        {
            let mut mix = Mix(seed);
            let (mut c, first) = crate::test_support::one_file_at(rules, path);
            let mut ids = vec![first];
            let mut next = first;
            for _ in 0..40 {
                next += 1 + i64::try_from(mix.below(4)).unwrap();
                ids.push(next);
            }
            ids.extend([1 << 40, (1 << 62) + 3]);
            for &id in ids.iter().skip(1) {
                add_file(&c, id, "x.jpg");
            }
            for (k, &id) in ids.iter().enumerate() {
                let hash = [u8::try_from(k % 250).unwrap() + 1; 32];
                match mix.below(6) {
                    0 | 1 => store_for(&mut c, id, &picture(hash, 100)),
                    2 => store_for(&mut c, id, &picture(hash, pg_model::pdq::MIN_QUALITY - 1)),
                    3 => {
                        let item = item_of(&c, id);
                        analyzed(&mut c, &item);
                    }
                    4 => {
                        store_for(&mut c, id, &picture(hash, 100));
                        c.conn.execute("UPDATE analysis SET pdq_dihedral = NULL WHERE file_id = ?1", [id]).unwrap();
                    }
                    _ => {}
                }
                let state = match mix.below(6) {
                    0 => Some("UPDATE files SET missing = 1 WHERE id = ?1"),
                    1 => Some("UPDATE files SET pending = 1 WHERE id = ?1"),
                    2 => Some("UPDATE files SET missing = 1, pending = 1 WHERE id = ?1"),
                    3 => {
                        Some("UPDATE files SET volume = NULL, file_ref = NULL, missing = 1, unpaired = 1 WHERE id = ?1")
                    }
                    _ => None,
                };
                if let Some(sql) = state {
                    c.conn.execute(sql, [id]).unwrap();
                }
                let owed = if mix.below(3) == 0 {
                    "DELETE FROM pairs_owed WHERE file_id = ?1"
                } else {
                    "INSERT OR IGNORE INTO pairs_owed(file_id) SELECT file_id FROM analysis WHERE file_id = ?1"
                };
                c.conn.execute(owed, [id]).unwrap();
            }
            c.conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
            c.conn
                .execute(
                    "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, pdq_quality, pdq_dihedral)
                     VALUES (?1, 1, 10, 100, 0, 'image', 0, 100, zeroblob(128))",
                    [next + 1],
                )
                .unwrap();
            c.conn.execute("INSERT INTO pairs_owed(file_id) VALUES (?1)", [next + 1]).unwrap();
            c.conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
            (c, next + 1)
        }
    }

    under_both_rules!(
        the_load_reads_what_the_joined_statement_did,
        the_load_reads_what_the_joined_statement_did_windows,
        the_load_reads_what_the_joined_statement_did_posix
    );

    /// Each of the load's id sets is read from an index alone, never from `files`' rows.
    #[test]
    fn the_loads_id_sets_are_read_from_indexes_alone() {
        let (c, _) = one_file(PathRules::Windows);
        for sql in [crate::work::WITH_IDENTITY, crate::work::PRESENT, crate::work::PRESENT_PENDING] {
            let plan: Vec<String> =
                raw::rows_of(&c, &format!("EXPLAIN QUERY PLAN {sql}"), [], |r| r.get::<_, String>(3));
            assert!(plan.len() == 1 && plan.iter().all(|p| p.contains("COVERING INDEX")), "{sql}: {plan:?}");
        }
    }

    /// A picture whose stored hashes are not 4 orientations' (`pg_model::pdq::from_blob`) fails the load, naming the
    /// file, rather than pairing what it can.
    #[test]
    fn a_malformed_blob_fails_the_load() {
        let (mut c, id) = one_file(PathRules::Windows);
        store_for(&mut c, id, &picture([1; 32], 100));
        c.conn.execute("UPDATE analysis SET pdq_dihedral = zeroblob(100) WHERE file_id = ?1", [id]).unwrap();
        let error = c.pairing_work().map(|_| ()).unwrap_err().to_string();
        assert!(error.contains(&format!("file {id} has malformed stored hashes")), "{error}");
    }

    fn set_rating(c: &Catalog, id: i64, n: u8) {
        let stars = Rating::new(n).filter(|r| r.get() > 0);
        c.set_rating(FileId::new(id), stars, false).unwrap();
    }

    /// The import's sync of the file's rating, in a transaction of its own.
    fn sync_file(c: &mut Catalog, id: i64, in_file: Option<Rating>) {
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().sync_file_rating(FileId::new(id), in_file).unwrap();
        tx.commit().unwrap();
    }

    fn three_way_rating_sync(rules: PathRules) {
        let (mut c, id) = one_file(rules);
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
        c.set_rating(FileId::new(id), Rating::new(3), true).unwrap();
        sync_file(&mut c, id, Rating::new(1));
        assert_eq!(raw::rating(&c, id), Some(3));
    }

    under_both_rules!(three_way_rating_sync, three_way_rating_sync_windows, three_way_rating_sync_posix);

    /// A file rated before its first import: the rating is queued for the file (its format isn't known yet), and the
    /// import, which finds no rating in the file, keeps the catalog's while the write is queued.
    fn a_rating_made_before_the_import_is_kept_and_queued(rules: PathRules) {
        let (mut c, id) = one_file(rules);
        c.set_rating(FileId::new(id), Rating::new(4), true).unwrap();
        assert_eq!(raw::queued(&c, id, "rating"), Some(4), "queued with no analysis");
        sync_file(&mut c, id, None);
        assert_eq!(raw::rating(&c, id), Some(4));
        assert_eq!(raw::queued(&c, id, "rating"), Some(4));
    }

    under_both_rules!(
        a_rating_made_before_the_import_is_kept_and_queued,
        a_rating_made_before_the_import_is_kept_and_queued_windows,
        a_rating_made_before_the_import_is_kept_and_queued_posix
    );
}
