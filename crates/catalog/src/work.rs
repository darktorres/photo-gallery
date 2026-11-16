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
/// `ids[i]`, owes its pairs when `owed[i]`, and has the stored hashes `hashes[i]`. The join takes the columns as they are.
pub struct PairingWork {
    pub ids: Vec<FileId>,
    /// Its pairs are owed (a `pairs_owed` row) and the pass can make them (its file is present and not pending, as
    /// [`Catalog::owed_pairs`] counts): the pass pairs it with the pictures before and after it. An owed picture whose
    /// file is missing or pending is only a partner here; its own pairs are made by the first pass after it is back.
    pub owed: Vec<bool>,
    /// Its kept orientations' hashes (`pg_model::pdq::ORIENTATIONS`, in `pg_analysis::pdq`'s order): what the pairs are
    /// made from, and, as `pg_model::pdq::to_blob` stores them, the guard of `PairsWriter`.
    pub hashes: Vec<[pg_model::pdq::Hash; pg_model::pdq::ORIENTATIONS]>,
    /// Analyses of a reliable quality with no stored hashes (`pdq_dihedral IS NULL`: made by a build before the
    /// orientation hashes, or whose hashes failed the check of migration 13), of present, non-pending files: not
    /// paired, and reported. Re-importing them (`gallery import --again`) makes them pictures; it reaches only those
    /// files, so only they are counted.
    pub legacy: usize,
}

/// The statement [`Catalog::pairing_work`] reads: the analysis of a reliable quality of every file with an identity, by
/// file id, with whether it owes its pairs and its file is present and not pending ([`OWED`]'s rule), and whether that
/// file is present and not pending (for the legacy count). `analysis` is the outer loop (`CROSS JOIN`), read in its own
/// rowid order, so the `ORDER BY` costs no sort; each file is a lookup by key, and so is the `pairs_owed` row. The rows
/// without stored hashes are the legacy ones, counted in the same scan.
///
/// A pair is a fact about two stored analyses, so a missing or pending file's analysis is a partner like any other: a
/// picture paired while its partner's file was away is never owed again when the file comes back without a new
/// analysis (a rescan that finds it, a rebind, a turn undone, an mtime put back, an older build's import version), and
/// a pair the pass left out then would never be made. A pair with a missing file is hidden where pairs are shown
/// (`present_pdq_pairs`), and every new analysis drops its file's pairs and owes them again. A displaced row (no
/// identity, `volume IS NULL`) is left out: its analysis stays only until a replacement deletes it, its pairs could
/// never be shown, and the file's new row would pair with it.
const PAIRING_WORK: &str = "SELECT a.file_id, o.file_id IS NOT NULL AND f.missing = 0 AND f.pending = 0, a.pdq_dihedral,
         f.missing = 0 AND f.pending = 0
     FROM analysis a CROSS JOIN files f LEFT JOIN pairs_owed o ON o.file_id = a.file_id
     WHERE f.id = a.file_id AND f.volume IS NOT NULL AND a.pdq_quality >= ?1
     ORDER BY a.file_id";

/// [`Catalog::pairing_work`], read on `conn` in one transaction (its one snapshot).
fn pairing_work_on(conn: &Connection) -> Result<PairingWork> {
    let tx = conn.unchecked_transaction()?;
    let bound: usize = tx.query_row("SELECT count(*) FROM analysis", [], |r| r.get(0))?;
    let mut work = PairingWork {
        ids: Vec::with_capacity(bound),
        owed: Vec::with_capacity(bound),
        hashes: Vec::with_capacity(bound),
        legacy: 0,
    };
    let mut stmt = tx.prepare(PAIRING_WORK)?;
    let mut rows = stmt.query([pg_model::pdq::MIN_QUALITY])?;
    while let Some(r) = rows.next()? {
        let id = FileId::new(r.get(0)?);
        let Some(blob) = r.get_ref(2)?.as_blob_or_null()? else {
            if r.get(3)? {
                work.legacy += 1;
            }
            continue;
        };
        let hashes = pg_model::pdq::from_blob(blob)
            .ok_or_else(|| Error::Other(anyhow!("file {} has malformed stored hashes", id.get())))?;
        work.ids.push(id);
        work.owed.push(r.get(1)?);
        work.hashes.push(hashes);
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
    /// hashes and a reliable quality, of a file with an identity (missing and pending ones too: `PAIRING_WORK` says
    /// why). The owed ones the pass can make (a `pairs_owed` row, the file present and not pending) are among them,
    /// with the others (the stored side). One snapshot, because a file that changed between two reads could be owed a
    /// pair that its partner's read never sees. The columns are sized from the analyses' count first, so they never
    /// grow (an upper bound: every analysis), and each row's hashes are read from the blob in place. The pass reads it
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
