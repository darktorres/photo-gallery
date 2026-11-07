//! Laws of the controller (`pg_client::controller`), over generated events. Each law is a property of the whole controller,
//! so it holds however the sub-machines are combined: the start gate's order, the one pause per debounce, the size's
//! clamps and thumbnails, the grid's actions without a current item, and the events' serde round trip (the recordings).

use pg_client::controller::prefs::{self, MAX_ROW_HEIGHT, MIN_ROW_HEIGHT};
use pg_client::controller::{Command, Controller, Event};
use pg_client::keys::Grid;
use pg_model::query::{Query, Sort};
use pg_model::view::Listing;
use proptest::prelude::*;

fn listing(folder: &str) -> Listing {
    Listing { folder: folder.to_string(), recursive: false, sort: Sort::default(), query: Query::default() }
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
            Command::OpenList { listing, .. } => Some(listing.folder.clone()),
            _ => None,
        });
        prop_assert_eq!(all, Some(String::new()));
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

    /// Events and commands survive the JSON a recording stores them in, unchanged.
    #[test]
    fn events_round_trip_through_json(size in size_event(), item in 0usize..100, index in any::<bool>()) {
        let events = [
            Event::Size(size),
            Event::CurrentChanged(index.then_some(item)),
            Event::Key(Grid::Step { dx: -1, dy: 2 }),
            Event::SetBorderMark { item, marked: index },
        ];
        for event in events {
            let text = serde_json::to_string(&event).expect("an event serialises");
            let back: Event = serde_json::from_str(&text).expect("an event deserialises");
            prop_assert_eq!(back, event);
        }
    }
}
