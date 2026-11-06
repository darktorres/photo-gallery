//! Traces of the grid's size (S-12.20): the clamp, the steps, and the thumbnail height.

use pg_client::controller::prefs::{Change, Event, MAX_ROW_HEIGHT, MIN_ROW_HEIGHT, Prefs};

#[test]
fn steps_grow_and_shrink_by_the_ctrl_factor_and_clamp() {
    let mut prefs = Prefs::default();
    assert_eq!(prefs.row_height(), 200);
    assert_eq!(prefs.update(Event::Resize { grow: true }), vec![Change::RowHeight(230)]);
    assert_eq!(prefs.update(Event::Resize { grow: false }), vec![Change::RowHeight(200)]);
    assert_eq!(prefs.update(Event::Chosen(10.0)), vec![Change::RowHeight(MIN_ROW_HEIGHT)]);
    assert_eq!(prefs.update(Event::Resize { grow: false }), vec![]);
    assert_eq!(prefs.update(Event::Chosen(5000.0)), vec![Change::RowHeight(MAX_ROW_HEIGHT)]);
    assert_eq!(prefs.update(Event::Factor(2.0)), vec![]);
}

#[test]
fn factors_multiply_the_row_height() {
    let mut prefs = Prefs::default();
    assert_eq!(prefs.update(Event::Factor(0.5)), vec![Change::RowHeight(100)]);
    assert_eq!(prefs.update(Event::Factor(1.0)), vec![]);
}

#[test]
fn the_thumbnail_height_is_the_row_height_times_the_scale() {
    let mut prefs = Prefs::default();
    assert_eq!(prefs.update(Event::Scale(1.5)), vec![Change::ThumbHeight(300)]);
    assert_eq!(prefs.update(Event::Chosen(101.0)), vec![Change::RowHeight(101)]);
    assert_eq!(prefs.thumb_height(), 152);
    assert_eq!(prefs.settled(), vec![Change::ThumbHeight(152), Change::SaveRowHeight(101)]);
}
