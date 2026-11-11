//! The grid's entries as the model holds them: what a list of files shows of each file (`Entry`, 56 bytes), and
//! what the grid's borders show of it. The catalog reads them; the front ends draw from them.

use crate::detection;
use crate::ids::FileId;
use crate::media::{Ext, Kind};
use crate::orientation::Orientation;
use crate::media::MediaType;
use crate::rating::Rating;
use crate::turns::Turns;

/// A file as the grid holds it: what laying it out, drawing its thumbnail and badge, and naming its image take.
/// The rest is read when the file is shown (`details`). Small and without a `String`, so a list of 190,000 is
/// 10.6 MB and quick to build.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Entry {
    pub id: FileId,
    /// With `rotation`, what makes an image URL current.
    pub mtime: i64,
    /// As analyzed, without `pending_turns`; 0 until the file is imported.
    pub width: u32,
    pub height: u32,
    /// Videos, once imported; `Entry::NO_DURATION` otherwise.
    pub duration_ms: u32,
    pub ext: Ext,
    /// Catalog rotation (quarter turns clockwise).
    pub rotation: Turns,
    /// Quarter turns clockwise asked for but not yet written (`queue_rotation`): the grid shows them already.
    pub pending_turns: Turns,
    /// The rating in stars, `None` when unrated (the catalog keeps no row for it).
    pub rating: Option<Rating>,
    /// More than one frame (known after the import).
    pub animated: bool,
    /// Orientation (1–8) of the analysis: what turns the stored pixels upright, the catalog rotation included.
    pub orientation: Orientation,
    /// Borders found: stored pixels cut from the left, top, right and bottom (`analysis.cut_*`), all 0 for none.
    pub cut: [u32; 4],
    pub borders: Borders,
}

const _: () = assert!(size_of::<Entry>() == 56, "Entry's size, in its doc");

/// Whether a file's borders were looked for, and what came of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Borders {
    /// Not looked for: not imported yet, a video, an animated or a tiny image.
    Unknown,
    None,
    Found,
    /// Found, and marked as not a border (the review).
    Marked,
}

impl Entry {
    pub const NO_DURATION: u32 = u32::MAX;

    pub fn kind(&self) -> Kind {
        self.ext.kind()
    }

    pub fn media_type(&self) -> MediaType {
        match self.kind() {
            Kind::Video => MediaType::Video,
            Kind::Image if self.animated => MediaType::Animated,
            Kind::Image => MediaType::Image,
        }
    }

    pub fn duration_ms(&self) -> Option<u32> {
        (self.duration_ms != Entry::NO_DURATION).then_some(self.duration_ms)
    }

    /// Width and height as shown, pending turns included; 0 before the import.
    pub fn shown_size(&self) -> (u32, u32) {
        if self.pending_turns.swaps_axes() { (self.height, self.width) } else { (self.width, self.height) }
    }

    /// Width ÷ height as shown; 1 before the import.
    pub fn aspect(&self) -> f64 {
        let (w, h) = self.shown_size();
        if w > 0 && h > 0 { f64::from(w) / f64::from(h) } else { 1.0 }
    }

    /// What is left without the borders, as fractions of the image as shown (pending turns included): x, y, width,
    /// height. `None` without borders found (marked ones included).
    pub fn content(&self) -> Option<[f64; 4]> {
        if !matches!(self.borders, Borders::Found | Borders::Marked) {
            return None;
        }
        let [l, t, r, b] = self.cut.map(f64::from);
        let (w, h) = self.stored_size()?;
        Some(self.shown([l, t, w - r, h - b]))
    }

    /// What cropping the borders cuts from the left, top, right and bottom of the image as shown, in pixels. `None`
    /// without borders found (marked ones included).
    pub fn shown_cut(&self) -> Option<[u32; 4]> {
        let [x, y, w, h] = self.content()?;
        let (sw, sh) = self.shown_size();
        let px =
            |f: f64, side: u32| crate::num::f64_to_u32((f * f64::from(side)).round().clamp(0.0, f64::from(u32::MAX)));
        Some([px(x, sw), px(y, sh), px(1.0 - x - w, sw), px(1.0 - y - h, sh)])
    }

    /// Where a border detector's side would cut, `depth` stored pixels from its edge of the stored image, as a line
    /// across the image as shown: x, y, width, height in fractions, one of them 0.
    pub fn side_line(&self, edge: detection::Edge, depth: u32) -> Option<[f64; 4]> {
        let (w, h) = self.stored_size()?;
        let d = f64::from(depth);
        Some(self.shown(match edge {
            detection::Edge::Top => [0.0, d, w, d],
            detection::Edge::Bottom => [0.0, h - d, w, h - d],
            detection::Edge::Left => [d, 0.0, d, h],
            detection::Edge::Right => [w - d, 0.0, w - d, h],
        }))
    }

    /// The stored image's size; the analysis' is upright, its transpose when the orientation turns it. `None`
    /// before the import.
    fn stored_size(&self) -> Option<(f64, f64)> {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        (w > 0.0 && h > 0.0).then_some(if self.orientation.swaps_axes() { (h, w) } else { (w, h) })
    }

    /// A rectangle of the stored image in pixels (left, top, right, bottom) as fractions of the image as shown:
    /// x, y, width, height.
    fn shown(&self, rect: [f64; 4]) -> [f64; 4] {
        let Some((w, h)) = self.stored_size() else { return [0.0; 4] };
        let [l, t, r, b] = rect;
        let [x0, y0, x1, y1] = self.orientation.upright_rect([l / w, t / h, r / w, b / h], self.pending_turns);
        [x0, y0, x1 - x0, y1 - y0]
    }
}
