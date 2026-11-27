//! The window's one `pg_app::App`, held for the process, and how the window tells the user what happened: it posts a
//! notice to the app's inbox ([`post`]), and says when one no longer holds ([`observe`]). The inbox logs each notice;
//! the `Notices` object shows them.
//!
//! The order of shutdown is in `lib.rs` (`run`): [`stop`] (the reader, the library), then the engine goes (the image
//! providers), then [`shutdown`] (the thumbnail cache, the inbox, then the catalog connections, the keeper last).

use std::sync::OnceLock;

use pg_app::{App, Config};
use pg_model::notice::{Notice, Op, Trigger};

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

/// The outcome of the operation `op`: a failure is posted (`failure` words it, for the error), and a success is the
/// operation's trigger, which clears the notice its failure kept. Both are the same `op`, so a failure can't be posted
/// without a clear for it. The value on success, `None` on a failure.
pub(crate) fn observe<T, E>(op: Op, result: Result<T, E>, failure: impl FnOnce(&E) -> Notice) -> Option<T> {
    observe_result(op, result, failure).ok()
}

/// As [`observe`], but a failure is handed back as well: for a caller whose own state shows the failure too (a listing
/// in error), and still needs the error.
pub(crate) fn observe_result<T, E>(op: Op, result: Result<T, E>, failure: impl FnOnce(&E) -> Notice) -> Result<T, E> {
    match result {
        Ok(value) => {
            get().inbox().resolve(Trigger::Op(op));
            Ok(value)
        }
        Err(e) => {
            post(failure(&e));
            Err(e)
        }
    }
}

/// An error's text as a notice keeps it (`pg_model::notice::error_text`).
pub(crate) use pg_model::notice::error_text;

/// The app. Only after [`open`].
pub(crate) fn get() -> &'static App {
    #[expect(
        clippy::expect_used,
        reason = "`run` opens the app before Qt starts and quits when it can't, so no Qt call comes before it"
    )]
    let app: &'static App = APP.get().expect("app::open not called");
    app
}

/// The reader's session, which the window's catalog reads go through (`App::reading`, then `session`).
pub(crate) fn session() -> pg_app::Session {
    get().reading().session()
}

/// Stop the background work and say what it couldn't stop. Call after the event loop.
pub(crate) fn stop() {
    if let Some(app) = APP.get()
        && let Err(e) = app.stop()
    {
        pg_log::warn("app", &format!("{e:#}"));
    }
}

/// The app's end, after [`stop`] and once the image providers are gone.
pub(crate) fn shutdown() {
    if let Some(app) = APP.get() {
        app.shutdown();
    }
}
