//! A worker's thumbnail pipeline: the decode (DCT-scaled, so a thumbnail never decodes more than it needs), the
//! resize and the JPEG encode, with the buffers a worker keeps from file to file. What a thumbnail is made of;
//! where thumbnails are kept is [`crate::thumbs`].

use std::path::Path;

use anyhow::{Context, Result};
use turbojpeg::{Compressor, Image, PixelFormat, Subsamp};

use crate::decode::{Decoded, Decoder, MinSize};
use crate::pixels::{Rgb, release_over};
use crate::resize::Resizer;
use pg_model::format::Format;

/// Lanczos3 from a DCT-scaled decode keeping ≥ 4× the target matches a full decode (FINDINGS "Thumbnails").
const DECODE_MARGIN: usize = 4;
/// libjpeg-turbo q85, 4:2:0 (SSIMULACRA2 ~82; FINDINGS "Thumbnails"). The cache format is still open (PLAN §9.1):
/// JPEG is the fastest to make on demand (~1 ms) and to show (~0.6 ms).
const JPEG_QUALITY: i32 = 85;
/// What a worker keeps of each buffer: a thumbnail decode is DCT-scaled to 4× its height, so this holds a 2048 px
/// high one of a 3:2 photo. A larger image (a PNG, decoded at full size) frees its buffers once it is done.
pub const KEEP_BYTES: usize = 3072 * 2048 * 3;

/// A worker's codecs and buffers for thumbnails, kept from file to file: the import's background pass keeps one
/// per worker, and the app one per image-provider thread.
pub struct Maker {
    decoder: Decoder,
    resizer: Resizer,
    compressor: Compressor,
    /// The decoded image (or a video's frame), the buffer it is oriented through, and its scaled copy.
    image: Rgb,
    spare: Vec<u8>,
    small: Rgb,
    /// libjpeg-turbo's output, at its worst-case size.
    jpeg: Vec<u8>,
    /// The file a request read (the app's: a thumbnail made on a miss, the viewer's decode).
    file: Vec<u8>,
}

impl Maker {
    pub fn new() -> Result<Maker> {
        let mut compressor = Compressor::new()?;
        compressor.set_quality(JPEG_QUALITY)?;
        compressor.set_subsamp(Subsamp::Sub2x2)?;
        Ok(Maker {
            decoder: Decoder::new()?,
            resizer: Resizer::new(),
            compressor,
            image: Rgb::default(),
            spare: Vec::new(),
            small: Rgb::default(),
            jpeg: Vec::new(),
            file: Vec::new(),
        })
    }

    /// The decoder, for a one-off decode on this thread.
    pub fn decoder(&mut self) -> &mut Decoder {
        &mut self.decoder
    }

    /// Read the file at `path` whole into the maker's buffer, as `std::fs::read` reads it (the same errors), for
    /// [`Maker::make_from_file`] or [`Maker::decode_file`]. The buffer is kept for the next file, up to
    /// [`KEEP_BYTES`].
    pub fn read_file(&mut self, path: &Path) -> std::io::Result<()> {
        use std::io::Read;
        self.file.clear();
        let result = std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut self.file)).map(drop);
        if result.is_err() {
            release_over(&mut self.file, KEEP_BYTES);
        }
        result
    }

    /// [`Maker::make`] of the file [`Maker::read_file`] read.
    pub fn make_from_file(&mut self, height: usize, turns: u8) -> Result<Vec<u8>> {
        let file = std::mem::take(&mut self.file);
        let mut out = Vec::new();
        let result = self.make_image(&file, height, turns, &mut out);
        self.file = file;
        self.trim();
        result.map(|()| out)
    }

    /// A one-off decode of the file [`Maker::read_file`] read (the viewer's): its pixels go to new buffers.
    pub fn decode_file(&mut self, min: MinSize, turns: u8) -> Result<Decoded> {
        let result = crate::display::decode(&mut self.decoder, &self.file, min, turns);
        release_over(&mut self.file, KEEP_BYTES);
        result
    }

    /// A thumbnail `height` px high (or the image's own height, if smaller), aspect ratio kept. `turns` is the
    /// file's catalog rotation.
    pub fn make(&mut self, bytes: &[u8], height: usize, turns: u8) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.make_into(bytes, height, turns, &mut out).map(|()| out)
    }

    /// [`Maker::make`] into `out` (replacing what it holds), a buffer the caller reuses: the background pass's,
    /// given back by the cache's writer ([`crate::thumbs::ThumbCache::put_batch`]).
    pub fn make_into(&mut self, bytes: &[u8], height: usize, turns: u8, out: &mut Vec<u8>) -> Result<()> {
        let result = self.make_image(bytes, height, turns, out);
        self.trim();
        result
    }

    fn make_image(&mut self, bytes: &[u8], height: usize, turns: u8, out: &mut Vec<u8>) -> Result<()> {
        let format = Format::sniff(bytes).context("unknown image format")?;
        let min = MinSize::Upright(0, height * DECODE_MARGIN);
        let exif = crate::metadata::orientation(bytes, format);
        self.decoder.decode_into(bytes, format, exif, min, turns, &mut self.image, &mut self.spare)?;
        self.encode_image(height, out)
    }

    /// Decode a video's frame (`video::frame`) into the maker, for [`Maker::encode_frame`].
    pub fn load_frame(&mut self, path: &Path, at_ms: i64, turns: u8) -> Result<()> {
        let result = crate::video::frame_into(path, at_ms, turns, &mut self.image, &mut self.spare);
        if result.is_err() {
            self.trim();
        }
        result
    }

    /// A thumbnail `height` px high (or the frame's own height) of the frame [`Maker::load_frame`] decoded.
    pub fn encode_frame(&mut self, height: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.encode_frame_into(height, &mut out).map(|()| out)
    }

    /// [`Maker::encode_frame`] into `out`, as [`Maker::make_into`].
    pub fn encode_frame_into(&mut self, height: usize, out: &mut Vec<u8>) -> Result<()> {
        let result = self.encode_image(height, out);
        self.trim();
        result
    }

    /// The thumbnail of `image`: scaled to `height`, then compressed into `jpeg`, sized for the worst case, and
    /// copied into `out` at its length.
    fn encode_image(&mut self, height: usize, out: &mut Vec<u8>) -> Result<()> {
        let img = if self.resizer.to_height(&self.image, height, &mut self.small) { &self.small } else { &self.image };
        // Every byte libjpeg-turbo returns is written by it: a grown buffer only zeroes what it grew by.
        self.jpeg.resize(self.compressor.buf_len(img.width, img.height)?, 0);
        let n = self.compressor.compress_to_slice(
            Image {
                pixels: img.pixels.as_slice(),
                width: img.width,
                pitch: img.width * 3,
                height: img.height,
                format: PixelFormat::RGB,
            },
            &mut self.jpeg,
        )?;
        out.clear();
        out.extend_from_slice(&self.jpeg[..n]);
        Ok(())
    }

    /// Free what one oversize image grew (see [`KEEP_BYTES`]).
    fn trim(&mut self) {
        for buf in [&mut self.image.pixels, &mut self.spare, &mut self.small.pixels, &mut self.jpeg, &mut self.file] {
            release_over(buf, KEEP_BYTES);
        }
        self.decoder.trim(KEEP_BYTES);
    }
}
