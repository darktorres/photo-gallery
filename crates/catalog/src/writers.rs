//! The writes of the scan, of the import (analyses, failures and the ratings they carry) and of the borders pass. Each
//! writer borrows the transaction it is made in, so it is dropped before that commits. Uses `types` and `error`; its
//! SQL macros are from `sql`.

use anyhow::Context;
use rusqlite::{OptionalExtension, Statement, Transaction, params};

use pg_model::analysis::Analyzed;
use pg_model::detection::Detection;
use pg_model::ids::{FileId, FileIdentity, FolderId};
use pg_model::rating::Rating;
use pg_model::rules::{Path as RulePath, PathRules};

use crate::error::Result;
use crate::types::{
    DELETE_FILE_RATING, DELETE_SIDES, Folder, Hit, Recorded, SET_FILE_RATING, ScannedFile, WorkItem, under_params,
};

// The passes' writes: one writer per transaction, each statement prepared once for it rather than looked up in
// rusqlite's statement cache per row (16 entries, which the import's statements alone would crowd). A writer
// borrows its transaction, so it is dropped before the transaction commits.

/// Write a file as a new row (or, on its identity, the same file's changed fields): unpaired, as `ScanWriter::file` says.
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

    /// Record borders looked for (`Catalog::pending_borders`; `None`: not looked
    /// for), replacing what was there, unless the file changed meanwhile (the import then analyzes it again, and the
    /// borders are due again) or a write of the app's into it is in progress (they stay due).
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
            let color = s.color.map(|[r, g, b]| (i64::from(r) << 16) | (i64::from(g) << 8) | i64::from(b));
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
