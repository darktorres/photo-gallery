//! The grid's size values. Each type is valid by construction: a constructor refuses what is out of its range,
//! and serde refuses it on read too, so a recording or a stored value can't hold a zero scale or a 5000-pixel thumbnail.
//! A float that is NaN or infinite is never a size, so every constructor refuses it.
use core::ops::RangeInclusive;
use serde::{Deserialize, Serialize};

use crate::num::{f64_to_u32, round_half_away};

/// The row heights the grid takes (pixels).
pub const ROW_HEIGHT: RangeInclusive<u32> = 60..=800;
/// The thumbnail heights the thumbnail cache holds (pixels).
pub const THUMB_HEIGHT: RangeInclusive<u32> = 16..=2048;

/// A row height in pixels, within [`ROW_HEIGHT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct RowHeight(u32);

impl RowHeight {
    /// The smallest row height.
    pub const MIN: RowHeight = RowHeight(*ROW_HEIGHT.start());
    /// The largest row height.
    pub const MAX: RowHeight = RowHeight(*ROW_HEIGHT.end());
    /// The row height a fresh window shows (the prefs' default).
    pub const DEFAULT: RowHeight = RowHeight(220);

    /// The row height `px`, or `None` if it is outside [`ROW_HEIGHT`].
    pub const fn new(px: u32) -> Option<RowHeight> {
        if px >= *ROW_HEIGHT.start() && px <= *ROW_HEIGHT.end() { Some(RowHeight(px)) } else { None }
    }

    /// A finite height rounded to the nearest pixel and clamped into [`ROW_HEIGHT`]; `None` for NaN or an infinity.
    pub fn from_finite(px: f64) -> Option<RowHeight> {
        px.is_finite().then(|| RowHeight(f64_to_u32(round_half_away(px)).clamp(RowHeight::MIN.0, RowHeight::MAX.0)))
    }

    /// The height in pixels.
    pub fn get(self) -> u32 {
        self.0
    }
}

/// Why a row height is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowHeightError {
    /// The pixel count is outside [`ROW_HEIGHT`].
    OutOfRange(u32),
}

impl core::fmt::Display for RowHeightError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfRange(px) => write!(f, "row height {px} is outside {ROW_HEIGHT:?}"),
        }
    }
}

impl core::error::Error for RowHeightError {}

impl TryFrom<u32> for RowHeight {
    type Error = RowHeightError;
    fn try_from(px: u32) -> Result<RowHeight, RowHeightError> {
        RowHeight::new(px).ok_or(RowHeightError::OutOfRange(px))
    }
}

impl core::fmt::Display for RowHeight {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<RowHeight> for u32 {
    fn from(row: RowHeight) -> u32 {
        row.0
    }
}

/// A thumbnail height in pixels, within [`THUMB_HEIGHT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct ThumbHeight(u32);

impl ThumbHeight {
    /// The thumbnail height of a fresh window's row: the default row height (220) at scale 1.
    pub const DEFAULT: ThumbHeight = ThumbHeight(RowHeight::DEFAULT.0);

    /// The thumbnail height `px`, or `None` if it is outside [`THUMB_HEIGHT`].
    pub fn new(px: u32) -> Option<ThumbHeight> {
        THUMB_HEIGHT.contains(&px).then_some(ThumbHeight(px))
    }

    /// The thumbnail height for a row height at a scale: the product rounded to a pixel, saturated into [`THUMB_HEIGHT`].
    pub fn for_row(row: RowHeight, scale: Scale) -> ThumbHeight {
        let px = f64::from(row.0) * scale.0;
        ThumbHeight(f64_to_u32(round_half_away(px)).clamp(*THUMB_HEIGHT.start(), *THUMB_HEIGHT.end()))
    }

    /// The height in pixels.
    pub fn get(self) -> u32 {
        self.0
    }
}

/// Why a thumbnail height is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbHeightError {
    /// The pixel count is outside [`THUMB_HEIGHT`].
    OutOfRange(u32),
}

impl core::fmt::Display for ThumbHeightError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfRange(px) => write!(f, "thumbnail height {px} is outside {THUMB_HEIGHT:?}"),
        }
    }
}

impl core::error::Error for ThumbHeightError {}

impl TryFrom<u32> for ThumbHeight {
    type Error = ThumbHeightError;
    fn try_from(px: u32) -> Result<ThumbHeight, ThumbHeightError> {
        ThumbHeight::new(px).ok_or(ThumbHeightError::OutOfRange(px))
    }
}

impl core::fmt::Display for ThumbHeight {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<ThumbHeight> for u32 {
    fn from(thumb: ThumbHeight) -> u32 {
        thumb.0
    }
}

/// Why a float is refused as a positive finite number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PositiveError {
    /// The value is NaN, an infinity, zero or negative.
    NotPositive(f64),
}

impl core::fmt::Display for PositiveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotPositive(v) => write!(f, "{v} is not a positive finite number"),
        }
    }
}

impl core::error::Error for PositiveError {}

/// A positive finite number: the scale, a factor, or a width. Each wrapper below adds its own name.
const fn positive(v: f64) -> Result<f64, PositiveError> {
    if v.is_finite() && v > 0.0 { Ok(v) } else { Err(PositiveError::NotPositive(v)) }
}

/// Declares a positive finite float newtype: its constructor, getter and conversions are the same for each, and
/// serde refuses a value out of range on read too.
macro_rules! positive_float {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(try_from = "f64", into = "f64")]
        pub struct $name(f64);

        impl $name {
            /// The value `v`, or `None` if it is not positive and finite.
            pub const fn new(v: f64) -> Option<$name> {
                match positive(v) {
                    Ok(v) => Some($name(v)),
                    Err(_) => None,
                }
            }

            /// The value as a number.
            pub fn get(self) -> f64 {
                self.0
            }
        }

        impl TryFrom<f64> for $name {
            type Error = PositiveError;
            fn try_from(v: f64) -> Result<$name, PositiveError> {
                positive(v).map($name)
            }
        }

        impl From<$name> for f64 {
            fn from(value: $name) -> f64 {
                value.0
            }
        }
    };
}

positive_float! {
    /// A screen's scale (its device pixel ratio): positive and finite.
    Scale
}

positive_float! {
    /// A multiplier of the row height (a pinch or a wheel step): positive and finite.
    Factor
}

positive_float! {
    /// The grid's width in pixels: positive and finite. A width of zero or less is not a width (the list has none yet), so it
    /// is `None` at the shell, not a value here.
    GridWidth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_height_bounds_and_rounding() {
        assert_eq!(RowHeight::from_finite(333.5).map(RowHeight::get), Some(334));
        assert_eq!(RowHeight::from_finite(5000.0).map(RowHeight::get), Some(800));
        assert_eq!(RowHeight::from_finite(-40.0).map(RowHeight::get), Some(60));
        assert_eq!(RowHeight::from_finite(f64::NAN), None);
        assert_eq!(RowHeight::from_finite(f64::INFINITY), None);
        assert_eq!(RowHeight::new(59), None);
        assert_eq!(RowHeight::new(801), None);
    }

    #[test]
    fn thumb_height_saturates_at_both_ends() {
        let scale = |v: f64| Scale::new(v).expect("a positive scale");
        assert_eq!(ThumbHeight::for_row(RowHeight::MAX, scale(3.0)).get(), 2048);
        assert_eq!(ThumbHeight::for_row(RowHeight::MIN, scale(0.25)).get(), 16);
        assert_eq!(ThumbHeight::for_row(RowHeight::new(220).unwrap(), scale(1.0)).get(), 220);
        assert_eq!(ThumbHeight::for_row(RowHeight::new(250).unwrap(), scale(1.5)).get(), 375);
    }

    #[test]
    fn positive_wrappers_refuse_bad_floats() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
            assert!(Scale::new(bad).is_none(), "scale {bad}");
            assert!(Factor::new(bad).is_none(), "factor {bad}");
            assert!(GridWidth::new(bad).is_none(), "width {bad}");
        }
        assert!(Scale::new(2.0).is_some());
    }

    #[test]
    fn serde_refuses_out_of_range_on_read() {
        assert!(serde_json::from_str::<RowHeight>("5000").is_err());
        assert!(serde_json::from_str::<ThumbHeight>("5000").is_err());
        assert!(serde_json::from_str::<Scale>("0.0").is_err());
        assert!(serde_json::from_str::<GridWidth>("-24.0").is_err());
        assert_eq!(serde_json::from_str::<RowHeight>("220").unwrap().get(), 220);
        assert_eq!(serde_json::to_string(&Scale::new(1.5).unwrap()).unwrap(), "1.5");
    }
}
