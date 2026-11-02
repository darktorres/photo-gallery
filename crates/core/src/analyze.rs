//! Everything the import computes from one file's bytes (PLAN §4): tiers 0a/0b/1 and what the grid needs to lay
//! out before any thumbnail exists. No thumbnails here: they are a background task after the import (PLAN §6).

use anyhow::{Context, Result};

use crate::borders::{self, Detection, Detector};
use crate::decode::{Decoder, Format, MinSize, Rgb, release_over};
use crate::metadata::{Meta, MetaScratch};
use crate::pdq;
use crate::resize::Resizer;

/// The version of what the import computes, stored with each analysis; files analyzed by an older one are redone,
/// in place. Not a schema version: bump it when a change below alters what is stored, before the first release too,
/// since redoing the analyses keeps the library, ratings and views, where deleting the catalog loses them. A
/// change to the tables instead edits the catalog's baseline in place (`catalog::BASELINE_VERSION` stays 1).
pub const IMPORT_VERSION: i64 = 1;

/// The JPEG decode keeps at least this many pixels on the upright long side…
const DECODE_MIN_LONG: usize = 512;
/// …and on the short side, so the later SSCD input (320 px, PLAN §5 tier 2) can come from the same decode rule.
const DECODE_MIN_SHORT: usize = 320;
/// PDQ input: fitted into 512 px with Lanczos3, as `eval/pdq.py` (Meta's recommendation).
const PDQ_SIDE: usize = 512;
/// ThumbHash input limit (the format's own maximum).
const THUMBHASH_SIDE: usize = 100;

pub struct Analysis {
    /// BLAKE3 of the file bytes (tier 0a).
    pub file_hash: [u8; 32],
    /// The image format (`decode::Format::name`), or a video's codec (`video::Probe::codec`).
    pub format: &'static str,
    /// EXIF orientation (1–8) the file asks for (a video's display matrix).
    pub orientation: u8,
    /// Upright full-size dimensions.
    pub width: u32,
    pub height: u32,
    pub thumbhash: ThumbHash,
    /// The rating the file itself holds (`Meta::rating`), for the catalog's three-way sync.
    pub file_rating: Option<u8>,
    /// Frames in the file (more than one: animated); an image's analysis is of the first.
    pub frames: u32,
    /// When it was taken (`Meta::taken`): wall-clock time, or a UTC instant when `taken_utc` (the catalog
    /// stores it as this computer's wall-clock time).
    pub taken: Option<i64>,
    pub taken_utc: bool,
    /// Tiers 0b and 1, for images. Videos have none (their near-duplicates are later work).
    pub pixels: Option<PixelHashes>,
    /// Videos: length, and where the frame for the ThumbHash and thumbnails is.
    pub duration_ms: Option<i64>,
    pub poster_ms: Option<i64>,
}

/// The longest ThumbHash (vendor/thumbhash `MAX_LEN`: 25 bytes with alpha, 24 opaque).
pub const THUMBHASH_MAX: usize = thumbhash::MAX_LEN;

/// A ThumbHash (`analysis.thumbhash`), held inline: the import's result, and the grid's placeholder.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ThumbHash {
    bytes: [u8; THUMBHASH_MAX],
    len: u8,
}

impl ThumbHash {
    /// `None` for one longer than any ThumbHash, which only a damaged catalog holds.
    pub fn new(hash: &[u8]) -> Option<ThumbHash> {
        let mut bytes = [0; THUMBHASH_MAX];
        bytes.get_mut(..hash.len())?.copy_from_slice(hash);
        Some(ThumbHash { bytes, len: hash.len() as u8 })
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

pub struct PixelHashes {
    /// XXH3-128 of the upright decoded pixels (tier 0b), see [`pixel_hash`].
    pub pixel_hash: [u8; 16],
    pub pdq: pdq::Pdq,
    /// The 8 dihedral PDQ hashes, for the query side of tier 1. Not stored.
    pub pdq_dihedral: [pdq::Hash; 8],
}

pub fn file_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// A worker's buffers for one file's decode, scaled copies and hashes, kept from file to file (a file larger than
/// [`KEEP_BYTES`] frees them after it). Its methods are the import's analysis and the borders pass.
pub struct Analyzer {
    decoder: Decoder,
    resizer: Resizer,
    hasher: pdq::Hasher,
    detector: Detector,
    /// The decoded image (also a video's frame)…
    pub(crate) image: Rgb,
    /// …and the buffer it is oriented through.
    pub(crate) spare: Vec<u8>,
    /// The image fitted for PDQ.
    pdq_in: Rgb,
    /// The ThumbHash input, and the encoder's planes (at most 100 × 100 values each, so never trimmed).
    small: Rgb,
    thumbhasher: thumbhash::Encoder,
    /// What reading each file's metadata works in.
    meta: MetaScratch,
}

/// What a worker keeps of each buffer: a 32 MP RGB image's pixels. A larger file's are freed once it is done.
pub const KEEP_BYTES: usize = 32_000_000 * 3;

impl Analyzer {
    pub fn new() -> Result<Analyzer> {
        Ok(Analyzer {
            decoder: Decoder::new()?,
            resizer: Resizer::new(),
            hasher: pdq::Hasher::default(),
            detector: Detector::default(),
            image: Rgb::default(),
            spare: Vec::new(),
            pdq_in: Rgb::default(),
            small: Rgb::default(),
            thumbhasher: thumbhash::Encoder::default(),
            meta: MetaScratch::default(),
        })
    }

    /// The import's analysis of an image. `turns`: the file's catalog rotation (quarter turns clockwise), part of
    /// what "upright" means.
    pub fn analyze(&mut self, bytes: &[u8], turns: u8) -> Result<Analysis> {
        let result = self.analyze_image(bytes, turns);
        self.trim();
        result
    }

    fn analyze_image(&mut self, bytes: &[u8], turns: u8) -> Result<Analysis> {
        let file_hash = file_hash(bytes);
        let format = Format::sniff(bytes).context("unknown image format")?;
        let meta = Meta::read(bytes, format, &mut self.meta);
        let min = MinSize::Sides(DECODE_MIN_LONG, DECODE_MIN_SHORT);
        let (image, spare) = (&mut self.image, &mut self.spare);
        let info = self.decoder.decode_into(bytes, format, meta.orientation, min, turns, image, spare)?;
        let img = &self.image;

        let pdq_img = if self.resizer.fit_within(img, PDQ_SIDE, &mut self.pdq_in) { &self.pdq_in } else { img };
        let (pdq, pdq_dihedral) = self.hasher.hash_rgb_dihedral(&pdq_img.pixels, pdq_img.width, pdq_img.height);

        Ok(Analysis {
            file_hash,
            format: info.format.name(),
            orientation: info.orientation,
            width: info.full_width,
            height: info.full_height,
            thumbhash: thumbhash(img, &mut self.resizer, &mut self.small, &mut self.thumbhasher),
            file_rating: meta.rating,
            frames: meta.frames,
            taken: meta.taken,
            taken_utc: false,
            pixels: Some(PixelHashes { pixel_hash: pixel_hash(img), pdq, pdq_dihedral }),
            duration_ms: None,
            poster_ms: None,
        })
    }

    /// The ThumbHash of the image in [`Analyzer::image`] (a video's frame).
    pub(crate) fn thumbhash_of_image(&mut self) -> ThumbHash {
        thumbhash(&self.image, &mut self.resizer, &mut self.small, &mut self.thumbhasher)
    }

    /// The borders of a file ([`borders::detect`] on its stored pixels, at full size), looked for after the
    /// import, in a pass of their own. `None` for animated images and images too small to look at.
    pub fn borders(&mut self, bytes: &[u8], turns: u8) -> Result<FileBorders> {
        let result = self.find_borders(bytes, turns);
        self.trim();
        result
    }

    fn find_borders(&mut self, bytes: &[u8], turns: u8) -> Result<FileBorders> {
        let format = Format::sniff(bytes).context("unknown image format")?;
        let meta = Meta::read(bytes, format, &mut self.meta);
        let info =
            self.decoder.decode_stored_into(bytes, format, meta.orientation, MinSize::Full, turns, &mut self.image)?;
        let frames = meta.frames;
        let (width, height) = (info.full_width, info.full_height);
        let borders = (frames <= 1 && (width.min(height) as usize) >= borders::MIN_SIDE)
            .then(|| self.detector.detect(&self.image, &borders::Params::default()));
        Ok(FileBorders { width, height, borders })
    }

    /// Free what one oversize file grew (see [`KEEP_BYTES`]).
    pub(crate) fn trim(&mut self) {
        for buf in [&mut self.image.pixels, &mut self.spare, &mut self.pdq_in.pixels, &mut self.small.pixels] {
            release_over(buf, KEEP_BYTES);
        }
        self.decoder.trim(KEEP_BYTES);
        self.detector.trim(KEEP_BYTES);
    }
}

/// What [`Analyzer::borders`] found.
pub struct FileBorders {
    /// The stored size.
    pub width: u32,
    pub height: u32,
    /// `None`: not looked for (animated, tiny).
    pub borders: Option<Detection>,
}

/// Hash of the upright pixels at the decode scale. The scale depends only on the stored size, so two files with
/// the same image data (re-saved metadata, a different container, lossless re-wraps like progressive ↔ baseline)
/// hash equal. The same pixels in a different codec (JPEG vs PNG) usually decode at a different scale and don't;
/// tier 1 finds those at PDQ distance 0.
pub fn pixel_hash(img: &Rgb) -> [u8; 16] {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    h.update(&(img.width as u32).to_le_bytes());
    h.update(&(img.height as u32).to_le_bytes());
    h.update(&img.pixels);
    h.digest128().to_le_bytes()
}

/// The ThumbHash of `img`, fitted into the format's 100 px through `small`. The encoder reads RGB as it is
/// (vendor/README.md "thumbhash"): the hash of the same pixels as opaque RGBA.
fn thumbhash(img: &Rgb, resizer: &mut Resizer, small: &mut Rgb, encoder: &mut thumbhash::Encoder) -> ThumbHash {
    let small = if resizer.fit_within(img, THUMBHASH_SIDE, small) { &*small } else { img };
    let mut bytes = [0; THUMBHASH_MAX];
    let len = encoder.rgb_to_thumb_hash_into(small.width, small.height, &small.pixels, &mut bytes);
    ThumbHash { bytes, len: len as u8 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thumbs::Maker;

    fn pattern(w: usize, h: usize, seed: usize) -> Vec<u8> {
        (0..w * h * 3).map(|i| ((i / 3 % w) * 7 + (i / 3 / w) * 3 + (i % 3) * 50 + seed * 31) as u8).collect()
    }

    fn jpeg(w: usize, h: usize, seed: usize) -> Vec<u8> {
        let pixels = pattern(w, h, seed);
        let image = turbojpeg::Image {
            pixels: &pixels[..],
            width: w,
            pitch: w * 3,
            height: h,
            format: turbojpeg::PixelFormat::RGB,
        };
        turbojpeg::compress(image, 90, turbojpeg::Subsamp::Sub2x2).unwrap().to_vec()
    }

    /// A PNG with transparency, flattened onto white when decoded.
    fn png_rgba(w: usize, h: usize, seed: usize) -> Vec<u8> {
        let rgb = pattern(w, h, seed);
        let rgba: Vec<u8> =
            rgb.as_chunks::<3>().0.iter().enumerate().flat_map(|(i, p)| [p[0], p[1], p[2], (i * 13) as u8]).collect();
        let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba).unwrap();
        let mut file = std::io::Cursor::new(Vec::new());
        img.write_to(&mut file, image::ImageFormat::Png).unwrap();
        file.into_inner()
    }

    fn summary(a: &Analysis) -> impl PartialEq + std::fmt::Debug + use<> {
        let px = a.pixels.as_ref().unwrap();
        let hashes = (px.pixel_hash, px.pdq.hash, px.pdq.quality, px.pdq_dihedral);
        (a.file_hash, a.format, a.orientation, a.width, a.height, a.thumbhash, a.frames, hashes)
    }

    /// One analyzer and one thumbnail maker for files of every size, format and turn, as a worker keeps them: what a
    /// file leaves in their buffers changes nothing of the next one's results.
    #[test]
    fn kept_buffers_give_what_new_ones_do() {
        let files = [
            (jpeg(1200, 800, 1), 0),
            (png_rgba(300, 200, 2), 1),
            (jpeg(640, 480, 3), 3),
            (jpeg(90, 60, 4), 2),
            (png_rgba(40, 700, 5), 0),
        ];
        let mut kept = Analyzer::new().unwrap();
        let mut maker = Maker::new().unwrap();
        for _ in 0..2 {
            for (bytes, turns) in &files {
                let new = || Analyzer::new().unwrap();
                assert_eq!(
                    summary(&kept.analyze(bytes, *turns).unwrap()),
                    summary(&new().analyze(bytes, *turns).unwrap())
                );
                let (a, b) = (kept.borders(bytes, *turns).unwrap(), new().borders(bytes, *turns).unwrap());
                assert_eq!((a.width, a.height, a.borders), (b.width, b.height, b.borders));
                let thumb = maker.make(bytes, 64, *turns).unwrap();
                assert_eq!(thumb, Maker::new().unwrap().make(bytes, 64, *turns).unwrap());
            }
        }
    }
}
