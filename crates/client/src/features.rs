//! Border detection's gate, as data: what a listing uses from it, and the listing without it. The desktop asks these
//! where it reads the preferences (`Prefs::features`); the controller and the keys stay pure.

use pg_model::features::Features;
use pg_model::query::{Query, Sort, SortKey};
use pg_model::view::Listing;

use crate::keys::GridKey;

/// Whether a grid action is refused: the borders' B and N, while border detection is off. Every other action is allowed.
pub fn grid_refusal(grid: GridKey, features: Features) -> bool {
    !features.borders && matches!(grid, GridKey::Borders | GridKey::MarkBorder)
}

/// Whether a query and a sort use border detection: a border facet, the review of the borders, or the sort by border depth.
pub fn uses_borders_in(query: &Query, sort: &Sort) -> bool {
    !query.borders.is_empty() || query.border_marked.is_some() || matches!(sort.key, SortKey::Borders)
}

/// Whether a listing uses border detection (see [`uses_borders_in`]).
pub fn uses_borders(listing: &Listing) -> bool {
    uses_borders_in(&listing.query, &listing.sort)
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
    use alloc::vec;

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

    #[test]
    fn a_border_facet_or_review_is_a_border_use_and_the_name_sort_is_not() {
        let facet = Query { borders: vec![pg_model::query::BorderFacet::Black], ..Query::default() };
        assert!(uses_borders_in(&facet, &Sort::default()));
        let review = Query { border_marked: Some(true), ..Query::default() };
        assert!(uses_borders_in(&review, &Sort::default()));
        assert!(!uses_borders_in(&Query::default(), &Sort::default()));
    }

    #[test]
    fn the_borders_keys_are_refused_only_while_borders_are_off() {
        let off = Features { borders: false, duplicates: true };
        let on = Features::ALL;
        assert!(grid_refusal(GridKey::Borders, off));
        assert!(grid_refusal(GridKey::MarkBorder, off));
        assert!(!grid_refusal(GridKey::Borders, on));
        assert!(!grid_refusal(GridKey::MarkBorder, on));
        for grid in [GridKey::Filters, GridKey::Rate(3), GridKey::Step { dx: 1, dy: 0 }, GridKey::Open] {
            assert!(!grid_refusal(grid, off), "{grid:?} is not a borders key");
        }
    }
}
