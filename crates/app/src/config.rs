//! What the app is opened with: the catalog folder, the preferences file and the thumbnail cache's cap. The environment
//! is read here, once (`from_env`); the owner may change a field before the app is opened.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    /// The catalog folder: `catalog.sqlite`, and the thumbnail cache beside it.
    pub catalog_dir: PathBuf,
    /// The preferences (`gallery.json`).
    pub prefs_path: PathBuf,
    /// The thumbnail cache's cap in bytes; `None` keeps the cap the cache has.
    pub thumb_cap_bytes: Option<i64>,
}

impl Config {
    /// The configuration from `PG_CATALOG_DIR` (default: `pg_library::default_catalog_dir`) and `PG_PREFS` (default:
    /// `gallery.json` beside the catalog folder, outside it, so the preferences outlive a catalog rebuild).
    pub fn from_env() -> Config {
        let catalog_dir =
            std::env::var_os("PG_CATALOG_DIR").map_or_else(pg_library::default_catalog_dir, PathBuf::from);
        let prefs_path = std::env::var_os("PG_PREFS").map_or_else(|| default_prefs(&catalog_dir), PathBuf::from);
        Config { catalog_dir, prefs_path, thumb_cap_bytes: None }
    }

    /// The catalog's database file.
    pub fn catalog_path(&self) -> PathBuf {
        self.catalog_dir.join("catalog.sqlite")
    }
}

fn default_prefs(catalog_dir: &Path) -> PathBuf {
    catalog_dir.parent().unwrap_or(catalog_dir).join("gallery.json")
}
