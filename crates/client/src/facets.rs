//! The list facets of the filter bar (`pg_model::query::ListFacet`): which choices each offers, in the bar's order, with
//! the counts the bar shows beside them. The desktop's FacetChoices and Filters take a facet by its index in [`ALL`]
//! (Qt's int at the bridge, where the bar names each one by a constant); the order is pinned by a test. Each choice is
//! sent to the bar as a key (`input::key_of`; `input::choice` reads it back) and named by `text::choice_label`.

use pg_model::media::MediaType;
use pg_model::query::{BorderFacet, Choice, ListFacet, Query, RatingFilter, Shape, Status};

/// Every list facet, in the filter bar's index order.
pub const ALL: [ListFacet; 6] =
    [ListFacet::Types, ListFacet::Shapes, ListFacet::Status, ListFacet::Borders, ListFacet::Ratings, ListFacet::Exts];

/// The facet at `index` (its position in [`ALL`]); `None` out of range.
pub fn from_index(index: usize) -> Option<ListFacet> {
    ALL.get(index).copied()
}

/// The counts the list facets show: a media type's and a borders value's, and the folder's extensions in the order the
/// bar shows them (most files first, then any chosen one the folder doesn't have, at 0, so it can be unchosen).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub types: Vec<(MediaType, i64)>,
    pub borders: Vec<(BorderFacet, i64)>,
    pub exts: Vec<(String, i64)>,
}

impl Counts {
    /// The counts of a folder (its types', borders' and extensions' counts, as the catalog gives them) for `query`: the
    /// extensions the query chose that the folder lacks are added after the folder's, at 0.
    pub fn new(
        types: Vec<(MediaType, i64)>,
        borders: Vec<(BorderFacet, i64)>,
        folder_exts: &[(String, i64)],
        query: &Query,
    ) -> Counts {
        let mut exts = folder_exts.to_vec();
        exts.extend(query.exts.iter().filter(|e| !folder_exts.iter().any(|(x, _)| x == *e)).map(|e| (e.clone(), 0)));
        Counts { types, borders, exts }
    }
}

/// The choices of a list facet, in the bar's order, each with its count (`None` where the bar shows none).
pub fn choices(facet: ListFacet, counts: &Counts) -> Vec<(Choice, Option<i64>)> {
    let media = |t: MediaType| counts.types.iter().find(|(x, _)| *x == t).map_or(0, |(_, n)| *n);
    let border = |b: BorderFacet| counts.borders.iter().find(|(x, _)| *x == b).map_or(0, |(_, n)| *n);
    match facet {
        ListFacet::Types => [MediaType::Image, MediaType::Animated, MediaType::Video]
            .map(|t| (Choice::Type(t), Some(media(t))))
            .to_vec(),
        ListFacet::Shapes => {
            [Shape::Landscape, Shape::Portrait, Shape::Square].map(|s| (Choice::Shape(s), None)).to_vec()
        }
        ListFacet::Status => {
            [Status::Imported, Status::Pending, Status::Failed].map(|s| (Choice::Status(s), None)).to_vec()
        }
        ListFacet::Borders => {
            [BorderFacet::None, BorderFacet::Black, BorderFacet::White, BorderFacet::Color, BorderFacet::Blur]
                .map(|b| (Choice::Border(b), Some(border(b))))
                .to_vec()
        }
        ListFacet::Ratings => {
            (0u8..6).filter_map(|n| RatingFilter::try_from(n).ok()).map(|r| (Choice::Rating(r), None)).collect()
        }
        ListFacet::Exts => counts.exts.iter().map(|(e, n)| (Choice::Ext(e.clone()), Some(*n))).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_extension_the_folder_lacks_follows_its_own_at_zero() {
        let query = Query { exts: vec!["png".into(), "heic".into()], ..Query::default() };
        let counts = Counts::new(Vec::new(), Vec::new(), &[("jpg".into(), 9), ("png".into(), 2)], &query);
        assert_eq!(counts.exts, [("jpg".into(), 9), ("png".into(), 2), ("heic".into(), 0)]);
    }

    #[test]
    fn a_facet_offers_its_choices_in_the_bars_order_with_their_counts() {
        let counts = Counts { types: vec![(MediaType::Video, 3)], ..Counts::default() };
        let types = choices(ListFacet::Types, &counts);
        assert_eq!(
            types,
            [
                (Choice::Type(MediaType::Image), Some(0)),
                (Choice::Type(MediaType::Animated), Some(0)),
                (Choice::Type(MediaType::Video), Some(3)),
            ]
        );
        assert!(choices(ListFacet::Shapes, &counts).iter().all(|(_, n)| n.is_none()), "shapes show no count");
        assert_eq!(choices(ListFacet::Ratings, &counts).len(), 6, "unrated, then one to five stars");
    }
}
