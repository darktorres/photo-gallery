//! Where the tests of the crates above ask the thumbnail cache to refuse its stores (the `fault` feature): the cache's
//! file gets a trigger that aborts every thumbnail write, as a full disk or a locked file would, so the pass gets the
//! store's error back.

use std::path::Path;

use anyhow::Result;

/// From now on, every thumbnail the cache in `cache_file` is asked to keep is refused. The trigger is in that file
/// alone, so a test that asks for it touches no other cache.
pub fn refuse_stores(cache_file: &Path) -> Result<()> {
    rusqlite::Connection::open(cache_file)?.execute_batch(
        "CREATE TRIGGER refuse_data BEFORE INSERT ON data
         BEGIN SELECT RAISE(ABORT, 'the cache refuses the thumbnail'); END;",
    )?;
    Ok(())
}

/// Undoes [`refuse_stores`] on `cache_file`.
pub fn allow_stores(cache_file: &Path) -> Result<()> {
    rusqlite::Connection::open(cache_file)?.execute_batch("DROP TRIGGER refuse_data")?;
    Ok(())
}
