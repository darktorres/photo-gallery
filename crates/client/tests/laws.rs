//! The laws of the justified layout (S-18.2), over generated aspects, widths, heights and spacings: the rows cover
//! every item once and in order, every row fits the width, the rows stay at or below the target height, and a closed
//! row fills the width exactly (`justify_into` scales it so).

use pg_client::layout::{Row, clamp_aspect, justify_into};
use proptest::prelude::*;

fn rows_of(aspects: &[f64], width: f64, target: f64, spacing: f64) -> Vec<Row> {
    let mut rows = Vec::new();
    justify_into(&mut rows, aspects.iter().copied(), width, target, spacing);
    rows
}

/// Aspect ratios as the images give them: thin strips to wide panoramas, with the odd zero or non-finite value (which
/// `clamp_aspect` lays out as a square or as the nearest limit).
fn aspects() -> impl Strategy<Value = Vec<f64>> {
    prop::collection::vec(
        prop_oneof![
            18 => 0.01f64..100.0,
            1 => Just(0.0),
            1 => Just(f64::NAN),
            1 => Just(f64::INFINITY),
        ],
        0..200,
    )
}

proptest! {
    #[test]
    fn rows_cover_every_item_once_and_in_order(
        aspects in aspects(),
        width in 1.0f64..4000.0,
        target in 1.0f64..800.0,
        spacing in 0.0f64..20.0,
    ) {
        let rows = rows_of(&aspects, width, target, spacing);
        let mut next = 0;
        for row in &rows {
            prop_assert_eq!(row.first, next);
            prop_assert!(row.count >= 1);
            next += row.count;
        }
        prop_assert_eq!(next, aspects.len());
    }

    #[test]
    fn rows_fit_the_width_and_stay_at_or_below_the_target(
        aspects in aspects(),
        width in 1.0f64..4000.0,
        target in 1.0f64..800.0,
        spacing in 0.0f64..20.0,
    ) {
        let rows = rows_of(&aspects, width, target, spacing);
        for (k, row) in rows.iter().enumerate() {
            let sum: f64 = aspects[row.first..row.first + row.count].iter().map(|&a| clamp_aspect(a)).sum();
            let used = sum * row.height + spacing * pg_model::num::count(row.count - 1);
            prop_assert!(row.height <= target + 1e-9, "row {k} is {} px, above the target {target}", row.height);
            // A row is at least 1 px high (`justify_into`'s floor), and at that floor it can be wider than the width: a
            // single item too wide for a 1 px row. Above the floor, the row fits.
            let floored = row.height <= 1.0 + 1e-9;
            if !floored {
                prop_assert!(used <= width + 1e-6 * width, "row {k} overflows: {used} > {width}");
            }
            // Every row but the last is closed, and a closed row is scaled to fill the width exactly.
            if k + 1 < rows.len() && !floored {
                prop_assert!(
                    (used - width).abs() <= 1e-6 * width,
                    "closed row {k} does not fill the width: {used} vs {width}"
                );
            }
        }
    }

    #[test]
    fn clamped_aspects_are_in_range_and_stable(aspect in prop::num::f64::ANY) {
        let clamped = clamp_aspect(aspect);
        prop_assert!((0.05..=20.0).contains(&clamped), "{clamped} is outside 1:20 to 20:1");
        prop_assert_eq!(clamp_aspect(clamped), clamped);
    }
}
