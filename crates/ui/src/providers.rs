//! The QML image providers. A file is named by `<file id>@<mtime>.<rotation>.<turns>` (`GalleryModel::imageKey`):
//! the version changes whenever the image does, so Qt's pixmap cache never serves a stale one, and `turns` are the
//! quarter turns the user asked for that aren't in the file yet: the image is served already turned by them, so
//! every view shows a turn at once.
//! - `image://thumb/<file>/<height>`
//! - `image://thumbhash/<file>`
//! - `image://full/<file>/<width>x<height>` (fitted), `…/full`, or `…/region/<x>,<y>,<w>,<h>/<width>x<height>` (the
//!   part of the image on screen while zoomed between fit and 1:1, `x`…`h` as fractions of the upright image).
//!
//! A thin C++ `QQuickAsyncImageProvider` (cpp/providers.cpp) queues each request on a thread pool and calls
//! `load_image` here.

#[cxx_qt::bridge]
pub mod ffi {
    /// An image for the C++ side: `format` 0 = encoded file bytes (JPEG), 1 = RGB888, 2 = RGBA8888. Empty `data`
    /// means it failed.
    #[namespace = "pg"]
    struct Pixels {
        width: i32,
        height: i32,
        format: i32,
        data: Vec<u8>,
    }

    extern "Rust" {
        #[namespace = "pg"]
        fn load_image(kind: i32, id: &str) -> Pixels;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlimageproviderbase.h");
        type QQmlImageProviderBase = cxx_qt_lib::QQmlImageProviderBase;

        include!("pg-ui/cpp/providers.h");
        /// A new provider of `kind` (`KIND_*`); the engine takes ownership.
        #[namespace = "pg"]
        fn new_image_provider(kind: i32) -> *mut QQmlImageProviderBase;
    }
}

use anyhow::Context;
use ffi::Pixels;

use crate::state;

pub const KIND_THUMB: i32 = 0;
pub const KIND_THUMBHASH: i32 = 1;
pub const KIND_FULL: i32 = 2;

fn failed() -> Pixels {
    Pixels { width: 0, height: 0, format: 0, data: Vec::new() }
}

fn size(s: &str) -> Option<(usize, usize)> {
    let (w, h) = s.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn full_view(parts: &[&str]) -> Option<state::View> {
    match parts {
        ["full"] => Some(state::View::Full),
        [fit] => size(fit).map(|(w, h)| state::View::Fit(w, h)),
        ["region", r, out] => {
            let r: Vec<f64> = r.split(',').map(str::parse).collect::<Result<_, _>>().ok()?;
            let &[x, y, w, h] = r.as_slice() else { return None };
            let (ow, oh) = size(out)?;
            let ok = (0.0..1.0).contains(&x) && (0.0..1.0).contains(&y) && w > 0.0 && h > 0.0;
            ok.then_some(state::View::Region((x, y, w.min(1.0 - x), h.min(1.0 - y)), ow.min(16384), oh.min(16384)))
        }
        _ => None,
    }
}

fn load_image(kind: i32, id: &str) -> Pixels {
    let parts: Vec<&str> = id.split('/').collect();
    let (file, version) = parts[0].split_once('@').unwrap_or((parts[0], ""));
    let Ok(file_id) = file.parse::<i64>() else { return failed() };
    // Only the pending turns are read from the version: the catalog says what the file is now.
    let turns = version.split('.').nth(2).and_then(|t| t.parse::<u8>().ok()).unwrap_or(0) % 4;
    let rgb = |img: pg_core::decode::Rgb| Pixels {
        width: img.width as i32,
        height: img.height as i32,
        format: 1,
        data: img.pixels,
    };
    let result = match kind {
        KIND_THUMB => parts
            .get(1)
            .and_then(|h| h.parse::<u32>().ok())
            .context("bad height")
            .and_then(|h| state::thumbnail(file_id, h.clamp(16, 2048)))
            .and_then(|jpeg| {
                if turns == 0 {
                    Ok(Pixels { width: 0, height: 0, format: 0, data: jpeg })
                } else {
                    state::turn_jpeg(&jpeg, turns).map(rgb)
                }
            }),
        KIND_THUMBHASH => state::thumbhash(file_id, turns).map(|(w, h, data)| Pixels {
            width: w as i32,
            height: h as i32,
            format: 2,
            data,
        }),
        KIND_FULL => match full_view(&parts[1..]) {
            Some(view) => state::full_image(file_id, view, turns).map(rgb),
            None => return failed(),
        },
        _ => return failed(),
    };
    result.unwrap_or_else(|e| {
        eprintln!("image {kind} {id}: {e:#}");
        failed()
    })
}
