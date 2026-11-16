//! Border detection's gate, as data: what a listing uses from it, and the listing without it. The desktop asks these
//! where it reads the preferences (`Prefs::features`); the controller and the keys stay pure.

use pg_model::query::SortKey;
use pg_model::view::Listing;

/// Whether a listing uses border detection: a border facet, the review of the borders, or the sort by border depth.
pub fn uses_borders(listing: &Listing) -> bool {
    !listing.query.borders.is_empty()
        || listing.query.border_marked.is_some()
        || matches!(listing.sort.key, SortKey::Borders)
}

/// The listing without what border detection gives: its border facets and review are dropped, and a sort by border depth
/// falls back to the name order (its direction stays).
pub fn without_borders(mut listing: Listing) -> Listing {
    listing.query.borders.clear();
    listing.query.border_marked = None;
    if matches!(listing.sort.key, SortKey::Borders) {
        listing.sort.key = SortKey::Name;
    }
    listing
}

#[cfg(test)]
mod tests {
    use pg_model::query::{Sort, SortKey};
    use pg_model::view::Listing;

    use super::*;

    #[test]
    fn a_sort_by_border_depth_uses_borders_and_loses_it() {
        let by_depth = Listing { sort: Sort { key: SortKey::Borders, descending: true }, ..Listing::default() };
        assert!(uses_borders(&by_depth));
        let gone = without_borders(by_depth);
        assert!(!uses_borders(&gone));
        assert!(matches!(gone.sort.key, SortKey::Name));
        assert!(gone.sort.descending, "the direction is kept");
    }

    #[test]
    fn a_listing_with_no_border_use_is_left_as_it_is() {
        let plain = Listing::default();
        assert!(!uses_borders(&plain));
        assert!(matches!(without_borders(plain).sort.key, SortKey::Name));
    }
}
