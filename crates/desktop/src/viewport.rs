//! The viewer's view of the image it shows (`pg_client::viewport`): fitted to the window or zoomed, panned, and the
//! region rendered for a zoomed view. QML keeps the gestures, the keys and the signals; the numbers, the clamping and
//! the image URLs are here. Each change is pushed back as the properties the QML binds to.

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
        /// At the fit zoom (follows the window's size).
        #[qproperty(bool, fitted, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, fit_zoom, cxx_name = "fitZoom", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, min_zoom, cxx_name = "minZoom", READ, WRITE, NOTIFY, FINAL)]
        /// The image's size on screen, in logical pixels.
        #[qproperty(f64, dw, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(f64, dh, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, zoomed, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(bool, pixels, READ, WRITE, NOTIFY, FINAL)]
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

        /// The image (by its key) fitted to the window, at device pixels: its URL.
        #[qinvokable]
        #[cxx_name = "fitSource"]
        fn fit_source(self: &Viewport, key: &QString) -> QString;

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

use core::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use pg_client::viewport::View;
use pg_model::orientation::turns_swap;
use pg_model::turns::Turns;

/// The view, and the values of its properties (kept in step by `sync`, so the bindings update).
pub struct ViewportRust {
    view: View,
    zoom: f64,
    ox: f64,
    oy: f64,
    fitted: bool,
    fit_zoom: f64,
    min_zoom: f64,
    dw: f64,
    dh: f64,
    zoomed: bool,
    pixels: bool,
}

impl ViewportRust {
    fn of(view: View) -> Self {
        Self {
            view,
            zoom: view.zoom,
            ox: view.ox,
            oy: view.oy,
            fitted: view.fitted,
            fit_zoom: view.fit_zoom(),
            min_zoom: view.min_zoom(),
            dw: view.dw(),
            dh: view.dh(),
            zoomed: view.zoomed(),
            pixels: view.pixels(),
        }
    }
}

impl Default for ViewportRust {
    fn default() -> Self {
        Self::of(View::new())
    }
}

impl qobject::Viewport {
    fn set_media(mut self: Pin<&mut Self>, fw: f64, fh: f64, real_size: bool) -> bool {
        let reset = self.as_mut().rust_mut().view.set_media(fw, fh, real_size);
        self.sync();
        reset
    }

    fn set_still(mut self: Pin<&mut Self>, still: bool) {
        self.as_mut().rust_mut().view.set_still(still);
        self.sync();
    }

    fn set_window(mut self: Pin<&mut Self>, width: f64, height: f64, dpr: f64, real_size: bool) -> bool {
        let reset = self.as_mut().rust_mut().view.set_window(width, height, dpr, real_size);
        self.sync();
        reset
    }

    fn zoom_at(mut self: Pin<&mut Self>, zoom: f64, px: f64, py: f64) {
        self.as_mut().rust_mut().view.set_zoom(zoom, px, py);
        self.sync();
    }

    fn toggle_one_to_one(mut self: Pin<&mut Self>, px: f64, py: f64) {
        self.as_mut().rust_mut().view.toggle_one_to_one(px, py);
        self.sync();
    }

    fn reset_view(mut self: Pin<&mut Self>, real_size: bool) {
        self.as_mut().rust_mut().view.reset(real_size);
        self.sync();
    }

    fn pan(mut self: Pin<&mut Self>, ox: f64, oy: f64) {
        self.as_mut().rust_mut().view.pan(ox, oy);
        self.sync();
    }

    /// Push the view's values to the properties (each setter signals its change).
    fn sync(mut self: Pin<&mut Self>) {
        let p = ViewportRust::of(self.view);
        self.as_mut().set_zoom(p.zoom);
        self.as_mut().set_ox(p.ox);
        self.as_mut().set_oy(p.oy);
        self.as_mut().set_fitted(p.fitted);
        self.as_mut().set_fit_zoom(p.fit_zoom);
        self.as_mut().set_min_zoom(p.min_zoom);
        self.as_mut().set_dw(p.dw);
        self.as_mut().set_dh(p.dh);
        self.as_mut().set_zoomed(p.zoomed);
        self.as_mut().set_pixels(p.pixels);
    }

    fn fit_source(&self, key: &QString) -> QString {
        let key = key.to_string();
        if key.is_empty() {
            return QString::default();
        }
        let (w, h) = self.view.fit_px();
        QString::from(&format!("image://full/{key}/{w}x{h}"))
    }

    fn region_source(&self, key: &QString) -> QString {
        let key = key.to_string();
        match self.view.region() {
            Some(r) if !key.is_empty() => QString::from(&format!(
                "image://full/{key}/region/{},{},{},{}/{}x{}",
                r.x, r.y, r.w, r.h, r.px_w, r.px_h
            )),
            _ => QString::default(),
        }
    }

    fn swaps_axes(&self, turns: i32) -> bool {
        turns_swap(Turns::new(u8::try_from(turns.rem_euclid(4)).unwrap_or(0)))
    }

    fn region_x(&self) -> f64 {
        self.view.region().map_or(0.0, |r| r.x)
    }

    fn region_y(&self) -> f64 {
        self.view.region().map_or(0.0, |r| r.y)
    }

    fn region_w(&self) -> f64 {
        self.view.region().map_or(0.0, |r| r.w)
    }

    fn region_h(&self) -> f64 {
        self.view.region().map_or(0.0, |r| r.h)
    }
}
