//! Resizing 8-bit sRGB with Magic Kernel Sharp 2021 (`kernel`), the one resize for every pipeline: the grid's and the
//! viewer's pixels, PDQ's and ThumbHash's input. The kernel runs on the sRGB bytes as they are, with no conversion to
//! linear light. The resize is part of the model: PDQ and ThumbHash take the bytes it returns.

use crate::error::{Error, Result};
use fast_image_resize::images::{TypedImage, TypedImageRef};
use fast_image_resize::pixels::U8x3;
use fast_image_resize::{ResizeAlg, ResizeOptions};
use pg_model::num::f64_to_pixels;

use crate::kernel::mks2021;
use crate::pixels::Rgb;

/// A resizer, kept by a worker from image to image. Each method writes into a caller's image, whose buffer keeps its
/// capacity. They fail as `fast_image_resize` does: when the image's pixels are fewer than its size needs, or (for
/// [`Resizer::resize_region`]) when the region starts outside the image; a region that runs past its far edge is held
/// to the image.
#[derive(Default)]
pub struct Resizer {
    inner: fast_image_resize::Resizer,
}

impl Resizer {
    /// A resizer with no buffers yet; it allocates them as the first resize needs them.
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
        let scale = crate::decode::MinSize::fit_scale(side, side, img.width, img.height);
        let w = f64_to_pixels((pg_model::num::count(img.width) * scale).round()).max(1);
        let h = f64_to_pixels((pg_model::num::count(img.height) * scale).round()).max(1);
        self.resize(img, w, h, out)?;
        Ok(true)
    }

    /// The image fitted into `width × height` into `out`, keeping its aspect ratio: down when it is larger than the
    /// box, up when it is smaller.
    pub fn fit_box(&mut self, img: &Rgb, width: usize, height: usize, out: &mut Rgb) -> Result<()> {
        let scale = crate::decode::MinSize::fit_scale(width, height, img.width, img.height);
        let w = f64_to_pixels((pg_model::num::count(img.width) * scale).round()).max(1);
        let h = f64_to_pixels((pg_model::num::count(img.height) * scale).round()).max(1);
        self.resize(img, w, h, out)
    }

    /// The image scaled to `height` (width from the aspect ratio) into `out`, never enlarged: `false` (and `out`
    /// untouched) if it isn't higher.
    pub fn to_height(&mut self, img: &Rgb, height: usize, out: &mut Rgb) -> Result<bool> {
        if img.height <= height {
            return Ok(false);
        }
        let w = f64_to_pixels(
            (pg_model::num::count(img.width) * pg_model::num::count(height) / pg_model::num::count(img.height)).round(),
        )
        .max(1);
        self.resize(img, w, height, out)?;
        Ok(true)
    }

    /// The image scaled to `width` (height from the aspect ratio) into `out`, never enlarged: `false` (and `out`
    /// untouched) if it isn't wider.
    pub fn to_width(&mut self, img: &Rgb, width: usize, out: &mut Rgb) -> Result<bool> {
        if img.width <= width {
            return Ok(false);
        }
        let h = f64_to_pixels(
            (pg_model::num::count(img.height) * pg_model::num::count(width) / pg_model::num::count(img.width)).round(),
        )
        .max(1);
        self.resize(img, width, h, out)?;
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
        // The region's far edges are rounded sums, so one can pass the image's edge by a rounding step, and the
        // resizer refuses a crop past the edge: the box is held inside the image.
        let (iw, ih) = (pg_model::num::count(img.width), pg_model::num::count(img.height));
        let (x, y) = (x.min(iw), y.min(ih));
        let (w, h) = (w.max(1.0).min(iw - x), h.max(1.0).min(ih - y));
        let kernel = mks2021().map_err(|e| Error::resize("the MKS 2021 filter", e))?;
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(kernel)).crop(x, y, w, h);
        self.run(img, width.max(1), height.max(1), &options, out)
    }

    /// Resizes `img` to `width` by `height` into `out`, with the MKS 2021 convolution kernel.
    pub fn resize(&mut self, img: &Rgb, width: usize, height: usize, out: &mut Rgb) -> Result<()> {
        let kernel = mks2021().map_err(|e| Error::resize("the MKS 2021 filter", e))?;
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(kernel));
        self.run(img, width, height, &options, out)
    }

    fn run(&mut self, img: &Rgb, width: usize, height: usize, options: &ResizeOptions, out: &mut Rgb) -> Result<()> {
        let (src_w, src_h) = (pixels_u32(img.width)?, pixels_u32(img.height)?);
        let (dst_w, dst_h) = (pixels_u32(width)?, pixels_u32(height)?);
        let overflows = || Error::invalid("image size overflows");
        let n_src = img.width.checked_mul(img.height).ok_or_else(overflows)?;
        let src_len = n_src.checked_mul(3).ok_or_else(overflows)?;
        let src_bytes =
            img.pixels.get(..src_len).ok_or_else(|| Error::invalid("image pixels shorter than its size"))?;
        let n_dst = width.checked_mul(height).ok_or_else(|| Error::invalid("output size overflows"))?;
        let dst_len = n_dst.checked_mul(3).ok_or_else(|| Error::invalid("output size overflows"))?;
        // Every output byte is written by the resize: a grown buffer only zeroes what it grew by.
        out.pixels.resize(dst_len, 0);
        let src = TypedImageRef::<U8x3>::from_buffer(src_w, src_h, src_bytes)
            .map_err(|e| Error::resize("image pixels shorter than its size", e))?;
        {
            let mut dst = TypedImage::<U8x3>::from_buffer(dst_w, dst_h, &mut out.pixels)
                .map_err(|e| Error::resize("resize target buffer", e))?;
            self.inner.resize_typed(&src, &mut dst, options).map_err(|e| Error::resize("resizing", e))?;
        }
        (out.width, out.height) = (width, height);
        Ok(())
    }
}

/// A side in pixels as `fast_image_resize` takes it.
fn pixels_u32(side: usize) -> Result<u32> {
    u32::try_from(side).map_err(|e| Error::resize("image side over u32", e))
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

    #[test]
    fn a_flat_image_stays_flat() {
        let flat = Rgb { width: 40, height: 20, pixels: vec![128; 40 * 20 * 3] };
        let mut out = Rgb::default();
        Resizer::new().resize(&flat, 16, 8, &mut out).unwrap();
        assert!(out.pixels.iter().all(|&v| v.abs_diff(128) <= 1), "a flat grey drifted");
    }

    #[test]
    fn a_black_and_white_pair_averages_the_srgb_bytes() {
        // Two pixels, black and white, averaged to one: the bytes 0 and 255 average to about 128 (the resize is on
        // sRGB bytes, not linear light, where the same pair gives about 188).
        let pair = Rgb { width: 2, height: 1, pixels: vec![0, 0, 0, 255, 255, 255] };
        let mut out = Rgb::default();
        Resizer::new().resize(&pair, 1, 1, &mut out).unwrap();
        let v = out.pixels.first().copied().unwrap_or_default();
        assert!((126..=130).contains(&v), "the average of black and white is {v}, expected about 128");
    }
}
