//! The laws of the aspect clamp the justified layout lays items out by, over generated aspects. The layout's own laws
//! (every item in a row once and in order, rows fitting the width, closed rows filling it) are in `layout.rs`, where
//! `justify_into` is crate-private.

use pg_client::layout::clamp_aspect;
use proptest::prelude::*;

proptest! {
    #[test]
    fn clamped_aspects_are_in_range_and_stable(aspect in prop::num::f64::ANY) {
        let clamped = clamp_aspect(aspect);
        prop_assert!((0.05..=20.0).contains(&clamped), "{clamped} is outside 1:20 to 20:1");
        prop_assert_eq!(clamp_aspect(clamped), clamped);
    }
}
