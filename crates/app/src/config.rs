//! What the app is opened with: the data folders (`pg_library::layout::Layout`), the thumbnail cache's cap, and where
//! the Yandex search gets its calls. The
//! environment is read here, once (`from_env`); the owner may change a field before the app is opened.

use std::path::Path;

use pg_library::layout::Layout;

use crate::yandex::YandexSource;

#[derive(Clone, Debug)]
pub struct Config {
    /// The data folders and the files in them (`PG_HOME`, or the default): the catalog folder, the preferences
    /// (`gallery.json`), the log (`gallery.log`), SQLite's aliases.
    pub layout: Layout,
    /// The thumbnail cache's cap in bytes; `None` keeps the cap the cache has.
    pub thumb_cap_bytes: Option<i64>,
    /// Where the Yandex search gets its calls: Yandex, unless the front end was given a recorded session.
    pub yandex: YandexSource,
}

impl Config {
    /// The configuration of the environment (`PG_HOME`, `PG_CATALOG_DIR`, `PG_PREFS`, and the platform's default folder).
    /// An error when no folder can be found for the data.
    pub fn from_env() -> Result<Config, String> {
        Ok(Config::from_layout(Layout::resolve(|name| std::env::var_os(name), None)?))
    }

    /// The configuration of a resolved layout, with the thumbnail cache's cap unset and the live Yandex search.
    pub fn from_layout(layout: Layout) -> Config {
        Config { layout, thumb_cap_bytes: None, yandex: YandexSource::Live }
    }

    /// The configuration of everything under one `home` folder, as the defaults give it (for tests and scratch runs).
    pub fn in_home(home: &Path) -> Config {
        Config::from_layout(Layout::in_home(home))
    }
}
