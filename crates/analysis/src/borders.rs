//! Borders around images (FINDINGS "Border cleaning"): solid ones (black, white, colored, JPEG-noisy) and blurred
//! padding bars (a blurred, enlarged copy of the image filling the letterbox). A port of the Python prototype that
//! measured them (`tools/borders.py`, removed once this matched it file for file; it is not in this repo), run on the stored
//! pixels at full size (no EXIF orientation: a crop of the stored image keeps the tag
//! valid), by a pass of its own after the import (`pg_borders`).
//!
//! Per side (top, bottom, left, right; repeated so nested borders peel off):
//! - A line is *solid* if nearly all its pixels lie within `tol` of the line's median colour, and that colour changes
//!   only gradually from line to line (absorbs JPEG noise and ringing).
//! - A line is *blurred* if its detail (mean gradient) is far below the image's central detail.
//! - The border is the run of such lines from the edge; the seam is the first strong line-to-line change near its end.
//! - Accepted only if the seam is a straight edge across most of the line (a dark sky or a bokeh background ends
//!   irregularly), and, for a blurred bar, if the content past the seam is clearly sharper.
//! - Then the rules that need the opposite side (`judge`): solid padding is digitally flat, and blurred bars come
//!   in equal pairs.
//!
//! The arithmetic is exact where Python's was float32: grays are kept as sums of the three channels (×3), medians of
//! 8-bit values doubled (×2), deviations from a median of medians ×4. Line statistics read every other pixel.

mod lines;
mod params;
mod seam;
mod side;
mod stats;
mod view;

use anyhow::{Context, Result};
use pg_codec::pixels::Rgb;
use pg_model::detection::{Detection, Edge, MAX_SIDES, Reason, Side, Sides};

pub use params::{BORDERS_VERSION, Params};
use side::{detect_side, judge};
use stats::median_f64;
use view::View;

/// Find the borders of a stored image, once. A pass over many images keeps a [`Detector`] instead. An error for an
/// image with a side over `u32::MAX`, which a [`Detection`] can't hold.
pub fn detect(img: &Rgb, p: &Params) -> Result<Detection> {
    Detector::default().detect(img, p)
}

/// The border detector's buffers, kept from image to image: each grows to what the largest image so far needed.
#[derive(Default)]
pub struct Detector {
    /// What a side's detection uses.
    sides: side::Scratch,
    /// [`blur_threshold`]'s line details.
    details: Vec<f64>,
}

impl Detector {
    /// Find the borders of a stored image (an error as [`detect`]'s).
    pub fn detect(&mut self, img: &Rgb, p: &Params) -> Result<Detection> {
        detect_with(self, img, p)
    }

    /// The bytes its buffers hold.
    pub fn capacity_bytes(&self) -> usize {
        self.sides.capacity_bytes() + self.details.capacity() * size_of::<f64>()
    }

    /// Frees its buffers if they hold more than `bytes` (after an image far larger than the rest).
    pub fn trim(&mut self, bytes: usize) {
        if self.capacity_bytes() > bytes {
            *self = Detector::default();
        }
    }
}

/// The rounds of peeling ([`MAX_SIDES`] counts them).
const PASSES: usize = 3;
const _: () = assert!(MAX_SIDES == PASSES * 4);

fn detect_with(d: &mut Detector, img: &Rgb, p: &Params) -> Result<Detection> {
    let (w, h) = (img.width, img.height);
    let mut content = [0, 0, w, h];
    let mut sides = Sides::new();
    // Pixels short of `width` × `height`: no borders are looked for (every read below is inside the image).
    if w.checked_mul(h).is_none_or(|n| img.pixels.len() / 3 < n) {
        return detection(w, h, content, sides);
    }
    let blur_thr = blur_threshold(img, p, &mut d.details);
    // Nested borders, e.g. a thin line inside a blurred bar. Each round finds at most one side of each edge, so
    // `sides` never holds more than `MAX_SIDES`.
    for pass in 0..PASSES {
        let mut changed = false;
        // Top/bottom first, then left/right on what is left, so corners never mix two borders.
        for pair in [[Edge::Top, Edge::Bottom], [Edge::Left, Edge::Right]] {
            let found = pair.map(|edge| {
                detect_side(&mut d.sides, &View::new(img, content, edge), p, blur_thr).map(|mut s| {
                    s.pass = u8::try_from(pass).unwrap_or(u8::MAX);
                    s
                })
            });
            // The opposite side's accepted detection, captured before either is judged, so the outcome doesn't
            // depend on which is judged first.
            let detected = found.clone().map(|s| s.filter(Side::accepted));
            let mut found = found;
            let [first, second] = &detected;
            for (side, other) in found.iter_mut().zip([second, first]) {
                if let Some(side) = side.as_mut().filter(|s| s.accepted()) {
                    judge(side, other.as_ref(), p);
                }
            }
            for side in found.into_iter().flatten() {
                if !side.accepted() {
                    if !sides.iter().any(|s| !s.accepted() && s.edge == side.edge && s.depth == side.depth) {
                        sides.push(side);
                    }
                    continue;
                }
                let depth = pg_model::num::index(side.depth);
                match side.edge {
                    Edge::Top => content[1] += depth,
                    Edge::Bottom => content[3] -= depth,
                    Edge::Left => content[0] += depth,
                    Edge::Right => content[2] -= depth,
                }
                sides.push(side);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let [l, t, r, b] = content;
    let any = sides.iter().any(Side::accepted);
    if any
        && (pg_model::num::count(r - l) < p.min_keep * pg_model::num::count(w)
            || pg_model::num::count(b - t) < p.min_keep * pg_model::num::count(h))
    {
        for s in sides.iter_mut().filter(|s| s.accepted()) {
            s.rejected = Some(Reason::TooLittleLeft);
        }
        content = [0, 0, w, h];
    }
    detection(w, h, content, sides)
}

/// The [`Detection`] of an image `w` × `h` with the content box `content` (inside the image); an error for a side
/// over `u32::MAX`.
fn detection(w: usize, h: usize, content: [usize; 4], sides: Sides) -> Result<Detection> {
    let width = u32::try_from(w).context("image width over u32")?;
    let height = u32::try_from(h).context("image height over u32")?;
    let [l, t, r, b] = content.map(u32::try_from);
    Ok(Detection { width, height, content: [l?, t?, r?, b?], sides })
}

/// The detail below which a line is blurred: a fraction of the image's central detail. `None` for images that are
/// smooth overall, where blurred bars aren't looked for.
fn blur_threshold(img: &Rgb, p: &Params, details: &mut Vec<f64>) -> Option<f64> {
    let (w, h) = (img.width, img.height);
    // Every 4th line of the middle third, every other pixel of the middle three quarters.
    let (x0, x1) = (w / 8, 7 * w / 8);
    let xs = (x0..x1).step_by(2);
    let n = xs.len();
    details.clear();
    if n > 1 {
        let pixels = img.pixels.as_chunks::<3>().0;
        details.extend((h / 3..2 * h / 3).step_by(4).filter_map(|y| {
            // The line's pixels at `xs`, each as the sum of its channels.
            let line = pixels.get(y * w + x0..y * w + x1)?;
            let mut sums = line.iter().step_by(2).map(|&[r, g, b]| i32::from(r) + i32::from(g) + i32::from(b));
            let mut prev = sums.next()?;
            let total: i64 = sums
                .map(|g| {
                    let step = i64::from((g - prev).abs());
                    prev = g;
                    step
                })
                .sum();
            Some(pg_model::num::signed(total) / (3.0 * pg_model::num::count(n - 1)))
        }));
    }
    let content_act = median_f64(details).unwrap_or(0.0);
    (content_act >= p.min_content_act).then(|| p.blur_abs.max(p.blur_rel * content_act))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::detection::{Class, Kind};

    /// Detailed content: deterministic noise in 6 px blocks, a little per pixel, over a gradient (a hash, so the
    /// tests need no crate).
    fn content(w: usize, h: usize, seed: u32) -> Rgb {
        let hash = |a: usize, b: usize, c: usize| {
            let [a, b, c] = [a, b, c].map(|v| u32::try_from(v).unwrap());
            let mut v = seed ^ a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B) ^ c << 28;
            v ^= v >> 15;
            v = v.wrapping_mul(0x2C1B_3C6D);
            v ^= v >> 12;
            v = v.wrapping_mul(0x297A_2D39);
            pg_model::num::index(v ^ (v >> 15))
        };
        let mut pixels = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    let block = hash(x / 6, y / 6, c) % 110;
                    let fine = hash(x + 7919, y, c) % 16;
                    pixels.push(u8::try_from(40 + block + fine + (x + y + 30 * c) % 60).unwrap());
                }
            }
        }
        Rgb { width: w, height: h, pixels }
    }

    fn set(img: &mut Rgb, x: usize, y: usize, px: [u8; 3]) {
        let i = (y * img.width + x) * 3;
        img.pixels[i..i + 3].copy_from_slice(&px);
    }

    fn fill(img: &mut Rgb, x0: usize, y0: usize, x1: usize, y1: usize, px: impl Fn(usize, usize) -> [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                set(img, x, y, px(x, y));
            }
        }
    }

    fn accepted(d: &Detection) -> Vec<(Edge, Kind, u32)> {
        d.sides.iter().filter(|s| s.accepted()).map(|s| (s.edge, s.kind, s.depth)).collect()
    }

    fn rejected(d: &Detection) -> Vec<(Edge, Reason)> {
        d.sides.iter().filter_map(|s| s.rejected.map(|r| (s.edge, r))).collect()
    }

    #[test]
    fn black_letterbox() {
        let (w, h) = (400, 300);
        let mut img = content(w, h, 1);
        fill(&mut img, 0, 0, w, 40, |_, _| [0, 0, 0]);
        fill(&mut img, 0, h - 40, w, h, |_, _| [2, 1, 0]);
        let d = detect(&img, &Params::default()).unwrap();
        // The seam is at 40, plus one pixel of margin.
        assert_eq!(accepted(&d), vec![(Edge::Top, Kind::Solid, 41), (Edge::Bottom, Kind::Solid, 41)]);
        assert_eq!(d.content, [0, 41, 400, 259]);
        assert_eq!(d.sides[0].class(), Class::Black);
    }

    #[test]
    fn white_side_padding() {
        let (w, h) = (500, 300);
        let mut img = content(w, h, 2);
        fill(&mut img, 0, 0, 90, h, |_, _| [255, 255, 255]);
        fill(&mut img, w - 90, 0, w, h, |_, _| [253, 254, 255]);
        let d = detect(&img, &Params::default()).unwrap();
        assert_eq!(accepted(&d), vec![(Edge::Left, Kind::Solid, 91), (Edge::Right, Kind::Solid, 91)]);
        assert_eq!(d.sides[0].class(), Class::White);
    }

    #[test]
    fn thin_dark_frame() {
        let (w, h) = (320, 240);
        let mut img = content(w, h, 3);
        let frame = |x: usize, y: usize| x < 5 || y < 5 || x >= w - 5 || y >= h - 5;
        fill(&mut img, 0, 0, w, h, |x, y| if frame(x, y) { [20, 18, 22] } else { [0; 3] });
        let inner = content(w, h, 3);
        for y in 5..h - 5 {
            for x in 5..w - 5 {
                let i = (y * w + x) * 3;
                let px = [inner.pixels[i], inner.pixels[i + 1], inner.pixels[i + 2]];
                set(&mut img, x, y, px);
            }
        }
        let d = detect(&img, &Params::default()).unwrap();
        let found = accepted(&d);
        assert_eq!(found.len(), 4, "{:?}", d.sides);
        assert!(found.iter().all(|&(_, k, depth)| k == Kind::Solid && (5..=7).contains(&depth)), "{found:?}");
    }

    /// A smooth bar: colour varying slowly along the line, so it is no solid line, and almost no detail.
    fn bar(x: usize, y: usize) -> [u8; 3] {
        let t = (pg_model::num::count(x) / 120.0).sin() * 60.0 + (pg_model::num::count(y) / 50.0).cos() * 10.0;
        [channel(110.0 + t), channel(100.0 + t * 0.8), channel(90.0 + t * 0.5)]
    }

    /// `x` rounded toward zero, as a channel value.
    fn channel(x: f64) -> u8 {
        pg_model::num::f64_to_u8(x.clamp(0.0, 255.0))
    }

    #[test]
    fn blurred_bars_in_pairs() {
        let (w, h) = (300, 520);
        let mut img = content(w, h, 4);
        fill(&mut img, 0, 0, w, 110, bar);
        fill(&mut img, 0, h - 110, w, h, bar);
        let d = detect(&img, &Params::default()).unwrap();
        let found = accepted(&d);
        assert_eq!(found.len(), 2, "{:?}", d.sides);
        // The seam is found where the detail below the row jumps, which a few lines of content already make; the
        // exact depth is pinned by the parity check against borders.py (FINDINGS), not here.
        assert!(found.iter().all(|&(_, k, depth)| k == Kind::Blur && (100..=113).contains(&depth)), "{found:?}");
        assert_eq!(found[0].2, found[1].2);
    }

    #[test]
    fn one_blurred_bar_is_a_sky() {
        let (w, h) = (300, 520);
        let mut img = content(w, h, 5);
        fill(&mut img, 0, 0, w, 110, bar);
        let d = detect(&img, &Params::default()).unwrap();
        assert!(!d.found(), "{:?}", d.sides);
        assert_eq!(rejected(&d), vec![(Edge::Top, Reason::UnpairedBlur)]);
        assert_eq!(d.content, [0, 0, u32::try_from(w).unwrap(), u32::try_from(h).unwrap()]);
    }

    #[test]
    fn shaded_wall_is_not_flat() {
        let (w, h) = (400, 300);
        let mut img = content(w, h, 6);
        // Each line uniform, the colour drifting down: a wall lit from above.
        fill(&mut img, 0, 0, w, 70, |_, y| [100 + y, 110 + y, 120 + y].map(|v| u8::try_from(v).unwrap()));
        let d = detect(&img, &Params::default()).unwrap();
        assert!(!d.found(), "{:?}", d.sides);
        assert_eq!(rejected(&d), vec![(Edge::Top, Reason::NotFlat)]);
    }

    #[test]
    fn too_deep() {
        let (w, h) = (400, 300);
        let mut img = content(w, h, 7);
        fill(&mut img, 0, 0, w, 150, |_, _| [0, 0, 0]);
        let d = detect(&img, &Params::default()).unwrap();
        assert!(!d.found());
        assert_eq!(rejected(&d), vec![(Edge::Top, Reason::TooDeep)]);
    }

    #[test]
    fn no_border() {
        let img = content(400, 300, 8);
        let d = detect(&img, &Params::default()).unwrap();
        assert!(d.sides.is_empty(), "{:?}", d.sides);
    }


    #[test]
    fn nested_borders_peel_off() {
        // Black letterbox bars, and inside them a white frame around the content. Above and below, the run of
        // border lines bridges the colour change (as borders.py's run_length does): one side each.
        let (w, h) = (400, 400);
        let mut img = content(w, h, 9);
        fill(&mut img, 0, 0, w, h, |x, y| {
            if y < 30 || y >= h - 30 {
                [0, 0, 0]
            } else if y < 36 || y >= h - 36 || x < 6 || x >= w - 6 {
                [250, 250, 250]
            } else {
                [0; 3]
            }
        });
        let inner = content(w, h, 9);
        for y in 36..h - 36 {
            for x in 6..w - 6 {
                let i = (y * w + x) * 3;
                set(&mut img, x, y, [inner.pixels[i], inner.pixels[i + 1], inner.pixels[i + 2]]);
            }
        }
        let d = detect(&img, &Params::default()).unwrap();
        let [l, t, r, b] = d.content;
        let (w, h) = (u32::try_from(w).unwrap(), u32::try_from(h).unwrap());
        assert!((6..=8).contains(&l) && (w - 8..=w - 6).contains(&r), "{:?} {:?}", d.content, d.sides);
        assert!((36..=39).contains(&t) && (h - 39..=h - 36).contains(&b), "{:?} {:?}", d.content, d.sides);
    }

    #[test]
    fn too_little_content_left() {
        // Letterbox bars of 36% each: each within the 45% a side may cut, together leaving 28% of the height.
        let (w, h) = (400, 400);
        let mut img = content(w, h, 10);
        fill(&mut img, 0, 0, w, 144, |_, _| [0, 0, 0]);
        fill(&mut img, 0, h - 144, w, h, |_, _| [0, 0, 0]);
        let d = detect(&img, &Params::default()).unwrap();
        assert!(!d.found(), "{:?}", d.sides);
        assert!(d.sides.iter().any(|s| s.rejected == Some(Reason::TooLittleLeft)));
        assert_eq!(d.content, [0, 0, u32::try_from(w).unwrap(), u32::try_from(h).unwrap()]);
    }

    #[test]
    fn a_kept_detector_finds_what_a_new_one_does() {
        // Images of different sizes and borders, in an order where each leaves larger buffers than the next needs.
        let mut images = Vec::new();
        let mut letterbox = content(400, 300, 11);
        fill(&mut letterbox, 0, 0, 400, 40, |_, _| [0, 0, 0]);
        fill(&mut letterbox, 0, 260, 400, 300, |_, _| [0, 0, 0]);
        let mut bars = content(300, 520, 12);
        fill(&mut bars, 0, 0, 300, 110, bar);
        fill(&mut bars, 0, 410, 300, 520, bar);
        let mut padded = content(500, 300, 13);
        fill(&mut padded, 0, 0, 90, 300, |_, _| [255, 255, 255]);
        let mut small = content(120, 90, 14);
        fill(&mut small, 0, 0, 120, 12, |_, _| [30, 60, 90]);
        images.extend([bars, letterbox, content(400, 300, 15), padded, small]);
        let mut kept = Detector::default();
        for _ in 0..2 {
            for img in &images {
                assert_eq!(kept.detect(img, &Params::default()).unwrap(), detect(img, &Params::default()).unwrap());
            }
        }
        assert!(kept.capacity_bytes() > 0);
        kept.trim(0);
        assert_eq!(kept.capacity_bytes(), 0);
    }
}
