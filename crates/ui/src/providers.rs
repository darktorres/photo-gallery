//! The QML image providers: `image://thumb/<file id>/<height>`, `image://thumbhash/<file id>` and
//! `image://full/<file id>/<width>x<height>` (or `/full`). A thin C++ `QQuickAsyncImageProvider` (cpp/providers.cpp)
//! queues each request on a thread pool and calls `load_image` here.

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

use ffi::Pixels;

use crate::state;

pub const KIND_THUMB: i32 = 0;
pub const KIND_THUMBHASH: i32 = 1;
pub const KIND_FULL: i32 = 2;

fn failed() -> Pixels {
    Pixels { width: 0, height: 0, format: 0, data: Vec::new() }
}

fn load_image(kind: i32, id: &str) -> Pixels {
    let mut parts = id.split('/');
    let Some(file_id) = parts.next().and_then(|s| s.parse::<i64>().ok()) else { return failed() };
    let arg = parts.next().unwrap_or("");
    let result = match kind {
        KIND_THUMB => arg
            .parse::<u32>()
            .map_err(anyhow::Error::from)
            .and_then(|h| state::thumbnail(file_id, h.clamp(16, 2048)))
            .map(|data| Pixels { width: 0, height: 0, format: 0, data }),
        KIND_THUMBHASH => {
            state::thumbhash(file_id).map(|(w, h, data)| Pixels { width: w as i32, height: h as i32, format: 2, data })
        }
        KIND_FULL => {
            let fit = arg.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
            state::full_image(file_id, fit).map(|img| Pixels {
                width: img.width as i32,
                height: img.height as i32,
                format: 1,
                data: img.pixels,
            })
        }
        _ => return failed(),
    };
    result.unwrap_or_else(|e| {
        eprintln!("image {kind} {id}: {e:#}");
        failed()
    })
}
