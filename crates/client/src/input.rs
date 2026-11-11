//! The filter bar's typed input. The bar names its list facets by their index in `facets::ALL` and its bounds by a
//! `Field` index (`Field::ALL`) and an `Edge` index (0 the low end, 1 the high end), so no facet name is a string
//! inside the core. A list value comes back from the bar as the key `key_of` gave it, read here at that edge only; the
//! text typed for a bound is empty (an open end), or a number, or a calendar day for the day fields. The query itself
//! is `pg_model::query`; nothing here reads the catalog.

use pg_model::date::Day;
use pg_model::query::{BorderFacet, Choice, DateFacet, DayEnd, End, ListFacet, RangeFacet, RatingFilter, Shape, Status, parse_number};
use pg_model::media::MediaType;

/// A list facet's value from the key the bar sends (the key `key_of` gives it).
pub fn choice(facet: ListFacet, value: &str) -> Option<Choice> {
    match facet {
        ListFacet::Types => MediaType::from_key(value).map(Choice::Type),
        ListFacet::Exts => Some(Choice::Ext(value.to_string())),
        ListFacet::Ratings => value.parse::<u8>().ok().and_then(|n| RatingFilter::try_from(n).ok()).map(Choice::Rating),
        ListFacet::Shapes => Shape::from_key(value).map(Choice::Shape),
        ListFacet::Status => Status::from_key(value).map(Choice::Status),
        ListFacet::Borders => BorderFacet::from_key(value).map(Choice::Border),
    }
}

/// The key a value is sent to the bar as; `choice` reads it back.
pub fn key_of(choice: &Choice) -> String {
    match choice {
        Choice::Type(t) => t.key().to_string(),
        Choice::Ext(e) => e.clone(),
        Choice::Rating(r) => u8::from(*r).to_string(),
        Choice::Shape(s) => s.key().to_string(),
        Choice::Status(s) => s.key().to_string(),
        Choice::Border(b) => b.key().to_string(),
    }
}

/// A field the bar edits the bounds of: a range facet's numbers, or a day facet's days.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Range(RangeFacet),
    Day(DateFacet),
}

impl Field {
    /// The fields in the bar's index order.
    pub const ALL: [Field; 5] = [
        Field::Range(RangeFacet::SizeMb),
        Field::Range(RangeFacet::Megapixels),
        Field::Range(RangeFacet::DurationS),
        Field::Day(DateFacet::Taken),
        Field::Day(DateFacet::Modified),
    ];

    /// The field at `index` (its position in `ALL`); `None` out of range.
    pub fn from_index(index: usize) -> Option<Field> {
        Self::ALL.get(index).copied()
    }
}

/// One end of a bound: `Low` is a range's `min` or a day range's `from`, `High` its `max` or `to`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Low,
    High,
}

impl Edge {
    /// The end at `index` (0 low, 1 high); `None` out of range.
    pub fn from_index(index: usize) -> Option<Edge> {
        match index {
            0 => Some(Edge::Low),
            1 => Some(Edge::High),
            _ => None,
        }
    }

    /// The end of a numeric range.
    pub fn number(self) -> End {
        match self {
            Edge::Low => End::Min,
            Edge::High => End::Max,
        }
    }

    /// The end of a day range.
    pub fn day(self) -> DayEnd {
        match self {
            Edge::Low => DayEnd::From,
            Edge::High => DayEnd::To,
        }
    }
}

/// Whether text typed for a bound is empty (an open end) or parses: a number for the range fields, a calendar day for
/// the day fields.
pub fn bound_is_valid(field: Field, text: &str) -> bool {
    text.trim().is_empty()
        || match field {
            Field::Range(_) => parse_number(text).is_some(),
            Field::Day(_) => Day::parse(text).is_some(),
        }
}

/// A day as the filter bar shows it, `""` when it is open.
pub fn day_text(day: Option<Day>) -> String {
    day.map(|d| d.to_string()).unwrap_or_default()
}

/// A yes/no/either box's index for a query's choice: 0 either way, 1 for `first`, 2 for the other. (The duplicates
/// box lists "with a duplicate" first; the review box "not marked".)
pub fn tri_index(value: Option<bool>, first: bool) -> usize {
    match value {
        None => 0,
        Some(v) if v == first => 1,
        Some(_) => 2,
    }
}

/// The choice a yes/no/either box's `index` is ([`tri_index`]'s inverse): either way for 0, or out of range.
pub fn tri_of_index(index: usize, first: bool) -> Option<bool> {
    match index {
        1 => Some(first),
        2 => Some(!first),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_and_ends_are_the_bars_indices() {
        assert_eq!(Field::from_index(0), Some(Field::Range(RangeFacet::SizeMb)));
        assert_eq!(Field::from_index(3), Some(Field::Day(DateFacet::Taken)));
        assert_eq!(Field::from_index(5), None);
        assert_eq!(Edge::from_index(0), Some(Edge::Low));
        assert_eq!(Edge::from_index(1), Some(Edge::High));
        assert_eq!(Edge::from_index(2), None);
        assert_eq!(Edge::Low.number(), End::Min);
        assert_eq!(Edge::High.day(), DayEnd::To);
    }

    #[test]
    fn a_yes_no_either_box_reads_back_its_index() {
        for first in [true, false] {
            for value in [None, Some(true), Some(false)] {
                assert_eq!(tri_of_index(tri_index(value, first), first), value);
            }
        }
        assert_eq!(tri_index(Some(true), true), 1);
        assert_eq!(tri_index(Some(true), false), 2);
        assert_eq!(tri_of_index(7, true), None);
    }

    #[test]
    fn an_unknown_list_value_is_no_choice() {
        assert!(choice(ListFacet::Types, "no-such-type").is_none());
        assert!(choice(ListFacet::Exts, "jpg").is_some());
    }

    #[test]
    fn a_choice_reads_back_from_its_key() {
        let values = [
            Choice::Type(MediaType::Video),
            Choice::Shape(Shape::Portrait),
            Choice::Status(Status::Failed),
            Choice::Border(BorderFacet::Blur),
            Choice::Rating(RatingFilter::Unrated),
            Choice::Ext("jpg".to_string()),
        ];
        let facets = [ListFacet::Types, ListFacet::Shapes, ListFacet::Status, ListFacet::Borders, ListFacet::Ratings, ListFacet::Exts];
        for (value, facet) in values.into_iter().zip(facets) {
            assert_eq!(choice(facet, &key_of(&value)), Some(value));
        }
    }

    #[test]
    fn an_empty_bound_is_open_and_junk_is_refused() {
        let size = Field::Range(RangeFacet::SizeMb);
        assert!(bound_is_valid(size, ""));
        assert!(bound_is_valid(size, "   "));
        assert!(bound_is_valid(size, "12.5"));
        assert!(!bound_is_valid(size, "lots"));
        assert!(!bound_is_valid(Field::Day(DateFacet::Taken), "lots"));
    }

    #[test]
    fn an_open_day_shows_as_empty_text() {
        assert_eq!(day_text(None), "");
    }
}
