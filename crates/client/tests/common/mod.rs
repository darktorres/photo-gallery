//! The fixtures the client's controller tests share. Each test file that uses them includes this module with `mod common;`
//! (the controller's sub-tests reach it as `super::common`).

use pg_client::controller::{Controller, Start};
use pg_model::name::Name;
use pg_model::rules::Merged;
use pg_model::size::RowHeight;
use pg_model::view::Listing;

/// The row height the fixtures use: a shown row of 200 and a saved one of 200.
pub(crate) const ROW_200: RowHeight = RowHeight::new(200).expect("a row height in range");

/// A controller at the start's row height (200) and the prefs' saved one (the same), starting at `listing`.
pub(crate) fn controller(listing: Listing) -> Controller {
    Controller::new(Start::new(listing, ROW_200, ROW_200))
}

/// A folder given as its path text, one part per `\`-separated component.
pub(crate) fn folder(text: &str) -> Merged {
    Merged::from(text.split('\\').filter(|p| !p.is_empty()).map(Name::from).collect::<Vec<_>>())
}
