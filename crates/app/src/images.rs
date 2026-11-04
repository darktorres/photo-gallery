//! The images the front end shows: thumbnails, placeholders, and the original as the viewer asks for it. Each image is made
//! from one consistent view of its file, never a mix: a thumbnail the cache keeps for the file as its key names it is
//! served from the key alone, turned by the key's turns; anything else is made from the file as the catalog has it now,
//! turned by the catalog's turns, and checked against the catalog before and after the file is read (`consistent`), so
//! that a write of the app's into the file at that moment is neither shown twice nor kept under the state before it.
//!
//! A thumbnail made on a miss goes to the cache's writer once it is shown, moved there rather than copied ([`Keep`]):
//! the front end hands it back with [`App::keep`] once it is done with the bytes.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pg_catalog::browse;
use pg_codec::decode::{MinSize, orient_pixels};
use pg_codec::pixels::Rgb;
use pg_codec::resize::Resizer;
use pg_model::ids::FileId;
use pg_model::orientation::Orientation;
use pg_model::state::FileSource;
use pg_model::turns::Turns;
use pg_thumbcache::{self as thumbs};

use crate::app::App;
use crate::error::{Error, Result, failed};

/// How long a request waits for a write of the app's into its file to end before it fails.
const WRITE_WAIT: Duration = Duration::from_secs(10);

/// Lanczos3 from a DCT-scaled decode keeping ≥ 4× the target matches a full decode (FINDINGS "Thumbnails").
const DECODE_MARGIN: usize = 4;

/// What an image key names (the grid's image key): the file as the grid showed it when the key was made.
#[derive(Clone, Copy, Debug)]
pub struct Named {
    pub source: thumbs::Source,
    /// Quarter turns clockwise asked for and not yet written then.
    pub turns: Turns,
}

/// What a thumbnail request found, as the catalog has the file.
enum Found {
    /// In the cache: read into the caller's buffer.
    Kept,
    /// Made from the original.
    Made(Vec<u8>),
    /// The original can't be read (offline). Whether the best smaller thumbnail the cache keeps of the file as it is was
    /// read into the caller's buffer.
    Offline(Error, bool),
}

/// A thumbnail, as [`App::thumbnail`] finds it.
pub struct Thumbnail {
    /// Quarter turns clockwise to show it turned by.
    pub turns: Turns,
    /// One made from the original: its bytes, and where the cache keeps it once it is shown. Otherwise the bytes are the
    /// cache's, in the buffer [`App::thumbnail`] was given.
    pub made: Option<(Vec<u8>, Keep)>,
}

/// Where the cache keeps a thumbnail made on a miss ([`App::keep`]).
#[derive(Clone, Copy, Debug)]
pub struct Keep {
    pub file_id: FileId,
    pub height: u32,
    pub source: thumbs::Source,
}

/// What the viewer asks for.
#[derive(Clone, Copy, Debug)]
pub enum View {
    /// Fitted into (width, height) device pixels, up or down.
    Fit(usize, usize),
    /// Full size (1:1 and beyond, and the loupe).
    Full,
    /// Zoom between fit and 1:1: the region (x, y, w, h, as fractions of the upright image) at (width, height) device
    /// pixels, the part of the zoomed image that is on screen.
    Region((f64, f64, f64, f64), usize, usize),
}

/// An image in a format Qt's scene graph uploads as it is (`QSGPlainTexture`): 4 bytes a pixel, RGBX8888 or RGBA8888
/// premultiplied. Any other format it converts on the render thread, a copy of the whole image at every upload.
pub struct Upload {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    /// RGBA8888 premultiplied; else RGBX8888 (X = 255).
    pub premultiplied: bool,
}

impl Upload {
    /// `img` as RGBX8888.
    pub fn opaque(img: &Rgb) -> Upload {
        let mut pixels = vec![255u8; img.width * img.height * 4];
        for (out, rgb) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(img.pixels.as_chunks::<3>().0) {
            out[..3].copy_from_slice(rgb);
        }
        Upload { width: img.width, height: img.height, pixels, premultiplied: false }
    }

    /// Straight RGBA as RGBA8888 premultiplied, in place, rounded as Qt premultiplies (`qPremultiply`), so the texture is
    /// the one Qt's own conversion made.
    pub fn straight(width: usize, height: usize, mut pixels: Vec<u8>) -> Upload {
        for px in pixels.as_chunks_mut::<4>().0 {
            let a = px[3] as u32;
            if a != 255 {
                for c in &mut px[..3] {
                    let t = *c as u32 * a;
                    *c = ((t + (t >> 8) + 0x80) >> 8) as u8;
                }
            }
        }
        Upload { width, height, pixels, premultiplied: true }
    }
}

/// The last full-size decode, kept for the zoom regions that follow each other while the user zooms and pans one image.
/// It is kept for the app, until the next decode: a 24 MP image is about 72 MB of RGB.
pub(crate) struct Decoded {
    file_id: FileId,
    mtime: i64,
    /// The file's size: a decode is reused only while the file still has it.
    size: i64,
    /// With the turns not yet written.
    rotation: Turns,
    /// Upright full size, and the decode (possibly DCT-scaled).
    full: (usize, usize),
    image: Arc<Rgb>,
}

impl App {
    /// `read` of the file, consistent with the catalog: the file's state is read before and after it, and the result
    /// counts only when no write of the app's into the file was in progress and the state stayed the same. The writer
    /// marks a write before it rewrites the file and records it in the transaction that ends the mark, so a write that
    /// overlapped the read shows in one state or the other. Otherwise it is read again once the write is done.
    fn consistent<T>(
        &self,
        file_id: FileId,
        mut read: impl FnMut(&FileSource) -> Result<T>,
    ) -> Result<(FileSource, T)> {
        let start = Instant::now();
        loop {
            let src = self.source(file_id)?;
            if !src.state.writing {
                let result = read(&src);
                if self.with_catalog(|c| browse::read::file_state(c, file_id))? == Some(src.state) {
                    return result.map(|r| (src, r));
                }
            }
            if start.elapsed() > WRITE_WAIT {
                return Err(Error::Rewriting(file_id.get()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn source(&self, file_id: FileId) -> Result<FileSource> {
        self.with_catalog(|c| browse::read::file_source(c, file_id))?.ok_or(Error::UnknownFile(file_id.get()))
    }

    /// A JPEG thumbnail `height` px high, and the quarter turns clockwise to show it turned by. When the cache keeps one at
    /// least that high for the file as `named` names it, that one, turned by the name's turns: nothing else is read.
    /// Otherwise as the catalog has the file now: kept, or made from the original (and to be kept), and turned by the
    /// catalog's turns. With the original offline, the best smaller one.
    ///
    /// A kept one is read into `jpeg`, a buffer the caller keeps from one thumbnail to the next, so that showing a cached
    /// thumbnail makes no buffer for its bytes.
    pub fn thumbnail(
        &self,
        file_id: FileId,
        height: u32,
        named: Option<Named>,
        jpeg: &mut Vec<u8>,
    ) -> Result<Thumbnail> {
        let kept = |turns| Thumbnail { turns, made: None };
        // A smaller one of the file as named, read into `jpeg`.
        let mut smaller = false;
        if let Some(n) = named {
            match self.thumbs.get_nearest_into(file_id, height, n.source, jpeg).map_err(failed)? {
                Some(h) if h >= height => return Ok(kept(n.turns)),
                Some(_) => smaller = true,
                None => {}
            }
        }
        let (src, found) = self.consistent(file_id, |src| {
            let current = thumbs::Source { mtime: src.state.mtime, rotation: src.state.rotation };
            let kept_smaller = if named.is_some_and(|n| n.source == current) {
                // Looked for already.
                false
            } else {
                match self.thumbs.get_nearest_into(file_id, height, current, jpeg).map_err(failed)? {
                    Some(h) if h >= height => return Ok(Found::Kept),
                    found => {
                        // A read overwrote the named one's bytes.
                        smaller &= found.is_none();
                        found.is_some()
                    }
                }
            };
            let made = if let Some(at) = src.poster_ms {
                // A video: its poster frame.
                match self.with_maker(|m| Ok(m.load_frame(Path::new(&src.path), at, current.rotation)))? {
                    Ok(()) => self.with_maker(|m| m.encode_frame(height as usize))?,
                    Err(e) => return Ok(Found::Offline(failed(e), kept_smaller)),
                }
            } else {
                // Read into the maker, which keeps the buffer for the next file.
                match self.with_maker(|m| Ok(m.read_file(Path::new(&src.path))))? {
                    Ok(()) => {}
                    Err(e) => return Ok(Found::Offline(failed(e), kept_smaller)),
                }
                self.with_maker(|m| m.make_from_file(height as usize, current.rotation))?
            };
            Ok(Found::Made(made))
        })?;
        let turns = src.state.turns;
        match found {
            Found::Kept | Found::Offline(_, true) => Ok(kept(turns)),
            Found::Made(made) => {
                let source = thumbs::Source { mtime: src.state.mtime, rotation: src.state.rotation };
                Ok(Thumbnail { turns, made: Some((made, Keep { file_id, height, source })) })
            }
            Found::Offline(e, false) => match named {
                Some(n) if smaller => Ok(kept(n.turns)),
                // A read of the file as it is (an earlier try of `consistent`) overwrote it: read again.
                Some(n) if self.thumbs.get_nearest_into(file_id, height, n.source, jpeg).map_err(failed)?.is_some() => {
                    Ok(kept(n.turns))
                }
                _ => Err(e),
            },
        }
    }

    /// Put a thumbnail made on a miss into the cache, in the background: it is shown whatever the write does.
    pub fn keep(&self, keep: Keep, data: Vec<u8>) {
        self.thumbs.put(thumbs::Thumb { file_id: keep.file_id, height: keep.height, source: keep.source, data });
    }

    /// A thumbnail (a JPEG) turned by `turns` quarter turns clockwise, for a turn not yet written into the file.
    pub fn turn_jpeg(&self, jpeg: &[u8], turns: Turns) -> Result<Upload> {
        self.with_maker(|m| {
            let decoded = pg_render::display::decode(m.decoder(), jpeg, MinSize::Full, turns)?;
            Ok(Upload::opaque(&decoded.image))
        })
    }

    /// The ThumbHash placeholder as a tiny image, shown as the catalog has the file now.
    pub fn placeholder(&self, file_id: FileId) -> Result<Upload> {
        let p =
            self.with_catalog(|c| browse::read::placeholder(c, file_id))?.ok_or(Error::UnknownFile(file_id.get()))?;
        let hash = p.hash.ok_or(Error::NotImported)?;
        let (w, h, rgba) =
            thumbhash::thumb_hash_to_rgba(hash.as_slice()).map_err(|_| Error::Failed("bad thumbhash".to_string()))?;
        let (rgba, w, h) = orient_pixels(rgba, w, h, 4, Orientation::UPRIGHT.compose(p.turns));
        Ok(Upload::straight(w, h, rgba))
    }

    /// The original as the catalog has it now, upright (EXIF and catalog rotation, then the turns not yet written), as the
    /// viewer asks for it.
    pub fn full_image(&self, file_id: FileId, view: View) -> Result<Upload> {
        let min = match view {
            View::Fit(w, h) => MinSize::Fit(w * DECODE_MARGIN, h * DECODE_MARGIN),
            View::Full => MinSize::Full,
            // The whole image at a scale where the region keeps ≥ DECODE_MARGIN× the output pixels.
            View::Region((_, _, rw, rh), w, h) => MinSize::Fit(
                (w as f64 * DECODE_MARGIN as f64 / rw.max(1e-6)).ceil() as usize,
                (h as f64 * DECODE_MARGIN as f64 / rh.max(1e-6)).ceil() as usize,
            ),
        };
        let (_, (img, fresh)) = self.consistent(file_id, |src| self.decode(file_id, src, min))?;
        // Kept once it is known to be of the file as the catalog has it.
        if let Some(fresh) = fresh {
            *self.last_decode.lock().unwrap() = Some(fresh);
        }
        // The decode stays shared (`last_decode`); what is shown is made from it in the upload format.
        match view {
            View::Fit(w, h) => {
                let mut fitted = Rgb::default();
                Resizer::new().fit_box(&img, w, h, &mut fitted);
                Ok(Upload::opaque(&fitted))
            }
            View::Full => Ok(Upload::opaque(&img)),
            View::Region((x, y, rw, rh), w, h) => {
                let (iw, ih) = (img.width as f64, img.height as f64);
                let mut out = Rgb::default();
                Resizer::new().resize_region(&img, (x * iw, y * ih, rw * iw, rh * ih), w, h, &mut out);
                Ok(Upload::opaque(&out))
            }
        }
    }

    /// Decode at least `min`, turned as the catalog says, reusing the last decode of the file in that state when it is big
    /// enough; a new decode comes with what to keep of it.
    fn decode(&self, file_id: FileId, src: &FileSource, min: MinSize) -> Result<(Arc<Rgb>, Option<Decoded>)> {
        let rotation = src.state.rotation.plus(src.state.turns);
        {
            let last = self.last_decode.lock().unwrap();
            if let Some(d) = last.as_ref()
                && d.file_id == file_id
                && d.mtime == src.state.mtime
                && d.size == src.state.size
                && d.rotation == rotation
                && big_enough(d.image.width, d.image.height, d.full, min)
            {
                return Ok((d.image.clone(), None));
            }
        }
        let (full, image) = if let Some(at) = src.poster_ms {
            // A video: its poster frame stands for it until it plays.
            let frame = pg_video::frame(Path::new(&src.path), at, rotation).map_err(failed)?;
            ((frame.width, frame.height), Arc::new(frame))
        } else {
            let decoded = self.with_maker(|m| {
                m.read_file(Path::new(&src.path)).map_err(|e| anyhow::anyhow!("original unavailable: {e}"))?;
                m.decode_file(min, rotation)
            })?;
            ((decoded.info.full_width as usize, decoded.info.full_height as usize), Arc::new(decoded.image))
        };
        Ok((
            image.clone(),
            Some(Decoded { file_id, mtime: src.state.mtime, size: src.state.size, rotation, full, image }),
        ))
    }
}

/// Whether a decode of `w × h` (of an upright full size `full`) satisfies `min`.
fn big_enough(w: usize, h: usize, full: (usize, usize), min: MinSize) -> bool {
    if (w, h) == full {
        return true;
    }
    match min {
        MinSize::Full => false,
        MinSize::Fit(bw, bh) => {
            let scale = (bw as f64 / full.0 as f64).min(bh as f64 / full.1 as f64).min(1.0);
            w as f64 >= (full.0 as f64 * scale).floor() && h as f64 >= (full.1 as f64 * scale).floor()
        }
        MinSize::Upright(mw, mh) => w >= mw.min(full.0) && h >= mh.min(full.1),
        MinSize::Sides(long, short) => {
            w.max(h) >= long.min(full.0.max(full.1)) && w.min(h) >= short.min(full.0.min(full.1))
        }
    }
}
