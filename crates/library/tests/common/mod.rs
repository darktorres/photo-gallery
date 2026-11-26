//! The setup the library's integration tests share. Each test file includes it with `mod common;`.

use std::path::{Path, PathBuf};

use pg_catalog::Catalog;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_platform::priority::Priority;

/// The tests' result: any error of the crates they use.
pub(crate) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The options the tests' passes run with: normal priority.
pub(crate) fn opts() -> Options {
    Options::new(Priority::Normal)
}

/// A run folder whose `images` folder `write` fills, scanned into a catalog beside it and not imported: the catalog, the
/// run's control, and the folder.
pub(crate) fn scanned(prefix: &str, write: impl FnOnce(&Path) -> Result<()>) -> Result<(Catalog, Control, PathBuf)> {
    let dir = pg_testkit::run_folder("scratch_library", prefix)?;
    let images = dir.join("images");
    std::fs::create_dir_all(&images)?;
    write(&images)?;
    let mut c = Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES)?;
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&images)?;
    c.add_root(&root)?;
    pg_scan::scan(&mut c, &root, &ctl)?;
    Ok((c, ctl, dir))
}
