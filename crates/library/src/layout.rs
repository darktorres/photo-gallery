//! Where the app keeps its data: the catalog folder (`catalog.sqlite`, and the thumbnail cache `thumbs.sqlite` beside
//! it), the preferences (`gallery.json`), the log (`gallery.log`), and the folder of SQLite's aliases (`sqlite-aliases`,
//! `pg_platform::sqlite`). All of it is resolved and named here, from the environment, so
//! the layout lives in one place, and nothing of the app's is written outside the home.
//!
//! For now they share one home folder. The preferences, the log and the thumbnail cache belong in a config, state and
//! cache folder of their own; moving each is a change to this module and its tests only.
//!
//! The home is, in order: `PG_HOME`; the parent of the catalog folder when `PG_CATALOG_DIR` gives one, so an existing
//! setup keeps its preferences beside it; the platform's default
//! ([`pg_platform::dirs::default_home`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use pg_platform::env::Var;

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
    /// default). An error when no home can be found; it is never the temporary folder.
    pub fn resolve(var: impl Fn(&str) -> Option<OsString>) -> Result<Layout, String> {
        let catalog_dir = pg_platform::env::path(&var, Var::CatalogDir);
        let home = match pg_platform::env::value(&var, Var::Home) {
            Some(home) => PathBuf::from(home),
            None => match &catalog_dir {
                Some(dir) => dir.parent().unwrap_or(dir).to_path_buf(),
                None => pg_platform::dirs::default_home(&var)?,
            },
        };
        let prefs = pg_platform::env::path(&var, Var::Prefs).unwrap_or_else(|| home.join("gallery.json"));
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

    /// Makes the catalog folder where it may be made: when it exists, when it is the default `catalog` folder under the
    /// home folder, or when `create` says so. Any other folder that is missing is not made, since it is most likely a
    /// drive that isn't connected (or a mistyped path): the error is `NotFound`, with the folder in its message.
    pub fn ensure_catalog_dir(&self, create: bool) -> std::io::Result<()> {
        if create || self.catalog_dir.is_dir() || self.catalog_dir == self.home.join("catalog") {
            return std::fs::create_dir_all(&self.catalog_dir);
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} not found - is the drive connected?", self.catalog_dir.display()),
        ))
    }

    /// The catalog's database file.
    pub fn catalog_path(&self) -> PathBuf {
        self.catalog_dir.join("catalog.sqlite")
    }

    /// The thumbnail cache's database file, beside the catalog.
    pub fn thumbs_path(&self) -> PathBuf {
        self.catalog_dir.join("thumbs.sqlite")
    }

    /// Where SQLite's aliases to a catalog folder it can't name are made (Windows: a folder whose name has no UTF-8
    /// form; `pg_platform::sqlite`): in the home, as everything else is.
    pub fn sqlite_aliases(&self) -> PathBuf {
        self.home.join("sqlite-aliases")
    }

    /// This layout with its catalog folder as SQLite opens it (`pg_platform::sqlite::openable_folder`): itself, or an
    /// alias in [`Layout::sqlite_aliases`] made for it. The catalog folder must exist. What is opened (the catalog, the
    /// cache) is opened through this one; what is shown is the layout itself.
    pub fn for_sqlite(&self) -> std::io::Result<Layout> {
        let catalog_dir = pg_platform::sqlite::openable_folder(&self.catalog_dir, &self.sqlite_aliases())?;
        Ok(Layout { catalog_dir, ..self.clone() })
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
        let layout = Layout::resolve(env(&[("PG_HOME", "/data/h"), ("PG_CATALOG_DIR", "/x/cat")])).unwrap();
        assert_eq!(layout.home(), Path::new("/data/h"));
        assert_eq!(layout.catalog_dir(), Path::new("/x/cat"));
        assert_eq!(layout.prefs(), Path::new("/data/h/gallery.json"));
        assert_eq!(layout.log(), Path::new("/data/h/gallery.log"));
    }

    #[test]
    fn the_parent_of_the_catalog_folder_is_the_home_as_before() {
        // `PG_CATALOG_DIR=D:/path/to/catalog` keeps its preferences and log beside the catalog, where they always were.
        let layout = Layout::resolve(env(&[("PG_CATALOG_DIR", "/x/run/catalog")])).unwrap();
        assert_eq!(layout.home(), Path::new("/x/run"));
        assert_eq!(layout.prefs(), Path::new("/x/run/gallery.json"));
        assert_eq!(layout.log(), Path::new("/x/run/gallery.log"));
    }

    #[test]
    fn prefs_path_overrides_only_the_preferences() {
        let layout = Layout::resolve(env(&[("PG_HOME", "/h"), ("PG_PREFS", "/p/prefs.json")])).unwrap();
        assert_eq!(layout.prefs(), Path::new("/p/prefs.json"));
        assert_eq!(layout.log(), Path::new("/h/gallery.log"));
    }

    #[test]
    fn with_nothing_set_the_platform_default_is_the_home() {
        // The default is per OS (`pg_platform::dirs`); on Windows the lookup's HOME is not used at all.
        if cfg!(windows) {
            let layout = Layout::resolve(env(&[("LOCALAPPDATA", r"C:\L")])).unwrap();
            assert_eq!(layout.home(), Path::new(r"C:\L").join("PhotoGallery"));
        } else {
            let layout = Layout::resolve(env(&[("HOME", "/home/a")])).unwrap();
            assert_eq!(layout.home(), Path::new("/home/a/.local/share/photo-gallery"));
            assert_eq!(
                layout.catalog_path(),
                PathBuf::from("/home/a/.local/share/photo-gallery/catalog/catalog.sqlite")
            );
        }
    }

    #[test]
    fn no_home_at_all_is_an_error_not_the_temporary_folder() {
        assert!(Layout::resolve(env(&[])).is_err());
    }

    #[test]
    fn in_home_puts_everything_under_one_folder() {
        let layout = Layout::in_home("/scratch/run");
        assert_eq!(layout.catalog_dir(), Path::new("/scratch/run/catalog"));
        assert_eq!(layout.catalog_path(), Path::new("/scratch/run/catalog/catalog.sqlite"));
        assert_eq!(layout.thumbs_path(), Path::new("/scratch/run/catalog/thumbs.sqlite"));
        assert_eq!(layout.prefs(), Path::new("/scratch/run/gallery.json"));
        assert_eq!(layout.log(), Path::new("/scratch/run/gallery.log"));
        assert_eq!(layout.sqlite_aliases(), Path::new("/scratch/run/sqlite-aliases"));
    }

    #[cfg(windows)]
    /// The catalog folder is made where the rule allows: the default one under the home, or any with `create`. Another
    /// missing folder is refused as `NotFound`, with nothing made (most likely its drive isn't connected).
    #[test]
    fn a_missing_catalog_folder_is_made_only_as_the_default_or_when_asked() {
        let dir = pg_testkit::run_folder("scratch_library", "catalog-folder").unwrap();
        let default = Layout::in_home(dir.join("home"));
        default.ensure_catalog_dir(false).unwrap();
        assert!(default.catalog_dir().is_dir(), "the default folder is made");

        let archive = dir.join("elsewhere").join("archive");
        let other =
            Layout::resolve(|name| (name == "PG_CATALOG_DIR").then(|| archive.clone().into_os_string())).unwrap();
        let err = other.ensure_catalog_dir(false).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(err.to_string().contains("is the drive connected?"), "{err}");
        assert!(!dir.join("elsewhere").exists(), "nothing is made for a folder that is not the default");

        other.ensure_catalog_dir(true).unwrap();
        assert!(other.catalog_dir().is_dir(), "with create, it is made");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The alias is a Windows mechanism (`pg_platform::sqlite::openable_folder`): on Unix SQLite takes the folder's bytes as
    /// they are, and the folder is not aliased there. This test therefore runs on Windows only.
    #[cfg(windows)]
    #[test]
    fn a_catalog_folder_with_no_utf8_name_is_aliased_inside_the_home() {
        let home = pg_testkit::run_folder("scratch_library", "layout").unwrap();
        // "cat" with an unpaired surrogate.
        let odd = pg_platform::testing::odd_name("cat");
        let layout = Layout::resolve(|name| match name {
            "PG_HOME" => Some(home.clone().into_os_string()),
            "PG_CATALOG_DIR" => Some(home.join(&odd).into_os_string()),
            _ => None,
        })
        .unwrap();
        std::fs::create_dir_all(layout.catalog_dir()).unwrap();
        let sqlite = layout.for_sqlite().unwrap();
        assert!(sqlite.catalog_dir().starts_with(layout.sqlite_aliases()), "{}", sqlite.catalog_dir().display());
        let catalog = crate::open_catalog(&sqlite).unwrap();
        drop(catalog);
        assert!(layout.catalog_path().exists(), "the catalog is in its own folder, through the alias");
        let _ = std::fs::remove_dir_all(&home);
    }
}
