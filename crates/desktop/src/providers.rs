//! The QML image providers. A file is named by `<file id>@<mtime>.<rotation>.<turns>` (`GalleryModel::image_key`, parsed
//! as `pg_model::image::ImageKey`): the version changes whenever the image does, so Qt's pixmap cache never serves a
//! stale one, and `turns` are the quarter turns the user asked for that aren't in the file yet: the image is served
//! already turned by them, so every view shows a turn at once. Each image comes from one view of the file (`pg_app::App`'s
//! image methods): a thumbnail the cache keeps for the file as the key names it, from the key alone; anything else from
//! the catalog alone, turned by its own turns. A request racing a written turn then shows the file turned once, either
//! way, and the model's refresh changes the key.
//! The URLs are `pg_client::image`'s, which builds and reads them; a provider is registered for each of its
//! `Provider`s (lib.rs), and its requests come here as that provider's `ImageKind`.
//!
//! A thin C++ `QQuickAsyncImageProvider` (cpp/providers.cpp) queues each request on a thread pool and calls
//! `load_image` here. A thumbnail is decoded here, by Qt (`QImage::fromData`, as an `Image` would), from the
//! bytes where they are: a cached one in the thread's buffer (`JPEG`), kept for the next one; one made on a miss in
//! its own, which then moves to the cache's writer. Neither is copied.

use crate::providers_bridge::ffi;

use std::cell::RefCell;

use anyhow::Context;
use cxx_qt_lib::QImage;
use ffi::{ImageKind, PixelFormat, Pixels};

use pg_client::image::{ImageRequest, Provider};
use pg_model::image::ImageKey;
use pg_model::size::ThumbHeight;
use pg_model::turns::Turns;
use pg_render::display::Rgba;

use crate::app;

thread_local! {
    /// The provider thread's buffer for cached thumbnails' bytes, kept from one to the next.
    static JPEG: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn failed() -> Pixels {
    Pixels { width: 0, height: 0, format: PixelFormat::Image, data: Vec::new(), image: QImage::default() }
}

/// A thumbnail `height` px high of the key's file (`App::thumbnail`), decoded, or turned by its turns. One made on a
/// miss moves to the cache's writer once decoded. A cached one that doesn't decode is dropped from the cache, so that
/// the next pass at its height makes it anew from the original.
fn thumbnail(key: ImageKey, height: ThumbHeight) -> anyhow::Result<Pixels> {
    let app = app::get();
    let file = key.file;
    JPEG.with_borrow_mut(|jpeg| {
        let thumb = app.images().thumbnail(key, height, jpeg)?;
        let bytes = thumb.made.as_ref().map_or(&jpeg[..], |(made, _)| made);
        let pixels = if thumb.turns == Turns::NONE {
            QImage::from_data(bytes, None).context("not an image Qt reads").map(|image| Pixels {
                width: 0,
                height: 0,
                format: PixelFormat::Image,
                data: Vec::new(),
                image,
            })
        } else {
            app.images().turn_jpeg(bytes, thumb.turns).map_err(anyhow::Error::from).and_then(upload)
        };
        if let Some((made, keep)) = thumb.made {
            app.images().keep(keep, made);
        } else if let Some(at) = thumb.kept_at
            && pixels.is_err()
            && let Err(e) = app.images().forget_thumb(file, at)
        {
            pg_log::warn("providers", &format!("thumbnail {} at {at} px: dropping it failed: {e:#}", file.get()));
        }
        pixels
    })
}

/// `img` as a texture Qt's scene graph uploads as it is (`QSGPlainTexture`): RGBX8888 for an opaque image, RGBA8888
/// premultiplied otherwise (any other format it converts on the render thread, a copy at every upload). The
/// premultiplication rounds as Qt's own (`qPremultiply`). An error when a side is past what Qt's int sizes hold.
fn upload(img: Rgba) -> anyhow::Result<Pixels> {
    Ok(Pixels {
        width: i32::try_from(img.width).context("image too wide for Qt")?,
        height: i32::try_from(img.height).context("image too high for Qt")?,
        format: if img.opaque { PixelFormat::Rgbx8888 } else { PixelFormat::Rgba8888Premultiplied },
        data: img.pixels,
        image: QImage::default(),
    })
}

/// The bridge's kind of the provider `provider`: the one map between the two.
pub(crate) fn kind_of(provider: Provider) -> ImageKind {
    match provider {
        Provider::Thumb => ImageKind::Thumb,
        Provider::Thumbhash => ImageKind::Thumbhash,
        Provider::Full => ImageKind::Full,
        Provider::Yandex => ImageKind::Yandex,
    }
}

/// A Yandex result's picture, by its id: the grid picture (the JPEG the search made of its file), or the file as its
/// host gave it (the preview's). Each is fetched now when the search doesn't keep it yet.
fn yandex_picture(id: u64, original: bool) -> anyhow::Result<Pixels> {
    let picture = crate::app::get().yandex().image(id)?;
    let bytes = if original { &picture.file.bytes } else { &picture.grid };
    let image = QImage::from_data(bytes, None).context("not an image Qt reads")?;
    Ok(Pixels { width: 0, height: 0, format: PixelFormat::Image, data: Vec::new(), image })
}

/// The image a request for `kind` and `id` (the URL's part after the provider's name, `pg_client::image`) asks for.
pub(crate) fn load_image(kind: ImageKind, id: &str) -> Pixels {
    let provider = Provider::ALL.into_iter().find(|&p| kind_of(p) == kind);
    let Some(request) = provider.and_then(|provider| ImageRequest::parse(provider, id)) else {
        pg_log::warn("providers", &format!("image {kind:?} {id}: not a request"));
        return failed();
    };
    let result = match request {
        ImageRequest::Thumb { key, height } => thumbnail(key, height),
        ImageRequest::Thumbhash { key } => {
            app::get().images().placeholder(key.file).map_err(anyhow::Error::from).and_then(upload)
        }
        ImageRequest::Full { key, view } => {
            app::get().images().full_image(key.file, view).map_err(anyhow::Error::from).and_then(upload)
        }
        ImageRequest::Yandex { id, original } => yandex_picture(id, original),
    };
    result.unwrap_or_else(|e| {
        pg_log::warn("providers", &format!("image {kind:?} {id}: {e:#}"));
        failed()
    })
}
