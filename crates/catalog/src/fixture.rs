//! Test catalogs: files inserted as the scanner would, with their merged folders. Behind the `fixture` feature (and in
//! this crate's tests), so every crate that reads a catalog in its tests builds the same files. `raw` (the rows as the
//! scanner writes them) is for this crate's own tests only.

#[cfg(test)]
pub(crate) mod raw;
pub mod state;

use crate::core::Catalog;
use crate::error::{Error, Result};
use pg_model::ids::{FileId, FolderId};
use pg_model::turns::Turns;
use rusqlite::params;

/// A file row; `folder` must exist in `folders`.
pub struct File {
    /// The file's id.
    pub id: FileId,
    /// The folder it is in (a `folders` row).
    pub folder: FolderId,
    /// The name's exact bytes (a name with no UTF-8 form is one of these too).
    pub name: &'static [u8],
    /// The file's kind, by its name (`"image"`, `"video"`; the row stores its code).
    pub kind: &'static str,
    /// The file's size in bytes.
    pub size: i64,
    /// The file's modification time (Unix seconds).
    pub mtime: i64,
    /// The file's identity on its volume (its file reference).
    pub file_ref: i64,
    /// Whether the file is marked missing.
    pub missing: bool,
    /// The file's quarter turns in the catalog.
    pub turns: Turns,
}

impl File {
    /// An image present on volume 1, its file reference its id.
    pub fn new(id: FileId, folder: FolderId, name: &'static str, size: i64, mtime: i64) -> File {
        File::named(id, folder, name.as_bytes(), size, mtime)
    }

    /// [`File::new`] for a name given as its exact bytes, which a `&str` can't hold (a lone surrogate, a raw byte).
    pub fn named(id: FileId, folder: FolderId, name: &'static [u8], size: i64, mtime: i64) -> File {
        File { id, folder, name, kind: "image", size, mtime, file_ref: id.get(), missing: false, turns: Turns::NONE }
    }
}

/// While held, every ledger write (record or clear, on any connection of this process) fails: a ledger that can't be
/// written, as an aborted statement would leave it. One at a time: the tests that hold it run one by one.
pub struct LedgerWritesFail(());

impl LedgerWritesFail {
    /// Makes the ledger's writes fail until the fault is dropped.
    pub fn start() -> LedgerWritesFail {
        crate::writers::LEDGER_FAULT.store(true, std::sync::atomic::Ordering::SeqCst);
        LedgerWritesFail(())
    }
}

impl Drop for LedgerWritesFail {
    fn drop(&mut self) {
        crate::writers::LEDGER_FAULT.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The write transactions this thread begins, timed while this is held: how long each held the catalog's write lock,
/// from its begin to its commit or rollback (`transaction::write_transaction`; every catalog write but the
/// preparation's version marks takes it). What a test of a walk measures: no hold may span a listing or a pause.
pub struct WriteHolds(());

impl WriteHolds {
    /// Start timing (one watch per thread: a second starts it over).
    pub fn watch() -> WriteHolds {
        crate::transaction::watch_holds();
        WriteHolds(())
    }

    /// The holds that ended since the watch began or was last taken, in the order they ended.
    pub fn take(&self) -> Vec<std::time::Duration> {
        crate::transaction::take_holds()
    }
}

impl Drop for WriteHolds {
    fn drop(&mut self) {
        crate::transaction::unwatch_holds();
    }
}

/// A folder's stored bytes, its `path_key`, and its merged folder (`mfolders.id`, made if there is none), from its path
/// text (as the tests write it) under the catalog's rules.
pub(crate) fn folder_row(c: &Catalog, path: &str) -> Result<(Vec<u8>, Vec<u8>, i64)> {
    let path = c.rules().parse(path).ok_or_else(|| Error::Invalid(format!("{path:?} is not a test folder path")))?;
    let mid = crate::lifecycle::merged_folder(&c.conn, &c.rules().folder_key(&path.parts))?;
    Ok((c.rules().join(&path), c.rules().path_key(&path), mid))
}

/// The stored code of a kind's name (`"image"`, `"video"`), as a test names it.
fn kind_code(name: &str) -> Result<i64> {
    match name {
        "image" => Ok(pg_model::media::Kind::Image.code()),
        "video" => Ok(pg_model::media::Kind::Video.code()),
        other => Err(Error::Invalid(format!("{other:?} is not a kind"))),
    }
}

/// Insert the files (scan 1 must exist), each on volume 1 (interned as a writer interns it) and in its folder's merged
/// folder.
pub fn insert(catalog: &Catalog, files: &[File]) -> Result<()> {
    let conn = &catalog.conn;
    conn.execute("INSERT OR IGNORE INTO volumes(serial) VALUES (1)", [])?;
    for f in files {
        let mid: i64 =
            conn.query_row("SELECT mid FROM folders WHERE id = ?1", [f.folder.get()], |r| r.get(0)).map_err(|e| {
                Error::step(
                    format!("the folder {} of {}", f.folder.get(), pg_model::name::Name::from_raw(f.name).display()),
                    e,
                )
            })?;
        let name = pg_model::name::Name::from_raw(f.name);
        let ext = pg_model::media::Ext::of_name_bytes(f.name)
            .ok_or_else(|| Error::Invalid(format!("{} is not a media file", name.display())))?;
        // A new row's change number, as the scan's writer takes it: one `meta.seq` a file, as the trigger it replaces took.
        let change: i64 =
            conn.query_row("UPDATE meta SET value = value + 1 WHERE key = 'seq' RETURNING value", [], |r| r.get(0))?;
        conn.execute(
            "INSERT INTO files(id, volume, file_ref, folder_id, mid, name, ext, kind, size, mtime, missing,
                               turns, pending, seq, changed)
             VALUES (?1, (SELECT id FROM volumes WHERE serial = 1), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12)",
            params![
                f.id.get(),
                f.file_ref,
                f.folder.get(),
                mid,
                f.name,
                ext.code(),
                kind_code(f.kind)?,
                f.size,
                f.mtime,
                f.missing,
                f.turns.get(),
                change
            ],
        )
        .map_err(|e| Error::step(format!("insert {}", name.display()), e))?;
    }
    Ok(())
}

/// A file's size and mtime as a scan that found it changed records them: the state the thumbnail and failure keys name,
/// so a file rewritten on disk is seen as changed only once this has run (the `files_pending_changed` trigger fires as it would).
pub fn set_state(catalog: &Catalog, id: FileId, size: i64, mtime: i64) -> Result<()> {
    catalog
        .conn
        .execute("UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1", params![id.get(), size, mtime])
        .map_err(|e| Error::step(format!("set the state of file {}", id.get()), e))?;
    Ok(())
}
