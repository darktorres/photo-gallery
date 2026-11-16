//! The work lists the passes read: the import's pending files, the borders' due analyses, the PDQ hashes the
//! duplicates are matched by, and files made due again (methods of `Catalog`); and a statement's rows as a `WorkList`,
//! each folder's path read once (`work_list`). Its SQL macros are from `sql`.

use anyhow::anyhow;
use pg_model::ids::FileId;
use pg_model::path::WorkList;
use pg_model::turns::Turns;
use rusqlite::Connection;

use crate::core::{Catalog, ReadOnly, Reader};
use crate::error::{Error, Result};
use crate::transaction::write_transaction;
use crate::types::{FOLDER_PATH, PairingWork, WorkItem, folder_path, pairing_marks};

/// `Catalog::pending_import`'s query.
pub(crate) fn pending_import_sql(retry_errors: bool) -> &'static str {
    if retry_errors {
        // A UNION, as an OR would scan every file; the failures are read from their index first.
        concat!(
            pending_files!(),
            " UNION SELECT ",
            work_columns!(),
            ", f.volume AS volume, f.file_ref & 0xFFFFFFFFFFFF AS record
             FROM analysis a CROSS JOIN files f
             WHERE a.error IS NOT NULL AND f.id = a.file_id AND f.missing = 0 ORDER BY volume, record"
        )
    } else {
        concat!(pending_files!(), " ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF")
    }
}

/// The work list of a statement whose rows start with `work_columns!()` and number at most `bound`, read in
/// `conn`'s transaction: the items in a `Vec` of that capacity, each folder's path read once.
pub(crate) fn work_list(conn: &Connection, stmt: &mut rusqlite::Statement, bound: usize) -> Result<WorkList<WorkItem>> {
    work_list_of(conn, stmt.query([])?, bound)
}

/// [`work_list`] of the rows a statement gave (`rows`, from its parameters or none).
fn work_list_of(conn: &Connection, mut rows: rusqlite::Rows<'_>, bound: usize) -> Result<WorkList<WorkItem>> {
    let (mut list, mut folder) = (WorkList::default(), None);
    while let Some(r) = rows.next()? {
        if folder.is_none() {
            // Only once there is work: a round with nothing to do reads nothing more.
            list = WorkList::with_capacity(bound, max_folder_id(conn)?);
            folder = Some(conn.prepare_cached(FOLDER_PATH)?);
        }
        let Some(folder) = folder.as_mut() else { unreachable!("made above") };
        let path = list.file(r.get(1)?, r.get_ref(2)?.as_blob()?, |arena| folder_path(folder, r, arena))?;
        list.items.push(WorkItem {
            id: FileId::new(r.get(0)?),
            path,
            size: r.get(3)?,
            mtime: r.get(4)?,
            rotation: Turns::new(r.get(5)?),
            kind: crate::types::kind_at(r, 6)?,
        });
    }
    Ok(list)
}

/// The item of file `id` as the work lists have it (`work_columns!`), or `None` when it isn't in the catalog.
pub(crate) fn work_item(conn: &Connection, id: FileId) -> Result<Option<WorkItem>> {
    let mut stmt = conn.prepare_cached(concat!("SELECT ", work_columns!(), " FROM files f WHERE f.id = ?1"))?;
    Ok(work_list_of(conn, stmt.query([id.get()])?, 1)?.items.pop())
}

/// The largest folder id of the catalog (-1 for none): an O(1) read of the table's last row. The thumbnail cache reads
/// the same through the `thumb_folders` view.
fn max_folder_id(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT coalesce(max(id), -1) FROM folders", [], |r| r.get(0))?)
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
    /// Without `retry_errors` it's read in that order from `files_to_import`, so nothing pending is one probe.
    pub fn pending_import(&self, retry_errors: bool) -> Result<WorkList<WorkItem>> {
        // Counted in the same snapshot: an upper bound (a failure may also be pending), each count read from its
        // index alone.
        let tx = self.conn.unchecked_transaction()?;
        let count = |sql: &str| -> Result<usize> { Ok(tx.query_row(sql, [], |r| r.get(0))?) };
        let mut bound = count("SELECT count(*) FROM files WHERE missing = 0 AND pending = 1")?;
        if retry_errors {
            bound += count("SELECT count(*) FROM analysis WHERE error IS NOT NULL")?;
        }
        let list = work_list(&tx, &mut tx.prepare(pending_import_sql(retry_errors))?, bound)?;
        tx.commit()?;
        Ok(list)
    }

    /// Analyses whose borders are due (new ones, and all after a new detector: `set_borders_version`), of
    /// files the import has nothing to do for, in on-disk order.
    pub fn pending_borders(&self) -> Result<WorkList<WorkItem>> {
        // An upper bound, from `analysis_borders_due` alone.
        let tx = self.conn.unchecked_transaction()?;
        let bound: usize = tx.query_row("SELECT count(*) FROM analysis WHERE borders_due = 1", [], |r| r.get(0))?;
        let mut stmt = tx.prepare(concat!(
            "SELECT ",
            work_columns!(),
            " FROM analysis a CROSS JOIN files f
             WHERE a.borders_due = 1 AND f.id = a.file_id AND f.missing = 0 AND f.pending = 0
             ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF"
        ))?;
        let list = work_list(&tx, &mut stmt, bound)?;
        drop(stmt);
        tx.commit()?;
        Ok(list)
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
    /// then makes is all of them again. Returns how many are owed.
    pub fn pairs_again(&mut self) -> Result<usize> {
        let tx = write_transaction(&self.conn)?;
        let owed = crate::lifecycle::owe_pairs_again(&tx)?;
        tx.commit()?;
        Ok(owed)
    }

    /// How many pictures owe their pairs: the files the pairing pass has yet to pair (present and not pending).
    pub fn owed_pairs(&self) -> Result<usize> {
        Ok(self.conn.query_row(&format!("SELECT count(*) {OWED}"), [], |r| r.get(0))?)
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
