//! What the app is opened with: the data folders (`pg_library::layout::Layout`) and the thumbnail cache's cap. The
//! environment is read here, once (`from_env`); the owner may change a field before the app is opened.

use std::path::{Path, PathBuf};

use pg_library::layout::Layout;

#[derive(Clone, Debug)]
pub struct Config {
    /// The folder the data is under (`PG_HOME`, or the default), shown at startup.
    pub home: PathBuf,
    /// The catalog folder: `catalog.sqlite`, and the thumbnail cache beside it.
    pub catalog_dir: PathBuf,
    /// The preferences (`gallery.json`).
    pub prefs_path: PathBuf,
    /// The app's log (`gallery.log`).
    pub log_path: PathBuf,
    /// The thumbnail cache's cap in bytes; `None` keeps the cap the cache has.
    pub thumb_cap_bytes: Option<i64>,
}

impl Config {
    /// The configuration of the environment (`PG_HOME`, `PG_CATALOG_DIR`, `PG_PREFS`, and the platform's default folder).
    /// An error when no folder can be found for the data.
    pub fn from_env() -> Result<Config, String> {
        Ok(Config::from_layout(&Layout::resolve(|name| std::env::var_os(name), None)?))
    }

    /// The configuration of a resolved layout, with the thumbnail cache's cap unset.
    pub fn from_layout(layout: &Layout) -> Config {
        Config {
            home: layout.home().to_path_buf(),
            catalog_dir: layout.catalog_dir().to_path_buf(),
            prefs_path: layout.prefs().to_path_buf(),
            log_path: layout.log().to_path_buf(),
            thumb_cap_bytes: None,
        }
    }

    /// The catalog's database file.
    pub fn catalog_path(&self) -> PathBuf {
        pg_library::catalog_path(&self.catalog_dir)
    }

    /// The configuration of everything under one `home` folder, as the defaults give it (for tests and scratch runs).
    pub fn in_home(home: &Path) -> Config {
        Config::from_layout(&Layout::in_home(home))
    }
}
