//! A file looked up: by id (its path, its folder, whether it is gone, what a queued write or a crop needs of it), by
//! path, or under a folder; and the catalog's counts (`status`). Methods of `Catalog`.

use std::collections::BTreeMap;
use std::rc::Rc;

use rusqlite::types::{Type, Value};
use rusqlite::{OptionalExtension, params};

use pg_model::format::Format;
use pg_model::ids::{FileId, FileIdentity, FileStamp};
use pg_model::media::Kind;
use pg_model::name::Name;
use pg_model::orientation::Orientation;
use pg_model::path::PathArena;
use pg_model::rules::Path as RulePath;
use pg_model::turns::Turns;

use crate::core::Catalog;
use crate::current::{CURRENT_BATCH, CURRENT_ROWS, IS_CURRENT, query_current};
use crate::error::{Error, Result};
use crate::types::{CatalogCounts, CropSource, Listed, WriteTarget, under_params};

impl Catalog {
    /// Whether the file is still as it was listed, with no write of the app's into it in progress: only then is what
    /// a pass read from it kept. Otherwise it is left as it is, due again (a changed file is stale; a file whose
    /// metadata the app is rewriting may be read part-written).
    pub fn is_current(&self, id: FileId, listed: &Listed) -> Result<bool> {
        let mut stmt = self.conn.prepare_cached(IS_CURRENT)?;
        query_current(&mut stmt, id, listed)
    }

    /// [`Catalog::is_current`] for each of `files`, in their order: one read per `CURRENT_BATCH` of them, not one per
    /// file. The thumbnails' writer checks a batch of its results this way, on the thread that stores them.
    pub fn currents(&self, files: &[(FileId, Listed)]) -> Result<Vec<bool>> {
        let mut out = Vec::with_capacity(files.len());
        let mut stmt = self.conn.prepare_cached(CURRENT_ROWS)?;
        for batch in files.chunks(CURRENT_BATCH) {
            let ids: Vec<Value> = batch.iter().map(|(id, _)| Value::Integer(id.get())).collect();
            let rows: BTreeMap<i64, (u64, i64, Turns)> = stmt
                .query_map([Rc::new(ids)], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, Turns::of(r.get(3)?)))))?
                .collect::<rusqlite::Result<_>>()?;
            out.extend(batch.iter().map(|(id, listed)| {
                rows.get(&id.get()).is_some_and(|&(size, mtime, turns)| {
                    size == listed.size && mtime == listed.mtime && turns == listed.turns
                })
            }));
        }
        Ok(out)
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
    /// The stamp of a file: its identity, size and mtime, as the catalog holds them. `None` for an unknown file, and for a
    /// displaced row (no identity: `volume` is NULL).
    pub fn stamp_of(&self, file_id: FileId) -> Result<Option<FileStamp>> {
        let row: Option<(i64, i64, i64, i64)> = self
            .conn
            .query_row(
                "SELECT volume, file_ref, size, mtime FROM files WHERE id = ?1 AND volume IS NOT NULL",
                [file_id.get()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        Ok(row.map(|(volume, file_ref, size, mtime)| FileStamp {
            identity: FileIdentity::new(volume.cast_unsigned(), file_ref.cast_unsigned()),
            size,
            mtime,
        }))
    }

    /// The file a stamp names, if it is still that file in the catalog: a row with the same identity, size and mtime that
    /// isn't missing. A different file that reuses the identity has other bytes or a later time, so it is not matched.
    /// Ids are not used, so a rebuilt or a new catalog finds the file by what it is.
    pub fn find_by_stamp(&self, stamp: FileStamp) -> Result<Option<FileId>> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM files WHERE volume = ?1 AND file_ref = ?2 AND size = ?3 AND mtime = ?4 AND missing = 0",
                params![
                    crate::convert::identity_to_sql(stamp.identity.volume),
                    crate::convert::identity_to_sql(stamp.identity.file_index),
                    stamp.size,
                    stamp.mtime,
                ],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.map(FileId::new))
    }

    pub(crate) fn file_path_of(&self, folder: &[u8], name: &[u8]) -> Result<RulePath> {
        let mut parts =
            self.rules.parse_bytes(folder).ok_or(Error::Stored { what: "a stored folder path has a NUL" })?;
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
             WHERE fo.match_key = ?1 AND f.missing = 0",
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
        let under = under_params(self.rules, folder);
        let ids: Vec<i64> =
            stmt.query_map(params![under.key, under.upper], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
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
                let format = Format::from_key(format).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        Type::Text,
                        format!("unknown image format {format}").into(),
                    )
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

    /// The counts, each a query over the catalog's tables. SQLite picks an index where one covers a query (the indexes
    /// are in `baseline.sql`). A count SQL returns negative is an error (rusqlite's checked `u64` read).
    pub fn status(&self) -> Result<CatalogCounts> {
        // One read snapshot: the counts agree with each other while a scan writes.
        let snapshot = self.conn.unchecked_transaction()?;
        let one = |sql: &str| -> Result<u64> { Ok(snapshot.query_row(sql, [], |r| r.get(0))?) };
        let files = one("SELECT count(*) FROM files WHERE missing = 0")?;
        let failed = one("SELECT count(*) FROM analysis a CROSS JOIN files f
             WHERE a.error IS NOT NULL AND f.id = a.file_id AND f.missing = 0")?;
        Ok(CatalogCounts {
            files,
            missing: one("SELECT count(*) FROM files WHERE missing <> 0")?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::one_file;
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;

    /// `currents` is `is_current` over a batch, in order: as listed, another size, another rotation, an absent file, and
    /// a batch longer than a read. A write in progress makes a file not current, whatever its listing.
    #[test]
    fn currents_is_is_current_over_a_batch() {
        let (c, _) = one_file(PathRules::Windows);
        let id = FileId::new(1);
        let listed = |size: u64, rotation: u8| Listed { size, mtime: 100, turns: Turns::new(rotation) };
        let files = [(id, listed(10, 0)), (id, listed(11, 0)), (id, listed(10, 1)), (FileId::new(9), listed(10, 0))];
        let want: Vec<bool> = files.iter().map(|(f, l)| c.is_current(*f, l).unwrap()).collect();
        assert_eq!(c.currents(&files).unwrap(), want);
        assert_eq!(want, [true, false, false, false]);

        let long: Vec<(FileId, Listed)> =
            (0..2 * CURRENT_BATCH + 1).map(|i| (id, listed(if i % 2 == 0 { 10 } else { 11 }, 0))).collect();
        let answers = c.currents(&long).unwrap();
        assert_eq!(answers.len(), long.len());
        assert!(answers.iter().enumerate().all(|(i, &current)| current == (i % 2 == 0)), "{answers:?}");

        c.conn
            .execute(
                "INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value)
                 VALUES (1, 'rotation', 1, 0, x'00', 1)",
                [],
            )
            .unwrap();
        assert_eq!(c.currents(&files[..1]).unwrap(), [false]);
        assert!(!c.is_current(id, &listed(10, 0)).unwrap());
    }
}

#[cfg(test)]
mod stamp_tests {
    use super::*;
    use crate::test_support::one_file;
    use pg_model::ids::FileStamp;
    use pg_model::rules::PathRules;

    /// The one test file: identity (volume 1, file reference 1), 10 bytes, modified at 100.
    fn the_file() -> FileStamp {
        FileStamp { identity: FileIdentity::new(1, 1), size: 10, mtime: 100 }
    }

    #[test]
    fn a_stamp_names_its_file_while_it_is_still_that_file() {
        let (c, _) = one_file(PathRules::Windows);
        assert_eq!(c.stamp_of(FileId::new(1)).unwrap(), Some(the_file()));
        assert_eq!(c.find_by_stamp(the_file()).unwrap(), Some(FileId::new(1)));
    }

    #[test]
    fn a_reused_identity_is_a_different_file() {
        let (c, _) = one_file(PathRules::Windows);
        assert_eq!(c.find_by_stamp(FileStamp { size: 11, ..the_file() }).unwrap(), None, "other bytes");
        assert_eq!(c.find_by_stamp(FileStamp { mtime: 101, ..the_file() }).unwrap(), None, "a later time");
    }

    #[test]
    fn a_renamed_file_keeps_its_stamp() {
        let (c, _) = one_file(PathRules::Windows);
        c.conn.execute("UPDATE files SET name = ?1 WHERE id = 1", [&b"renamed.jpg"[..]]).unwrap();
        assert_eq!(c.find_by_stamp(the_file()).unwrap(), Some(FileId::new(1)));
    }

    #[test]
    fn a_missing_file_is_not_selected() {
        let (c, _) = one_file(PathRules::Windows);
        c.conn.execute("UPDATE files SET missing = 1 WHERE id = 1", []).unwrap();
        assert_eq!(c.find_by_stamp(the_file()).unwrap(), None);
    }

    #[test]
    fn a_displaced_row_has_no_stamp() {
        let (c, _) = one_file(PathRules::Windows);
        c.conn.execute("UPDATE files SET volume = NULL, file_ref = NULL WHERE id = 1", []).unwrap();
        assert_eq!(c.stamp_of(FileId::new(1)).unwrap(), None);
        assert_eq!(c.find_by_stamp(the_file()).unwrap(), None);
    }

    #[test]
    fn a_stamp_finds_its_file_by_identity_whatever_its_row_id() {
        // The same file, with file reference 2, is row 1 here: a stamp that names it finds row 1, not an id it had before.
        let (c, _) = one_file(PathRules::Windows);
        c.conn.execute("UPDATE files SET file_ref = 2 WHERE id = 1", []).unwrap();
        assert_eq!(
            c.find_by_stamp(FileStamp { identity: FileIdentity::new(1, 2), size: 10, mtime: 100 }).unwrap(),
            Some(FileId::new(1))
        );
    }
}
