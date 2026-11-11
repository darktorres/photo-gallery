//! A file looked up: by id (its path, its folder, whether it is gone, what a queued write or a crop needs of it), by
//! path, or under a folder; and the catalog's counts (`status`). Methods of `Catalog`.

use anyhow::anyhow;
use rusqlite::types::Type;
use rusqlite::{OptionalExtension, params};

use pg_model::format::Format;
use pg_model::ids::FileId;
use pg_model::media::Kind;
use pg_model::name::Name;
use pg_model::orientation::Orientation;
use pg_model::path::PathArena;
use pg_model::rules::Path as RulePath;

use crate::core::Catalog;
use crate::current::{IS_CURRENT, query_current};
use crate::error::Result;
use crate::types::{CropSource, Listed, Status, WriteTarget, under_params};

impl Catalog {
    /// Whether the file is still as it was listed, with no write of the app's into it in progress: only then is what
    /// a pass read from it kept. Otherwise it is left as it is, due again (a changed file is stale; a file whose
    /// metadata the app is rewriting may be read part-written).
    pub fn is_current(&self, id: FileId, listed: &Listed) -> Result<bool> {
        let mut stmt = self.conn.prepare_cached(IS_CURRENT)?;
        query_current(&mut stmt, id, listed)
    }

    /// Whether the file can't be written: it is missing from the catalog, or no longer listed.
    pub fn file_gone(&self, file_id: FileId) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT coalesce((SELECT missing FROM files WHERE id = ?1), 1)",
            [file_id.get()],
            |r| r.get(0),
        )?)
    }

    /// The folder the file is in, as the catalog has it; `None` if there is no such file. A failed query is an error.
    pub fn file_folder(&self, file_id: FileId) -> Result<Option<RulePath>> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
                [file_id.get()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(bytes.and_then(|bytes| self.rules.parse_bytes(&bytes)))
    }

    /// The folder the file is in while the library shows it (present); `None` for a file gone, or missing.
    pub fn present_file_folder(&self, file_id: FileId) -> Result<Option<RulePath>> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1 AND f.missing = 0",
                [file_id.get()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(bytes.and_then(|bytes| self.rules.parse_bytes(&bytes)))
    }

    /// Where a file is: its folder and name, exact. `None` when the catalog has no such file.
    pub fn file_path(&self, file_id: FileId) -> Result<Option<RulePath>> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT fo.path, f.name FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
                [file_id.get()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(folder, name)| self.file_path_of(&folder, &name)).transpose()
    }

    /// A file's path from its folder's stored bytes and its name's: the folder, the separator, the name.
    pub(crate) fn file_path_of(&self, folder: &[u8], name: &[u8]) -> Result<RulePath> {
        let mut parts = self.rules.parse_bytes(folder).ok_or_else(|| anyhow!("a stored folder path has a NUL"))?;
        parts.parts.push(Name::from_raw(name));
        Ok(parts)
    }

    /// What a queued write needs of its file: its path, its analyzed format, and its kind, each read at the row (a
    /// format or a kind that doesn't read is a damaged row, and an error).
    pub fn write_target(&self, file_id: FileId) -> Result<WriteTarget> {
        let (folder, name, format, kind): (Vec<u8>, Vec<u8>, Option<Format>, Kind) = self.conn.query_row(
            "SELECT fo.path, f.name, a.format, f.kind FROM files f JOIN folders fo ON fo.id = f.folder_id
             LEFT JOIN analysis a ON a.file_id = f.id WHERE f.id = ?1",
            [file_id.get()],
            |r| Ok((r.get(0)?, r.get(1)?, crate::types::format_at(r, 2)?, crate::types::kind_at(r, 3)?)),
        )?;
        Ok(WriteTarget { path: self.file_path_of(&folder, &name)?, format, kind })
    }

    /// The present file at an absolute path (Windows paths: any case).
    pub fn file_by_path(&self, path: &RulePath) -> Result<Option<FileId>> {
        let Some((folder, name)) = self.rules.split_file_name(path) else { return Ok(None) };
        let key = self.rules.folder_key(&folder.parts);
        let name_key = name.name_key();
        // The files of the folder's key (every drive's copy of it), then the one at this path by the rules: the same
        // drive, and a name equal by key, which is case-insensitive where the rules are (SQLite's lower() is ASCII).
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.id, f.name, fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id
             WHERE fo.key = ?1 AND f.missing = 0",
        )?;
        let rows =
            stmt.query_map([key], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?)))?;
        for row in rows {
            let (id, stored_name, stored_folder) = row?;
            let same_folder = self.rules.parse_bytes(&stored_folder).is_some_and(|f| self.rules.same(&f, &folder));
            let mut stored_key = Vec::new();
            Name::from_raw(&stored_name).push_name_key(&mut stored_key);
            if same_folder && stored_key == name_key {
                return Ok(Some(FileId::new(id)));
            }
        }
        Ok(None)
    }

    /// The present files in a folder and its subfolders (a path in the catalog's case, without a trailing
    /// separator).
    pub fn files_under(&self, folder: &RulePath) -> Result<Vec<FileId>> {
        let mut stmt = self.conn.prepare(concat!(
            "SELECT id FROM files WHERE missing = 0 AND folder_id IN (",
            under_root!(),
            ")"
        ))?;
        let [p1, p2, p3] = under_params(self.rules, folder);
        let ids: Vec<i64> = stmt.query_map(params![p1, p2, p3], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids.into_iter().map(FileId::new).collect())
    }

    /// What cropping a file's borders off needs: its borders as found in the file as it was analyzed (`size`,
    /// `mtime`: the caller checks the file is still that one), its path added to `paths`. `None` when it has none to
    /// crop: none found, marked as not a border, or not looked for in the file as it is.
    pub fn crop_source(&self, file_id: FileId, paths: &mut PathArena) -> Result<Option<CropSource>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT (SELECT path FROM folders WHERE id = f.folder_id), f.name, f.size, f.mtime,
                        a.format, a.orientation, a.width, a.height, a.cut_left, a.cut_top, a.cut_right, a.cut_bottom
                 FROM files f JOIN analysis a ON a.file_id = f.id
                 WHERE f.id = ?1 AND f.missing = 0 AND f.pending = 0 AND a.borders_due = 0
                   AND a.size = f.size AND a.mtime = f.mtime
                   AND a.cut_left + a.cut_top + a.cut_right + a.cut_bottom > 0
                   AND NOT EXISTS(SELECT 1 FROM border_marks WHERE file_id = f.id)",
            )?
            .query_row([file_id.get()], |r| {
                let (orientation, w, h): (Orientation, u32, u32) =
                    (Orientation::from_tag(r.get(5)?), r.get(6)?, r.get(7)?);
                let (width, height) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
                let format = r.get_ref(4)?.as_str()?;
                let format = Format::from_name(format).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(4, Type::Text, format!("unknown format {format}").into())
                })?;
                let (folder, name) = (r.get_ref(0)?.as_blob()?, r.get_ref(1)?.as_blob()?);
                let path = self.rules.sep_byte();
                Ok(CropSource {
                    path: paths.push_parts(&[folder, &[path], name]),
                    size: r.get(2)?,
                    mtime: r.get(3)?,
                    format,
                    width,
                    height,
                    cut: [r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?],
                })
            })
            .optional()?)
    }

    /// Each count from its own index where there is one (`files_present`, `files_to_import`, `analysis_failed`).
    pub fn status(&self) -> Result<Status> {
        // One read snapshot: the counts agree with each other while a scan writes.
        let snapshot = self.conn.unchecked_transaction()?;
        let one = |sql: &str| -> Result<i64> { Ok(snapshot.query_row(sql, [], |r| r.get(0))?) };
        let files = one("SELECT count(*) FROM files WHERE missing = 0")?;
        let failed = one("SELECT count(*) FROM analysis a CROSS JOIN files f
             WHERE a.error IS NOT NULL AND f.id = a.file_id AND f.missing = 0")?;
        Ok(Status {
            files,
            missing: one("SELECT count(*) FROM files")? - files,
            // Present files with an analysis that isn't a failure, current or not: the one count that reads them all.
            analyzed: one(
                "SELECT count(*) FROM analysis a JOIN files f ON f.id = a.file_id WHERE f.missing = 0 AND a.error IS NULL",
            )?,
            failed,
            pending: one("SELECT count(*) FROM files WHERE missing = 0 AND pending = 1")?,
            pdq_pairs: one("SELECT count(*) FROM present_pdq_pairs")?,
            queued_writes: one("SELECT count(*) FROM file_writes")?,
            failed_writes: one("SELECT count(*) FROM file_writes WHERE error IS NOT NULL")?,
        })
    }
}
