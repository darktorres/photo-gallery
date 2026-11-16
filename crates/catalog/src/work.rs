//! The work lists the passes read: the import's pending files, the borders' due analyses, the PDQ hashes the
//! duplicates are matched by, and files made due again (methods of `Catalog`); and a statement's rows as a `WorkList`,
//! each folder's path read once (`work_list`). Its SQL macros are from `sql`.

use anyhow::anyhow;
use pg_model::ids::FileId;
use pg_model::path::WorkList;
use pg_model::turns::Turns;
use rusqlite::Connection;

use crate::core::Catalog;
use crate::error::{Error, Result};
use crate::transaction::write_transaction;
use crate::types::{FOLDER_PATH, WorkItem, folder_path};

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

/// The pictures of the pairing pass, in file-id order, as columns (`Catalog::pairing_work`): picture `i` is file
/// `ids[i]`, owes its pairs when `owed[i]`, and has the stored hashes `hashes[i]` and quality `quality[i]`. The join takes
/// the columns as they are.
pub struct PairingWork {
    pub ids: Vec<FileId>,
    /// Its pairs are owed (a `pairs_owed` row) and the pass can make them (its file is present and not pending, as
    /// [`Catalog::owed_pairs`] counts): the pass pairs it with the pictures before and after it. An owed picture whose
    /// file is missing or pending is only a partner here; its own pairs are made by the first pass after it is back.
    pub owed: Vec<bool>,
    /// Its kept orientations' hashes (`pg_model::pdq::ORIENTATIONS`, in `pg_analysis::pdq`'s order): what the pairs are
    /// made from, and, as `pg_model::pdq::to_blob` stores them, what `PairsWriter`'s guard compares (the pass takes them
    /// into its join, which keeps the one copy).
    pub hashes: Vec<[pg_model::pdq::Hash; pg_model::pdq::ORIENTATIONS]>,
    /// Its `pdq_quality`: with the hashes, what `PairsWriter`'s guard compares.
    pub quality: Vec<u8>,
    /// Analyses of a reliable quality with no stored hashes (`pdq_dihedral IS NULL`: made by a build before the
    /// orientation hashes, or whose hashes failed the check of migration 13), of present, non-pending files: not
    /// paired, and reported. Re-importing them (`gallery import --again`) makes them pictures; it reaches only those
    /// files, so only they are counted.
    pub legacy: usize,
    /// The catalog's pairing marks in the load's snapshot, which the writes compare theirs with (`PairsWriter::write`).
    pub marks: PairingMarks,
}

/// What the pairing pass's pairs depend on beyond the pictures: `meta.pairing_version`, the pairing the pairs are made by
/// (another build's `Catalog::prepare` changes it and owes every pair again), and `meta.pairing_epoch`, which moves on
/// whenever an analysis or an owed row that the pairs could have been made from changes (migration 14). A write whose
/// marks are the load's needs no guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairingMarks {
    pub version: i64,
    pub epoch: i64,
}

/// The pairing marks as `conn`'s transaction sees them.
pub(crate) fn pairing_marks(conn: &Connection) -> Result<PairingMarks> {
    let mut stmt = conn.prepare_cached(
        "SELECT (SELECT value FROM meta WHERE key = 'pairing_version'),
             (SELECT value FROM meta WHERE key = 'pairing_epoch')",
    )?;
    Ok(stmt.query_row([], |r| Ok(PairingMarks { version: r.get(0)?, epoch: r.get(1)? }))?)
}

/// What [`Catalog::pairing_work`] reads: the analyses of a reliable quality, scanned in file-id order (`analysis`'s rowid
/// order, so the `ORDER BY` costs no sort). Which of them are pictures, and which owe their pairs, comes from four id
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
const PICTURES: &str =
    "SELECT file_id, pdq_dihedral, pdq_quality FROM analysis WHERE pdq_quality >= ?1 ORDER BY file_id";
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

/// [`Catalog::pairing_work`], read on `conn` in one transaction (its one snapshot).
fn pairing_work_on(conn: &Connection) -> Result<PairingWork> {
    let tx = conn.unchecked_transaction()?;
    let marks = pairing_marks(&tx)?;
    let identity = IdSet::read(&tx, WITH_IDENTITY)?;
    let present = IdSet::read(&tx, PRESENT)?;
    let pending = IdSet::read(&tx, PRESENT_PENDING)?;
    let owed = IdSet::read(&tx, OWED_IDS)?;
    // Each picture is a file with an identity: the columns never grow.
    let bound = identity.0.len();
    let mut work = PairingWork {
        ids: Vec::with_capacity(bound),
        owed: Vec::with_capacity(bound),
        hashes: Vec::with_capacity(bound),
        quality: Vec::with_capacity(bound),
        legacy: 0,
        marks,
    };
    let (mut identity, mut present, mut pending, mut owed) =
        (identity.walk(), present.walk(), pending.walk(), owed.walk());
    let mut stmt = tx.prepare(PICTURES)?;
    let mut rows = stmt.query([pg_model::pdq::MIN_QUALITY])?;
    while let Some(r) = rows.next()? {
        let raw: i64 = r.get(0)?;
        if !identity.has(raw) {
            continue;
        }
        let id = FileId::new(raw);
        // Present and not pending: the pass can make its pairs.
        let makes = present.has(raw) && !pending.has(raw);
        let Some(blob) = r.get_ref(1)?.as_blob_or_null()? else {
            if makes {
                work.legacy += 1;
            }
            continue;
        };
        let hashes = pg_model::pdq::from_blob(blob)
            .ok_or_else(|| Error::Other(anyhow!("file {raw} has malformed stored hashes")))?;
        work.ids.push(id);
        work.owed.push(owed.has(raw) && makes);
        work.hashes.push(hashes);
        work.quality.push(r.get(2)?);
    }
    drop(rows);
    drop(stmt);
    tx.commit()?;
    Ok(work)
}

impl crate::Reader<'_> {
    /// [`Catalog::pairing_work`], on this reader's connection.
    pub fn pairing_work(&self) -> Result<PairingWork> {
        pairing_work_on(self.conn())
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
    /// on a [`Reader`](crate::Reader) of its own ([`Reader::pairing_work`](crate::Reader::pairing_work)).
    pub fn pairing_work(&self) -> Result<PairingWork> {
        pairing_work_on(&self.conn)
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
