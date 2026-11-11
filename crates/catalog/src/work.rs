//! The work lists the passes read: the import's pending files (`pending_import_sql`), and a statement's rows as a
//! `WorkList`, each folder's path read once (`work_list`). Its SQL macros are from `sql`.

use pg_model::ids::FileId;
use pg_model::path::WorkList;
use pg_model::turns::Turns;
use rusqlite::Connection;

use crate::error::Result;
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
    let mut rows = stmt.query([])?;
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

/// The largest folder id of the catalog (-1 for none): an O(1) read of the table's last row. The thumbnail cache reads
/// the same through the `thumb_folders` view.
fn max_folder_id(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT coalesce(max(id), -1) FROM folders", [], |r| r.get(0))?)
}
