//! Antialiased Lanczos3 resizing (`fast_image_resize`, SIMD). One frozen filter for every pipeline: the resize is
//! part of the model (PLAN §3).

use anyhow::{Context, Result};
use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions};

use crate::pixels::Rgb;

/// A resizer and its buffers, kept by a worker from image to image. Each method writes into a caller's image,
/// whose buffer keeps its capacity. They fail as `fast_image_resize` does: when the image's pixels are fewer than its
/// size needs, or (for [`Resizer::resize_region`]) when the region is not inside the image.
#[derive(Default)]
pub struct Resizer {
    inner: fast_image_resize::Resizer,
}

impl Resizer {
    pub fn new() -> Resizer {
        Resizer::default()
    }

    /// The image fitted into `side × side` into `out`, keeping its aspect ratio; `false` (and `out` untouched) if
    /// it already fits.
    pub fn fit_within(&mut self, img: &Rgb, side: usize, out: &mut Rgb) -> Result<bool> {
        let long = img.width.max(img.height);
        if long <= side {
            return Ok(false);
        }
        let scale = pg_model::num::count(side) / pg_model::num::count(long);
        let w = to_pixels((pg_model::num::count(img.width) * scale).round()).max(1);
        let h = to_pixels((pg_model::num::count(img.height) * scale).round()).max(1);
        self.resize(img, w, h, out)?;
        Ok(true)
    }

    /// The image fitted into `width × height` into `out`, keeping its aspect ratio: down when it is larger than the
    /// box, up when it is smaller.
    pub fn fit_box(&mut self, img: &Rgb, width: usize, height: usize, out: &mut Rgb) -> Result<()> {
        let scale = (pg_model::num::count(width) / pg_model::num::count(img.width))
            .min(pg_model::num::count(height) / pg_model::num::count(img.height));
        let w = to_pixels((pg_model::num::count(img.width) * scale).round()).max(1);
        let h = to_pixels((pg_model::num::count(img.height) * scale).round()).max(1);
        self.resize(img, w, h, out)
    }

    /// The image scaled to `height` (width from the aspect ratio) into `out`, never enlarged: `false` (and `out`
    /// untouched) if it isn't higher.
    pub fn to_height(&mut self, img: &Rgb, height: usize, out: &mut Rgb) -> Result<bool> {
        if img.height <= height {
            return Ok(false);
        }
        let w = to_pixels(
            (pg_model::num::count(img.width) * pg_model::num::count(height) / pg_model::num::count(img.height)).round(),
        )
        .max(1);
        self.resize(img, w, height, out)?;
        Ok(true)
    }

    /// A region of the image (in its pixels; fractional edges are exact) resampled to `width × height` into `out`:
    /// the viewer's zoom between fit and 1:1 renders only what is on screen.
    pub fn resize_region(
        &mut self,
        img: &Rgb,
        region: (f64, f64, f64, f64),
        width: usize,
        height: usize,
        out: &mut Rgb,
    ) -> Result<()> {
        let (x, y, w, h) = region;
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)).crop(
            x,
            y,
            w.max(1.0),
            h.max(1.0),
        );
        self.run(img, width.max(1), height.max(1), &options, out)
    }

    pub fn resize(&mut self, img: &Rgb, width: usize, height: usize, out: &mut Rgb) -> Result<()> {
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
        self.run(img, width, height, &options, out)
    }

    fn run(&mut self, img: &Rgb, width: usize, height: usize, options: &ResizeOptions, out: &mut Rgb) -> Result<()> {
        let (src_w, src_h) = (pixels_u32(img.width)?, pixels_u32(img.height)?);
        let src =
            ImageRef::new(src_w, src_h, &img.pixels, PixelType::U8x3).context("image pixels shorter than its size")?;
        // Every pixel is written by the resize: a grown buffer only zeroes what it grew by.
        out.pixels.resize(width * height * 3, 0);
        (out.width, out.height) = (width, height);
        let (dst_w, dst_h) = (pixels_u32(width)?, pixels_u32(height)?);
        let mut dst =
            Image::from_slice_u8(dst_w, dst_h, &mut out.pixels, PixelType::U8x3).context("resize target buffer")?;
        self.inner.resize(&src, &mut dst, options).context("resizing")?;
        Ok(())
    }
}

/// A side in pixels as `fast_image_resize` takes it.
fn pixels_u32(side: usize) -> Result<u32> {
    u32::try_from(side).context("image side over u32")
}

/// `x` rounded toward zero, as a count of pixels.
fn to_pixels(x: f64) -> usize {
    pg_model::num::f64_to_usize(x.clamp(0.0, f64::from(u32::MAX)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_box_enlarges_a_small_image_and_shrinks_a_large_one() {
        let small = Rgb { width: 4, height: 2, pixels: vec![128; 4 * 2 * 3] };
        let mut out = Rgb::default();
        Resizer::new().fit_box(&small, 16, 9, &mut out).unwrap();
        assert_eq!((out.width, out.height), (16, 8));
        assert_eq!(out.pixels.len(), 16 * 8 * 3);

        let large = Rgb { width: 40, height: 20, pixels: vec![128; 40 * 20 * 3] };
        Resizer::new().fit_box(&large, 16, 9, &mut out).unwrap();
        assert_eq!((out.width, out.height), (16, 8));
    }
}
