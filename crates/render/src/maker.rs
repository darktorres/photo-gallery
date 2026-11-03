//! A worker's thumbnail pipeline: the decode (DCT-scaled, so a thumbnail never decodes more than it needs), the
//! resize and the JPEG encode, with the buffers a worker keeps from file to file. What a thumbnail is made of;
//! where thumbnails are kept is `pg_thumbcache`.

use pg_model::turns::Turns;
use std::path::Path;

use anyhow::{Context, Result};
use turbojpeg::{Compressor, Image, PixelFormat, Subsamp};

use pg_codec::decode::{Decoded, Decoder, MinSize};
use pg_codec::pixels::{Rgb, release_over};
use pg_codec::resize::Resizer;
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
    pub fn make_from_file(&mut self, height: usize, turns: Turns) -> Result<Vec<u8>> {
        let file = std::mem::take(&mut self.file);
        let mut out = Vec::new();
        let result = self.make_image(&file, height, turns, &mut out);
        self.file = file;
        self.trim();
        result.map(|()| out)
    }

    /// A one-off decode of the file [`Maker::read_file`] read (the viewer's): its pixels go to new buffers.
    pub fn decode_file(&mut self, min: MinSize, turns: Turns) -> Result<Decoded> {
        let result = crate::display::decode(&mut self.decoder, &self.file, min, turns);
        release_over(&mut self.file, KEEP_BYTES);
        result
    }

    /// A thumbnail `height` px high (or the image's own height, if smaller), aspect ratio kept. `turns` is the
    /// file's catalog rotation.
    pub fn make(&mut self, bytes: &[u8], height: usize, turns: Turns) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.make_into(bytes, height, turns, &mut out).map(|()| out)
    }

    /// [`Maker::make`] into `out` (replacing what it holds), a buffer the caller reuses: the background pass's,
    /// given back by the cache's writer (`pg_thumbcache::ThumbCache::put_batch`).
    pub fn make_into(&mut self, bytes: &[u8], height: usize, turns: Turns, out: &mut Vec<u8>) -> Result<()> {
        let result = self.make_image(bytes, height, turns, out);
        self.trim();
        result
    }

    fn make_image(&mut self, bytes: &[u8], height: usize, turns: Turns, out: &mut Vec<u8>) -> Result<()> {
        let format = Format::sniff(bytes).context("unknown image format")?;
        let min = MinSize::Upright(0, height * DECODE_MARGIN);
        let exif = pg_metadata::orientation(bytes, format);
        self.decoder.decode_into(bytes, format, exif, min, turns, &mut self.image, &mut self.spare)?;
        self.encode_image(height, out)
    }

    /// Decode a video's frame (`video::frame`) into the maker, for [`Maker::encode_frame`].
    pub fn load_frame(&mut self, path: &Path, at_ms: i64, turns: Turns) -> Result<()> {
        let result = pg_video::frame_into(path, at_ms, turns, &mut self.image, &mut self.spare);
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

#[cfg(test)]
mod tests {
    use super::*;
    use pg_analysis::analyze::Analyzer;
    use pg_model::analysis::Analysis;
    use pg_testkit::test_folder;
    use std::path::PathBuf;

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
        pg_testkit::png_rgba(w as u32, h as u32, &rgba)
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
            (jpeg(1200, 800, 1), Turns::new(0)),
            (png_rgba(300, 200, 2), Turns::new(1)),
            (jpeg(640, 480, 3), Turns::new(3)),
            (jpeg(90, 60, 4), Turns::new(2)),
            (png_rgba(40, 700, 5), Turns::new(0)),
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

    const SCRATCH: &str = r"D:\PhotoGallery\scratch_maker";

    /// Files read into the maker make the thumbnails and decodes their bytes make, file after file (larger, smaller,
    /// missing, not an image), and a missing file fails as `std::fs::read` does.
    #[test]
    fn files_read_into_the_maker_are_their_bytes() {
        let dir = test_folder(SCRATCH, "read-file");
        let files: Vec<(PathBuf, Vec<u8>)> = [(640, 480, 1), (64, 48, 2), (1200, 800, 3), (33, 77, 4)]
            .into_iter()
            .enumerate()
            .map(|(i, (w, h, seed))| (dir.join(format!("{i}.jpg")), jpeg(w, h, seed)))
            .chain([(dir.join("text.jpg"), b"not an image".to_vec())])
            .collect();
        for (path, bytes) in &files {
            std::fs::write(path, bytes).unwrap();
        }
        let missing = dir.join("missing.jpg");
        let mut maker = Maker::new().unwrap();
        for _ in 0..2 {
            for (path, bytes) in &files {
                maker.read_file(path).unwrap();
                let made = maker.make_from_file(120, Turns::new(1)).map_err(|e| format!("{e:#}"));
                let expected = Maker::new().unwrap().make(bytes, 120, Turns::new(1)).map_err(|e| format!("{e:#}"));
                assert_eq!(made, expected, "{}", path.display());
                maker.read_file(path).unwrap();
                let decoded =
                    maker.decode_file(MinSize::Fit(300, 300), Turns::new(3)).map(|d| (d.image.width, d.image.pixels));
                let expected =
                    crate::display::decode(&mut Decoder::new().unwrap(), bytes, MinSize::Fit(300, 300), Turns::new(3));
                let expected = expected.map(|d| (d.image.width, d.image.pixels));
                assert_eq!(decoded.map_err(|e| format!("{e:#}")), expected.map_err(|e| format!("{e:#}")));
            }
            let err = maker.read_file(&missing).unwrap_err();
            assert_eq!(err.to_string(), std::fs::read(&missing).unwrap_err().to_string());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
