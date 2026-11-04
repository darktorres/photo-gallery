//! The application layer: what the front ends share, without any UI. The desktop (`pg-desktop`) is Qt glue over this crate:
//! it opens an [`App`] from a [`Config`], reads what it shows through the methods here, and runs the work on the threads
//! these methods start. Nothing here prints: what the user should hear is a [`Note`] on the app's notes bus, and the
//! front end decides how to say it.
//!
//! The pieces: [`app`] (the owner: connections, the thumbnail cache, the preferences, the reader, the library),
//! [`images`] (thumbnails, placeholders, the viewer's original), [`reader`] (the catalog reads off the UI thread),
//! [`library`] (the folders, the scan and the write-back), [`edit`] (ratings and marks), [`views`] (the place the grid
//! shows, the saved views), [`yandex`] (the reverse image search and saving its results), [`prefs`] (the server's
//! preferences), [`pool`] (bounded pools of connections and makers), and [`error`].

pub mod app;
pub mod config;
pub mod edit;
pub mod error;
pub mod images;
pub mod library;
pub mod pool;
pub mod prefs;
pub mod reader;
pub mod views;
pub mod yandex;

pub use app::{App, Note};
pub use config::Config;
pub use error::{Error, Result};
pub use images::{Keep, Thumbnail, Upload};
pub use library::{Library, Status};
pub use prefs::Prefs;
pub use reader::{Kind, Session};
