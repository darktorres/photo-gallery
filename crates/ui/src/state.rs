//! Shared state and the image work behind the QML image providers. Provider requests run on Qt thread-pool
//! threads; each thread keeps its own catalog connection and decoder (thread-locals), and the thumbnail cache is
//! shared behind a mutex.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use pg_core::browse;
use pg_core::catalog::Catalog;
use pg_core::decode::{Decoder, MinSize, Rgb};
use pg_core::resize::fit_box;
use pg_core::thumbs::{self, ThumbCache};
use turbojpeg::Compressor;

pub struct AppState {
    pub catalog_path: PathBuf,
    thumbs: Mutex<ThumbCache>,
}

static STATE: OnceLock<AppState> = OnceLock::new();

thread_local! {
    static CATALOG: RefCell<Option<Catalog>> = const { RefCell::new(None) };
    static CODECS: RefCell<Option<(Decoder, Compressor)>> = const { RefCell::new(None) };
}

/// Opens (and migrates) the catalog and the thumbnail cache. Call once, before the UI starts.
pub fn init(catalog_dir: PathBuf, thumb_cap_bytes: i64) -> Result<()> {
    std::fs::create_dir_all(&catalog_dir)?;
    let catalog_path = catalog_dir.join("catalog.sqlite");
    Catalog::open(&catalog_path)?;
    let thumbs = Mutex::new(ThumbCache::open(&catalog_dir.join("thumbs.sqlite"), thumb_cap_bytes)?);
    STATE.set(AppState { catalog_path, thumbs }).map_err(|_| anyhow::anyhow!("state initialized twice"))
}

pub fn get() -> &'static AppState {
    STATE.get().expect("state::init not called")
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

fn read_original(path: &str) -> std::io::Result<Vec<u8>> {
    let long = if path.len() < 240 || path.starts_with(r"\\") { path.to_string() } else { format!(r"\\?\{path}") };
    std::fs::read(long)
}

/// A JPEG thumbnail `height` px high: from the cache when one at least that high exists (the UI scales it down),
/// otherwise made now from the original and cached. With the original offline, the best smaller one.
pub fn thumbnail(file_id: i64, height: u32) -> Result<Vec<u8>> {
    let browse::FileSource { path, mtime, .. } = source(file_id)?;
    let cached = get().thumbs.lock().unwrap().get_nearest(file_id, height, mtime)?;
    if let Some((h, data)) = &cached
        && *h >= height
    {
        return Ok(data.clone());
    }
    let bytes = match read_original(&path) {
        Ok(b) => b,
        Err(e) => return cached.map(|(_, d)| d).ok_or(e).context("original unavailable"),
    };
    let data = with_codecs(|d, e| thumbs::make(&bytes, height as usize, d, e))?;
    get().thumbs.lock().unwrap().put_batch(&[(file_id, height, mtime, data.clone())], None)?;
    Ok(data)
}

/// The ThumbHash placeholder as a tiny RGBA image.
pub fn thumbhash(file_id: i64) -> Result<(usize, usize, Vec<u8>)> {
    let hash = source(file_id)?.thumbhash.context("not imported yet")?;
    thumbhash::thumb_hash_to_rgba(&hash).map_err(|_| anyhow::anyhow!("bad thumbhash"))
}

/// Lanczos3 from a DCT-scaled decode keeping ≥ 4× the target matches a full decode (FINDINGS "Thumbnails").
const DECODE_MARGIN: usize = 4;

/// The original, upright, fitted into `fit` (width, height) with Lanczos3 and never enlarged; `None` for full
/// size.
pub fn full_image(file_id: i64, fit: Option<(usize, usize)>) -> Result<Rgb> {
    let bytes = read_original(&source(file_id)?.path).context("original unavailable")?;
    let min = fit.map_or(MinSize::Full, |(w, h)| MinSize::Fit(w * DECODE_MARGIN, h * DECODE_MARGIN));
    let img = with_codecs(|d, _| d.decode(&bytes, min))?.image;
    Ok(match fit {
        Some((w, h)) => fit_box(&img, w, h).unwrap_or(img),
        None => img,
    })
}
