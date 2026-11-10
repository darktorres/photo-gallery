//! What a rewrite does to the stored pixels: a turn under an EXIF orientation, or a crop.

use anyhow::{Context, Result, ensure};

use pg_codec::decode::transform;
use pg_model::orientation::Orientation;

/// What happens to the stored pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Op {
    /// Turned under an EXIF orientation (1–8).
    Orient(Orientation),
    /// Cut to this rectangle: left, top, width, height.
    Crop([usize; 4]),
}

impl Op {
    /// Width and height swap.
    pub(super) fn quarter(self) -> bool {
        matches!(self, Op::Orient(o) if o.swaps_axes())
    }

    /// The new size of a `w` × `h` raster.
    pub(super) fn size(self, w: usize, h: usize) -> (usize, usize) {
        match self {
            Op::Orient(_) if self.quarter() => (h, w),
            Op::Orient(_) => (w, h),
            Op::Crop([_, _, cw, ch]) => (cw, ch),
        }
    }

    /// The pixels of a `w` × `h` raster of `unit` bytes per pixel, with the new width and height. A crop that
    /// reaches outside the raster's rows is an error.
    pub(super) fn apply(self, pixels: &[u8], w: usize, h: usize, unit: usize) -> Result<(Vec<u8>, usize, usize)> {
        match self {
            Op::Orient(o) => Ok(transform(pixels, w, h, unit, o)),
            Op::Crop([x, y, cw, ch]) => {
                let mut out = Vec::with_capacity(cw * ch * unit);
                // (`max`: a raster no pixel wide has no rows.)
                for row in pixels.chunks_exact((w * unit).max(1)).skip(y).take(ch) {
                    out.extend_from_slice(row.get(x * unit..(x + cw) * unit).context("the crop is outside the image")?);
                }
                ensure!(out.len() == cw * ch * unit, "the crop is outside the image");
                Ok((out, cw, ch))
            }
        }
    }

    /// Where the `w` × `h` frame at (`left`, `top`) of a `sw` × `sh` screen (a GIF's logical screen) goes: its
    /// new place, the part of it kept (left, top, width, height, in the frame), and the new screen size. `None`
    /// when nothing of it is kept.
    ///
    /// A turn keeps the part of the frame on the screen: a frame may stick out past it (real GIFs have them), and
    /// the app's decoder places it into the screen as far as the screen goes, so that part is the picture the user
    /// turned. A frame with no pixels, or wholly off the screen, keeps nothing.
    #[expect(clippy::type_complexity, reason = "the placement and the kept part are returned together")]
    pub(super) fn frame(
        self,
        [left, top, w, h]: [usize; 4],
        sw: usize,
        sh: usize,
    ) -> Option<((usize, usize), [usize; 4], (usize, usize))> {
        match self {
            Op::Orient(o) => {
                // The part on the screen: x0..x1, y0..y1. Empty (a 0-size frame, or one past the screen's edge)
                // keeps nothing, so `x1 - 1` and `y1 - 1` below are pixels of the screen.
                let (x0, y0) = (left, top);
                let (x1, y1) = (left.saturating_add(w).min(sw), top.saturating_add(h).min(sh));
                if x0 >= x1 || y0 >= y1 {
                    return None;
                }
                let (a, b) = (o.shown_pixel(x0, y0, sw, sh), o.shown_pixel(x1 - 1, y1 - 1, sw, sh));
                Some(((a.0.min(b.0), a.1.min(b.1)), [x0 - left, y0 - top, x1 - x0, y1 - y0], self.size(sw, sh)))
            }
            Op::Crop([cx, cy, cw, ch]) => {
                let (x0, y0) = (left.max(cx), top.max(cy));
                let (x1, y1) = ((left + w).min(cx + cw), (top + h).min(cy + ch));
                (x0 < x1 && y0 < y1).then_some(((x0 - cx, y0 - cy), [x0 - left, y0 - top, x1 - x0, y1 - y0], (cw, ch)))
            }
        }
    }
}
