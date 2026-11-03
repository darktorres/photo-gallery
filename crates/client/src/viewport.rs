//! The viewer's view of one image: fitted to the window or zoomed, panned, and the part of the image a zoomed view
//! renders as a region. The image's top-left corner is (`ox`, `oy`) in logical pixels; `zoom` is device pixels per
//! image pixel, and `dpr` converts between the two. Only a still image zooms: any other is fitted.

/// The largest zoom: 32 device pixels per image pixel.
pub const MAX_ZOOM: f64 = 32.0;

/// A part of the image, as fractions of it (x, y, w, h) and its size in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub px_w: i64,
    pub px_h: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    fw: f64,
    fh: f64,
    still: bool,
    /// The window, in logical pixels, and the device pixel ratio.
    width: f64,
    height: f64,
    dpr: f64,
    pub zoom: f64,
    pub ox: f64,
    pub oy: f64,
    /// At the fit zoom (it follows window resizes): above 1 for an image smaller than the window, which is enlarged.
    pub fitted: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            fw: 0.0,
            fh: 0.0,
            still: false,
            width: 0.0,
            height: 0.0,
            dpr: 1.0,
            zoom: 1.0,
            ox: 0.0,
            oy: 0.0,
            fitted: true,
        }
    }
}

/// `v` rounded to six places: a region's URL names it, so a pixel of panning doesn't make a new one.
fn six_places(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

impl View {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the image zooms: a still image of known size.
    pub fn zoomable(&self) -> bool {
        self.fw > 0.0 && self.fh > 0.0 && self.still
    }

    /// The fit zoom: the zoom at which the image fills the window (1 when it can't zoom).
    pub fn fit_zoom(&self) -> f64 {
        if self.zoomable() { (self.width * self.dpr / self.fw).min(self.height * self.dpr / self.fh) } else { 1.0 }
    }

    /// The smallest zoom: the fit, or 1:1 when the fit enlarges the image (1:1 is then the furthest zoom out).
    pub fn min_zoom(&self) -> f64 {
        self.fit_zoom().min(1.0)
    }

    /// The image's size on screen, in logical pixels.
    pub fn dw(&self) -> f64 {
        self.fw * self.zoom / self.dpr
    }

    pub fn dh(&self) -> f64 {
        self.fh * self.zoom / self.dpr
    }

    /// Zoomed away from the fit.
    pub fn zoomed(&self) -> bool {
        self.zoomable() && !self.fitted
    }

    /// 1:1 and beyond, away from the fit: the original is shown, nearest neighbour.
    pub fn pixels(&self) -> bool {
        self.zoomable() && self.zoom >= 0.9999 && !self.fitted
    }

    /// The window in device pixels, rounded: the size the fitted image is rendered at.
    pub fn fit_px(&self) -> (i64, i64) {
        ((self.width * self.dpr).round() as i64, (self.height * self.dpr).round() as i64)
    }

    /// Whether the image is still (for zooming) without a size change: its own flag, no reset.
    pub fn set_still(&mut self, still: bool) {
        self.still = still;
    }

    /// A new image of this size. Returns whether the view was reset (it is, when fitted and the size changed).
    pub fn set_media(&mut self, fw: f64, fh: f64, real_size: bool) -> bool {
        let resized = fw != self.fw || fh != self.fh;
        self.fw = fw;
        self.fh = fh;
        if resized && self.fitted {
            self.reset(real_size);
            true
        } else {
            false
        }
    }

    /// A new window size and device pixel ratio. Returns whether the view was reset (when fitted), otherwise it is
    /// kept in place and only clamped to the window.
    pub fn set_window(&mut self, width: f64, height: f64, dpr: f64, real_size: bool) -> bool {
        self.width = width;
        self.height = height;
        self.dpr = dpr;
        if self.fitted {
            self.reset(real_size);
            true
        } else {
            self.clamp();
            false
        }
    }

    /// Keep the image on screen: centred along a side that fits, otherwise no gap at the edges.
    pub fn clamp(&mut self) {
        let (dw, dh) = (self.dw(), self.dh());
        self.ox = if dw <= self.width { (self.width - dw) / 2.0 } else { (self.width - dw).max(self.ox).min(0.0) };
        self.oy = if dh <= self.height { (self.height - dh) / 2.0 } else { (self.height - dh).max(self.oy).min(0.0) };
    }

    /// Pan to the image's corner (`ox`, `oy`), kept on screen.
    pub fn pan(&mut self, ox: f64, oy: f64) {
        self.ox = ox;
        self.oy = oy;
        self.clamp();
    }

    /// Zoom to `zoom` (within the limits) keeping the image point under (`px`, `py`) where it is. Nothing for an
    /// image that does not zoom.
    pub fn set_zoom(&mut self, zoom: f64, px: f64, py: f64) {
        if !self.zoomable() {
            return;
        }
        let zoom = zoom.max(self.min_zoom()).min(MAX_ZOOM);
        let (u, v) = ((px - self.ox) / self.dw(), (py - self.oy) / self.dh());
        self.zoom = zoom;
        let fit = self.fit_zoom();
        self.fitted = (zoom - fit).abs() <= fit * 0.0001;
        self.ox = px - u * self.dw();
        self.oy = py - v * self.dh();
        self.clamp();
    }

    /// Show the image fitted, or at 1:1 in Real Size mode (when that differs from the fit).
    pub fn reset(&mut self, real_size: bool) {
        self.fitted = !(real_size && self.zoomable() && (self.fit_zoom() - 1.0).abs() > 0.0001);
        self.zoom = if self.fitted { self.fit_zoom() } else { 1.0 };
        self.ox = (self.width - self.dw()) / 2.0;
        self.oy = (self.height - self.dh()) / 2.0;
        self.clamp();
    }

    /// Double-click: fit to 1:1 at the point, or 1:1 back to the fit.
    pub fn toggle_one_to_one(&mut self, px: f64, py: f64) {
        let zoom = if self.fitted { 1.0 } else { self.fit_zoom() };
        self.set_zoom(zoom, px, py);
    }

    /// The part of the image on screen, for a zoomed view rendered by Rust. None when none of it shows.
    pub fn region(&self) -> Option<Region> {
        let (dw, dh) = (self.dw(), self.dh());
        let (x0, y0) = (self.ox.max(0.0), self.oy.max(0.0));
        let (x1, y1) = ((self.ox + dw).min(self.width), (self.oy + dh).min(self.height));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(Region {
            x: six_places((x0 - self.ox) / dw),
            y: six_places((y0 - self.oy) / dh),
            w: six_places((x1 - x0) / dw),
            h: six_places((y1 - y0) / dh),
            px_w: ((x1 - x0) * self.dpr).round() as i64,
            px_h: ((y1 - y0) * self.dpr).round() as i64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1000×500 still in a 500×400 window at 1× (fit: 0.5).
    fn view() -> View {
        let mut v = View::new();
        v.set_window(500.0, 400.0, 1.0, false);
        v.set_still(true);
        v.set_media(1000.0, 500.0, false);
        v
    }

    #[test]
    fn opens_fitted_and_centred() {
        let v = view();
        assert!(v.fitted);
        assert_eq!(v.fit_zoom(), 0.5);
        assert_eq!((v.dw(), v.dh()), (500.0, 250.0));
        assert_eq!((v.ox, v.oy), (0.0, 75.0));
        assert!(!v.zoomed());
        assert!(v.region().is_some());
    }

    #[test]
    fn a_small_image_is_enlarged_to_fit_and_its_min_zoom_is_1_to_1() {
        let mut v = View::new();
        v.set_window(500.0, 400.0, 1.0, false);
        v.set_still(true);
        v.set_media(100.0, 50.0, false);
        assert_eq!(v.fit_zoom(), 5.0);
        assert_eq!(v.min_zoom(), 1.0);
    }

    #[test]
    fn real_size_opens_at_one_to_one_when_that_differs_from_the_fit() {
        let mut v = view();
        v.reset(true);
        assert!(!v.fitted);
        assert_eq!(v.zoom, 1.0);
        assert!(v.zoomed());
        assert!(v.pixels(), "1:1 away from the fit shows the original's pixels");
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut v = view();
        v.set_zoom(2.0, 250.0, 200.0);
        assert!(v.zoomed());
        // The image point under the cursor (x = 0.5 of the 500 px fitted width) stays there after the zoom.
        let after = (250.0 - v.ox) / v.dw();
        assert!((0.5 - after).abs() < 1e-9);
    }

    #[test]
    fn zoom_is_limited_to_the_fit_and_to_32() {
        let mut v = view();
        v.set_zoom(0.01, 0.0, 0.0);
        assert_eq!(v.zoom, v.min_zoom());
        v.set_zoom(1000.0, 0.0, 0.0);
        assert_eq!(v.zoom, MAX_ZOOM);
    }

    #[test]
    fn pan_stays_on_screen_and_centres_a_side_that_fits() {
        let mut v = view();
        v.set_zoom(2.0, 0.0, 0.0);
        v.pan(100.0, 100.0);
        assert_eq!(v.ox, 0.0, "no gap at the left edge");
        assert_eq!(v.oy, 0.0, "no gap at the top edge");
        v.pan(-10_000.0, 0.0);
        assert_eq!(v.ox, v.width - v.dw());
        // The fitted view is centred, whatever the pan asked.
        v.reset(false);
        v.pan(40.0, 40.0);
        assert_eq!(v.ox, 0.0);
        assert_eq!(v.oy, 75.0);
    }

    #[test]
    fn a_resize_resets_a_fitted_view_and_only_clamps_a_zoomed_one() {
        let mut v = view();
        assert!(v.set_window(800.0, 400.0, 1.0, false));
        assert_eq!(v.fit_zoom(), 0.8);
        v.set_zoom(2.0, 400.0, 200.0);
        assert!(!v.set_window(800.0, 300.0, 1.0, false));
        assert!(!v.fitted);
    }

    #[test]
    fn a_new_still_flag_does_not_reset_but_stops_the_zoom() {
        let mut v = view();
        v.set_zoom(2.0, 0.0, 0.0);
        assert!(!v.fitted);
        v.set_still(false);
        assert!(!v.fitted, "only a new size resets the view");
        assert!(!v.zoomable());
        assert!(!v.zoomed(), "a view that cannot zoom is never zoomed");
    }

    #[test]
    fn region_is_the_visible_part_as_fractions() {
        let mut v = view();
        v.set_zoom(2.0, 250.0, 200.0);
        let r = v.region().expect("some of the image shows");
        assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= 1.0 + 1e-6 && r.y + r.h <= 1.0 + 1e-6);
        assert_eq!(r.px_w, (r.w * v.fw * v.zoom).round() as i64);

        assert_eq!(v.fit_px(), (500, 400));
    }

    #[test]
    fn toggle_goes_to_one_to_one_and_back() {
        let mut v = view();
        v.toggle_one_to_one(250.0, 200.0);
        assert_eq!(v.zoom, 1.0);
        assert!(!v.fitted);
        v.toggle_one_to_one(250.0, 200.0);
        assert!(v.fitted);
        assert_eq!(v.zoom, 0.5);
    }
}
