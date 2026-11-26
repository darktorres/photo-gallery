//! Numeric conversions the app computes with: integers to floats (the counts, sizes and timestamps), `u32` to `usize`
//! (indexes and dimensions) and floats to integers. A float-to-integer `as` saturates by the language's rule (below the
//! target's range is its minimum, above it its maximum, NaN is 0, and the fraction is dropped), so a caller clamps only
//! to a narrower range of its own domain, never to the target type's. Each helper is the one `as` for its conversion,
//! and the expect on it states why that conversion is exact or safe, so the call sites carry no expect.

use alloc::format;
use alloc::string::String;

// `usize` holds every `u32`, so `index` never saturates.
const _: () = assert!(usize::BITS >= 32);

/// A `u32` (a dimension, offset or index) as a `usize`. Saturates at `usize::MAX`, which is at least `u32::MAX` on every
/// target, so the saturation is never reached.
pub fn index(x: u32) -> usize {
    usize::try_from(x).unwrap_or(usize::MAX)
}

/// A count held as a `usize` (of items, rows, files or pixels, counted in memory) as the `u64` the notices, the progress
/// counters and the stored counts take. It saturates: a count past `u64`'s range is `u64::MAX`.
pub fn count_u64(x: usize) -> u64 {
    u64::try_from(x).unwrap_or(u64::MAX)
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

/// A byte size, a count of 100 ns ticks, or a pass's count of files, as an `f64`.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "an f64 holds integers exactly up to 2^53 (9 PB of bytes), which no size the app holds reaches"
)]
pub fn size(x: u64) -> f64 {
    x as f64
}

/// A count of instructions or cycles, in millions, as the probes show it: whole millions with an `M` label, so the unit
/// is never implied.
pub fn millions(count: f64) -> String {
    format!("{:.0} M", count / 1e6)
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

/// A float as an `f32`, rounded to the nearest value; one past `f32`'s finite range is an infinity of its sign.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "the cast rounds to the nearest f32 by the language's rule, an infinity past its range"
)]
pub fn f64_to_f32(x: f64) -> f32 {
    x as f32
}

/// A float as a `u8`, its fraction dropped. It saturates at `u8`'s range, and NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a float-to-integer cast saturates by the language's rule, NaN to 0, and drops the fraction"
)]
pub fn f64_to_u8(x: f64) -> u8 {
    x as u8
}

/// A float as a `u16`, its fraction dropped. It saturates at `u16`'s range, and NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a float-to-integer cast saturates by the language's rule, NaN to 0, and drops the fraction"
)]
pub fn f64_to_u16(x: f64) -> u16 {
    x as u16
}

/// A float as a `u32`, its fraction dropped. It saturates at `u32`'s range, and NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a float-to-integer cast saturates by the language's rule, NaN to 0, and drops the fraction"
)]
pub fn f64_to_u32(x: f64) -> u32 {
    x as u32
}

/// A float as a `u64`, its fraction dropped. It saturates at `u64`'s range, and NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a float-to-integer cast saturates by the language's rule, NaN to 0, and drops the fraction"
)]
pub fn f64_to_u64(x: f64) -> u64 {
    x as u64
}

/// A float as a `usize`, its fraction dropped. It saturates at `usize`'s range, and NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a float-to-integer cast saturates by the language's rule, NaN to 0, and drops the fraction"
)]
pub fn f64_to_usize(x: f64) -> usize {
    x as usize
}

/// A float as a count of pixels or lines, its fraction dropped: clamped to `u32`'s range (no picture's side passes it),
/// then as a `usize`. NaN and negatives are 0.
pub fn f64_to_pixels(x: f64) -> usize {
    f64_to_usize(x.clamp(0.0, f64::from(u32::MAX)))
}

/// A float as an `i32`, its fraction dropped. It saturates at `i32`'s range, and NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "a float-to-integer cast saturates by the language's rule, NaN to 0, and drops the fraction"
)]
pub fn f64_to_i32(x: f64) -> i32 {
    x as i32
}

/// A float as an `i64`, its fraction dropped. It saturates at `i64`'s range, and NaN is 0.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "a float-to-integer cast saturates by the language's rule, NaN to 0, and drops the fraction"
)]
pub fn f64_to_i64(x: f64) -> i64 {
    x as i64
}

/// `x` rounded to a whole number, halves away from zero, as `f64::round` does. `round` is not in `core` on a stable
/// toolchain, and this crate is `no_std`. It is exact for every float: from 2^52 up a float is already whole, and below
/// it the truncation to `i64` is exact, so the fraction left over is too. NaN and the infinities come back as they are.
pub fn round_half_away(x: f64) -> f64 {
    const WHOLE: f64 = 4_503_599_627_370_496.0; // 2^52
    if x.is_nan() || x.abs() >= WHOLE {
        return x;
    }
    let truncated = signed(f64_to_i64(x));
    let rounded = match x - truncated {
        fraction if fraction >= 0.5 => truncated + 1.0,
        fraction if fraction <= -0.5 => truncated - 1.0,
        _ => truncated,
    };
    // A zero keeps the sign of `x`, as `f64::round` does: `round(-0.4)` is `-0.0`.
    if rounded == 0.0 { 0.0_f64.copysign(x) } else { rounded }
}
