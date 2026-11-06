//! The start gate's traces (`pg_client::controller::start`): the tree and the list in either order, a list placed after
//! the grid's width, a folder the tree no longer has, and the events after the start.

use pg_client::controller::start::{Command, Event, StartGate};
use proptest::prelude::*;

use Command::{OpenAll, Select, Started};
use Event::{ListPositioned, ListShown, TreeLoaded};

/// Runs `events` through a new gate, and the commands they give in order.
fn run(events: &[Event]) -> Vec<Command> {
    let mut gate = StartGate::default();
    events.iter().flat_map(|&e| gate.update(e)).collect()
}

#[test]
fn the_start_traces() {
    // The cases the desktop can produce. A list "placed" is `ListShown` then `ListPositioned`.
    let cases: [(&str, &[Event], &[Command]); 9] = [
        (
            "tree first, then the list with the session's file",
            &[TreeLoaded { folder_known: true }, ListShown { last_file: Some(3) }, ListPositioned],
            &[Select(3), Started],
        ),
        (
            "list first, then the tree: the start waits for the tree",
            &[ListShown { last_file: Some(3) }, ListPositioned, TreeLoaded { folder_known: true }],
            &[Select(3), Started],
        ),
        (
            "the list has no file the session left",
            &[TreeLoaded { folder_known: true }, ListShown { last_file: None }, ListPositioned],
            &[Started],
        ),
        (
            "a list placed after the grid's width (the relayout) still starts",
            &[ListShown { last_file: Some(2) }, TreeLoaded { folder_known: true }, ListPositioned],
            &[Select(2), Started],
        ),
        (
            "the folder is gone at the start: all folders are listed, then the start",
            &[TreeLoaded { folder_known: false }, ListShown { last_file: None }, ListPositioned],
            &[OpenAll, Started],
        ),
        (
            "a list of the gone folder before the tree says so: the start waits for the list asked again",
            &[
                ListShown { last_file: Some(1) },
                ListPositioned,
                TreeLoaded { folder_known: false },
                ListShown { last_file: None },
                ListPositioned,
            ],
            &[Select(1), OpenAll, Started],
        ),
        (
            "a second tree and list after the start change nothing",
            &[
                TreeLoaded { folder_known: true },
                ListShown { last_file: Some(0) },
                ListPositioned,
                TreeLoaded { folder_known: true },
                ListShown { last_file: Some(5) },
                ListPositioned,
            ],
            &[Select(0), Started],
        ),
        (
            "a folder gone after the start lists all folders again",
            &[
                TreeLoaded { folder_known: true },
                ListShown { last_file: None },
                ListPositioned,
                TreeLoaded { folder_known: false },
                ListShown { last_file: None },
                ListPositioned,
            ],
            &[Started, OpenAll],
        ),
        ("the list alone does not start the window", &[ListShown { last_file: Some(4) }, ListPositioned], &[Select(4)]),
    ];
    for (name, events, expected) in cases {
        assert_eq!(run(events), expected, "{name}");
    }
}

#[test]
fn a_list_shown_but_not_placed_is_not_the_start() {
    let mut gate = StartGate::default();
    assert!(gate.update(TreeLoaded { folder_known: true }).is_empty());
    assert!(gate.update(ListShown { last_file: None }).is_empty());
    assert!(!gate.is_started());
    assert_eq!(gate.update(ListPositioned), [Started]);
    assert!(gate.is_started());
}

fn event() -> impl Strategy<Value = Event> {
    prop_oneof![
        any::<bool>().prop_map(|folder_known| TreeLoaded { folder_known }),
        prop::option::of(0usize..100).prop_map(|last_file| ListShown { last_file }),
        Just(ListPositioned),
    ]
}

proptest! {
    // Whatever the order: the start happens at most once, only after the tree and a placed list were reported, and no
    // selection is asked for once it is over.
    #[test]
    fn the_start_happens_once_and_after_its_parts(events in prop::collection::vec(event(), 0..40)) {
        let mut gate = StartGate::default();
        let mut tree = false;
        let mut placed = false;
        let mut started = 0;
        for e in events {
            match e {
                TreeLoaded { folder_known } => {
                    tree = true;
                    if !folder_known {
                        placed = false;
                    }
                }
                ListShown { .. } => {}
                ListPositioned => placed = true,
            }
            for c in gate.update(e) {
                match c {
                    Started => {
                        started += 1;
                        prop_assert!(tree && placed, "started before the tree and a placed list");
                    }
                    Select(_) => prop_assert_eq!(started, 0, "a selection after the start"),
                    OpenAll => {}
                }
            }
        }
        prop_assert!(started <= 1);
    }
}
