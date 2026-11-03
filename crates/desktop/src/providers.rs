//! The QML image providers. A file is named by `<file id>@<mtime>.<rotation>.<turns>` (`GalleryModel::imageKey`):
//! the version changes whenever the image does, so Qt's pixmap cache never serves a stale one, and `turns` are the
//! quarter turns the user asked for that aren't in the file yet: the image is served already turned by them, so
//! every view shows a turn at once. Each image comes from one view of the file (`state`): a thumbnail the cache
//! keeps for the file as the key names it, from the key alone; anything else from the catalog alone, turned by
//! its own turns. A request racing a written turn then shows the file turned once, either way, and the model's
//! refresh changes the key.
//! - `image://thumb/<file>/<height>`
//! - `image://thumbhash/<file>`
//! - `image://full/<file>/<width>x<height>` (fitted), `…/full`, or `…/region/<x>,<y>,<w>,<h>/<width>x<height>` (the
//!   part of the image on screen while zoomed between fit and 1:1, `x`…`h` as fractions of the upright image).
//!
//! A thin C++ `QQuickAsyncImageProvider` (cpp/providers.cpp) queues each request on a thread pool and calls
//! `load_image` here. A thumbnail is decoded here, by Qt (`QImage::fromData`, as an `Image` would), from the
//! bytes where they are: a cached one in the thread's buffer (`JPEG`), kept for the next one; one made on a miss in
//! its own, which then moves to the cache's writer. Neither is copied.

#[cxx_qt::bridge]
pub mod ffi {
    /// An image for the C++ side: `format` 0 = `image`, already a `QImage` (a decoded thumbnail); 1 = RGBX8888,
    /// 2 = RGBA8888 premultiplied (`state::Upload`) in `data`, which Qt takes as it is, without a copy. A null
    /// `image` or an empty `data` means it failed.
    #[namespace = "pg"]
    struct Pixels {
        width: i32,
        height: i32,
        format: i32,
        data: Vec<u8>,
        image: QImage,
    }

    extern "Rust" {
        #[namespace = "pg"]
        fn load_image(kind: i32, id: &str) -> Pixels;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qimage.h");
        type QImage = cxx_qt_lib::QImage;

        include!("cxx-qt-lib/qqmlimageproviderbase.h");
        type QQmlImageProviderBase = cxx_qt_lib::QQmlImageProviderBase;

        include!("pg-desktop/cpp/providers.h");
        /// A new provider of `kind` (`KIND_*`); the engine takes ownership.
        #[namespace = "pg"]
        fn new_image_provider(kind: i32) -> *mut QQmlImageProviderBase;
    }
}

use std::cell::RefCell;

use anyhow::Context;
use cxx_qt_lib::QImage;
use ffi::Pixels;

use crate::state;

pub const KIND_THUMB: i32 = 0;
pub const KIND_THUMBHASH: i32 = 1;
pub const KIND_FULL: i32 = 2;

thread_local! {
    /// The provider thread's buffer for cached thumbnails' bytes, kept from one to the next.
    static JPEG: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn failed() -> Pixels {
    Pixels { width: 0, height: 0, format: 0, data: Vec::new(), image: QImage::default() }
}

/// A thumbnail `height` px high of `file_id` (`state::thumbnail`), decoded, or turned by its turns. One made on a
/// miss moves to the cache's writer once decoded.
fn thumbnail(file_id: i64, height: u32, named: Option<state::Named>) -> anyhow::Result<Pixels> {
    JPEG.with_borrow_mut(|jpeg| {
        let thumb = state::thumbnail(file_id, height, named, jpeg)?;
        let bytes = thumb.made.as_ref().map_or(&jpeg[..], |(made, _)| made);
        let pixels = if thumb.turns == 0 {
            let image = QImage::from_data(bytes, None).context("not an image Qt reads")?;
            Ok(Pixels { width: 0, height: 0, format: 0, data: Vec::new(), image })
        } else {
            state::turn_jpeg(bytes, thumb.turns).map(upload)
        };
        if let Some((made, keep)) = thumb.made {
            keep.put(made);
        }
        pixels
    })
}

fn upload(img: state::Upload) -> Pixels {
    Pixels {
        width: img.width as i32,
        height: img.height as i32,
        format: if img.premultiplied { 2 } else { 1 },
        data: img.pixels,
        image: QImage::default(),
    }
}

fn size(s: &str) -> Option<(usize, usize)> {
    let (w, h) = s.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// What follows the key in `image://full/…`.
fn full_view(rest: &str) -> Option<state::View> {
    if rest == "full" {
        return Some(state::View::Full);
    }
    let Some(region) = rest.strip_prefix("region/") else { return size(rest).map(|(w, h)| state::View::Fit(w, h)) };
    let (r, out) = region.split_once('/')?;
    let mut r = r.split(',').map(str::parse::<f64>);
    let (Some(Ok(x)), Some(Ok(y)), Some(Ok(w)), Some(Ok(h)), None) = (r.next(), r.next(), r.next(), r.next(), r.next())
    else {
        return None;
    };
    let (ow, oh) = size(out)?;
    let ok = (0.0..1.0).contains(&x) && (0.0..1.0).contains(&y) && w > 0.0 && h > 0.0;
    ok.then_some(state::View::Region((x, y, w.min(1.0 - x), h.min(1.0 - y)), ow.min(16384), oh.min(16384)))
}

/// A key's file, and what its version names (`None` without a version that reads).
fn parse_key(key: &str) -> Option<(i64, Option<state::Named>)> {
    let (file, version) = key.split_once('@').map_or((key, None), |(f, v)| (f, Some(v)));
    let named = version.and_then(|v| {
        let mut parts = v.split('.');
        let mtime = parts.next()?.parse().ok()?;
        let rotation = parts.next()?.parse::<u8>().ok()? % 4;
        let turns = parts.next()?.parse::<u8>().ok()? % 4;
        Some(state::Named { source: pg_thumbcache::Source { mtime, rotation }, turns })
    });
    Some((file.parse().ok()?, named))
}

fn load_image(kind: i32, id: &str) -> Pixels {
    let (key, rest) = id.split_once('/').unwrap_or((id, ""));
    let Some((file_id, named)) = parse_key(key) else { return failed() };
    let result = match kind {
        KIND_THUMB => {
            rest.parse::<u32>().context("bad height").and_then(|h| thumbnail(file_id, h.clamp(16, 2048), named))
        }
        KIND_THUMBHASH => state::placeholder(file_id).map(upload),
        KIND_FULL => match full_view(rest) {
            Some(view) => state::full_image(file_id, view).map(upload),
            None => return failed(),
        },
        _ => return failed(),
    };
    result.unwrap_or_else(|e| {
        eprintln!("image {kind} {id}: {e:#}");
        failed()
    })
}
