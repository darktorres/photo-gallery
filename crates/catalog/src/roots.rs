//! The library folders (`roots`) and their scans: which folders the library holds, a scan's start and end, and the
//! files a finished scan didn't see. Methods of `Catalog`.

use anyhow::anyhow;
use rusqlite::{Connection, OptionalExtension, params};

use pg_model::ids::{RootId, ScanId};
use pg_model::rules::Path as RulePath;

use crate::core::{Catalog, write_transaction};
use crate::error::Result;
use crate::types::{Root, folders_under, skipped_folders, under_params};

impl Catalog {
    /// The library folders, by path, with their ids.
    pub fn roots(&self) -> Result<Vec<Root>> {
        self.roots_on(&self.conn)
    }

    /// [`Catalog::roots`] on `conn`, which may be a transaction's.
    pub(crate) fn roots_on(&self, conn: &Connection) -> Result<Vec<Root>> {
        let mut stmt = conn.prepare("SELECT id, path FROM roots ORDER BY path")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        let mut roots = Vec::new();
        for row in rows {
            let (id, bytes) = row?;
            let path = self.rules.parse_bytes(&bytes).ok_or_else(|| anyhow!("a stored library path has a NUL"))?;
            roots.push(Root { id: RootId::new(id), path });
        }
        Ok(roots)
    }

    /// The id of a library folder, by its path; `None` when the path is not one.
    pub fn root_id(&self, path: &RulePath) -> Result<Option<RootId>> {
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM roots WHERE path = ?1", [self.rules.join(path)], |r| r.get(0))
            .optional()?;
        Ok(id.map(RootId::new))
    }

    /// Add a folder to the library. A folder already inside the library adds nothing (returns false); one that
    /// contains library folders replaces them.
    pub fn add_root(&self, root: &RulePath) -> Result<bool> {
        // One transaction: the roots it replaces go only with the insert that replaces them (a crash between the two
        // would lose them), and the check reads what the replacement will see.
        let tx = write_transaction(&self.conn)?;
        let roots = self.roots_on(&tx)?;
        if roots.iter().any(|r| self.rules.is_under(root, &r.path)) {
            return Ok(false);
        }
        for r in roots.iter().filter(|r| self.rules.is_under(&r.path, root)) {
            tx.execute("DELETE FROM roots WHERE id = ?1", [r.id.get()])?;
        }
        tx.execute("INSERT INTO roots(path, added) VALUES (?1, unixepoch())", [self.rules.join(root)])?;
        tx.commit()?;
        Ok(true)
    }

    /// Take a folder out of the library. Its files are marked missing, not deleted: adding it back (or the drive
    /// returning) brings them back with their analysis and ratings. Returns how many files it hid; `None`, with
    /// nothing changed, when `root` is not a library folder.
    pub fn remove_root(&self, root: RootId) -> Result<Option<usize>> {
        let tx = write_transaction(&self.conn)?;
        let path: Option<Vec<u8>> =
            tx.query_row("SELECT path FROM roots WHERE id = ?1", [root.get()], |r| r.get(0)).optional()?;
        let Some(path) = path else { return Ok(None) };
        let path = self.rules.parse_bytes(&path).ok_or_else(|| anyhow!("a stored library path has a NUL"))?;
        tx.execute("DELETE FROM roots WHERE id = ?1", [root.get()])?;
        let [p1, p2, p3] = under_params(self.rules, &path);
        let n = tx.execute(
            concat!("UPDATE files SET missing = 1 WHERE missing = 0 AND folder_id IN (", under_root!(), ")"),
            params![p1, p2, p3],
        )?;
        tx.commit()?;
        Ok(Some(n))
    }

    pub fn begin_scan(&self, root: &RulePath) -> Result<ScanId> {
        self.conn.execute("INSERT INTO scans(root, started) VALUES (?1, unixepoch())", [self.rules.join(root)])?;
        Ok(ScanId::new(self.conn.last_insert_rowid()))
    }

    pub fn finish_scan(&self, scan: ScanId) -> Result<()> {
        self.conn.execute("UPDATE scans SET finished = unixepoch() WHERE id = ?1", [scan.get()])?;
        Ok(())
    }

    /// Mark files under `root` that this scan didn't see as missing. Their rows stay (ratings, decisions). Files in
    /// `skipped` folders (and their subfolders) are left as they are: the scan didn't list them (it couldn't, or they
    /// are reparse folders it doesn't follow), so it saw nothing there to compare with.
    pub fn mark_missing(&self, root: &RulePath, scan: ScanId, skipped: &[RulePath]) -> Result<usize> {
        // One transaction: all of the folders are marked, or none, and the commit is one for the whole pass.
        let tx = write_transaction(&self.conn)?;
        let skipped = skipped_folders(&tx, self.rules, skipped)?;
        let mut marked = 0;
        {
            let mut mark =
                tx.prepare("UPDATE files SET missing = 1 WHERE folder_id = ?1 AND missing = 0 AND last_scan < ?2")?;
            for folder in folders_under(&tx, self.rules, root)? {
                if !skipped.contains(&folder) {
                    marked += mark.execute(params![folder, scan.get()])?;
                }
            }
        }
        tx.commit()?;
        Ok(marked)
    }
}
