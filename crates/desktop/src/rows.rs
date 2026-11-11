//! Qt's int convention at the bridges: a row, an item or a choice is an `int`, -1 (any negative) for none. The crates
//! below take `Option<usize>`; these two convert at the bridge, one each way.

/// A row as Qt gives it: `None` for -1 (or any negative).
pub(crate) fn row_of(row: i32) -> Option<usize> {
    usize::try_from(row).ok()
}

/// A row as Qt takes it: -1 for none, saturating at `i32::MAX`.
pub(crate) fn to_row(row: Option<usize>) -> i32 {
    row.map_or(-1, |row| i32::try_from(row).unwrap_or(i32::MAX))
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
}
