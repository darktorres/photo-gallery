//! Decoding to upright 8-bit RGB, the way the evaluation loaded it (`eval/common.py::load_rgb`): EXIF orientation applied,
//! transparency flattened onto white, and JPEGs decoded at a reduced scale in the DCT when the caller needs fewer
//! pixels (2.1× faster than a full decode, nearly the same embedding).
//!
//! The scale is a pure function of the stored dimensions and the caller's minimum size, so the same file always
//! decodes to the same pixels. Index and query images must go through the same decode.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};
use pg_model::error::chain;
use pg_model::turns::Turns;
use turbojpeg::{Decompressor, Image, PixelFormat, ScalingFactor};

use crate::codecs::Codecs;
use crate::codecs::limits::decoded_size;
use crate::pixels::Rgb;
use pg_model::format::Format;
use pg_model::orientation::Orientation;

/// The JPEGs libjpeg-turbo refused, which the decode took by the full-size path instead, since the process started. A
/// camera whose files always trip libjpeg-turbo shows here, where it would otherwise show only as a slow decode.
pub static JPEG_FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// What a decode found, besides its pixels.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    /// The format the file is in.
    pub format: Format,
    /// Orientation (1–8) that shows the image upright: the EXIF one, combined with the catalog rotation. Applied by
    /// [`Decoder::decode_into`], not by [`Decoder::decode_stored_into`].
    pub orientation: Orientation,
    /// Dimensions of the full-size image, as the pixels are: upright after an oriented decode, stored otherwise.
    pub full_width: u32,
    /// The height of the full-size image, as `full_width` is.
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

/// A file's upright size and its orientation, from its header alone: what [`Decoder::decode_into`] reports for it, with
/// no pixels read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dims {
    /// The width, in pixels, as the header states it.
    pub width: u32,
    /// The height, in pixels, as the header states it.
    pub height: u32,
    /// The orientation the header states.
    pub orientation: Orientation,
}

/// An image decoded upright (`pg_render::display::decode`, which reads the file's orientation).
pub struct Decoded {
    /// The pixels, upright.
    pub image: Rgb,
    /// What the decode found out about the file.
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
    /// sides' scales (above 1 for an image smaller than the box). The one scale the decode, the resizer and the viewer's
    /// check use: the decode's least size rounds the fitted sides up (never below the fit), and the resizer rounds them to
    /// the nearest pixel.
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
    /// Threads for a full-size JPEG decode, through parajpeg (0: libjpeg-turbo only).
    full_threads: usize,
    codecs: Codecs,
}

/// What a decode is asked for: the file's format (`Format::sniff`), its EXIF orientation (`pg_metadata::orientation`, 1
/// without one), the least size the pixels may be decoded to ([`MinSize`]) and the catalog's quarter turns.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    /// The file's format (`Format::sniff`).
    pub format: Format,
    /// The file's EXIF orientation (1 without one).
    pub exif: Orientation,
    /// The least size the pixels may be decoded to.
    pub min: MinSize,
    /// The catalog's quarter turns, composed with `exif`.
    pub turns: Turns,
}

impl Decoder {
    /// A decoder that uses the default number of threads.
    pub fn new() -> Result<Decoder> {
        Self::with_full_threads(0)
    }

    /// A decoder whose full-size JPEG decodes run on `threads` threads, through the `parajpeg` crate. Its pixels
    /// are libjpeg-turbo's, so the choice changes no hash; a JPEG parajpeg doesn't decode goes to libjpeg-turbo.
    /// Smaller decodes (the thumbnails') stay on libjpeg-turbo's one thread, which is what the batch work wants.
    pub fn with_full_threads(threads: usize) -> Result<Decoder> {
        Ok(Decoder {
            jpeg: Decompressor::new().map_err(Error::decoder)?,
            full_threads: threads,
            codecs: Codecs::default(),
        })
    }

    /// Free what one oversize file grew: buffers over `bytes` (see [`crate::pixels::release_over`]).
    pub fn trim(&mut self, bytes: usize) {
        self.codecs.trim(bytes);
    }

    /// Decode into a worker's buffers, which keep their capacity from file to file: the pixels go into `out`, and are
    /// oriented through `spare` (swapped with `out`'s). The `request` says what to decode, as [`Request`].
    pub fn decode_into(&mut self, bytes: &[u8], request: Request, out: &mut Rgb, spare: &mut Vec<u8>) -> Result<Info> {
        let info = self.decode_stored_into(bytes, request, out)?;
        orient_in_place(out, info.orientation, spare);
        Ok(info.upright())
    }

    /// [`Decoder::decode_into`] without the orientation: the pixels as stored, which is what a lossless crop of the
    /// file cuts (borders). `request.min` still means the upright size; `request.turns` is used for that and for
    /// `orientation`.
    pub fn decode_stored_into(&mut self, bytes: &[u8], request: Request, out: &mut Rgb) -> Result<Info> {
        let Request { format, exif, min, turns } = request;
        let orientation = exif.compose(turns);
        let quarter_turn = orientation.swaps_axes();
        let (jpeg, refused) = match format {
            Format::Jpeg => match self.decode_jpeg(bytes, min, quarter_turn, out) {
                Ok(decoded) => (Some(decoded), None),
                Err(e) => (None, Some(e)),
            },
            _ => (None, None),
        };
        // Non-JPEG, and JPEGs libjpeg-turbo rejects (CMYK, some damaged files): at full size, as the `image` crate
        // decodes them. Each such fallback is counted, and when it fails too the error says what libjpeg-turbo made of
        // the file.
        let (scale_denom, (full_width, full_height)) = match jpeg {
            Some(r) => r,
            None => {
                if refused.is_some() {
                    JPEG_FALLBACKS.fetch_add(1, Ordering::Relaxed);
                }
                if let Err(e) = self.codecs.decode(bytes, format, out) {
                    return Err(match refused {
                        Some(turbo) => Error::decode(
                            format!("libjpeg-turbo refused it first: {}", chain(&turbo)),
                            Box::<dyn std::error::Error + Send + Sync>::from(e),
                        ),
                        None => Error::decoder(e),
                    });
                }
                let width = u32::try_from(out.width).map_err(|e| Error::decode("image width over u32", e))?;
                (1, (width, u32::try_from(out.height).map_err(|e| Error::decode("image height over u32", e))?))
            }
        };
        Ok(Info { format, orientation, full_width, full_height, scale_denom })
    }

    /// The upright size and orientation of a file, from its header alone (no pixels): the same sizes
    /// [`Decoder::decode_into`] reports. A JPEG takes libjpeg-turbo's header read, and zune-jpeg's where that rejects
    /// it, as the decode does.
    pub fn dimensions(&mut self, bytes: &[u8], format: Format, exif: Orientation, turns: Turns) -> Result<Dims> {
        let orientation = exif.compose(turns);
        let (width, height) = match format {
            Format::Jpeg => match self.jpeg.read_header(bytes) {
                Ok(header) => (
                    u32::try_from(header.width).map_err(|e| Error::decode("JPEG width over u32", e))?,
                    u32::try_from(header.height).map_err(|e| Error::decode("JPEG height over u32", e))?,
                ),
                Err(_) => self.codecs.dimensions(bytes, format).map_err(Error::decoder)?,
            },
            _ => self.codecs.dimensions(bytes, format).map_err(Error::decoder)?,
        };
        let info = Info { format, orientation, full_width: width, full_height: height, scale_denom: 1 }.upright();
        Ok(Dims { width: info.full_width, height: info.full_height, orientation })
    }

    /// The stored (not yet oriented) image into `out`; the scale denominator and the full stored size.
    fn decode_jpeg(
        &mut self,
        bytes: &[u8],
        min: MinSize,
        quarter_turn: bool,
        out: &mut Rgb,
    ) -> Result<(u8, (u32, u32))> {
        // A full decode on several threads (the viewer's decoder only). Whatever parajpeg doesn't decode goes on to
        // libjpeg-turbo below, which then decides.
        if matches!(min, MinSize::Full)
            && self.full_threads > 0
            && let Some(header) = parajpeg::header(bytes)
        {
            out.pixels.resize(decoded_size("the JPEG", &[header.rgb_len()])?, 0);
            if parajpeg::decode_rgb(bytes, self.full_threads, header, &mut out.pixels) {
                (out.width, out.height) = (pg_model::num::index(header.width), pg_model::num::index(header.height));
                return Ok((1, (header.width, header.height)));
            }
        }
        let header = self.jpeg.read_header(bytes).map_err(Error::decoder)?;
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
                    pg_model::num::f64_to_pixels((pg_model::num::count(w) * scale).ceil()),
                    pg_model::num::f64_to_pixels((pg_model::num::count(h) * scale).ceil()),
                )
            }
        };
        // libjpeg-turbo rounds scaled sizes up.
        let denom = [8, 4, 2].into_iter().find(|&d| w.div_ceil(d) >= min_w && h.div_ceil(d) >= min_h).unwrap_or(1);
        self.jpeg.set_scaling_factor(ScalingFactor::new(1, denom)).map_err(Error::decoder)?;
        let scaled = header.scaled(self.jpeg.scaling_factor());
        // Every byte is written by the decode: a grown buffer only zeroes what it grew by. The size is checked against
        // the cap before it is allocated: a damaged header can declare any size.
        out.pixels.resize(decoded_size("the JPEG", &[scaled.width, scaled.height, 3])?, 0);
        (out.width, out.height) = (scaled.width, scaled.height);
        self.jpeg
            .decompress(
                bytes,
                Image {
                    pixels: out.pixels.as_mut_slice(),
                    width: scaled.width,
                    pitch: scaled.width * 3,
                    height: scaled.height,
                    format: PixelFormat::RGB,
                },
            )
            .map_err(Error::decoder)?;
        let full = (
            u32::try_from(header.width).map_err(|e| Error::decode("JPEG width over u32", e))?,
            u32::try_from(header.height).map_err(|e| Error::decode("JPEG height over u32", e))?,
        );
        Ok((u8::try_from(denom).map_err(|e| Error::decode("scale denominator over u8", e))?, full))
    }
}

/// Apply an EXIF orientation to `img` in place, through `spare` (which gets `img`'s old buffer). An upright image is
/// left as it is.
fn orient_in_place(img: &mut Rgb, orientation: Orientation, spare: &mut Vec<u8>) {
    if orientation == Orientation::UPRIGHT {
        return;
    }
    (img.width, img.height) = transform_into(&img.pixels, img.width, img.height, 3, orientation, spare);
    std::mem::swap(&mut img.pixels, spare);
}

/// Pixels per side of a tile of the output [`transform_into`] writes. A quarter turn reads its source down a column of
/// the stored picture, a row of stored pixels apart; a tile reads each of its source rows in a short run, so the lines
/// it reads stay in the cache.
const TILE: usize = 32;

/// Apply an EXIF orientation (1–8) to a packed `w` × `h` raster of `unit`-byte pixels: the raster as it is shown. It is
/// written into `out` (whose capacity is kept), and its new width and height are returned. A raster shorter than
/// `w` × `h` pixels leaves the pixels it lacks black.
pub fn transform_into(
    pixels: &[u8],
    w: usize,
    h: usize,
    unit: usize,
    orientation: Orientation,
    out: &mut Vec<u8>,
) -> (usize, usize) {
    let (ow, oh) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
    // Every pixel is written by `gather`: a grown buffer only zeroes what it grew by.
    out.resize(w * h * unit, 0);
    // The pixel sizes the codecs and the viewer use are constants, so `gather` copies them at a fixed size.
    match unit {
        1 => gather(pixels, w, h, 1, orientation, out),
        3 => gather(pixels, w, h, 3, orientation, out),
        4 => gather(pixels, w, h, 4, orientation, out),
        _ => gather(pixels, w, h, unit, orientation, out),
    }
    (ow, oh)
}

/// [`transform_into`] into a new buffer: the raster as it is shown, with its new width and height.
pub fn transform(pixels: &[u8], w: usize, h: usize, unit: usize, orientation: Orientation) -> (Vec<u8>, usize, usize) {
    let mut out = Vec::new();
    let (ow, oh) = transform_into(pixels, w, h, unit, orientation, &mut out);
    (out, ow, oh)
}

/// The copy of [`transform_into`], tile by tile. The stored pixel an output pixel is copied from is `first + x * step`
/// along its output row, in `usize` with wrapping: [`Orientation::source_pixel`] is affine in the output column, so the
/// first pixel of a row and its step give the whole row, with no match per pixel. Inlined into each `unit` above.
#[inline(always)]
fn gather(pixels: &[u8], w: usize, h: usize, unit: usize, orientation: Orientation, out: &mut [u8]) {
    let (ow, oh) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
    for ty in (0..oh).step_by(TILE) {
        for tx in (0..ow).step_by(TILE) {
            for y in ty..(ty + TILE).min(oh) {
                let first = stored_index(orientation, 0, y, w, h);
                // The second pixel of a row is in the picture only where the row has one (`ow > 1`).
                let step = if ow > 1 { stored_index(orientation, 1, y, w, h).wrapping_sub(first) } else { 0 };
                for x in tx..(tx + TILE).min(ow) {
                    let d = (y * ow + x) * unit;
                    let s = first.wrapping_add(x.wrapping_mul(step)) * unit;
                    if let Some(dst) = out.get_mut(d..d + unit) {
                        match pixels.get(s..s + unit) {
                            Some(p) => dst.copy_from_slice(p),
                            None => dst.fill(0),
                        }
                    }
                }
            }
        }
    }
}

/// The index of the stored pixel [`Orientation::source_pixel`] gives for the upright pixel (`x`, `y`).
fn stored_index(orientation: Orientation, x: usize, y: usize, w: usize, h: usize) -> usize {
    let (sx, sy) = orientation.source_pixel(x, y, w, h);
    sy * w + sx
}

#[cfg(test)]
mod parallel_tests {
    use super::*;
    use crate::encode::{Chroma, JpegEncoder};

    /// A picture with edges, a gradient in each channel and some noise, so every part of the decode has work to do.
    fn picture(width: usize, height: usize) -> Rgb {
        let mut pixels = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let noise = (x * 7919 + y * 104_729) % 61;
                pixels.push(u8::try_from(x * 255 / width + noise).unwrap_or(255));
                pixels.push(u8::try_from(y * 255 / height).unwrap_or(255));
                pixels.push(u8::try_from((x + y) * 255 / (width + height)).unwrap_or(255));
            }
        }
        Rgb { width, height, pixels }
    }

    fn jpeg_of(width: usize, height: usize, chroma: Chroma) -> Vec<u8> {
        JpegEncoder::new(90, chroma).unwrap().encode(&picture(width, height)).unwrap()
    }

    fn decode_full(decoder: &mut Decoder, bytes: &[u8]) -> Result<Rgb> {
        let mut out = Rgb::default();
        decoder.decode_into(
            bytes,
            Request { format: Format::Jpeg, exif: Orientation::UPRIGHT, min: MinSize::Full, turns: Turns::NONE },
            &mut out,
            &mut Vec::new(),
        )?;
        Ok(out)
    }

    #[test]
    fn parajpeg_decodes_the_pixels_libjpeg_turbo_does() {
        for (width, height, chroma) in [(64, 48, Chroma::Half), (101, 37, Chroma::Full), (640, 480, Chroma::Half)] {
            let jpeg = jpeg_of(width, height, chroma);
            let plain = decode_full(&mut Decoder::new().unwrap(), &jpeg).unwrap();
            let header = parajpeg::header(&jpeg).expect("parajpeg reads the header of a JPEG it can decode");
            let mut parallel = vec![0u8; header.rgb_len()];
            assert!(parajpeg::decode_rgb(&jpeg, 3, header, &mut parallel), "{width}x{height}");
            assert!(parallel == plain.pixels, "{width}x{height}: parajpeg's pixels differ from libjpeg-turbo's");
        }
    }

    #[test]
    fn the_parallel_decoder_gives_the_same_image() {
        let jpeg = jpeg_of(640, 480, Chroma::Half);
        let plain = decode_full(&mut Decoder::new().unwrap(), &jpeg).unwrap();
        let parallel = decode_full(&mut Decoder::with_full_threads(6).unwrap(), &jpeg).unwrap();
        assert_eq!((parallel.width, parallel.height), (640, 480));
        assert!(parallel.pixels == plain.pixels);
    }

    #[test]
    fn a_damaged_jpeg_gets_libjpegs_answer() {
        let mut jpeg = jpeg_of(640, 480, Chroma::Half);
        jpeg.truncate(jpeg.len() / 2);
        let plain = decode_full(&mut Decoder::new().unwrap(), &jpeg);
        let parallel = decode_full(&mut Decoder::with_full_threads(6).unwrap(), &jpeg);
        assert_eq!(plain.is_ok(), parallel.is_ok());
        if let (Ok(plain), Ok(parallel)) = (plain, parallel) {
            assert!(parallel.pixels == plain.pixels);
        }
    }

    // A header that declares a picture over the cap is refused before its buffer is allocated: the size is patched into
    // a real JPEG's frame header (40,000 x 40,000 is over 512 MiB at three bytes a pixel).
    #[test]
    fn a_jpeg_declaring_a_picture_over_the_cap_is_refused_before_it_is_allocated() {
        let mut jpeg = jpeg_of(64, 48, Chroma::Half);
        let frame = jpeg.windows(2).position(|w| w == [0xFF, 0xC0]).expect("a baseline frame header");
        jpeg[frame + 5..frame + 7].copy_from_slice(&40_000u16.to_be_bytes());
        jpeg[frame + 7..frame + 9].copy_from_slice(&40_000u16.to_be_bytes());
        let Err(error) = decode_full(&mut Decoder::new().unwrap(), &jpeg) else {
            panic!("a 40,000 x 40,000 picture was decoded");
        };
        let message = format!("{error:#}");
        assert!(message.contains("too large"), "{message}");
    }

    #[test]
    fn a_thumbnail_decode_stays_on_libjpeg_turbo() {
        // Upright (a thumbnail's minimum) is not a full decode: the parallel path is not taken, and the size is the
        // scaled one, which only libjpeg-turbo gives.
        let jpeg = jpeg_of(640, 480, Chroma::Half);
        let mut out = Rgb::default();
        Decoder::with_full_threads(6)
            .unwrap()
            .decode_into(
                &jpeg,
                Request {
                    format: Format::Jpeg,
                    exif: Orientation::UPRIGHT,
                    min: MinSize::Upright(100, 100),
                    turns: Turns::NONE,
                },
                &mut out,
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!((out.width, out.height), (160, 120));
    }

    // Every JPEG under `PG_JPEG_COMPARE_DIR`: parajpeg's full decode against libjpeg-turbo's, pixel by pixel, on the
    // files libjpeg-turbo decodes. Ignored by default, since it reads a folder of photos: `cargo test -p pg-codec --
    // --ignored compares_real_jpegs`.
    #[test]
    #[ignore = "reads a folder of real photos: set PG_JPEG_COMPARE_DIR"]
    fn compares_real_jpegs() {
        use std::path::PathBuf;
        let root = PathBuf::from(
            std::env::var_os(pg_testkit::inputs::JPEG_COMPARE_DIR).expect(pg_testkit::inputs::JPEG_COMPARE_DIR),
        );
        let mut turbo = Decoder::new().unwrap();
        let (mut compared, mut no_header, mut mismatched) = (0usize, 0usize, Vec::new());
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let jpeg = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
                if !jpeg {
                    continue;
                }
                let bytes = std::fs::read(&path).unwrap();
                let Ok(plain) = decode_full(&mut turbo, &bytes) else { continue };
                let Some(header) = parajpeg::header(&bytes) else {
                    no_header += 1;
                    continue;
                };
                let mut out = vec![0u8; header.rgb_len()];
                compared += 1;
                if !(parajpeg::decode_rgb(&bytes, 6, header, &mut out) && out == plain.pixels) {
                    mismatched.push(path);
                }
            }
        }
        eprintln!("compared {compared}, parajpeg declined {no_header}, mismatched {}", mismatched.len());
        assert!(compared > 0, "no JPEG to compare under PG_JPEG_COMPARE_DIR");
        assert!(mismatched.is_empty(), "parajpeg's pixels differ from libjpeg-turbo's: {mismatched:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::orient;

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
                Request { format: Format::Png, exif: Orientation::UPRIGHT, min: MinSize::Full, turns: Turns::NONE },
                &mut decoded,
                &mut Vec::new(),
            )
            .unwrap();
        let want = crate::codecs::test_support::flatten_rgba(&rgba);
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
