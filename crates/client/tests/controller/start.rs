//! The start gate's traces (`pg_client::controller::start`): the tree and the list in either order, a list placed after
//! the grid's width, a folder the tree no longer has (the listing stays as it is), a failed tree read (a state the gate
//! passes), and the events after the start. The session's last file (the prefs') is the gate's, and its selection is
//! asked for at each list shown until the start is over.

use pg_client::controller::start::{Command, Event, StartGate};
use pg_model::ids::FileId;
use proptest::prelude::*;

use Command::{RestoreSelection, Started};
use Event::{ListPositioned, ListShown, TreeFailed, TreeLoaded};

/// Runs `events` through a new gate that restores `last_file`, and the commands they give in order.
fn run(last_file: Option<FileId>, events: &[Event]) -> Vec<Command> {
    let mut gate = StartGate::new(last_file);
    events.iter().flat_map(|&e| gate.update(e)).collect()
}

fn file(id: i64) -> FileId {
    FileId::new(id)
}

#[test]
fn the_start_traces() {
    // The cases the desktop can produce. A list "placed" is `ListShown` then `ListPositioned`.
    /// A case: its name, the file the session left, the events, and the commands they must give.
    type Case<'a> = (&'a str, Option<FileId>, &'a [Event], &'a [Command]);
    let cases: [Case<'_>; 9] = [
        (
            "tree first, then the list with the session's file",
            Some(file(3)),
            &[TreeLoaded, ListShown, ListPositioned],
            &[RestoreSelection(file(3)), Started],
        ),
        (
            "list first, then the tree: the start waits for the tree",
            Some(file(3)),
            &[ListShown, ListPositioned, TreeLoaded],
            &[RestoreSelection(file(3)), Started],
        ),
        ("no file was remembered: nothing is restored", None, &[TreeLoaded, ListShown, ListPositioned], &[Started]),
        (
            "a list placed after the grid's width (the relayout) still starts",
            Some(file(2)),
            &[ListShown, TreeLoaded, ListPositioned],
            &[RestoreSelection(file(2)), Started],
        ),
        (
            "a folder the tree no longer has is listed as it is: the start goes on with that listing",
            Some(file(1)),
            &[TreeLoaded, ListShown, ListPositioned],
            &[RestoreSelection(file(1)), Started],
        ),
        (
            "a failed tree read is a state the gate passes: the start goes on with the listing it has",
            Some(file(2)),
            &[TreeFailed, ListShown, ListPositioned],
            &[RestoreSelection(file(2)), Started],
        ),
        (
            "a tree that fails after the list is shown still starts once it is placed",
            Some(file(2)),
            &[ListShown, ListPositioned, TreeFailed],
            &[RestoreSelection(file(2)), Started],
        ),
        (
            "a second tree and list after the start change nothing",
            Some(file(0)),
            &[TreeLoaded, ListShown, ListPositioned, TreeLoaded, ListShown, ListPositioned],
            &[RestoreSelection(file(0)), Started],
        ),
        (
            "the list alone does not start the window",
            Some(file(4)),
            &[ListShown, ListPositioned],
            &[RestoreSelection(file(4))],
        ),
    ];
    for (name, last_file, events, expected) in cases {
        assert_eq!(run(last_file, events), expected, "{name}");
    }
}

#[test]
fn a_list_shown_but_not_placed_is_not_the_start() {
    let mut gate = StartGate::new(None);
    assert!(gate.update(TreeLoaded).is_empty());
    assert!(gate.update(ListShown).is_empty());
    assert!(!gate.is_started());
    assert_eq!(gate.update(ListPositioned), [Started]);
    assert!(gate.is_started());
}

fn event() -> impl Strategy<Value = Event> {
    prop_oneof![Just(TreeLoaded), Just(TreeFailed), Just(ListShown), Just(ListPositioned)]
}

proptest! {
    // Whatever the order: the start happens at most once, only after the tree (loaded or failed) and a placed list were
    // reported, and no selection is asked for once it is over.
    #[test]
    fn the_start_happens_once_and_after_its_parts(events in prop::collection::vec(event(), 0..40)) {
        let mut gate = StartGate::new(Some(file(7)));
        let mut tree = false;
        let mut placed = false;
        let mut started = 0;
        for e in events {
            match e {
                TreeLoaded | TreeFailed => tree = true,
                ListShown => {}
                ListPositioned => placed = true,
            }
            for c in gate.update(e) {
                match c {
                    Started => {
                        started += 1;
                        prop_assert!(tree && placed, "started before the tree and a placed list");
                    }
                    RestoreSelection(_) => prop_assert_eq!(started, 0, "a selection after the start"),
                }
            }
        }
        prop_assert!(started <= 1);
    }
}
