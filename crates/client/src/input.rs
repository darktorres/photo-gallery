//! The filter bar's typed input. The bar names its list facets by their index in `Facet::ALL` and its bounds by a
//! `Field` index (`Field::ALL`) and an `Edge` index (0 the low end, 1 the high end), so no facet name is a string
//! inside the core. A list value comes back from the bar as the key `key_of` gave it, read here at that edge only; the
//! text typed for a bound is empty (an open end), or a number, or a calendar day for the day fields. The query itself
//! is `pg_model::query`; nothing here reads the catalog.

use pg_model::date::Day;
use pg_model::query::{
    BorderFacet, Choice, DateFacet, DayEnd, End, MediaType, RangeFacet, RatingFilter, Shape, Status, parse_number,
};

use crate::facets::Facet;

/// A list facet's value from the key the bar sends (the key `key_of` gives it).
pub fn choice(facet: Facet, value: &str) -> Option<Choice> {
    match facet {
        Facet::Types => MediaType::from_key(value).map(Choice::Type),
        Facet::Exts => Some(Choice::Ext(value.to_string())),
        Facet::Ratings => value.parse::<u8>().ok().and_then(|n| RatingFilter::try_from(n).ok()).map(Choice::Rating),
        Facet::Orientations => Shape::from_key(value).map(Choice::Shape),
        Facet::Status => Status::from_key(value).map(Choice::Status),
        Facet::Borders => BorderFacet::from_key(value).map(Choice::Border),
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
    pub fn from_index(index: i32) -> Option<Field> {
        usize::try_from(index).ok().and_then(|i| Self::ALL.get(i).copied())
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
    pub fn from_index(index: i32) -> Option<Edge> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_and_ends_are_the_bars_indices() {
        assert_eq!(Field::from_index(0), Some(Field::Range(RangeFacet::SizeMb)));
        assert_eq!(Field::from_index(3), Some(Field::Day(DateFacet::Taken)));
        assert_eq!(Field::from_index(5), None);
        assert_eq!(Field::from_index(-1), None);
        assert_eq!(Edge::from_index(0), Some(Edge::Low));
        assert_eq!(Edge::from_index(1), Some(Edge::High));
        assert_eq!(Edge::from_index(2), None);
        assert_eq!(Edge::Low.number(), End::Min);
        assert_eq!(Edge::High.day(), DayEnd::To);
    }

    #[test]
    fn an_unknown_list_value_is_no_choice() {
        assert!(choice(Facet::Types, "no-such-type").is_none());
        assert!(choice(Facet::Exts, "jpg").is_some());
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
        let facets = [Facet::Types, Facet::Orientations, Facet::Status, Facet::Borders, Facet::Ratings, Facet::Exts];
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
