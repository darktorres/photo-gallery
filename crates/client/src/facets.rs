//! The list facets of the filter bar, typed. The desktop's FacetChoices and Filters take a facet by its index in
//! `Facet::ALL` (an `i32` at the QML boundary, where the bar names each one by a constant); the order is pinned by a test.

/// A list facet: the choices of one field, each sent to the bar as a key (`input::key_of`; `input::choice` reads it back)
/// and named by `text::choice_label`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facet {
    Types,
    Orientations,
    Status,
    Borders,
    Ratings,
    Exts,
}

impl Facet {
    /// Every facet, in the filter bar's index order.
    pub const ALL: [Facet; 6] =
        [Facet::Types, Facet::Orientations, Facet::Status, Facet::Borders, Facet::Ratings, Facet::Exts];

    /// The facet at `index` (its position in `ALL`); `None` out of range.
    pub fn from_index(index: usize) -> Option<Facet> {
        Self::ALL.get(index).copied()
    }

    /// The facet's name in the query's terms.
    pub fn key(self) -> &'static str {
        match self {
            Facet::Types => "types",
            Facet::Orientations => "orientations",
            Facet::Status => "status",
            Facet::Borders => "borders",
            Facet::Ratings => "ratings",
            Facet::Exts => "exts",
        }
    }
}
