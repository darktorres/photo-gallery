//! Decoding to upright 8-bit RGB, the way M0 evaluated it (`eval/common.py::load_rgb`): EXIF orientation applied,
//! transparency flattened onto white, and JPEGs decoded at a reduced scale in the DCT when the caller needs fewer
//! pixels (FINDINGS "Decode benchmark": 2.1× faster than a full decode, nearly the same embedding).
//!
//! The scale is a pure function of the stored dimensions and the caller's minimum size, so the same file always
//! decodes to the same pixels. Index and query images must go through the same decode (FINDINGS: "The rule that
//! matters").

use anyhow::{Context, Result};
use turbojpeg::{Decompressor, Image, PixelFormat, ScalingFactor};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Jpeg,
    Png,
    Gif,
    Webp,
    Bmp,
    Tiff,
}

impl Format {
    pub fn sniff(bytes: &[u8]) -> Option<Format> {
        Some(match bytes {
            [0xFF, 0xD8, 0xFF, ..] => Format::Jpeg,
            [0x89, b'P', b'N', b'G', ..] => Format::Png,
            [b'G', b'I', b'F', b'8', ..] => Format::Gif,
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Format::Webp,
            [b'B', b'M', ..] => Format::Bmp,
            [b'I', b'I', 0x2A, 0x00, ..] | [b'M', b'M', 0x00, 0x2A, ..] => Format::Tiff,
            _ => return None,
        })
    }

    /// The inverse of [`Format::name`].
    pub fn from_name(name: &str) -> Option<Format> {
        [Format::Jpeg, Format::Png, Format::Gif, Format::Webp, Format::Bmp, Format::Tiff]
            .into_iter()
            .find(|f| f.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Jpeg => "jpeg",
            Format::Png => "png",
            Format::Gif => "gif",
            Format::Webp => "webp",
            Format::Bmp => "bmp",
            Format::Tiff => "tiff",
        }
    }
}

/// An upright RGB image.
#[derive(Clone)]
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub struct Decoded {
    pub image: Rgb,
    pub format: Format,
    /// Orientation (1–8) that was applied: the EXIF one, combined with the catalog rotation.
    pub orientation: u8,
    /// Upright dimensions of the full-size image.
    pub full_width: u32,
    pub full_height: u32,
    /// The DCT scale denominator used (1, 2, 4 or 8; always 1 for non-JPEG).
    pub scale_denom: u8,
}

/// The least a JPEG may be DCT-scaled down to.
#[derive(Clone, Copy, Debug)]
pub enum MinSize {
    /// Full size.
    Full,
    /// Upright width × height.
    Upright(usize, usize),
    /// Long side × short side, whatever the orientation.
    Sides(usize, usize),
    /// At least the size the image has when fitted into this upright box (aspect kept).
    Fit(usize, usize),
}

/// Per-thread decoder state (libjpeg-turbo handles are reused across images).
pub struct Decoder {
    jpeg: Decompressor,
}

impl Decoder {
    pub fn new() -> Result<Decoder> {
        Ok(Decoder { jpeg: Decompressor::new()? })
    }

    /// Decode at the smallest DCT scale that still keeps `min` (full size if the image is smaller). `turns` is
    /// the catalog rotation (`files.rotation`, quarter turns clockwise), applied after the EXIF orientation.
    pub fn decode(&mut self, bytes: &[u8], min: MinSize, turns: u8) -> Result<Decoded> {
        let format = Format::sniff(bytes).context("unknown image format")?;
        let orientation = crate::metadata::compose_orientation(exif_orientation(bytes), turns);
        let quarter_turn = matches!(orientation, 5..=8);

        let jpeg = match format {
            Format::Jpeg => self.decode_jpeg(bytes, min, quarter_turn).ok(),
            _ => None,
        };
        // Non-JPEG, and JPEGs libjpeg-turbo rejects (CMYK, some damaged files): the `image` crate, at full size.
        let (stored, scale_denom, (full_w, full_h)) = match jpeg {
            Some(r) => r,
            None => {
                let rgb = decode_other(bytes, format)?;
                let size = (rgb.width as u32, rgb.height as u32);
                (rgb, 1, size)
            }
        };
        let image = orient(stored, orientation);
        let (full_width, full_height) = if quarter_turn { (full_h, full_w) } else { (full_w, full_h) };
        Ok(Decoded { image, format, orientation, full_width, full_height, scale_denom })
    }

    /// The stored (not yet oriented) image, the scale denominator and the full stored size.
    fn decode_jpeg(&mut self, bytes: &[u8], min: MinSize, quarter_turn: bool) -> Result<(Rgb, u8, (u32, u32))> {
        let header = self.jpeg.read_header(bytes)?;
        let (w, h) = (header.width, header.height);
        // The decode happens before rotation, so an upright minimum turns with the image.
        let (min_w, min_h) = match min {
            MinSize::Full => (usize::MAX, usize::MAX),
            MinSize::Upright(mw, mh) if quarter_turn => (mh, mw),
            MinSize::Upright(mw, mh) => (mw, mh),
            MinSize::Sides(long, short) if w >= h => (long, short),
            MinSize::Sides(long, short) => (short, long),
            MinSize::Fit(bw, bh) => {
                let (bw, bh) = if quarter_turn { (bh, bw) } else { (bw, bh) };
                let scale = (bw as f64 / w as f64).min(bh as f64 / h as f64);
                ((w as f64 * scale).ceil() as usize, (h as f64 * scale).ceil() as usize)
            }
        };
        // libjpeg-turbo rounds scaled sizes up.
        let denom = [8, 4, 2].into_iter().find(|&d| w.div_ceil(d) >= min_w && h.div_ceil(d) >= min_h).unwrap_or(1);
        self.jpeg.set_scaling_factor(ScalingFactor::new(1, denom))?;
        let scaled = header.scaled(self.jpeg.scaling_factor());
        let mut pixels = vec![0u8; scaled.width * scaled.height * 3];
        self.jpeg.decompress(
            bytes,
            Image {
                pixels: pixels.as_mut_slice(),
                width: scaled.width,
                pitch: scaled.width * 3,
                height: scaled.height,
                format: PixelFormat::RGB,
            },
        )?;
        let rgb = Rgb { width: scaled.width, height: scaled.height, pixels };
        Ok((rgb, denom as u8, (header.width as u32, header.height as u32)))
    }
}

fn decode_other(bytes: &[u8], format: Format) -> Result<Rgb> {
    let fmt = match format {
        Format::Jpeg => image::ImageFormat::Jpeg,
        Format::Png => image::ImageFormat::Png,
        Format::Gif => image::ImageFormat::Gif,
        Format::Webp => image::ImageFormat::WebP,
        Format::Bmp => image::ImageFormat::Bmp,
        Format::Tiff => image::ImageFormat::Tiff,
    };
    let img = image::load_from_memory_with_format(bytes, fmt)?;
    let (width, height) = (img.width() as usize, img.height() as usize);
    let pixels = if img.color().has_alpha() {
        // Flatten onto white, like the evaluation.
        img.to_rgba8()
            .pixels()
            .flat_map(|p| {
                let a = p[3] as u32;
                [0, 1, 2].map(|c| ((p[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8)
            })
            .collect()
    } else {
        img.to_rgb8().into_raw()
    };
    Ok(Rgb { width, height, pixels })
}

/// EXIF orientation 1–8; 1 when absent or unreadable.
pub fn exif_orientation(bytes: &[u8]) -> u8 {
    let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(bytes)) else {
        return 1;
    };
    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .filter(|o| (1..=8).contains(o))
        .map_or(1, |o| o as u8)
}

/// Apply an EXIF orientation, giving the upright image.
pub fn orient(img: Rgb, orientation: u8) -> Rgb {
    if !(2..=8).contains(&orientation) {
        return img;
    }
    let (pixels, width, height) = transform(&img.pixels, img.width, img.height, 3, orientation);
    Rgb { width, height, pixels }
}

/// Apply an EXIF orientation (1–8) to a packed `w` × `h` raster of `unit`-byte pixels: the raster as it is shown.
/// Returns it with its new width and height.
pub fn transform(pixels: &[u8], w: usize, h: usize, unit: usize, orientation: u8) -> (Vec<u8>, usize, usize) {
    let (ow, oh) = if orientation >= 5 { (h, w) } else { (w, h) };
    let mut out = vec![0u8; w * h * unit];
    for y in 0..oh {
        for x in 0..ow {
            // Source pixel for output (x, y).
            let (sx, sy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                8 => (w - 1 - y, x),
                _ => (x, y),
            };
            let (s, d) = ((sy * w + sx) * unit, (y * ow + x) * unit);
            out[d..d + unit].copy_from_slice(&pixels[s..s + unit]);
        }
    }
    (out, ow, oh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientations_map_corners() {
        // 3×2 image whose pixels are numbered 0..6 in reading order.
        let img = || Rgb { width: 3, height: 2, pixels: (0..6u8).flat_map(|i| [i, 0, 0]).collect() };
        let first = |o| {
            let r = orient(img(), o);
            (r.width, r.height, r.pixels.chunks(3).map(|p| p[0]).collect::<Vec<_>>())
        };
        assert_eq!(first(1), (3, 2, vec![0, 1, 2, 3, 4, 5]));
        assert_eq!(first(2), (3, 2, vec![2, 1, 0, 5, 4, 3]));
        assert_eq!(first(3), (3, 2, vec![5, 4, 3, 2, 1, 0]));
        assert_eq!(first(4), (3, 2, vec![3, 4, 5, 0, 1, 2]));
        assert_eq!(first(5), (2, 3, vec![0, 3, 1, 4, 2, 5]));
        assert_eq!(first(6), (2, 3, vec![3, 0, 4, 1, 5, 2])); // 90° clockwise
        assert_eq!(first(7), (2, 3, vec![5, 2, 4, 1, 3, 0]));
        assert_eq!(first(8), (2, 3, vec![2, 5, 1, 4, 0, 3])); // 90° counter-clockwise
    }
}
