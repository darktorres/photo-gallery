//! Everything the import computes from one file's bytes (PLAN §4): tiers 0a/0b/1 and what the grid needs to lay
//! out before any thumbnail exists. No thumbnails here: they are a background task after the import (PLAN §6).

use anyhow::Result;

use crate::decode::{Decoder, MinSize, Rgb};
use crate::pdq;
use crate::resize::fit_within;

/// Bump when anything below changes what is stored; files analyzed by an older version are redone.
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
    pub format: String,
    /// EXIF orientation (1–8) the file asks for (a video's display matrix).
    pub orientation: u8,
    /// Upright full-size dimensions.
    pub width: u32,
    pub height: u32,
    pub thumbhash: Vec<u8>,
    /// The rating the file itself holds (metadata::read_rating), for the catalog's three-way sync.
    pub file_rating: Option<u8>,
    /// Frames in the file (more than one: animated); an image's analysis is of the first.
    pub frames: u32,
    /// When it was taken (metadata::read_taken): wall-clock time, or a UTC instant when `taken_utc` (the catalog
    /// stores it as this computer's wall-clock time).
    pub taken: Option<i64>,
    pub taken_utc: bool,
    /// Tiers 0b and 1, for images. Videos have none (their near-duplicates are later work).
    pub pixels: Option<PixelHashes>,
    /// Videos: length, and where the frame for the ThumbHash and thumbnails is.
    pub duration_ms: Option<i64>,
    pub poster_ms: Option<i64>,
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

/// `turns`: the file's catalog rotation (quarter turns clockwise), part of what "upright" means.
pub fn analyze(bytes: &[u8], turns: u8, decoder: &mut Decoder) -> Result<Analysis> {
    let file_hash = file_hash(bytes);
    let decoded = decoder.decode(bytes, MinSize::Sides(DECODE_MIN_LONG, DECODE_MIN_SHORT), turns)?;
    let img = &decoded.image;

    let pdq_in = fit_within(img, PDQ_SIDE);
    let pdq_img = pdq_in.as_ref().unwrap_or(img);
    let (pdq, pdq_dihedral) = pdq::hash_rgb_dihedral(&pdq_img.pixels, pdq_img.width, pdq_img.height);

    Ok(Analysis {
        file_hash,
        format: decoded.format.name().to_string(),
        orientation: decoded.orientation,
        width: decoded.full_width,
        height: decoded.full_height,
        thumbhash: thumbhash(img),
        file_rating: crate::metadata::read_rating(bytes, decoded.format),
        frames: crate::metadata::frame_count(bytes, decoded.format),
        taken: crate::metadata::read_taken(bytes, decoded.format),
        taken_utc: false,
        pixels: Some(PixelHashes { pixel_hash: pixel_hash(img), pdq, pdq_dihedral }),
        duration_ms: None,
        poster_ms: None,
    })
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

pub(crate) fn thumbhash(img: &Rgb) -> Vec<u8> {
    let small = fit_within(img, THUMBHASH_SIDE);
    let small = small.as_ref().unwrap_or(img);
    let rgba: Vec<u8> = small.pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    thumbhash::rgba_to_thumb_hash(small.width, small.height, &rgba)
}
