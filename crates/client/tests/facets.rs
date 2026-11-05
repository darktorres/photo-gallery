//! The list facets' indices, which the filter bar's constants (FilterBar.qml, `facet…`) name: pinned here.

use pg_client::facets::Facet;

#[test]
fn the_facets_keep_their_indices() {
    assert_eq!(Facet::ALL.map(Facet::key), ["types", "orientations", "status", "borders", "ratings", "exts"]);
    for (i, facet) in Facet::ALL.into_iter().enumerate() {
        assert_eq!(Facet::from_index(i as i32), Some(facet));
    }
    assert_eq!(Facet::from_index(6), None);
    assert_eq!(Facet::from_index(-1), None);
}
