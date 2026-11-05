//! The window's one `pg_app::App`, held for the process, and what the window says about what the app notes. The app
//! itself has no UI: the notes it publishes are printed here, on stderr, as the command line always did.
//!
//! The order of shutdown is in `lib.rs` (`run`): [`stop`] (the reader, the library), then the engine goes (the image
//! providers), then [`shutdown`] (the thumbnail cache, then the catalog connections, the keeper last).

use std::sync::OnceLock;

use pg_app::{App, Config, Note};

static APP: OnceLock<App> = OnceLock::new();

/// Open the app from `config` and start printing its notes. Call once, on the GUI thread, before the UI starts.
pub fn open(config: Config) -> anyhow::Result<()> {
    let app = App::open(config)?;
    let notes = app.notes();
    std::thread::Builder::new().name("app notes".into()).spawn(move || {
        while let Some(note) = notes.recv() {
            eprintln!("{}", note_text(&note));
        }
    })?;
    APP.set(app).map_err(|_| anyhow::anyhow!("the app was opened twice"))
}

/// The app. Only after [`open`].
pub fn get() -> &'static App {
    APP.get().expect("app::open not called")
}

/// Stop the background work and say what it couldn't stop. Call after the event loop.
pub fn stop() {
    if let Some(app) = APP.get()
        && let Err(e) = app.stop()
    {
        eprintln!("{e}");
    }
}

/// The app's end, after [`stop`] and once the image providers are gone.
pub fn shutdown() {
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
        Note::Thumbs(event) => {
            use pg_app::reexport::ThumbEvent as Event;
            match event {
                Event::TotalsFixed { kept, actual } => {
                    format!("thumbnail cache: totals were {kept:?}, the entries hold {actual:?}; fixed")
                }
                Event::TouchesNotWritten { error } => format!("thumbnail cache: touches not written yet: {error}"),
                Event::NotStored { error } => format!("thumbnail cache: a thumbnail not stored: {error}"),
                Event::WalNotMerged { error } => format!("thumbnail cache: the WAL was not merged at close: {error}"),
            }
        }
    }
}
