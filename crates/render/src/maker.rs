//! A worker's thumbnail and placeholder pipeline: the decode (DCT-scaled, so a thumbnail never decodes more than it
//! needs), the resize and the JPEG encode, and the ThumbHash placeholder of a picture, with the buffers a worker keeps
//! from file to file. What a thumbnail and a placeholder are made of; where thumbnails are kept is `pg_thumbcache`.

use pg_model::analysis::{THUMBHASH_MAX, ThumbHash};
use pg_model::image::View;
use pg_model::turns::Turns;
use std::path::Path;

use anyhow::Result;

use pg_codec::decode::{Decoder, MinSize, transform_into};
use pg_codec::encode::{Chroma, JpegEncoder};
use pg_codec::pixels::{Rgb, release_over};
use pg_codec::resize::Resizer;
use pg_model::orientation::Orientation;

use crate::display::{self, DECODE_MARGIN, Shown};

// The steps a worker's picture takes, each a tally of its thread time, over every worker (process-wide: a pass that runs
// them attaches the tallies it caused to its span). The decode is a thumbnail's and a placeholder's alike; the resize and
// the encode are a thumbnail's, the resize and the hash a placeholder's.
pub static DECODE: pg_perf::Tally = pg_perf::Tally::new("render.decode");
pub static RESIZE: pg_perf::Tally = pg_perf::Tally::new("render.resize");
pub static ENCODE: pg_perf::Tally = pg_perf::Tally::new("render.encode");
pub static THUMBHASH: pg_perf::Tally = pg_perf::Tally::new("render.thumbhash");

/// The image a placeholder is encoded from is fitted into this many px on its long side (the encoder's blur is at its
/// 100 px).
pub const THUMBHASH_SIDE: usize = 100;
/// The decode a placeholder of a file takes keeps at least this many px on the long side: the fit to [`THUMBHASH_SIDE`]
/// is then a resize of a picture at least twice its size, not of a scaled-down one.
pub const PLACEHOLDER_DECODE_SIDE: usize = 200;

/// libjpeg-turbo q85, 4:2:0 (SSIMULACRA2 ~82). The cache format is still open:
/// JPEG is the fastest to make on demand (~1 ms) and to show (~0.6 ms).
const JPEG_QUALITY: i32 = 85;
/// What a worker keeps of each buffer: a thumbnail decode is DCT-scaled to 4× its height, so this holds a 2048 px
/// high one of a 3:2 photo. A larger image (a PNG, decoded at full size) frees its buffers once it is done.
pub const KEEP_BYTES: usize = 3072 * 2048 * 3;

/// Why an image wasn't made of a file, by what to do about it: tried again later, or not until the file changes.
#[derive(Debug, thiserror::Error)]
pub enum MakeError {
    /// The file couldn't be read: an I/O failure (an offline drive, a file in use, a file gone), which the next try may
    /// not have. Only a video's open is read here; an image's bytes are read before ([`Maker::make_of_file`]).
    #[error(transparent)]
    Unreadable(std::io::Error),
    /// The file was read and doesn't decode (damaged, truncated, a format the decoder can't take), or the encoder failed:
    /// it fails again until the file changes, or the decoder does ([`crate::THUMBS_VERSION`]).
    #[error(transparent)]
    Undecodable(anyhow::Error),
}

impl MakeError {
    /// A video's frame error: [`MakeError::Unreadable`] when its file couldn't be opened ([`pg_video::Unopened`]).
    fn of_video(e: anyhow::Error) -> MakeError {
        match e.downcast::<pg_video::Unopened>() {
            Ok(unopened) => MakeError::Unreadable(unopened.0),
            Err(e) => MakeError::Undecodable(e),
        }
    }
}

/// A worker's codecs and buffers for thumbnails, kept from file to file: the import's background pass keeps one
/// per worker, and the app one per image-provider thread.
pub struct Maker {
    decoder: Decoder,
    resizer: Resizer,
    encoder: JpegEncoder,
    /// The decoded image (or a video's frame), the buffer it is oriented through, and its scaled copy.
    image: Rgb,
    spare: Vec<u8>,
    small: Rgb,
    /// The file a request read (the app's: a thumbnail made on a miss, the viewer's decode).
    file: Vec<u8>,
    thumbhasher: thumbhash::Encoder,
}

/// Threads for the viewer's full-size decodes (`Maker::decode_of_file`), through parajpeg. Six
/// of the i7-8700's twelve: on a 7.5 MP camera JPEG a decode takes 30 ms against libjpeg-turbo's 57 ms (19 ms with all
/// twelve), at about twice the CPU time, which leaves the import's workers some of the cores while a photo is shown.
pub const VIEWER_THREADS: usize = 6;

impl Maker {
    pub fn new() -> Result<Maker> {
        Ok(Maker {
            decoder: Decoder::with_full_threads(VIEWER_THREADS)?,
            resizer: Resizer::new(),
            encoder: JpegEncoder::new(JPEG_QUALITY, Chroma::Half)?,
            image: Rgb::default(),
            spare: Vec::new(),
            small: Rgb::default(),
            file: Vec::new(),
            thumbhasher: thumbhash::Encoder::default(),
        })
    }

    /// The decoder, for a one-off decode on this thread.
    pub fn decoder(&mut self) -> &mut Decoder {
        &mut self.decoder
    }

    /// The resizer, for the viewer's resizes on this thread: its coefficients and buffers are kept from one resize to the
    /// next.
    pub fn resizer(&mut self) -> &mut Resizer {
        &mut self.resizer
    }

    /// A thumbnail `height` px high of the file at `path`, turned by `turns` (its catalog rotation). The file is read
    /// and made in this one call, so the buffer holds this file's bytes for all of it. A file that can't be read is
    /// [`MakeError::Unreadable`].
    pub fn make_of_file(&mut self, path: &Path, height: usize, turns: Turns) -> Result<Vec<u8>, MakeError> {
        self.read_file(path).map_err(MakeError::Unreadable)?;
        self.make_from_file(height, turns)
    }

    /// A one-off decode of the file at `path` for the viewer's `view` ([`display::min_size`]), read and decoded in this
    /// one call as [`Maker::make_of_file`] reads it. Its pixels go to new buffers.
    pub fn decode_of_file(&mut self, path: &Path, view: View, turns: Turns) -> Result<Shown, MakeError> {
        self.read_file(path).map_err(MakeError::Unreadable)?;
        self.decode_file(view, turns)
    }

    /// Read the file at `path` whole into the maker's buffer, as `std::fs::read` reads it (the same errors), for
    /// [`Maker::make_from_file`] or [`Maker::decode_file`]. The buffer is kept for the next file, up to
    /// [`KEEP_BYTES`].
    fn read_file(&mut self, path: &Path) -> std::io::Result<()> {
        use std::io::Read;
        self.file.clear();
        let result = std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut self.file)).map(drop);
        if result.is_err() {
            release_over(&mut self.file, KEEP_BYTES);
        }
        result
    }

    /// [`Maker::make`] of the file [`Maker::read_file`] read.
    fn make_from_file(&mut self, height: usize, turns: Turns) -> Result<Vec<u8>, MakeError> {
        let file = std::mem::take(&mut self.file);
        let mut out = Vec::new();
        let result = self.make_image(&file, height, turns, &mut out);
        self.file = file;
        self.trim();
        result.map(|()| out).map_err(MakeError::Undecodable)
    }

    /// A one-off decode of the file [`Maker::read_file`] read, for the viewer's `view` ([`display::min_size`]): its
    /// pixels go to new buffers.
    fn decode_file(&mut self, view: View, turns: Turns) -> Result<Shown, MakeError> {
        let result = display::decode(&mut self.decoder, &self.file, display::min_size(view), turns);
        release_over(&mut self.file, KEEP_BYTES);
        result.map(Shown::from).map_err(MakeError::Undecodable)
    }

    /// A thumbnail `height` px high (or the image's own height, if smaller), aspect ratio kept. `turns` is the
    /// file's catalog rotation.
    pub fn make(&mut self, bytes: &[u8], height: usize, turns: Turns) -> Result<Vec<u8>, MakeError> {
        let mut out = Vec::new();
        self.make_into(bytes, height, turns, &mut out).map(|()| out)
    }

    /// [`Maker::make`] into `out` (replacing what it holds), a buffer the caller reuses: the background pass's,
    /// given back by the cache's writer (`pg_thumbcache::ThumbCache::put_batch`).
    pub fn make_into(&mut self, bytes: &[u8], height: usize, turns: Turns, out: &mut Vec<u8>) -> Result<(), MakeError> {
        let result = self.make_image(bytes, height, turns, out);
        self.trim();
        result.map_err(MakeError::Undecodable)
    }

    fn make_image(&mut self, bytes: &[u8], height: usize, turns: Turns, out: &mut Vec<u8>) -> Result<()> {
        let min = MinSize::Upright(0, height * DECODE_MARGIN);
        // The decode is left as stored: the thumbnail's scale comes before its turn, so the turn touches the thumbnail
        // and not the decoded image, which is several times its size.
        let orientation = {
            let _decode = DECODE.timer();
            display::decode_stored_into(&mut self.decoder, bytes, min, turns, &mut self.image)?.orientation
        };
        self.encode_turned(orientation, height, out)
    }

    /// The placeholder (ThumbHash) of the image `bytes`, upright as `turns` makes it: a DCT-scaled decode keeping
    /// [`PLACEHOLDER_DECODE_SIDE`] px on the long side, fitted to [`THUMBHASH_SIDE`] and encoded.
    pub fn placeholder(&mut self, bytes: &[u8], turns: Turns) -> Result<ThumbHash, MakeError> {
        let min = MinSize::Sides(PLACEHOLDER_DECODE_SIDE, 0);
        let decoded = {
            let _decode = DECODE.timer();
            display::decode_into(&mut self.decoder, bytes, min, turns, &mut self.image, &mut self.spare)
        };
        let result = decoded.and_then(|_| self.thumbhash_of_image());
        self.trim();
        result.map_err(MakeError::Undecodable)
    }

    /// The placeholder of a thumbnail the cache holds (`jpeg`: upright already, so no turns), decoded whole: the same
    /// picture as [`Maker::placeholder`] of the file, at the thumbnail's size.
    pub fn placeholder_of_thumbnail(&mut self, jpeg: &[u8]) -> Result<ThumbHash, MakeError> {
        let decoded = {
            let _decode = DECODE.timer();
            display::decode_into(&mut self.decoder, jpeg, MinSize::Full, Turns::NONE, &mut self.image, &mut self.spare)
        };
        let result = decoded.and_then(|_| self.thumbhash_of_image());
        self.trim();
        result.map_err(MakeError::Undecodable)
    }

    /// The placeholder of a video's frame at `at_ms` (its poster, `turns` the file's rotation), as [`Maker::load_frame`]
    /// decodes it.
    pub fn placeholder_of_frame(&mut self, path: &Path, at_ms: i64, turns: Turns) -> Result<ThumbHash, MakeError> {
        self.decode_frame(path, at_ms, turns, MinSize::Sides(PLACEHOLDER_DECODE_SIDE, 0))?;
        let result = self.thumbhash_of_image();
        self.trim();
        result.map_err(MakeError::Undecodable)
    }

    /// The ThumbHash of the decoded image (`self.image`), fitted to [`THUMBHASH_SIDE`] through `small`. The encoder reads
    /// the RGB as it is: the hash of the same pixels as opaque RGBA.
    fn thumbhash_of_image(&mut self) -> Result<ThumbHash> {
        let img = {
            let _resize = RESIZE.timer();
            if self.resizer.fit_within(&self.image, THUMBHASH_SIDE, &mut self.small)? {
                &self.small
            } else {
                &self.image
            }
        };
        let mut bytes = [0; THUMBHASH_MAX];
        let len = {
            let _hash = THUMBHASH.timer();
            self.thumbhasher.rgb_to_thumb_hash_into(img.width, img.height, &img.pixels, &mut bytes)
        };
        Ok(ThumbHash::encoded(bytes, len))
    }

    /// Decode a video's frame (`video::frame`) into the maker, for [`Maker::encode_frame`]: at the size a `height`-px
    /// thumbnail of it is made from, as [`Maker::make`] decodes a still (the frame is scaled while it is converted).
    pub fn load_frame(&mut self, path: &Path, at_ms: i64, turns: Turns, height: usize) -> Result<(), MakeError> {
        self.decode_frame(path, at_ms, turns, MinSize::Upright(0, height * DECODE_MARGIN))
    }

    /// [`Maker::load_frame`] at the size `min` asks for.
    fn decode_frame(&mut self, path: &Path, at_ms: i64, turns: Turns, min: MinSize) -> Result<(), MakeError> {
        let result = pg_video::frame_into(path, at_ms, turns, min, &mut self.image, &mut self.spare);
        if result.is_err() {
            self.trim();
        }
        result.map_err(MakeError::of_video)
    }

    /// A thumbnail `height` px high (or the frame's own height) of the frame [`Maker::load_frame`] decoded.
    pub fn encode_frame(&mut self, height: usize) -> Result<Vec<u8>, MakeError> {
        let mut out = Vec::new();
        self.encode_frame_into(height, &mut out).map(|()| out)
    }

    /// [`Maker::encode_frame`] into `out`, as [`Maker::make_into`].
    pub fn encode_frame_into(&mut self, height: usize, out: &mut Vec<u8>) -> Result<(), MakeError> {
        let result = self.encode_image(height, out);
        self.trim();
        result.map_err(MakeError::Undecodable)
    }

    /// The thumbnail of `image`: scaled to `height`, then encoded into `out`.
    fn encode_image(&mut self, height: usize, out: &mut Vec<u8>) -> Result<()> {
        self.encode_turned(Orientation::UPRIGHT, height, out)
    }

    /// The thumbnail of the decoded image (`self.image`, as stored) shown turned by `orientation`: scaled to `height`
    /// px high once turned, then turned, then encoded into `out`. The scale comes first, on the stored pixels, so the
    /// turn moves the thumbnail's pixels and not the decode's.
    fn encode_turned(&mut self, orientation: Orientation, height: usize, out: &mut Vec<u8>) -> Result<()> {
        let scaled = {
            let _resize = RESIZE.timer();
            if orientation.swaps_axes() {
                self.resizer.to_width(&self.image, height, &mut self.small)?
            } else {
                self.resizer.to_height(&self.image, height, &mut self.small)?
            }
        };
        let img = if scaled { &mut self.small } else { &mut self.image };
        if orientation != Orientation::UPRIGHT {
            let (w, h) = transform_into(&img.pixels, img.width, img.height, 3, orientation, &mut self.spare);
            (img.width, img.height) = (w, h);
            std::mem::swap(&mut img.pixels, &mut self.spare);
        }
        let _encode = ENCODE.timer();
        self.encoder.encode_into(img, out)
    }

    /// Free what one oversize image grew (see [`KEEP_BYTES`]).
    fn trim(&mut self) {
        for buf in [&mut self.image.pixels, &mut self.spare, &mut self.small.pixels, &mut self.file] {
            release_over(buf, KEEP_BYTES);
        }
        self.encoder.trim(KEEP_BYTES);
        self.decoder.trim(KEEP_BYTES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_analysis::analyze::Analyzer;
    use pg_model::analysis::Analysis;
    use pg_testkit::run_folder;
    use std::path::PathBuf;

    fn pattern(w: usize, h: usize, seed: usize) -> Vec<u8> {
        let value = |i: usize| (i / 3 % w) * 7 + (i / 3 / w) * 3 + (i % 3) * 50 + seed * 31;
        // The low byte of each value: the pattern wraps.
        (0..w * h * 3).map(|i| value(i).to_le_bytes()[0]).collect()
    }

    fn jpeg(w: usize, h: usize, seed: usize) -> Vec<u8> {
        let image = Rgb { width: w, height: h, pixels: pattern(w, h, seed) };
        JpegEncoder::new(90, Chroma::Half).unwrap().encode(&image).unwrap()
    }

    /// A PNG with transparency, flattened onto white when decoded.
    fn png_rgba(w: usize, h: usize, seed: usize) -> Vec<u8> {
        let rgb = pattern(w, h, seed);
        // An alpha ramp that wraps: the low byte of i × 13.
        let alpha = |i: usize| (i * 13).to_le_bytes()[0];
        let rgba: Vec<u8> =
            rgb.as_chunks::<3>().0.iter().enumerate().flat_map(|(i, p)| [p[0], p[1], p[2], alpha(i)]).collect();
        pg_testkit::png_rgba(u32::try_from(w).unwrap(), u32::try_from(h).unwrap(), &rgba).unwrap()
    }

    fn summary(r: &Analysis) -> impl PartialEq + std::fmt::Debug + use<> {
        (r.format(), r.orientation, r.width, r.height, r.frames)
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
                let (a, b) = (kept.hash_image(bytes, *turns).unwrap(), new().hash_image(bytes, *turns).unwrap());
                assert_eq!((a.file, a.pixels.pixel_hash), (b.file, b.pixels.pixel_hash));
                let (a, b) = (kept.borders(bytes, *turns).unwrap(), new().borders(bytes, *turns).unwrap());
                assert_eq!((a.width, a.height, a.borders), (b.width, b.height, b.borders));
                let thumb = maker.make(bytes, 64, *turns).unwrap();
                assert_eq!(thumb, Maker::new().unwrap().make(bytes, 64, *turns).unwrap());
                let placeholder = maker.placeholder(bytes, *turns).unwrap();
                assert_eq!(placeholder, Maker::new().unwrap().placeholder(bytes, *turns).unwrap());
            }
        }
    }

    /// The model's longest ThumbHash is the encoder's own: the literal the model writes must be the encoder's length.
    #[test]
    fn thumbhash_max_is_the_encoders_longest() {
        assert_eq!(THUMBHASH_MAX, thumbhash::MAX_LEN);
    }

    /// The placeholder of a file and of the thumbnail the cache keeps of it are both ThumbHashes of the same picture:
    /// each decodes to the same size, and to the same mean colour within a few levels.
    #[test]
    fn a_placeholder_of_the_file_and_of_its_thumbnail_are_the_same_picture() {
        let bytes = jpeg(1200, 800, 1);
        let mut maker = Maker::new().unwrap();
        let of_file = maker.placeholder(&bytes, Turns::NONE).unwrap();
        let thumb = maker.make(&bytes, 256, Turns::NONE).unwrap();
        let of_thumb = maker.placeholder_of_thumbnail(&thumb).unwrap();
        let (w1, h1, p1) =
            thumbhash::thumb_hash_to_rgba(of_file.as_slice()).ok().unwrap_or_else(|| panic!("a ThumbHash"));
        let (w2, h2, p2) =
            thumbhash::thumb_hash_to_rgba(of_thumb.as_slice()).ok().unwrap_or_else(|| panic!("a ThumbHash"));
        assert_eq!((w1, h1), (w2, h2));
        let mean = |p: &[u8]| p.iter().map(|&b| u64::from(b)).sum::<u64>() / u64::try_from(p.len()).unwrap();
        assert!(mean(&p1).abs_diff(mean(&p2)) < 4, "{} and {}", mean(&p1), mean(&p2));
    }

    const SCRATCH: &str = "scratch_maker";

    /// Files read into the maker make the thumbnails and decodes their bytes make, file after file (larger, smaller,
    /// missing, not an image), and a missing file fails as `std::fs::read` does.
    #[test]
    fn files_read_into_the_maker_are_their_bytes() {
        let dir = run_folder(SCRATCH, "read-file").unwrap();
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
                let view = View::Fit(75, 75);
                let decoded = maker.decode_file(view, Turns::new(3)).map(|d| (d.image.width, d.image.pixels));
                let expected =
                    crate::display::decode(&mut Decoder::new().unwrap(), bytes, display::min_size(view), Turns::new(3));
                let expected = expected.map(|d| (d.image.width, d.image.pixels));
                assert_eq!(decoded.map_err(|e| format!("{e:#}")), expected.map_err(|e| format!("{e:#}")));
            }
            let err = maker.read_file(&missing).unwrap_err();
            assert_eq!(err.to_string(), std::fs::read(&missing).unwrap_err().to_string());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What a failure is decides whether it is tried again: a video whose file can't be opened is unreadable (an offline
    /// drive), an image that is cut short doesn't decode, though the codec's own error under it is an I/O one.
    #[test]
    fn a_file_that_cant_be_opened_is_unreadable_and_a_damaged_one_undecodable() {
        let dir = run_folder(SCRATCH, "make-errors").unwrap();
        let mut maker = Maker::new().unwrap();
        let gone = maker.load_frame(&dir.join("gone.mp4"), 0, Turns::NONE, 64);
        assert!(
            matches!(gone, Err(MakeError::Unreadable(ref e)) if e.kind() == std::io::ErrorKind::NotFound),
            "{gone:?}"
        );
        let whole = jpeg(640, 480, 1);
        // Cut inside its frame header (the SOF segment: its marker and length are there, its size is not): the frame
        // is what the decoder can't do without, so the cut file is undecodable. Found by the marker, not an offset.
        let sof = whole.windows(2).position(|w| w == [0xFF, 0xC0] || w == [0xFF, 0xC2]).expect("a frame header");
        let cut = whole.get(..sof + 6).unwrap();
        assert!(matches!(maker.make(cut, 64, Turns::NONE), Err(MakeError::Undecodable(_))));
        assert!(matches!(maker.make(b"not an image", 64, Turns::NONE), Err(MakeError::Undecodable(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
