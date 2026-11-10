//! One side of the image's content box, read as if it were the top edge.

use pg_codec::pixels::Rgb;
use pg_model::detection::Edge;

/// One side of the image's current content box, seen as if it were the top edge: `line` counts from the edge
/// inwards, `pos` along the edge.
pub(super) struct View<'a> {
    img: &'a Rgb,
    pub(super) edge: Edge,
    box_: [usize; 4],
    /// Lines from the edge to the opposite one.
    pub(super) depth: usize,
    /// The positions of a line read (every other one).
    pub(super) positions: usize,
}

impl<'a> View<'a> {
    pub(super) fn new(img: &'a Rgb, box_: [usize; 4], edge: Edge) -> View<'a> {
        let [l, t, r, b] = box_;
        let (depth, len) = match edge {
            Edge::Top | Edge::Bottom => (b - t, r - l),
            Edge::Left | Edge::Right => (r - l, b - t),
        };
        View { img, edge, box_, depth, positions: len.div_ceil(2) }
    }

    /// A line's read pixels (every other one along it), in order: `positions` of them.
    #[inline]
    pub(super) fn line(&self, line: usize) -> impl Iterator<Item = [i32; 3]> + '_ {
        let [l, t, r, b] = self.box_;
        let w = self.img.width;
        // The pixel index of the line's first read pixel, and the step to the next one (0 only in an image 0 px
        // wide, which has no pixels to read).
        let (first, step) = match self.edge {
            Edge::Top => ((t + line) * w + l, 2),
            Edge::Bottom => ((b - 1 - line) * w + l, 2),
            Edge::Left => (t * w + l + line, 2 * w),
            Edge::Right => (t * w + r - 1 - line, 2 * w),
        };
        let pixels = self.img.pixels.as_chunks::<3>().0;
        pixels.iter().skip(first).step_by(step.max(1)).take(self.positions).map(|&px| px.map(i32::from))
    }

    /// A line's steps between neighbouring read pixels, 3 × |gray step|, in order (`positions - 1` of them).
    pub(super) fn steps3(&self, line: usize) -> impl Iterator<Item = u32> + '_ {
        // A read pixel as the sum of its channels (3 × gray).
        let mut grays = self.line(line).map(|px| px.iter().sum::<i32>());
        let mut prev = grays.next().unwrap_or(0);
        grays.map(move |g| {
            let step = (g - prev).unsigned_abs();
            prev = g;
            step
        })
    }

    /// A line's detail: its steps, summed.
    fn detail3(&self, line: usize) -> i64 {
        self.steps3(line).map(i64::from).sum()
    }

    /// Mean gray step of a line (`borders.py`'s `detail`).
    pub(super) fn detail(&self, line: usize) -> f64 {
        pg_model::num::signed(self.detail3(line)) / (3.0 * pg_model::num::count(self.positions.max(2) - 1))
    }
}
