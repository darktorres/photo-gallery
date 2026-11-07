//! Laws of the container walks' byte search (S-18.2): `find` is where a naive search first finds the needle, and
//! `range_in` gives back the place of a slice of the bytes. The needle is never empty: `find` windows over it.

use pg_container::{find, range_in};
use proptest::prelude::*;

/// The first position of `needle` in `haystack`, by trying each position.
fn naive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (0..haystack.len()).find(|&i| haystack.get(i..).is_some_and(|rest| rest.starts_with(needle)))
}

proptest! {
    // A small alphabet makes matches and partial matches common.
    #[test]
    fn find_is_where_a_naive_search_finds_it(
        haystack in prop::collection::vec(0u8..3, 0..40),
        needle in prop::collection::vec(0u8..3, 1..4),
    ) {
        prop_assert_eq!(find(&haystack, &needle), naive(&haystack, &needle));
    }

    // A slice of the bytes is found where it is.
    #[test]
    fn a_slice_is_at_its_own_range(
        bytes in prop::collection::vec(any::<u8>(), 1..40),
        (start, end) in (0usize..40, 0usize..40).prop_map(|(a, b)| (a.min(b), a.max(b))),
    ) {
        let end = end.min(bytes.len());
        let start = start.min(end);
        prop_assert_eq!(range_in(&bytes, &bytes[start..end]), start..end);
    }
}
