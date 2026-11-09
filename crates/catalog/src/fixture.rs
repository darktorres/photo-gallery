//! Test catalogs: files inserted as the scanner would, with their order keys. Behind the `fixture` feature (and in
//! this crate's tests), so every crate that reads a catalog in its tests builds the same files.

#[cfg(test)]
pub(crate) mod raw;
pub mod state;

use crate::Catalog;
use anyhow::{Context, anyhow};
use pg_model::ids::{FileId, FolderId};
use pg_model::turns::Turns;
use rusqlite::params;

/// A file row; `folder` must exist in `folders`.
pub struct File {
    pub id: FileId,
    pub folder: FolderId,
    /// The name's exact bytes (a name with no UTF-8 form is one of these too).
    pub name: &'static [u8],
    pub kind: &'static str,
    pub size: i64,
    pub mtime: i64,
    pub file_ref: i64,
    pub missing: bool,
    pub rotation: Turns,
}

impl File {
    /// An image present on volume 1, its file reference its id.
    pub fn new(id: FileId, folder: FolderId, name: &'static str, size: i64, mtime: i64) -> File {
        File::named(id, folder, name.as_bytes(), size, mtime)
    }

    /// [`File::new`] for a name given as its exact bytes, which a `&str` can't hold (a lone surrogate, a raw byte).
    pub fn named(id: FileId, folder: FolderId, name: &'static [u8], size: i64, mtime: i64) -> File {
        File { id, folder, name, kind: "image", size, mtime, file_ref: id.get(), missing: false, rotation: Turns::NONE }
    }
}

/// A folder's stored bytes and key, from its path text (as the tests write it) under the catalog's rules.
pub(crate) fn folder_row(c: &Catalog, path: &str) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let path = c.rules().parse(path).ok_or_else(|| anyhow!("{path:?} is not a test folder path"))?;
    Ok((c.rules().join(&path), c.rules().folder_key(&path.parts)))
}

/// Insert the files (scan 1 must exist).
pub fn insert(catalog: &Catalog, files: &[File]) -> anyhow::Result<()> {
    let conn = &catalog.conn;
    for f in files {
        let folder: Vec<u8> =
            conn.query_row("SELECT path FROM folders WHERE id = ?1", [f.folder.get()], |r| r.get(0)).with_context(
                || format!("the folder {} of {}", f.folder.get(), pg_model::name::Name::from_raw(f.name).display()),
            )?;
        let rules = catalog.rules();
        let folder_parts = rules.parse_bytes(&folder).ok_or_else(|| anyhow!("a stored folder path has a NUL"))?;
        let name = pg_model::name::Name::from_raw(f.name);
        let order_key = rules.order_key(&rules.folder_key(&folder_parts.parts), &name);
        let ext = pg_model::media::Ext::of_name_bytes(f.name)
            .ok_or_else(|| anyhow!("{} is not a media file", name.display()))?;
        conn.execute(
            "INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, missing,
                               rotation, order_key)
             VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?11)",
            params![
                f.id.get(),
                f.file_ref,
                f.folder.get(),
                f.name,
                ext.name(),
                f.kind,
                f.size,
                f.mtime,
                f.missing,
                f.rotation.get(),
                order_key
            ],
        )
        .with_context(|| format!("insert {}", name.display()))?;
    }
    Ok(())
}
