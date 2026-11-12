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

use pg_client::controller::replay::{self, Record};
use pg_client::controller::{Command, Controller, Event};
use pg_client::facets::Counts;
use pg_model::size::RowHeight;
use pg_model::view::{Listing, SavedView};

use crate::{app, command_line, deadline};

/// The controller, the filter bar's counts, and the recorder (`None` unless `PG_RECORD` names a file).
struct Core {
    controller: Controller,
    counts: Counts,
    recorder: Option<std::fs::File>,
}

thread_local! {
    /// The one controller. Its calls are made one at a time on the GUI thread, and none calls back into another.
    static CORE: RefCell<Core> = RefCell::new(Core::new());
}

impl Core {
    fn new() -> Core {
        // The reads are here; `start_state` decides from what they give.
        let options = command_line::start_options();
        let applied = options.apply_view.as_deref().and_then(|name| app::get().views().view(name).ok().flatten());
        let (listing, row, saved_row) =
            start_state(app::get().prefs().get().row_height, &options, app::get().views().listing(), applied);
        let controller = Controller::new(listing, row, saved_row);
        let recorder = std::env::var_os("PG_RECORD").and_then(|path| match std::fs::File::create(&path) {
            Ok(file) => Some(file),
            Err(e) => {
                pg_log::warn("record", &format!("PG_RECORD {}: {e}", path.to_string_lossy()));
                None
            }
        });
        let mut core = Core { controller, counts: Counts::default(), recorder };
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
                    pg_log::warn("record", &format!("PG_RECORD: {e}"));
                }
            }
            Err(e) => pg_log::warn("record", &format!("PG_RECORD: {e}")),
        }
    }
}

/// Where the window starts, as `(listing, shown row, saved row)`, from what it is given (pure: the reads are `Core::new`'s).
/// The listing is the session's (its folder, subfolders, sort and query), then `--sort` and `--query`, then the saved view
/// `--apply-view` names, when `applied` has it (its listing wins over the session's). The row shown is `--size` when given,
/// else the prefs' saved row; the saved row is always the prefs', so a start's size is never saved by the start itself.
fn start_state(
    saved_row: RowHeight,
    options: &command_line::StartOptions,
    session: Listing,
    applied: Option<SavedView>,
) -> (Listing, RowHeight, RowHeight) {
    let mut listing = session;
    if let Some(query) = options.query.clone() {
        listing.query = query;
    }
    if let Some(sort) = options.sort {
        listing.sort = sort;
    }
    if let Some(view) = applied {
        listing = view.listing;
    }
    (listing, options.size.unwrap_or(saved_row), saved_row)
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
    let _ = update(Event::Listing(pg_client::controller::listing::Event::QueryChanged(query)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::query::{Query, Sort};

    fn row(px: u32) -> RowHeight {
        RowHeight::new(px).expect("a row height in range")
    }

    fn options(size: Option<RowHeight>, query: Option<Query>, sort: Option<Sort>) -> command_line::StartOptions {
        command_line::StartOptions { query, sort, apply_view: None, size }
    }

    fn listing(name: &str) -> Listing {
        Listing { query: Query { name: name.into(), ..Query::default() }, ..Listing::default() }
    }

    #[test]
    fn a_size_option_shows_its_row_and_the_saved_row_stays_the_prefs() {
        let (_, shown, saved) = start_state(row(220), &options(Some(row(120)), None, None), listing(""), None);
        assert_eq!((shown, saved), (row(120), row(220)));
    }

    #[test]
    fn without_a_size_the_window_shows_the_saved_row() {
        let (_, shown, saved) = start_state(row(220), &options(None, None, None), listing(""), None);
        assert_eq!((shown, saved), (row(220), row(220)));
    }

    #[test]
    fn the_options_shape_the_session_listing_and_a_view_wins_over_them() {
        let typed = |name: &str| Query { name: name.into(), ..Query::default() };
        let by_option = options(None, Some(typed("option")), None);
        let (got, _, _) = start_state(row(220), &by_option, listing("session"), None);
        assert_eq!(got.query, typed("option"));
        let view = SavedView { name: "view".into(), listing: listing("view") };
        let (got, _, _) = start_state(row(220), &by_option, listing("session"), Some(view));
        assert_eq!(got.query, typed("view"), "the saved view a start applies wins over --query");
    }
}
