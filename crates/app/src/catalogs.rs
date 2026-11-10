//! The catalog connections: the keeper (opened first, kept for the app's life, closed last) and a bounded pool that the
//! image and read work borrows from. Every connection is to the one catalog file, opened under the native rules.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use pg_catalog::Catalog;
use pg_model::ids::FileId;
use pg_model::rules::Path as RulePath;

use crate::error::{Error, Result};
use crate::pool::Pool;

/// Idle catalog connections kept for the next borrower; more are closed when returned.
const KEEP_CATALOGS: usize = 4;

/// A catalog path as the system holds it: the catalogs are opened under the native rules (`with_catalog`), so their paths
/// are the native ones.
pub(crate) fn path_of(path: &RulePath) -> PathBuf {
    pg_platform::names::path_buf(&pg_platform::native::PATH_RULES.join(path))
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
        self.pool
            .with(|| Catalog::open(&self.path, pg_platform::native::PATH_RULES).map_err(Error::from), |c| f(c).map_err(Error::from))
    }

    /// A file's path as the system holds it, for the programs that open the file (std, FFmpeg, the animation player).
    /// The catalog's paths are exact, so this is where they become a `PathBuf`. `UnknownFile` when the catalog has no
    /// such file.
    pub fn file_path(&self, file_id: FileId) -> Result<PathBuf> {
        let path = self.with_catalog(|c| c.file_path(file_id))?.ok_or(Error::UnknownFile(file_id.get()))?;
        Ok(path_of(&path))
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
