//! The detector's thresholds, and the version that marks what they detect.

/// Bump when anything here changes what is detected; detections of an older version are redone.
pub const BORDERS_VERSION: i64 = 1;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Max per-channel distance from the line colour that still counts as border.
    pub tol: i32,
    /// Max fraction of a solid line's pixels beyond `tol`.
    pub solid_off: f64,
    /// A blurred line has less detail than this fraction of the image's central detail…
    pub blur_rel: f64,
    /// …or than this mean gradient (gray levels per pixel).
    pub blur_abs: f64,
    /// Below this central detail, an image is smooth overall and blurred bars aren't looked for.
    pub min_content_act: f64,
    /// Thinner borders are ignored.
    pub min_px: usize,
    /// A border covers at most this fraction of a side.
    pub max_frac: f64,
    /// The content keeps at least this fraction of each dimension.
    pub min_keep: f64,
    /// Min fraction of the seam that must be a sharp straight edge…
    pub straight: f64,
    /// …for blurred bars, which have no colour edge to rely on.
    pub blur_straight: f64,
    /// Blurred bars on opposite sides may differ in depth by this fraction.
    pub blur_sym: f64,
    /// Per-pixel change that marks a column as crossing the seam.
    pub seam_step: i32,
    /// Real padding is digitally flat away from the seam; a wall or sky has shading. Max median pixel deviation of
    /// a one-sided solid border…
    pub flat_dev: f64,
    /// …of a solid border when a side is detected opposite it (of any kind; `judge` checks `other.is_some()`)…
    pub flat_dev_paired: f64,
    /// …and max drift of the line colour across the border's depth (JPEG noise moves pixels, not line medians).
    pub max_drift: f64,
    /// Extra pixels removed past each seam (JPEG chroma blending at the edge).
    pub margin: usize,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            tol: 24,
            solid_off: 0.02,
            blur_rel: 0.15,
            blur_abs: 1.0,
            min_content_act: 3.0,
            min_px: 3,
            max_frac: 0.45,
            min_keep: 0.3,
            straight: 0.5,
            blur_straight: 0.7,
            blur_sym: 0.1,
            seam_step: 12,
            flat_dev: 4.0,
            flat_dev_paired: 8.0,
            max_drift: 8.0,
            margin: 1,
        }
    }
}
