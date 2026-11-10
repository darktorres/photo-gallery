//! The listing's policy by traces: each change, applied to a listing, gives the listing it must and the commands.

use pg_client::controller::listing::{Command, Event, update};
use pg_client::controller::{Command as Do, Controller, Event as Report};
use pg_model::name::Name;
use pg_model::query::{Query, Sort, SortKey, Status};
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

/// The tree's two first rows: "All" is the empty folder with its subfolders, "Drive roots" without them, whatever
/// recursion was before.
#[test]
fn all_folders_and_the_drive_roots_set_the_folder_and_the_subfolders() {
    let mut listing = start();
    assert_eq!(update(&mut listing, Event::OpenAll), OPEN);
    assert!(listing.folder.is_empty() && listing.recursive);
    assert_eq!(update(&mut listing, Event::OpenRoots), OPEN);
    assert!(listing.folder.is_empty() && !listing.recursive);
    update(&mut listing, Event::OpenAll);
    assert!(listing.folder.is_empty() && listing.recursive);
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
    update(&mut listing, Event::SortChoice(3));
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

/// A reveal shows the file's folder with nothing filtered (the sort and the subfolders kept), and selects the file in
/// the next list shown, once; the failed imports are listed from every folder.
#[test]
fn a_reveal_lists_the_files_folder_and_selects_it_once_shown() {
    let mut listing = start();
    listing.query = Query { name: "x".into(), ..Query::default() };
    assert_eq!(update(&mut listing, Event::Reveal(folder(r"alpha\beta"))), OPEN);
    assert_eq!((listing.folder.clone(), listing.query.clone()), (folder(r"alpha\beta"), Query::default()));
    assert_eq!(update(&mut listing, Event::FailedImports), OPEN);
    assert!(listing.folder.is_empty() && listing.recursive);
    assert_eq!(listing.query, Query { status: vec![Status::Failed], ..Query::default() });

    let mut c = Controller::new(start(), 200, 1.0);
    let out = c.update(0, Report::Reveal { file: 7, folder: folder("beta") });
    assert!(matches!(&out[..], [Do::OpenList { keep: false, .. }, Do::SaveSession(_)]), "{out:?}");
    assert_eq!(c.listing().folder, folder("beta"));
    let shown = |c: &mut Controller| c.update(1, Report::ListShown { count: 3, last_file: None });
    assert!(shown(&mut c).contains(&Do::Reveal(7)));
    assert!(!shown(&mut c).contains(&Do::Reveal(7)), "once");
}
