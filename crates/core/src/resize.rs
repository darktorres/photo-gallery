//! Antialiased Lanczos3 resizing (`fast_image_resize`, SIMD). One frozen filter for every pipeline: the resize is
//! part of the model (PLAN §3.8).

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

use crate::decode::Rgb;

/// The image fitted into `side × side`, keeping its aspect ratio; `None` if it already fits.
pub fn fit_within(img: &Rgb, side: usize) -> Option<Rgb> {
    let long = img.width.max(img.height);
    if long <= side {
        return None;
    }
    let scale = side as f64 / long as f64;
    let w = ((img.width as f64 * scale).round() as usize).max(1);
    let h = ((img.height as f64 * scale).round() as usize).max(1);
    Some(resize(img, w, h))
}

/// The image fitted into `width × height`, keeping its aspect ratio; `None` if it already fits.
pub fn fit_box(img: &Rgb, width: usize, height: usize) -> Option<Rgb> {
    if img.width <= width && img.height <= height {
        return None;
    }
    let scale = (width as f64 / img.width as f64).min(height as f64 / img.height as f64);
    let w = ((img.width as f64 * scale).round() as usize).max(1);
    let h = ((img.height as f64 * scale).round() as usize).max(1);
    Some(resize(img, w, h))
}

/// The image scaled to `height` (width from the aspect ratio), never enlarged.
pub fn to_height(img: &Rgb, height: usize) -> Option<Rgb> {
    if img.height <= height {
        return None;
    }
    let w = ((img.width as f64 * height as f64 / img.height as f64).round() as usize).max(1);
    Some(resize(img, w, height))
}

/// A region of the image (in its pixels; fractional edges are exact) resampled to `width × height`: the viewer's
/// zoom between fit and 1:1 renders only what is on screen.
pub fn resize_region(img: &Rgb, region: (f64, f64, f64, f64), width: usize, height: usize) -> Rgb {
    let src = ImageRef::new(img.width as u32, img.height as u32, &img.pixels, PixelType::U8x3)
        .expect("buffer matches its size");
    let mut dst = Image::new(width.max(1) as u32, height.max(1) as u32, PixelType::U8x3);
    let (x, y, w, h) = region;
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)).crop(
        x,
        y,
        w.max(1.0),
        h.max(1.0),
    );
    Resizer::new().resize(&src, &mut dst, &options).expect("same pixel type");
    Rgb { width: width.max(1), height: height.max(1), pixels: dst.into_vec() }
}

pub fn resize(img: &Rgb, width: usize, height: usize) -> Rgb {
    let src = ImageRef::new(img.width as u32, img.height as u32, &img.pixels, PixelType::U8x3)
        .expect("buffer matches its size");
    let mut dst = Image::new(width as u32, height as u32, PixelType::U8x3);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    Resizer::new().resize(&src, &mut dst, &options).expect("same pixel type");
    Rgb { width, height, pixels: dst.into_vec() }
}
