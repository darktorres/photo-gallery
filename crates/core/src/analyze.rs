//! Everything the import computes from one file's bytes (PLAN §4): tiers 0a/0b/1 and what the grid needs to lay
//! out before any thumbnail exists. No thumbnails here: they are a background task after the import (PLAN §6).

use anyhow::{Context, Result};
use std::path::Path;

use crate::analysis::{Analysis, ImageFacts, Media, PixelHashes, THUMBHASH_MAX, Taken, ThumbHash, VideoFacts};
use crate::borders::{self, Detector};
use crate::decode::{Decoder, MinSize};
use crate::detection::{self, Detection};
use crate::format::Format;
use crate::metadata::{Meta, MetaScratch};
use crate::orientation;
use crate::pdq;
use crate::pixels::{Rgb, release_over};
use crate::resize::Resizer;

/// The version of what the import computes, stored with each analysis; files analyzed by an older one are redone,
/// in place, which keeps the library, ratings and views. Not a schema version. Until the first release it is not
/// bumped: a change that alters what is stored means the owner recreates the catalogs (a change to the tables is
/// made in place in the catalog's baseline, `catalog::BASELINE_VERSION` stays 1). From the first release on, a change
/// that alters what is stored bumps it.
pub const IMPORT_VERSION: i64 = 1;

/// The JPEG decode keeps at least this many pixels on the upright long side…
const DECODE_MIN_LONG: usize = 512;
/// …and on the short side, so the later SSCD input (320 px, PLAN §5 tier 2) can come from the same decode rule.
const DECODE_MIN_SHORT: usize = 320;
/// PDQ input: fitted into 512 px with Lanczos3, as `eval/pdq.py` (Meta's recommendation).
const PDQ_SIDE: usize = 512;
/// ThumbHash input limit (the format's own maximum).
const THUMBHASH_SIDE: usize = 100;

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
            version: IMPORT_VERSION,
            file_hash,
            orientation: info.orientation,
            width: info.full_width,
            height: info.full_height,
            thumbhash: thumbhash(img, &mut self.resizer, &mut self.small, &mut self.thumbhasher),
            frames: meta.frames,
            taken: meta.taken.map(Taken::Wall),
            media: Media::Image(ImageFacts {
                format: info.format,
                file_rating: meta.rating,
                pixels: PixelHashes { pixel_hash: pixel_hash(img), pdq, pdq_dihedral },
            }),
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
        let borders = (frames <= 1 && (width.min(height) as usize) >= detection::MIN_SIDE)
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
    ThumbHash::encoded(bytes, len)
}

/// A video's analysis (the file's hash, its probe, and its poster frame's ThumbHash): what the import stores.
pub fn video(path: &Path, turns: u8, analyzer: &mut Analyzer) -> Result<Analysis> {
    let result = analyze_video(path, turns, analyzer);
    analyzer.trim();
    result
}

fn analyze_video(path: &Path, turns: u8, a: &mut Analyzer) -> Result<Analysis> {
    let file_hash = crate::video::file_hash(path)?;
    let p = crate::video::probe(path)?;
    let poster = crate::video::poster_ms(p.duration_ms);
    crate::video::frame_into(path, poster, turns, &mut a.image, &mut a.spare)?;
    let (width, height) = if orientation::turns_swap(turns) { (p.height, p.width) } else { (p.width, p.height) };
    Ok(Analysis {
        version: IMPORT_VERSION,
        file_hash,
        // The stills' orientation includes the catalog turn (`decode_into`); the video's must too, or its stored
        // size and its shown orientation disagree (R-02). `width`/`height` above already carry the display matrix.
        orientation: orientation::compose(p.orientation, turns),
        width,
        height,
        thumbhash: a.thumbhash_of_image(),
        frames: p.frames,
        taken: p.taken.map(|(t, utc)| if utc { Taken::Utc(t) } else { Taken::Wall(t) }),
        media: Media::Video(VideoFacts { codec: p.codec, duration_ms: p.duration_ms, poster_ms: poster }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maker::Maker;

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
        let px = &a.image().unwrap().pixels;
        let hashes = (px.pixel_hash, px.pdq.hash, px.pdq.quality, px.pdq_dihedral);
        (a.file_hash, a.format(), a.orientation, a.width, a.height, a.thumbhash, a.frames, hashes)
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
