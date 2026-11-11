//! Traces of the grid's size (S-12.20): the clamp, the steps, and the thumbnail height.

use pg_client::controller::size::{Change, Event, GridSize, MAX_ROW_HEIGHT, MIN_ROW_HEIGHT};

#[test]
fn steps_grow_and_shrink_by_the_ctrl_factor_and_clamp() {
    let mut size = GridSize::default();
    assert_eq!(size.row_height(), 200);
    assert_eq!(size.update(Event::Resize { grow: true }), vec![Change::RowHeight(230)]);
    assert_eq!(size.update(Event::Resize { grow: false }), vec![Change::RowHeight(200)]);
    assert_eq!(size.update(Event::Chosen(10.0)), vec![Change::RowHeight(MIN_ROW_HEIGHT)]);
    assert_eq!(size.update(Event::Resize { grow: false }), vec![]);
    assert_eq!(size.update(Event::Chosen(5000.0)), vec![Change::RowHeight(MAX_ROW_HEIGHT)]);
    assert_eq!(size.update(Event::Factor(2.0)), vec![]);
}

#[test]
fn factors_multiply_the_row_height() {
    let mut size = GridSize::default();
    assert_eq!(size.update(Event::Factor(0.5)), vec![Change::RowHeight(100)]);
    assert_eq!(size.update(Event::Factor(1.0)), vec![]);
}

#[test]
fn the_thumbnail_height_is_the_row_height_times_the_scale() {
    let mut size = GridSize::default();
    assert_eq!(size.update(Event::Scale(1.5)), vec![Change::ThumbHeight(300)]);
    assert_eq!(size.update(Event::Chosen(101.0)), vec![Change::RowHeight(101)]);
    assert_eq!(size.thumb_height(), 152);
    assert_eq!(size.settled(), vec![Change::ThumbHeight(152), Change::SaveRowHeight(101)]);
}
