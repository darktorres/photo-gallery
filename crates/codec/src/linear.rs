//! sRGB and linear light (IEC 61966-2-1). The resizer averages in linear light: averaging the encoded values darkens
//! high-contrast detail. Two tables, built once and read by every worker: the
//! 256 decodes of an 8-bit sRGB sample, and the 65,536 encodes of a linear value on a 16-bit grid, each rounded once
//! to 8 bits. Encoding through the 16-bit grid is exact to the rounding of the 8-bit output.

use std::sync::OnceLock;

/// The sRGB electro-optical transfer function: an encoded value in 0–1 to a linear one.
fn eotf(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Its inverse: a linear value in 0–1 to an encoded one (clamped to 0–1 first).
fn oetf(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// The 16-bit encode grid's last index.
const GRID_MAX: usize = 65_535;

struct Tables {
    /// `decode[v]` is the linear value of the 8-bit sRGB sample `v`.
    decode: Vec<f32>,
    /// `encode[k]` is the 8-bit sRGB sample of the linear value `k / 65535`.
    encode: Vec<u8>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let decode = (0..=255usize).map(|v| pg_model::num::f64_to_f32(eotf(pg_model::num::count(v) / 255.0))).collect();
        let encode = (0..=GRID_MAX)
            .map(|k| pg_model::num::f64_to_u8((oetf(pg_model::num::count(k) / 65_535.0) * 255.0).round()))
            .collect();
        Tables { decode, encode }
    })
}

/// The linear value (0–1) of an 8-bit sRGB sample.
pub(crate) fn decode(v: u8) -> f32 {
    tables().decode.get(usize::from(v)).copied().unwrap_or(0.0)
}

/// The 8-bit sRGB sample of a linear value, clamped to 0–1 and rounded once.
pub(crate) fn encode(v: f32) -> u8 {
    let k = pg_model::num::f64_to_usize(f64::from(v.clamp(0.0, 1.0)) * pg_model::num::count(GRID_MAX) + 0.5);
    tables().encode.get(k).copied().unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_8_bit_sample_survives_a_round_trip() {
        for v in 0..=u8::MAX {
            assert_eq!(encode(decode(v)), v, "sample {v}");
        }
    }

    #[test]
    fn the_ends_and_the_midpoint_are_where_srgb_puts_them() {
        assert_eq!(decode(0), 0.0);
        assert!((decode(u8::MAX) - 1.0).abs() < 1e-6);
        // Half the light is the sample 188 (linear 0.5 encodes to 0.7354 of 255), not 128.
        assert_eq!(encode(0.5), 188);
        assert_eq!(encode(-1.0), 0);
        assert_eq!(encode(2.0), u8::MAX);
    }
}
