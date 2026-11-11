//! The application: the composition root of the app's services. It opens them in dependency order and owns them, and it is
//! the one place that orders their shutdown: background work first ([`App::stop`]), then the thumbnail cache and what it
//! still tells, then the inbox, then the catalog connections with the keeper last ([`App::shutdown`]). The last connection to a catalog file deletes its WAL, so
//! the keeper is the one that must close last. The front end holds one `App` for the process; nothing here is a `static`.
//!
//! The services are reached through the accessors: [`App::inbox`], [`App::catalogs`], [`App::prefs`], [`App::images`],
//! [`App::reading`], [`App::views`], [`App::edit`], [`App::library`] and [`App::yandex`]. Each service holds `Arc` handles to the services
//! below it, never to the `App`.
//!
//! The library prints nothing (its crates deny printing); what happens that the user should hear about is a
//! [`pg_model::notice::Notice`] posted to the inbox, which the front end words and shows.

use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use pg_jobs::events::Bus;
use pg_thumbcache::ThumbCache;

use crate::catalogs::Catalogs;
use crate::config::Config;
use crate::edit::Edit;
use crate::error::{Result, failed};
use crate::images::Images;
use crate::inbox::Inbox;
use crate::library::LibraryService;
use crate::makers::Makers;
use crate::prefs::PrefStore;
use crate::reader::Reading;
use crate::views::Views;
use crate::yandex::Yandex;
use pg_model::notice::{ERROR_CAP, Notice, tidy};

/// The app's services, opened by [`App::open`].
pub struct App {
    inbox: Arc<Inbox>,
    /// The thread that posts the thumbnail cache's events; it ends when the cache closes, and is joined at shutdown.
    forwarder: Mutex<Option<JoinHandle<()>>>,
    /// The thumbnail cache, leaked to a `'static` reference so the library's threads can borrow it. Closed at shutdown.
    thumbs: &'static ThumbCache,
    catalogs: Arc<Catalogs>,
    prefs: Arc<PrefStore>,
    images: Images,
    reading: Reading,
    views: Views,
    edit: Edit,
    library: Arc<LibraryService>,
    yandex: Yandex,
}

impl App {
    /// Open (and migrate) the catalog, the thumbnail cache, and the preferences. A cache that can't be opened only means
    /// thumbnails aren't kept (a notice says why). The library starts on first use ([`LibraryService::start`]).
    pub fn open(config: Config) -> Result<App> {
        let shown = config.layout.catalog_dir();
        std::fs::create_dir_all(shown).map_err(|e| failed(format!("{}: {e}", shown.display())))?;
        // The catalog folder as SQLite opens it (an alias in the home for a folder it can't name).
        let layout = config.layout.for_sqlite().map_err(|e| failed(format!("{}: {e}", shown.display())))?;
        let catalog_path = layout.catalog_path();
        let keeper = pg_library::open_catalog(&layout)?;
        let catalog_id = keeper.catalog_id()?;
        let rebuilt = keeper.rebuilt().cloned();
        let catalogs = Arc::new(Catalogs::new(catalog_path.clone(), keeper));
        let inbox = Inbox::start(catalog_path.clone())?;
        if let Some(r) = rebuilt {
            inbox.post(r.notice());
        }
        let thumb_events = Arc::new(Bus::new(64));
        let forwarder = forward_thumb_events(thumb_events.clone(), inbox.clone())?;
        let thumbs = pg_library::open_thumbs(&layout, &catalog_id, config.thumb_cap_bytes, thumb_events)
            .unwrap_or_else(|e| {
                inbox.post(Notice::CacheDisabled { error: tidy(&format!("{e:#}"), ERROR_CAP) });
                ThumbCache::disabled()
            });
        let thumbs: &'static ThumbCache = Box::leak(Box::new(thumbs));
        let prefs = Arc::new(PrefStore::open(config.layout.prefs().to_path_buf(), inbox.clone()));
        let makers = Arc::new(Makers::new());
        let reading = Reading::start(catalog_path.clone(), catalogs.clone())?;
        let library = Arc::new(LibraryService::new(catalog_path, catalogs.clone(), thumbs, inbox.clone()));
        let yandex = Yandex::new(&config.yandex, prefs.clone(), makers.clone());
        let images = Images::new(catalogs.clone(), makers, thumbs, inbox.clone());
        Ok(App {
            inbox,
            forwarder: Mutex::new(Some(forwarder)),
            thumbs,
            images,
            views: Views::new(catalogs.clone()),
            edit: Edit::new(catalogs.clone(), prefs.clone(), library.clone()),
            catalogs,
            prefs,
            reading,
            library,
            yandex,
        })
    }

    /// What the app tells the user: post to it, read what it lists, and mark and dismiss its entries.
    pub fn inbox(&self) -> &Inbox {
        &self.inbox
    }
    /// The catalog connections, and a file's path as the system holds it.
    pub fn catalogs(&self) -> &Catalogs {
        &self.catalogs
    }

    /// The preferences: read them, and change them and save them.
    pub fn prefs(&self) -> &PrefStore {
        &self.prefs
    }

    /// The images the front end shows: thumbnails, placeholders, and the viewer's original.
    pub fn images(&self) -> &Images {
        &self.images
    }

    /// The catalog reads: the reader's session (the grid, the folder tree, the counts), and a file's details and entry.
    pub fn reading(&self) -> &Reading {
        &self.reading
    }

    /// Where the grid looks (the listing), and the saved views.
    pub fn views(&self) -> &Views {
        &self.views
    }

    /// What the user changes about a file: its rating and its border mark.
    pub fn edit(&self) -> &Edit {
        &self.edit
    }

    /// The library: its folders, and its job and writer, which start on first use.
    pub fn library(&self) -> &LibraryService {
        &self.library
    }

    /// The reverse image search on Yandex, and the saves of its results.
    pub fn yandex(&self) -> &Yandex {
        &self.yandex
    }

    /// Stop the background work: the Yandex search, the catalog reader, then the library's writer and job. Waits for each
    /// to end (a Yandex request only a while: it is left to end with the process). After this
    /// nothing runs on its own, so the front end can drop what borrows the app and then [`App::shutdown`]. An error says
    /// the job was still running after its wait (its last changes are merged at the next open).
    pub fn stop(&self) -> Result<()> {
        if !self.yandex.stop() {
            pg_log::warn("yandex", "a search, a save or a fetch was still running at the stop");
        }
        self.reading.stop();
        self.library.stop()
    }

    /// The end, after [`App::stop`] and once nothing borrows the app's images (so the front end may keep it in a `static`):
    /// the thumbnail cache writes what it holds and merges its WAL; then the catalog connections close, the keeper last,
    /// so the last connection to the file deletes its WAL. Afterwards the app has no connections; it must not be used.
    pub fn shutdown(&self) {
        self.thumbs.close();
        // The cache's last events are posted before the inbox writes what it has queued and stops.
        if let Some(forwarder) = self.forwarder.lock().unwrap_or_else(PoisonError::into_inner).take()
            && forwarder.join().is_err()
        {
            pg_log::error("app", "the thumbnail events' thread panicked");
        }
        self.inbox.close();
        if let Some(e) = self.catalogs.close() {
            // The inbox has closed: the log is all that hears it.
            let notice = Notice::CatalogNotClosed { error: tidy(&format!("{e:#}"), ERROR_CAP) };
            pg_log::warn("catalog", &serde_json::to_string(&notice).unwrap_or_else(|_| format!("{e:#}")));
        }
    }
}

/// Post the thumbnail cache's events to the inbox, on a thread that ends when the cache's bus is dropped. The writer
/// thread owns that bus and drops it when it returns; `App::shutdown` stops it by closing the cache. An error if the
/// thread can't be started.
fn forward_thumb_events(events: Arc<Bus<pg_thumbcache::Event>>, inbox: Arc<Inbox>) -> Result<JoinHandle<()>> {
    let sub = events.subscribe();
    std::thread::Builder::new()
        .name("thumbnail events".into())
        .spawn(move || {
            while let Some(event) = sub.recv() {
                inbox.post(event.into_notice());
            }
        })
        .map_err(|e| failed(format!("can't start the thumbnail event forwarder: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogs::path_of;
    use crate::error::Error;
    use pg_model::ids::FileId;

    // A catalog path becomes the exact system path, a name with no UTF-8 form included, in the form that reaches the
    // file exactly (the verbatim one on Windows).
    #[test]
    fn a_catalog_path_is_the_exact_system_path() {
        // A lone surrogate (WTF-8 on Windows; the same bytes are a non-UTF-8 name on Unix).
        let text: &[u8] = if cfg!(windows) { b"D:\\a\\x\xED\xA0\x80.jpg" } else { b"/a/x\xED\xA0\x80.jpg" };
        let path = pg_platform::native::PATH_RULES.parse_bytes(text).unwrap();
        assert_eq!(pg_platform::native::PATH_RULES.join(&path), text);
        assert_eq!(path_of(&path), pg_platform::names::disk_path_of(text));
        let disk: &[u8] = if cfg!(windows) { b"\\\\?\\D:\\a\\x\xED\xA0\x80.jpg" } else { text };
        assert_eq!(pg_platform::names::canonical_of(&path_of(&path)), disk);
    }

    // A file the catalog holds opens by its exact path, through the app's catalog connections: a folder and a name with
    // no UTF-8 form (a lone surrogate on Windows, a raw byte on Unix).
    #[test]
    fn a_file_of_the_catalog_opens_by_its_exact_path() {
        use pg_catalog::fixture::{self, File};
        use pg_model::ids::FolderId;
        let dir = pg_testkit::test_folder("scratch_app", "file-path").unwrap();
        let app = App::open(Config::in_home(&dir)).unwrap();
        let folder = if cfg!(windows) { r"D:\a" } else { "/a" };
        let name: &'static [u8] = b"x\xED\xA0\x80.jpg";
        app.catalogs()
            .with_catalog(|c| {
                fixture::state::add_scan(c, 1, folder).unwrap();
                fixture::state::add_folder(c, 1, folder).unwrap();
                fixture::insert(c, &[File::named(FileId::new(1), FolderId::new(1), name, 10, 0)]).unwrap();
                Ok(())
            })
            .unwrap();
        let text: &[u8] = if cfg!(windows) { b"D:\\a\\x\xED\xA0\x80.jpg" } else { b"/a/x\xED\xA0\x80.jpg" };
        assert_eq!(app.catalogs().file_path(FileId::new(1)).unwrap(), pg_platform::names::disk_path_of(text));
        assert!(matches!(app.catalogs().file_path(FileId::new(2)), Err(Error::UnknownFile(id)) if id == FileId::new(2)));
        app.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
