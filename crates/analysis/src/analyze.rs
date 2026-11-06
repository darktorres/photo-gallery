//! Everything the import computes from one file's bytes (PLAN §4): tiers 0a/0b/1 and what the grid needs to lay
//! out before any thumbnail exists. No thumbnails here: they are a background task after the import (PLAN §6).

use anyhow::{Context, Result};
use pg_model::rating::Rating;
use pg_model::turns::Turns;
use std::fs::File;
use std::io;
use std::path::Path;

use crate::borders::{self, Detector};
use crate::pdq;
use pg_codec::decode::{Decoder, MinSize};
use pg_codec::pixels::{Rgb, release_over};
use pg_codec::resize::Resizer;
use pg_metadata::{Meta, MetaScratch};
use pg_model::analysis::{
    Analysis, Analyzed, ImageFacts, Media, PixelHashes, THUMBHASH_MAX, Taken, ThumbHash, VideoFacts,
};
use pg_model::detection::{self, Detection};
use pg_model::format::Format;
use pg_model::orientation;

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

/// The BLAKE3 of a file's bytes, loaded.
pub fn file_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// [`file_hash`] of a file streamed from `path`, for a video (not read whole): the same digest, the hasher reading the
/// file in chunks.
pub fn file_hash_of(path: &Path) -> io::Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(File::open(path)?)?;
    Ok(*hasher.finalize().as_bytes())
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
    pub fn analyze(&mut self, bytes: &[u8], turns: Turns) -> Result<Analyzed> {
        let result = self.analyze_image(bytes, turns);
        self.trim();
        result
    }

    fn analyze_image(&mut self, bytes: &[u8], turns: Turns) -> Result<Analyzed> {
        let file_hash = file_hash(bytes);
        let format = Format::sniff(bytes).context("unknown image format")?;
        let meta = Meta::read(bytes, format, &mut self.meta);
        let min = MinSize::Sides(DECODE_MIN_LONG, DECODE_MIN_SHORT);
        let (image, spare) = (&mut self.image, &mut self.spare);
        let info = self.decoder.decode_into(bytes, format, meta.orientation, min, turns, image, spare)?;
        let img = &self.image;

        let pdq_img = if self.resizer.fit_within(img, PDQ_SIDE, &mut self.pdq_in)? { &self.pdq_in } else { img };
        let (pdq, pdq_dihedral) = self.hasher.hash_rgb_dihedral(&pdq_img.pixels, pdq_img.width, pdq_img.height);

        let record = Analysis {
            version: IMPORT_VERSION,
            file_hash,
            orientation: info.orientation,
            width: info.full_width,
            height: info.full_height,
            thumbhash: thumbhash(img, &mut self.resizer, &mut self.small, &mut self.thumbhasher)?,
            frames: meta.frames,
            taken: meta.taken.map(Taken::Wall),
            media: Media::Image(ImageFacts {
                format: info.format,
                file_rating: meta.rating.and_then(Rating::new),
                pixels: PixelHashes { pixel_hash: pixel_hash(img)?, pdq },
            }),
        };
        Ok(Analyzed { record, pdq_dihedral: Some(pdq_dihedral) })
    }

    /// The ThumbHash of the image in [`Analyzer::image`] (a video's frame).
    pub(crate) fn thumbhash_of_image(&mut self) -> Result<ThumbHash> {
        thumbhash(&self.image, &mut self.resizer, &mut self.small, &mut self.thumbhasher)
    }

    /// The borders of a file ([`borders::detect`] on its stored pixels, at full size), looked for after the
    /// import, in a pass of their own. `None` for animated images and images too small to look at.
    pub fn borders(&mut self, bytes: &[u8], turns: Turns) -> Result<FileBorders> {
        let result = self.find_borders(bytes, turns);
        self.trim();
        result
    }

    fn find_borders(&mut self, bytes: &[u8], turns: Turns) -> Result<FileBorders> {
        let format = Format::sniff(bytes).context("unknown image format")?;
        let meta = Meta::read(bytes, format, &mut self.meta);
        let info =
            self.decoder.decode_stored_into(bytes, format, meta.orientation, MinSize::Full, turns, &mut self.image)?;
        let frames = meta.frames;
        let (width, height) = (info.full_width, info.full_height);
        let borders = (frames <= 1 && pg_model::num::index(width.min(height)) >= detection::MIN_SIDE)
            .then(|| self.detector.detect(&self.image, &borders::Params::default()))
            .transpose()?;
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
/// tier 1 finds those at PDQ distance 0. An error for a side over `u32::MAX`, which the hash takes as 4 bytes.
pub fn pixel_hash(img: &Rgb) -> Result<[u8; 16]> {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    h.update(&u32::try_from(img.width).context("image width over u32")?.to_le_bytes());
    h.update(&u32::try_from(img.height).context("image height over u32")?.to_le_bytes());
    h.update(&img.pixels);
    Ok(h.digest128().to_le_bytes())
}

/// The ThumbHash of `img`, fitted into the format's 100 px through `small`. The encoder reads RGB as it is
/// (vendor/README.md "thumbhash"): the hash of the same pixels as opaque RGBA.
fn thumbhash(img: &Rgb, resizer: &mut Resizer, small: &mut Rgb, encoder: &mut thumbhash::Encoder) -> Result<ThumbHash> {
    let small = if resizer.fit_within(img, THUMBHASH_SIDE, small)? { &*small } else { img };
    let mut bytes = [0; THUMBHASH_MAX];
    let len = encoder.rgb_to_thumb_hash_into(small.width, small.height, &small.pixels, &mut bytes);
    Ok(ThumbHash::encoded(bytes, len))
}

/// The import's analysis of a video: tier 0a, upright size, duration, codec, recording date, and the ThumbHash
/// of the poster frame (decoded into the worker's buffers). `turns` is the catalog rotation.
pub fn video(path: &Path, turns: Turns, analyzer: &mut Analyzer) -> Result<Analyzed> {
    let result = analyze_video(path, turns, analyzer);
    analyzer.trim();
    result
}

fn analyze_video(path: &Path, turns: Turns, a: &mut Analyzer) -> Result<Analyzed> {
    let file_hash = file_hash_of(path).with_context(|| format!("reading {}", path.display()))?;
    let p = pg_video::probe(path)?;
    let poster = pg_video::poster_ms(p.duration_ms);
    pg_video::frame_into(path, poster, turns, &mut a.image, &mut a.spare)?;
    let (width, height) = if orientation::turns_swap(turns) { (p.height, p.width) } else { (p.width, p.height) };
    let record = Analysis {
        version: IMPORT_VERSION,
        file_hash,
        // The stills' orientation includes the catalog turn (`decode_into`); the video's must too, or its stored
        // size and its shown orientation disagree. `width`/`height` above already carry the display matrix.
        orientation: p.orientation.compose(turns),
        width,
        height,
        thumbhash: a.thumbhash_of_image()?,
        frames: p.frames,
        taken: p.taken,
        media: Media::Video(VideoFacts { codec: p.codec, duration_ms: p.duration_ms, poster_ms: poster }),
    };
    Ok(Analyzed { record, pdq_dihedral: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A streamed file (a video) hashes as its loaded bytes do: one digest for both ways in.
    #[test]
    fn a_streamed_file_hashes_as_its_bytes_do() {
        let dir = pg_testkit::run_folder("scratch_analysis", "analysis").unwrap();
        let bytes: Vec<u8> =
            (0..3_000_000u32).map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24).unwrap()).collect();
        let path = dir.join("streamed.bin");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(file_hash_of(&path).unwrap(), file_hash(&bytes));
        assert!(file_hash_of(&dir.join("missing.bin")).is_err());
    }

    /// The model writes the longest ThumbHash as a literal; it must be the encoder's own length.
    #[test]
    fn thumbhash_max_is_the_encoders_longest() {
        assert_eq!(THUMBHASH_MAX, thumbhash::MAX_LEN);
    }
}
