//! A file's border sides as detection found them (`border_sides`), and the user's mark that they are not a border
//! (`set_border_mark`, the review). Methods of `Catalog`. The sides are read by the read side's `border_sides_on`
//! (`browse::read`), which the details read in their snapshot.

use rusqlite::params;

use pg_model::detection::{self, Detection};
use pg_model::ids::FileId;

use crate::core::Catalog;
use crate::current::{IS_CURRENT, query_current};
use crate::error::{Error, Result};
use crate::work::work_item;
use crate::writers::BordersWriter;

impl Catalog {
    /// Whether a file is in the library now: its row is there and not hidden. A pass checks it before it writes the file.
    pub fn is_present(&self, file_id: FileId) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1 AND missing = 0)",
            [file_id.get()],
            |r| r.get(0),
        )?)
    }

    /// A file's borders were cut off in its original (`pg_crop::cut`), which now has the (size, mtime) `after`. The cut is
    /// exact, so the file has no borders left: its detection is stored as none now, with no mark (the review's). Its
    /// analysis was made of the uncut pixels, and its size and mtime no longer match the file's, so the next import
    /// analyzes it again, and the borders pass looks at the cut file once more.
    pub fn borders_cut(&self, file_id: FileId, after: (u64, i64)) -> Result<()> {
        let tx = crate::transaction::write_transaction(&self.conn)?;
        // A file removed from the library since its cut was made (absent, or hidden for a removal) has nothing to record:
        // that is not a failure of anything the user still has.
        let present: bool =
            tx.query_row("SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1 AND missing = 0)", [file_id.get()], |r| {
                r.get(0)
            })?;
        if !present {
            return Ok(());
        }
        tx.execute("UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1", params![file_id.get(), after.0, after.1])?;
        // The file as it is now, which the detection is stored for, and the stored size its analysis has (a detection of
        // none has no cut, whatever that size is).
        let item = work_item(&tx, file_id)?.ok_or(Error::NotInCatalog { id: file_id.get() })?;
        let (width, height): (u32, u32) =
            tx.query_row("SELECT width, height FROM analysis WHERE file_id = ?1", [file_id.get()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        // The store is refused for a file with a write of the app's in progress: the cut is then not recorded, and
        // the import sees the file changed.
        if !query_current(&mut tx.prepare(IS_CURRENT)?, file_id, &item.listed())? {
            return Err(Error::BeingWritten { id: file_id.get() });
        }
        if !BordersWriter::new(&tx)?.store_borders(&item, None, Some(&Detection::none(width, height)))? {
            return Err(Error::OtherBuildsBorders { id: file_id.get() });
        }
        tx.commit()?;
        Ok(())
    }

    /// The sides detection found in a file, as the catalog keeps them. The keys are read in place. A file with more
    /// than detection finds (`detection::MAX_SIDES`) can only be in a damaged catalog: an error.
    pub fn border_sides(&self, file_id: FileId) -> Result<detection::Sides> {
        crate::browse::read::border_sides_on(&self.conn, file_id)
    }

    /// Mark a file's borders as not a border, or take the mark back (the review).
    pub fn set_border_mark(&self, file_id: FileId, marked: bool) -> Result<()> {
        if marked {
            self.conn
                .prepare_cached("INSERT OR REPLACE INTO border_marks(file_id, marked) VALUES (?1, unixepoch())")?
                .execute([file_id.get()])?;
        } else {
            self.conn.prepare_cached("DELETE FROM border_marks WHERE file_id = ?1")?.execute([file_id.get()])?;
        }
        Ok(())
    }
}
