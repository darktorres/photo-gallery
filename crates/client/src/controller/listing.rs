//! The listing's policy (S-12.15): what a change to where the grid looks does. The desktop keeps the listing and applies
//! each change it reports; this says what follows, as commands, with no Qt. Every change that is applied opens a list
//! and remembers the session, except a sort choice that isn't one (it changes nothing).

use pg_model::query::{Query, Status};
use pg_model::rules::Merged;
use pg_model::view::{Listing, SavedView};
use serde::{Deserialize, Serialize};

use crate::sort::sort_choice;

/// A change to the listing: made by the user, or by the start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// Show a merged folder (the empty folder = all folders, or the drive roots' own files without subfolders).
    OpenFolder(Merged),
    /// Show every folder: the empty folder with its subfolders (the tree's "All" row, and the start's fallback).
    OpenAll,
    /// Show the files directly in the drive roots: the empty folder without its subfolders (the tree's "Drive roots"
    /// row).
    OpenRoots,
    /// With, or without, the files of its subfolders.
    Recursive(bool),
    /// The sort box's choice (`pg_client::sort`).
    SortChoice(usize),
    /// The sort's way turned.
    DescendingToggled,
    /// The filter bar's query, edited.
    QueryChanged(Query),
    /// A saved view: its folder, subfolders, sort and query replace the listing's.
    ViewApplied(SavedView),
    /// Show a folder with nothing filtered, so that a file in it is listed (the inbox's Reveal).
    Reveal(Merged),
    /// Show the files the import failed on, in every folder (the inbox's "Show the failed files").
    FailedImports,
}

/// What follows a change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// List the files the listing asks for (shown from the top).
    OpenList,
    /// Remember the listing for the next start.
    SaveSession,
}

/// Apply `event` to `listing`, and the commands it leaves, in order: none when the event changes nothing.
pub fn update(listing: &mut Listing, event: Event) -> Vec<Command> {
    match event {
        Event::OpenFolder(folder) => listing.folder = folder,
        Event::OpenAll => (listing.folder, listing.recursive) = (Merged::default(), true),
        Event::OpenRoots => (listing.folder, listing.recursive) = (Merged::default(), false),
        Event::Recursive(on) => listing.recursive = on,
        Event::SortChoice(index) => match sort_choice(index) {
            Some(sort) => listing.sort = sort,
            None => return Vec::new(),
        },
        Event::DescendingToggled => listing.sort.descending = !listing.sort.descending,
        Event::QueryChanged(query) => listing.query = query,
        Event::ViewApplied(view) => {
            *listing = Listing { folder: view.folder, recursive: view.recursive, sort: view.sort, query: view.query };
        }
        Event::Reveal(folder) => (listing.folder, listing.query) = (folder, Query::default()),
        Event::FailedImports => {
            listing.folder = Merged::default();
            listing.recursive = true;
            listing.query = Query { status: vec![Status::Failed], ..Query::default() };
        }
    }
    vec![Command::OpenList, Command::SaveSession]
}
