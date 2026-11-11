//! The grid's size (S-12.20): the row height the user chose (Ctrl +/−, the size box, a pinch), clamped, and the thumbnail
//! height it asks for (the row height times the screen's scale). The row height changes at once; the thumbnail height and
//! the saved row height follow after a pause, which the deadlines (`deadlines.rs`) time. Nothing here calls Qt.
use serde::{Deserialize, Serialize};

/// The smallest row height the grid takes (pixels).
pub const MIN_ROW_HEIGHT: i32 = 60;
/// The largest row height the grid takes (pixels).
pub const MAX_ROW_HEIGHT: i32 = 800;
/// One Ctrl + step: the rows grow or shrink by this factor.
const STEP: f64 = 1.15;

/// A request about the size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// One Ctrl +/− step: grow, or shrink.
    Resize { grow: bool },
    /// The row height multiplied by `factor` (a pinch or a wheel step).
    Factor(f64),
    /// The row height chosen (the size box, the session's saved height).
    Chosen(f64),
    /// The screen's scale (device pixel ratio) changed.
    Scale(f64),
}

/// What the desktop does about a size change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// The row height is now this (the grid re-lays out at once).
    RowHeight(i32),
    /// The thumbnail height is now this (the library asks for thumbnails of it).
    ThumbHeight(i32),
    /// Remember this row height for the next start (after a pause).
    SaveRowHeight(i32),
}

/// The size, as the core holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSize {
    row_height: i32,
    scale: f64,
}

impl Default for GridSize {
    fn default() -> GridSize {
        GridSize { row_height: 200, scale: 1.0 }
    }
}

impl GridSize {
    /// The size a window starts with: the row height (clamped like any size) and the screen's scale.
    pub fn with(row_height: i32, scale: f64) -> GridSize {
        GridSize { row_height: row_height.clamp(MIN_ROW_HEIGHT, MAX_ROW_HEIGHT), scale }
    }

    /// The row height the grid has.
    pub fn row_height(&self) -> i32 {
        self.row_height
    }

    /// The screen's scale (device pixel ratio) the thumbnails are sized for.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The thumbnail height for the row height and the scale.
    pub fn thumb_height(&self) -> i32 {
        round_i32(f64::from(self.row_height) * self.scale)
    }

    /// Apply `event`: the changes it makes, in order. A row height that stays the same changes nothing.
    pub fn update(&mut self, event: Event) -> Vec<Change> {
        match event {
            Event::Resize { grow } => {
                let factor = if grow { STEP } else { 1.0 / STEP };
                self.set_row(f64::from(self.row_height) * factor)
            }
            Event::Factor(factor) => self.set_row(f64::from(self.row_height) * factor),
            Event::Chosen(height) => self.set_row(height),
            Event::Scale(scale) => {
                self.scale = scale;
                vec![Change::ThumbHeight(self.thumb_height())]
            }
        }
    }

    /// The thumbnail and the saved height, after the pause that follows a change to the row height.
    pub fn settled(&self) -> Vec<Change> {
        vec![Change::ThumbHeight(self.thumb_height()), Change::SaveRowHeight(self.row_height)]
    }

    fn set_row(&mut self, height: f64) -> Vec<Change> {
        let height = round_i32(height.clamp(f64::from(MIN_ROW_HEIGHT), f64::from(MAX_ROW_HEIGHT)));
        if height == self.row_height {
            return Vec::new();
        }
        self.row_height = height;
        vec![Change::RowHeight(height)]
    }
}

/// `v` rounded to the nearest whole pixel, held to `i32`'s range (NaN gives 0, as a float cast does).
fn round_i32(v: f64) -> i32 {
    pg_model::num::f64_to_i32(v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)))
}
