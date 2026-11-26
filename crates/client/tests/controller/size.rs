//! Traces of the grid's size through the public `GridSize`: the clamp, the steps, and the thumbnail height.

use pg_client::controller::size::{Change, Event, GridSize};
use pg_model::size::{Factor, RowHeight, Scale, ThumbHeight};

use super::common::ROW_200;

const ROW_100: RowHeight = RowHeight::new(100).expect("a row height in range");
const ROW_101: RowHeight = RowHeight::new(101).expect("a row height in range");
const ROW_230: RowHeight = RowHeight::new(230).expect("a row height in range");
const SCALE_1_5: Scale = Scale::new(1.5).expect("a positive scale");

#[test]
fn steps_grow_and_shrink_by_the_ctrl_factor_and_clamp() {
    let mut size = GridSize::new(ROW_200, ROW_200);
    assert_eq!(size.row_height(), ROW_200);
    assert_eq!(size.update(Event::Resize { grow: true }), vec![Change::RowHeight(ROW_230)]);
    assert_eq!(size.update(Event::Resize { grow: false }), vec![Change::RowHeight(ROW_200)]);
    assert_eq!(size.update(Event::Chosen(RowHeight::MIN)), vec![Change::RowHeight(RowHeight::MIN)]);
    assert_eq!(size.update(Event::Resize { grow: false }), vec![]);
    assert_eq!(size.update(Event::Chosen(RowHeight::MAX)), vec![Change::RowHeight(RowHeight::MAX)]);
    assert_eq!(size.update(Event::Factor(Factor::new(2.0).expect("a factor"))), vec![]);
}

#[test]
fn factors_multiply_the_row_height() {
    let mut size = GridSize::new(ROW_200, ROW_200);
    assert_eq!(size.update(Event::Factor(Factor::new(0.5).expect("a factor"))), vec![Change::RowHeight(ROW_100)]);
    assert_eq!(size.update(Event::Factor(Factor::new(1.0).expect("a factor"))), vec![]);
}

#[test]
fn the_thumbnail_height_is_the_row_height_times_the_scale() {
    let mut size = GridSize::new(ROW_200, ROW_200);
    assert_eq!(size.update(Event::Scale(SCALE_1_5)), vec![Change::ThumbHeight(ThumbHeight::new(300).unwrap())]);
    assert_eq!(size.update(Event::Chosen(ROW_101)), vec![Change::RowHeight(ROW_101)]);
    assert_eq!(size.thumb_height(), Some(ThumbHeight::new(152).unwrap()));
    assert_eq!(
        size.settled(),
        vec![Change::ThumbHeight(ThumbHeight::new(152).unwrap()), Change::SaveRowHeight(ROW_101)]
    );
}
