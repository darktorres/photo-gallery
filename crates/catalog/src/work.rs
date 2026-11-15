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

/// A picture the pairing pass pairs (`Catalog::pairing_work`).
pub struct PairingPicture {
    pub id: FileId,
    /// Its pairs are owed (`analysis.dups_state`): the pass pairs it with the pictures before and after it.
    pub due: bool,
    /// The hash the pairs are made from (the analysis's `pdq`, orientation 0): the guard of `PairsWriter`.
    pub pdq: pg_model::pdq::Hash,
    /// Its eight orientations' hashes, in `pg_analysis::pdq`'s order.
    pub orientations: [pg_model::pdq::Hash; 8],
}

/// The pictures of the pairing pass, and how many analyses it leaves out.
pub struct PairingWork {
    /// Every picture, in file-id order.
    pub pictures: Vec<PairingPicture>,
    /// Analyses made before the orientation hashes were stored (`pdq_dihedral IS NULL`) and of a reliable quality: not
    /// paired, and reported. Re-importing them (`gallery import --again`) makes them pictures.
    pub legacy: usize,
}

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

    /// The pictures the pairing pass pairs, in file-id order, read in one snapshot: a picture is a present, non-pending
    /// file whose analysis has orientation hashes and a reliable quality. The owed ones (`dups_state = 'due'`) are
    /// among them, with the others (the stored side). One snapshot, because a file that changed between two reads
    /// could be owed a pair that its partner's read never sees.
    pub fn pairing_work(&self) -> Result<PairingWork> {
        let tx = self.conn.unchecked_transaction()?;
        let legacy: usize = tx.query_row(
            "SELECT count(*) FROM analysis a JOIN files f ON f.id = a.file_id
             WHERE f.missing = 0 AND f.pending = 0 AND a.pdq_dihedral IS NULL AND a.pdq_quality >= ?1",
            [pg_model::pdq::MIN_QUALITY],
            |r| r.get(0),
        )?;
        let mut stmt = tx.prepare(
            "SELECT a.file_id, a.dups_state = 'due', a.pdq, a.pdq_dihedral
             FROM analysis a JOIN files f ON f.id = a.file_id
             WHERE f.missing = 0 AND f.pending = 0 AND a.pdq_dihedral IS NOT NULL AND a.pdq_quality >= ?1
             ORDER BY a.file_id",
        )?;
        let mut rows = stmt.query([pg_model::pdq::MIN_QUALITY])?;
        let mut pictures = Vec::new();
        while let Some(r) = rows.next()? {
            let id = FileId::new(r.get(0)?);
            let due: bool = r.get(1)?;
            let pdq: Vec<u8> = r.get(2)?;
            let dihedral: Vec<u8> = r.get(3)?;
            let (pdq, orientations) =
                match (<[u8; 32]>::try_from(pdq.as_slice()), <[[u8; 32]; 8]>::try_from(dihedral.as_chunks::<32>().0)) {
                    (Ok(pdq), Ok(orientations)) => (pg_model::pdq::Hash(pdq), orientations.map(pg_model::pdq::Hash)),
                    _ => return Err(Error::Other(anyhow!("file {} has a malformed PDQ hash", id.get()))),
                };
            pictures.push(PairingPicture { id, due, pdq, orientations });
        }
        drop(rows);
        drop(stmt);
        tx.commit()?;
        Ok(PairingWork { pictures, legacy })
    }

    /// Owe every eligible picture its pairs again, and drop the pairs (`gallery pair --again`): what the pairing pass
    /// then makes is all of them again. Returns how many are owed.
    pub fn pairs_again(&mut self) -> Result<usize> {
        let tx = write_transaction(&self.conn)?;
        let owed = crate::lifecycle::owe_pairs_again(&tx)?;
        tx.commit()?;
        Ok(owed)
    }

    /// How many pairs are owed: the files the pairing pass has yet to pair (present and not pending).
    pub fn owed_pairs(&self) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM analysis a JOIN files f ON f.id = a.file_id
             WHERE a.dups_state = 'due' AND f.missing = 0 AND f.pending = 0",
            [],
            |r| r.get(0),
        )?)
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
