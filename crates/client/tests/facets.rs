//! The list facets' order, which the filter bar shows them in: pinned here.

use pg_client::facets::ALL;
use pg_model::query::ListFacet;

#[test]
fn the_facets_keep_the_bars_order() {
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
}
