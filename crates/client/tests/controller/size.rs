//! Traces of the grid's size (S-12.20) through the public `GridSize`: the clamp, the steps, and the thumbnail height.

#![expect(
    clippy::expect_used,
    reason = "fixtures: the row heights and scales the helpers build are constants inside their ranges"
)]

use pg_client::controller::size::{Change, Event, GridSize};
use pg_model::size::{Factor, RowHeight, Scale, ThumbHeight};

fn row(px: u32) -> RowHeight {
    RowHeight::new(px).expect("a row height in range")
}

fn scale(v: f64) -> Scale {
    Scale::new(v).expect("a positive scale")
}

#[test]
fn steps_grow_and_shrink_by_the_ctrl_factor_and_clamp() {
    let mut size = GridSize::new(row(200), row(200));
    assert_eq!(size.row_height(), row(200));
    assert_eq!(size.update(Event::Resize { grow: true }), vec![Change::RowHeight(row(230))]);
    assert_eq!(size.update(Event::Resize { grow: false }), vec![Change::RowHeight(row(200))]);
    assert_eq!(size.update(Event::Chosen(RowHeight::MIN)), vec![Change::RowHeight(RowHeight::MIN)]);
    assert_eq!(size.update(Event::Resize { grow: false }), vec![]);
    assert_eq!(size.update(Event::Chosen(RowHeight::MAX)), vec![Change::RowHeight(RowHeight::MAX)]);
    assert_eq!(size.update(Event::Factor(Factor::new(2.0).expect("a factor"))), vec![]);
}

#[test]
fn factors_multiply_the_row_height() {
    let mut size = GridSize::new(row(200), row(200));
    assert_eq!(size.update(Event::Factor(Factor::new(0.5).expect("a factor"))), vec![Change::RowHeight(row(100))]);
    assert_eq!(size.update(Event::Factor(Factor::new(1.0).expect("a factor"))), vec![]);
}

#[test]
fn the_thumbnail_height_is_the_row_height_times_the_scale() {
    let mut size = GridSize::new(row(200), row(200));
    assert_eq!(size.update(Event::Scale(scale(1.5))), vec![Change::ThumbHeight(ThumbHeight::new(300).unwrap())]);
    assert_eq!(size.update(Event::Chosen(row(101))), vec![Change::RowHeight(row(101))]);
    assert_eq!(size.thumb_height(), Some(ThumbHeight::new(152).unwrap()));
    assert_eq!(
        size.settled(),
        vec![Change::ThumbHeight(ThumbHeight::new(152).unwrap()), Change::SaveRowHeight(row(101))]
    );
}
