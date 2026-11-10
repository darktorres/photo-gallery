//! The cxx-qt bridge of [`crate::viewport`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::viewport::state::ViewportRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// Device pixels per image pixel.
        #[qproperty(f64, zoom, READ, WRITE, NOTIFY, FINAL)]
        /// The image's top-left corner, in logical pixels.
        #[qproperty(f64, ox, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, oy, READ, WRITE, NOTIFY, FINAL)]
        /// The image's size on screen, in logical pixels.
        #[qproperty(f64, dw, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, dh, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, zoomed, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, pixels, READ, WRITE, NOTIFY, FINAL)]
        /// The fitted image's size, in device pixels (the window's size times the device pixel ratio). The viewer
        /// renders the fit at it once it has settled after a resize (`fitSource` takes the size it settled on).
        #[qproperty(i64, fit_width, cxx_name = "fitWidth", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i64, fit_height, cxx_name = "fitHeight", READ, WRITE, NOTIFY, FINAL)]
        type Viewport = super::ViewportRust;
    }

    unsafe extern "RustQt" {
        /// A new image of this size (in image pixels). Returns whether the view was reset (fitted, and the size
        /// changed): then the caller also signals the reset.
        #[qinvokable]
        #[cxx_name = "setMedia"]
        fn set_media(self: Pin<&mut Viewport>, fw: f64, fh: f64, real_size: bool) -> bool;

        /// Whether the image can zoom (a still image). Changing it doesn't reset the view.
        #[qinvokable]
        #[cxx_name = "setStill"]
        fn set_still(self: Pin<&mut Viewport>, still: bool);

        /// The window's size (logical pixels) and the device pixel ratio. Returns whether the view was reset.
        #[qinvokable]
        #[cxx_name = "setWindow"]
        fn set_window(self: Pin<&mut Viewport>, width: f64, height: f64, dpr: f64, real_size: bool) -> bool;

        /// Zoom to `zoom`, keeping the image point under (px, py) where it is.
        #[qinvokable]
        #[cxx_name = "setZoom"]
        fn zoom_at(self: Pin<&mut Viewport>, zoom: f64, px: f64, py: f64);

        /// Double-click: fit ↔ 1:1 at the point.
        #[qinvokable]
        #[cxx_name = "toggleOneToOne"]
        fn toggle_one_to_one(self: Pin<&mut Viewport>, px: f64, py: f64);

        /// Show the image fitted, or at 1:1 in Real Size mode.
        #[qinvokable]
        #[cxx_name = "resetView"]
        fn reset_view(self: Pin<&mut Viewport>, real_size: bool);

        /// Pan the image's corner to (ox, oy), kept on screen.
        #[qinvokable]
        fn pan(self: Pin<&mut Viewport>, ox: f64, oy: f64);

        /// The image (by its key) fitted to `width` × `height` device pixels (the `fitWidth` and `fitHeight` the
        /// viewer settled on): its URL. The size is an argument, so a QML binding to the URL follows it.
        #[qinvokable]
        #[cxx_name = "fitSource"]
        fn fit_source(self: &Viewport, key: &QString, width: i64, height: i64) -> QString;

        /// The region of the image on screen, rendered at the zoom: its URL, or "" when none of it shows. Its
        /// fractions are `regionX`, `regionY`, `regionW` and `regionH`.
        #[qinvokable]
        #[cxx_name = "regionSource"]
        fn region_source(self: &Viewport, key: &QString) -> QString;

        /// Whether a count of quarter turns (player turns, 0 to 3) swaps the picture's width and height.
        #[qinvokable]
        #[cxx_name = "swapsAxes"]
        fn swaps_axes(self: &Viewport, turns: i32) -> bool;

        #[qinvokable]
        #[cxx_name = "regionX"]
        fn region_x(self: &Viewport) -> f64;

        #[qinvokable]
        #[cxx_name = "regionY"]
        fn region_y(self: &Viewport) -> f64;

        #[qinvokable]
        #[cxx_name = "regionW"]
        fn region_w(self: &Viewport) -> f64;

        #[qinvokable]
        #[cxx_name = "regionH"]
        fn region_h(self: &Viewport) -> f64;
    }
}
