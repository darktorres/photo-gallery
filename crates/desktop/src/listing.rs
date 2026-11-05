//! The listing the window shows (PLAN §7): the folder, its subfolders, the sort and the filter bar's query, as one typed
//! value on the GUI thread. Every change to it goes through `dispatch`, which applies the change and returns the commands
//! the listing's policy leaves (`pg_client::controller::listing`); the caller carries them out (`shell.rs` opens the list
//! and saves the session, `filters.rs` leaves the session to the list's showing). The shell (`shell.rs`) shows the folder,
//! subfolders and sort, and keeps the saved views; the filter bar (`filters.rs`) reads and writes the query; the grid
//! model (`gallery.rs`) reads all four when it asks for a list. Nothing crosses the bridge as JSON.

use std::cell::RefCell;

use pg_client::controller::listing::{Command, Event, update};
use pg_model::view::Listing;

use crate::{app, command_line};

thread_local! {
    /// Where the window starts (`start`), then what the user chooses. Only the GUI thread touches it.
    static LISTING: RefCell<Listing> = RefCell::new(start());
}

/// Where the window starts: the session's listing (its folder, subfolders, sort and query), then `--sort` and `--query`,
/// then the saved view `--apply-view` names, when there is one (its folder, subfolders, sort and query, which win over
/// the session's). The filter bar starts with the listing's query (`filters.rs`).
fn start() -> Listing {
    let mut listing = app::get().listing();
    let options = command_line::start_options();
    if let Some(query) = options.query {
        listing.query = query;
    }
    if let Some(sort) = options.sort {
        listing.sort = sort;
    }
    if let Some(name) = options.apply_view
        && let Some(v) = app::get().view(&name).ok().flatten()
    {
        listing = Listing { folder: v.folder, recursive: v.recursive, sort: v.sort, query: v.query };
    }
    listing
}

/// The listing as it is now.
pub(crate) fn get() -> Listing {
    LISTING.with(|listing| listing.borrow().clone())
}

/// Change the listing by `event`, and return the commands that follow. Nothing here calls Qt, so a handler the
/// commands lead to can read the listing.
pub(crate) fn dispatch(event: Event) -> Vec<Command> {
    LISTING.with(|listing| update(&mut listing.borrow_mut(), event))
}

/// Remember the listing for the next start.
pub(crate) fn save_session() {
    if let Err(e) = app::get().save_listing(&get()) {
        eprintln!("session: {e:#}");
    }
}

/// Change the listing's folder spelling without opening a list (the tree spells the folder it has).
pub(crate) fn respell(folder: String) {
    LISTING.with(|listing| listing.borrow_mut().folder = folder);
}
