//! The EXIF orientation (1–8): how a stored picture is shown. Model-shaped: arithmetic over the eight values, with no
//! decoding, metadata or I/O, so the codec, the metadata reader, the video reader, the borders and the catalog can all
//! use it without depending on each other.
//!
//! An orientation is a mirror left-right (applied first), then quarter turns clockwise ([`Orientation::parts`]). The
//! coordinate maps ([`Orientation::shown_pixel`], [`Orientation::shown_edge`]) apply that same mirror and turn to a point
//! of the stored picture: a pixel index, or a fraction of the picture's size. The decoder's transform
//! (`decode::transform_into`) gathers the same mapping, and the decoder's tests check the two against each other.

use std::ops::Sub;

use crate::turns::Turns;

/// An EXIF orientation, 1 to 8: the value of the tag that shows a stored picture upright. The only ways in are
/// [`Orientation::new`] (for a value that must be one) and [`Orientation::from_tag`] (for a tag as the file has it).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Orientation(u8);

impl Orientation {
    /// The picture as stored: nothing to turn.
    pub const UPRIGHT: Orientation = Orientation(1);

    /// The orientation with that EXIF value, or `None` when it is outside 1–8.
    pub const fn new(tag: u8) -> Option<Orientation> {
        match tag {
            1..=8 => Some(Orientation(tag)),
            _ => None,
        }
    }

    /// The orientation a file's tag shows: its value when it is 1–8, and upright otherwise (a broken tag shows the
    /// picture as stored).
    pub const fn from_tag(tag: u8) -> Orientation {
        match Orientation::new(tag) {
            Some(o) => o,
            None => Orientation::UPRIGHT,
        }
    }

    /// The EXIF value, 1–8: what the tag holds, and what the catalog stores.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The orientation's (mirrored, quarter turns clockwise): the upright picture is the stored one mirrored left-right
    /// (if so), then turned.
    pub fn parts(self) -> (bool, Turns) {
        let (mirrored, turns) = match self.0 {
            2 => (true, 0),
            3 => (false, 2),
            4 => (true, 2),
            5 => (true, 3),
            6 => (false, 1),
            7 => (true, 1),
            8 => (false, 3),
            _ => (false, 0),
        };
        (mirrored, Turns::new(turns))
    }

    /// The orientation with that mirror and those quarter turns.
    pub fn from_parts(mirrored: bool, turns: Turns) -> Orientation {
        Orientation(match (mirrored, turns.get()) {
            (false, 0) => 1,
            (true, 0) => 2,
            (false, 2) => 3,
            (true, 2) => 4,
            (true, 3) => 5,
            (false, 1) => 6,
            (true, 1) => 7,
            _ => 8,
        })
    }

    /// The orientation that shows the image `turns` quarter turns further clockwise than `self` does.
    pub fn compose(self, turns: Turns) -> Orientation {
        let (mirrored, t) = self.parts();
        Orientation::from_parts(mirrored, t.plus(turns))
    }

    /// Whether the orientation turns the picture a quarter turn (or three): its width and height swap.
    pub fn swaps_axes(self) -> bool {
        (5..=8).contains(&self.0)
    }

    /// Where the pixel (`x`, `y`) of a `w` × `h` stored picture is shown.
    pub fn shown_pixel(self, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
        shown(self, (x, y), (w, h), 1)
    }

    /// The stored pixel of a `w` × `h` stored picture that is shown at (`x`, `y`) of the upright picture: the inverse
    /// of [`Orientation::shown_pixel`], which the decoder reads its pixels by.
    pub fn source_pixel(self, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
        match self.0 {
            2 => (w - 1 - x, y),
            3 => (w - 1 - x, h - 1 - y),
            4 => (x, h - 1 - y),
            5 => (y, x),
            6 => (y, h - 1 - x),
            7 => (w - 1 - y, h - 1 - x),
            8 => (w - 1 - y, x),
            _ => (x, y),
        }
    }

    /// Where the point (`u`, `v`), as fractions of the stored picture's width and height, is shown.
    pub fn shown_edge(self, u: f64, v: f64) -> (f64, f64) {
        shown(self, (u, v), (1.0, 1.0), 0.0)
    }

    /// A rectangle of the stored picture (fractions: left, top, right, bottom) in the picture as shown: turned upright
    /// by `self` (as [`Orientation::shown_edge`] maps it), then `turns` quarter turns clockwise.
    pub fn upright_rect(self, rect: [f64; 4], turns: Turns) -> [f64; 4] {
        // Where a stored point (u, v) is shown.
        let shown = |(u, v): (f64, f64)| self.shown_edge(u, v);
        let turned = |p: (f64, f64)| (0..turns.get()).fold(p, |(x, y), _| (1.0 - y, x));
        let [l, t, r, b] = rect;
        let (a, c) = (turned(shown((l, t))), turned(shown((r, b))));
        [a.0.min(c.0), a.1.min(c.1), a.0.max(c.0), a.1.max(c.1)]
    }
}

/// Whether `turns` quarter turns (clockwise) swap the picture's sides: an odd number does. The catalog's turns are
/// counted this way, before they are composed with the file's own orientation ([`Orientation::compose`]).
pub fn turns_swap(turns: Turns) -> bool {
    turns.swaps_axes()
}

/// The one coordinate map. `last` is how far the last index lies below its extent: 1 for pixels (the last pixel of a
/// width `w` is `w - 1`), 0 for fractions of a unit extent.
fn shown<T>(o: Orientation, (x, y): (T, T), (w, h): (T, T), last: T) -> (T, T)
where
    T: Copy + Sub<Output = T>,
{
    match o.0 {
        2 => ((w - last) - x, y),
        3 => ((w - last) - x, (h - last) - y),
        4 => (x, (h - last) - y),
        5 => (y, x),
        6 => ((h - last) - y, x),
        7 => ((h - last) - y, (w - last) - x),
        8 => (y, (w - last) - x),
        _ => (x, y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(tag: u8) -> Orientation {
        Orientation::new(tag).unwrap()
    }

    #[test]
    fn only_1_to_8_are_orientations() {
        assert_eq!(Orientation::new(0), None);
        assert_eq!(Orientation::new(9), None);
        assert_eq!(Orientation::from_tag(0), Orientation::UPRIGHT, "a broken tag shows the picture as stored");
        assert_eq!(Orientation::from_tag(6).get(), 6);
    }

    #[test]
    fn composes_orientations() {
        // Four quarter turns are the identity; one turn of the identity is EXIF 6 (90° clockwise).
        for tag in 1..=8 {
            assert_eq!(o(tag).compose(Turns::new(4)), o(tag));
            assert_eq!(o(tag).compose(Turns::new(0)), o(tag));
        }
        assert_eq!(Orientation::UPRIGHT.compose(Turns::new(1)), o(6));
        assert_eq!(o(6).compose(Turns::new(1)), o(3));
        assert_eq!(Orientation::UPRIGHT.compose(Turns::new(3)), o(8));
        assert_eq!(o(2).compose(Turns::new(1)), o(7));
    }

    #[test]
    fn parts_round_trip() {
        for tag in 1..=8 {
            let (mirrored, turns) = o(tag).parts();
            assert_eq!(Orientation::from_parts(mirrored, turns), o(tag));
        }
    }

    #[test]
    fn swapping_follows_the_quarter_turns() {
        assert_eq!((1..=8).filter(|&t| o(t).swaps_axes()).collect::<Vec<_>>(), [5, 6, 7, 8]);
    }

    #[test]
    fn the_catalog_turns_swap_exactly_when_the_composed_orientation_does() {
        // The catalog stores turns on their own (`turns_swap`); the file's own orientation is composed in afterwards
        // (`compose`). Both must agree on whether the sides swap, for every file orientation.
        for tag in 1..=8 {
            for turns in 0..4 {
                assert_eq!(
                    turns_swap(Turns::new(turns)),
                    o(tag).compose(Turns::new(turns)).swaps_axes() != o(tag).swaps_axes(),
                    "o = {tag}, turns = {turns}"
                );
            }
        }
    }

    #[test]
    fn the_source_map_is_the_inverse_of_the_pixel_map() {
        // The decoder reads each upright pixel from `source_pixel`; each must be where `shown_pixel` puts it.
        let (w, h) = (3, 2);
        for tag in 1..=8 {
            let orientation = o(tag);
            let (ow, oh) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
            for y in 0..oh {
                for x in 0..ow {
                    let (sx, sy) = orientation.source_pixel(x, y, w, h);
                    assert_eq!(orientation.shown_pixel(sx, sy, w, h), (x, y), "o = {tag}, upright ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_edge_map_is_the_pixel_map_on_fractions() {
        // The centre of pixel (x, y) in a w × h picture, as a fraction, is shown where the pixel's centre is shown.
        let (w, h) = (3usize, 2usize);
        for tag in 1..=8 {
            let orientation = o(tag);
            for y in 0..h {
                for x in 0..w {
                    let (u, v) = ((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
                    let (nx, ny) = orientation.shown_pixel(x, y, w, h);
                    let (eu, ev) = orientation.shown_edge(u, v);
                    let (ow, oh) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
                    assert!((eu - (nx as f64 + 0.5) / ow as f64).abs() < 1e-12, "o = {tag}");
                    assert!((ev - (ny as f64 + 0.5) / oh as f64).abs() < 1e-12, "o = {tag}");
                }
            }
        }
    }
}
