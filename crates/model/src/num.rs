//! Numeric conversions the app computes with: integers to floats (the counts, sizes and timestamps), `u32` to `usize`
//! (indexes and dimensions) and floats to integers (after the caller clamps them to the target's range). Each helper is
//! the one `as` for its conversion, and the expect on it states why that conversion is exact or in range, so the call
//! sites carry no expect.

// `usize` holds every `u32`, so `index` never saturates.
const _: () = assert!(usize::BITS >= 32);

/// A `u32` (a dimension, offset or index) as a `usize`. Saturates at `usize::MAX`, which is at least `u32::MAX` on every
/// target, so the saturation is never reached.
pub fn index(x: u32) -> usize {
    usize::try_from(x).unwrap_or(usize::MAX)
}

/// A count (of items, pixels, rows or lines) as an `f64`.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "an f64 holds integers exactly up to 2^53, which no count the app holds reaches"
)]
pub fn count(x: usize) -> f64 {
    x as f64
}

/// A byte size, or a count of 100 ns ticks, as an `f64`.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "an f64 holds integers exactly up to 2^53 (9 PB of bytes), which no size the app holds reaches"
)]
pub fn size(x: u64) -> f64 {
    x as f64
}

/// A signed value, such as a timestamp in milliseconds, as an `f64`.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "an f64 holds integers exactly up to 2^53, which no timestamp or signed count the app holds reaches"
)]
pub fn signed(x: i64) -> f64 {
    x as f64
}

/// A float the caller has clamped to `f32`'s finite range, as an `f32`, rounded to the nearest value.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "the caller clamps to f32's finite range, so the cast only rounds"
)]
pub fn f64_to_f32(x: f64) -> f32 {
    x as f32
}

/// A float the caller has clamped to `u8`'s range, as a `u8`. NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller clamps to u8's range, so the cast only drops the fraction; NaN is 0"
)]
pub fn f64_to_u8(x: f64) -> u8 {
    x as u8
}

/// A float the caller has clamped to `u16`'s range, as a `u16`. NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller clamps to u16's range, so the cast only drops the fraction; NaN is 0"
)]
pub fn f64_to_u16(x: f64) -> u16 {
    x as u16
}

/// A float the caller has clamped to `u32`'s range, as a `u32`. NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller clamps to u32's range, so the cast only drops the fraction; NaN is 0"
)]
pub fn f64_to_u32(x: f64) -> u32 {
    x as u32
}

/// A float the caller has clamped to `u64`'s range, as a `u64`. NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller clamps to u64's range, so the cast only drops the fraction; NaN is 0"
)]
pub fn f64_to_u64(x: f64) -> u64 {
    x as u64
}

/// A float the caller has clamped to `usize`'s range, as a `usize`. NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller clamps to usize's range, so the cast only drops the fraction; NaN is 0"
)]
pub fn f64_to_usize(x: f64) -> usize {
    x as usize
}

/// A float the caller has clamped to `i32`'s range, as an `i32`. NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "the caller clamps to i32's range, so the cast only drops the fraction; NaN is 0"
)]
pub fn f64_to_i32(x: f64) -> i32 {
    x as i32
}

/// A float the caller has clamped to `i64`'s range, as an `i64`. NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "the caller clamps to i64's range, so the cast only drops the fraction; NaN is 0"
)]
pub fn f64_to_i64(x: f64) -> i64 {
    x as i64
}
