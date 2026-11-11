//! The application layer: what the front ends share, without any UI. The desktop (`pg-desktop`) is Qt glue over this crate:
//! it opens an [`App`] from a [`Config`], reads what it shows through the services the app holds, and runs the work on the
//! threads these services start. Nothing here prints: what the user should hear is a notice posted to the [`Inbox`], and the
//! front end words it.
//!
//! The pieces: [`app`] (the composition root: it opens the services in order and orders their shutdown), [`inbox`] (what
//! the app tells the user), [`catalogs`] (the catalog connections), [`images`] (thumbnails, placeholders, the viewer's
//! original), [`reader`] (the catalog reads off the UI thread, and [`Reading`]), [`library`] (the folders, the scan and the
//! write-back), [`edit`] (ratings and marks), [`views`] (the place the grid shows, the saved views), [`yandex`] (the reverse
//! image search and saving its results), [`prefs`] (the preferences), [`store`] (the JSON file they are kept in), [`pool`]
//! (bounded pools of connections and makers), and [`error`].

pub mod app;
pub mod catalogs;
pub mod config;
pub mod edit;
pub mod error;
pub mod images;
pub mod inbox;
pub mod library;
mod makers;
pub mod pool;
pub mod prefs;
pub mod reader;
pub mod reexport;
pub mod store;
pub mod views;
pub mod yandex;

pub use app::App;
pub use catalogs::Catalogs;
pub use config::Config;
pub use edit::Edit;
pub use error::{Error, Result};
pub use images::{Images, Keep, Thumbnail};
pub use inbox::{Inbox, InboxEvent, Listing};
pub use library::{Library, LibraryService, Status};
pub use prefs::{PrefStore, Prefs};
pub use reader::{Kind, Reading, Session};
pub use views::Views;
