//! What the import computes from one file: what it is and how big it is, from its header and its container, with no
//! decoding. The pixel work (the duplicate hashes, the thumbnails, the placeholders, the borders) is a pass of its own
//! after the import.

use crate::{Error, Result, side};
use pg_model::turns::Turns;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::borders::{self, Detector};
use crate::pdq;
use pg_codec::decode::{Decoder, MinSize, Request};
use pg_codec::pixels::{Rgb, release_over};
use pg_codec::resize::Resizer;
use pg_metadata::{Meta, MetaScratch};
use pg_model::analysis::{
    Analysis, DECODE_MIN_LONG, DECODE_MIN_SHORT, FileHash, ImageFacts, ImageHashes, Media, PDQ_SIDE, PixelHashes,
    Taken, VideoFacts,
};
use pg_model::detection::{self, Detection};
use pg_model::format::Format;

/// The version of what the import computes, stored with each analysis; files analyzed by an older one are redone,
/// in place, which keeps the library, ratings and views. Not a schema version (the catalog's schema is `pg_catalog`'s
/// baseline and migrations). Bumped when what is stored changes: the rule, per input, is the registry's
/// (`pg_model::versions`). Not bumped by the header-only import: the stored facts are the same values, read from the
/// header instead of a decode.
pub const IMPORT_VERSION: i64 = pg_model::versions::IMPORT_VERSION;

/// The chunk a video's hash reads at a time: a stop waits for one chunk at most (a few milliseconds at a disk's rate),
/// not for the whole file (section 7: about 6 s per GB on a hard disk).
pub const HASH_CHUNK: usize = 1 << 20;

/// The content hash of a file streamed from `path`, for a video (not read whole): `FileHash::of` its bytes, read in
/// chunks of [`HASH_CHUNK`] with `stopped` asked before each. `None` when it said stop first: not hashed yet.
fn file_hash_of(path: &Path, stopped: &dyn Fn() -> bool) -> io::Result<Option<FileHash>> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut chunk = vec![0u8; HASH_CHUNK];
    loop {
        if stopped() {
            return Ok(None);
        }
        let read = match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(chunk.get(..read).unwrap_or_default());
    }
    Ok(Some(FileHash(*hasher.finalize().as_bytes())))
}

/// A worker's buffers for one file's decode, scaled copies and hashes, kept from file to file (a file larger than
/// [`KEEP_BYTES`] frees them after it). Its methods are the import's analysis, the Hashes pass and the borders pass.
pub struct Analyzer {
    decoder: Decoder,
    resizer: Resizer,
    hasher: pdq::Hasher,
    detector: Detector,
    /// The decoded image (for the hashes and the borders)...
    pub(crate) image: Rgb,
    /// ...and the buffer it is oriented through.
    pub(crate) spare: Vec<u8>,
    /// The image fitted for PDQ.
    pdq_in: Rgb,
    /// What reading each file's metadata works in.
    meta: MetaScratch,
}

/// What a worker keeps of each buffer: a 32 MP RGB image's pixels. A larger file's are freed once it is done.
pub const KEEP_BYTES: usize = 32_000_000 * 3;

impl Analyzer {
    /// An analyzer with its own decoder and resizer, for one worker's files.
    pub fn new() -> Result<Analyzer> {
        Ok(Analyzer {
            decoder: Decoder::new()?,
            resizer: Resizer::new(),
            hasher: pdq::Hasher::default(),
            detector: Detector::default(),
            image: Rgb::default(),
            spare: Vec::new(),
            pdq_in: Rgb::default(),
            meta: MetaScratch::default(),
        })
    }

    /// The import's analysis of an image, from its header alone: no pixels are decoded. `turns`: the file's catalog
    /// rotation (quarter turns clockwise), part of what "upright" means.
    pub fn analyze(&mut self, bytes: &[u8], turns: Turns) -> Result<Analysis> {
        self.analyze_image(bytes, turns)
    }

    /// The import's analysis of a video: upright size, duration, codec, recording date and frame count, from the
    /// container's headers (`pg_video::probe`). The poster frame is decoded later, by the thumbnail and placeholder
    /// passes. `turns` is the catalog rotation.
    pub fn video(&mut self, path: &Path, turns: Turns) -> Result<Analysis> {
        analyze_video(path, turns)
    }

    /// The duplicate hashes of an image, for the Hashes pass: decoded at the hashing size, and the same values whenever
    /// they are made.
    pub fn hash_image(&mut self, bytes: &[u8], turns: Turns) -> Result<ImageHashes> {
        let result = self.hash_decoded(bytes, turns);
        self.trim();
        result
    }

    /// The file hash (tier 0a) of a video, streamed from `path` in chunks, for the Hashes pass. `stopped` is asked between
    /// chunks: `None` when it said stop, not hashed yet (not a failure: the file stays owed).
    pub fn video_file_hash(&self, path: &Path, stopped: &dyn Fn() -> bool) -> Result<Option<FileHash>> {
        file_hash_of(path, stopped).map_err(|source| Error::Read { path: path.to_path_buf(), source })
    }

    fn analyze_image(&mut self, bytes: &[u8], turns: Turns) -> Result<Analysis> {
        let format = Format::sniff(bytes).ok_or(Error::Invalid("unknown image format"))?;
        let meta = Meta::read(bytes, format, &mut self.meta);
        let dims = self.decoder.dimensions(bytes, format, meta.orientation, turns)?;
        Ok(Analysis {
            version: IMPORT_VERSION,
            orientation: dims.orientation,
            exif_orientation: meta.orientation,
            width: dims.width,
            height: dims.height,
            frames: meta.frames,
            taken: meta.taken.map(Taken::Wall),
            media: Media::Image(ImageFacts { format, file_rating: meta.rating }),
        })
    }

    /// Decodes an image for its duplicate hashes, at the hashing decode's size.
    fn hash_decoded(&mut self, bytes: &[u8], turns: Turns) -> Result<ImageHashes> {
        let format = Format::sniff(bytes).ok_or(Error::Invalid("unknown image format"))?;
        let meta = Meta::read(bytes, format, &mut self.meta);
        let min = MinSize::Sides(DECODE_MIN_LONG, DECODE_MIN_SHORT);
        let (image, spare) = (&mut self.image, &mut self.spare);
        self.decoder.decode_into(bytes, Request { format, exif: meta.orientation, min, turns }, image, spare)?;
        image_hashes(FileHash::of(bytes), &self.image, &mut self.resizer, &mut self.pdq_in, &mut self.hasher)
    }

    /// The borders of a file ([`borders::detect`] on its stored pixels, at full size), looked for after the
    /// import, in a pass of their own. `None` for animated images and images too small to look at.
    pub fn borders(&mut self, bytes: &[u8], turns: Turns) -> Result<FileBorders> {
        let result = self.find_borders(bytes, turns);
        self.trim();
        result
    }

    fn find_borders(&mut self, bytes: &[u8], turns: Turns) -> Result<FileBorders> {
        let format = Format::sniff(bytes).ok_or(Error::Invalid("unknown image format"))?;
        let meta = Meta::read(bytes, format, &mut self.meta);
        let request = Request { format, exif: meta.orientation, min: MinSize::Full, turns };
        let info = self.decoder.decode_stored_into(bytes, request, &mut self.image)?;
        let frames = meta.frames;
        // The stored size, upright as `analysis.width/height` are (the crop's cut is of the stored pixels).
        let (width, height) = if info.orientation.swaps_axes() {
            (info.full_height, info.full_width)
        } else {
            (info.full_width, info.full_height)
        };
        let borders = (frames <= 1 && pg_model::num::index(width.min(height)) >= detection::MIN_SIDE)
            .then(|| self.detector.detect(&self.image, &borders::Params::default()))
            .transpose()?;
        Ok(FileBorders { width, height, borders })
    }

    /// Free what one oversize file grew (see [`KEEP_BYTES`]).
    pub(crate) fn trim(&mut self) {
        for buf in [&mut self.image.pixels, &mut self.spare, &mut self.pdq_in.pixels] {
            release_over(buf, KEEP_BYTES);
        }
        self.decoder.trim(KEEP_BYTES);
        self.detector.trim(KEEP_BYTES);
    }
}

/// What [`Analyzer::borders`] found.
pub struct FileBorders {
    /// The stored size, upright (what `analysis.width/height` hold): the borders pass checks the header's dimensions
    /// against it.
    pub width: u32,
    /// The stored height, upright, as `width` is.
    pub height: u32,
    /// `None`: not looked for (animated, tiny).
    pub borders: Option<Detection>,
}

/// The duplicate hashes of an upright decoded image and of its file's bytes (`file`): the one definition the Hashes pass
/// makes. The PDQ input is the image fitted into [`PDQ_SIDE`] (`pdq_in` is its buffer).
fn image_hashes(
    file: FileHash,
    img: &Rgb,
    resizer: &mut Resizer,
    pdq_in: &mut Rgb,
    hasher: &mut pdq::Hasher,
) -> Result<ImageHashes> {
    let pdq_img = if resizer.fit_within(img, PDQ_SIDE, pdq_in)? { &*pdq_in } else { img };
    let pdq = hasher.hash_rgb_dihedral(&pdq_img.pixels, pdq_img.width, pdq_img.height);
    Ok(ImageHashes { file, pixels: PixelHashes { pixel_hash: pixel_hash(img)?, pdq } })
}

/// Hash of the upright pixels at the decode scale. The scale depends only on the stored size, so two files with
/// the same image data (re-saved metadata, a different container, lossless re-wraps like progressive <-> baseline)
/// hash equal. The same pixels in a different codec (JPEG vs PNG) usually decode at a different scale and don't;
/// tier 1 finds those at PDQ distance 0. An error for a side over `u32::MAX`, which the hash takes as 4 bytes.
pub fn pixel_hash(img: &Rgb) -> Result<[u8; 16]> {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    h.update(&side(img.width, "image width over u32")?.to_le_bytes());
    h.update(&side(img.height, "image height over u32")?.to_le_bytes());
    h.update(&img.pixels);
    Ok(h.digest128().to_le_bytes())
}

fn analyze_video(path: &Path, turns: Turns) -> Result<Analysis> {
    let p = pg_video::probe(path)?;
    let poster = pg_video::poster_ms(p.duration_ms);
    let (width, height) = if turns.swaps_axes() { (p.height, p.width) } else { (p.width, p.height) };
    Ok(Analysis {
        version: IMPORT_VERSION,
        // The stills' orientation includes the catalog turn (`decode_into`); the video's must too, or its stored
        // size and its shown orientation disagree. `width`/`height` above already carry the display matrix.
        orientation: p.orientation.compose(turns),
        exif_orientation: p.orientation,
        width,
        height,
        frames: p.frames,
        taken: p.taken,
        // An unknown length is the grid's sentinel; the poster is clamped to 0..=POSTER_MAX_MS by `poster_ms`.
        media: Media::Video(VideoFacts {
            codec: p.codec,
            duration_ms: u32::try_from(p.duration_ms).unwrap_or(pg_model::grid::Entry::NO_DURATION),
            poster_ms: u32::try_from(poster).unwrap_or(0),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A streamed file (a video) hashes as its loaded bytes do: one digest for both ways in.
    #[test]
    fn a_streamed_file_hashes_as_its_bytes_do() {
        let dir = pg_testkit::run_folder("scratch_analysis", "analysis").unwrap();
        let bytes: Vec<u8> =
            (0..3_000_000u32).map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24).unwrap()).collect();
        let path = dir.join("streamed.bin");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(file_hash_of(&path, &|| false).unwrap(), Some(FileHash::of(&bytes)));
        assert!(file_hash_of(&dir.join("missing.bin"), &|| false).is_err());
    }

    // A stop is looked for between chunks (section 7): asked after the first chunk, the hash stops there, having read
    // one chunk, and says "not hashed yet" rather than failing.
    #[test]
    fn a_video_hash_stops_within_one_chunk() {
        let dir = pg_testkit::run_folder("scratch_analysis", "analysis-stop").unwrap();
        let path = dir.join("long.bin");
        std::fs::write(&path, vec![7u8; HASH_CHUNK * 4]).unwrap();
        let asked = std::sync::atomic::AtomicUsize::new(0);
        let stopped = || asked.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 1;
        assert_eq!(file_hash_of(&path, &stopped).unwrap(), None);
        assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 2, "asked before the first chunk and the second");
    }

    // The duplicate hashes are the same whenever they are made: the decode at the hashing size is deterministic, and the
    // import stores none.
    #[test]
    fn the_hashes_are_made_from_the_same_decode_each_time() {
        let bytes = pg_codec::fixtures::png_rgb(8, 8, &[128; 192]).unwrap();
        let mut analyzer = Analyzer::new().unwrap();
        let turns = pg_model::turns::Turns::NONE;
        let first = analyzer.hash_image(&bytes, turns).unwrap();
        let again = analyzer.hash_image(&bytes, turns).unwrap();
        assert_eq!(first.file, again.file);
        assert_eq!(first.pixels.pixel_hash, again.pixels.pixel_hash);
        assert_eq!(
            pg_model::pdq::to_blob(&first.pixels.pdq.orientations),
            pg_model::pdq::to_blob(&again.pixels.pdq.orientations)
        );
    }
}
