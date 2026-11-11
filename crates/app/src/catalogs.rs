//! The catalog connections: the keeper (opened first, kept for the app's life, closed last) and a bounded pool that the
//! image and read work borrows from. Every connection is to the one catalog file, opened under the native rules.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use pg_catalog::Catalog;
use pg_catalog::ledger::FailedFile;
use pg_model::ids::FileId;
use pg_model::notice::FilePass;
use pg_model::rules::Path as RulePath;

use crate::error::{Error, Result};
use crate::pool::Pool;

/// Idle catalog connections kept for the next borrower; more are closed when returned.
const KEEP_CATALOGS: usize = 4;

/// The path the file at a catalog path is opened by (`pg_platform::names::disk_path`): the catalogs are opened under
/// the native rules (`with_catalog`), so their paths are the native ones.
pub(crate) fn path_of(path: &RulePath) -> PathBuf {
    pg_platform::names::disk_path(pg_platform::native::PATH_RULES, path)
}

/// The catalog connections of the app.
pub struct Catalogs {
    path: PathBuf,
    /// The connection opened first, kept for the app's life; closed last at shutdown.
    keeper: Mutex<Option<Catalog>>,
    pool: Pool<Catalog>,
}

impl Catalogs {
    /// The connections around `keeper`, which is open on the catalog at `path`.
    pub(crate) fn new(path: PathBuf, keeper: Catalog) -> Catalogs {
        Catalogs { path, keeper: Mutex::new(Some(keeper)), pool: Pool::new(KEEP_CATALOGS) }
    }

    /// Run `f` with a catalog connection: an idle one, or one opened for it (and kept for the next borrower, up to a few).
    pub(crate) fn with_catalog<R>(&self, f: impl FnOnce(&Catalog) -> pg_catalog::Result<R>) -> Result<R> {
        self.pool.with(
            || Catalog::open(&self.path, pg_platform::native::PATH_RULES).map_err(Error::from),
            |c| f(c).map_err(Error::from),
        )
    }

    /// The path a file is opened by, for the programs that open it (std, FFmpeg, the animation and video players): its
    /// `disk_path`, so a folder whose name ends in a dot or a space is the folder it names. Not for display (it has the
    /// verbatim prefix on Windows). `UnknownFile` when the catalog has no such file.
    pub fn file_path(&self, file_id: FileId) -> Result<PathBuf> {
        let path = self.with_catalog(|c| c.file_path(file_id))?.ok_or(Error::UnknownFile(file_id))?;
        Ok(path_of(&path))
    }

    /// The folder of a file the library shows (present), where a reveal opens it; `None` for one gone or missing.
    pub fn present_folder(&self, file_id: FileId) -> Result<Option<RulePath>> {
        self.with_catalog(|c| c.present_file_folder(file_id))
    }

    /// The failed files of `pass`'s inbox entry, newest first, at most `limit` (`pg_catalog::Catalog::failed_files`): what the
    /// entry lists when it is opened.
    pub fn failed_files(&self, pass: FilePass, limit: usize) -> Result<Vec<FailedFile>> {
        self.with_catalog(|c| c.failed_files(pass, limit))
    }

    /// Close the pooled connections, then the keeper, so the last connection to the file deletes its WAL. Returns the
    /// keeper's error when it could not close (the pooled connections' errors are not reported).
    pub(crate) fn close(&self) -> Option<pg_catalog::Error> {
        for catalog in self.pool.drain() {
            let _ = catalog.close();
        }
        let keeper = self.keeper.lock().unwrap_or_else(PoisonError::into_inner).take()?;
        keeper.close().err()
    }
}
