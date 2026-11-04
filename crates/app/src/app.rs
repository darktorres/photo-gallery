//! The application: what the front ends share. It owns the catalog connections (a keeper, and a pool the image and read
//! work borrows from), the thumbnail cache, the thumbnail makers, the server's preferences, the catalog reader, and the
//! library (the scan and the write-back). The front end holds one `App` for the process; nothing here is a `static`.
//!
//! Shutdown has an order, which the owner follows (`stop`, then drop what borrows the app, then `shutdown`): background
//! work first, then the thumbnail cache, then the catalog connections with the keeper last. The last connection to a
//! catalog file deletes its WAL, so the keeper is the one that must close last.
//!
//! The library prints nothing (its crates deny printing); what happens that the user should hear about is a [`Note`] on
//! the app's notes bus ([`App::notes`]), which the front end prints or shows.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use pg_catalog::Catalog;
use pg_jobs::events::{Bus, Subscription};
use pg_render::maker::Maker;
use pg_thumbcache::ThumbCache;

use crate::config::Config;
use crate::error::{Error, Result, failed};
use crate::images::Decoded;
use crate::library::Library;
use crate::pool::Pool;
use crate::prefs::{Prefs, Store};
use crate::reader::{Reader, Session};

/// What the user should hear about, besides the results of what they asked for. Printed or shown by the front end.
#[derive(Clone, Debug)]
pub enum Note {
    /// The preferences file didn't parse: it was kept aside (`kept`), and the defaults are in use.
    PrefsUnreadable { path: PathBuf, error: String, kept: Result<PathBuf, String> },
    /// The thumbnail cache reported something (a repair, a write it couldn't make, a WAL it couldn't merge).
    Thumbs(pg_thumbcache::Event),
    /// The preferences changed in memory but the file couldn't be written; the change holds until the app closes.
    PrefsNotSaved { path: PathBuf, error: String },
}

/// Idle catalog connections kept for the next borrower; more are closed when returned.
const KEEP_CATALOGS: usize = 4;
/// Idle thumbnail makers kept (each holds at most `pg_render::maker::KEEP_BYTES` of buffers).
const KEEP_MAKERS: usize = 8;

pub struct App {
    pub(crate) config: Config,
    /// The connection opened first, kept for the app's life; closed last at shutdown.
    keeper: Mutex<Option<Catalog>>,
    catalogs: Pool<Catalog>,
    pub(crate) makers: Pool<Maker>,
    /// The thumbnail cache, leaked to a `'static` reference so the library's threads can borrow it. Closed at shutdown.
    pub(crate) thumbs: &'static ThumbCache,
    /// The last full-size decode (`images::full_image`).
    pub(crate) last_decode: Mutex<Option<Decoded>>,
    prefs: Store,
    reader: Arc<Reader>,
    session: Session,
    pub(crate) library_cell: OnceLock<Library>,
    notes: Arc<Bus<Note>>,
    /// The subscription taken at the opening, handed to the first [`App::notes`] caller.
    first_notes: Mutex<Option<Subscription<Note>>>,
}

impl App {
    /// Open (and migrate) the catalog, the thumbnail cache, and the preferences. A cache that can't be opened only means
    /// thumbnails aren't kept (a note says why). The library starts on first use ([`App::library`]).
    pub fn open(config: Config) -> Result<App> {
        std::fs::create_dir_all(&config.catalog_dir)
            .map_err(|e| failed(format!("{}: {e}", config.catalog_dir.display())))?;
        let catalog_path = config.catalog_path();
        let keeper = Catalog::prepare(&catalog_path, pg_library::versions())?;
        let catalog_id = keeper.catalog_id()?;
        let notes: Arc<Bus<Note>> = Arc::new(Bus::new(64));
        let first_notes = notes.subscribe();
        let thumb_events = Arc::new(Bus::new(64));
        forward_thumb_events(thumb_events.clone(), notes.clone());
        let thumbs = ThumbCache::open(&config.catalog_dir, &catalog_id, config.thumb_cap_bytes, thumb_events)
            .unwrap_or_else(|e| {
                notes
                    .publish(Note::Thumbs(pg_thumbcache::Event::NotStored { error: format!("cache disabled: {e:#}") }));
                ThumbCache::disabled()
            });
        let thumbs: &'static ThumbCache = Box::leak(Box::new(thumbs));
        let (prefs, note) = Store::load(config.prefs_path.clone());
        if let Some(note) = note {
            notes.publish(note);
        }
        let reader = Reader::start(catalog_path);
        Ok(App {
            config,
            keeper: Mutex::new(Some(keeper)),
            catalogs: Pool::new(KEEP_CATALOGS),
            makers: Pool::new(KEEP_MAKERS),
            thumbs,
            last_decode: Mutex::new(None),
            prefs,
            session: Session::new(1, reader.clone()),
            reader,
            library_cell: OnceLock::new(),
            notes,
            first_notes: Mutex::new(Some(first_notes)),
        })
    }

    /// What the app has to say, from the first note on: the first call gets the subscription the app held since it opened (so
    /// the notes of the opening are not lost); later calls subscribe from now on.
    pub fn notes(&self) -> Subscription<Note> {
        self.first_notes.lock().unwrap().take().unwrap_or_else(|| self.notes.subscribe())
    }

    /// The catalog reader's session (the front end's one).
    pub fn session(&self) -> Session {
        self.session.clone()
    }

    /// The preferences, as they are now.
    pub fn prefs(&self) -> Prefs {
        self.prefs.get()
    }

    /// Change the preferences and save them. The change is kept in memory even when the save fails; the error says why
    /// it wasn't written.
    pub fn update_prefs(&self, change: impl FnOnce(&mut Prefs)) -> std::io::Result<()> {
        let result = self.prefs.update(change);
        if let Err(e) = &result {
            self.notes.publish(Note::PrefsNotSaved { path: self.config.prefs_path.clone(), error: e.to_string() });
        }
        result
    }

    /// Run `f` with a catalog connection: an idle one, or one opened for it (and kept for the next borrower, up to a few).
    pub fn with_catalog<R>(&self, f: impl FnOnce(&Catalog) -> pg_catalog::Result<R>) -> Result<R> {
        let path = self.config.catalog_path();
        self.catalogs.with(|| Catalog::open(&path).map_err(Error::from), |c| f(c).map_err(Error::from))
    }

    /// Run `f` with a thumbnail maker (and its decoder): an idle one, or a new one.
    pub(crate) fn with_maker<R>(&self, f: impl FnOnce(&mut Maker) -> anyhow::Result<R>) -> Result<R> {
        self.makers.with(|| Maker::new().map_err(failed), |m| f(m).map_err(failed))
    }

    /// Stop the background work: the catalog reader, then the library's writer and job. Waits for each to end. After this
    /// nothing runs on its own, so the front end can drop what borrows the app and then [`App::shutdown`]. An error says
    /// the job was still running after its wait (its last changes are merged at the next open).
    pub fn stop(&self) -> Result<()> {
        self.reader.stop();
        self.stop_library()
    }

    /// The end, after [`App::stop`] and once nothing borrows the app's images (so the front end may keep it in a `static`):
    /// the thumbnail cache writes what it holds and merges its WAL; then the catalog connections close, the keeper last,
    /// so the last connection to the file deletes its WAL. Afterwards the app has no connections; it must not be used.
    pub fn shutdown(&self) {
        self.thumbs.close();
        for catalog in self.catalogs.drain() {
            let _ = catalog.close();
        }
        if let Some(keeper) = self.keeper.lock().unwrap().take()
            && let Err(e) = keeper.close()
        {
            self.notes.publish(Note::Thumbs(pg_thumbcache::Event::WalNotMerged { error: format!("catalog: {e:#}") }));
        }
    }
}

/// Forward the thumbnail cache's events to the notes bus, on a thread that ends once the cache is gone.
fn forward_thumb_events(events: Arc<Bus<pg_thumbcache::Event>>, notes: Arc<Bus<Note>>) {
    let sub = events.subscribe();
    std::thread::Builder::new()
        .name("thumbnail events".into())
        .spawn(move || {
            while let Some(event) = sub.recv() {
                notes.publish(Note::Thumbs(event));
            }
        })
        .expect("spawn the thumbnail event forwarder");
}
