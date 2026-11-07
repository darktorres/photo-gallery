//! The listing's policy by traces: each change, applied to a listing, gives the listing it must and the commands.

use pg_client::controller::listing::{Command, Event, update};
use pg_model::name::Name;
use pg_model::query::{Query, Sort, SortKey};
use pg_model::rules::Merged;
use pg_model::view::{Listing, SavedView};

const OPEN: [Command; 2] = [Command::OpenList, Command::SaveSession];

/// A folder given as its path text, one part per `\`-separated component.
fn folder(text: &str) -> Merged {
    Merged::from(text.split('\\').filter(|p| !p.is_empty()).map(Name::from).collect::<Vec<_>>())
}

/// The listing the traces start from: its sort is the sort box's first choice, name ascending.
fn start() -> Listing {
    let sort = Sort { key: SortKey::Name, descending: false };
    Listing { folder: folder("alpha"), recursive: false, sort, query: Query::default() }
}

#[test]
fn every_change_that_applies_opens_the_list_and_saves_the_session() {
    let cases = [
        Event::OpenFolder(folder("beta")),
        Event::Recursive(true),
        Event::SortChoice(2),
        Event::DescendingToggled,
        Event::QueryChanged(Query { name: "x".into(), ..Query::default() }),
    ];
    for event in cases {
        let mut listing = start();
        assert_eq!(update(&mut listing, event.clone()), OPEN, "{event:?}");
    }
}

#[test]
fn a_folder_is_the_one_shown() {
    let mut listing = start();
    update(&mut listing, Event::OpenFolder(Merged::default()));
    assert!(listing.folder.is_empty());
}

#[test]
fn subfolders_are_as_chosen() {
    let mut listing = start();
    update(&mut listing, Event::Recursive(true));
    assert!(listing.recursive);
    update(&mut listing, Event::Recursive(false));
    assert!(!listing.recursive);
}

#[test]
fn the_sort_box_sets_key_and_way() {
    let mut listing = start();
    update(&mut listing, Event::SortChoice(0));
    assert_eq!(listing.sort, Sort { key: SortKey::Name, descending: false });
    update(&mut listing, Event::SortChoice(2));
    assert_eq!(listing.sort, Sort { key: SortKey::Modified, descending: true });
}

#[test]
fn an_index_that_is_no_choice_changes_nothing_and_opens_nothing() {
    let mut listing = start();
    let before = listing.sort;
    assert_eq!(update(&mut listing, Event::SortChoice(99)), Vec::<Command>::new());
    assert_eq!(listing.sort, before);
}

#[test]
fn toggling_the_way_flips_it_and_keeps_the_key() {
    let mut listing = start();
    update(&mut listing, Event::DescendingToggled);
    assert_eq!(listing.sort, Sort { key: SortKey::Name, descending: true });
    update(&mut listing, Event::DescendingToggled);
    assert_eq!(listing.sort, Sort { key: SortKey::Name, descending: false });
}

#[test]
fn a_saved_view_replaces_the_listing_whole() {
    let mut listing = start();
    let view = SavedView {
        name: "gamma view".into(),
        folder: folder("gamma"),
        recursive: true,
        query: Query { name: "cat".into(), ..Query::default() },
        sort: Sort { key: SortKey::Size, descending: true },
    };
    assert_eq!(update(&mut listing, Event::ViewApplied(view.clone())), OPEN);
    assert_eq!(listing.folder, view.folder);
    assert_eq!(listing.recursive, view.recursive);
    assert_eq!(listing.sort, view.sort);
    assert_eq!(listing.query, view.query);
}
