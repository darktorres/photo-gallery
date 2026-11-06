//! Traces of the grid's input policy (S-12.16).

use pg_client::controller::input::{Command, Event, Input};
use pg_client::keys::Grid;

fn run(events: &[Event]) -> (Input, Vec<Vec<Command>>) {
    let mut input = Input::default();
    let out = events.iter().map(|e| input.update(*e)).collect();
    (input, out)
}

#[test]
fn keys_act_on_the_current_item_only_once_one_is_selected() {
    let (_, out) = run(&[
        Event::ListShown { count: 10 },
        Event::Key(Grid::Open),
        Event::Key(Grid::Rate(4)),
        Event::Key(Grid::MarkBorder),
        Event::CurrentChanged(Some(3)),
        Event::Key(Grid::Open),
        Event::Key(Grid::Rate(4)),
        Event::Key(Grid::Turn { forward: true }),
    ]);
    assert_eq!(out[1], vec![]);
    assert_eq!(out[2], vec![]);
    assert_eq!(out[3], vec![]);
    assert_eq!(out[5], vec![Command::ShowViewer(3)]);
    assert_eq!(out[6], vec![Command::Act(Grid::Rate(4))]);
    assert_eq!(out[7], vec![Command::Act(Grid::Turn { forward: true })]);
}

#[test]
fn steps_are_left_to_the_layout_and_first_and_last_resolve_by_count() {
    let (_, out) = run(&[
        Event::Key(Grid::Step { dx: 1, dy: 0 }),
        Event::Key(Grid::First),
        Event::Key(Grid::Last),
        Event::ListShown { count: 0 },
        Event::Key(Grid::First),
        Event::Key(Grid::Last),
        Event::ListShown { count: 5 },
        Event::Key(Grid::Last),
    ]);
    assert_eq!(out[0], vec![Command::Step { dx: 1, dy: 0 }]);
    assert_eq!(out[1], vec![]);
    assert_eq!(out[2], vec![]);
    assert_eq!(out[3], vec![]);
    assert_eq!(out[4], vec![]);
    assert_eq!(out[5], vec![]);
    assert_eq!(out[7], vec![Command::Select(4)]);
}

#[test]
fn a_list_that_shrinks_past_the_current_item_clears_it() {
    let (input, out) = run(&[
        Event::ListShown { count: 10 },
        Event::CurrentChanged(Some(7)),
        Event::ListShown { count: 5 },
        Event::Key(Grid::Open),
    ]);
    assert!(!input.has_current());
    assert_eq!(out[3], vec![]);
}

#[test]
fn size_and_panel_keys_are_forwarded() {
    let (_, out) =
        run(&[Event::Key(Grid::Resize { grow: true }), Event::Key(Grid::Filters), Event::Key(Grid::Borders)]);
    assert_eq!(out[0], vec![Command::Forward(Grid::Resize { grow: true })]);
    assert_eq!(out[1], vec![Command::Forward(Grid::Filters)]);
    assert_eq!(out[2], vec![Command::Forward(Grid::Borders)]);
}

#[test]
fn viewer_steps_are_clamped_to_the_list() {
    let (input, _) = run(&[Event::ListShown { count: 4 }]);
    assert_eq!(input.viewer_step(0, -1), 0);
    assert_eq!(input.viewer_step(0, 1), 1);
    assert_eq!(input.viewer_step(3, 1), 3);
    assert_eq!(input.viewer_step(2, -5), 0);
    let (empty, _) = run(&[]);
    assert_eq!(empty.viewer_step(0, 1), 0);
}
