//! The QML image providers. A file is named by `<file id>@<mtime>.<rotation>.<turns>` (`GalleryModel::image_key`, parsed
//! as `pg_model::image::ImageKey`): the version changes whenever the image does, so Qt's pixmap cache never serves a
//! stale one, and `turns` are the quarter turns the user asked for that aren't in the file yet: the image is served
//! already turned by them, so every view shows a turn at once. Each image comes from one view of the file (`pg_app::App`'s
//! image methods): a thumbnail the cache keeps for the file as the key names it, from the key alone; anything else from
//! the catalog alone, turned by its own turns. A request racing a written turn then shows the file turned once, either
//! way, and the model's refresh changes the key.
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
    /// Which provider a request is for: the `image://` name it is registered under (lib.rs).
    #[namespace = "pg"]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum ImageKind {
        Thumb,
        Thumbhash,
        Full,
    }

    /// How a `Pixels` holds its image: `Image` is a `QImage` (a decoded thumbnail, in `image`); the others are raw
    /// pixels in `data` (`pg_app::Upload`), which Qt takes as they are, without a copy.
    #[namespace = "pg"]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum PixelFormat {
        Image,
        Rgbx8888,
        Rgba8888Premultiplied,
    }

    /// An image for the C++ side, in `format`. A null `image` or an empty `data` means it failed.
    #[namespace = "pg"]
    struct Pixels {
        width: i32,
        height: i32,
        format: PixelFormat,
        data: Vec<u8>,
        image: QImage,
    }

    extern "Rust" {
        #[namespace = "pg"]
        fn load_image(kind: ImageKind, id: &str) -> Pixels;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qimage.h");
        type QImage = cxx_qt_lib::QImage;

        include!("cxx-qt-lib/qqmlimageproviderbase.h");
        type QQmlImageProviderBase = cxx_qt_lib::QQmlImageProviderBase;

        include!("pg-desktop/cpp/providers.h");
        /// A new provider of `kind`; the engine takes ownership.
        #[namespace = "pg"]
        fn new_image_provider(kind: ImageKind) -> *mut QQmlImageProviderBase;
    }
}

use std::cell::RefCell;

use anyhow::Context;
use cxx_qt_lib::QImage;
use ffi::{ImageKind, PixelFormat, Pixels};

use pg_app::Upload;
use pg_model::image::{ImageKey, ImageRequest};
use pg_model::turns::Turns;

use crate::app;

thread_local! {
    /// The provider thread's buffer for cached thumbnails' bytes, kept from one to the next.
    static JPEG: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn failed() -> Pixels {
    Pixels { width: 0, height: 0, format: PixelFormat::Image, data: Vec::new(), image: QImage::default() }
}

/// A thumbnail `height` px high of the key's file (`App::thumbnail`), decoded, or turned by its turns. One made on a
/// miss moves to the cache's writer once decoded.
fn thumbnail(key: ImageKey, height: u32) -> anyhow::Result<Pixels> {
    let app = app::get();
    JPEG.with_borrow_mut(|jpeg| {
        let thumb = app.thumbnail(key, height, jpeg)?;
        let bytes = thumb.made.as_ref().map_or(&jpeg[..], |(made, _)| made);
        let pixels = if thumb.turns == Turns::NONE {
            let image = QImage::from_data(bytes, None).context("not an image Qt reads")?;
            Ok(Pixels { width: 0, height: 0, format: PixelFormat::Image, data: Vec::new(), image })
        } else {
            app.turn_jpeg(bytes, thumb.turns).map_err(anyhow::Error::from).and_then(upload)
        };
        if let Some((made, keep)) = thumb.made {
            app.keep(keep, made);
        }
        pixels
    })
}

/// `img` for Qt; an error when a side is past what Qt's int sizes hold.
fn upload(img: Upload) -> anyhow::Result<Pixels> {
    Ok(Pixels {
        width: i32::try_from(img.width).context("image too wide for Qt")?,
        height: i32::try_from(img.height).context("image too high for Qt")?,
        format: if img.premultiplied { PixelFormat::Rgba8888Premultiplied } else { PixelFormat::Rgbx8888 },
        data: img.pixels,
        image: QImage::default(),
    })
}

/// The `image://` name `kind` is registered under.
fn provider(kind: ImageKind) -> &'static str {
    match kind {
        ImageKind::Thumb => "thumb",
        ImageKind::Thumbhash => "thumbhash",
        ImageKind::Full => "full",
        _ => "",
    }
}

/// The image a request for `kind` and `id` (the URL's part after the provider's name, `pg_model::image`) asks for.
fn load_image(kind: ImageKind, id: &str) -> Pixels {
    let Some(request) = ImageRequest::parse(provider(kind), id) else {
        crate::panic_log::note(&format!("image {kind:?} {id}: not a request"));
        return failed();
    };
    let result = match request {
        ImageRequest::Thumb { key, height } => thumbnail(key, height),
        ImageRequest::Thumbhash { key } => {
            app::get().placeholder(key.file).map_err(anyhow::Error::from).and_then(upload)
        }
        ImageRequest::Full { key, view } => {
            app::get().full_image(key.file, view).map_err(anyhow::Error::from).and_then(upload)
        }
    };
    result.unwrap_or_else(|e| {
        crate::panic_log::note(&format!("image {kind:?} {id}: {e:#}"));
        failed()
    })
}
