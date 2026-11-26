//! Qt's number conventions at the bridges: a row, an item or a choice is an `int`, -1 (any negative) for none; a file id
//! is a `real` (QML holds it exactly to 2^53, where an `int` would truncate it). The crates below take `Option<usize>` and
//! `FileId`; these convert at the bridge, one each way.

use pg_model::ids::FileId;

/// The largest whole number a QML `real` holds exactly (2^53): the largest file id that crosses the bridge.
const EXACT_REAL: f64 = 9_007_199_254_740_992.0;

/// A file id as QML gives it, a `real`. `None` for a value that is not a whole number from 0 to 2^53, which no catalog
/// file has, so the caller shows nothing for it.
pub(crate) fn file_id_of(value: f64) -> Option<FileId> {
    if value.fract() == 0.0 && (0.0..=EXACT_REAL).contains(&value) {
        Some(FileId::new(pg_model::num::f64_to_i64(value)))
    } else {
        None
    }
}

/// A row as Qt gives it: `None` for -1 (or any negative).
pub(crate) fn row_of(row: i32) -> Option<usize> {
    usize::try_from(row).ok()
}

/// A row as Qt takes it: -1 for none, saturating at `i32::MAX`.
pub(crate) fn to_row(row: Option<usize>) -> i32 {
    row.map_or(-1, qt_int)
}

/// A count, a row or a side in pixels as Qt's int, saturating: a Qt model holds no more rows than an int counts.
pub(crate) fn qt_int(n: impl TryInto<i32>) -> i32 {
    n.try_into().unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_goes_both_ways_and_none_is_minus_one() {
        assert_eq!(row_of(-1), None);
        assert_eq!(row_of(-7), None);
        assert_eq!(row_of(3), Some(3));
        assert_eq!(to_row(None), -1);
        assert_eq!(to_row(Some(3)), 3);
        assert_eq!(to_row(Some(usize::MAX)), i32::MAX);
    }

    #[test]
    fn a_file_id_is_a_whole_real_from_zero_to_two_to_the_53() {
        assert_eq!(file_id_of(42.0), Some(FileId::new(42)));
        assert_eq!(file_id_of(EXACT_REAL), Some(FileId::new(1 << 53)));
        assert_eq!(file_id_of(1.5), None);
        assert_eq!(file_id_of(-1.0), None);
        assert_eq!(file_id_of(f64::NAN), None);
        assert_eq!(file_id_of(f64::INFINITY), None);
    }
}
