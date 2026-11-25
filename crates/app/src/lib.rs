//! The application layer the window uses, without any UI (the command line drives the passes directly through
//! `pg-library`). The desktop (`pg-desktop`) is Qt glue over this crate:
//! it opens an [`App`] from a [`Config`], reads what it shows through the services the app holds, and runs the work on the
//! threads these services start. Nothing here prints: what the user should hear is a notice posted to the [`Inbox`], and the
//! front end words it.
//!
//! The pieces: the composition root (it opens the services in order and orders their shutdown), the inbox (what the app
//! tells the user, [`Inbox`]), the catalog connections, the thumbnails, placeholders and the viewer's original, the catalog
//! reads off the UI thread ([`Reading`]), the folders, the scan and the write-back, ratings and marks, the place the grid
//! shows and the saved views, the reverse image search and saving its results (`yandex`, a public module), the preferences
//! and the JSON file they are kept in (`store`, a public module), the reader counts the lanes learned per disk (`profiles`,
//! a public module), bounded pools of connections and makers (`pool`, a public module), and the error type ([`Error`]).

#![forbid(unsafe_code)]

mod app;
mod catalogs;
mod config;
mod edit;
mod error;
mod images;
mod inbox;
mod library;
mod makers;
pub mod pool;
mod prefs;
pub mod profiles;
mod reader;
pub mod store;
mod views;
pub mod yandex;

pub use app::App;
pub use catalogs::Catalogs;
pub use config::Config;
pub use edit::Edit;
pub use error::{Error, Result};
pub use images::{Images, Keep, Thumbnail};
pub use inbox::{Inbox, InboxEvent, InboxListing};
pub use library::{LaneLines, Library, LibraryActivity, LibraryService, StartConfig};
pub use prefs::{PrefStore, Prefs};
pub use reader::{Failed, FailedItems, Kind, Reading, Session};
pub use views::{Views, query_from_json, sort_from_json};

/// Whether the job on this thread caught a panic: the front end's one panic check (`pg_jobs::panics::caught`), which the
/// app makes available so that the front end need not depend on the job crate.
pub fn panic_caught() -> bool {
    pg_jobs::panics::caught()
}

/// What the app's test modules share.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;

    use pg_library::layout::Layout;

    /// A test folder of its own under the app's scratch base, with its catalog made and closed: the folder, and the
    /// layout that `pg_library::open_catalog` opens it by.
    pub(crate) fn catalog_home(name: &str) -> (PathBuf, Layout) {
        let dir = pg_testkit::run_folder("scratch_app", name).unwrap();
        let layout = Layout::in_home(&dir);
        std::fs::create_dir_all(layout.catalog_dir()).unwrap();
        drop(pg_library::open_catalog(&layout).unwrap());
        (dir, layout)
    }
}
