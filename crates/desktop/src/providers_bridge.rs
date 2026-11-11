//! The cxx-qt bridge of [`crate::providers`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::providers::load_image;

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
        /// A Yandex result's picture: its grid picture (`image://yandex/<result id>`), or its file as the host gave it
        /// (`image://yandex/<result id>/original`).
        Yandex,
    }

    /// How a `Pixels` holds its image: `Image` is a `QImage` (a decoded thumbnail, in `image`); the others are raw
    /// pixels in `data` (`pg_render::display::Upload`), which Qt takes as they are, without a copy.
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
