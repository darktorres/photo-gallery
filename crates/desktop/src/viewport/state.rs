//! The state of the `Viewport` QObject: the bridge's backing struct (the view and its property values).

use pg_client::viewport::View;

/// The view, and the values of its properties (kept in step by `sync`, so the bindings update).
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct ViewportRust {
    pub(super) view: View,
    pub(crate) zoom: f64,
    pub(crate) ox: f64,
    pub(crate) oy: f64,
    pub(crate) dw: f64,
    pub(crate) dh: f64,
    pub(crate) zoomed: bool,
    pub(crate) pixels: bool,
    pub(crate) fit_width: i64,
    pub(crate) fit_height: i64,
}
