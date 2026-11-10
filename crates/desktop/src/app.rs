//! The window's one `pg_app::App`, held for the process, and how the window tells the user what happened: it posts a
//! notice to the app's inbox ([`post`]), and says when one no longer holds ([`resolved`]). The inbox logs each notice;
//! the `Notices` object shows them.
//!
//! The order of shutdown is in `lib.rs` (`run`): [`stop`] (the reader, the library), then the engine goes (the image
//! providers), then [`shutdown`] (the thumbnail cache, the inbox, then the catalog connections, the keeper last).

use std::fmt::Display;
use std::sync::OnceLock;

use pg_app::{App, Config};
use pg_model::notice::{ERROR_CAP, Key, Notice, Part, tidy};

static APP: OnceLock<App> = OnceLock::new();

/// Open the app from `config`. Call once, on the GUI thread, before the UI starts.
pub(crate) fn open(config: Config) -> anyhow::Result<()> {
    let app = App::open(config)?;
    APP.set(app).map_err(|_| anyhow::anyhow!("the app was opened twice"))
}

/// Tell the user `notice`: the inbox keeps it (and logs it) as its kind says.
pub(crate) fn post(notice: Notice) {
    get().inbox().post(notice);
}

/// The kept notices of the kind `code` (a `Notice::code`) no longer hold: what they said failed went through. Costs
/// nothing when the inbox holds none.
pub(crate) fn resolved(code: &'static str) {
    get().inbox().clear(Key::all(code, None).to_buf());
}

/// [`resolved`], for the kept notice of the kind `code` whose part is `part` (an edit's operation).
pub(crate) fn resolved_part(code: &'static str, part: &'static str) {
    get().inbox().clear(Key { code, root: None, part: Some(Part::Code(part)) }.to_buf());
}

/// An error's text as a notice keeps it: its causes after it (`{:#}`), on one line, capped.
pub(crate) fn error_text(error: &impl Display) -> String {
    tidy(&format!("{error:#}"), ERROR_CAP)
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
        pg_log::warn("app", &e.to_string());
    }
}

/// The app's end, after [`stop`] and once the image providers are gone.
pub(crate) fn shutdown() {
    if let Some(app) = APP.get() {
        app.shutdown();
    }
}
