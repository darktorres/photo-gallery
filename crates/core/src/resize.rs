//! Antialiased Lanczos3 resizing (`fast_image_resize`, SIMD). One frozen filter for every pipeline: the resize is
//! part of the model (PLAN §3.8).

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions};

use crate::pixels::Rgb;

/// A resizer and its buffers, kept by a worker from image to image. Each method writes into a caller's image,
/// whose buffer keeps its capacity.
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
    pub fn fit_within(&mut self, img: &Rgb, side: usize, out: &mut Rgb) -> bool {
        let long = img.width.max(img.height);
        if long <= side {
            return false;
        }
        let scale = side as f64 / long as f64;
        let w = ((img.width as f64 * scale).round() as usize).max(1);
        let h = ((img.height as f64 * scale).round() as usize).max(1);
        self.resize(img, w, h, out);
        true
    }

    /// The image fitted into `width × height` into `out`, keeping its aspect ratio; `false` (and `out` untouched)
    /// if it already fits.
    pub fn fit_box(&mut self, img: &Rgb, width: usize, height: usize, out: &mut Rgb) -> bool {
        if img.width <= width && img.height <= height {
            return false;
        }
        let scale = (width as f64 / img.width as f64).min(height as f64 / img.height as f64);
        let w = ((img.width as f64 * scale).round() as usize).max(1);
        let h = ((img.height as f64 * scale).round() as usize).max(1);
        self.resize(img, w, h, out);
        true
    }

    /// The image scaled to `height` (width from the aspect ratio) into `out`, never enlarged: `false` (and `out`
    /// untouched) if it isn't higher.
    pub fn to_height(&mut self, img: &Rgb, height: usize, out: &mut Rgb) -> bool {
        if img.height <= height {
            return false;
        }
        let w = ((img.width as f64 * height as f64 / img.height as f64).round() as usize).max(1);
        self.resize(img, w, height, out);
        true
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
    ) {
        let (x, y, w, h) = region;
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)).crop(
            x,
            y,
            w.max(1.0),
            h.max(1.0),
        );
        self.run(img, width.max(1), height.max(1), &options, out);
    }

    pub fn resize(&mut self, img: &Rgb, width: usize, height: usize, out: &mut Rgb) {
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
        self.run(img, width, height, &options, out);
    }

    fn run(&mut self, img: &Rgb, width: usize, height: usize, options: &ResizeOptions, out: &mut Rgb) {
        let src = ImageRef::new(img.width as u32, img.height as u32, &img.pixels, PixelType::U8x3)
            .expect("buffer matches its size");
        // Every pixel is written by the resize: a grown buffer only zeroes what it grew by.
        out.pixels.resize(width * height * 3, 0);
        (out.width, out.height) = (width, height);
        let mut dst = Image::from_slice_u8(width as u32, height as u32, &mut out.pixels, PixelType::U8x3)
            .expect("buffer matches its size");
        self.inner.resize(&src, &mut dst, options).expect("same pixel type");
    }
}
