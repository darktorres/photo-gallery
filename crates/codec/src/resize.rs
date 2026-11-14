//! Resizing in linear light with Magic Kernel Sharp 2021 (`kernel`), the one resize for every pipeline: the grid's and
//! the viewer's pixels, PDQ's and ThumbHash's input. An 8-bit sRGB image is decoded to linear light (`linear`), resized
//! in floating point, and encoded back to 8-bit sRGB once. The resize is part of the model (PLAN §3 rule 8): PDQ and
//! ThumbHash take the sRGB bytes it returns, as their specs do (docs/temp/THUMBNAIL_RESAMPLING.md §7.2).

use anyhow::{Context, Result};
use fast_image_resize::images::{TypedImage, TypedImageRef};
use fast_image_resize::pixels::F32x3;
use fast_image_resize::{ResizeAlg, ResizeOptions};

use crate::kernel::mks2021;
use crate::linear;
use crate::pixels::Rgb;

/// A resizer and its buffers, kept by a worker from image to image. Each method writes into a caller's image,
/// whose buffer keeps its capacity. They fail as `fast_image_resize` does: when the image's pixels are fewer than its
/// size needs, or (for [`Resizer::resize_region`]) when the region is not inside the image.
#[derive(Default)]
pub struct Resizer {
    inner: fast_image_resize::Resizer,
    /// The source in linear light, and the resize's output, kept from image to image.
    src: Vec<F32x3>,
    dst: Vec<F32x3>,
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
        let scale = crate::decode::MinSize::fit_scale(width, height, img.width, img.height);
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
        let options =
            ResizeOptions::new().resize_alg(ResizeAlg::Convolution(mks2021())).crop(x, y, w.max(1.0), h.max(1.0));
        self.run(img, width.max(1), height.max(1), &options, out)
    }

    pub fn resize(&mut self, img: &Rgb, width: usize, height: usize, out: &mut Rgb) -> Result<()> {
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(mks2021()));
        self.run(img, width, height, &options, out)
    }

    fn run(&mut self, img: &Rgb, width: usize, height: usize, options: &ResizeOptions, out: &mut Rgb) -> Result<()> {
        let (src_w, src_h) = (pixels_u32(img.width)?, pixels_u32(img.height)?);
        let (dst_w, dst_h) = (pixels_u32(width)?, pixels_u32(height)?);
        let n_src = img.width.checked_mul(img.height).context("image size overflows")?;
        let src_bytes = img.pixels.get(..n_src.checked_mul(3).context("image size overflows")?);
        let src_bytes = src_bytes.context("image pixels shorter than its size")?;
        decode_into(&mut self.src, src_bytes);
        let n_dst = width.checked_mul(height).context("output size overflows")?;
        self.dst.clear();
        self.dst.resize(n_dst, F32x3::new([0.0; 3]));
        let src = TypedImageRef::new(src_w, src_h, &self.src).context("image pixels shorter than its size")?;
        let mut dst = TypedImage::from_pixels_slice(dst_w, dst_h, &mut self.dst).context("resize target buffer")?;
        self.inner.resize_typed(&src, &mut dst, options).context("resizing")?;
        // Every output byte is written by the encode below: a grown buffer only zeroes what it grew by.
        out.pixels.resize(n_dst.checked_mul(3).context("output size overflows")?, 0);
        (out.width, out.height) = (width, height);
        let (out_px, _) = out.pixels.as_chunks_mut::<3>();
        for (bytes, px) in out_px.iter_mut().zip(&self.dst) {
            let [x, y, z] = px.0;
            let [r, g, b] = bytes;
            (*r, *g, *b) = (linear::encode(x), linear::encode(y), linear::encode(z));
        }
        Ok(())
    }
}

/// The sRGB bytes of an image as linear light, into `src`, which keeps its capacity.
fn decode_into(src: &mut Vec<F32x3>, bytes: &[u8]) {
    src.clear();
    let (pixels, _) = bytes.as_chunks::<3>();
    src.extend(pixels.iter().map(|&[r, g, b]| F32x3::new([linear::decode(r), linear::decode(g), linear::decode(b)])));
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

    #[test]
    fn a_flat_image_stays_flat() {
        let flat = Rgb { width: 40, height: 20, pixels: vec![128; 40 * 20 * 3] };
        let mut out = Rgb::default();
        Resizer::new().resize(&flat, 16, 8, &mut out).unwrap();
        assert!(out.pixels.iter().all(|&v| v.abs_diff(128) <= 1), "a flat grey drifted");
    }

    #[test]
    fn a_black_and_white_pair_averages_in_linear_light() {
        // Two pixels, black and white, averaged to one: linear 0.5 is sRGB 188. Averaging the encoded 0 and 255
        // would give 128, the darker result this resize avoids.
        let pair = Rgb { width: 2, height: 1, pixels: vec![0, 0, 0, 255, 255, 255] };
        let mut out = Rgb::default();
        Resizer::new().resize(&pair, 1, 1, &mut out).unwrap();
        let v = out.pixels.first().copied().unwrap_or_default();
        assert!((186..=190).contains(&v), "the average of black and white is {v}, expected about 188");
    }
}
