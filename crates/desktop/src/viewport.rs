//! The viewer's view of the image it shows (`pg_client::viewport`): fitted to the window or zoomed, panned, and the
//! region rendered for a zoomed view. QML keeps the gestures, the keys and the signals; the numbers, the clamping and
//! the image URLs are here. Each change is pushed back as the properties the QML binds to.

pub(crate) mod state;
use state::ViewportRust;

use crate::viewport_bridge::qobject;

use core::pin::Pin;
use cxx_qt::CxxQtType;

use cxx_qt_lib::QString;
use pg_client::viewport::View;
use pg_client::image::ImageRequest;
use pg_model::image::{self, ImageKey};
use pg_model::turns::Turns;

/// A device-pixel count as the model's `usize` (a view's size is never negative).
fn px(n: i64) -> usize {
    usize::try_from(n).unwrap_or(0)
}

/// The `image://full/` URL of `view` of the file `key` names, or empty when the key doesn't read.
pub(crate) fn full_url(key: &QString, view: image::View) -> QString {
    match ImageKey::parse(&key.to_string()) {
        Some(key) => QString::from(&ImageRequest::Full { key, view }.url()),
        None => QString::default(),
    }
}

impl ViewportRust {
    fn of(view: View) -> Self {
        let (fit_width, fit_height) = view.fit_px();
        Self {
            view,
            zoom: view.zoom,
            ox: view.ox,
            oy: view.oy,
            dw: view.dw(),
            dh: view.dh(),
            zoomed: view.zoomed(),
            pixels: view.pixels(),
            fit_width,
            fit_height,
        }
    }
}

impl Default for ViewportRust {
    fn default() -> Self {
        Self::of(View::new())
    }
}

impl qobject::Viewport {
    pub(crate) fn set_media(mut self: Pin<&mut Self>, fw: f64, fh: f64, real_size: bool) -> bool {
        let reset = self.as_mut().rust_mut().view.set_media(fw, fh, real_size);
        self.sync();
        reset
    }

    pub(crate) fn set_still(mut self: Pin<&mut Self>, still: bool) {
        self.as_mut().rust_mut().view.set_still(still);
        self.sync();
    }

    pub(crate) fn set_window(mut self: Pin<&mut Self>, width: f64, height: f64, dpr: f64, real_size: bool) -> bool {
        let reset = self.as_mut().rust_mut().view.set_window(width, height, dpr, real_size);
        self.sync();
        reset
    }

    pub(crate) fn zoom_at(mut self: Pin<&mut Self>, zoom: f64, px: f64, py: f64) {
        self.as_mut().rust_mut().view.set_zoom(zoom, px, py);
        self.sync();
    }

    pub(crate) fn toggle_one_to_one(mut self: Pin<&mut Self>, px: f64, py: f64) {
        self.as_mut().rust_mut().view.toggle_one_to_one(px, py);
        self.sync();
    }

    pub(crate) fn reset_view(mut self: Pin<&mut Self>, real_size: bool) {
        self.as_mut().rust_mut().view.reset(real_size);
        self.sync();
    }

    pub(crate) fn pan(mut self: Pin<&mut Self>, ox: f64, oy: f64) {
        self.as_mut().rust_mut().view.pan(ox, oy);
        self.sync();
    }

    /// Push the view's values to the properties (each setter signals its change).
    fn sync(mut self: Pin<&mut Self>) {
        let p = ViewportRust::of(self.view);
        self.as_mut().set_zoom(p.zoom);
        self.as_mut().set_ox(p.ox);
        self.as_mut().set_oy(p.oy);
        self.as_mut().set_dw(p.dw);
        self.as_mut().set_dh(p.dh);
        self.as_mut().set_zoomed(p.zoomed);
        self.as_mut().set_pixels(p.pixels);
        self.as_mut().set_fit_width(p.fit_width);
        self.as_mut().set_fit_height(p.fit_height);
    }

    pub(crate) fn fit_source(&self, key: &QString, width: i64, height: i64) -> QString {
        full_url(key, image::View::Fit(px(width), px(height)))
    }

    pub(crate) fn region_source(&self, key: &QString) -> QString {
        match self.view.region() {
            Some(r) => full_url(key, image::View::Region((r.x, r.y, r.w, r.h), px(r.px_w), px(r.px_h))),
            None => QString::default(),
        }
    }

    pub(crate) fn swaps_axes(&self, turns: i32) -> bool {
        Turns::of(i64::from(turns)).swaps_axes()
    }

    pub(crate) fn region_x(&self) -> f64 {
        self.view.region().map_or(0.0, |r| r.x)
    }

    pub(crate) fn region_y(&self) -> f64 {
        self.view.region().map_or(0.0, |r| r.y)
    }

    pub(crate) fn region_w(&self) -> f64 {
        self.view.region().map_or(0.0, |r| r.w)
    }

    pub(crate) fn region_h(&self) -> f64 {
        self.view.region().map_or(0.0, |r| r.h)
    }
}
