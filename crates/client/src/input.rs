//! The filter bar's typed input: the names the bar uses for its facets (`types`, `size_mb`, `taken`, …), the value a
//! list facet takes from its name and a key, and the text typed for a bound (empty is open; otherwise a number, or a
//! calendar day for the day facets). The query itself is `pg_model::query`; nothing here reads the catalog.

use pg_model::date::Day;
use pg_model::query::{
    BorderFacet, Choice, DateFacet, DayEnd, End, MediaType, RangeFacet, RatingFilter, Shape, Status, parse_number,
};

/// A list facet's value as the filter bar names it (`types`, `exts`, `ratings`, `orientations`, `status`, `borders`).
pub fn choice(facet: &str, value: &str) -> Option<Choice> {
    match facet {
        "types" => MediaType::from_key(value).map(Choice::Type),
        "exts" => Some(Choice::Ext(value.to_string())),
        "ratings" => value.parse::<u8>().ok().and_then(|n| RatingFilter::try_from(n).ok()).map(Choice::Rating),
        "orientations" => Shape::from_key(value).map(Choice::Shape),
        "status" => Status::from_key(value).map(Choice::Status),
        "borders" => BorderFacet::from_key(value).map(Choice::Border),
        _ => None,
    }
}

/// A range facet of the filter bar (`size_mb`, `megapixels`, `duration_s`).
pub fn range_facet(name: &str) -> Option<RangeFacet> {
    match name {
        "size_mb" => Some(RangeFacet::SizeMb),
        "megapixels" => Some(RangeFacet::Megapixels),
        "duration_s" => Some(RangeFacet::DurationS),
        _ => None,
    }
}

/// A day facet of the filter bar (`taken`, `modified`).
pub fn date_facet(name: &str) -> Option<DateFacet> {
    match name {
        "taken" => Some(DateFacet::Taken),
        "modified" => Some(DateFacet::Modified),
        _ => None,
    }
}

/// The end of a range facet's text (`min`, `max`).
pub fn end_of(name: &str) -> Option<End> {
    match name {
        "min" => Some(End::Min),
        "max" => Some(End::Max),
        _ => None,
    }
}

/// The end of a day facet's text (`from`, `to`).
pub fn day_end_of(name: &str) -> Option<DayEnd> {
    match name {
        "from" => Some(DayEnd::From),
        "to" => Some(DayEnd::To),
        _ => None,
    }
}

/// Whether text typed for a bound is empty (an open end) or parses: a number for the number facets, a calendar day
/// for the day facets.
pub fn bound_is_valid(facet: &str, text: &str) -> bool {
    text.trim().is_empty()
        || if date_facet(facet).is_some() { Day::parse(text).is_some() } else { parse_number(text).is_some() }
}

/// A day as the filter bar shows it, `""` when it is open.
pub fn day_text(day: Option<Day>) -> String {
    day.map(|d| d.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facet_names_map_to_their_facets_and_nothing_else() {
        assert!(range_facet("size_mb").is_some());
        assert!(range_facet("taken").is_none());
        assert!(date_facet("taken").is_some());
        assert!(date_facet("size_mb").is_none());
        assert!(end_of("min").is_some() && end_of("from").is_none());
        assert!(day_end_of("to").is_some() && day_end_of("max").is_none());
    }

    #[test]
    fn an_unknown_list_value_is_no_choice() {
        assert!(choice("types", "no-such-type").is_none());
        assert!(choice("no-such-facet", "image").is_none());
        assert!(choice("exts", "jpg").is_some());
    }

    #[test]
    fn an_empty_bound_is_open_and_junk_is_refused() {
        assert!(bound_is_valid("size_mb", ""));
        assert!(bound_is_valid("size_mb", "   "));
        assert!(bound_is_valid("size_mb", "12.5"));
        assert!(!bound_is_valid("size_mb", "lots"));
        assert!(!bound_is_valid("taken", "lots"));
    }

    #[test]
    fn an_open_day_shows_as_empty_text() {
        assert_eq!(day_text(None), "");
    }
}
