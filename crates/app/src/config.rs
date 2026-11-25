//! What the app is opened with: the data folders (`pg_library::layout::Layout`), the thumbnail cache's cap, and where
//! the Yandex search gets its calls. The
//! environment is read here, once (`from_env`); the owner may change a field before the app is opened.

use std::path::Path;

use pg_library::layout::Layout;

use crate::library::StartConfig;
use crate::yandex::YandexSource;

/// The app's settings as the app starts with them: the layout of its data folders and the choices the user has made.
#[derive(Clone, Debug)]
pub struct Config {
    /// The data folders and the files in them (`PG_HOME`, or the default): the catalog folder, the preferences
    /// (`gallery.json`), the log (`gallery.log`), SQLite's aliases.
    pub layout: Layout,
    /// The thumbnail cache's cap; `None` keeps the cap the cache has.
    pub thumb_cap: Option<pg_model::cache::ThumbCap>,
    /// Where the Yandex search gets its calls: Yandex, unless the front end was given a recorded session.
    pub yandex: YandexSource,
    /// How the library's job starts: its settings (the pinned counts, the passes a round runs) and whether its writer runs.
    /// The app's default runs the writer; a headless run turns it off (`StartConfig::writer`).
    pub start: StartConfig,
}

impl Config {
    /// The configuration of the environment (`PG_HOME`, `PG_CATALOG_DIR`, `PG_PREFS`, and the platform's default folder).
    /// An error when no folder can be found for the data.
    pub fn from_env() -> Result<Config, String> {
        Ok(Config::from_layout(Layout::resolve(pg_platform::env::process)?))
    }

    /// The configuration of a resolved layout, with the thumbnail cache's cap unset and the live Yandex search.
    pub fn from_layout(layout: Layout) -> Config {
        Config { layout, thumb_cap: None, yandex: YandexSource::Live, start: StartConfig::default() }
    }

    /// The configuration of everything under one `home` folder, as the defaults give it (for tests and scratch runs).
    pub fn in_home(home: &Path) -> Config {
        Config::from_layout(Layout::in_home(home))
    }
}
