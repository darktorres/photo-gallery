//! The words the front end shows: the facet labels and the filter line, the grid's badge and info line, the jobs' status
//! and notice lines, the write-back line, the Yandex panel's lines, the rating symbols and the timecodes. Each line is a
//! [`Message`], a typed value with its values; its words are the Fluent entry of the same id in `locales/en/text.ftl`
//! ([`render`]). The functions below build the messages for the values they are given, so this crate needs no Qt and no
//! job type; the desktop maps the job's values in. Nothing here reads the catalog.

use std::sync::atomic::{AtomicBool, Ordering};

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use pg_model::detection::Class;
use pg_model::query::{BorderFacet, Choice, DateRange, MediaType, Query, Range, RatingFilter, Shape, Status};
use unic_langid::LanguageIdentifier;

use crate::input::day_text;

/// The rating symbols, by rating (0 unrated, 5 best).
pub const RATING_SYMBOLS: [&str; 6] = ["☆", "❌", "❔", "🔨", "✔️", "⭐"];

/// The English words, one entry per message (see the file's header).
const ENGLISH: &str = include_str!("../locales/en/text.ftl");

/// The English words of the front end's own literals, by id (`ui`): the labels, tooltips and prompts the QML shows.
const ENGLISH_UI: &str = include_str!("../locales/en/ui.ftl");

thread_local! {
    // Fluent's bundle is not thread-safe, so each thread that words a line builds its own from the files once.
    static BUNDLE: FluentBundle<FluentResource> = english();
}

/// Set by `--locale=pseudo`: every line is then shown in the pseudo-locale (see [`pseudo`]).
static PSEUDO: AtomicBool = AtomicBool::new(false);

/// Shows the lines in the pseudo-locale when `on`, or in English.
pub fn set_pseudo(on: bool) {
    PSEUDO.store(on, Ordering::Relaxed);
}

fn english() -> FluentBundle<FluentResource> {
    // A literal tag, so it parses; the root locale would stand in for it otherwise.
    let langid: LanguageIdentifier = "en".parse().unwrap_or_default();
    let mut bundle = FluentBundle::new(vec![langid]);
    // Fluent wraps each placeable in invisible isolation marks for right-to-left text; the screen wants the plain text.
    bundle.set_use_isolating(false);
    for source in [ENGLISH, ENGLISH_UI] {
        let resource = FluentResource::try_new(source.to_string()).unwrap_or_else(|(resource, _)| resource);
        // The English entries name no id twice (the tests check it); were one named twice, its first entry is kept
        // and the rest of the file is added all the same.
        let _ = bundle.add_resource(resource);
    }
    bundle
}

/// A line of the front end's text: its entry in `locales/en/text.ftl` and the values it words. The variants are the
/// lines the functions below make; a variant without an entry (or an entry without a variant) fails the test that
/// lists them.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    TypeImage,
    TypeAnimated,
    TypeVideo,
    ShapeLandscape,
    ShapePortrait,
    ShapeSquare,
    StatusImported,
    StatusPending,
    StatusFailed,
    BorderNone,
    BorderBlack,
    BorderWhite,
    BorderColor,
    BorderBlur,
    ExtNone,
    FilterAll,
    FilterUnrated,
    FilterWithDuplicates,
    FilterWithoutDuplicates,
    FilterTaken { days: String },
    FilterModified { days: String },
    FilterRangeMb { from: String, to: String },
    FilterRangeMp { from: String, to: String },
    FilterRangeSeconds { from: String, to: String },
    FilterBorders { values: String },
    FilterBordersMarked,
    FilterBordersNotMarked,
    FilterName { name: String },
    BadgeVideo { time: String },
    BadgeApng,
    InfoNotImported,
    InfoDims { width: String, height: String },
    InfoSizeMb { size: String },
    InfoSizeKb { size: String },
    InfoKept,
    InfoLine { path: String, dims: String, size: String, kept: String, borders: String },
    ClassBlack,
    ClassWhite,
    ClassColor,
    ClassBlur,
    BordersInfo { names: String, left: String, top: String, right: String, bottom: String, marked: String },
    BordersMarked,
    PassScan { root: String, count: String },
    PassImport,
    PassBorders,
    PassCrop,
    PassThumbs { height: String },
    PassWorking { what: String, done: String, total: String, rate: String, eta: String, failed: String },
    PassFailed { count: String },
    EtaSeconds { seconds: String },
    EtaMinutes { minutes: String },
    EtaHours { hours: String, minutes: String },
    WritingChanges { count: usize },
    ChangesUnwritten { count: usize, error: String },
    UnreadableFolders { root: String, count: String },
    ScanFailed { root: String, error: String },
    RootRefused { root: String, reason: String },
    ScanNote { root: String, kind: String, count: String },
    RoundPanicked { message: String },
    CacheFull { height: String, made: String },
    CroppedSaved { count: usize, folder: String },
    CroppedSkipped { count: usize },
    CroppedFailed { failed: String, error: String },
    YandexCaptcha,
    YandexNotStill,
    YandexUploading,
    YandexOtherSizes,
    YandexSimilar,
    YandexUnreadablePhoto { error: String },
    YandexSimilarStopped { shown: String, reason: String },
    YandexSimilarFailed { reason: String },
}

impl Message {
    /// The entry this message is worded by in `locales/en/text.ftl`.
    pub fn id(&self) -> &'static str {
        match self {
            Message::TypeImage => "type-image",
            Message::TypeAnimated => "type-animated",
            Message::TypeVideo => "type-video",
            Message::ShapeLandscape => "shape-landscape",
            Message::ShapePortrait => "shape-portrait",
            Message::ShapeSquare => "shape-square",
            Message::StatusImported => "status-imported",
            Message::StatusPending => "status-pending",
            Message::StatusFailed => "status-failed",
            Message::BorderNone => "border-none",
            Message::BorderBlack => "border-black",
            Message::BorderWhite => "border-white",
            Message::BorderColor => "border-color",
            Message::BorderBlur => "border-blur",
            Message::ExtNone => "ext-none",
            Message::FilterAll => "filter-all",
            Message::FilterUnrated => "filter-unrated",
            Message::FilterWithDuplicates => "filter-with-duplicates",
            Message::FilterWithoutDuplicates => "filter-without-duplicates",
            Message::FilterTaken { .. } => "filter-taken",
            Message::FilterModified { .. } => "filter-modified",
            Message::FilterRangeMb { .. } => "filter-range-mb",
            Message::FilterRangeMp { .. } => "filter-range-mp",
            Message::FilterRangeSeconds { .. } => "filter-range-seconds",
            Message::FilterBorders { .. } => "filter-borders",
            Message::FilterBordersMarked => "filter-borders-marked",
            Message::FilterBordersNotMarked => "filter-borders-not-marked",
            Message::FilterName { .. } => "filter-name",
            Message::BadgeVideo { .. } => "badge-video",
            Message::BadgeApng => "badge-apng",
            Message::InfoNotImported => "info-not-imported",
            Message::InfoDims { .. } => "info-dims",
            Message::InfoSizeMb { .. } => "info-size-mb",
            Message::InfoSizeKb { .. } => "info-size-kb",
            Message::InfoKept => "info-kept",
            Message::InfoLine { .. } => "info-line",
            Message::ClassBlack => "class-black",
            Message::ClassWhite => "class-white",
            Message::ClassColor => "class-color",
            Message::ClassBlur => "class-blur",
            Message::BordersInfo { .. } => "borders-info",
            Message::BordersMarked => "borders-marked",
            Message::PassScan { .. } => "pass-scan",
            Message::PassImport => "pass-import",
            Message::PassBorders => "pass-borders",
            Message::PassCrop => "pass-crop",
            Message::PassThumbs { .. } => "pass-thumbs",
            Message::PassWorking { .. } => "pass-working",
            Message::PassFailed { .. } => "pass-failed",
            Message::EtaSeconds { .. } => "eta-seconds",
            Message::EtaMinutes { .. } => "eta-minutes",
            Message::EtaHours { .. } => "eta-hours",
            Message::WritingChanges { .. } => "writing-changes",
            Message::ChangesUnwritten { .. } => "changes-unwritten",
            Message::UnreadableFolders { .. } => "unreadable-folders",
            Message::ScanFailed { .. } => "scan-failed",
            Message::RootRefused { .. } => "root-refused",
            Message::ScanNote { .. } => "scan-note",
            Message::RoundPanicked { .. } => "round-panicked",
            Message::CacheFull { .. } => "cache-full",
            Message::CroppedSaved { .. } => "cropped-saved",
            Message::CroppedSkipped { .. } => "cropped-skipped",
            Message::CroppedFailed { .. } => "cropped-failed",
            Message::YandexCaptcha => "yandex-captcha",
            Message::YandexNotStill => "yandex-not-still",
            Message::YandexUploading => "yandex-uploading",
            Message::YandexOtherSizes => "yandex-other-sizes",
            Message::YandexSimilar => "yandex-similar",
            Message::YandexUnreadablePhoto { .. } => "yandex-unreadable-photo",
            Message::YandexSimilarStopped { .. } => "yandex-similar-stopped",
            Message::YandexSimilarFailed { .. } => "yandex-similar-failed",
        }
    }

    /// The values the entry takes. A count is passed twice: as a number, which the plural forms choose by, and as its
    /// digits, which the text shows (so no thousands separator is added).
    fn args(&self) -> FluentArgs<'_> {
        let mut a = FluentArgs::new();
        match self {
            Message::FilterTaken { days } | Message::FilterModified { days } => {
                a.set("days", days.as_str());
            }
            Message::FilterRangeMb { from, to }
            | Message::FilterRangeMp { from, to }
            | Message::FilterRangeSeconds { from, to } => {
                a.set("from", from.as_str());
                a.set("to", to.as_str());
            }
            Message::FilterBorders { values } => {
                a.set("values", values.as_str());
            }
            Message::FilterName { name } => {
                a.set("name", name.as_str());
            }
            Message::BadgeVideo { time } => {
                a.set("time", time.as_str());
            }
            Message::InfoDims { width, height } => {
                a.set("width", width.as_str());
                a.set("height", height.as_str());
            }
            Message::InfoSizeMb { size } | Message::InfoSizeKb { size } => {
                a.set("size", size.as_str());
            }
            Message::InfoLine { path, dims, size, kept, borders } => {
                a.set("path", path.as_str());
                a.set("dims", dims.as_str());
                a.set("size", size.as_str());
                a.set("kept", kept.as_str());
                a.set("borders", borders.as_str());
            }
            Message::BordersInfo { names, left, top, right, bottom, marked } => {
                a.set("names", names.as_str());
                a.set("left", left.as_str());
                a.set("top", top.as_str());
                a.set("right", right.as_str());
                a.set("bottom", bottom.as_str());
                a.set("marked", marked.as_str());
            }
            Message::PassScan { root, count } => {
                a.set("root", root.as_str());
                a.set("count", count.as_str());
            }
            Message::PassThumbs { height } => {
                a.set("height", height.as_str());
            }
            Message::PassWorking { what, done, total, rate, eta, failed } => {
                a.set("what", what.as_str());
                a.set("done", done.as_str());
                a.set("total", total.as_str());
                a.set("rate", rate.as_str());
                a.set("eta", eta.as_str());
                a.set("failed", failed.as_str());
            }
            Message::PassFailed { count } => {
                a.set("count", count.as_str());
            }
            Message::EtaSeconds { seconds } => {
                a.set("seconds", seconds.as_str());
            }
            Message::EtaMinutes { minutes } => {
                a.set("minutes", minutes.as_str());
            }
            Message::EtaHours { hours, minutes } => {
                a.set("hours", hours.as_str());
                a.set("minutes", minutes.as_str());
            }
            Message::WritingChanges { count } => plural(&mut a, *count),
            Message::ChangesUnwritten { count, error } => {
                plural(&mut a, *count);
                a.set("error", error.as_str());
            }
            Message::UnreadableFolders { root, count } => {
                a.set("root", root.as_str());
                a.set("count", count.as_str());
            }
            Message::ScanFailed { root, error } => {
                a.set("root", root.as_str());
                a.set("error", error.as_str());
            }
            Message::RootRefused { root, reason } => {
                a.set("root", root.as_str());
                a.set("reason", reason.as_str());
            }
            Message::ScanNote { root, kind, count } => {
                a.set("root", root.as_str());
                a.set("kind", kind.as_str());
                a.set("count", count.as_str());
            }
            Message::RoundPanicked { message } => {
                a.set("message", message.as_str());
            }
            Message::CacheFull { height, made } => {
                a.set("height", height.as_str());
                a.set("made", made.as_str());
            }
            Message::CroppedSaved { count, folder } => {
                plural(&mut a, *count);
                a.set("folder", folder.as_str());
            }
            Message::CroppedSkipped { count } => plural(&mut a, *count),
            Message::CroppedFailed { failed, error } => {
                a.set("failed", failed.as_str());
                a.set("error", error.as_str());
            }
            Message::YandexUnreadablePhoto { error } => {
                a.set("error", error.as_str());
            }
            Message::YandexSimilarStopped { shown, reason } => {
                a.set("shown", shown.as_str());
                a.set("reason", reason.as_str());
            }
            Message::YandexSimilarFailed { reason } => {
                a.set("reason", reason.as_str());
            }
            _ => {}
        };
        a
    }

    /// Every variant, with sample values: the test that the entries and the variants match runs on this list.
    #[cfg(test)]
    fn samples() -> Vec<Message> {
        let s = || "x".to_string();
        vec![
            Message::TypeImage,
            Message::TypeAnimated,
            Message::TypeVideo,
            Message::ShapeLandscape,
            Message::ShapePortrait,
            Message::ShapeSquare,
            Message::StatusImported,
            Message::StatusPending,
            Message::StatusFailed,
            Message::BorderNone,
            Message::BorderBlack,
            Message::BorderWhite,
            Message::BorderColor,
            Message::BorderBlur,
            Message::ExtNone,
            Message::FilterAll,
            Message::FilterUnrated,
            Message::FilterWithDuplicates,
            Message::FilterWithoutDuplicates,
            Message::FilterTaken { days: s() },
            Message::FilterModified { days: s() },
            Message::FilterRangeMb { from: s(), to: s() },
            Message::FilterRangeMp { from: s(), to: s() },
            Message::FilterRangeSeconds { from: s(), to: s() },
            Message::FilterBorders { values: s() },
            Message::FilterBordersMarked,
            Message::FilterBordersNotMarked,
            Message::FilterName { name: s() },
            Message::BadgeVideo { time: s() },
            Message::BadgeApng,
            Message::InfoNotImported,
            Message::InfoDims { width: s(), height: s() },
            Message::InfoSizeMb { size: s() },
            Message::InfoSizeKb { size: s() },
            Message::InfoKept,
            Message::InfoLine { path: s(), dims: s(), size: s(), kept: s(), borders: s() },
            Message::ClassBlack,
            Message::ClassWhite,
            Message::ClassColor,
            Message::ClassBlur,
            Message::BordersInfo { names: s(), left: s(), top: s(), right: s(), bottom: s(), marked: s() },
            Message::BordersMarked,
            Message::PassScan { root: s(), count: s() },
            Message::PassImport,
            Message::PassBorders,
            Message::PassCrop,
            Message::PassThumbs { height: s() },
            Message::PassWorking { what: s(), done: s(), total: s(), rate: s(), eta: s(), failed: s() },
            Message::PassFailed { count: s() },
            Message::EtaSeconds { seconds: s() },
            Message::EtaMinutes { minutes: s() },
            Message::EtaHours { hours: s(), minutes: s() },
            Message::WritingChanges { count: 1 },
            Message::ChangesUnwritten { count: 1, error: s() },
            Message::UnreadableFolders { root: s(), count: s() },
            Message::ScanFailed { root: s(), error: s() },
            Message::RootRefused { root: s(), reason: s() },
            Message::ScanNote { root: s(), kind: s(), count: s() },
            Message::RoundPanicked { message: s() },
            Message::CacheFull { height: s(), made: s() },
            Message::CroppedSaved { count: 1, folder: s() },
            Message::CroppedSkipped { count: 1 },
            Message::CroppedFailed { failed: s(), error: s() },
            Message::YandexCaptcha,
            Message::YandexNotStill,
            Message::YandexUploading,
            Message::YandexOtherSizes,
            Message::YandexSimilar,
            Message::YandexUnreadablePhoto { error: s() },
            Message::YandexSimilarStopped { shown: s(), reason: s() },
            Message::YandexSimilarFailed { reason: s() },
        ]
    }
}

/// The count for a plural entry: its number selects the form, its digits are the text.
fn plural(a: &mut FluentArgs<'_>, count: usize) {
    a.set("count", pg_model::num::count(count));
    a.set("text", count.to_string());
}

/// The line `message` reads as in the English words.
pub fn render(message: &Message) -> String {
    shown(message.id(), &message.args())
}

/// A line of the front end's own literals: the entry `id` of `locales/en/ui.ftl`, with `count` (which chooses a plural
/// form, and is not shown) and the values `text` and `other`, which are shown as they are, so a number goes in as text
/// (Fluent may group its digits). A missing entry shows its id, which the desktop's test refuses.
pub fn ui(id: &str, count: f64, text: &str, other: &str) -> String {
    let mut args = FluentArgs::new();
    args.set("count", count);
    args.set("text", text);
    args.set("other", other);
    shown(id, &args)
}

fn shown(id: &str, args: &FluentArgs<'_>) -> String {
    let line = BUNDLE.with(|bundle| {
        let Some(entry) = bundle.get_message(id) else { return id.to_string() };
        let Some(pattern) = entry.value() else { return id.to_string() };
        let mut errors = Vec::new();
        bundle.format_pattern(pattern, Some(args), &mut errors).into_owned()
    });
    if PSEUDO.load(Ordering::Relaxed) { pseudo(&line) } else { line }
}

/// The pseudo-locale: each letter accented, the line padded by a third and bracketed, so that a place too small for a
/// longer translation shows as clipped in the screenshot matrix. Digits and symbols are kept.
fn pseudo(english: &str) -> String {
    const ACCENTED: &str = "åƀçđéƒğĥíĵķĺḿñöþqŕšťüṽŵẋýž";
    let accented: Vec<char> = ACCENTED.chars().collect();
    let mut out = String::with_capacity(english.len() * 2);
    out.push('⟦');
    for c in english.chars() {
        if c.is_ascii_alphabetic() {
            let index = u8::try_from(c.to_ascii_lowercase()).unwrap_or(b'a') - b'a';
            let base = accented.get(usize::from(index)).copied().unwrap_or(c);
            out.push(if c.is_ascii_uppercase() { base.to_uppercase().next().unwrap_or(base) } else { base });
        } else {
            out.push(c);
        }
    }
    out.extend(std::iter::repeat_n('~', english.chars().count() / 3 + 1));
    out.push('⟧');
    out
}

/// The symbol of a rating; a rating past 5 is shown as 5.
pub fn rating_symbol(rating: u8) -> &'static str {
    let [.., best] = RATING_SYMBOLS;
    RATING_SYMBOLS.get(usize::from(rating)).copied().unwrap_or(best)
}

/// A playback position or a length in whole seconds: m:ss, or h:mm:ss from an hour.
pub fn timecode(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// How the filter bar names a media type.
pub fn type_label(media: MediaType) -> String {
    render(&match media {
        MediaType::Image => Message::TypeImage,
        MediaType::Animated => Message::TypeAnimated,
        MediaType::Video => Message::TypeVideo,
    })
}

/// How the filter bar names a shape (an orientation).
pub fn shape_label(shape: Shape) -> String {
    render(&match shape {
        Shape::Landscape => Message::ShapeLandscape,
        Shape::Portrait => Message::ShapePortrait,
        Shape::Square => Message::ShapeSquare,
    })
}

/// How the filter bar names an import status.
pub fn status_label(status: Status) -> String {
    render(&match status {
        Status::Imported => Message::StatusImported,
        Status::Pending => Message::StatusPending,
        Status::Failed => Message::StatusFailed,
    })
}

/// How the filter bar names a borders value.
pub fn border_label(border: BorderFacet) -> String {
    render(&match border {
        BorderFacet::None => Message::BorderNone,
        BorderFacet::Black => Message::BorderBlack,
        BorderFacet::White => Message::BorderWhite,
        BorderFacet::Color => Message::BorderColor,
        BorderFacet::Blur => Message::BorderBlur,
    })
}

/// How the filter bar names a value a list facet chooses: its label, the extension's name (`(none)` for the empty one),
/// or the rating's number.
pub fn choice_label(choice: &Choice) -> String {
    match choice {
        Choice::Type(media) => type_label(*media),
        Choice::Ext(ext) if ext.is_empty() => render(&Message::ExtNone),
        Choice::Ext(ext) => ext.clone(),
        Choice::Rating(rating) => u8::from(*rating).to_string(),
        Choice::Shape(shape) => shape_label(*shape),
        Choice::Status(status) => status_label(*status),
        Choice::Border(border) => border_label(*border),
    }
}

/// What `q` keeps, in one line: "all files" when it filters nothing.
pub fn describe(q: &Query) -> String {
    let open = |v: Option<f64>| v.map(|v| v.to_string()).unwrap_or_default();
    let range = |r: &Range, message: fn(String, String) -> Message| render(&message(open(r.min), open(r.max)));
    let days = |r: &DateRange| format!("{} – {}", day_text(r.from), day_text(r.to));
    let mut parts: Vec<String> = Vec::new();
    if !q.types.is_empty() {
        parts.push(q.types.iter().map(|t| type_label(*t)).collect::<Vec<_>>().join(", "));
    }
    if !q.exts.is_empty() {
        parts.push(q.exts.join(", "));
    }
    if !q.ratings.is_empty() {
        let ratings: Vec<String> = q
            .ratings
            .iter()
            .map(|&r| match r {
                RatingFilter::Unrated => render(&Message::FilterUnrated),
                RatingFilter::Stars(stars) => rating_symbol(stars.get()).to_string(),
            })
            .collect();
        parts.push(ratings.join(" "));
    }
    if q.size_mb.min.is_some() || q.size_mb.max.is_some() {
        parts.push(range(&q.size_mb, |from, to| Message::FilterRangeMb { from, to }));
    }
    if q.megapixels.min.is_some() || q.megapixels.max.is_some() {
        parts.push(range(&q.megapixels, |from, to| Message::FilterRangeMp { from, to }));
    }
    if !q.shapes.is_empty() {
        parts.push(q.shapes.iter().map(|o| o.key()).collect::<Vec<_>>().join(", "));
    }
    if q.taken.from.is_some() || q.taken.to.is_some() {
        parts.push(render(&Message::FilterTaken { days: days(&q.taken) }));
    }
    if q.modified.from.is_some() || q.modified.to.is_some() {
        parts.push(render(&Message::FilterModified { days: days(&q.modified) }));
    }
    if q.duration_s.min.is_some() || q.duration_s.max.is_some() {
        parts.push(range(&q.duration_s, |from, to| Message::FilterRangeSeconds { from, to }));
    }
    if let Some(d) = q.duplicates {
        parts.push(render(&if d { Message::FilterWithDuplicates } else { Message::FilterWithoutDuplicates }));
    }
    if !q.status.is_empty() {
        parts.push(q.status.iter().map(|s| status_label(*s).to_lowercase()).collect::<Vec<_>>().join(", "));
    }
    if !q.borders.is_empty() {
        let values: Vec<String> = q.borders.iter().map(|b| border_label(*b).to_lowercase()).collect();
        parts.push(render(&Message::FilterBorders { values: values.join(", ") }));
    }
    if let Some(m) = q.border_marked {
        parts.push(render(&if m { Message::FilterBordersMarked } else { Message::FilterBordersNotMarked }));
    }
    if !q.name.is_empty() {
        parts.push(render(&Message::FilterName { name: q.name.clone() }));
    }
    if parts.is_empty() { render(&Message::FilterAll) } else { parts.join(" · ") }
}

/// The badge on a grid cell: a video's length, an animation's format; nothing for a still image.
pub fn badge(media: MediaType, duration_ms: Option<u32>, ext: &str) -> String {
    match media {
        MediaType::Video => match duration_ms {
            Some(ms) => render(&Message::BadgeVideo { time: timecode((u64::from(ms) + 500) / 1000) }),
            None => "▶".to_string(),
        },
        MediaType::Animated if ext == "png" => render(&Message::BadgeApng),
        MediaType::Animated => ext.to_uppercase(),
        MediaType::Image => String::new(),
    }
}

/// The borders of a file that a crop would cut: whether the user marked them not a border, the cut in stored pixels
/// (left, top, right, bottom), and the classes of the accepted sides.
pub struct FoundBorders<'a> {
    pub marked: bool,
    pub cut: [u32; 4],
    pub classes: &'a [Class],
}

/// The info line's part about borders; empty when there are none to show.
pub fn borders_info(borders: Option<FoundBorders<'_>>) -> String {
    let Some(b) = borders else { return String::new() };
    let mut names: Vec<String> = Vec::new();
    for c in b.classes {
        let name = render(&match c {
            Class::Black => Message::ClassBlack,
            Class::White => Message::ClassWhite,
            Class::Color => Message::ClassColor,
            Class::Blur => Message::ClassBlur,
        });
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let [left, top, right, bottom] = b.cut;
    let marked = if b.marked { render(&Message::BordersMarked) } else { String::new() };
    render(&Message::BordersInfo {
        names: names.join(", "),
        left: left.to_string(),
        top: top.to_string(),
        right: right.to_string(),
        bottom: bottom.to_string(),
        marked,
    })
}

/// A file's info line: its path, its size on screen and in the file, and the notes on its rating and borders.
pub fn info(
    path: &str,
    shown: (u32, u32),
    bytes: u64,
    rating_kept_in_catalog: bool,
    borders: Option<FoundBorders<'_>>,
) -> String {
    let (w, h) = shown;
    let dims = if w > 0 {
        render(&Message::InfoDims { width: w.to_string(), height: h.to_string() })
    } else {
        render(&Message::InfoNotImported)
    };
    let bytes = pg_model::num::size(bytes);
    let size = if bytes >= 1e6 {
        render(&Message::InfoSizeMb { size: format!("{:.1}", bytes / 1e6) })
    } else {
        render(&Message::InfoSizeKb { size: format!("{:.0}", bytes / 1e3) })
    };
    let kept = if rating_kept_in_catalog { render(&Message::InfoKept) } else { String::new() };
    render(&Message::InfoLine { path: path.to_string(), dims, size, kept, borders: borders_info(borders) })
}

/// A pass the job runs, as the status line names it.
pub enum Pass<'a> {
    Idle,
    Scan { root: &'a str },
    Import,
    Borders,
    Crop,
    Thumbs { height: u32 },
}

/// A pass's counts: its files done and in all, the failures, the rate (files a second) and the time left (seconds).
pub struct Counts {
    pub done: u64,
    pub total: u64,
    pub failed: u64,
    pub rate: f64,
    pub eta_secs: f64,
}

/// The status line's text for a pass's progress; empty when idle.
pub fn pass_status(pass: Pass<'_>, c: &Counts) -> String {
    match pass {
        Pass::Idle => String::new(),
        Pass::Scan { root } => render(&Message::PassScan { root: root.to_string(), count: group_digits(c.done) }),
        Pass::Import => working(&render(&Message::PassImport), c),
        Pass::Borders => working(&render(&Message::PassBorders), c),
        Pass::Crop => working(&render(&Message::PassCrop), c),
        Pass::Thumbs { height } => working(&render(&Message::PassThumbs { height: height.to_string() }), c),
    }
}

/// A pass that counts its files: how far it is, its rate, and what is left.
fn working(what: &str, c: &Counts) -> String {
    let failed =
        if c.failed > 0 { render(&Message::PassFailed { count: group_digits(c.failed) }) } else { String::new() };
    render(&Message::PassWorking {
        what: what.to_string(),
        done: group_digits(c.done),
        total: group_digits(c.total),
        rate: format!("{:.0}", c.rate),
        eta: duration(c.eta_secs),
        failed,
    })
}

/// A count with its thousands grouped: 1,234,567.
pub fn group_digits(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A time left, in seconds, as the status line says it: "42 s left", "3 min left", "1 h 5 min left".
pub fn duration(secs: f64) -> String {
    let s = round_u64(secs);
    render(&match s {
        0..60 => Message::EtaSeconds { seconds: s.to_string() },
        60..3600 => Message::EtaMinutes { minutes: s.div_ceil(60).to_string() },
        _ => Message::EtaHours { hours: (s / 3600).to_string(), minutes: (s % 3600 / 60).to_string() },
    })
}

/// `v` rounded to a whole number, held to `u64`'s range: a negative value gives 0, and one past the range the largest
/// `f64` below 2⁶⁴ (18,446,744,073,709,549,568); NaN gives 0, as a float cast does.
fn round_u64(v: f64) -> u64 {
    pg_model::num::f64_to_u64(v.round().clamp(0.0, 18_446_744_073_709_549_568.0))
}

/// The write-back line: the changes not yet in the files, or the latest failure; empty when there are none.
pub fn write_status(queued: usize, error: Option<&str>) -> String {
    match (queued, error) {
        (0, None) => String::new(),
        (n, None) => render(&Message::WritingChanges { count: n }),
        (n, Some(e)) => render(&Message::ChangesUnwritten { count: n, error: e.to_string() }),
    }
}

/// A root whose folders a scan couldn't read.
pub fn unreadable_folders(root: &str, count: u64) -> String {
    render(&Message::UnreadableFolders { root: root.to_string(), count: count.to_string() })
}

/// A scan that failed.
pub fn scan_failed(root: &str, error: &str) -> String {
    render(&Message::ScanFailed { root: root.to_string(), error: error.to_string() })
}

/// A folder in the library that the scanner refuses (not on a local NTFS disk, or on a network file system).
pub fn root_refused(root: &str, reason: &str) -> String {
    render(&Message::RootRefused { root: root.to_string(), reason: reason.to_string() })
}

/// What a scan of a folder counted: `kind` is the note's kind (`duplicates`, `reparse`, `ambiguous`, `unpaired`,
/// `replaced`, `rebound`), `count` how many.
pub fn scan_note(root: &str, kind: &str, count: u64) -> String {
    render(&Message::ScanNote { root: root.to_string(), kind: kind.to_string(), count: count.to_string() })
}

/// A round of the job that panicked (caught).
pub fn round_panicked(message: &str) -> String {
    render(&Message::RoundPanicked { message: message.to_string() })
}

/// The thumbnail cache is full: `made` thumbnails at `height` px weren't made.
pub fn cache_full(height: u32, made: usize) -> String {
    render(&Message::CacheFull { height: height.to_string(), made: made.to_string() })
}

/// A crop job that ran to the end of its request: what it saved in `folder`, what it skipped and failed, and the first
/// error when some failed.
pub fn cropped(saved: usize, folder: &str, skipped: usize, failed: usize, first_error: Option<&str>) -> String {
    let mut note = render(&Message::CroppedSaved { count: saved, folder: folder.to_string() });
    if skipped > 0 {
        note += "; ";
        note += &render(&Message::CroppedSkipped { count: skipped });
    }
    if let Some(e) = first_error {
        note += "; ";
        note += &render(&Message::CroppedFailed { failed: failed.to_string(), error: e.to_string() });
    }
    note
}

/// The photo to search couldn't be read.
pub fn yandex_unreadable_photo(error: &str) -> String {
    render(&Message::YandexUnreadablePhoto { error: error.to_string() })
}

/// The similar images stopped after `shown` results, for `reason`.
pub fn yandex_similar_stopped(shown: usize, reason: &str) -> String {
    render(&Message::YandexSimilarStopped { shown: shown.to_string(), reason: reason.to_string() })
}

/// The similar images failed, for `reason` (the other sizes are still shown).
pub fn yandex_similar_failed(reason: &str) -> String {
    render(&Message::YandexSimilarFailed { reason: reason.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids of the entries in the English file: the lines `id = ...` that start a message (not a comment, not a
    /// variant's line of a select expression, which is indented).
    fn entries_in_file() -> Vec<&'static str> {
        ENGLISH
            .lines()
            .filter(|line| !line.starts_with(' ') && !line.starts_with('#'))
            .filter_map(|line| line.split_once(" =").map(|(id, _)| id))
            .collect()
    }

    #[test]
    fn the_messages_and_the_english_entries_name_the_same_ids() {
        let mut in_file = entries_in_file();
        in_file.sort_unstable();
        let mut in_code: Vec<&str> = Message::samples().iter().map(Message::id).collect();
        in_code.sort_unstable();
        let unique = in_code.len();
        in_code.dedup();
        assert_eq!(in_code.len(), unique, "two messages share an id");
        assert_eq!(in_file, in_code, "the English file and the Message variants name different entries");
    }

    #[test]
    fn every_message_is_worded_by_its_entry() {
        for message in Message::samples() {
            let line = render(&message);
            assert!(!line.is_empty(), "{} renders empty", message.id());
            assert_ne!(line, message.id(), "{} has no entry", message.id());
        }
    }

    #[test]
    fn the_english_file_parses() {
        assert!(FluentResource::try_new(ENGLISH.to_string()).is_ok(), "text.ftl has a syntax error");
    }

    /// The ids of the entries in `ui.ftl`, the same way as [`entries_in_file`].
    fn ui_entries() -> Vec<&'static str> {
        ENGLISH_UI
            .lines()
            .filter(|line| !line.starts_with(' ') && !line.starts_with('#'))
            .filter_map(|line| line.split_once(" =").map(|(id, _)| id))
            .collect()
    }

    #[test]
    fn the_ui_file_parses_and_names_each_id_once() {
        assert!(FluentResource::try_new(ENGLISH_UI.to_string()).is_ok(), "ui.ftl has a syntax error");
        let mut ids = ui_entries();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "ui.ftl names an id twice");
        assert!(ids.iter().all(|id| id.starts_with("ui-")), "every ui.ftl id starts with ui-");
        assert!(entries_in_file().iter().all(|id| !id.starts_with("ui-")), "text.ftl has a ui- id");
    }

    #[test]
    fn a_ui_line_is_worded_by_its_entry_and_its_values() {
        assert_eq!(ui("ui-save-picked", 2.0, "2", ""), "Save 2 images…");
        assert_eq!(ui("ui-save-picked", 1.0, "1", ""), "Save 1 image…");
        assert_eq!(ui("ui-window-title", 0.0, "D:\\Photos", ""), "Photo Gallery — D:\\Photos");
        assert_eq!(ui("ui-no-such-entry", 0.0, "", ""), "ui-no-such-entry", "a missing entry shows its id");
    }

    #[test]
    fn the_pseudo_locale_accents_letters_and_keeps_digits() {
        let line = pseudo("Save 12 images, Ab");
        assert!(line.starts_with('⟦') && line.ends_with('⟧'), "{line}");
        assert!(line.contains("12"), "{line}");
        assert!(line.contains("Åƀ"), "the capital A is accented as Å and the lowercase b as ƀ: {line}");
        assert!(line.chars().count() > "Save 12 images, Ab".chars().count() + 4, "the pseudo line is padded: {line}");
        assert!(!line.contains("Save"), "the letters are accented: {line}");
    }

    #[test]
    fn timecodes_drop_the_hour_until_there_is_one() {
        assert_eq!(timecode(0), "0:00");
        assert_eq!(timecode(65), "1:05");
        assert_eq!(timecode(3599), "59:59");
        assert_eq!(timecode(3600), "1:00:00");
        assert_eq!(timecode(3725), "1:02:05");
    }

    #[test]
    fn ratings_past_five_show_the_best_symbol() {
        assert_eq!(rating_symbol(0), "☆");
        assert_eq!(rating_symbol(5), "⭐");
        assert_eq!(rating_symbol(9), "⭐");
    }

    #[test]
    fn a_video_badge_rounds_its_length_to_the_second() {
        assert_eq!(badge(MediaType::Video, Some(1499), "mp4"), "▶ 0:01");
        assert_eq!(badge(MediaType::Video, Some(1500), "mp4"), "▶ 0:02");
        assert_eq!(badge(MediaType::Video, None, "mp4"), "▶");
        assert_eq!(badge(MediaType::Animated, None, "png"), "APNG");
        assert_eq!(badge(MediaType::Animated, None, "gif"), "GIF");
        assert_eq!(badge(MediaType::Image, None, "jpg"), "");
    }

    #[test]
    fn counts_are_grouped_and_times_left_are_named() {
        assert_eq!(group_digits(1_234_567), "1,234,567");
        assert_eq!(group_digits(999), "999");
        assert_eq!(duration(42.4), "42 s left");
        assert_eq!(duration(61.0), "2 min left");
        assert_eq!(duration(3900.0), "1 h 5 min left");
    }

    #[test]
    fn the_write_line_counts_its_changes() {
        assert_eq!(write_status(0, None), "");
        assert_eq!(write_status(1, None), "Writing 1 change into files");
        assert_eq!(write_status(3, None), "Writing 3 changes into files");
        assert_eq!(write_status(3, Some("offline")), "3 changes not yet in the files: offline");
        assert_eq!(write_status(1, Some("offline")), "1 change not yet in the files: offline");
    }

    #[test]
    fn a_pass_line_says_how_far_it_is() {
        let c = Counts { done: 1500, total: 4000, failed: 2, rate: 100.0, eta_secs: 25.0 };
        assert_eq!(pass_status(Pass::Import, &c), "Importing 1,500 of 4,000 · 100/s · 25 s left · 2 failed");
        assert_eq!(pass_status(Pass::Idle, &c), "");
    }

    #[test]
    fn the_crop_note_pluralises() {
        assert_eq!(cropped(1, "D:/out", 0, 0, None), "Saved 1 cropped copy in D:/out");
        assert_eq!(
            cropped(2, "D:/out", 2, 1, Some("disk full")),
            "Saved 2 cropped copies in D:/out; 2 files had nothing to crop any more; 1 failed, the first: disk full"
        );
    }

    #[test]
    fn an_info_line_names_what_the_catalog_keeps() {
        let line = info("D:/a.jpg", (0, 0), 2_500_000, true, None);
        assert_eq!(line, "D:/a.jpg   not imported   2.5 MB   rating kept in the catalog");
    }

    #[test]
    fn the_filter_line_and_the_borders_read_as_before() {
        assert_eq!(describe(&Query::default()), "all files");
        let classes = [Class::Black, Class::Black, Class::Color];
        let borders = FoundBorders { marked: true, cut: [1, 2, 3, 4], classes: &classes };
        assert_eq!(borders_info(Some(borders)), "   borders black, colored (L 1 T 2 R 3 B 4 px) — marked not a border");
        assert_eq!(borders_info(None), "");
    }
}
