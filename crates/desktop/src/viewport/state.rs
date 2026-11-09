//! The state of the `Viewport` QObject: the bridge's backing struct (the view and its property values).

use pg_client::viewport::View;

/// The view, and the values of its properties (kept in step by `sync`, so the bindings update).
pub(super) struct ViewportRust {
    pub(super) view: View,
    pub(super) zoom: f64,
    pub(super) ox: f64,
    pub(super) oy: f64,
    pub(super) dw: f64,
    pub(super) dh: f64,
    pub(super) zoomed: bool,
    pub(super) pixels: bool,
    pub(super) fit_width: i64,
    pub(super) fit_height: i64,
}
