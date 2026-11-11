//! Decoding to upright 8-bit RGB, the way M0 evaluated it (`eval/common.py::load_rgb`): EXIF orientation applied,
//! transparency flattened onto white, and JPEGs decoded at a reduced scale in the DCT when the caller needs fewer
//! pixels (FINDINGS "Decode benchmark": 2.1× faster than a full decode, nearly the same embedding).
//!
//! The scale is a pure function of the stored dimensions and the caller's minimum size, so the same file always
//! decodes to the same pixels. Index and query images must go through the same decode (FINDINGS: "The rule that
//! matters").

use anyhow::{Context, Result};
use pg_model::turns::Turns;
use turbojpeg::{Decompressor, Image, PixelFormat, ScalingFactor};

use crate::codecs::Codecs;
use crate::pixels::Rgb;
use pg_model::format::Format;
use pg_model::orientation::Orientation;

/// What a decode found, besides its pixels.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub format: Format,
    /// Orientation (1–8) that shows the image upright: the EXIF one, combined with the catalog rotation. Applied by
    /// [`Decoder::decode_into`], not by [`Decoder::decode_stored_into`].
    pub orientation: Orientation,
    /// Dimensions of the full-size image, as the pixels are: upright after an oriented decode, stored otherwise.
    pub full_width: u32,
    pub full_height: u32,
    /// The DCT scale denominator used (1, 2, 4 or 8; always 1 for non-JPEG).
    pub scale_denom: u8,
}

impl Info {
    /// The same, for the image once oriented.
    fn upright(self) -> Info {
        let (full_width, full_height) = if self.orientation.swaps_axes() {
            (self.full_height, self.full_width)
        } else {
            (self.full_width, self.full_height)
        };
        Info { full_width, full_height, ..self }
    }
}

/// An image decoded upright (`pg_render::display::decode`, which reads the file's orientation).
pub struct Decoded {
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

impl MinSize {
    /// The scale that fits a `width × height` image into a `box_w × box_h` box, its aspect kept: the smaller of the two
    /// sides' scales (above 1 for an image smaller than the box). The one fit the decode, the resizer and the viewer's
    /// check use.
    pub fn fit_scale(box_w: usize, box_h: usize, width: usize, height: usize) -> f64 {
        let count = pg_model::num::count;
        (count(box_w) / count(width)).min(count(box_h) / count(height))
    }
}

/// Per-thread decoder state, reused across images: libjpeg-turbo's handle, and the other formats' decoders' buffers
/// (`Codecs`). The pixels go into the caller's buffers ([`Decoder::decode_into`], or
/// [`Decoder::decode_stored_into`] for the file as stored).
pub struct Decoder {
    jpeg: Decompressor,
    codecs: Codecs,
}

impl Decoder {
    pub fn new() -> Result<Decoder> {
        Ok(Decoder { jpeg: Decompressor::new()?, codecs: Codecs::default() })
    }

    /// Free what one oversize file grew: buffers over `bytes` (see [`crate::pixels::release_over`]).
    pub fn trim(&mut self, bytes: usize) {
        self.codecs.trim(bytes);
    }

    /// Decode into a worker's buffers, which keep their capacity from file to file: the pixels go into
    /// `out`, and are oriented through `spare` (swapped with `out`'s). `format` is the file's (`Format::sniff`),
    /// `exif` its EXIF orientation (`pg_metadata::orientation`, 1 without one).
    #[expect(clippy::too_many_arguments, reason = "the worker's buffers are passed one by one, so they keep capacity")]
    pub fn decode_into(
        &mut self,
        bytes: &[u8],
        format: Format,
        exif: Orientation,
        min: MinSize,
        turns: Turns,
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
        exif: Orientation,
        min: MinSize,
        turns: Turns,
        out: &mut Rgb,
    ) -> Result<Info> {
        let orientation = exif.compose(turns);
        let quarter_turn = orientation.swaps_axes();
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
                let width = u32::try_from(out.width).context("image width over u32")?;
                (1, (width, u32::try_from(out.height).context("image height over u32")?))
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
                let scale = MinSize::fit_scale(bw, bh, w, h);
                (
                    to_pixels((pg_model::num::count(w) * scale).ceil()),
                    to_pixels((pg_model::num::count(h) * scale).ceil()),
                )
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
        let full = (
            u32::try_from(header.width).context("JPEG width over u32")?,
            u32::try_from(header.height).context("JPEG height over u32")?,
        );
        Ok((u8::try_from(denom).context("scale denominator over u8")?, full))
    }
}

/// `x` rounded toward zero, as a count of pixels.
fn to_pixels(x: f64) -> usize {
    pg_model::num::f64_to_usize(x.clamp(0.0, f64::from(u32::MAX)))
}

/// Apply an EXIF orientation to `img` in place, through `spare` (which gets `img`'s old buffer).
pub fn orient_into(img: &mut Rgb, orientation: Orientation, spare: &mut Vec<u8>) {
    if orientation == Orientation::UPRIGHT {
        return;
    }
    (img.width, img.height) = transform_into(&img.pixels, img.width, img.height, 3, orientation, spare);
    std::mem::swap(&mut img.pixels, spare);
}

/// [`transform`], which leaves `pixels` as they are at orientation 1 (or none).
pub fn orient_pixels(
    pixels: Vec<u8>,
    w: usize,
    h: usize,
    unit: usize,
    orientation: Orientation,
) -> (Vec<u8>, usize, usize) {
    if orientation == Orientation::UPRIGHT {
        return (pixels, w, h);
    }
    transform(&pixels, w, h, unit, orientation)
}

/// Apply an EXIF orientation (1–8) to a packed `w` × `h` raster of `unit`-byte pixels: the raster as it is shown.
/// Returns it with its new width and height.
pub fn transform(pixels: &[u8], w: usize, h: usize, unit: usize, orientation: Orientation) -> (Vec<u8>, usize, usize) {
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
    orientation: Orientation,
    out: &mut Vec<u8>,
) -> (usize, usize) {
    let (ow, oh) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
    // Every pixel is written below: a grown buffer only zeroes what it grew by.
    out.resize(w * h * unit, 0);
    // (An empty side, or a zero `unit`, leaves `out` empty: there are no rows.)
    for (y, row) in out.chunks_exact_mut((ow * unit).max(1)).enumerate() {
        for (x, d) in row.chunks_exact_mut(unit.max(1)).enumerate() {
            let (sx, sy) = orientation.source_pixel(x, y, w, h);
            let s = (sy * w + sx) * unit;
            // A raster shorter than `w` × `h` pixels leaves the pixels it lacks black.
            match pixels.get(s..s + unit) {
                Some(p) => d.copy_from_slice(p),
                None => d.fill(0),
            }
        }
    }
    (ow, oh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::orient;

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
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| u8::try_from(i * 37 % 256).unwrap()).collect();
        let (w32, h32) = (u32::try_from(w).unwrap(), u32::try_from(h).unwrap());
        let img = image::RgbaImage::from_raw(w32, h32, rgba.clone()).unwrap();
        let mut file = std::io::Cursor::new(Vec::new());
        img.write_to(&mut file, image::ImageFormat::Png).unwrap();
        // A PNG without EXIF is shown as stored: orientation 1.
        let mut decoded = Rgb::default();
        Decoder::new()
            .unwrap()
            .decode_into(
                file.get_ref(),
                Format::Png,
                Orientation::UPRIGHT,
                MinSize::Full,
                Turns::NONE,
                &mut decoded,
                &mut Vec::new(),
            )
            .unwrap();
        let want: Vec<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                let a = u32::from(p[3]);
                [0, 1, 2].map(|c| u8::try_from((u32::from(p[c]) * a + 255 * (255 - a) + 127) / 255).unwrap())
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
            let r = orient(img(), Orientation::new(o).unwrap());
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

    #[test]
    fn composition_matches_turning_the_upright_image() {
        let img = || Rgb { width: 3, height: 2, pixels: (0..6u8).flat_map(|i| [i, 0, 0]).collect() };
        for o in 1..=8 {
            // Turning the upright image clockwise is EXIF 6 applied to it.
            let expected = orient(orient(img(), Orientation::new(o).unwrap()), Orientation::new(6).unwrap());
            let got = orient(img(), Orientation::new(o).unwrap().compose(Turns::new(1)));
            assert_eq!(
                (got.width, got.height, &got.pixels),
                (expected.width, expected.height, &expected.pixels),
                "o = {o}"
            );
        }
    }

    #[test]
    fn the_pixel_map_is_where_the_decoder_puts_each_pixel() {
        // A 3×2 picture numbered in reading order: each pixel's value must sit at its shown position in `orient`.
        let (w, h) = (3, 2);
        let img = || Rgb { width: w, height: h, pixels: (0..6u8).flat_map(|i| [i, 0, 0]).collect() };
        for o in 1..=8 {
            let out = orient(img(), Orientation::new(o).unwrap());
            for y in 0..h {
                for x in 0..w {
                    let (nx, ny) = Orientation::new(o).unwrap().shown_pixel(x, y, w, h);
                    let at = (ny * out.width + nx) * 3;
                    assert_eq!(out.pixels[at], u8::try_from(y * w + x).unwrap(), "o = {o}, pixel ({x}, {y})");
                }
            }
        }
    }
}
