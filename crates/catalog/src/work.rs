//! The work lists the passes read: the import's pending files, the borders' due analyses, the PDQ hashes the
//! duplicates are matched by, and files made due again (methods of `Catalog`); and a statement's rows as a `WorkList`,
//! each folder's path read once (`work_list`). Its SQL macros are from `sql`.

use std::collections::BTreeSet;

use anyhow::anyhow;
use pg_model::ids::FileId;
use pg_model::path::WorkList;
use pg_model::turns::Turns;
use rusqlite::{Connection, OptionalExtension};

use crate::core::{Catalog, ReadOnly, Reader};
use crate::error::{Error, Result};
use crate::transaction::write_transaction;
use crate::types::{FOLDER_PATH, PairingWork, PlaceholderWork, VolumeSet, WorkItem, folder_path, pairing_marks};
use pg_model::analysis::PLACEHOLDER_VERSION;

/// `Catalog::volumes`' query: a loose index scan over `files`'s `(volume, file_ref)` index, one step a volume (the
/// `min` of the volumes above the last one found), not a read of every file. A displaced row (no volume) is none. The
/// thumbnail cache's `catalog_volumes` is the same statement over the attached catalog.
const VOLUMES: &str = "WITH RECURSIVE v(volume) AS (
         SELECT min(volume) FROM files WHERE volume IS NOT NULL
         UNION ALL
         SELECT (SELECT min(volume) FROM files WHERE volume > v.volume) FROM v WHERE v.volume IS NOT NULL
     )
     SELECT volume FROM v WHERE volume IS NOT NULL";

impl Catalog {
    /// The volumes the files `ids` are on, as the catalog stores them (the ids that are not in the catalog are none). A job
    /// that changes those files holds the lanes of those volumes while it does.
    pub fn volumes_of(&self, ids: &[FileId]) -> Result<BTreeSet<i64>> {
        let mut stmt = self.conn.prepare_cached("SELECT volume FROM files WHERE id = ?1")?;
        let mut volumes = BTreeSet::new();
        for id in ids {
            let found: Option<i64> = stmt.query_row([id.get()], |row| row.get(0)).optional()?;
            volumes.extend(found);
        }
        Ok(volumes)
    }

    /// The distinct volumes the catalog holds files on, ascending (the non-NULL `files.volume`s). The cost is one index
    /// probe a volume, whatever the number of files: a plain `SELECT DISTINCT volume` would read the whole index.
    pub fn volumes(&self) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare_cached(VOLUMES)?;
        Ok(stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }
}

/// `Catalog::pending_import`'s query, for `volumes`.
pub(crate) fn pending_import_sql(retry_errors: bool, volumes: &VolumeSet) -> String {
    let and = volumes.and_in("f.volume");
    if retry_errors {
        // A UNION, as an OR would scan every file; the failures are read from their index first. The set's filter is in
        // both branches.
        format!(
            "{}{and} UNION SELECT {}, f.volume AS volume, f.file_ref & 0xFFFFFFFFFFFF AS record
             FROM analysis a CROSS JOIN files f
             WHERE a.error IS NOT NULL AND f.id = a.file_id AND f.missing = 0{and} ORDER BY volume, record",
            pending_files!(),
            work_columns!(),
        )
    } else {
        format!("{}{and} ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF", pending_files!())
    }
}

/// The due-borders rows joined for `volumes`: from the queue for every volume (the due analyses are few), and from the
/// set's files for some, so a lane probes the queue by key rather than walking it.
fn borders_from(volumes: &VolumeSet) -> String {
    match volumes {
        VolumeSet::All => "FROM analysis a CROSS JOIN files f WHERE a.borders_due = 1 AND f.id = a.file_id".to_string(),
        VolumeSet::Only(_) => format!(
            "FROM files f CROSS JOIN analysis a WHERE a.file_id = f.id AND a.borders_due = 1{}",
            volumes.and_in("f.volume")
        ),
    }
}

/// The owed-hashes rows joined for `volumes`, with the same choice of driver as [`borders_from`].
fn hashes_from(volumes: &VolumeSet) -> String {
    match volumes {
        VolumeSet::All => {
            "FROM hashes_owed h CROSS JOIN analysis a CROSS JOIN files f WHERE h.file_id = a.file_id AND f.id = a.file_id"
                .to_string()
        }
        VolumeSet::Only(_) => format!(
            "FROM files f CROSS JOIN hashes_owed h CROSS JOIN analysis a WHERE h.file_id = f.id AND a.file_id = f.id{}",
            volumes.and_in("f.volume")
        ),
    }
}

/// The work list of a statement whose rows start with `work_columns!()` and number at most `bound`, read in
/// `conn`'s transaction: the items in a `Vec` of that capacity, each folder's path read once.
pub(crate) fn work_list(conn: &Connection, stmt: &mut rusqlite::Statement, bound: usize) -> Result<WorkList<WorkItem>> {
    work_list_of(conn, stmt.query([])?, bound)
}

/// [`work_list`] of the rows a statement gave (`rows`, from its parameters or none).
fn work_list_of(conn: &Connection, rows: rusqlite::Rows<'_>, bound: usize) -> Result<WorkList<WorkItem>> {
    work_list_mapped(conn, rows, bound, |item, _| Ok(item))
}

/// [`work_list`]'s rows, each made into the item `make` gives from its work item and its row (`make` reads the columns
/// after `work_columns!()` by index: the list reads the first seven).
pub(crate) fn work_list_mapped<T>(
    conn: &Connection,
    mut rows: rusqlite::Rows<'_>,
    bound: usize,
    make: impl Fn(WorkItem, &rusqlite::Row<'_>) -> Result<T>,
) -> Result<WorkList<T>> {
    let (mut list, mut folder) = (WorkList::default(), None);
    while let Some(r) = rows.next()? {
        if folder.is_none() {
            // Only once there is work: a round with nothing to do reads nothing more.
            list = WorkList::with_capacity(bound);
            folder = Some(conn.prepare_cached(FOLDER_PATH)?);
        }
        let Some(folder) = folder.as_mut() else { unreachable!("made above") };
        let path = list.file(r.get(1)?, r.get_ref(2)?.as_blob()?, |arena| folder_path(folder, r, arena))?;
        let item = WorkItem {
            id: FileId::new(r.get(0)?),
            path,
            size: r.get(3)?,
            mtime: r.get(4)?,
            rotation: Turns::new(r.get(5)?),
            kind: crate::types::kind_at(r, 6)?,
        };
        list.items.push(make(item, r)?);
    }
    Ok(list)
}

/// The placeholder work (`Catalog::pending_placeholders`, `Catalog::owed_placeholders`): the present, non-pending files
/// whose analysis is good and whose placeholder is not at their identity and the current version (`?1`). A file the app
/// is writing is left out until the write is done, and a file the Thumbs pass failed at its identity is left out until
/// it changes or Retry clears its row.
const OWED_PLACEHOLDERS: &str = "FROM files f
     JOIN analysis a ON a.file_id = f.id AND a.error IS NULL
     LEFT JOIN placeholders p ON p.file_id = f.id AND p.size = f.size AND p.mtime = f.mtime
         AND p.rotation = f.rotation AND p.version = ?1
     WHERE f.missing = 0 AND f.pending = 0 AND p.file_id IS NULL
       AND NOT EXISTS (SELECT 1 FROM file_writes w WHERE w.file_id = f.id AND w.temp IS NOT NULL)
       AND NOT EXISTS (SELECT 1 FROM file_failures x WHERE x.file_id = f.id AND x.pass = 'thumbs'
           AND x.size = f.size AND x.mtime = f.mtime AND x.rotation = f.rotation)";

/// The item of file `id` as the work lists have it (`work_columns!`), or `None` when it isn't in the catalog.
pub(crate) fn work_item(conn: &Connection, id: FileId) -> Result<Option<WorkItem>> {
    let mut stmt = conn.prepare_cached(concat!("SELECT ", work_columns!(), " FROM files f WHERE f.id = ?1"))?;
    Ok(work_list_of(conn, stmt.query([id.get()])?, 1)?.items.pop())
}

/// What [`Catalog::pairing_work`] reads: the analyses of a reliable quality in a range of file ids (all of them, or one
/// part of a split read), scanned in file-id order (`analysis`'s rowid order, so the `ORDER BY` costs no sort). Which of them are pictures, and which owe their pairs, comes from four id
/// sets read before it in the same snapshot, each from an index alone ([`WITH_IDENTITY`], [`PRESENT`],
/// [`PRESENT_PENDING`], [`OWED_IDS`]), and walked beside the scan: no lookup per row. A row with no stored hashes is a
/// legacy one, counted in the same scan.
///
/// A picture is the analysis of a file with an identity. A pair is a fact about two stored analyses, so a missing or
/// pending file's analysis is a partner like any other: a picture paired while its partner's file was away is never
/// owed again when the file comes back without a new analysis (a rescan that finds it, a rebind, a turn undone, an
/// mtime put back, an older build's import version), and a pair the pass left out then would never be made. A pair
/// with a missing file is hidden where pairs are shown (`present_pdq_pairs`), and every new analysis drops its file's
/// pairs and owes them again. A displaced row (no identity, `volume IS NULL`) is left out: its analysis stays only until
/// a replacement deletes it, its pairs could never be shown, and the file's new row would pair with it. So is an
/// analysis with no `files` row at all, which the foreign key's cascade leaves none of.
pub(crate) const PICTURES: &str = "SELECT file_id, pdq_dihedral, pdq_quality FROM analysis
     WHERE file_id BETWEEN ?2 AND ?3 AND pdq_quality >= ?1 ORDER BY file_id";
/// The files with an identity, from `UNIQUE(volume, file_ref)`'s index alone (NULLs sort first, so this is one range).
pub(crate) const WITH_IDENTITY: &str = "SELECT id FROM files WHERE volume IS NOT NULL";
/// The present files, and the present ones the import has yet to analyze: those of the first and not the second make
/// their pairs ([`OWED`]'s rule), and only they count as legacy.
pub(crate) const PRESENT: &str = "SELECT id FROM files INDEXED BY files_present WHERE missing = 0";
pub(crate) const PRESENT_PENDING: &str =
    "SELECT id FROM files INDEXED BY files_to_import WHERE missing = 0 AND pending = 1";
/// The pictures that owe their pairs.
const OWED_IDS: &str = "SELECT file_id FROM pairs_owed ORDER BY file_id";

/// A set of file ids read by one statement, sorted: a sorted list rather than a bitmap, so a catalog's largest id does
/// not size it.
struct IdSet(Vec<i64>);

impl IdSet {
    fn read(tx: &Connection, sql: &str) -> Result<IdSet> {
        let mut ids: Vec<i64> = tx.prepare(sql)?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        ids.sort_unstable();
        Ok(IdSet(ids))
    }

    /// A walk of the set, asked about ids that never decrease.
    fn walk(&self) -> IdWalk<'_> {
        IdWalk { rest: &self.0 }
    }

    /// A walk of the set's ids from `id` on.
    fn walk_from(&self, id: i64) -> IdWalk<'_> {
        IdWalk { rest: self.0.get(self.0.partition_point(|&x| x < id)..).unwrap_or_default() }
    }
}

/// [`IdSet::walk`]: each question skips the ids below the one asked, so a walk beside a scan costs one step a member.
struct IdWalk<'s> {
    rest: &'s [i64],
}

impl IdWalk<'_> {
    /// Whether `id` is in the set: `id` is at least every id asked before.
    fn has(&mut self, id: i64) -> bool {
        while let Some((&first, rest)) = self.rest.split_first()
            && first < id
        {
            self.rest = rest;
        }
        self.rest.first() == Some(&id)
    }
}

/// A picture's stored hashes, as the load's column holds them.
type Hashes = [pg_model::pdq::Hash; pg_model::pdq::ORIENTATIONS];

/// Where a part of the load's scan puts its pictures, in id order.
trait Columns {
    fn put(&mut self, id: i64, hashes: Hashes, quality: u8);
}

/// The load's columns, grown by each picture: a load in one part, with nothing to fill first or move after.
struct Grown {
    ids: Vec<i64>,
    hashes: Vec<Hashes>,
    quality: Vec<u8>,
}

impl Grown {
    fn with_capacity(n: usize) -> Grown {
        Grown { ids: Vec::with_capacity(n), hashes: Vec::with_capacity(n), quality: Vec::with_capacity(n) }
    }
}

impl Columns for Grown {
    fn put(&mut self, id: i64, hashes: Hashes, quality: u8) {
        self.ids.push(id);
        self.hashes.push(hashes);
        self.quality.push(quality);
    }
}

/// A part's own slices of the load's columns (a split load, each part on a thread of its own), as long as its files
/// with an identity are many (the most pictures it can hold); `filled` slots are its pictures.
struct Slots<'w> {
    ids: &'w mut [i64],
    hashes: &'w mut [Hashes],
    quality: &'w mut [u8],
    filled: usize,
}

impl Columns for Slots<'_> {
    fn put(&mut self, id: i64, hashes: Hashes, quality: u8) {
        let at = self.filled;
        let (Some(i), Some(h), Some(q)) = (self.ids.get_mut(at), self.hashes.get_mut(at), self.quality.get_mut(at))
        else {
            unreachable!("a part has a slot for each of its files with an identity, and each picture is one");
        };
        (*i, *h, *q) = (id, hashes, quality);
        self.filled += 1;
    }
}

/// Scans the file ids `lo..=hi` of `analysis` on `conn` (one statement, so one snapshot), putting the analyses of the
/// files in `identity` in `out`; returns the legacy rows' ids, in order.
fn scan_part(conn: &Connection, identity: &IdSet, lo: i64, hi: i64, out: &mut impl Columns) -> Result<Vec<i64>> {
    let mut identity = identity.walk_from(lo);
    let mut stmt = conn.prepare(PICTURES)?;
    let mut rows = stmt.query(rusqlite::params![pg_model::pdq::MIN_QUALITY, lo, hi])?;
    let mut legacy = Vec::new();
    while let Some(r) = rows.next()? {
        let raw: i64 = r.get(0)?;
        if !identity.has(raw) {
            continue;
        }
        let Some(blob) = r.get_ref(1)?.as_blob_or_null()? else {
            legacy.push(raw);
            continue;
        };
        let hashes = pg_model::pdq::from_blob(blob)
            .ok_or_else(|| Error::Other(anyhow!("file {raw} has malformed stored hashes")))?;
        out.put(raw, hashes, r.get(2)?);
    }
    Ok(legacy)
}

/// [`Catalog::pairing_work`], read on `conn` in one transaction, with `analysis` scanned on `opener`'s connections in
/// up to `threads` parts at once (each thread started by `start`), or in `conn`'s transaction alone without one.
///
/// The parts are cut at equal counts of files with an identity. `conn` reads the pairing marks first, so every part's
/// snapshot is that one's or a later one: a change to an analysis between them moves the epoch, and the pass's writes
/// then guard every pair (`PairsWriter::write`), which is what makes pictures read in several snapshots safe. The other
/// id sets are read on `conn` while the parts scan.
fn pairing_work_on(
    conn: &Connection,
    opener: Option<ReadOnly>,
    threads: usize,
    start: &(dyn Fn() + Sync),
) -> Result<PairingWork> {
    let tx = conn.unchecked_transaction()?;
    let marks = pairing_marks(&tx)?;
    let identity = IdSet::read(&tx, WITH_IDENTITY)?;
    let read_sets = || -> Result<[IdSet; 3]> {
        Ok([IdSet::read(&tx, PRESENT)?, IdSet::read(&tx, PRESENT_PENDING)?, IdSet::read(&tx, OWED_IDS)?])
    };
    // Each picture is a file with an identity: the columns never grow.
    let bound = identity.0.len();
    let parts = threads.clamp(1, bound.max(1));
    let (columns, legacy, sets) = match opener {
        Some(opener) if parts > 1 => split_scan(&opener, &identity, parts, start, &read_sets)?,
        _ => {
            let mut columns = Grown::with_capacity(bound);
            let legacy = scan_part(&tx, &identity, i64::MIN, i64::MAX, &mut columns)?;
            (columns, legacy, read_sets()?)
        }
    };
    let [present, pending, owed] = sets;
    // Present and not pending: the pass can make its pairs, and only then is a legacy row counted.
    let (mut present_w, mut pending_w, mut owed_w) = (present.walk(), pending.walk(), owed.walk());
    let owes: Vec<bool> =
        columns.ids.iter().map(|&id| present_w.has(id) && !pending_w.has(id) && owed_w.has(id)).collect();
    let (mut present_w, mut pending_w) = (present.walk(), pending.walk());
    let legacy = legacy.iter().filter(|&&id| present_w.has(id) && !pending_w.has(id)).count();
    tx.commit()?;
    let Grown { ids, hashes, quality } = columns;
    Ok(PairingWork { ids: ids.into_iter().map(FileId::new).collect(), owed: owes, hashes, quality, legacy, marks })
}

/// The scan of [`pairing_work_on`] in `parts` parts at once, each on a connection of `opener`'s and a thread of its own,
/// while `read_sets` reads the other id sets on the caller's: the columns, the legacy rows' ids and those sets. The
/// columns are filled first and each part writes a slice of its own, then the parts are moved together in id order: the
/// columns are never copied whole.
fn split_scan(
    opener: &ReadOnly,
    identity: &IdSet,
    parts: usize,
    start: &(dyn Fn() + Sync),
    read_sets: &dyn Fn() -> Result<[IdSet; 3]>,
) -> Result<(Grown, Vec<i64>, [IdSet; 3])> {
    let bound = identity.0.len();
    // Part k holds the identity ids cuts[k]..cuts[k + 1].
    let cuts: Vec<usize> = (0..=parts).map(|k| k * bound / parts).collect();
    let mut columns = Grown {
        ids: vec![0; bound],
        hashes: vec![[pg_model::pdq::Hash([0; 32]); pg_model::pdq::ORIENTATIONS]; bound],
        quality: vec![0; bound],
    };
    let (sets, scanned) = {
        let mut split = Vec::with_capacity(parts);
        let (mut ids, mut hashes, mut quality) =
            (&mut columns.ids[..], &mut columns.hashes[..], &mut columns.quality[..]);
        for (k, pair) in cuts.windows(2).enumerate() {
            let &[from, to] = pair else { unreachable!("windows of two") };
            let lo = if k == 0 { i64::MIN } else { identity.0.get(from).copied().unwrap_or(i64::MAX) };
            let hi = match identity.0.get(to) {
                Some(&next) if k + 1 < parts => next - 1,
                _ => i64::MAX,
            };
            let (part_ids, more_ids) = std::mem::take(&mut ids).split_at_mut(to - from);
            let (part_hashes, more_hashes) = std::mem::take(&mut hashes).split_at_mut(to - from);
            let (part_quality, more_quality) = std::mem::take(&mut quality).split_at_mut(to - from);
            (ids, hashes, quality) = (more_ids, more_hashes, more_quality);
            split.push((lo, hi, Slots { ids: part_ids, hashes: part_hashes, quality: part_quality, filled: 0 }));
        }
        std::thread::scope(|scope| {
            let handles: Vec<_> = split
                .into_iter()
                .map(|(lo, hi, mut slots)| {
                    scope.spawn(move || -> Result<(usize, Vec<i64>)> {
                        start();
                        let legacy = scan_part(&opener.open()?, identity, lo, hi, &mut slots)?;
                        Ok((slots.filled, legacy))
                    })
                })
                .collect();
            let sets = read_sets();
            let scanned: Vec<Result<(usize, Vec<i64>)>> = handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
                .collect();
            (sets, scanned)
        })
    };
    let sets = sets?;
    let scanned = scanned.into_iter().collect::<Result<Vec<_>>>()?;
    let mut filled = 0;
    let mut legacy = Vec::new();
    for ((part, part_legacy), &from) in scanned.into_iter().zip(&cuts) {
        // The first part is where it belongs already.
        if from != filled {
            columns.ids.copy_within(from..from + part, filled);
            columns.hashes.copy_within(from..from + part, filled);
            columns.quality.copy_within(from..from + part, filled);
        }
        filled += part;
        legacy.extend(part_legacy);
    }
    columns.ids.truncate(filled);
    columns.hashes.truncate(filled);
    columns.quality.truncate(filled);
    Ok((columns, legacy, sets))
}

impl Reader<'_> {
    /// [`Catalog::pairing_work`], on this reader's connection.
    pub fn pairing_work(&self) -> Result<PairingWork> {
        self.pairing_work_split(1, &|| {})
    }

    /// [`Self::pairing_work`] with `analysis` scanned in up to `threads` parts at once, each on a connection of its own
    /// that shares this reader's cache budget, and on a thread started by `start` (the pass's priority). The parts read
    /// in snapshots no older than the pairing marks' (`pairing_work_on` says why that is safe). In memory, one part.
    pub fn pairing_work_split(&self, threads: usize, start: &(dyn Fn() + Sync)) -> Result<PairingWork> {
        let opener = if threads > 1 { self.opener(threads) } else { None };
        pairing_work_on(self.conn(), opener, threads, start)
    }
}

/// The owed pictures the pairing pass can make: those of present, non-pending files (`PairingWork::owed`). A missing
/// or pending one stays owed, and the first pass after its file is back makes its pairs.
const OWED: &str = "FROM pairs_owed o CROSS JOIN files f WHERE f.id = o.file_id AND f.missing = 0 AND f.pending = 0";

impl Catalog {
    /// Files whose analysis is absent or stale (`files.pending`), and with `retry_errors` those that failed, in
    /// on-disk order (volume, then MFT record number), which reads ~3× faster than path order on an HDD.
    /// Without `retry_errors` it's read in that order from `files_to_import`, so nothing pending is one probe. `volumes`
    /// limits the list to those volumes.
    pub fn pending_import(&self, retry_errors: bool, volumes: &VolumeSet) -> Result<WorkList<WorkItem>> {
        // Counted in the same snapshot: an upper bound (a failure may also be pending), each count read from its
        // index alone.
        let tx = self.conn.unchecked_transaction()?;
        let count = |sql: &str| -> Result<usize> { Ok(tx.query_row(sql, [], |r| r.get(0))?) };
        let and = volumes.and_in("f.volume");
        let mut bound = count(&format!("SELECT count(*) FROM files f WHERE f.missing = 0 AND f.pending = 1{and}"))?;
        if retry_errors {
            bound += match volumes {
                VolumeSet::All => count("SELECT count(*) FROM analysis WHERE error IS NOT NULL")?,
                VolumeSet::Only(_) => count(&format!(
                    "SELECT count(*) FROM analysis a CROSS JOIN files f WHERE a.error IS NOT NULL AND f.id = a.file_id{and}"
                ))?,
            };
        }
        let sql = pending_import_sql(retry_errors, volumes);
        let list = work_list(&tx, &mut tx.prepare(&sql)?, bound)?;
        tx.commit()?;
        Ok(list)
    }

    /// Analyses whose borders are due (new ones, and all after a new detector: `set_borders_version`), of
    /// files the import has nothing to do for, in on-disk order. `volumes` limits the list to those volumes.
    pub fn pending_borders(&self, volumes: &VolumeSet) -> Result<WorkList<WorkItem>> {
        let tx = self.conn.unchecked_transaction()?;
        let from = borders_from(volumes);
        // An upper bound: the due analyses of the set.
        let bound: usize = tx.query_row(&format!("SELECT count(*) {from}"), [], |r| r.get(0))?;
        let mut stmt = tx.prepare(&format!(
            "SELECT {} {from} AND f.missing = 0 AND f.pending = 0
               AND NOT EXISTS (SELECT 1 FROM file_failures x WHERE x.file_id = f.id AND x.pass = 'thumbs'
                   AND x.size = f.size AND x.mtime = f.mtime AND x.rotation = f.rotation)
             ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF",
            work_columns!(),
        ))?;
        let list = work_list(&tx, &mut stmt, bound)?;
        drop(stmt);
        tx.commit()?;
        Ok(list)
    }

    /// The analyses the import stored without their duplicate hashes (`hashes_owed`), of present, non-pending files, in
    /// on-disk order: the Hashes pass's work. `volumes` limits the list to those volumes.
    pub fn pending_hashes(&self, volumes: &VolumeSet) -> Result<WorkList<WorkItem>> {
        let tx = self.conn.unchecked_transaction()?;
        let from = hashes_from(volumes);
        // An upper bound: the owed hashes of the set.
        let bound: usize = tx.query_row(&format!("SELECT count(*) {from}"), [], |r| r.get(0))?;
        let mut stmt = tx.prepare(&format!(
            "SELECT {} {from} AND f.missing = 0 AND f.pending = 0
             ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF",
            work_columns!(),
        ))?;
        let list = work_list(&tx, &mut stmt, bound)?;
        drop(stmt);
        tx.commit()?;
        Ok(list)
    }

    /// The files the Placeholders pass has to make a placeholder for (`OWED_PLACEHOLDERS`), in on-disk order. `volumes`
    /// limits the list to those volumes.
    pub fn pending_placeholders(&self, volumes: &VolumeSet) -> Result<WorkList<PlaceholderWork>> {
        let tx = self.conn.unchecked_transaction()?;
        let and = volumes.and_in("f.volume");
        // An upper bound: the present, non-pending files of the set.
        let bound: usize = tx.query_row(
            &format!("SELECT count(*) FROM files f WHERE f.missing = 0 AND f.pending = 0{and}"),
            [],
            |r| r.get(0),
        )?;
        let mut stmt = tx.prepare(&format!(
            "SELECT {}, CASE WHEN f.kind = '{}' THEN coalesce(a.poster_ms, 0) END AS poster_ms {OWED_PLACEHOLDERS}{and}
             ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF",
            work_columns!(),
            pg_model::kind_video!(),
        ))?;
        let list = work_list_mapped(&tx, stmt.query([PLACEHOLDER_VERSION])?, bound, |item, r| {
            Ok(PlaceholderWork { item, poster_ms: r.get::<_, Option<i64>>(7)?.unwrap_or(0) })
        })?;
        drop(stmt);
        tx.commit()?;
        Ok(list)
    }

    /// How many files the Placeholders pass still owes a placeholder (the rows `pending_placeholders` gives).
    pub fn owed_placeholders(&self) -> Result<usize> {
        Ok(self.conn.query_row(&format!("SELECT count(*) {OWED_PLACEHOLDERS}"), [PLACEHOLDER_VERSION], |r| r.get(0))?)
    }

    /// How many present, non-pending files are stored without their duplicate hashes (`hashes_owed`): the status's count
    /// of what duplicates detection, while it was off, left for the Hashes pass.
    pub fn owed_hashes(&self) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM hashes_owed h CROSS JOIN files f
             WHERE f.id = h.file_id AND f.missing = 0 AND f.pending = 0",
            [],
            |r| r.get(0),
        )?)
    }

    /// The pictures the pairing pass pairs, in file-id order, read in one snapshot: a picture is an analysis with stored
    /// hashes and a reliable quality, of a file with an identity (missing and pending ones too: `PICTURES` says
    /// why). The owed ones the pass can make (a `pairs_owed` row, the file present and not pending) are among them,
    /// with the others (the stored side). One snapshot, because a file that changed between two reads could be owed a
    /// pair that its partner's read never sees. The columns are sized from the files with an identity first, so they
    /// never grow, and each row's hashes are read from the blob in place. The pass reads it
    /// on a [`Reader`] of its own ([`Reader::pairing_work`]).
    pub fn pairing_work(&self) -> Result<PairingWork> {
        pairing_work_on(&self.conn, None, 1, &|| {})
    }

    /// Owe every eligible picture its pairs again, and drop the pairs (`gallery pair --again`): what the pairing pass
    /// then makes is all of them again. Returns how many are owed. The WAL this write leaves is merged into the file
    /// before returning: otherwise the pass's first commits merge it, which took about a second on the 187,804-picture
    /// library (the pass alone takes 0.8 s after a merge).
    pub fn pairs_again(&mut self) -> Result<usize> {
        let tx = write_transaction(&self.conn)?;
        let owed = crate::lifecycle::owe_pairs_again(&tx)?;
        tx.commit()?;
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(owed)
    }

    /// How many pictures owe their pairs: the files the pairing pass has yet to pair (present and not pending).
    pub fn owed_pairs(&self) -> Result<usize> {
        Ok(self.conn.query_row(&format!("SELECT count(*) {OWED}"), [], |r| r.get(0))?)
    }

    /// Whether any file owes its duplicate hashes (`hashes_owed`, present and not pending): one probe. The duplicate
    /// list waits while one does (`NOT_DUPLICATES`).
    pub fn has_owed_hashes(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM hashes_owed h JOIN files ff ON ff.id = h.file_id
             WHERE ff.missing = 0 AND ff.pending = 0)",
            [],
            |r| r.get(0),
        )?)
    }

    /// Whether any picture owes its pairs ([`Catalog::owed_pairs`] above zero): the pairing pass's "nothing to do",
    /// one probe.
    pub fn has_owed_pairs(&self) -> Result<bool> {
        Ok(self.conn.query_row(&format!("SELECT EXISTS(SELECT 1 {OWED})"), [], |r| r.get(0))?)
    }

    /// Make these files due for the import again (`files.pending`), whatever their analysis: after a fix to what
    /// the import reads, which does not bump IMPORT_VERSION. Their next analysis
    /// clears it, as any does (`analysis_added`). Missing files are left as they are. Returns how many it marked.
    pub fn mark_pending(&self, ids: &[FileId]) -> Result<usize> {
        let tx = write_transaction(&self.conn)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::spelled;
    use crate::fixture::{File, insert, raw};
    use crate::test_support::{store_taken, versions};
    use pg_model::ids::FolderId;
    use pg_model::rules::PathRules;
    use std::path::Path;

    /// Six files on volumes 1, 3 and 7, their records out of id order: on volume 3, file 4's record is below file 3's,
    /// and on volume 7, file 6's below file 5's. Files 1, 3 and 4 are analysed (their hashes and borders are owed, and
    /// they are placed); files 2, 5 and 6 are pending.
    fn three_volumes() -> Catalog {
        let rules = PathRules::Posix;
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        crate::fixture::state::add_scan(&c, 1, &spelled(rules, "D:")).unwrap();
        crate::fixture::state::add_folder(&c, 1, &spelled(rules, r"D:\x")).unwrap();
        let file = |id: i64, file_ref: i64, name: &'static str| File {
            file_ref,
            ..File::new(FileId::new(id), FolderId::new(1), name, 10, 100)
        };
        insert(
            &c,
            &[
                file(1, 2, "a.jpg"),
                file(2, 1, "b.jpg"),
                file(3, 9, "c.jpg"),
                file(4, 5, "d.jpg"),
                file(5, 7, "e.jpg"),
                file(6, 3, "f.jpg"),
            ],
        )
        .unwrap();
        for (id, volume) in [(1, 1), (2, 1), (3, 3), (4, 3), (5, 7), (6, 7)] {
            raw::set_volume(&c, FileId::new(id), Some(volume));
        }
        for id in [1, 3, 4] {
            let items = c.pending_import(false, &VolumeSet::All).unwrap().items;
            let item = items.iter().find(|i| i.id == FileId::new(id)).unwrap();
            store_taken(&mut c, item, None);
        }
        c
    }

    /// The ids of the import, hashes, borders and placeholder lists for `volumes`, in the order each gives them.
    fn lists(c: &Catalog, volumes: &VolumeSet) -> [Vec<i64>; 4] {
        [
            c.pending_import(false, volumes).unwrap().items.iter().map(|i| i.id.get()).collect(),
            c.pending_hashes(volumes).unwrap().items.iter().map(|i| i.id.get()).collect(),
            c.pending_borders(volumes).unwrap().items.iter().map(|i| i.id.get()).collect(),
            c.pending_placeholders(volumes).unwrap().items.iter().map(|p| p.item.id.get()).collect(),
        ]
    }

    /// The statement `pending_borders` runs for `volumes`, as it is written there (its FROM clause is `borders_from`).
    fn borders_statement(volumes: &VolumeSet) -> String {
        format!(
            "SELECT {} {} AND f.missing = 0 AND f.pending = 0
               AND NOT EXISTS (SELECT 1 FROM file_failures x WHERE x.file_id = f.id AND x.pass = 'thumbs'
                   AND x.size = f.size AND x.mtime = f.mtime AND x.rotation = f.rotation)
             ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF",
            work_columns!(),
            borders_from(volumes),
        )
    }

    /// The statement `pending_hashes` runs for `volumes`, as it is written there (its FROM clause is `hashes_from`).
    fn hashes_statement(volumes: &VolumeSet) -> String {
        format!(
            "SELECT {} {} AND f.missing = 0 AND f.pending = 0 ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF",
            work_columns!(),
            hashes_from(volumes),
        )
    }

    #[test]
    fn the_lists_of_all_volumes_are_the_lists_of_each_volume_in_volume_then_record_order() {
        let c = three_volumes();
        let all = lists(&c, &VolumeSet::All);
        assert_eq!(all, [vec![2, 6, 5], vec![1, 4, 3], vec![1, 4, 3], vec![1, 4, 3]]);
        let mut union: [Vec<i64>; 4] = Default::default();
        for volume in [1, 3, 7] {
            for (acc, part) in union.iter_mut().zip(lists(&c, &VolumeSet::Only(vec![volume]))) {
                acc.extend(part);
            }
        }
        assert_eq!(union, all, "the volumes' lists, one after another, are the whole catalog's");
        assert_eq!(
            lists(&c, &VolumeSet::Only(vec![3])),
            [vec![], vec![4, 3], vec![4, 3], vec![4, 3]],
            "within a volume, by record"
        );
    }

    #[test]
    fn the_import_list_of_a_volume_is_a_search_of_the_volume_index_with_no_sort() {
        let c = three_volumes();
        let plan = raw::query_plan(&c, &pending_import_sql(false, &VolumeSet::Only(vec![1])), []);
        assert!(plan.contains("files_to_import (volume=?)") && !plan.contains("TEMP B-TREE"), "{plan}");
    }

    #[test]
    fn a_volume_filtered_borders_or_hashes_list_drives_from_the_files() {
        let c = three_volumes();
        let set = VolumeSet::Only(vec![3]);
        for (label, sql) in [("borders", borders_statement(&set)), ("hashes", hashes_statement(&set))] {
            let plan = raw::query_plan(&c, &sql, []);
            assert!(plan.contains("SEARCH f USING INDEX sqlite_autoindex_files_1 (volume=?)"), "{label}: {plan}");
            assert!(!plan.contains("SCAN a") && !plan.contains("SCAN h"), "{label}: {plan}");
        }
    }

    #[test]
    fn volumes_is_a_search_of_the_volume_index_not_a_scan_of_files() {
        let c = three_volumes();
        let plan = raw::query_plan(&c, VOLUMES, []);
        assert!(plan.contains("sqlite_autoindex_files_1 (volume>?)") && !plan.contains("SCAN files"), "{plan}");
    }

    #[test]
    fn volumes_are_the_distinct_volumes_in_order_and_a_displaced_row_is_none() {
        let c = three_volumes();
        assert_eq!(c.volumes().unwrap(), vec![1, 3, 7]);
        raw::set_volume(&c, FileId::new(2), None);
        assert_eq!(c.volumes().unwrap(), vec![1, 3, 7], "a NULL volume is no volume, and its file is still on 1");
        raw::set_volume(&c, FileId::new(1), None);
        assert_eq!(c.volumes().unwrap(), vec![3, 7], "volume 1 has no files now");
    }

    #[test]
    fn a_volume_set_holds_each_id_as_the_scan_stores_it() {
        assert_eq!(VolumeSet::only(&[1, 2]), VolumeSet::Only(vec![1, 2]));
        assert_eq!(VolumeSet::only(&[i64::MAX.unsigned_abs()]), VolumeSet::Only(vec![i64::MAX]));
        assert_eq!(VolumeSet::only(&[u64::MAX]), VolumeSet::Only(vec![-1]));
    }

    #[test]
    fn the_volume_clause_is_a_literal_list() {
        assert_eq!(VolumeSet::All.and_in("f.volume"), "");
        assert_eq!(VolumeSet::Only(vec![3, -1]).and_in("f.volume"), " AND f.volume IN (3,-1)");
        assert_eq!(VolumeSet::Only(vec![]).and_in("f.volume"), " AND f.volume IN ()");
    }

    #[test]
    fn the_import_list_filters_each_branch_of_the_retry_union() {
        let sql = pending_import_sql(true, &VolumeSet::Only(vec![3]));
        assert_eq!(sql.matches("f.volume IN (3)").count(), 2, "{sql}");
        assert!(!pending_import_sql(true, &VolumeSet::All).contains("IN ("));
        assert!(pending_import_sql(false, &VolumeSet::All).ends_with("ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF"));
    }

    #[test]
    fn a_volume_set_limits_each_pending_list_to_its_volumes() {
        let (c, id) = crate::test_support::one_file(pg_model::rules::PathRules::Posix);
        let volume: i64 = c.conn.query_row("SELECT volume FROM files WHERE id = ?1", [id], |r| r.get(0)).unwrap();
        let here = VolumeSet::Only(vec![volume]);
        let elsewhere = VolumeSet::Only(vec![volume + 1]);
        assert_eq!(c.pending_import(false, &VolumeSet::All).unwrap().len(), 1);
        assert_eq!(c.pending_import(false, &here).unwrap().len(), 1, "its own volume lists it");
        assert!(c.pending_import(false, &elsewhere).unwrap().is_empty(), "another volume does not");
        assert!(c.pending_import(true, &elsewhere).unwrap().is_empty(), "the retry form too");
        assert!(c.pending_borders(&elsewhere).unwrap().is_empty());
        assert!(c.pending_hashes(&elsewhere).unwrap().is_empty());
    }
}
