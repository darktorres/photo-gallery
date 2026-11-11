//! The list facets' indices, which the filter bar's constants (FilterBar.qml, `facet…`) name: pinned here.

use pg_client::facets::{ALL, from_index};
use pg_model::query::ListFacet;

#[test]
fn the_facets_keep_their_indices() {
    assert_eq!(
        ALL,
        [
            ListFacet::Types,
            ListFacet::Shapes,
            ListFacet::Status,
            ListFacet::Borders,
            ListFacet::Ratings,
            ListFacet::Exts
        ]
    );
    for (i, facet) in ALL.into_iter().enumerate() {
        assert_eq!(from_index(i), Some(facet));
    }
    assert_eq!(from_index(6), None);
}
