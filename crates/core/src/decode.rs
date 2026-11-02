//! Decoding to upright 8-bit RGB, the way M0 evaluated it (`eval/common.py::load_rgb`): EXIF orientation applied,
//! transparency flattened onto white, and JPEGs decoded at a reduced scale in the DCT when the caller needs fewer
//! pixels (FINDINGS "Decode benchmark": 2.1× faster than a full decode, nearly the same embedding).
//!
//! The scale is a pure function of the stored dimensions and the caller's minimum size, so the same file always
//! decodes to the same pixels. Index and query images must go through the same decode (FINDINGS: "The rule that
//! matters").

use anyhow::Result;
use turbojpeg::{Decompressor, Image, PixelFormat, ScalingFactor};

use crate::codecs::Codecs;
use crate::format::Format;
use crate::orientation;

/// An upright RGB image.
#[derive(Clone, Default)]
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// What a decode found, besides its pixels.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub format: Format,
    /// Orientation (1–8) that shows the image upright: the EXIF one, combined with the catalog rotation. Applied by
    /// [`Decoder::decode_into`], not by [`Decoder::decode_stored_into`].
    pub orientation: u8,
    /// Dimensions of the full-size image, as the pixels are: upright after an oriented decode, stored otherwise.
    pub full_width: u32,
    pub full_height: u32,
    /// The DCT scale denominator used (1, 2, 4 or 8; always 1 for non-JPEG).
    pub scale_denom: u8,
}

impl Info {
    /// The same, for the image once oriented.
    fn upright(self) -> Info {
        let (full_width, full_height) = if matches!(self.orientation, 5..=8) {
            (self.full_height, self.full_width)
        } else {
            (self.full_width, self.full_height)
        };
        Info { full_width, full_height, ..self }
    }
}

/// An image decoded upright ([`Decoder::decode`]).
pub struct Decoded {
    pub image: Rgb,
    pub info: Info,
}

/// An image as it is stored: decoded, not oriented ([`Decoder::decode_stored`]).
pub struct Stored {
    pub image: Rgb,
    pub info: Info,
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

/// Per-thread decoder state, reused across images: libjpeg-turbo's handle, and the other formats' decoders' buffers
/// ([`Codecs`]). The pixels go into the caller's buffers ([`Decoder::decode_into`]), or into new ones
/// ([`Decoder::decode`]).
pub struct Decoder {
    jpeg: Decompressor,
    codecs: Codecs,
}

impl Decoder {
    pub fn new() -> Result<Decoder> {
        Ok(Decoder { jpeg: Decompressor::new()?, codecs: Codecs::default() })
    }

    /// Free what one oversize file grew: buffers over `bytes` (see [`release_over`]).
    pub fn trim(&mut self, bytes: usize) {
        self.codecs.trim(bytes);
    }

    /// [`Decoder::decode`] into a worker's buffers, which keep their capacity from file to file: the pixels go into
    /// `out`, and are oriented through `spare` (swapped with `out`'s). `format` is the file's (`Format::sniff`),
    /// `exif` its EXIF orientation (`metadata::orientation`, 1 without one).
    #[allow(clippy::too_many_arguments)]
    pub fn decode_into(
        &mut self,
        bytes: &[u8],
        format: Format,
        exif: u8,
        min: MinSize,
        turns: u8,
        out: &mut Rgb,
        spare: &mut Vec<u8>,
    ) -> Result<Info> {
        let info = self.decode_stored_into(bytes, format, exif, min, turns, out)?;
        orient_into(out, info.orientation, spare);
        Ok(info.upright())
    }

    /// [`Decoder::decode_into`] without the orientation: the pixels as stored, which is what a lossless crop of the
    /// file cuts (borders). `min` still means the upright size; `turns` is used for that and for `orientation`.
    pub fn decode_stored_into(
        &mut self,
        bytes: &[u8],
        format: Format,
        exif: u8,
        min: MinSize,
        turns: u8,
        out: &mut Rgb,
    ) -> Result<Info> {
        let orientation = crate::orientation::compose(exif, turns);
        let quarter_turn = matches!(orientation, 5..=8);
        let jpeg = match format {
            Format::Jpeg => self.decode_jpeg(bytes, min, quarter_turn, out).ok(),
            _ => None,
        };
        // Non-JPEG, and JPEGs libjpeg-turbo rejects (CMYK, some damaged files): at full size, as the `image` crate
        // decodes them.
        let (scale_denom, (full_width, full_height)) = match jpeg {
            Some(r) => r,
            None => {
                self.codecs.decode(bytes, format, out)?;
                (1, (out.width as u32, out.height as u32))
            }
        };
        Ok(Info { format, orientation, full_width, full_height, scale_denom })
    }

    /// The stored (not yet oriented) image into `out`; the scale denominator and the full stored size.
    fn decode_jpeg(
        &mut self,
        bytes: &[u8],
        min: MinSize,
        quarter_turn: bool,
        out: &mut Rgb,
    ) -> Result<(u8, (u32, u32))> {
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
        // Every byte is written by the decode: a grown buffer only zeroes what it grew by.
        out.pixels.resize(scaled.width * scaled.height * 3, 0);
        (out.width, out.height) = (scaled.width, scaled.height);
        self.jpeg.decompress(
            bytes,
            Image {
                pixels: out.pixels.as_mut_slice(),
                width: scaled.width,
                pitch: scaled.width * 3,
                height: scaled.height,
                format: PixelFormat::RGB,
            },
        )?;
        Ok((denom as u8, (header.width as u32, header.height as u32)))
    }
}

/// Apply an EXIF orientation, giving the upright image.
pub fn orient(img: Rgb, orientation: u8) -> Rgb {
    let (pixels, width, height) = orient_pixels(img.pixels, img.width, img.height, 3, orientation);
    Rgb { width, height, pixels }
}

/// Apply an EXIF orientation to `img` in place, through `spare` (which gets `img`'s old buffer).
pub fn orient_into(img: &mut Rgb, orientation: u8, spare: &mut Vec<u8>) {
    if !(2..=8).contains(&orientation) {
        return;
    }
    (img.width, img.height) = transform_into(&img.pixels, img.width, img.height, 3, orientation, spare);
    std::mem::swap(&mut img.pixels, spare);
}

/// [`transform`], which leaves `pixels` as they are at orientation 1 (or none).
pub fn orient_pixels(pixels: Vec<u8>, w: usize, h: usize, unit: usize, orientation: u8) -> (Vec<u8>, usize, usize) {
    if !(2..=8).contains(&orientation) {
        return (pixels, w, h);
    }
    transform(&pixels, w, h, unit, orientation)
}

/// Apply an EXIF orientation (1–8) to a packed `w` × `h` raster of `unit`-byte pixels: the raster as it is shown.
/// Returns it with its new width and height.
pub fn transform(pixels: &[u8], w: usize, h: usize, unit: usize, orientation: u8) -> (Vec<u8>, usize, usize) {
    let mut out = Vec::new();
    let (ow, oh) = transform_into(pixels, w, h, unit, orientation, &mut out);
    (out, ow, oh)
}

/// [`transform`] into `out`, whose capacity is kept. Returns the new width and height.
pub fn transform_into(
    pixels: &[u8],
    w: usize,
    h: usize,
    unit: usize,
    orientation: u8,
    out: &mut Vec<u8>,
) -> (usize, usize) {
    let (ow, oh) = if orientation::swaps_axes(orientation) { (h, w) } else { (w, h) };
    // Every pixel is written below: a grown buffer only zeroes what it grew by.
    out.resize(w * h * unit, 0);
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
    (ow, oh)
}

/// Free `buf` if it holds more than `bytes` (after one image far larger than the rest), so a worker's buffers don't
/// keep the largest image's size for the rest of a pass.
pub fn release_over(buf: &mut Vec<u8>, bytes: usize) {
    if buf.capacity() > bytes {
        *buf = Vec::new();
    }
}

/// Test helper: an RGB8 TIFF written by the tiff crate's encoder, classic or BigTIFF (`image` writes only classic).
#[cfg(test)]
pub(crate) fn tiff_rgb8(w: u32, h: u32, pixels: &[u8], big: bool) -> Vec<u8> {
    use tiff::encoder::{TiffEncoder, colortype::RGB8};
    let mut out = std::io::Cursor::new(Vec::new());
    if big {
        TiffEncoder::new_big(&mut out).unwrap().write_image::<RGB8>(w, h, pixels).unwrap();
    } else {
        TiffEncoder::new(&mut out).unwrap().write_image::<RGB8>(w, h, pixels).unwrap();
    }
    out.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_classic_tiff_and_bigtiff() {
        assert_eq!(Format::sniff(b"II*\0\x08\0\0\0"), Some(Format::Tiff));
        assert_eq!(Format::sniff(b"MM\0*\0\0\0\x08"), Some(Format::Tiff));
        assert_eq!(Format::sniff(b"II+\0\x08\0\0\0\x10\0\0\0\0\0\0\0"), Some(Format::Tiff));
        assert_eq!(Format::sniff(b"MM\0+\0\x08\0\0\0\0\0\0\0\x10"), Some(Format::Tiff));
        // A 43 whose offset size isn't 8, or a 43 cut short, isn't a BigTIFF header.
        assert_eq!(Format::sniff(b"II+\0\x04\0\0\0"), None);
        assert_eq!(Format::sniff(b"MM\0+\0\x08\0\x01"), None);
        assert_eq!(Format::sniff(b"II+\0\x08"), None);
    }

    #[test]
    fn transparency_is_flattened_onto_white() {
        let (w, h) = (17, 5);
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 37 % 256) as u8).collect();
        let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba.clone()).unwrap();
        let mut file = std::io::Cursor::new(Vec::new());
        img.write_to(&mut file, image::ImageFormat::Png).unwrap();
        let decoded =
            crate::display::decode(&mut Decoder::new().unwrap(), file.get_ref(), MinSize::Full, 0).unwrap().image;
        let want: Vec<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                let a = p[3] as u32;
                [0, 1, 2].map(|c| ((p[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8)
            })
            .collect();
        assert_eq!((decoded.width, decoded.height), (w, h));
        assert_eq!(decoded.pixels, want);
    }

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
