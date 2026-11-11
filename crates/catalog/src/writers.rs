//! The writes of the scan, of the import (analyses, failures and the ratings they carry), of the borders pass and of
//! the passes' failure ledger. Each writer borrows the transaction it is made in, so it is dropped before that commits.
//! The ledger's set-based writes are functions over a connection ([`record_failures_on`], [`clear_failures_on`]), which
//! `Catalog::record_failures`/`clear_failures` run in a transaction of their own and the [`LedgerWriter`] in its
//! caller's. Uses `types`, `ledger::failure` and `error`; its SQL macros are from `sql`.

use anyhow::{Context, anyhow};
use rusqlite::{Connection, OptionalExtension, Statement, Transaction, params};

use pg_model::analysis::Analyzed;
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
    /// The writer of the walk `scan` of `root`, in `tx`: the walk's lease is renewed (the heartbeat of each batch).
    pub(crate) fn new(
        tx: &'tx Transaction,
        scan: ScanId,
        rules: PathRules,
        root: &RulePath,
    ) -> Result<ScanWriter<'tx>> {
        tx.prepare_cached("UPDATE scan_leases SET heartbeat = unixepoch() WHERE id = ?1")?.execute([scan.get()])?;
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
            // A new row is unpaired: the pairing at the end of the scan decides what it is (S-17.7 decision 4). A row
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
    /// didn't see (`Catalog::mark_missing`). One already recorded keeps its serial, so it is not news again. Nothing is
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

    /// Record a file the scan found, by identity (S-17.7 decision 2). A file moved or renamed keeps its row, and so
    /// does one whose `created` is unknown on either side; a missing row of another root counts as moved here
    /// (decision 5), a present one is [`Recorded::Foreign`]. A hit whose `created` differs is a different file: the row
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
    /// way: its listing gives no content to analyze, and a row that is there is kept (S-17.7 decision 9).
    pub fn seen_identity(&mut self, identity: FileIdentity) -> Result<()> {
        self.seen_identity.execute(params![
            identity.volume.cast_signed(),
            identity.file_index.cast_signed(),
            self.scan
        ])?;
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
    pub(crate) fn new(tx: &'tx Transaction, import: i64) -> Result<AnalysisWriter<'tx>> {
        Ok(AnalysisWriter {
            import,
            current: tx.prepare(crate::current::IS_CURRENT)?,
            delete_pairs: tx.prepare("DELETE FROM pdq_pairs WHERE a = ?1 OR b = ?1")?,
            analysis: tx.prepare(
                "INSERT OR REPLACE INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error,
                    file_hash, pixel_hash, format, orientation, width, height, pdq, pdq_quality, thumbhash, frames,
                    taken, taken_utc, duration_ms, poster_ms, borders_due, pdq_dihedral)
                 VALUES (?1, ?2, ?3, ?4, ?19, ?20, unixepoch(), NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                         CASE WHEN ?16 THEN unixepoch(?15, 'unixepoch', 'localtime') ELSE ?15 END,
                         CASE WHEN ?16 THEN ?15 END, ?17, ?18, 0, ?21)",
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
            queued_rating: tx.prepare(concat!(
                "SELECT EXISTS(SELECT 1 FROM file_writes WHERE file_id = ?1 AND kind = '",
                pg_model::write_rating!(),
                "')"
            ))?,
            set_rating: tx.prepare(SET_RATING)?,
            delete_rating: tx.prepare(DELETE_RATING)?,
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
        crate::current::query_current(&mut self.current, item.id, &item.listed())
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
            &a.file_hash.0[..],
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
    use crate::{Catalog, Recorded, ScannedFile, WorkItem, queue};
    use pg_analysis::analyze::IMPORT_VERSION;
    use pg_model::analysis::{Analysis, Taken, ThumbHash};
    use pg_model::detection::Detection;
    use pg_model::ids::{FileId, FileIdentity, FolderId, ScanId};
    use pg_model::media::Ext;
    use pg_model::name::Name;
    use pg_model::orientation::Orientation;
    use pg_model::rating::Rating;
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;
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
        let prior = c.prior_states(&p(rules, r"D:\x"), &[identity]).unwrap().remove(0);
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
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
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
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
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
            tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
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
