//! Where the app keeps its data: the catalog folder (`catalog.sqlite`, and the thumbnail cache beside it), the preferences
//! (`gallery.json`) and the log (`gallery.log`). All of it is resolved here, from the environment and the `--catalog-dir`
//! flag, so the layout lives in one place.
//!
//! For now they share one home folder. The preferences, the log and the thumbnail cache belong in a config, state and
//! cache folder of their own; moving each is a change to this module and its tests only.
//!
//! The home is, in order: `PG_HOME`; the parent of the catalog folder when one is given (`--catalog-dir` or
//! `PG_CATALOG_DIR`), so an existing setup keeps its preferences beside it; the platform's default
//! ([`pg_platform::dirs::default_home`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The data folders of one run. Build it with [`Layout::resolve`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    home: PathBuf,
    catalog_dir: PathBuf,
    prefs: PathBuf,
    log: PathBuf,
}

impl Layout {
    /// The layout under the environment `var` gives (`PG_HOME`, `PG_CATALOG_DIR`, `PG_PREFS`, and the platform's
    /// default), and `catalog_dir` when the caller was given one (the `--catalog-dir` flag, which wins over
    /// `PG_CATALOG_DIR`). An error when no home can be found; it is never the temporary folder.
    pub fn resolve(var: impl Fn(&str) -> Option<OsString>, catalog_dir: Option<PathBuf>) -> Result<Layout, String> {
        let catalog_dir = catalog_dir.or_else(|| var("PG_CATALOG_DIR").map(PathBuf::from));
        let home = match var("PG_HOME") {
            Some(home) => PathBuf::from(home),
            None => match &catalog_dir {
                Some(dir) => dir.parent().unwrap_or(dir).to_path_buf(),
                None => pg_platform::dirs::default_home(&var)?,
            },
        };
        let prefs = var("PG_PREFS").map_or_else(|| home.join("gallery.json"), PathBuf::from);
        Ok(Layout {
            catalog_dir: catalog_dir.unwrap_or_else(|| home.join("catalog")),
            prefs,
            log: home.join("gallery.log"),
            home,
        })
    }

    /// The layout of a home folder with everything under it, as the defaults give; for tests and scratch runs.
    pub fn in_home(home: impl Into<PathBuf>) -> Layout {
        let home = home.into();
        Layout {
            catalog_dir: home.join("catalog"),
            prefs: home.join("gallery.json"),
            log: home.join("gallery.log"),
            home,
        }
    }

    /// The folder the rest is under (the app shows it at startup).
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The catalog folder: `catalog.sqlite`, and the thumbnail cache (`thumbs.sqlite`) beside it.
    pub fn catalog_dir(&self) -> &Path {
        &self.catalog_dir
    }

    /// The catalog's database file.
    pub fn catalog_path(&self) -> PathBuf {
        self.catalog_dir.join("catalog.sqlite")
    }

    /// The preferences file.
    pub fn prefs(&self) -> &Path {
        &self.prefs
    }

    /// The app's log.
    pub fn log(&self) -> &Path {
        &self.log
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: BTreeMap<String, OsString> = pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn pg_home_wins_over_everything_else() {
        let layout = Layout::resolve(env(&[("PG_HOME", "/data/h"), ("PG_CATALOG_DIR", "/x/cat")]), None).unwrap();
        assert_eq!(layout.home(), Path::new("/data/h"));
        assert_eq!(layout.catalog_dir(), Path::new("/x/cat"));
        assert_eq!(layout.prefs(), Path::new("/data/h/gallery.json"));
        assert_eq!(layout.log(), Path::new("/data/h/gallery.log"));
    }

    #[test]
    fn the_parent_of_the_catalog_folder_is_the_home_as_before() {
        // `PG_CATALOG_DIR=D:/path/to/catalog` keeps its preferences and log beside the catalog, where they always were.
        let layout = Layout::resolve(env(&[("PG_CATALOG_DIR", "/x/run/catalog")]), None).unwrap();
        assert_eq!(layout.home(), Path::new("/x/run"));
        assert_eq!(layout.prefs(), Path::new("/x/run/gallery.json"));
        assert_eq!(layout.log(), Path::new("/x/run/gallery.log"));
    }

    #[test]
    fn the_flag_wins_over_the_variable_and_moves_the_home_too() {
        let layout =
            Layout::resolve(env(&[("PG_CATALOG_DIR", "/x/a/catalog")]), Some(PathBuf::from("/y/b/catalog"))).unwrap();
        assert_eq!(layout.catalog_dir(), Path::new("/y/b/catalog"));
        assert_eq!(layout.home(), Path::new("/y/b"));
    }

    #[test]
    fn prefs_path_overrides_only_the_preferences() {
        let layout = Layout::resolve(env(&[("PG_HOME", "/h"), ("PG_PREFS", "/p/prefs.json")]), None).unwrap();
        assert_eq!(layout.prefs(), Path::new("/p/prefs.json"));
        assert_eq!(layout.log(), Path::new("/h/gallery.log"));
    }

    #[test]
    fn with_nothing_set_the_platform_default_is_the_home() {
        // The default is per OS (`pg_platform::dirs`); on Windows the lookup's HOME is not used at all.
        if cfg!(windows) {
            let layout = Layout::resolve(env(&[("LOCALAPPDATA", r"C:\L")]), None).unwrap();
            assert_eq!(layout.home(), Path::new(r"C:\L").join("PhotoGallery"));
        } else {
            let layout = Layout::resolve(env(&[("HOME", "/home/a")]), None).unwrap();
            assert_eq!(layout.home(), Path::new("/home/a/.local/share/photo-gallery"));
            assert_eq!(
                layout.catalog_path(),
                PathBuf::from("/home/a/.local/share/photo-gallery/catalog/catalog.sqlite")
            );
        }
    }

    #[test]
    fn no_home_at_all_is_an_error_not_the_temporary_folder() {
        assert!(Layout::resolve(env(&[]), None).is_err());
    }

    #[test]
    fn in_home_puts_everything_under_one_folder() {
        let layout = Layout::in_home("/scratch/run");
        assert_eq!(layout.catalog_dir(), Path::new("/scratch/run/catalog"));
        assert_eq!(layout.prefs(), Path::new("/scratch/run/gallery.json"));
        assert_eq!(layout.log(), Path::new("/scratch/run/gallery.log"));
    }
}
