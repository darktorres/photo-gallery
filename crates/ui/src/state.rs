//! Shared state and the image work behind the QML image providers. Provider requests run on Qt thread-pool
//! threads; each thread keeps its own catalog connection and decoder (thread-locals), and reads the thumbnail
//! cache through its own connection too (`ThumbCache` takes no lock to read).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use pg_core::browse;
use pg_core::catalog::Catalog;
use pg_core::decode::{Decoder, MinSize, Rgb, transform};
use pg_core::metadata::compose_orientation;
use pg_core::resize::{fit_box, resize_region};
use pg_core::thumbs::{self, ThumbCache};
use turbojpeg::Compressor;

pub struct AppState {
    pub catalog_dir: PathBuf,
    pub catalog_path: PathBuf,
    thumbs: ThumbCache,
}

static STATE: OnceLock<AppState> = OnceLock::new();

thread_local! {
    static CATALOG: RefCell<Option<Catalog>> = const { RefCell::new(None) };
    static CODECS: RefCell<Option<(Decoder, Compressor)>> = const { RefCell::new(None) };
}

/// Opens (and migrates) the catalog, as this thread's connection, and the thumbnail cache. Call once, on the GUI
/// thread, before the UI starts. A cache that can't be opened only means thumbnails aren't kept.
pub fn init(catalog_dir: PathBuf, thumb_cap_bytes: Option<i64>) -> Result<()> {
    std::fs::create_dir_all(&catalog_dir)?;
    let catalog_path = catalog_dir.join("catalog.sqlite");
    let catalog = Catalog::open(&catalog_path)?;
    // Kept for the GUI thread rather than closed: closing the last connection checkpoints and deletes the WAL,
    // and the first QML object would open it again.
    CATALOG.with(|c| *c.borrow_mut() = Some(catalog));
    let thumbs =
        ThumbCache::open(&catalog_dir, thumb_cap_bytes, Box::new(|e| eprintln!("{e:#}"))).unwrap_or_else(|e| {
            eprintln!("thumbnail cache disabled: {e:#}");
            ThumbCache::disabled()
        });
    STATE.set(AppState { catalog_dir, catalog_path, thumbs }).map_err(|_| anyhow::anyhow!("state initialized twice"))
}

pub fn get() -> &'static AppState {
    STATE.get().expect("state::init not called")
}

/// The thumbnail cache, shared by the image provider and the background job.
pub fn thumbs() -> &'static ThumbCache {
    &get().thumbs
}

/// After the event loop: write what the thumbnail cache's writer still holds (the `static` is never dropped).
pub fn shutdown() {
    if let Some(state) = STATE.get() {
        state.thumbs.close();
    }
}

/// Run `f` with this thread's catalog connection.
pub fn with_catalog<R>(f: impl FnOnce(&Catalog) -> Result<R>) -> Result<R> {
    CATALOG.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = Some(Catalog::open(&get().catalog_path)?);
        }
        f(c.as_ref().unwrap())
    })
}

fn with_codecs<R>(f: impl FnOnce(&mut Decoder, &mut Compressor) -> Result<R>) -> Result<R> {
    CODECS.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = Some((Decoder::new()?, Compressor::new()?));
        }
        let (d, e) = c.as_mut().unwrap();
        f(d, e)
    })
}

fn source(file_id: i64) -> Result<browse::FileSource> {
    with_catalog(|c| browse::file_source(&c.conn, file_id))?.context("unknown file")
}

pub(crate) fn long_path(path: &str) -> String {
    if path.len() < 240 || path.starts_with(r"\\") { path.to_string() } else { format!(r"\\?\{path}") }
}

fn read_original(path: &str) -> std::io::Result<Vec<u8>> {
    std::fs::read(long_path(path))
}

/// A JPEG thumbnail `height` px high: from the cache when one at least that high exists (the UI scales it down),
/// otherwise made now from the original and cached. With the original offline, the best smaller one.
pub fn thumbnail(file_id: i64, height: u32) -> Result<Vec<u8>> {
    let browse::FileSource { path, mtime, rotation, poster_ms, .. } = source(file_id)?;
    let current = thumbs::Source { mtime, rotation };
    let cached = thumbs().get_nearest(file_id, height, current)?;
    if let Some((h, data)) = &cached
        && *h >= height
    {
        return Ok(data.clone());
    }
    let data = if let Some(at) = poster_ms {
        // A video: its poster frame.
        match pg_core::video::frame(Path::new(&long_path(&path)), at, rotation) {
            Ok(frame) => with_codecs(|_, e| thumbs::encode(&frame, height as usize, e))?,
            Err(e) => return cached.map(|(_, d)| d).ok_or(e).context("original unavailable"),
        }
    } else {
        let bytes = match read_original(&path) {
            Ok(b) => b,
            Err(e) => return cached.map(|(_, d)| d).ok_or(e).context("original unavailable"),
        };
        with_codecs(|d, e| thumbs::make(&bytes, height as usize, rotation, d, e))?
    };
    // Kept in the background: shown now, whatever the write does.
    thumbs().put(thumbs::Thumb { file_id, height, source: current, data: data.clone() });
    Ok(data)
}

/// A thumbnail turned by `turns` quarter turns clockwise (a turn not yet written into the file).
pub fn turn_jpeg(jpeg: &[u8], turns: u8) -> Result<Rgb> {
    with_codecs(|d, _| Ok(d.decode(jpeg, MinSize::Full, turns)?.image))
}

/// The ThumbHash placeholder as a tiny RGBA image, turned by `turns`.
pub fn thumbhash(file_id: i64, turns: u8) -> Result<(usize, usize, Vec<u8>)> {
    let hash = source(file_id)?.thumbhash.context("not imported yet")?;
    let (w, h, rgba) = thumbhash::thumb_hash_to_rgba(&hash).map_err(|_| anyhow::anyhow!("bad thumbhash"))?;
    let (rgba, w, h) = transform(&rgba, w, h, 4, compose_orientation(1, turns));
    Ok((w, h, rgba))
}

/// Lanczos3 from a DCT-scaled decode keeping ≥ 4× the target matches a full decode (FINDINGS "Thumbnails").
const DECODE_MARGIN: usize = 4;

/// What the viewer asks for.
#[derive(Clone, Copy, Debug)]
pub enum View {
    /// Fitted into (width, height) device pixels, never enlarged.
    Fit(usize, usize),
    /// Full size (1:1 and beyond, and the loupe).
    Full,
    /// Zoom between fit and 1:1: the region (x, y, w, h, as fractions of the upright image) at (width, height)
    /// device pixels, the part of the zoomed image that is on screen.
    Region((f64, f64, f64, f64), usize, usize),
}

/// The last decode, kept for the zoom regions that follow each other while the user zooms and pans one image.
struct Decoded {
    file_id: i64,
    mtime: i64,
    rotation: u8,
    /// Upright full size, and the decode (possibly DCT-scaled).
    full: (usize, usize),
    image: Arc<Rgb>,
}

static LAST_DECODE: Mutex<Option<Decoded>> = Mutex::new(None);

/// The original, upright (EXIF and catalog rotation, then `turns` not yet written), as the viewer asks for it.
pub fn full_image(file_id: i64, view: View, turns: u8) -> Result<Rgb> {
    let mut src = source(file_id)?;
    src.rotation = (src.rotation + turns) % 4;
    match view {
        View::Fit(w, h) => {
            let img = decode(file_id, &src, MinSize::Fit(w * DECODE_MARGIN, h * DECODE_MARGIN))?;
            Ok(fit_box(&img, w, h).unwrap_or_else(|| (*img).clone()))
        }
        View::Full => Ok((*decode(file_id, &src, MinSize::Full)?).clone()),
        View::Region((x, y, rw, rh), w, h) => {
            // The whole image at a scale where the region keeps ≥ DECODE_MARGIN× the output pixels.
            let box_w = (w as f64 * DECODE_MARGIN as f64 / rw.max(1e-6)).ceil() as usize;
            let box_h = (h as f64 * DECODE_MARGIN as f64 / rh.max(1e-6)).ceil() as usize;
            let img = decode(file_id, &src, MinSize::Fit(box_w, box_h))?;
            let (iw, ih) = (img.width as f64, img.height as f64);
            Ok(resize_region(&img, (x * iw, y * ih, rw * iw, rh * ih), w, h))
        }
    }
}

/// Decode at least `min`, reusing the last decode of the same file when it is big enough.
fn decode(file_id: i64, src: &browse::FileSource, min: MinSize) -> Result<Arc<Rgb>> {
    {
        let last = LAST_DECODE.lock().unwrap();
        if let Some(d) = last.as_ref()
            && d.file_id == file_id
            && d.mtime == src.mtime
            && d.rotation == src.rotation
            && big_enough(d.image.width, d.image.height, d.full, min)
        {
            return Ok(d.image.clone());
        }
    }
    let (full, image) = if let Some(at) = src.poster_ms {
        // A video: its poster frame stands for it until it plays.
        let frame = pg_core::video::frame(Path::new(&long_path(&src.path)), at, src.rotation)?;
        ((frame.width, frame.height), Arc::new(frame))
    } else {
        let bytes = read_original(&src.path).context("original unavailable")?;
        let decoded = with_codecs(|d, _| d.decode(&bytes, min, src.rotation))?;
        ((decoded.full_width as usize, decoded.full_height as usize), Arc::new(decoded.image))
    };
    *LAST_DECODE.lock().unwrap() =
        Some(Decoded { file_id, mtime: src.mtime, rotation: src.rotation, full, image: image.clone() });
    Ok(image)
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
