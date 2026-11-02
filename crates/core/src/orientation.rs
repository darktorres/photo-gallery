//! The EXIF orientation (1–8): how a stored picture is shown. Model-shaped: arithmetic over the eight values, with no
//! decoding, metadata or I/O, so the codec, the metadata reader, the video reader, the borders and the catalog can all
//! use it without depending on each other.
//!
//! An orientation is a mirror left-right (applied first), then quarter turns clockwise ([`parts`]). The coordinate maps
//! ([`shown_pixel`], [`shown_edge`]) apply that same mirror and turn to a point of the stored picture: a pixel index, or
//! a fraction of the picture's size. The decoder's transform (`decode::transform_into`) gathers the same mapping, and
//! its test checks the two against each other.

use std::ops::Sub;

/// The orientation's (mirrored, quarter turns clockwise): the upright picture is the stored one mirrored left-right (if
/// so), then turned. Values outside 1–8 read as 1.
pub fn parts(o: u8) -> (bool, u8) {
    match o {
        2 => (true, 0),
        3 => (false, 2),
        4 => (true, 2),
        5 => (true, 3),
        6 => (false, 1),
        7 => (true, 1),
        8 => (false, 3),
        _ => (false, 0),
    }
}

/// The orientation with that mirror and those quarter turns (the turns taken modulo 4).
pub fn from_parts(mirrored: bool, turns: u8) -> u8 {
    match (mirrored, turns % 4) {
        (false, 0) => 1,
        (true, 0) => 2,
        (false, 2) => 3,
        (true, 2) => 4,
        (true, 3) => 5,
        (false, 1) => 6,
        (true, 1) => 7,
        _ => 8,
    }
}

/// The orientation that shows the image `turns` quarter turns further clockwise than `o` does.
pub fn compose(o: u8, turns: u8) -> u8 {
    let (mirrored, t) = parts(o);
    from_parts(mirrored, t + turns)
}

/// Whether the orientation turns the picture a quarter turn (or three): its width and height swap.
pub fn swaps_axes(o: u8) -> bool {
    (5..=8).contains(&o)
}

/// Whether `turns` quarter turns (clockwise) swap the picture's sides: an odd number does. The catalog's turns are
/// counted this way, before they are composed with the file's own orientation (`compose`).
pub fn turns_swap(turns: u8) -> bool {
    turns % 2 == 1
}

/// Where the pixel (`x`, `y`) of a `w` × `h` stored picture is shown.
pub fn shown_pixel(o: u8, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
    shown(o, (x, y), (w, h), 1)
}

/// The stored pixel of a `w` × `h` stored picture that is shown at (`x`, `y`) of the upright picture: the inverse
/// of `shown_pixel`, which the decoder reads its pixels by.
pub fn source_pixel(o: u8, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
    match o {
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
pub fn shown_edge(o: u8, u: f64, v: f64) -> (f64, f64) {
    shown(o, (u, v), (1.0, 1.0), 0.0)
}

/// The one coordinate map. `last` is how far the last index lies below its extent: 1 for pixels (the last pixel of a
/// width `w` is `w - 1`), 0 for fractions of a unit extent.
fn shown<T>(o: u8, (x, y): (T, T), (w, h): (T, T), last: T) -> (T, T)
where
    T: Copy + Sub<Output = T>,
{
    match o {
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
    use crate::decode::orient;
    use crate::pixels::Rgb;

    #[test]
    fn composes_orientations() {
        // Four quarter turns are the identity; one turn of the identity is EXIF 6 (90° clockwise).
        for o in 1..=8 {
            assert_eq!(compose(o, 4), o);
            assert_eq!(compose(o, 0), o);
        }
        assert_eq!(compose(1, 1), 6);
        assert_eq!(compose(6, 1), 3);
        assert_eq!(compose(1, 3), 8);
        assert_eq!(compose(2, 1), 7);
    }

    #[test]
    fn composition_matches_turning_the_upright_image() {
        let img = || Rgb { width: 3, height: 2, pixels: (0..6u8).flat_map(|i| [i, 0, 0]).collect() };
        for o in 1..=8 {
            // Turning the upright image clockwise is EXIF 6 applied to it.
            let expected = orient(orient(img(), o), 6);
            let got = orient(img(), compose(o, 1));
            assert_eq!(
                (got.width, got.height, &got.pixels),
                (expected.width, expected.height, &expected.pixels),
                "o = {o}"
            );
        }
    }

    #[test]
    fn swapping_follows_the_quarter_turns() {
        assert_eq!((1..=8).filter(|&o| swaps_axes(o)).collect::<Vec<_>>(), [5, 6, 7, 8]);
    }

    #[test]
    fn the_catalog_turns_swap_exactly_when_the_composed_orientation_does() {
        // The catalog stores turns on their own (`turns_swap`); the file's own orientation is composed in afterwards
        // (`compose`). Both must agree on whether the sides swap, for every file orientation.
        for o in 1..=8 {
            for turns in 0..4 {
                assert_eq!(
                    turns_swap(turns),
                    swaps_axes(compose(o, turns)) != swaps_axes(o),
                    "o = {o}, turns = {turns}"
                );
            }
        }
    }

    #[test]
    fn the_pixel_map_is_where_the_decoder_puts_each_pixel() {
        // A 3×2 picture numbered in reading order: each pixel's value must sit at its shown position in `orient`.
        let (w, h) = (3, 2);
        let img = || Rgb { width: w, height: h, pixels: (0..6u8).flat_map(|i| [i, 0, 0]).collect() };
        for o in 1..=8 {
            let out = orient(img(), o);
            for y in 0..h {
                for x in 0..w {
                    let (nx, ny) = shown_pixel(o, x, y, w, h);
                    let at = (ny * out.width + nx) * 3;
                    assert_eq!(out.pixels[at], (y * w + x) as u8, "o = {o}, pixel ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_source_map_is_the_inverse_of_the_pixel_map() {
        // The decoder reads each upright pixel from `source_pixel`; each must be where `shown_pixel` puts it.
        let (w, h) = (3, 2);
        for o in 1..=8 {
            let (ow, oh) = if swaps_axes(o) { (h, w) } else { (w, h) };
            for y in 0..oh {
                for x in 0..ow {
                    let (sx, sy) = source_pixel(o, x, y, w, h);
                    assert_eq!(shown_pixel(o, sx, sy, w, h), (x, y), "o = {o}, upright ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_edge_map_is_the_pixel_map_on_fractions() {
        // The centre of pixel (x, y) in a w × h picture, as a fraction, is shown where the pixel's centre is shown.
        let (w, h) = (3usize, 2usize);
        for o in 1..=8 {
            for y in 0..h {
                for x in 0..w {
                    let (u, v) = ((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
                    let (nx, ny) = shown_pixel(o, x, y, w, h);
                    let (eu, ev) = shown_edge(o, u, v);
                    let (ow, oh) = if swaps_axes(o) { (h, w) } else { (w, h) };
                    assert!((eu - (nx as f64 + 0.5) / ow as f64).abs() < 1e-12, "o = {o}");
                    assert!((ev - (ny as f64 + 0.5) / oh as f64).abs() < 1e-12, "o = {o}");
                }
            }
        }
    }
}
