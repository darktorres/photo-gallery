//! The query a view filters by and the order it sorts in (PLAN §7): the data, and the edits a filter bar makes to it, as
//! typed values. Facets combine with AND, the values inside one list facet with OR, and an empty facet filters nothing.
//! The SQL is the catalog's (`pg_catalog::query`), and so is the JSON the views and the session store: the serde names
//! kept here are the stored ones. Text typed into a filter is read by the front end, which gives this module values.

use serde::{Deserialize, Deserializer, Serialize};

use crate::date::Day;
use crate::rating::Rating;

/// Type, as the filter sees it: the file's kind (`media::Kind`), with animated images apart from still ones.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "lowercase")]
#[derive(strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum MediaType {
    Image,
    Animated,
    Video,
}

impl MediaType {
    /// The name in JSON and in `CASE` (strum's snake_case, which serde's names are too).
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<MediaType> {
        key.parse().ok()
    }
}

/// Upright shape. Within 1% of square counts as square.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
#[derive(strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum Shape {
    Landscape,
    Portrait,
    Square,
}

impl Shape {
    /// The name in JSON.
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<Shape> {
        key.parse().ok()
    }
}

/// Where a file is in the import.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
#[derive(strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum Status {
    /// Analyzed, and the analysis is current.
    Imported,
    /// Not analyzed yet, or changed since (the background job will get to it).
    Pending,
    /// The import couldn't read or decode it.
    Failed,
}

impl Status {
    /// The name in JSON.
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<Status> {
        key.parse().ok()
    }
}

/// Borders, as the filter sees them: none found, or a side found of a class (`detection::Class`). Borders the user
/// marked as not a border count as none.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "lowercase")]
#[derive(strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum BorderFacet {
    None,
    Black,
    White,
    Color,
    Blur,
}

impl BorderFacet {
    /// The name in JSON (a class's, `detection::Class::key`).
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<BorderFacet> {
        key.parse().ok()
    }
}

/// A rating the query keeps: unrated, or a number of stars. In JSON, 0 is unrated and 1 to 5 the stars.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum RatingFilter {
    Unrated,
    Stars(Rating),
}

impl TryFrom<u8> for RatingFilter {
    type Error = u8;

    /// 0 is unrated; a rating from 1 to 5 is its stars; anything else is the value back.
    fn try_from(n: u8) -> Result<RatingFilter, u8> {
        match n {
            0 => Ok(RatingFilter::Unrated),
            n => Rating::new(n).map(RatingFilter::Stars).ok_or(n),
        }
    }
}

impl From<RatingFilter> for u8 {
    fn from(rating: RatingFilter) -> u8 {
        match rating {
            RatingFilter::Unrated => 0,
            RatingFilter::Stars(stars) => stars.get(),
        }
    }
}

/// What a view sorts by.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
#[derive(strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum SortKey {
    /// Folders together, in path order (all drives of a merged folder as one), then by name.
    Name,
    Size,
    Modified,
    /// When it was taken; files that don't say sort by their modified time (as local wall-clock time).
    Taken,
    Megapixels,
    Duration,
    /// The largest share of the image borders take on any one side; files without borders last.
    Borders,
}

impl SortKey {
    /// The name in JSON.
    pub fn key(self) -> &'static str {
        self.into()
    }

    pub fn from_key(key: &str) -> Option<SortKey> {
        key.parse().ok()
    }
}

/// A sort: its key, and whether it runs down.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sort {
    pub key: SortKey,
    #[serde(default)]
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Sort {
        Sort { key: SortKey::Name, descending: false }
    }
}

/// A numeric range; either end may be open. Both ends are included.
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Range {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl Range {
    pub fn is_empty(&self) -> bool {
        self.min.is_none() && self.max.is_none()
    }
}

/// Calendar days, both included; either end may be open.
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct DateRange {
    /// Days in JSON are `YYYY-MM-DD`; a stored date that doesn't parse doesn't filter, as before.
    #[serde(deserialize_with = "lenient_day")]
    pub from: Option<Day>,
    #[serde(deserialize_with = "lenient_day")]
    pub to: Option<Day>,
}

impl DateRange {
    pub fn is_empty(&self) -> bool {
        self.from.is_none() && self.to.is_none()
    }
}

fn lenient_day<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Day>, D::Error> {
    Ok(Option::<String>::deserialize(de)?.as_deref().and_then(Day::parse))
}

/// The facets a view filters by. `Query::default()` shows everything.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Query {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<MediaType>,
    /// Lower-case extensions without the dot.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exts: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ratings: Vec<RatingFilter>,
    #[serde(skip_serializing_if = "Range::is_empty")]
    pub size_mb: Range,
    #[serde(skip_serializing_if = "Range::is_empty")]
    pub megapixels: Range,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<Shape>,
    /// When it was taken (`analysis.taken`, wall-clock time).
    #[serde(skip_serializing_if = "DateRange::is_empty")]
    pub taken: DateRange,
    /// Last modified, in this computer's time zone.
    #[serde(skip_serializing_if = "DateRange::is_empty")]
    pub modified: DateRange,
    /// Seconds; videos only (anything without a duration is left out when set).
    #[serde(skip_serializing_if = "Range::is_empty")]
    pub duration_s: Range,
    /// Some(true): files with a duplicate among the present files (identical bytes, identical pixels or a tier-1
    /// pair within `pg_model::pdq::GROUP_DISTANCE`); Some(false): imported files without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicates: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub status: Vec<Status>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub borders: Vec<BorderFacet>,
    /// The review: Some(true), files whose borders the user marked as not a border; Some(false), files with borders
    /// found and not marked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_marked: Option<bool>,
    /// Part of the file name, any case.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// A list facet of the query.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListFacet {
    Types,
    Exts,
    Ratings,
    Shapes,
    Status,
    Borders,
}

/// A value a list facet chooses.
#[derive(Clone, PartialEq, Debug)]
pub enum Choice {
    Type(MediaType),
    Ext(String),
    Rating(RatingFilter),
    Shape(Shape),
    Status(Status),
    Border(BorderFacet),
}

impl Choice {
    /// The list facet the value belongs to.
    pub fn facet(&self) -> ListFacet {
        match self {
            Choice::Type(_) => ListFacet::Types,
            Choice::Ext(_) => ListFacet::Exts,
            Choice::Rating(_) => ListFacet::Ratings,
            Choice::Shape(_) => ListFacet::Shapes,
            Choice::Status(_) => ListFacet::Status,
            Choice::Border(_) => ListFacet::Borders,
        }
    }
}

/// A range facet of the query: its numbers, as the filter bar edits them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RangeFacet {
    SizeMb,
    Megapixels,
    DurationS,
}

/// A day facet of the query.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DateFacet {
    Taken,
    Modified,
}

/// The end of a numeric range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum End {
    Min,
    Max,
}

/// The end of a day range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DayEnd {
    From,
    To,
}

impl Query {
    /// Whether it filters anything.
    pub fn is_empty(&self) -> bool {
        *self == Query::default()
    }

    /// How many facets filter.
    pub fn active(&self) -> usize {
        [
            !self.types.is_empty(),
            !self.exts.is_empty(),
            !self.ratings.is_empty(),
            !self.size_mb.is_empty(),
            !self.megapixels.is_empty(),
            !self.shapes.is_empty(),
            !self.taken.is_empty(),
            !self.modified.is_empty(),
            !self.duration_s.is_empty(),
            self.duplicates.is_some(),
            !self.status.is_empty(),
            !self.borders.is_empty(),
            self.border_marked.is_some(),
            !self.name.is_empty(),
        ]
        .into_iter()
        .filter(|&filters| filters)
        .count()
    }

    /// The values chosen in a list facet, in the order they were chosen.
    pub fn chosen(&self, facet: ListFacet) -> Vec<Choice> {
        match facet {
            ListFacet::Types => self.types.iter().map(|&t| Choice::Type(t)).collect(),
            ListFacet::Exts => self.exts.iter().map(|e| Choice::Ext(e.clone())).collect(),
            ListFacet::Ratings => self.ratings.iter().map(|&r| Choice::Rating(r)).collect(),
            ListFacet::Shapes => self.shapes.iter().map(|&s| Choice::Shape(s)).collect(),
            ListFacet::Status => self.status.iter().map(|&s| Choice::Status(s)).collect(),
            ListFacet::Borders => self.borders.iter().map(|&b| Choice::Border(b)).collect(),
        }
    }

    pub fn has(&self, choice: &Choice) -> bool {
        self.chosen(choice.facet()).contains(choice)
    }

    /// Choose a value of a list facet, or unchoose it if it is chosen.
    pub fn toggle(&mut self, choice: Choice) {
        fn flip<T: PartialEq>(list: &mut Vec<T>, value: T) {
            match list.iter().position(|v| *v == value) {
                Some(i) => {
                    list.remove(i);
                }
                None => list.push(value),
            }
        }
        match choice {
            Choice::Type(t) => flip(&mut self.types, t),
            Choice::Ext(e) => flip(&mut self.exts, e),
            Choice::Rating(r) => flip(&mut self.ratings, r),
            Choice::Shape(s) => flip(&mut self.shapes, s),
            Choice::Status(s) => flip(&mut self.status, s),
            Choice::Border(b) => flip(&mut self.borders, b),
        }
    }

    fn range_mut(&mut self, facet: RangeFacet) -> &mut Range {
        match facet {
            RangeFacet::SizeMb => &mut self.size_mb,
            RangeFacet::Megapixels => &mut self.megapixels,
            RangeFacet::DurationS => &mut self.duration_s,
        }
    }

    fn day_range_mut(&mut self, facet: DateFacet) -> &mut DateRange {
        match facet {
            DateFacet::Taken => &mut self.taken,
            DateFacet::Modified => &mut self.modified,
        }
    }

    /// One end of a range facet; `None` when it is open.
    pub fn number(&self, facet: RangeFacet, end: End) -> Option<f64> {
        let range = match facet {
            RangeFacet::SizeMb => self.size_mb,
            RangeFacet::Megapixels => self.megapixels,
            RangeFacet::DurationS => self.duration_s,
        };
        match end {
            End::Min => range.min,
            End::Max => range.max,
        }
    }

    /// Set one end of a range facet; `None` opens it.
    pub fn set_number(&mut self, facet: RangeFacet, end: End, value: Option<f64>) {
        let range = self.range_mut(facet);
        match end {
            End::Min => range.min = value,
            End::Max => range.max = value,
        }
    }

    /// One end of a day facet; `None` when it is open.
    pub fn day(&self, facet: DateFacet, end: DayEnd) -> Option<Day> {
        let range = match facet {
            DateFacet::Taken => self.taken,
            DateFacet::Modified => self.modified,
        };
        match end {
            DayEnd::From => range.from,
            DayEnd::To => range.to,
        }
    }

    /// Set one end of a day facet; `None` opens it.
    pub fn set_day(&mut self, facet: DateFacet, end: DayEnd, value: Option<Day>) {
        let range = self.day_range_mut(facet);
        match end {
            DayEnd::From => range.from = value,
            DayEnd::To => range.to = value,
        }
    }
}

/// A number as typed, with a comma or a point before the decimals; `None` when it doesn't parse or isn't finite.
pub fn parse_number(text: &str) -> Option<f64> {
    text.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    fn stars(n: u8) -> RatingFilter {
        RatingFilter::Stars(Rating::new(n).unwrap())
    }

    #[test]
    fn list_facets_toggle() {
        let mut q = Query::default();
        q.toggle(Choice::Type(MediaType::Video));
        q.toggle(Choice::Rating(RatingFilter::Unrated));
        q.toggle(Choice::Rating(stars(5)));
        q.toggle(Choice::Ext("jpg".into()));
        q.toggle(Choice::Shape(Shape::Square));
        q.toggle(Choice::Status(Status::Failed));
        q.toggle(Choice::Border(BorderFacet::Blur));
        assert_eq!(q.borders, vec![BorderFacet::Blur]);
        q.toggle(Choice::Border(BorderFacet::Blur));
        assert_eq!(q.types, vec![MediaType::Video]);
        assert_eq!(q.ratings, vec![RatingFilter::Unrated, stars(5)]);
        assert!(q.has(&Choice::Rating(stars(5))) && q.has(&Choice::Ext("jpg".into())));
        assert!(!q.has(&Choice::Type(MediaType::Image)));
        assert_eq!(q.chosen(ListFacet::Ratings), vec![Choice::Rating(RatingFilter::Unrated), Choice::Rating(stars(5))]);
        assert_eq!(q.active(), 5);
        q.toggle(Choice::Rating(RatingFilter::Unrated));
        q.toggle(Choice::Type(MediaType::Video));
        assert_eq!(q.ratings, vec![stars(5)]);
        assert!(q.types.is_empty());
        assert_eq!(q.active(), 4);
    }

    #[test]
    fn a_rating_filter_is_zero_or_stars() {
        assert_eq!(RatingFilter::try_from(0), Ok(RatingFilter::Unrated));
        assert_eq!(RatingFilter::try_from(5), Ok(stars(5)));
        assert_eq!(RatingFilter::try_from(6), Err(6));
        assert_eq!(u8::from(RatingFilter::Unrated), 0);
        assert_eq!(u8::from(stars(3)), 3);
    }

    #[test]
    fn range_facets_hold_their_ends() {
        let mut q = Query::default();
        q.set_number(RangeFacet::SizeMb, End::Min, Some(1.5));
        q.set_number(RangeFacet::Megapixels, End::Max, Some(12.0));
        q.set_day(DateFacet::Taken, DayEnd::From, Day::parse("2024-02-29"));
        assert_eq!(q.size_mb, Range { min: Some(1.5), max: None });
        assert_eq!(q.number(RangeFacet::Megapixels, End::Max), Some(12.0));
        assert_eq!(q.day(DateFacet::Taken, DayEnd::From), Day::parse("2024-02-29"));
        assert_eq!(q.day(DateFacet::Taken, DayEnd::To), None);
        assert_eq!(q.active(), 3);
        q.set_number(RangeFacet::SizeMb, End::Min, None);
        q.set_day(DateFacet::Taken, DayEnd::From, None);
        assert!(q.size_mb.is_empty() && q.taken.is_empty());
        assert_eq!(q.active(), 1);
    }

    #[test]
    fn numbers_parse_with_a_comma_or_a_point() {
        assert_eq!(parse_number(" 1,5 "), Some(1.5));
        assert_eq!(parse_number("12"), Some(12.0));
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("1 MB"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn the_sort_is_by_name_and_ascending_unless_told() {
        assert_eq!(Sort::default(), Sort { key: SortKey::Name, descending: false });
        assert_eq!(SortKey::iter().count(), 7);
    }
}
