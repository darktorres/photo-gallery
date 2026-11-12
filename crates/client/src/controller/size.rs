//! The grid's size (S-12.20): the row height the user chose (Ctrl +/−, the size box, a pinch), clamped, and the thumbnail
//! height it asks for (the row height times the screen's scale). The row height changes at once; the thumbnail height and
//! the saved row height follow after a pause, which the deadlines (`deadlines.rs`) time. Nothing here calls Qt.
//!
//! Every value is one of `pg_model::size`'s types, so a bad value is refused where it is made (the shell), and nothing
//! here holds a made-up size: a scale that hasn't arrived is `None`, and no thumbnail height is named until it has.
use pg_model::size::{Factor, RowHeight, Scale, ThumbHeight};
use serde::{Deserialize, Serialize};

/// One Ctrl + step: the rows grow or shrink by this factor.
const STEP: f64 = 1.15;

/// A request about the size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// One Ctrl +/− step: grow, or shrink.
    Resize { grow: bool },
    /// The row height multiplied by `factor` (a pinch or a wheel step).
    Factor(Factor),
    /// The row height chosen (the size box, the session's saved height).
    Chosen(RowHeight),
    /// The window's scale (its device pixel ratio) changed, or arrived at the start.
    Scale(Scale),
}

/// What the desktop does about a size change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// The row height is now this (the grid re-lays out at once).
    RowHeight(RowHeight),
    /// The thumbnail height is now this (the library asks for thumbnails of it).
    ThumbHeight(ThumbHeight),
    /// Remember this row height for the next start (after a pause).
    SaveRowHeight(RowHeight),
}

/// The size, as the core holds it. `shown_thumb` is the thumbnail height last emitted, and `saved_row` the row height last
/// saved (the prefs' at the start), so a change is emitted only when it differs from what the desktop has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSize {
    row: RowHeight,
    scale: Option<Scale>,
    shown_thumb: Option<ThumbHeight>,
    saved_row: RowHeight,
}

impl GridSize {
    /// The size a window starts with: the row height it shows, and the row height the prefs have saved.
    pub fn new(row: RowHeight, saved_row: RowHeight) -> GridSize {
        GridSize { row, scale: None, shown_thumb: None, saved_row }
    }

    /// The row height the grid has.
    pub fn row_height(&self) -> RowHeight {
        self.row
    }

    /// The window's scale, once it has arrived.
    pub fn scale(&self) -> Option<Scale> {
        self.scale
    }

    /// The row height last saved (the prefs', until a change is settled and saved).
    pub fn saved_row(&self) -> RowHeight {
        self.saved_row
    }

    /// The thumbnail height the row height and the scale ask for, once the scale has arrived.
    pub fn thumb_height(&self) -> Option<ThumbHeight> {
        self.scale.map(|s| ThumbHeight::for_row(self.row, s))
    }

    /// Apply `event`: the changes it makes, in order. A row height that stays the same changes nothing. A scale changes
    /// the thumbnail height at once; a row height changes it only after the pause (`settled`).
    pub fn update(&mut self, event: Event) -> Vec<Change> {
        match event {
            Event::Resize { grow } => {
                let factor = if grow { STEP } else { 1.0 / STEP };
                self.set_row(f64::from(self.row.get()) * factor)
            }
            Event::Factor(factor) => self.set_row(f64::from(self.row.get()) * factor.get()),
            Event::Chosen(height) => self.set_row_exact(height),
            Event::Scale(scale) => {
                self.scale = Some(scale);
                self.emit_thumb().into_iter().collect()
            }
        }
    }

    /// The thumbnail and the saved row height, after the pause that follows a change to the row height. Each is emitted
    /// only when it differs from the last one emitted or saved, so 220 → 250 → 220 within one pause emits nothing.
    pub fn settled(&mut self) -> Vec<Change> {
        let mut out: Vec<Change> = self.emit_thumb().into_iter().collect();
        if self.row != self.saved_row {
            self.saved_row = self.row;
            out.push(Change::SaveRowHeight(self.row));
        }
        out
    }

    /// The thumbnail height, if it differs from the one last emitted (and the scale has arrived).
    fn emit_thumb(&mut self) -> Option<Change> {
        let thumb = self.thumb_height()?;
        if self.shown_thumb == Some(thumb) {
            return None;
        }
        self.shown_thumb = Some(thumb);
        Some(Change::ThumbHeight(thumb))
    }

    fn set_row(&mut self, height: f64) -> Vec<Change> {
        match RowHeight::from_finite(height) {
            Some(row) => self.set_row_exact(row),
            None => Vec::new(),
        }
    }

    fn set_row_exact(&mut self, height: RowHeight) -> Vec<Change> {
        if height == self.row {
            return Vec::new();
        }
        self.row = height;
        vec![Change::RowHeight(height)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(px: f64) -> RowHeight {
        RowHeight::from_finite(px).expect("a finite row height")
    }

    fn scale(v: f64) -> Scale {
        Scale::new(v).expect("a positive scale")
    }

    #[test]
    fn no_thumbnail_height_before_the_scale_arrives() {
        let mut size = GridSize::new(row(220.0), row(220.0));
        assert_eq!(size.thumb_height(), None);
        assert!(size.settled().is_empty());
    }

    #[test]
    fn the_first_scale_emits_its_thumbnail_height_and_an_equal_one_nothing() {
        let mut size = GridSize::new(row(220.0), row(220.0));
        let first = size.update(Event::Scale(scale(1.0)));
        assert_eq!(first, vec![Change::ThumbHeight(ThumbHeight::new(220).unwrap())]);
        assert!(size.update(Event::Scale(scale(1.0))).is_empty());
    }

    #[test]
    fn a_row_change_is_emitted_at_once_and_its_thumbnail_after_the_pause() {
        let mut size = GridSize::new(row(220.0), row(220.0));
        size.update(Event::Scale(scale(1.0)));
        assert_eq!(size.update(Event::Chosen(row(250.0))), vec![Change::RowHeight(row(250.0))]);
        assert_eq!(
            size.settled(),
            vec![Change::ThumbHeight(ThumbHeight::new(250).unwrap()), Change::SaveRowHeight(row(250.0))]
        );
    }

    #[test]
    fn a_round_trip_within_one_pause_emits_nothing_when_it_ends_where_it_began() {
        let mut size = GridSize::new(row(220.0), row(220.0));
        size.update(Event::Scale(scale(1.0)));
        size.update(Event::Chosen(row(250.0)));
        size.update(Event::Chosen(row(220.0)));
        assert!(size.settled().is_empty());
    }

    #[test]
    fn a_start_at_the_saved_height_with_a_scale_emits_only_the_thumbnail() {
        let mut size = GridSize::new(row(220.0), row(220.0));
        assert_eq!(size.update(Event::Scale(scale(1.0))), vec![Change::ThumbHeight(ThumbHeight::new(220).unwrap())]);
        assert!(size.settled().is_empty(), "nothing saved: the start's height is the saved one");
    }

    #[test]
    fn a_start_row_differing_from_the_saved_one_is_saved_after_the_pause() {
        let mut size = GridSize::new(row(120.0), row(220.0));
        assert_eq!(size.update(Event::Scale(scale(1.0))), vec![Change::ThumbHeight(ThumbHeight::new(120).unwrap())]);
        assert_eq!(size.settled(), vec![Change::SaveRowHeight(row(120.0))], "the thumbnail was emitted by the scale");
    }

    #[test]
    fn steps_stay_in_the_row_range() {
        let mut size = GridSize::new(row(790.0), row(790.0));
        size.update(Event::Factor(Factor::new(10.0).unwrap()));
        assert_eq!(size.row_height(), RowHeight::MAX);
    }
}
