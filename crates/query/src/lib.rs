//! What a view shows and in what order (PLAN §7): a `Query` of facets and a `Sort` (`pg_model::query`, the data and
//! its typed editing). Facets combine with AND, the values inside one facet with OR, and an empty facet doesn't filter.
//! Both become SQL here, with every value the user typed bound as a parameter. The JSON they are saved and sessioned as
//! is read and written here too (`from_json`, `to_json`).
//!
//! The SQL expects the browse query's aliases: `f` the file, `a` its analysis (joined only without an error), `r`
//! its rating, `m` its border mark (`f.order_key` is the name order).

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;
use rusqlite::types::{Value, ValueRef};
use serde::Serialize;
use serde::de::DeserializeOwned;
use strum::IntoEnumIterator;

use pg_model::group_distance;
use pg_model::query::{BorderFacet, MediaType, Query, Range, RatingFilter, Shape, Sort, SortKey, Status};

/// FILETIME (100 ns since 1601) of the Unix epoch, in seconds. A macro, for SQL made at compile time.
macro_rules! filetime_unix_offset {
    () => {
        11_644_473_600_i64
    };
}

// SQL used in more than one place is a macro, so every piece is a constant (`concat!`).

/// A file with borders found that the user didn't mark as not a border.
macro_rules! bordered {
    () => {
        "(a.cut_left + a.cut_top + a.cut_right + a.cut_bottom > 0 AND m.file_id IS NULL)"
    };
}

/// The orientations that swap the picture's sides (`orientation::swaps_axes`), as an SQL list. The test beside
/// `key_sql` checks that it is that rule.
macro_rules! swapping_orientations {
    () => {
        "5, 6, 7, 8"
    };
}

/// `BorderFacet::None`: looked for (a still image), and nothing that counts.
macro_rules! border_none {
    () => {
        concat!("(a.cut_left IS NOT NULL AND NOT ", bordered!(), ")")
    };
}

/// A border class's facet (its `BorderFacet::key`).
macro_rules! border_class {
    ($class:literal) => {
        concat!(
            "(",
            bordered!(),
            " AND EXISTS (SELECT 1 FROM border_sides s
                     WHERE s.class = '",
            $class,
            "' AND s.file_id = f.id AND s.rejected IS NULL))"
        )
    };
}

macro_rules! present_analysis {
    () => {
        "SELECT 1 FROM analysis d JOIN files df ON df.id = d.file_id
        WHERE d.file_id != f.id AND d.error IS NULL AND df.missing = 0"
    };
}

macro_rules! pair {
    ($this:literal, $other:literal) => {
        concat!(
            "EXISTS (SELECT 1 FROM pdq_pairs p JOIN files o ON o.id = p.",
            $other,
            "
                     WHERE p.",
            $this,
            " = f.id AND p.distance <= ",
            group_distance!(),
            " AND o.missing = 0)"
        )
    };
}

/// File sizes are in MB as Explorer counts them (MiB).
const MB: f64 = 1_048_576.0;

/// The condition of a type's value.
fn media_sql(media: MediaType) -> &'static str {
    match media {
        MediaType::Image => "(f.kind = 'image' AND coalesce(a.frames, 1) <= 1)",
        MediaType::Animated => "(f.kind = 'image' AND a.frames > 1)",
        MediaType::Video => "f.kind = 'video'",
    }
}

/// The type of a row, for the counts (`browse::read::facets`).
pub const MEDIA_CASE: &str =
    "CASE WHEN f.kind = 'video' THEN 'video' WHEN a.frames > 1 THEN 'animated' ELSE 'image' END";

/// The condition of a shape's value.
fn shape_sql(shape: Shape) -> &'static str {
    match shape {
        Shape::Landscape => "(a.width - a.height) * 100 > a.width",
        Shape::Portrait => "(a.height - a.width) * 100 > a.height",
        Shape::Square => "abs(a.width - a.height) * 100 <= max(a.width, a.height)",
    }
}

/// The condition of a status's value. Every file is one of the three: pending (`files.pending`, what the import does
/// next), else imported or failed by its current analysis.
fn status_sql(status: Status) -> &'static str {
    match status {
        Status::Imported => {
            "(f.pending = 0 AND EXISTS (SELECT 1 FROM analysis s WHERE s.file_id = f.id AND s.error IS NULL))"
        }
        Status::Pending => "f.pending = 1",
        Status::Failed => {
            "(f.pending = 0 AND EXISTS (SELECT 1 FROM analysis s WHERE s.file_id = f.id AND s.error IS NOT NULL))"
        }
    }
}

/// The condition of a border value (a class's, `detection::Class`, or none).
pub fn border_sql(border: BorderFacet) -> &'static str {
    match border {
        BorderFacet::None => border_none!(),
        BorderFacet::Black => border_class!("black"),
        BorderFacet::White => border_class!("white"),
        BorderFacet::Color => border_class!("color"),
        BorderFacet::Blur => border_class!("blur"),
    }
}

/// What `browse::read::facets` counts for borders, for the files that pass `cond` (the other facets' condition, as SQL):
/// one term per value, in `BorderFacet::iter()`'s order, each the number of those files it holds. A file with a black
/// and a blurred side counts for both.
pub fn border_counts(cond: &str) -> Vec<String> {
    BorderFacet::iter()
        .map(|b| format!("coalesce(sum(CASE WHEN ({cond}) AND {} THEN 1 ELSE 0 END), 0)", border_sql(b)))
        .collect()
}

/// The first Unix second of the local day whose midnight is the wall-clock time bound to each `?` (the day's seconds,
/// as `Day::seconds` has them): the first instant at which the local clock reads that midnight or later. It is the
/// usual instant, or, where the zone's offset changes at midnight, the earlier of two instants that read it (a
/// repeated hour), or the change itself (a skipped midnight). The offsets just before and after the day tell which.
/// Every `?` takes the same value: bind it once per `?` (`matches('?')` counts them).
pub fn local_start_sql() -> String {
    let offset = |instant: &str| format!("(unixepoch({instant}, 'unixepoch', 'localtime') - {instant})");
    let before = offset("(? - 86400)");
    let after = offset("(? + 86400)");
    let at_before = "(? - ".to_string() + &before + ")";
    let at_after = "(? - ".to_string() + &after + ")";
    let fits_before = format!("{} = {before}", offset(&at_before));
    let fits_after = format!("{} = {after}", offset(&at_after));
    format!(
        "(CASE WHEN {fits_before} AND {fits_after} THEN min({at_before}, {at_after})
              WHEN {fits_before} THEN {at_before} WHEN {fits_after} THEN {at_after} ELSE {at_before} END)"
    )
}

/// One the user marked.
const MARKED: &str = "(a.cut_left + a.cut_top + a.cut_right + a.cut_bottom > 0 AND m.file_id IS NOT NULL)";

/// A facet whose own values are left out when counting it (`browse::read::facets`): each value's count is what choosing
/// it would show, given the other facets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Facet {
    Types,
    Exts,
    Borders,
}

/// A WHERE condition and the values it binds, in order.
pub struct Condition {
    pub sql: String,
    pub params: Vec<Value>,
}

/// The condition of a query.
pub fn condition(query: &Query) -> Condition {
    condition_without(query, None)
}

/// The condition, leaving one facet out.
pub fn condition_without(query: &Query, skip: Option<Facet>) -> Condition {
    // Written into one string: each part ANDed on, a facet's values ORed in parentheses.
    let mut sql = String::new();
    let mut params: Vec<Value> = Vec::new();
    fn and(sql: &mut String) -> &mut String {
        if !sql.is_empty() {
            sql.push_str(" AND ");
        }
        sql
    }
    fn any<'a>(sql: &mut String, items: impl IntoIterator<Item = &'a str>) {
        let sql = and(sql);
        sql.push('(');
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 {
                sql.push_str(" OR ");
            }
            sql.push_str(item);
        }
        sql.push(')');
    }
    /// `column IN (?, …)` for `n` values.
    fn among(sql: &mut String, column: &str, n: usize) {
        sql.push_str(column);
        sql.push_str(" IN (");
        for i in 0..n {
            sql.push_str(if i > 0 { ", ?" } else { "?" });
        }
        sql.push(')');
    }

    if !query.types.is_empty() && skip != Some(Facet::Types) {
        any(&mut sql, query.types.iter().map(|&t| media_sql(t)));
    }
    if !query.exts.is_empty() && skip != Some(Facet::Exts) {
        among(and(&mut sql), "f.ext", query.exts.len());
        params.extend(query.exts.iter().map(|e| Value::Text(e.trim_start_matches('.').to_lowercase())));
    }
    let unrated = query.ratings.contains(&RatingFilter::Unrated);
    let stars: Vec<u8> = query
        .ratings
        .iter()
        .filter_map(|&r| match r {
            RatingFilter::Stars(stars) => Some(stars.get()),
            RatingFilter::Unrated => None,
        })
        .collect();
    if unrated || !stars.is_empty() {
        let sql = and(&mut sql);
        sql.push('(');
        if unrated {
            sql.push_str("r.rating IS NULL");
        }
        if !stars.is_empty() {
            if unrated {
                sql.push_str(" OR ");
            }
            among(sql, "r.rating", stars.len());
            params.extend(stars.iter().map(|&n| Value::Integer(n.into())));
        }
        sql.push(')');
    }
    let mut range = |column: &str, r: &Range, scale: f64| {
        for (end, op) in [(r.min, " >= ?"), (r.max, " <= ?")] {
            if let Some(v) = end {
                let sql = and(&mut sql);
                sql.push_str(column);
                sql.push_str(op);
                params.push(Value::Real(v * scale));
            }
        }
    };
    range("f.size", &query.size_mb, MB);
    range("a.width * a.height", &query.megapixels, 1e6);
    range("a.duration_ms", &query.duration_s, 1e3);
    if !query.shapes.is_empty() {
        any(&mut sql, query.shapes.iter().map(|&o| shape_sql(o)));
    }
    // Taken is wall-clock time (`analysis.taken`, already in this computer's zone): compared as is. Modified is a UTC
    // FILETIME: the local day's bounds are converted, to the instant the local clock first reads them (`local_start_sql`).
    let modified =
        |cmp: &str| format!("f.mtime {cmp} ((({}) + {}) * 10000000)", local_start_sql(), filetime_unix_offset!());
    let bounds = [
        (query.taken.from, "a.taken >= ?".to_string(), 0),
        (query.taken.to, "a.taken < ?".to_string(), 86_400),
        (query.modified.from, modified(">="), 0),
        (query.modified.to, modified("<"), 86_400),
    ];
    for (bound, condition, add) in bounds {
        if let Some(day) = bound {
            and(&mut sql).push_str(&condition);
            // The wall-clock seconds, once for each `?` of the condition.
            for _ in 0..condition.matches('?').count() {
                params.push(Value::Integer(day.seconds() + add));
            }
        }
    }
    match query.duplicates {
        Some(true) => and(&mut sql).push_str(DUPLICATES),
        Some(false) => and(&mut sql).push_str(NOT_DUPLICATES),
        None => {}
    }
    if !query.status.is_empty() {
        any(&mut sql, query.status.iter().map(|&s| status_sql(s)));
    }
    if !query.borders.is_empty() && skip != Some(Facet::Borders) {
        any(&mut sql, query.borders.iter().map(|&b| border_sql(b)));
    }
    match query.border_marked {
        Some(true) => and(&mut sql).push_str(MARKED),
        Some(false) => and(&mut sql).push_str(bordered!()),
        None => {}
    }
    let name = query.name.trim();
    if !name.is_empty() {
        and(&mut sql).push_str("pg_contains(f.name, ?)");
        params.push(Value::Text(name.to_string()));
    }

    if sql.is_empty() {
        sql.push('1');
    }
    Condition { sql, params }
}

/// `T` as JSON, as a view's query or sort is saved and sessioned.
pub fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".into())
}

/// `T` from JSON; malformed or empty text is `T::default()` (everything, for a query).
pub fn from_json<T: DeserializeOwned + Default>(json: &str) -> T {
    serde_json::from_str(json).unwrap_or_default()
}

/// A duplicate among the present files: identical bytes (tier 0a), identical pixels (0b) or a tier-1 pair within
/// `GROUP_DISTANCE`. The pair table's primary key is (a, b) and it has an index on b, hence one EXISTS per side.
macro_rules! duplicates {
    () => {
        concat!(
            "(EXISTS (",
            present_analysis!(),
            " AND d.file_hash = a.file_hash)
          OR EXISTS (",
            present_analysis!(),
            " AND d.pixel_hash = a.pixel_hash) OR ",
            pair!("a", "b"),
            " OR ",
            pair!("b", "a"),
            ")"
        )
    };
}
const DUPLICATES: &str = duplicates!();
/// An imported file without one.
const NOT_DUPLICATES: &str = concat!("a.file_id IS NOT NULL AND NOT ", duplicates!());

/// The value a sort orders by, as an integer expression over the file `f` and its analysis `a` (NULL: none, which
/// comes last either way); `None` for name order (`files.order_key`, then the id for files of the same name in
/// the same folder on two drives), which is also every other key's order for ties (`browse::read::list_grid`).
pub fn key_sql(sort: Sort) -> Option<&'static str> {
    match sort.key {
        SortKey::Name => None,
        SortKey::Size => Some("f.size"),
        SortKey::Modified => Some("f.mtime"),
        SortKey::Taken => Some(concat!(
            "coalesce(a.taken, unixepoch(f.mtime / 10000000 - ",
            filetime_unix_offset!(),
            ", 'unixepoch', 'localtime'))"
        )),
        SortKey::Megapixels => Some("a.width * a.height"),
        SortKey::Duration => Some("a.duration_ms"),
        // In millionths. Cuts are in stored pixels: a quarter turn (orientation 5–8) swaps the upright sides.
        SortKey::Borders => Some(concat!(
            "CASE WHEN ",
            bordered!(),
            " THEN CAST(1e6 * max(
                1.0 * max(a.cut_left, a.cut_right) / CASE WHEN a.orientation IN (",
            swapping_orientations!(),
            ") THEN a.height ELSE a.width END,
                1.0 * max(a.cut_top, a.cut_bottom) / CASE WHEN a.orientation IN (",
            swapping_orientations!(),
            ") THEN a.width ELSE a.height END)
                AS INTEGER) END"
        )),
    }
}

/// SQL functions the conditions use, registered on every catalog connection (`Catalog::prepare`, `Catalog::open`).
pub fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    // Whether a name contains a text, ignoring case as Windows does (Unicode, unlike SQLite's LIKE). A name is its
    // stored bytes (`files.name` is a BLOB); it is matched as its display text, so an unpaired surrogate is U+FFFD.
    conn.create_scalar_function(
        "pg_contains",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            // Borrowed from SQLite: a NULL (or a text, which is not a stored name) contains nothing.
            let (ValueRef::Blob(name), Ok(_)) = (ctx.get_raw(0), ctx.get_raw(1).as_str()) else {
                return Ok(false);
            };
            // The text is the statement's parameter: lowercased at the first row, and kept by SQLite with it for
            // the rest (auxiliary data).
            let needle = ctx.get_or_create_aux(1, |text| {
                text.as_str().map(Needle::new).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
            })?;
            Ok(needle.found_in(&String::from_utf8_lossy(name)))
        },
    )
}

/// Whether `name` contains `text`, both in lower case (Unicode): `name.to_lowercase().contains(&text.to_lowercase())`.
#[cfg(test)]
pub fn contains_ignoring_case(name: &str, text: &str) -> bool {
    Needle::new(text).found_in(name)
}

/// A text looked for in names, in lower case.
struct Needle {
    lower: String,
    ascii: bool,
}

impl Needle {
    fn new(text: &str) -> Needle {
        let lower = text.to_lowercase();
        let ascii = lower.is_ascii();
        Needle { lower, ascii }
    }

    /// Whether `name` in lower case (`str::to_lowercase`) contains it, without allocating. Character by
    /// character, the lower case is `str::to_lowercase`'s for every character but Σ, whose lower case depends on
    /// where it stands (a final one is ς), so a name with one is lowercased whole.
    fn found_in(&self, name: &str) -> bool {
        let needle = self.lower.as_str();
        if self.ascii && name.is_ascii() {
            let (name, needle) = (name.as_bytes(), needle.as_bytes());
            return needle.is_empty() || name.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle));
        }
        if name.contains('Σ') {
            return name.to_lowercase().contains(needle);
        }
        let mut rest = name.chars().flat_map(char::to_lowercase);
        loop {
            let mut at = rest.clone();
            if needle.chars().all(|c| at.next() == Some(c)) {
                return true;
            }
            if rest.next().is_none() {
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::date::Day;
    use pg_model::query::DateRange;
    use strum::IntoEnumIterator;

    #[test]
    fn the_sql_swap_list_is_the_swap_rule() {
        // The borders sort reads the swap from SQL; the list must be exactly the orientations that swap the sides.
        let listed: Vec<u8> = swapping_orientations!().split(", ").map(|o| o.parse().unwrap()).collect();
        assert_eq!(
            listed,
            (1..=8).filter(|&o| pg_model::orientation::Orientation::from_tag(o).swaps_axes()).collect::<Vec<_>>()
        );
        let borders = Sort { key: SortKey::Borders, descending: false };
        assert!(key_sql(borders).unwrap().contains(concat!("IN (", swapping_orientations!(), ")")));
    }

    /// Every name of up to 3 and text of up to 2 characters from a set chosen for their lower cases: ASCII,
    /// İ (2 characters in lower case), Σ σ ς (the final sigma), Kelvin K (lower case ASCII), ß ẞ, a combining dot,
    /// a space and a digit; and some longer ones. The function, and the same through SQL with the text as a
    /// parameter, against `str::to_lowercase` on both.
    #[test]
    fn a_name_contains_a_text_as_their_lower_cases_do() {
        let set = ['a', 'B', 'i', 'I', 'İ', 'Σ', 'σ', 'ς', '\u{212A}', 'k', 'ß', 'ẞ', '\u{307}', ' ', '1'];
        let strings = |max: usize| {
            let mut all = vec![String::new()];
            let mut last = vec![String::new()];
            for _ in 0..max {
                last = last.iter().flat_map(|s| set.iter().map(move |c| format!("{s}{c}"))).collect();
                all.extend(last.iter().cloned());
            }
            all
        };
        let mut names = strings(3);
        names
            .extend(["ὈΔΥΣΣΕΎΣ", "ΣΊΣΥΦΟΣ.jpg", "İSTANBUL 2019", "Straße_STRASSE", "x\u{212A}elvin"].map(String::from));
        let mut texts = strings(2);
        texts.extend(["σσ", "ΣΣΕ", "ος", "i\u{307}st", "STRASS", "ße_s", "kelvin"].map(String::from));
        let conn = Connection::open_in_memory().unwrap();
        register_functions(&conn).unwrap();
        let mut stmt = conn.prepare("SELECT pg_contains(CAST(value AS BLOB), ?1) FROM json_each(?2)").unwrap();
        let names_json = serde_json::to_string(&names).unwrap();
        for text in &texts {
            let want: Vec<bool> = names.iter().map(|n| n.to_lowercase().contains(&text.to_lowercase())).collect();
            let got: Vec<bool> = names.iter().map(|n| contains_ignoring_case(n, text)).collect();
            assert_eq!(got, want, "{text:?}");
            let sql: Vec<bool> =
                stmt.query_map((text, &names_json), |r| r.get(0)).unwrap().map(Result::unwrap).collect();
            assert_eq!(sql, want, "{text:?} through SQL");
        }
        // A NULL contains nothing, and a text is not a stored name (the names are blobs): it is found in nothing.
        // A blob is its bytes' display text.
        let one = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, bool>(0)).unwrap();
        assert!(!one("SELECT pg_contains(NULL, 'a')") && !one("SELECT pg_contains('a', NULL)"));
        assert!(!one("SELECT pg_contains('Ab', 'a')"));
        assert!(one("SELECT pg_contains(CAST('Ab' AS BLOB), 'a')") && !one("SELECT pg_contains(x'61', 'b')"));
        // A surrogate (WTF-8, `ED A0 80` for U+D800) shows as U+FFFD, which the text can match.
        assert!(one("SELECT pg_contains(x'61EDA080', char(65533))"));
    }

    /// A name that is no UTF-8 is matched as its display text: its valid parts are found, and its bad bytes show as
    /// U+FFFD. A query is text, so it can't name the bad bytes; it can find the text on either side of them only when
    /// they are not between the two halves it asks for (`o.j` is not in `photo` + the bad bytes + `.jpg`).
    #[test]
    fn a_name_that_is_no_utf8_is_matched_by_its_valid_parts() {
        let conn = Connection::open_in_memory().unwrap();
        register_functions(&conn).unwrap();
        let found = |name: &[u8], text: &str| -> bool {
            conn.query_row("SELECT pg_contains(?1, ?2)", (name, text), |r| r.get::<_, bool>(0)).unwrap()
        };
        // A lone surrogate (WTF-8 `ED A0 80`) between `photo` and `.jpg`, and a raw `0xFF` byte after `IMG_`.
        let surrogate: &[u8] = b"photo\xED\xA0\x80.jpg";
        assert!(found(surrogate, "photo") && found(surrogate, ".jpg"));
        assert!(found(surrogate, "\u{FFFD}") && !found(surrogate, "o.j"));
        let raw: &[u8] = b"IMG_\xFF";
        assert!(found(raw, "img_") && found(raw, "\u{FFFD}"));
    }

    #[test]
    fn json_round_trips() {
        let q = Query {
            types: vec![MediaType::Video, MediaType::Animated],
            exts: vec!["mp4".into()],
            ratings: vec![RatingFilter::Unrated, RatingFilter::try_from(5).unwrap()],
            size_mb: Range { min: Some(1.5), max: None },
            taken: DateRange { from: Day::parse("2024-01-01"), to: None },
            duplicates: Some(false),
            status: vec![Status::Failed],
            borders: vec![BorderFacet::None, BorderFacet::White],
            border_marked: Some(true),
            name: "cat".into(),
            ..Query::default()
        };
        assert_eq!(from_json::<Query>(&to_json(&q)), q);
        assert_eq!(to_json(&Query::default()), "{}");
        assert_eq!(from_json::<Query>("not json"), Query::default());
        let s = Sort { key: SortKey::Taken, descending: true };
        assert_eq!(from_json::<Sort>(&to_json(&s)), s);
        assert_eq!(from_json::<Sort>(r#"{"key":"size"}"#), Sort { key: SortKey::Size, descending: false });
        assert_eq!(MediaType::from_key("animated"), Some(MediaType::Animated));
    }

    #[test]
    fn a_stored_date_that_doesnt_parse_is_no_filter_and_keeps_the_rest() {
        // Hand-edited JSON with a day that doesn't exist: that end is open, and the other facets still filter.
        let stored: Query = from_json(r#"{"ratings":[0,5],"taken":{"from":"2024-02-30","to":"2024-03-01"}}"#);
        assert_eq!(stored.ratings, vec![RatingFilter::Unrated, RatingFilter::try_from(5).unwrap()]);
        assert_eq!(stored.taken, DateRange { from: None, to: Day::parse("2024-03-01") });
        assert!(to_json(&stored).contains(r#""taken":{"from":null,"to":"2024-03-01"}"#));
    }

    fn json<T: Serialize>(value: &T) -> String {
        serde_json::to_string(value).unwrap()
    }

    #[test]
    fn keys_are_the_json_names() {
        // The names the catalog and the saved views store, in the order the lists show them.
        assert_eq!(MediaType::iter().map(MediaType::key).collect::<Vec<_>>(), ["image", "animated", "video"]);
        assert_eq!(Shape::iter().map(Shape::key).collect::<Vec<_>>(), ["landscape", "portrait", "square"]);
        assert_eq!(Status::iter().map(Status::key).collect::<Vec<_>>(), ["imported", "pending", "failed"]);
        assert_eq!(
            BorderFacet::iter().map(BorderFacet::key).collect::<Vec<_>>(),
            ["none", "black", "white", "color", "blur"],
        );
        assert_eq!(
            SortKey::iter().map(SortKey::key).collect::<Vec<_>>(),
            ["name", "size", "modified", "taken", "megapixels", "duration", "borders"],
        );
        for t in [MediaType::Image, MediaType::Animated, MediaType::Video] {
            assert_eq!(json(&t), format!("\"{}\"", t.key()));
        }
        for o in Shape::iter() {
            assert_eq!(json(&o), format!("\"{}\"", o.key()));
            assert_eq!(Shape::from_key(o.key()), Some(o));
        }
        for s in Status::iter() {
            assert_eq!(json(&s), format!("\"{}\"", s.key()));
            assert_eq!(Status::from_key(s.key()), Some(s));
        }
        for b in BorderFacet::iter() {
            assert_eq!(json(&b), format!("\"{}\"", b.key()));
            assert_eq!(BorderFacet::from_key(b.key()), Some(b));
        }
        for c in pg_model::detection::Class::iter() {
            assert_eq!(BorderFacet::from_key(c.key()).map(BorderFacet::key), Some(c.key()));
        }
        for k in SortKey::iter() {
            assert_eq!(json(&k), format!("\"{}\"", k.key()));
            assert_eq!(SortKey::from_key(k.key()), Some(k));
        }
        assert_eq!(SortKey::from_key("nope"), None);
    }

    #[test]
    fn values_are_bound_not_spliced() {
        let q = Query { exts: vec!["jpg'); DROP TABLE files; --".into()], name: "a'b".into(), ..Query::default() };
        let c = condition(&q);
        assert!(!c.sql.contains("DROP") && !c.sql.contains("a'b"));
        assert_eq!(c.params.len(), 2);
    }

    #[test]
    fn border_counts_follow_the_condition_and_each_facet_sql() {
        let counts = border_counts("x = 1");
        assert_eq!(counts.len(), BorderFacet::iter().count());
        for (count, b) in counts.iter().zip(BorderFacet::iter()) {
            assert_eq!(count, &format!("coalesce(sum(CASE WHEN (x = 1) AND {} THEN 1 ELSE 0 END), 0)", border_sql(b)));
        }
        for b in BorderFacet::iter().skip(1) {
            assert!(border_sql(b).contains(&format!("s.class = '{}'", b.key())), "{b:?}");
        }
    }

    /// The local day's first instant, for every day from 2000 to 2030, in this machine's zone: the instant before it
    /// reads earlier than midnight, and it reads midnight or later. A zone's changes at midnight (where the day has a
    /// skipped or a repeated hour) are among these days wherever they fall.
    #[test]
    fn a_local_day_starts_where_its_clock_first_reads_midnight() {
        use rusqlite::params_from_iter;
        let c = Connection::open_in_memory().unwrap();
        let local = |instant: i64| -> i64 {
            c.query_row("SELECT unixepoch(?1, 'unixepoch', 'localtime')", [instant], |r| r.get(0)).unwrap()
        };
        let sql = format!("SELECT {}", local_start_sql());
        let days_2000_to_2030 = 10_957..=21_915;
        for day in days_2000_to_2030 {
            let midnight = day * 86_400;
            let values = vec![midnight; sql.matches('?').count()];
            let start: i64 = c.query_row(&sql, params_from_iter(values), |r| r.get(0)).unwrap();
            assert!(local(start) >= midnight, "day {day}: {start} reads {}", local(start));
            assert!(local(start - 1) < midnight, "day {day}: {start} is not the first");
        }
    }
}
