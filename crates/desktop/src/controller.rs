//! The window's controller, on the GUI thread: the one `pg_client` controller, which holds the window's policy (the start,
//! the listing, the grid's keys, the panels, the size and the pauses). The shell reports each event through `update` and
//! carries out the commands that come back (`shell.rs`); the filter bar and the grid model read the listing through
//! `listing`. Nothing crosses the bridge as JSON.
//!
//! Beside the controller, not in it: the filter bar's counts (`counts`, `set_counts`). They are display data that no
//! command depends on, so they are not events (recording each would only bloat the replays).
//!
//! With `PG_RECORD` set to a file, every event and the commands it gave are written there as JSON lines
//! (`pg_client::controller::replay`), for `pg-client`'s replay test.

use std::cell::RefCell;
use std::io::Write;

use pg_app::Prefs;
use pg_client::controller::replay::{self, Record};
use pg_client::controller::{Command, Controller, Event, Start};
use pg_client::facets::Counts;
use pg_model::query::{Query, Sort};
use pg_model::view::{Listing, SavedView};

use crate::{app, command_line, deadline};

/// The controller, the filter bar's counts, the recorder (`None` unless `PG_RECORD` names a file), and whether the start
/// left out what it was given for border detection (`start_refused`).
struct Core {
    controller: Controller,
    counts: Counts,
    recorder: Option<std::fs::File>,
    options_refused: bool,
}

thread_local! {
    /// The one controller. Its calls are made one at a time on the GUI thread, and none calls back into another.
    static CORE: RefCell<Core> = RefCell::new(Core::new());
}

impl Core {
    fn new() -> Core {
        // The reads are here; `start_state` decides from what they give.
        let options = command_line::start_options();
        let applied = options.apply_view.as_deref().and_then(|name| match app::get().views().view(name) {
            Ok(Some(view)) => Some(view),
            Ok(None) => {
                pg_log::warn("start", &format!("--apply-view={name}: no saved view has that name; ignored"));
                None
            }
            Err(e) => {
                pg_log::warn("start", &format!("--apply-view={name}: {e:#}; ignored"));
                None
            }
        });
        // A session that can't be read starts from the default listing, for this start only: the stored text stays.
        let session = app::get().views().listing().unwrap_or_else(|e| {
            app::post(pg_model::notice::Notice::SessionNotRead { error: app::error_text(&e) });
            Listing::default()
        });
        let options_refused = start_refused(&app::get().prefs().get(), &options, applied.as_ref());
        let mut start = start_state(&app::get().prefs().get(), &options, session, applied);
        // The remembered file is found now, with the catalog open: its stamp names a row only while it is still that file.
        start.last_file = app::get()
            .prefs()
            .get()
            .last_file
            .and_then(|stamp| app::get().catalogs().find_by_stamp(stamp).ok().flatten());
        let controller = Controller::new(start);
        let recorder =
            pg_platform::env::read(pg_platform::env::Var::Record).and_then(|path| match std::fs::File::create(&path) {
                Ok(file) => Some(file),
                Err(e) => {
                    pg_log::warn("record", &format!("PG_RECORD {}: {e:#}", path.to_string_lossy()));
                    None
                }
            });
        let mut core = Core { controller, counts: Counts::default(), recorder, options_refused };
        let init = replay::init(&core.controller);
        core.write(&init);
        core
    }

    fn write(&mut self, record: &Record) {
        let Some(file) = self.recorder.as_mut() else {
            return;
        };
        match serde_json::to_string(record) {
            Ok(line) => {
                if let Err(e) = writeln!(file, "{line}") {
                    pg_log::warn("record", &format!("PG_RECORD: {e:#}"));
                }
            }
            Err(e) => pg_log::warn("record", &format!("PG_RECORD: {e:#}")),
        }
    }
}

/// Where the window starts, from what it is given (pure: the reads are `Core::new`'s). The listing is the session's (its
/// folder, subfolders, sort and query), then `--sort` and `--query`, then the saved view `--apply-view` names, when
/// `applied` has it (its listing wins over the session's). The row shown is `--size` when given, else the prefs' saved
/// row; the saved row is always the prefs', so a start's size is never saved by the start itself. The panels are the
/// prefs' (borders, dev mode), each overridden by its option; the filter bar is closed unless `--filters` opens it.
/// Nothing here is saved.
pub(crate) fn start_state(
    prefs: &Prefs,
    options: &command_line::StartOptions,
    session: Listing,
    applied: Option<SavedView>,
) -> Start {
    let mut listing = session;
    if let Some(query) = options.query.clone() {
        listing.query = query;
    }
    if let Some(sort) = options.sort {
        listing.sort = sort;
    }
    let features = prefs.features();
    // A saved view that uses borders is not applied while border detection is off (`start_refused` says so).
    let applied = applied.filter(|view| features.borders || !pg_client::features::uses_borders(&view.listing));
    let apply_view = applied.is_some();
    if let Some(view) = applied {
        listing = view.listing;
    }
    // With border detection off, the listing starts without what it would give (its border facets, the sort by border
    // depth), and the borders are not shown: nothing is left behind a control that does nothing. The session's own
    // borders go silently; the options' are said (`start_refused`).
    if !features.borders {
        listing = pg_client::features::without_borders(listing);
    }
    Start {
        listing,
        row: options.size.unwrap_or(prefs.row_height),
        saved_row: prefs.row_height,
        filters_open: options.filters_open.unwrap_or(false),
        show_borders: options.show_borders.unwrap_or(prefs.show_borders) && features.borders,
        dev_mode: options.dev_mode.unwrap_or(prefs.dev_mode),
        // Resolved by the caller, which has the catalog open (`find_by_stamp`); a start from prefs alone has none yet.
        last_file: None,
        apply_view,
    }
}

/// Whether a start left out something it was given because border detection is off: an option's border query or sort, or
/// a saved view that uses borders (`start_state` keeps the view out). The window says so once the start is over.
pub(crate) fn start_refused(prefs: &Prefs, options: &command_line::StartOptions, applied: Option<&SavedView>) -> bool {
    if prefs.features().borders {
        return false;
    }
    let by_query = options.query.as_ref().is_some_and(|q| pg_client::features::uses_borders_in(q, &Sort::default()));
    let by_sort = options.sort.as_ref().is_some_and(|s| pg_client::features::uses_borders_in(&Query::default(), s));
    let by_view = applied.is_some_and(|view| pg_client::features::uses_borders(&view.listing));
    by_query || by_sort || by_view
}

/// Report `event` at the current time, and return the commands it gives, in order. The caller carries them out after
/// this returns (the controller is not borrowed meanwhile).
pub(crate) fn update(event: Event) -> Vec<Command> {
    let now = deadline::now_ms();
    CORE.with(|core| {
        let mut core = core.borrow_mut();
        let recorded = core.recorder.is_some().then(|| event.clone());
        let commands = core.controller.update(now, event);
        if let Some(event) = recorded {
            core.write(&Record::Update { now_ms: now, event, commands: commands.clone() });
        }
        commands
    })
}

/// Read the controller's state, without reporting an event.
pub(crate) fn with<T>(read: impl FnOnce(&Controller) -> T) -> T {
    CORE.with(|core| read(&core.borrow().controller))
}

/// The listing as it is now.
/// Whether the start left out what it was given because border detection is off (`start_refused`): the window says so
/// once the start is over.
pub(crate) fn options_refused() -> bool {
    CORE.with(|core| core.borrow().options_refused)
}

pub(crate) fn listing() -> Listing {
    with(|c| c.listing().clone())
}

/// The counts the filter bar's list facets show, as `Filters` last set them.
pub(crate) fn counts() -> Counts {
    CORE.with(|core| core.borrow().counts.clone())
}

/// The counts the list facets show (`Filters` sets them when they are in or the query changed).
pub(crate) fn set_counts(counts: Counts) {
    CORE.with(|core| core.borrow_mut().counts = counts);
}

/// The filter bar's query was edited: the listing takes it at once; the list is shown after the controller's pause
/// (`Event::QueryEdited`, reported by the shell).
pub(crate) fn query_changed(query: pg_model::query::Query) {
    let _ = update(Event::Listing(Box::new(pg_client::controller::listing::Event::QueryChanged(query))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::query::{Query, Sort};
    use pg_model::size::RowHeight;

    fn row(px: u32) -> RowHeight {
        RowHeight::new(px).expect("a row height in range")
    }

    /// The prefs with the saved row given and the rest as a fresh install has them.
    fn prefs(saved: RowHeight) -> Prefs {
        Prefs { row_height: saved, ..Prefs::default() }
    }

    fn options(size: Option<RowHeight>, query: Option<Query>, sort: Option<Sort>) -> command_line::StartOptions {
        command_line::StartOptions {
            query,
            sort,
            apply_view: None,
            size,
            filters_open: None,
            show_borders: None,
            dev_mode: None,
        }
    }

    fn listing(name: &str) -> Listing {
        Listing { query: Query { name: name.into(), ..Query::default() }, ..Listing::default() }
    }

    #[test]
    fn a_size_option_shows_its_row_and_the_saved_row_stays_the_prefs() {
        let start = start_state(&prefs(row(220)), &options(Some(row(120)), None, None), listing(""), None);
        assert_eq!((start.row, start.saved_row), (row(120), row(220)));
    }

    #[test]
    fn without_a_size_the_window_shows_the_saved_row() {
        let start = start_state(&prefs(row(220)), &options(None, None, None), listing(""), None);
        assert_eq!((start.row, start.saved_row), (row(220), row(220)));
    }

    #[test]
    fn the_options_shape_the_session_listing_and_a_view_wins_over_them() {
        let typed = |name: &str| Query { name: name.into(), ..Query::default() };
        let by_option = options(None, Some(typed("option")), None);
        let start = start_state(&prefs(row(220)), &by_option, listing("session"), None);
        assert_eq!(start.listing.query, typed("option"));
        let view = SavedView { name: "view".into(), listing: listing("view") };
        let start = start_state(&prefs(row(220)), &by_option, listing("session"), Some(view));
        assert_eq!(start.listing.query, typed("view"), "the saved view a start applies wins over --query");
    }

    /// The panels start as the prefs have them, and each option overrides its own: a start's `--borders=0` shows no
    /// borders whatever the prefs say, and nothing here is saved.
    #[test]
    fn the_panels_are_the_prefs_unless_an_option_says_otherwise() {
        let mut saved = prefs(row(220));
        saved.show_borders = true;
        let start = start_state(&saved, &options(None, None, None), listing(""), None);
        assert!(start.show_borders && !start.dev_mode && !start.filters_open);
        let mut chosen = options(None, None, None);
        chosen.show_borders = Some(false);
        chosen.dev_mode = Some(true);
        chosen.filters_open = Some(true);
        let start = start_state(&saved, &chosen, listing(""), None);
        assert!(!start.show_borders && start.dev_mode && start.filters_open);
        assert_eq!(start.saved_row, row(220));
    }

    /// The prefs' last file is not the start's on its own: the caller resolves it against the catalog. A view the start
    /// applies is remembered once the start is over.
    #[test]
    fn the_last_file_is_resolved_by_the_caller_and_an_applied_view_is_flagged() {
        let mut saved = prefs(row(220));
        saved.last_file =
            Some(pg_model::ids::FileStamp { identity: pg_model::ids::FileIdentity::new(7, 42), size: 10, mtime: 20 });
        let start = start_state(&saved, &options(None, None, None), listing(""), None);
        assert_eq!(start.last_file, None);
        assert!(!start.apply_view);
        let view = SavedView { name: "view".into(), listing: listing("view") };
        let start = start_state(&saved, &options(None, None, None), listing(""), Some(view));
        assert!(start.apply_view);
    }

    /// With border detection off, a saved view that uses borders is left out and an option's border sort is said; the
    /// session's own borders are dropped without a word, and nothing is refused for a listing that uses none.
    #[test]
    fn a_border_use_is_refused_when_borders_are_off_and_the_session_drops_it_silently() {
        let mut off = prefs(row(220));
        off.detect_borders = false;
        let by_depth = Sort { key: pg_model::query::SortKey::Borders, descending: false };
        let mut by_option = options(None, None, None);
        by_option.sort = Some(by_depth);
        let session = Listing { sort: by_depth, ..listing("session") };
        let view = || SavedView {
            name: "view".into(),
            listing: Listing { query: Query { border_marked: Some(true), ..Query::default() }, ..listing("view") },
        };

        // The option's border sort is refused: the start shows the name order instead.
        assert!(start_refused(&off, &by_option, None));
        let start = start_state(&off, &by_option, session, None);
        assert!(matches!(start.listing.sort.key, pg_model::query::SortKey::Name));
        assert!(!start.apply_view);

        // A saved view that uses borders is not applied at all, and is refused; the session's listing stays.
        assert!(start_refused(&off, &options(None, None, None), Some(&view())));
        let start = start_state(&off, &options(None, None, None), listing("session"), Some(view()));
        assert!(!start.apply_view);
        assert_eq!(start.listing.query.name, "session");

        // The session's own border sort alone is dropped silently: nothing was given, so nothing is refused.
        let session = Listing { sort: by_depth, ..listing("session") };
        assert!(!start_refused(&off, &options(None, None, None), None));
        assert!(matches!(
            start_state(&off, &options(None, None, None), session, None).listing.sort.key,
            pg_model::query::SortKey::Name
        ));

        // With borders on, nothing is refused.
        assert!(!start_refused(&prefs(row(220)), &by_option, Some(&view())));
    }
}
