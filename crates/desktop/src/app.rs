//! The window's one `pg_app::App`, held for the process, and what the window says about what the app notes. The app
//! itself has no UI: the notes it publishes are printed here, on stderr, as the command line always did.
//!
//! The order of shutdown is in `lib.rs` (`run`): [`stop`] (the reader, the library), then the engine goes (the image
//! providers), then [`shutdown`] (the thumbnail cache, then the catalog connections, the keeper last).

use std::sync::OnceLock;

use pg_app::reexport::ThumbStartOverReason;
use pg_app::{App, Config, Note};
use pg_client::text::{self, CacheStartOver};

static APP: OnceLock<App> = OnceLock::new();

/// Open the app from `config` and start printing its notes. Call once, on the GUI thread, before the UI starts.
pub(crate) fn open(config: Config) -> anyhow::Result<()> {
    let app = App::open(config)?;
    let notes = app.notes().subscribe();
    std::thread::Builder::new().name("app notes".into()).spawn(move || {
        while let Some(note) = notes.recv() {
            crate::log::note(&note_text(&note));
        }
    })?;
    APP.set(app).map_err(|_| anyhow::anyhow!("the app was opened twice"))
}

/// The app. Only after [`open`].
pub(crate) fn get() -> &'static App {
    #[expect(
        clippy::expect_used,
        reason = "`run` opens the app before Qt starts and quits when it can't, so no Qt call comes before it"
    )]
    let app: &'static App = APP.get().expect("app::open not called");
    app
}

/// Stop the background work and say what it couldn't stop. Call after the event loop.
pub(crate) fn stop() {
    if let Some(app) = APP.get()
        && let Err(e) = app.stop()
    {
        crate::log::note(&e.to_string());
    }
}

/// The app's end, after [`stop`] and once the image providers are gone.
pub(crate) fn shutdown() {
    if let Some(app) = APP.get() {
        app.shutdown();
    }
}

/// What a note says, on the command line.
fn note_text(note: &Note) -> String {
    match note {
        Note::PrefsUnreadable { path, error, kept } => match kept {
            Ok(bad) => format!(
                "preferences: {} didn't parse ({error}); kept as {}, the defaults are in use",
                path.display(),
                bad.display()
            ),
            Err(e) => {
                format!("preferences: {} didn't parse ({error}), and couldn't be kept aside: {e}", path.display())
            }
        },
        Note::PrefsNotSaved { path, error } => {
            format!(
                "preferences: {} couldn't be saved ({error}); the change holds until the app closes",
                path.display()
            )
        }
        Note::CatalogRebuilt(r) => {
            format!("catalog rebuilt: {}; the old one is kept as {}{}", r.why, r.kept.display(), r.carried_note())
        }
        Note::Thumbs(event) => {
            use pg_app::reexport::ThumbEvent as Event;
            match event {
                Event::TotalsFixed { kept, actual } => {
                    format!("thumbnail cache: totals were {kept:?}, the entries hold {actual:?}; fixed")
                }
                Event::TouchesNotWritten { error } => format!("thumbnail cache: touches not written yet: {error}"),
                Event::NotStored { error } => format!("thumbnail cache: a thumbnail not stored: {error}"),
                Event::WalNotMerged { error } => format!("thumbnail cache: the WAL was not merged at close: {error}"),
                Event::StartedOver { reason, path, detail } => format!(
                    "{}; detail: {detail}",
                    text::thumb_cache_started_over(started_over_reason(*reason), &path.display().to_string())
                ),
            }
        }
    }
}

/// The client's words for why the thumbnail cache was started over.
fn started_over_reason(reason: ThumbStartOverReason) -> CacheStartOver {
    match reason {
        ThumbStartOverReason::Damaged => CacheStartOver::Damaged,
        ThumbStartOverReason::NotADatabase => CacheStartOver::NotADatabase,
        ThumbStartOverReason::OtherCatalog => CacheStartOver::OtherCatalog,
        ThumbStartOverReason::OtherBuild => CacheStartOver::OtherBuild,
    }
}
