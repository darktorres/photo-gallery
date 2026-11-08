//! Laws of the controller (`pg_client::controller`), over generated events. Each law is a property of the whole controller,
//! so it holds however the sub-machines are combined: the start gate's order, the one pause per debounce, the size's
//! clamps and thumbnails, the grid's actions without a current item, and the events' serde round trip (the recordings).

use pg_client::controller::listing as listing_event;
use pg_client::controller::prefs::{self, MAX_ROW_HEIGHT, MIN_ROW_HEIGHT};
use pg_client::controller::{Command, Controller, Event};
use pg_client::keys::Grid;
use pg_model::name::Name;
use pg_model::query::{Query, Sort, SortKey};
use pg_model::rules::Merged;
use pg_model::view::{Listing, SavedView};
use proptest::prelude::*;

/// A folder given as its path text, one part a `\`-separated component (the test's own spelling).
fn folder(text: &str) -> Merged {
    Merged::from(text.split('\\').filter(|p| !p.is_empty()).map(Name::from).collect::<Vec<_>>())
}

fn listing(text: &str) -> Listing {
    Listing { folder: folder(text), recursive: false, sort: Sort::default(), query: Query::default() }
}

fn size_event() -> impl Strategy<Value = prefs::Event> {
    prop_oneof![
        any::<bool>().prop_map(|grow| prefs::Event::Resize { grow }),
        (0.1f64..4.0).prop_map(prefs::Event::Factor),
        (-100.0f64..2000.0).prop_map(prefs::Event::Chosen),
        (0.5f64..3.0).prop_map(prefs::Event::Scale),
    ]
}

/// The grid keys that act on the current item, or need it: without one, they do nothing.
fn item_key() -> impl Strategy<Value = Grid> {
    prop_oneof![
        (0u8..=5).prop_map(Grid::Rate),
        any::<bool>().prop_map(|forward| Grid::Turn { forward }),
        Just(Grid::MarkBorder),
        Just(Grid::Open),
    ]
}

proptest! {
    /// The size stays within its clamps, and after the pause its thumbnail and saved height are the controller's own.
    #[test]
    fn size_stays_clamped_and_settles_to_the_thumbnails(events in prop::collection::vec(size_event(), 1..40)) {
        let mut c = Controller::new(listing(""), 200, 1.0);
        let mut now = 0u64;
        for event in events {
            now += 10;
            c.update(now, Event::Size(event));
            let row = c.prefs().row_height();
            prop_assert!((MIN_ROW_HEIGHT..=MAX_ROW_HEIGHT).contains(&row));
        }
        if let Some(at) = c.next_deadline() {
            let settled = c.update(at, Event::Timeout);
            let row = c.prefs().row_height();
            prop_assert!(settled.contains(&Command::Size(prefs::Change::ThumbHeight(c.prefs().thumb_height()))));
            prop_assert!(settled.contains(&Command::Size(prefs::Change::SaveRowHeight(row))));
        }
    }

    /// Query edits closer together than the pause leave one list, at the last edit plus the pause.
    #[test]
    fn edits_closer_than_the_pause_leave_one_list_at_the_last(gaps in prop::collection::vec(0u64..300, 1..30)) {
        let mut c = Controller::new(listing(""), 200, 1.0);
        let mut now = 0u64;
        for gap in gaps {
            now += gap;
            c.update(now, Event::QueryEdited);
        }
        prop_assert_eq!(c.next_deadline(), Some(now + 300));
        let early = c.update(now + 299, Event::Timeout);
        let opened_early = early.iter().any(|c| matches!(c, Command::OpenList { .. }));
        prop_assert!(!opened_early);
        let due = c.update(now + 300, Event::Timeout);
        prop_assert_eq!(due.iter().filter(|c| matches!(c, Command::OpenList { .. })).count(), 1);
    }

    /// The start is over exactly once, on the event that completes it, whichever order the tree and the list come in.
    #[test]
    fn the_start_finishes_once_whatever_the_order(order in 0usize..3, count in 1usize..12, last in 0usize..12) {
        let tree = Event::TreeLoaded { folder_known: true };
        let shown = Event::ListShown { count, last_file: (last < count).then_some(last) };
        let placed = Event::ListPositioned;
        // The list is shown before it is placed; the tree may come before, between or after them.
        let events = match order {
            0 => [tree, shown, placed],
            1 => [shown, tree, placed],
            _ => [shown, placed, tree],
        };
        let mut c = Controller::new(listing(""), 200, 1.0);
        let mut started = Vec::new();
        for (i, event) in events.into_iter().enumerate() {
            let commands = c.update(u64::try_from(i).unwrap_or(u64::MAX), event);
            if commands.contains(&Command::Started) {
                started.push(i);
            }
        }
        prop_assert_eq!(started, vec![2]);
        prop_assert!(c.started());
    }

    /// A folder the tree no longer has lists all folders at once; the start waits for that list.
    #[test]
    fn a_folder_the_tree_lost_lists_all_folders(folder in "[a-z]{1,8}") {
        let mut c = Controller::new(listing(&folder), 200, 1.0);
        let commands = c.update(0, Event::TreeLoaded { folder_known: false });
        let all = commands.iter().find_map(|c| match c {
            Command::OpenList { listing, .. } => Some((listing.folder.clone(), listing.recursive)),
            _ => None,
        });
        // All folders: the empty folder with its subfolders (without them it would be the drive roots' own files).
        prop_assert_eq!(all, Some((Merged::default(), true)));
        prop_assert!(!c.started());
    }

    /// Without a current item, the keys that act on one do nothing; with one, they act on that item.
    #[test]
    fn item_keys_act_on_the_current_item_only(key in item_key(), item in 0usize..50) {
        let mut c = Controller::new(listing(""), 200, 1.0);
        c.update(0, Event::ListShown { count: 60, last_file: None });
        prop_assert!(c.update(1, Event::Key(key)).is_empty());
        c.update(2, Event::CurrentChanged(Some(item)));
        let acted = c.update(3, Event::Key(key));
        prop_assert_eq!(acted.len(), 1);
        match acted[0] {
            Command::Rate { item: i, .. } | Command::Turn { item: i, .. } | Command::ToggleBorderMark(i) => {
                prop_assert_eq!(i, item)
            }
            Command::ShowViewer(i) => prop_assert_eq!(i, item),
            ref other => prop_assert!(false, "unexpected {other:?}"),
        }
    }

    /// Events and commands survive the JSON a recording stores them in, unchanged, folders included: one with a lone
    /// surrogate and one with a raw byte, which no text holds.
    #[test]
    fn events_round_trip_through_json(size in size_event(), item in 0usize..100, index in any::<bool>()) {
        let odd = Merged::from(vec![Name::from_wide(&[0x61, 0xD800]), Name::from_raw(b"x\xFF")]);
        let view = SavedView {
            name: "odd".into(),
            folder: odd.clone(),
            recursive: index,
            query: Query::default(),
            sort: Sort { key: SortKey::Name, descending: index },
        };
        let events = [
            Event::Size(size),
            Event::CurrentChanged(index.then_some(item)),
            Event::Key(Grid::Step { dx: -1, dy: 2 }),
            Event::SetBorderMark { item, marked: index },
            Event::Listing(listing_event::Event::OpenFolder(odd.clone())),
            Event::Listing(listing_event::Event::OpenAll),
            Event::Listing(listing_event::Event::OpenRoots),
            Event::Listing(listing_event::Event::ViewApplied(view)),
            Event::FolderSpelled(odd),
        ];
        for event in events {
            let text = serde_json::to_string(&event).expect("an event serialises");
            let back: Event = serde_json::from_str(&text).expect("an event deserialises");
            prop_assert_eq!(back, event);
        }
    }
}

/// How many of `commands` are session saves, last-file saves and lists shown.
fn saves_and_lists(commands: &[Command]) -> (usize, usize, usize) {
    let count = |f: fn(&Command) -> bool| commands.iter().filter(|c| f(c)).count();
    (
        count(|c| matches!(c, Command::SaveSession(_))),
        count(|c| matches!(c, Command::SaveLastFile)),
        count(|c| matches!(c, Command::OpenList { .. })),
    )
}

/// A session save waiting at the close is made, with the last file's.
#[test]
fn a_pending_session_save_is_made_at_the_close_with_the_last_file() {
    let mut c = Controller::new(listing(""), 200, 1.0);
    c.update(0, Event::ListShown { count: 10, last_file: None });
    c.update(1, Event::CurrentChanged(Some(3)));
    let flushed = c.update(2, Event::Flush);
    assert_eq!(saves_and_lists(&flushed), (1, 1, 0), "{flushed:?}");
}

/// A query typed within its pause of the close is saved with the session, and no list is shown for a closing window.
#[test]
fn a_pending_query_is_saved_at_the_close_without_a_list() {
    let mut c = Controller::new(listing(""), 200, 1.0);
    let query = Query { name: "beach".into(), ..Query::default() };
    c.update(0, Event::Listing(listing_event::Event::QueryChanged(query.clone())));
    c.update(0, Event::QueryEdited);
    let flushed = c.update(100, Event::Flush);
    assert_eq!(saves_and_lists(&flushed).0, 1, "{flushed:?}");
    assert_eq!(saves_and_lists(&flushed).2, 0, "{flushed:?}");
    let saved = flushed.iter().find_map(|c| match c {
        Command::SaveSession(listing) => Some(listing.query.clone()),
        _ => None,
    });
    assert_eq!(saved, Some(query));
}

/// A query and a session save both waiting make one session save and one last-file save, not one each.
#[test]
fn a_pending_query_and_session_save_make_one_save_at_the_close() {
    let mut c = Controller::new(listing(""), 200, 1.0);
    c.update(0, Event::ListShown { count: 10, last_file: None });
    c.update(1, Event::CurrentChanged(Some(3)));
    c.update(2, Event::Listing(listing_event::Event::QueryChanged(Query { name: "x".into(), ..Query::default() })));
    c.update(2, Event::QueryEdited);
    let flushed = c.update(3, Event::Flush);
    assert_eq!(saves_and_lists(&flushed), (1, 1, 0), "{flushed:?}");
    assert!(c.next_deadline().is_none(), "nothing is left waiting");
}
