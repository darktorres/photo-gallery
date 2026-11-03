//! Test catalogs: files inserted as the scanner would, with their order keys. Behind the `fixture` feature (and in
//! this crate's tests), so every crate that reads a catalog in its tests builds the same files.

use crate::Catalog;
use pg_model::turns::Turns;
use rusqlite::{Connection, params};

/// A file row; `folder` must exist in `folders`.
pub struct File {
    pub id: i64,
    pub folder: i64,
    pub name: &'static str,
    pub kind: &'static str,
    pub size: i64,
    pub mtime: i64,
    pub file_ref: i64,
    pub missing: bool,
    pub rotation: Turns,
}

impl File {
    /// An image present on volume 1, its file reference its id.
    pub fn new(id: i64, folder: i64, name: &'static str, size: i64, mtime: i64) -> File {
        File { id, folder, name, kind: "image", size, mtime, file_ref: id, missing: false, rotation: Turns::NONE }
    }
}

/// The catalog's connection, for the tests of the crates around the catalog, which check the tables a write left
/// behind. Test-only (this module is behind the `fixture` feature): production code reads through the catalog's
/// functions.
pub fn connection(catalog: &Catalog) -> &Connection {
    &catalog.conn
}

/// Insert the files (scan 1 must exist).
pub fn insert(catalog: &Catalog, files: &[File]) {
    let conn = &catalog.conn;
    for f in files {
        let folder: String =
            conn.query_row("SELECT path FROM folders WHERE id = ?1", [f.folder], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, missing,
                               rotation, order_key)
             VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?11)",
            params![
                f.id,
                f.file_ref,
                f.folder,
                f.name,
                pg_model::media::Ext::of_name(f.name).expect("a media file").name(),
                f.kind,
                f.size,
                f.mtime,
                f.missing,
                f.rotation.get(),
                pg_model::path::order_key_of(&folder, f.name)
            ],
        )
        .unwrap();
    }
}
