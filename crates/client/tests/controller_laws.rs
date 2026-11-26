//! Laws of the controller (`pg_client::controller`), over generated events. Each law is a property of the whole controller,
//! so it holds however the sub-machines are combined: the start gate's order, the one pause per debounce, the size's
//! clamps and thumbnails, the grid's actions without a current item, and the events' serde round trip (the recordings).

use pg_client::controller::listing as listing_event;
use pg_client::controller::size;
use pg_client::controller::{Command, Controller, Event, Start};
use pg_client::keys::GridKey;
use pg_model::ids::FileId;
use pg_model::name::Name;
use pg_model::query::{Query, Sort, SortKey};
use pg_model::rules::Merged;
use pg_model::size::{Factor, RowHeight, Scale};
use pg_model::turns::Turns;
use pg_model::view::{Listing, SavedView};
use proptest::prelude::*;

mod common;

use common::{ROW_200, controller, folder};

/// The scale a window reports first.
const SCALE_1: Scale = Scale::new(1.0).expect("a positive scale");

/// A controller at scale 1 (the window's scale arrives as the first size event, as the desktop sends it).
fn scaled(listing: Listing) -> Controller {
    let mut c = controller(listing);
    c.update(0, Event::Size(size::Event::Scale(SCALE_1)));
    c
}

fn listing(text: &str) -> Listing {
    Listing { folder: folder(text), recursive: false, sort: Sort::default(), query: Query::default() }
}

fn size_event() -> impl Strategy<Value = size::Event> {
    prop_oneof![
        any::<bool>().prop_map(|grow| size::Event::Resize { grow }),
        (0.1f64..4.0).prop_filter_map("a positive factor", Factor::new).prop_map(size::Event::Factor),
        (-100.0f64..2000.0).prop_filter_map("a finite height", RowHeight::from_finite).prop_map(size::Event::Chosen),
        (0.5f64..3.0).prop_filter_map("a positive scale", Scale::new).prop_map(size::Event::Scale),
    ]
}

/// The grid keys that act on the current item, or need it: without one, they do nothing.
fn item_key() -> impl Strategy<Value = GridKey> {
    prop_oneof![
        (0u8..=5).prop_map(GridKey::Rate),
        any::<bool>().prop_map(|forward| GridKey::Turn { forward }),
        Just(GridKey::MarkBorder),
        Just(GridKey::Open),
    ]
}

/// A turn of an item by the signed count the desktop sends (1 clockwise, -1 anticlockwise, and any other count the bridge
/// can carry): the event holds the quarter turns that count is.
fn turn_event() -> impl Strategy<Value = Event> {
    (0usize..100, -8i64..=8).prop_map(|(item, count)| Event::Turn { item, turns: Turns::of(count) })
}

proptest! {
    /// A turn survives its recording, unchanged.
    #[test]
    fn turn_events_round_trip_through_json(event in turn_event()) {
        let text = serde_json::to_string(&event).expect("an event serialises");
        let back: Event = serde_json::from_str(&text).expect("an event deserialises");
        prop_assert_eq!(back, event);
    }

    /// A recording that holds a signed count (a counterclockwise turn is -1) reads back as the same quarter turns the
    /// controller computes, so an older recording still replays.
    #[test]
    fn a_recorded_signed_turn_reads_as_its_quarter_turns(count in -8i64..=8) {
        let recorded = serde_json::json!({ "Turn": { "item": 1, "turns": count } });
        let event: Event = serde_json::from_value(recorded).expect("a recorded signed turn reads");
        prop_assert_eq!(event, Event::Turn { item: 1, turns: Turns::of(count) });
    }
}

proptest! {
    /// The size stays within its clamps, and after the pause its thumbnail is the controller's own, and a saved height is
    /// the row height the pause settled on.
    #[test]
    fn size_stays_clamped_and_settles_to_the_thumbnails(events in prop::collection::vec(size_event(), 1..40)) {
        let mut c = scaled(listing(""));
        let mut now = 0u64;
        for event in events {
            now += 10;
            c.update(now, Event::Size(event));
            let r = c.grid_size().row_height().get();
            prop_assert!(pg_model::size::ROW_HEIGHT.contains(&r));
        }
        if let Some(at) = c.next_deadline() {
            let settled = c.update(at, Event::Timeout);
            let thumb = c.grid_size().thumb_height().expect("the scale arrived");
            let shown = settled.iter().filter(|c| matches!(c, Command::Size(size::Change::ThumbHeight(_)))).count();
            prop_assert!(shown <= 1, "a thumbnail height is emitted once per pause: {settled:?}");
            for command in &settled {
                if let Command::Size(size::Change::ThumbHeight(t)) = command {
                    prop_assert_eq!(*t, thumb);
                }
                if let Command::Size(size::Change::SaveRowHeight(r)) = command {
                    prop_assert_eq!(*r, c.grid_size().row_height());
                }
            }
        }
    }

    /// Query edits closer together than the pause leave one list, at the last edit plus the pause.
    #[test]
    fn edits_closer_than_the_pause_leave_one_list_at_the_last(gaps in prop::collection::vec(0u64..300, 1..30)) {
        let mut c = controller(listing(""));
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
    fn the_start_finishes_once_whatever_the_order(order in 0usize..3, count in 1usize..12) {
        let tree = Event::TreeLoaded { folder_known: true };
        let shown = Event::ListShown { count, generation: None };
        let placed = Event::ListPositioned;
        // The list is shown before it is placed; the tree may come before, between or after them.
        let events = match order {
            0 => [tree, shown, placed],
            1 => [shown, tree, placed],
            _ => [shown, placed, tree],
        };
        let mut c = controller(listing(""));
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
        let mut c = controller(listing(&folder));
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
        let mut c = controller(listing(""));
        c.update(0, Event::ListShown { count: 60, generation: None });
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
            listing: Listing {
                folder: odd.clone(),
                recursive: index,
                query: Query::default(),
                sort: Sort { key: SortKey::Name, descending: index },
            },
        };
        let events = [
            Event::Size(size),
            Event::CurrentChanged(index.then_some(item)),
            Event::Key(GridKey::Step { dx: -1, dy: 2 }),
            Event::SetBorderMark { item, marked: index },
            Event::Listing(Box::new(listing_event::Event::OpenFolder(odd.clone()))),
            Event::Listing(Box::new(listing_event::Event::OpenAll)),
            Event::Listing(Box::new(listing_event::Event::OpenRoots)),
            Event::Listing(Box::new(listing_event::Event::ViewApplied(view))),
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

/// A controller whose start is over: the tree and the first list are in and placed.
fn started(listing: Listing) -> Controller {
    let mut c = controller(listing);
    c.update(0, Event::TreeLoaded { folder_known: true });
    c.update(0, Event::ListShown { count: 10, generation: None });
    c.update(0, Event::ListPositioned);
    assert!(c.started());
    c
}

/// Nothing is saved before the start is over: a flush then makes no session save and no last-file save, so a start
/// option never becomes the session's.
#[test]
fn nothing_is_saved_before_the_start_is_over() {
    let mut c = controller(listing(""));
    c.update(0, Event::CurrentChanged(Some(3)));
    let flushed = c.update(1, Event::Flush);
    assert_eq!(saves_and_lists(&flushed), (0, 0, 0), "{flushed:?}");
}

/// The session's last file is selected in the first list shown: the gate asks, and the model selects it if it is there.
#[test]
fn the_last_file_is_restored_in_the_first_list_shown() {
    let start = Start { last_file: Some(FileId::new(5)), ..Start::new(listing(""), ROW_200, ROW_200) };
    let mut c = Controller::new(start);
    let out = c.update(0, Event::ListShown { count: 10, generation: None });
    assert!(out.contains(&Command::RestoreSelection(FileId::new(5))), "{out:?}");
}

/// A view applied at the start is remembered once the start is over, and not before: the view is what the session is
/// from then on. Without an applied view the start saves nothing.
#[test]
fn an_applied_view_is_remembered_once_the_start_is_over() {
    let applied = Start { apply_view: true, ..Start::new(listing(""), ROW_200, ROW_200) };
    let mut c = Controller::new(applied);
    let mut out = c.update(0, Event::TreeLoaded { folder_known: true });
    out.extend(c.update(0, Event::ListShown { count: 10, generation: None }));
    out.extend(c.update(0, Event::ListPositioned));
    let saved = out.iter().position(|c| matches!(c, Command::SaveSession(_)));
    let started = out.iter().position(|c| matches!(c, Command::Started));
    assert!(saved.is_some() && saved < started, "remembered before Started: {out:?}");
    let mut plain = controller(listing(""));
    let mut out = plain.update(0, Event::TreeLoaded { folder_known: true });
    out.extend(plain.update(0, Event::ListShown { count: 10, generation: None }));
    out.extend(plain.update(0, Event::ListPositioned));
    assert!(!out.iter().any(|c| matches!(c, Command::SaveSession(_))), "{out:?}");
}

/// A session save waiting at the close is made, with the last file's. Only once the start is over: before it, the
/// listing is the start's, and nothing is saved.
#[test]
fn a_pending_session_save_is_made_at_the_close_with_the_last_file() {
    let mut c = started(listing(""));
    c.update(1, Event::CurrentChanged(Some(3)));
    let flushed = c.update(2, Event::Flush);
    assert_eq!(saves_and_lists(&flushed), (1, 1, 0), "{flushed:?}");
}

/// A query typed within its pause of the close is saved with the session, and no list is shown for a closing window.
#[test]
fn a_pending_query_is_saved_at_the_close_without_a_list() {
    let mut c = started(listing(""));
    let query = Query { name: "beach".into(), ..Query::default() };
    c.update(0, Event::Listing(Box::new(listing_event::Event::QueryChanged(query.clone()))));
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
    let mut c = started(listing(""));
    c.update(1, Event::CurrentChanged(Some(3)));
    c.update(
        2,
        Event::Listing(Box::new(listing_event::Event::QueryChanged(Query { name: "x".into(), ..Query::default() }))),
    );
    c.update(2, Event::QueryEdited);
    let flushed = c.update(3, Event::Flush);
    assert_eq!(saves_and_lists(&flushed), (1, 1, 0), "{flushed:?}");
    assert!(c.next_deadline().is_none(), "nothing is left waiting");
}

/// A rating made in the viewer rates the picture and shows the next one: the controller's step, not the viewer's.
#[test]
fn a_rating_in_the_viewer_rates_and_then_shows_the_next_picture() {
    let mut c = started(listing(""));
    let rating = pg_model::rating::Rating::new(4).expect("four stars is a rating");
    let out = c.update(1, Event::RateInViewer { item: 2, rating: Some(rating) });
    assert_eq!(out.first(), Some(&Command::Rate { item: 2, rating: Some(rating) }), "{out:?}");
    assert!(out.contains(&Command::ShowViewer(3)), "{out:?}");
}

/// The 0 key takes the rating away: the command carries `None`, not a rating of zero.
#[test]
fn the_zero_key_takes_the_rating_away() {
    let mut c = started(listing(""));
    c.update(0, Event::CurrentChanged(Some(3)));
    let out = c.update(3, Event::Key(GridKey::Rate(0)));
    assert!(out.contains(&Command::Rate { item: 3, rating: None }), "{out:?}");
}

/// A viewer's 0 takes the rating away, and the viewer still moves on to the next picture.
#[test]
fn a_zero_in_the_viewer_takes_the_rating_away_and_shows_the_next_picture() {
    let mut c = started(listing(""));
    let out = c.update(1, Event::RateInViewer { item: 2, rating: None });
    assert_eq!(out.first(), Some(&Command::Rate { item: 2, rating: None }), "{out:?}");
    assert!(out.contains(&Command::ShowViewer(3)), "{out:?}");
}

/// The last picture stays where it is: a rating there rates it and shows nothing else.
#[test]
fn a_rating_of_the_last_picture_in_the_viewer_shows_no_other() {
    let mut c = started(listing(""));
    let rating = pg_model::rating::Rating::new(5).expect("five stars is a rating");
    let out = c.update(1, Event::RateInViewer { item: 9, rating: Some(rating) });
    assert!(out.contains(&Command::Rate { item: 9, rating: Some(rating) }), "{out:?}");
    assert!(!out.iter().any(|c| matches!(c, Command::ShowViewer(_))), "{out:?}");
}

/// The window's selection and its similar-search requests are commands the controller hands on, unchanged.
#[test]
fn selection_and_similar_search_are_handed_on_to_the_window() {
    let mut c = started(listing(""));
    assert!(c.update(1, Event::Select(4)).contains(&Command::Select(4)));
    let file = FileId::new(7);
    assert!(c.update(2, Event::FindSimilar(file)).contains(&Command::FindSimilar(file)));
    assert!(c.update(3, Event::FindSimilarCommandLine).contains(&Command::FindSimilarCommandLine));
}

/// A listing that shows the failed files takes the inbox away, and the viewer only when it is open.
#[test]
fn a_listing_of_failures_closes_the_inbox_and_an_open_viewer_only() {
    use pg_client::controller::panels::{Event as PanelEvent, PanelKind};
    let mut c = started(listing(""));
    let closed = c.update(1, Event::Listing(Box::new(listing_event::Event::FailedImports)));
    assert!(closed.contains(&Command::ClosePanel(PanelKind::Inbox)), "{closed:?}");
    assert!(!closed.contains(&Command::CloseViewer), "{closed:?}");
    c.update(2, Event::Panel(PanelEvent::SetViewerOpen(true)));
    let with_viewer = c.update(3, Event::Listing(Box::new(listing_event::Event::FailedImports)));
    assert!(with_viewer.contains(&Command::CloseViewer), "{with_viewer:?}");
}
