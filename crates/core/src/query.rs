//! What a view shows and in what order (PLAN §7): a [`Query`] of facets and a [`Sort`]. Facets combine with AND,
//! the values inside one facet with OR, and an empty facet doesn't filter. Both become SQL here, with every value
//! the user typed bound as a parameter, and both travel as JSON (saved views, the session, the QML side).
//!
//! The SQL expects the browse query's aliases: `f` the file, `a` its analysis (joined only without an error), `r`
//! its rating (`f.order_key` is the name order).

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

use crate::analyze::IMPORT_VERSION;
use crate::metadata::parse_iso_date;
use crate::pdq::GROUP_DISTANCE;

/// FILETIME (100 ns since 1601) of the Unix epoch, in seconds.
const FILETIME_UNIX_OFFSET: i64 = 11_644_473_600;
/// File sizes are in MB as Explorer counts them (MiB).
const MB: f64 = 1_048_576.0;

/// Type, as the filter sees it: the file's kind (`media::Kind`), with animated images apart from still ones.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Image,
    Animated,
    Video,
}

impl MediaType {
    fn sql(self) -> &'static str {
        match self {
            MediaType::Image => "(f.kind = 'image' AND coalesce(a.frames, 1) <= 1)",
            MediaType::Animated => "(f.kind = 'image' AND a.frames > 1)",
            MediaType::Video => "f.kind = 'video'",
        }
    }

    /// The type of a row, for the counts (`browse::facets`).
    pub(crate) const CASE: &'static str =
        "CASE WHEN f.kind = 'video' THEN 'video' WHEN a.frames > 1 THEN 'animated' ELSE 'image' END";

    /// The name in JSON and in `CASE`.
    pub fn key(self) -> &'static str {
        match self {
            MediaType::Image => "image",
            MediaType::Animated => "animated",
            MediaType::Video => "video",
        }
    }

    pub fn from_key(key: &str) -> Option<MediaType> {
        [MediaType::Image, MediaType::Animated, MediaType::Video].into_iter().find(|t| t.key() == key)
    }
}

/// Upright shape. Within 1% of square counts as square.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    Landscape,
    Portrait,
    Square,
}

impl Orientation {
    const ALL: [Orientation; 3] = [Orientation::Landscape, Orientation::Portrait, Orientation::Square];

    /// The name in JSON.
    pub fn key(self) -> &'static str {
        match self {
            Orientation::Landscape => "landscape",
            Orientation::Portrait => "portrait",
            Orientation::Square => "square",
        }
    }

    pub fn from_key(key: &str) -> Option<Orientation> {
        Orientation::ALL.into_iter().find(|o| o.key() == key)
    }

    fn sql(self) -> &'static str {
        match self {
            Orientation::Landscape => "(a.width - a.height) * 100 > a.width",
            Orientation::Portrait => "(a.height - a.width) * 100 > a.height",
            Orientation::Square => "abs(a.width - a.height) * 100 <= max(a.width, a.height)",
        }
    }
}

/// Where a file is in the import.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Analyzed, and the analysis is current.
    Imported,
    /// Not analyzed yet, or changed since (the background job will get to it).
    Pending,
    /// The import couldn't read or decode it.
    Failed,
}

impl Status {
    const ALL: [Status; 3] = [Status::Imported, Status::Pending, Status::Failed];

    /// The name in JSON.
    pub fn key(self) -> &'static str {
        match self {
            Status::Imported => "imported",
            Status::Pending => "pending",
            Status::Failed => "failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|s| s.key() == key)
    }

    fn sql(self) -> String {
        let current = format!("s.version = {IMPORT_VERSION} AND s.size = f.size AND s.mtime = f.mtime");
        match self {
            Status::Imported => {
                format!("EXISTS (SELECT 1 FROM analysis s WHERE s.file_id = f.id AND s.error IS NULL AND {current})")
            }
            Status::Pending => format!("NOT EXISTS (SELECT 1 FROM analysis s WHERE s.file_id = f.id AND {current})"),
            Status::Failed => {
                "EXISTS (SELECT 1 FROM analysis s WHERE s.file_id = f.id AND s.error IS NOT NULL)".to_string()
            }
        }
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
    fn is_empty(&self) -> bool {
        self.min.is_none() && self.max.is_none()
    }
}

/// Calendar days `YYYY-MM-DD`, both included; either end may be open. A malformed date doesn't filter (the UI
/// marks it).
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct DateRange {
    pub from: Option<String>,
    pub to: Option<String>,
}

impl DateRange {
    fn is_empty(&self) -> bool {
        self.from.is_none() && self.to.is_none()
    }
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
    /// 0 is unrated.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ratings: Vec<u8>,
    #[serde(skip_serializing_if = "Range::is_empty")]
    pub size_mb: Range,
    #[serde(skip_serializing_if = "Range::is_empty")]
    pub megapixels: Range,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub orientations: Vec<Orientation>,
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
    /// pair within `pdq::GROUP_DISTANCE`); Some(false): imported files without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicates: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub status: Vec<Status>,
    /// Part of the file name, any case.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// A facet whose own values are left out when counting it (`browse::facets`): each value's count is what choosing
/// it would show, given the other facets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Facet {
    Types,
    Exts,
}

/// A WHERE condition and the values it binds, in order.
pub struct Condition {
    pub sql: String,
    pub params: Vec<Value>,
}

impl Query {
    /// The query a JSON text describes; malformed or empty text is everything (`Query::default`).
    pub fn from_json(json: &str) -> Query {
        serde_json::from_str(json).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Whether it filters anything.
    pub fn is_empty(&self) -> bool {
        *self == Query::default()
    }

    pub fn condition(&self) -> Condition {
        self.condition_without(None)
    }

    /// The condition, leaving one facet out.
    pub fn condition_without(&self, skip: Option<Facet>) -> Condition {
        let mut parts: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let any = |items: Vec<String>| format!("({})", items.join(" OR "));

        if !self.types.is_empty() && skip != Some(Facet::Types) {
            parts.push(any(self.types.iter().map(|t| t.sql().to_string()).collect()));
        }
        if !self.exts.is_empty() && skip != Some(Facet::Exts) {
            parts.push(format!("f.ext IN ({})", vec!["?"; self.exts.len()].join(", ")));
            params.extend(self.exts.iter().map(|e| Value::Text(e.trim_start_matches('.').to_lowercase())));
        }
        if !self.ratings.is_empty() {
            let mut either = Vec::new();
            if self.ratings.contains(&0) {
                either.push("r.rating IS NULL".to_string());
            }
            let stars: Vec<&u8> = self.ratings.iter().filter(|&&n| (1..=5).contains(&n)).collect();
            if !stars.is_empty() {
                either.push(format!("r.rating IN ({})", vec!["?"; stars.len()].join(", ")));
                params.extend(stars.into_iter().map(|&n| Value::Integer(n.into())));
            }
            if !either.is_empty() {
                parts.push(any(either));
            }
        }
        let mut range = |column: &str, r: &Range, scale: f64| {
            if let Some(min) = r.min {
                parts.push(format!("{column} >= ?"));
                params.push(Value::Real(min * scale));
            }
            if let Some(max) = r.max {
                parts.push(format!("{column} <= ?"));
                params.push(Value::Real(max * scale));
            }
        };
        range("f.size", &self.size_mb, MB);
        range("a.width * a.height", &self.megapixels, 1e6);
        range("a.duration_ms", &self.duration_s, 1e3);
        if !self.orientations.is_empty() {
            parts.push(any(self.orientations.iter().map(|o| o.sql().to_string()).collect()));
        }
        // Taken is wall-clock time: compared as is. Modified is a UTC FILETIME: the local day's bounds are converted.
        let day = |d: &Option<String>| d.as_deref().and_then(parse_iso_date).map(|s| s - s.rem_euclid(86_400));
        if let Some(from) = day(&self.taken.from) {
            parts.push("a.taken >= ?".into());
            params.push(Value::Integer(from));
        }
        if let Some(to) = day(&self.taken.to) {
            parts.push("a.taken < ?".into());
            params.push(Value::Integer(to + 86_400));
        }
        let local_day_start = format!("(unixepoch(?, 'unixepoch', 'utc') + {FILETIME_UNIX_OFFSET}) * 10000000");
        if let Some(from) = day(&self.modified.from) {
            parts.push(format!("f.mtime >= {local_day_start}"));
            params.push(Value::Integer(from));
        }
        if let Some(to) = day(&self.modified.to) {
            parts.push(format!("f.mtime < {local_day_start}"));
            params.push(Value::Integer(to + 86_400));
        }
        match self.duplicates {
            Some(true) => parts.push(duplicates()),
            Some(false) => parts.push(format!("a.file_id IS NOT NULL AND NOT {}", duplicates())),
            None => {}
        }
        if !self.status.is_empty() {
            parts.push(any(self.status.iter().map(|s| s.sql()).collect()));
        }
        let name = self.name.trim();
        if !name.is_empty() {
            parts.push("pg_contains(f.name, ?)".into());
            params.push(Value::Text(name.to_string()));
        }

        Condition { sql: if parts.is_empty() { "1".into() } else { parts.join(" AND ") }, params }
    }
}

/// Editing a query from a filter bar. Facets and values go by their JSON names: list facets `types`, `exts`,
/// `ratings` (`"0"`–`"5"`), `orientations` and `status`; range facets `size_mb`, `megapixels` and `duration_s`
/// (ends `min`, `max`) and `taken` and `modified` (ends `from`, `to`, as `YYYY-MM-DD`). A name or value it doesn't
/// know changes nothing.
impl Query {
    /// How many facets filter.
    pub fn active(&self) -> usize {
        [
            !self.types.is_empty(),
            !self.exts.is_empty(),
            !self.ratings.is_empty(),
            !self.size_mb.is_empty(),
            !self.megapixels.is_empty(),
            !self.orientations.is_empty(),
            !self.taken.is_empty(),
            !self.modified.is_empty(),
            !self.duration_s.is_empty(),
            self.duplicates.is_some(),
            !self.status.is_empty(),
            !self.name.is_empty(),
        ]
        .into_iter()
        .filter(|&filters| filters)
        .count()
    }

    /// The values chosen in a list facet, in the order they were chosen.
    pub fn chosen(&self, facet: &str) -> Vec<String> {
        match facet {
            "types" => self.types.iter().map(|t| t.key().to_string()).collect(),
            "exts" => self.exts.clone(),
            "ratings" => self.ratings.iter().map(u8::to_string).collect(),
            "orientations" => self.orientations.iter().map(|o| o.key().to_string()).collect(),
            "status" => self.status.iter().map(|s| s.key().to_string()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn has(&self, facet: &str, value: &str) -> bool {
        self.chosen(facet).iter().any(|v| v == value)
    }

    /// Choose a value of a list facet, or unchoose it if it is chosen.
    pub fn toggle(&mut self, facet: &str, value: &str) {
        fn flip<T: PartialEq>(list: &mut Vec<T>, value: T) {
            match list.iter().position(|v| *v == value) {
                Some(i) => {
                    list.remove(i);
                }
                None => list.push(value),
            }
        }
        match facet {
            "types" => MediaType::from_key(value).into_iter().for_each(|t| flip(&mut self.types, t)),
            "exts" => flip(&mut self.exts, value.to_string()),
            "ratings" => {
                value.parse::<u8>().ok().filter(|&n| n <= 5).into_iter().for_each(|n| flip(&mut self.ratings, n))
            }
            "orientations" => Orientation::from_key(value).into_iter().for_each(|o| flip(&mut self.orientations, o)),
            "status" => Status::from_key(value).into_iter().for_each(|s| flip(&mut self.status, s)),
            _ => {}
        }
    }

    fn range_mut(&mut self, facet: &str) -> Option<&mut Range> {
        match facet {
            "size_mb" => Some(&mut self.size_mb),
            "megapixels" => Some(&mut self.megapixels),
            "duration_s" => Some(&mut self.duration_s),
            _ => None,
        }
    }

    fn date_range_mut(&mut self, facet: &str) -> Option<&mut DateRange> {
        match facet {
            "taken" => Some(&mut self.taken),
            "modified" => Some(&mut self.modified),
            _ => None,
        }
    }

    /// One end of a range facet as text, `""` when it is open.
    pub fn bound(&self, facet: &str, end: &str) -> String {
        let number = |r: &Range| match end {
            "min" => r.min,
            "max" => r.max,
            _ => None,
        };
        let day = |r: &DateRange| match end {
            "from" => r.from.clone(),
            "to" => r.to.clone(),
            _ => None,
        };
        match facet {
            "size_mb" => number(&self.size_mb).map(|v| v.to_string()),
            "megapixels" => number(&self.megapixels).map(|v| v.to_string()),
            "duration_s" => number(&self.duration_s).map(|v| v.to_string()),
            "taken" => day(&self.taken),
            "modified" => day(&self.modified),
            _ => None,
        }
        .unwrap_or_default()
    }

    /// Set one end of a range facet from what was typed. Text that doesn't parse (see `bound_is_valid`) leaves
    /// that end open.
    pub fn set_bound(&mut self, facet: &str, end: &str, text: &str) {
        if let Some(r) = self.range_mut(facet) {
            let value = parse_number(text);
            match end {
                "min" => r.min = value,
                "max" => r.max = value,
                _ => {}
            }
        } else if let Some(r) = self.date_range_mut(facet) {
            let value = parse_day(text);
            match end {
                "from" => r.from = value,
                "to" => r.to = value,
                _ => {}
            }
        }
    }

    /// Whether text typed for a range facet is empty (an open end) or parses: a number (`,` or `.` before the
    /// decimals) for `size_mb`, `megapixels` and `duration_s`, a calendar day `YYYY-MM-DD` for `taken` and `modified`.
    pub fn bound_is_valid(facet: &str, text: &str) -> bool {
        text.trim().is_empty()
            || match facet {
                "taken" | "modified" => parse_day(text).is_some(),
                _ => parse_number(text).is_some(),
            }
    }
}

/// A number as typed, with a comma or a point before the decimals.
fn parse_number(text: &str) -> Option<f64> {
    text.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A calendar day `YYYY-MM-DD` that exists.
fn parse_day(text: &str) -> Option<String> {
    let t = text.trim();
    let b = t.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !digits(0..4) || !digits(5..7) || !digits(8..10) {
        return None;
    }
    let (y, m, d): (u32, u32, u32) = (t[0..4].parse().ok()?, t[5..7].parse().ok()?, t[8..10].parse().ok()?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (1..=days).contains(&d).then(|| t.to_string())
}

/// A duplicate among the present files: identical bytes (tier 0a), identical pixels (0b) or a tier-1 pair within
/// `GROUP_DISTANCE`. The pair table's primary key is (a, b) and it has an index on b, hence one EXISTS per side.
fn duplicates() -> String {
    let present_analysis = "SELECT 1 FROM analysis d JOIN files df ON df.id = d.file_id
        WHERE d.file_id != f.id AND d.error IS NULL AND df.missing = 0";
    let pair = |this: &str, other: &str| {
        format!(
            "EXISTS (SELECT 1 FROM pdq_pairs p JOIN files o ON o.id = p.{other}
                     WHERE p.{this} = f.id AND p.distance <= {GROUP_DISTANCE} AND o.missing = 0)"
        )
    };
    format!(
        "(EXISTS ({present_analysis} AND d.file_hash = a.file_hash)
          OR EXISTS ({present_analysis} AND d.pixel_hash = a.pixel_hash) OR {} OR {})",
        pair("a", "b"),
        pair("b", "a")
    )
}

/// What a view is ordered by.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    /// Folders together, in path order (all drives of a merged folder as one), then by name.
    Name,
    Size,
    Modified,
    /// When it was taken; files that don't say sort by their modified time (as local wall-clock time).
    Taken,
    Megapixels,
    Duration,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sort {
    pub key: SortKey,
    #[serde(default)]
    pub descending: bool,
}

impl SortKey {
    const ALL: [SortKey; 6] =
        [SortKey::Name, SortKey::Size, SortKey::Modified, SortKey::Taken, SortKey::Megapixels, SortKey::Duration];

    /// The name in JSON.
    pub fn key(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Modified => "modified",
            SortKey::Taken => "taken",
            SortKey::Megapixels => "megapixels",
            SortKey::Duration => "duration",
        }
    }

    pub fn from_key(key: &str) -> Option<SortKey> {
        SortKey::ALL.into_iter().find(|k| k.key() == key)
    }
}

impl Default for Sort {
    fn default() -> Sort {
        Sort { key: SortKey::Name, descending: false }
    }
}

impl Sort {
    pub fn from_json(json: &str) -> Sort {
        serde_json::from_str(json).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// The value sorted on, as an integer expression over the file `f` and its analysis `a` (NULL: none, which
    /// comes last either way); `None` for name order (`files.order_key`, then the id for files of the same name in
    /// the same folder on two drives), which is also every other key's order for ties (`browse::list_grid`).
    pub fn key_sql(self) -> Option<String> {
        let mtime_local = format!("unixepoch(f.mtime / 10000000 - {FILETIME_UNIX_OFFSET}, 'unixepoch', 'localtime')");
        match self.key {
            SortKey::Name => None,
            SortKey::Size => Some("f.size".into()),
            SortKey::Modified => Some("f.mtime".into()),
            SortKey::Taken => Some(format!("coalesce(a.taken, {mtime_local})")),
            SortKey::Megapixels => Some("a.width * a.height".into()),
            SortKey::Duration => Some("a.duration_ms".into()),
        }
    }
}

/// SQL functions the conditions use, registered on every catalog connection (`Catalog::open`).
pub fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    // Whether a name contains a text, ignoring case as Windows does (Unicode, unlike SQLite's LIKE).
    conn.create_scalar_function(
        "pg_contains",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            // Borrowed from SQLite: a NULL (or a blob) contains nothing.
            let (Ok(name), Ok(text)) = (ctx.get_raw(0).as_str(), ctx.get_raw(1).as_str()) else {
                return Ok(false);
            };
            Ok(contains_ignoring_case(name, text))
        },
    )
}

/// Whether `name` contains `text`, both in lower case (Unicode). ASCII, the usual case, without allocating.
pub fn contains_ignoring_case(name: &str, text: &str) -> bool {
    if name.is_ascii() && text.is_ascii() {
        let (name, text) = (name.as_bytes(), text.as_bytes());
        text.is_empty() || name.windows(text.len()).any(|w| w.eq_ignore_ascii_case(text))
    } else {
        name.to_lowercase().contains(&text.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips() {
        let q = Query {
            types: vec![MediaType::Video, MediaType::Animated],
            exts: vec!["mp4".into()],
            ratings: vec![0, 5],
            size_mb: Range { min: Some(1.5), max: None },
            taken: DateRange { from: Some("2024-01-01".into()), to: None },
            duplicates: Some(false),
            status: vec![Status::Failed],
            name: "cat".into(),
            ..Query::default()
        };
        assert_eq!(Query::from_json(&q.to_json()), q);
        assert_eq!(Query::default().to_json(), "{}");
        assert_eq!(Query::from_json("not json"), Query::default());
        let s = Sort { key: SortKey::Taken, descending: true };
        assert_eq!(Sort::from_json(&s.to_json()), s);
        assert_eq!(Sort::from_json(r#"{"key":"size"}"#), Sort { key: SortKey::Size, descending: false });
        assert_eq!(MediaType::from_key("animated"), Some(MediaType::Animated));
    }

    fn json<T: Serialize>(value: &T) -> String {
        serde_json::to_string(value).unwrap()
    }

    #[test]
    fn keys_are_the_json_names() {
        for t in [MediaType::Image, MediaType::Animated, MediaType::Video] {
            assert_eq!(json(&t), format!("\"{}\"", t.key()));
        }
        for o in Orientation::ALL {
            assert_eq!(json(&o), format!("\"{}\"", o.key()));
            assert_eq!(Orientation::from_key(o.key()), Some(o));
        }
        for s in Status::ALL {
            assert_eq!(json(&s), format!("\"{}\"", s.key()));
            assert_eq!(Status::from_key(s.key()), Some(s));
        }
        for k in SortKey::ALL {
            assert_eq!(json(&k), format!("\"{}\"", k.key()));
            assert_eq!(SortKey::from_key(k.key()), Some(k));
        }
        assert_eq!(SortKey::from_key("nope"), None);
    }

    #[test]
    fn list_facets_toggle() {
        let mut q = Query::default();
        q.toggle("types", "video");
        q.toggle("ratings", "0");
        q.toggle("ratings", "5");
        q.toggle("exts", "jpg");
        q.toggle("orientations", "square");
        q.toggle("status", "failed");
        assert_eq!(q.types, vec![MediaType::Video]);
        assert_eq!(q.ratings, vec![0, 5]);
        assert!(q.has("ratings", "5") && q.has("exts", "jpg") && !q.has("types", "image"));
        assert_eq!(q.chosen("ratings"), vec!["0", "5"]);
        assert_eq!(q.active(), 5);
        q.toggle("ratings", "0");
        q.toggle("types", "video");
        assert_eq!(q.ratings, vec![5]);
        assert!(q.types.is_empty());
        assert_eq!(q.active(), 4);
        // Unknown facets and values, and ratings out of range, change nothing.
        let before = q.clone();
        q.toggle("types", "picture");
        q.toggle("ratings", "6");
        q.toggle("ratings", "x");
        q.toggle("colour", "red");
        assert_eq!(q, before);
    }

    #[test]
    fn range_facets_parse_what_was_typed() {
        let mut q = Query::default();
        q.set_bound("size_mb", "min", " 1,5 ");
        q.set_bound("megapixels", "max", "12");
        q.set_bound("taken", "from", "2024-02-29");
        assert_eq!(q.size_mb, Range { min: Some(1.5), max: None });
        assert_eq!(q.bound("size_mb", "min"), "1.5");
        assert_eq!(q.bound("megapixels", "max"), "12");
        assert_eq!(q.bound("taken", "from"), "2024-02-29");
        assert_eq!(q.bound("taken", "to"), "");
        assert_eq!(q.active(), 3);
        // Text that doesn't parse opens that end again.
        q.set_bound("size_mb", "min", "lots");
        q.set_bound("taken", "from", "2023-02-29");
        assert!(q.size_mb.is_empty() && q.taken.is_empty());
        assert_eq!(q.active(), 1);
        assert!(Query::bound_is_valid("size_mb", ""));
        assert!(Query::bound_is_valid("duration_s", "2.5"));
        assert!(!Query::bound_is_valid("duration_s", "inf"));
        assert!(!Query::bound_is_valid("size_mb", "1 MB"));
        assert!(Query::bound_is_valid("modified", "2000-12-31"));
        assert!(!Query::bound_is_valid("modified", "2000-13-01"));
        assert!(!Query::bound_is_valid("modified", "1900-02-29"));
        assert!(!Query::bound_is_valid("modified", "2000-1-1"));
    }

    #[test]
    fn values_are_bound_not_spliced() {
        let q = Query { exts: vec!["jpg'); DROP TABLE files; --".into()], name: "a'b".into(), ..Query::default() };
        let c = q.condition();
        assert!(!c.sql.contains("DROP") && !c.sql.contains("a'b"));
        assert_eq!(c.params.len(), 2);
    }
}
