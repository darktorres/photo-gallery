//! The words the front end shows: the facet labels and the filter line, the grid's badge and info line, the jobs' status
//! and notice lines, the write-back line, the Yandex panel's lines, the rating symbols and the timecodes. Each line is a
//! [`Message`], a typed value with its values; its words are the Fluent entry of the same id in `locales/en/text.ftl`
//! ([`render`]). The functions below build the messages for the values they are given, so this crate needs no Qt and no
//! job type; the desktop maps the job's values in. Nothing here reads the catalog.
//!
//! The words and the bundle are in `words`, the typed lines in `message`; this module builds the lines.

#![forbid(unsafe_code)]

mod message;
mod notice;
#[cfg(test)]
mod test_support;
mod words;

use pg_model::detection::Class;
use pg_model::media::MediaType;
use pg_model::name::Name;
use pg_model::notice::Notice;
use pg_model::progress::{Phase, Progress};
use pg_model::query::{BorderFacet, Choice, DateRange, ListFacet, Query, Range, RatingFilter, Shape, Status};
use pg_model::rules::PathRules;
use pg_model::yandex::{RequestError, SaveFailure, Saved, SearchError, SimilarNote, YandexStatus};

pub use message::{Message, refused_reason, render};
use pg_client::input::day_text;
use pg_client::keys::Hint;
pub use words::{set_pseudo, ui};

/// The drive anchors a folder is on, joined for its tree row: `D: E:`. The model gives the names; the space between them
/// is the front end's to write.
pub fn anchors<'a>(names: impl IntoIterator<Item = &'a Name>) -> String {
    names.into_iter().map(|name| name.display()).collect::<Vec<_>>().join(" ")
}

/// The rating symbols, by rating (0 unrated, 5 best).
pub const RATING_SYMBOLS: [&str; 6] = ["☆", "❌", "❔", "🔨", "✔️", "⭐"];

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

/// What `q` keeps, in one line: "all files" when it filters nothing. Every word is the bundle's: each chosen value is
/// worded as its chip is (`choice_label`), with the case the bundle gives it; a range's ends are Fluent numbers; and
/// the separators between values and between parts are entries too. Only the user's own text (an extension, a name
/// needle) and the dates' digits come from elsewhere.
pub fn describe(q: &Query) -> String {
    let list = |values: Vec<String>| values.join(&render(&Message::FilterListSeparator));
    let chosen = |facet: ListFacet| list(q.chosen(facet).iter().map(choice_label).collect());
    let ranged = |r: &Range| r.min.is_some() || r.max.is_some();
    let dated = |r: &DateRange| r.from.is_some() || r.to.is_some();
    let days = |r: &DateRange| render(&Message::FilterDays { from: day_text(r.from), to: day_text(r.to) });
    let mut parts: Vec<String> = Vec::new();
    if !q.types.is_empty() {
        parts.push(chosen(ListFacet::Types));
    }
    if !q.exts.is_empty() {
        parts.push(chosen(ListFacet::Exts));
    }
    if !q.ratings.is_empty() {
        // A rating is its symbol, which no locale words, or the bundle's "unrated".
        let ratings = q.ratings.iter().map(|&r| match r {
            RatingFilter::Unrated => render(&Message::FilterUnrated),
            RatingFilter::Stars(stars) => rating_symbol(stars.get()).to_string(),
        });
        parts.push(list(ratings.collect()));
    }
    if ranged(&q.size_mb) {
        parts.push(render(&Message::FilterRangeMb { from: q.size_mb.min, to: q.size_mb.max }));
    }
    if ranged(&q.megapixels) {
        parts.push(render(&Message::FilterRangeMp { from: q.megapixels.min, to: q.megapixels.max }));
    }
    if !q.shapes.is_empty() {
        parts.push(chosen(ListFacet::Shapes));
    }
    if dated(&q.taken) {
        parts.push(render(&Message::FilterTaken { days: days(&q.taken) }));
    }
    if dated(&q.modified) {
        parts.push(render(&Message::FilterModified { days: days(&q.modified) }));
    }
    if ranged(&q.duration_s) {
        parts.push(render(&Message::FilterRangeSeconds { from: q.duration_s.min, to: q.duration_s.max }));
    }
    if let Some(d) = q.duplicates {
        parts.push(render(&if d { Message::FilterWithDuplicates } else { Message::FilterWithoutDuplicates }));
    }
    if !q.status.is_empty() {
        parts.push(chosen(ListFacet::Status));
    }
    if !q.borders.is_empty() {
        parts.push(render(&Message::FilterBorders { values: chosen(ListFacet::Borders) }));
    }
    if let Some(m) = q.border_marked {
        parts.push(render(&if m { Message::FilterBordersMarked } else { Message::FilterBordersNotMarked }));
    }
    if !q.name.is_empty() {
        parts.push(render(&Message::FilterName { name: q.name.clone() }));
    }
    if parts.is_empty() { render(&Message::FilterAll) } else { parts.join(&render(&Message::FilterPartSeparator)) }
}

/// The badge on a grid cell: a video's length, an animation's format; nothing for a still image. A video's mark is ►, a
/// glyph the window's font has (text.ftl, `msg-badge-video`).
pub fn badge(media: MediaType, duration_ms: Option<u32>, ext: &str) -> String {
    match media {
        MediaType::Video => match duration_ms {
            Some(ms) => render(&Message::BadgeVideo { time: timecode((u64::from(ms) + 500) / 1000) }),
            None => "►".to_string(),
        },
        MediaType::Animated if ext == "png" => render(&Message::BadgeApng),
        MediaType::Animated => ext.to_uppercase(),
        MediaType::Image => String::new(),
    }
}

/// The borders of a file that a crop would cut: whether the user marked them not a border, the cut in stored pixels
/// (left, top, right, bottom), and the classes of the accepted sides.
pub struct FoundBorders<'a> {
    /// Whether the borders are marked.
    pub marked: bool,
    /// The cut, as (left, top, right, bottom) in pixels.
    pub cut: [u32; 4],
    /// The classes of the accepted sides.
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

/// The name of the pass in `phase`, as the status line and the command line's progress say it ("Importing",
/// "Scanning D:\Photos"); empty when idle. A scanned folder is shown under `rules`, the catalog's.
pub fn pass_name(phase: &Phase, rules: PathRules) -> String {
    match phase {
        Phase::Idle => String::new(),
        Phase::Scan { root } => render(&Message::PassScan { root: rules.display(root) }),
        Phase::Import => render(&Message::PassImport),
        Phase::Hashes => render(&Message::PassHashes),
        Phase::Pairing => render(&Message::PassPairing),
        Phase::Borders => render(&Message::PassBorders),
        Phase::Crop => render(&Message::PassCrop),
        Phase::Thumbs { height } => render(&Message::PassThumbs { height: height.to_string() }),
        Phase::Placeholders => render(&Message::PassPlaceholders),
        Phase::Removing => render(&Message::PassRemoving),
    }
}

/// The status line's text for a pass's progress; empty when idle. A scan counts what it found; the other passes how
/// far they are, their rate and the time left. What failed is said apart ([`pass_failed`]).
pub fn pass_status(p: &Progress, rules: PathRules) -> String {
    let what = pass_name(&p.phase, rules);
    match p.phase {
        Phase::Idle => what,
        Phase::Scan { .. } => render(&Message::PassFound { what, count: group_digits(p.done) }),
        _ => render(&Message::PassWorking {
            what,
            done: group_digits(p.done),
            total: group_digits(p.total),
            rate: format!("{:.0}", p.rate()),
            eta: duration(p.eta()),
        }),
    }
}

/// A lane's row while the lane is paused by itself, or the background work is: the pass's name and how far it is, with
/// no rate or time left (they don't move while it is paused). The drive's label is the row's own, not this line's.
pub fn lane_paused_status(p: &Progress, rules: PathRules) -> String {
    let what = pass_name(&p.phase, rules);
    match p.phase {
        Phase::Idle => render(&Message::LanePaused),
        Phase::Scan { .. } => render(&Message::LanePausedFound { what, count: group_digits(p.done) }),
        // A pass whose total isn't known says its name only: "0 of 0" would read as a finished pass.
        _ if p.total == 0 => render(&Message::PausedPass { status: what }),
        _ => render(&Message::LanePausedPass { what, done: group_digits(p.done), total: group_digits(p.total) }),
    }
}

/// How many files the pass in progress failed on ("2 failed": those it recorded, and those it couldn't read), the status
/// bar's link to its failed files; empty when none did, or for a scan (its folders' failures are a notice of their own).
pub fn pass_failed(p: &Progress) -> String {
    match p.phase {
        Phase::Idle | Phase::Scan { .. } => String::new(),
        _ if p.failed == 0 => String::new(),
        _ => render(&Message::PassFailed { count: group_digits(p.failed) }),
    }
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
        _ => {
            // Rounded up, as the minutes branch rounds them: the hours and the minutes come from one count.
            let minutes = s.div_ceil(60);
            Message::EtaHours { hours: (minutes / 60).to_string(), minutes: (minutes % 60).to_string() }
        }
    })
}

/// `v` rounded to a whole number; the conversion saturates at `u64`'s range (a negative value and NaN give 0).
fn round_u64(v: f64) -> u64 {
    pg_model::num::f64_to_u64(v.round())
}

/// The write-back line: the changes not yet in the files, or the latest failure; empty when there are none.
pub fn write_status(queued: u64, error: Option<&str>) -> String {
    match (queued, error) {
        (0, None) => String::new(),
        (n, None) => render(&Message::WritingChanges { count: n }),
        (0, Some(e)) => render(&Message::WriteQueueFailed { error: e.to_string() }),
        (n, Some(e)) => render(&Message::ChangesUnwritten { count: n, error: e.to_string() }),
    }
}

/// A notice as one line, its title then its details: the command line's, and the inbox's toasts and rows.
pub fn notice_line(notice: &Notice, rules: PathRules) -> String {
    notice_line_with(notice, rules, false)
}

/// A notice as one line of plain text, for the clipboard: the same words, with no bidi isolation marks in the paths (another
/// program pastes the text, and the marks would go with it).
pub fn notice_line_plain(notice: &Notice, rules: PathRules) -> String {
    notice_line_with(notice, rules, true)
}

fn notice_line_with(notice: &Notice, rules: PathRules, plain: bool) -> String {
    let title = render(&Message::NoticeTitle { notice: notice.clone(), rules, plain });
    let body = render(&Message::NoticeBody { notice: notice.clone(), rules, plain });
    render(&Message::NoticeLine { title, body })
}

/// The entry that joins the labels of a key hint line.
pub const HINT_SEPARATOR: &str = "ui-hint-separator";
/// The entry of the status bar's selection line: the item's info, then its rating symbol.
pub const SELECTION_LINE: &str = "ui-selection-line";

/// A key hint line: each label's key and label (`locales/en/ui.ftl`), a space between them, the lines joined by the separator
/// entry.
pub fn hint(labels: &[Hint]) -> String {
    let words: Vec<String> = labels
        .iter()
        .map(|label| {
            let (key, text) = key_and_label(*label);
            if text.is_empty() { key } else { format!("{key} {text}") }
        })
        .collect();
    words.join(&ui(HINT_SEPARATOR, 0.0, "", ""))
}

/// A key hint line as chips: each label's key and its label (empty for a key alone), in the line's order. The viewer shows
/// each key in its own chip, with its label beside it.
pub fn hint_chips(labels: &[Hint]) -> (Vec<String>, Vec<String>) {
    labels.iter().map(|label| key_and_label(*label)).unzip()
}

/// A label's key and the text after it (empty when it has none).
fn key_and_label(label: Hint) -> (String, String) {
    let key = ui(label.key_id(), 0.0, "", "");
    let text = label.label_id().map(|id| ui(id, 0.0, "", "")).unwrap_or_default();
    (key, text)
}

/// A Yandex save that ended: `written` files into `folder`, and when some failed, how many and the first failure.
pub fn yandex_saved(written: usize, folder: &str, failed: usize, first_error: Option<&str>) -> String {
    match first_error {
        None => render(&Message::YandexSaved { count: written, folder: folder.to_string() }),
        Some(e) if written == 0 => {
            render(&Message::YandexSaveNothingWritten { failed: failed.to_string(), error: e.to_string() })
        }
        Some(e) => render(&Message::YandexSavedFailed {
            written: written.to_string(),
            failed: failed.to_string(),
            error: e.to_string(),
        }),
    }
}

/// What the user reads when a Yandex request fails: each error has its own words; the values they carry (the
/// transport's error, what an answer lacked) are the lower layers' text.
pub fn yandex_request(e: &RequestError) -> String {
    match e {
        RequestError::Captcha => render(&Message::YandexCaptcha),
        RequestError::Status(status) => render(&Message::YandexHttpStatus { status: status.to_string() }),
        RequestError::Transport(error) => render(&Message::YandexTransport { error: error.clone() }),
        RequestError::Unexpected(reason) => render(&match reason {
            pg_model::yandex::UnexpectedReason::UploadNotJson => Message::YandexUploadNotJson,
            pg_model::yandex::UnexpectedReason::UploadNoCbirId => Message::YandexUploadNoCbirId,
            pg_model::yandex::UnexpectedReason::UploadNoOriginal => Message::YandexUploadNoOriginal,
            pg_model::yandex::UnexpectedReason::ResultsPageNoState => Message::YandexResultsPageNoState,
            pg_model::yandex::UnexpectedReason::ResultsStateNotJson => Message::YandexResultsStateNotJson,
            pg_model::yandex::UnexpectedReason::SimilarNotJson => Message::YandexSimilarNotJson,
            pg_model::yandex::UnexpectedReason::SimilarNoResults => Message::YandexSimilarNoResults,
            pg_model::yandex::UnexpectedReason::NotAnImage => Message::YandexNotAnImage,
            pg_model::yandex::UnexpectedReason::TooLarge => Message::YandexTooLarge,
            pg_model::yandex::UnexpectedReason::NoResult => Message::YandexNoResult,
            pg_model::yandex::UnexpectedReason::NoSearch => Message::YandexNoSearch,
            pg_model::yandex::UnexpectedReason::NoSearchOnScreen => Message::YandexNoSearchOnScreen,
            pg_model::yandex::UnexpectedReason::NotCalled => Message::YandexNotCalled,
        }),
        RequestError::Recorded(error) => render(&Message::YandexRecorded { error: error.clone() }),
        RequestError::Undecodable(reason) => render(&Message::YandexUndecodable { reason: reason.clone() }),
    }
}

/// Where a Yandex search or save is.
pub fn yandex_phase(phase: pg_model::yandex::Phase) -> String {
    use pg_model::yandex::Phase;
    match phase {
        Phase::Starting => render(&Message::YandexStarting),
        Phase::Uploading => render(&Message::YandexUploading),
        Phase::OtherSizes => render(&Message::YandexOtherSizes),
        Phase::Similar => render(&Message::YandexSimilar),
        Phase::Saving { files } => render(&Message::YandexSaving { count: files }),
        Phase::SavingFile { n, of } => render(&Message::YandexSavingFile { n: n.to_string(), of: of.to_string() }),
    }
}

fn yandex_search_error(e: &SearchError) -> String {
    match e {
        SearchError::NotStill => render(&Message::YandexNotStill),
        SearchError::Unreadable(e) | SearchError::Undecodable(e) => {
            render(&Message::YandexUnreadablePhoto { error: e.clone() })
        }
        SearchError::Request(e) => yandex_request(e),
    }
}

fn yandex_similar_note(note: &SimilarNote) -> String {
    match note {
        SimilarNote::Stopped { shown, error } => {
            render(&Message::YandexSimilarStopped { shown: shown.to_string(), reason: yandex_request(error) })
        }
        SimilarNote::Failed(error) => render(&Message::YandexSimilarFailed { reason: yandex_request(error) }),
    }
}

fn yandex_failure(failure: &SaveFailure) -> String {
    match failure {
        SaveFailure::Client(e) => yandex_request(e),
        SaveFailure::Download { url, error } => {
            render(&Message::YandexDownloadFailed { url: url.clone(), error: yandex_request(error) })
        }
        SaveFailure::Write { url, path, error } => {
            render(&Message::YandexWriteFailed { url: url.clone(), path: path.clone(), error: error.clone() })
        }
    }
}

/// A Yandex search's or save's outcome, as the panel's status line shows it: empty for a search that ended with no note.
pub fn yandex_status(status: &YandexStatus, log: &str) -> String {
    match status {
        YandexStatus::Working(phase) => yandex_phase(*phase),
        YandexStatus::Searched(note) => note.as_ref().map_or_else(String::new, yandex_similar_note),
        YandexStatus::SearchFailed(e) => yandex_search_error(e),
        YandexStatus::Saved(Saved { folder, written, failed, first_error }) => {
            yandex_saved(*written, folder, *failed, first_error.as_ref().map(yandex_failure).as_deref())
        }
        // The panic that ended it is in the log (a thread the system refused is `YandexNotStarted`'s line).
        YandexStatus::Panicked => render(&Message::YandexPanicked { log: log.to_string() }),
    }
}

/// The box for a panic that ends the app: the panic's `message`, then where its details are: in `log`, or, when
/// writing them there failed, why (`log_error`).
pub fn internal_error(message: &str, log: &str, log_error: Option<&str>) -> String {
    let details = match log_error {
        None => render(&Message::DetailsInLog { log: log.to_string() }),
        Some(e) => render(&Message::DetailsNotLogged { log: log.to_string(), error: e.to_string() }),
    };
    format!("{}\n\n{message}\n\n{details}", render(&Message::InternalError))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::notice::Source;
    use pg_model::notice::{LedgerCounts, ScanNote, StartOverReason};
    use pg_model::refusal::RefusedReason;
    use pg_model::write::WriteKind;

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
        assert_eq!(badge(MediaType::Video, Some(1499), "mp4"), "► 0:01");
        assert_eq!(badge(MediaType::Video, Some(1500), "mp4"), "► 0:02");
        assert_eq!(badge(MediaType::Video, None, "mp4"), "►");
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
        assert_eq!(write_status(0, Some("denied")), "A change couldn't be written into its file: denied");
    }

    /// A rendered notice's title and body, under the Windows rules.
    fn worded(notice: &Notice) -> (String, String) {
        let rules = PathRules::Windows;
        (
            render(&Message::NoticeTitle { notice: notice.clone(), rules, plain: false }),
            render(&Message::NoticeBody { notice: notice.clone(), rules, plain: false }),
        )
    }

    fn d(rules: PathRules, path: &str) -> pg_model::rules::Path {
        rules.parse(path).unwrap()
    }

    /// `path` as a notice's words set it: bidi-isolated (U+2068 … U+2069, `notice.rs`).
    fn iso(path: &str) -> String {
        format!("\u{2068}{path}\u{2069}")
    }

    #[test]
    fn the_writers_notices_are_worded() {
        assert_eq!(
            worded(&Notice::WriteDropped { write: WriteKind::Rating, count: 1 }).1,
            "A queued rating was dropped: its file is no longer in the library"
        );
        assert_eq!(
            worded(&Notice::WriteDropped { write: WriteKind::Rotation, count: 3 }).1,
            "3 queued changes were dropped, the latest a turn: their files are no longer in the library"
        );
        assert_eq!(
            worded(&Notice::WriteFailed {
                error: "disk full".into(),
                panicked: false,
                file: Some("D:/a.jpg".into()),
                temp: Some("D:/.a.jpg.rating.pg-write".into()),
            })
            .1,
            format!(
                "{} may be incomplete: copying its change into it failed (disk full). Its whole new content is in {}, \
                 and the copy is tried again.",
                iso("D:/a.jpg"),
                iso("D:/.a.jpg.rating.pg-write")
            )
        );
        assert_eq!(
            worded(&Notice::WriteFailed { error: "boom".into(), panicked: true, file: None, temp: None }).1,
            "internal error on this file: boom. It is tried again later."
        );
    }

    #[test]
    fn the_panic_words_are_exact() {
        // The bundle's isolation is off (`words.rs`), so a panic's message is set plain.
        assert_eq!(super::message::file_panicked("boom"), "internal error on this file: boom");
        let (title, body) = worded(&Notice::RoundPanicked { message: "boom".into() });
        assert_eq!(title, "Internal error in the background job");
        assert_eq!(body, "boom. The job went on with the next request; the details are in the log.");
    }
    #[test]
    fn a_pass_line_says_how_far_it_is() {
        let rules = PathRules::Windows;
        let elapsed = std::time::Duration::from_secs(15);
        let since = Default::default();
        let p = Progress { phase: Phase::Import, done: 1500, total: 4000, failed: 2, bytes: 0, elapsed, since };
        assert_eq!(pass_status(&p, rules), "Importing 1,500 of 4,000 · 100/s · 25 s left");
        assert_eq!(pass_failed(&p), "2 failed");
        let height = pg_model::size::ThumbHeight::new(256).expect("a height in range");
        let thumbs = Progress { phase: Phase::Thumbs { height }, failed: 0, ..p.clone() };
        assert_eq!(pass_status(&thumbs, rules), "Thumbnails at 256 px 1,500 of 4,000 · 100/s · 25 s left");
        assert_eq!(pass_failed(&thumbs), "");
        let root = rules.parse(r"D:\Photos").unwrap();
        let scan = Progress { phase: Phase::Scan { root }, done: 1234, total: 0, ..p.clone() };
        assert_eq!(pass_status(&scan, rules), "Scanning D:\\Photos · 1,234 images found");
        assert_eq!(pass_failed(&scan), "", "a scan's failures are its notice's");
        assert_eq!(pass_name(&scan.phase, rules), "Scanning D:\\Photos");
        assert_eq!(pass_status(&Progress { phase: Phase::Idle, ..p }, rules), "");
    }

    #[test]
    fn a_paused_lane_says_its_pass_with_its_count_and_no_rate_or_time_left() {
        let rules = PathRules::Windows;
        assert_eq!(lane_paused_status(&Progress::idle(), rules), "Paused");
        let working = Progress { phase: Phase::Import, done: 10, total: 40, ..Progress::idle() };
        assert_eq!(lane_paused_status(&working, rules), "Paused · Importing 10 of 40");
        let unknown = Progress { phase: Phase::Import, done: 10, total: 0, ..Progress::idle() };
        assert_eq!(lane_paused_status(&unknown, rules), "Paused · Importing", "an unknown total says its name only");
        let root = rules.parse(r"D:\Photos").unwrap();
        let scan = Progress { phase: Phase::Scan { root }, done: 1234, total: 0, ..Progress::idle() };
        assert_eq!(lane_paused_status(&scan, rules), "Paused · Scanning D:\\Photos · 1,234 images found");
    }

    /// The clipboard's line has no isolation marks in its paths; otherwise it is the same line.
    #[test]
    fn a_plain_notice_line_has_no_isolation_marks() {
        let notice = Notice::FolderAlreadyAdded { folder: PathRules::Windows.parse(r"D:\Photos").unwrap() };
        let plain = notice_line_plain(&notice, PathRules::Windows);
        assert!(plain.contains(r"D:\Photos"), "{plain}");
        assert!(!plain.contains('\u{2068}') && !plain.contains('\u{2069}'), "{plain}");
        let isolated = notice_line(&notice, PathRules::Windows);
        assert!(isolated.contains('\u{2068}'), "{isolated}");
        assert_eq!(plain, isolated.replace(['\u{2068}', '\u{2069}'], ""));
    }

    #[test]
    fn the_crop_notice_says_what_failed() {
        let cropped = |failed, failures| Notice::Cropped {
            folder: "D:/out".into(),
            saved: 2,
            skipped: 1,
            failed,
            failures,
            unfinished: 0,
            changed: 0,
            missing: 0,
        };
        let out = iso("D:/out");
        assert_eq!(worded(&cropped(0, Vec::new())).1, format!("2 saved in {out}; 1 had nothing to crop; none failed"));
        let first = pg_model::notice::FileFailure {
            path: PathRules::Windows.parse(r"D:\a.jpg").unwrap(),
            error: "disk full".into(),
            panicked: false,
        };
        assert_eq!(
            worded(&cropped(1, vec![first])).1,
            format!("2 saved in {out}; 1 had nothing to crop; 1 failed, the first: {}: disk full", iso(r"D:\a.jpg"))
        );
    }

    #[test]
    fn a_stopped_crop_says_how_many_were_not_reached() {
        let stopped = Notice::Cropped {
            folder: "D:/out".into(),
            saved: 2,
            skipped: 1,
            failed: 0,
            failures: Vec::new(),
            unfinished: 4,
            changed: 0,
            missing: 0,
        };
        let out = iso("D:/out");
        assert_eq!(
            worded(&stopped).1,
            format!("2 saved in {out}; 1 had nothing to crop; none failed; 4 not reached: the pass was stopped first")
        );
        // Files that changed or went are not "nothing to crop": they are said as their own counts.
        let gone = Notice::Cropped {
            folder: "D:/out".into(),
            saved: 2,
            skipped: 1,
            failed: 0,
            failures: Vec::new(),
            unfinished: 0,
            changed: 1,
            missing: 2,
        };
        assert_eq!(
            worded(&gone).1,
            format!(
                "2 saved in {out}; 1 had nothing to crop; 1 changed on disk since their borders were found, so left as they \
                 are; 2 no longer in the library; none failed"
            )
        );
    }

    #[test]
    fn the_library_lines_are_worded() {
        let rules = PathRules::Windows;
        assert_eq!(
            worded(&Notice::FolderAlreadyAdded { folder: d(rules, r"D:\Photos") }).0,
            format!("{} is already in the library", iso(r"D:\Photos"))
        );
        assert_eq!(
            worded(&Notice::FolderRefused { folder: d(rules, r"Z:\Photos"), reason: RefusedReason::NetworkDrive }).1,
            format!("{} can't be added: its drive is a network drive", iso(r"Z:\Photos"))
        );
        assert_eq!(
            worded(&Notice::LibraryUnavailable { error: "no thread".into() }).1,
            "Nothing is scanned, imported or written into the files: no thread"
        );
        assert_eq!(
            worded(&Notice::ScanNote { root: d(rules, r"D:\p"), note: ScanNote::Duplicates, count: 2 }),
            (
                "Duplicate paths skipped".to_string(),
                format!("{}: 2 files were skipped as duplicates (another path to the same file)", iso(r"D:\p"))
            )
        );
        // The scans' folder ledger is one entry: its counts when the inbox is viewed, and words of its own without.
        let folders = |counts| Notice::ScanFoldersFailed { counts };
        assert_eq!(
            worded(&folders(Some(LedgerCounts { new: 2, earlier: 1 }))),
            ("Folders couldn't be scanned".to_string(), "2 new, 1 earlier".to_string())
        );
        assert_eq!(
            worded(&folders(None)).1,
            "Some folders couldn't be read, or the scan refuses them, so their files aren't in the library"
        );
        assert_eq!(
            notice_line(&Notice::RoundFailed { error: "disk full".into() }, rules),
            "The background job failed: disk full"
        );
        assert_eq!(render(&Message::SourceLabel { source: Source::Thumbs }), "Thumbnails");
    }
    /// Each refusal has words of its own: none falls to the entry's default, and no two read the same.
    #[test]
    fn every_refusal_reason_is_worded() {
        use pg_model::refusal::Fact;
        let unreadable = |fact| RefusedReason::Unreadable { fact, error: "Access is denied.".into() };
        let reasons = [
            RefusedReason::NetworkPath,
            unreadable(Fact::Volume),
            unreadable(Fact::Folder),
            unreadable(Fact::RealPath),
            unreadable(Fact::RealVolume),
            unreadable(Fact::FileSystem),
            unreadable(Fact::Drive),
            RefusedReason::NetworkDrive,
            RefusedReason::RemovableDrive,
            RefusedReason::OtherDrive,
            RefusedReason::Link,
            RefusedReason::RealNotLocal,
            RefusedReason::RealOnOtherVolume,
            RefusedReason::NotNtfs { file_system: "FAT32".into() },
            RefusedReason::OpaqueDisk,
            RefusedReason::Nfs,
            RefusedReason::Smb,
            RefusedReason::Fat,
            RefusedReason::ExFat,
            RefusedReason::Pseudo { file_system: "proc".into() },
            RefusedReason::UnsupportedSystem,
        ];
        let fallback =
            render(&Message::RefusedReason { code: "no-such-code".into(), file_system: "".into(), error: "".into() });
        let mut all: Vec<String> = reasons.iter().map(message::refused_reason).collect();
        for (reason, words) in reasons.iter().zip(&all) {
            assert_ne!(words, &fallback, "{reason:?} falls to the default");
        }
        assert_eq!(message::refused_reason(&RefusedReason::RemovableDrive), "its drive is removable");
        assert_eq!(
            message::refused_reason(&RefusedReason::NotNtfs { file_system: "FAT32".into() }),
            "its file system is FAT32, not NTFS"
        );
        assert_eq!(message::refused_reason(&unreadable(Fact::Volume)), "its volume can't be read: Access is denied.");
        assert_eq!(
            worded(&Notice::FolderRefused { folder: d(PathRules::Windows, r"D:\p"), reason: RefusedReason::Link }).1,
            format!("{} can't be added: it is a link (a junction or a symbolic link)", iso(r"D:\p"))
        );
        all.sort();
        all.dedup();
        assert_eq!(all.len(), reasons.len(), "two reasons read the same");
    }

    #[test]
    fn the_yandex_lines_are_worded() {
        assert_eq!(render(&Message::YandexStarting), "Starting…");
        use pg_model::yandex::Phase;
        assert_eq!(yandex_phase(Phase::Saving { files: 1 }), "Saving 1 file…");
        assert_eq!(yandex_phase(Phase::Saving { files: 3 }), "Saving 3 files…");
        assert_eq!(yandex_phase(Phase::SavingFile { n: 2, of: 5 }), "Saving 2 of 5…");
        let failed = Saved {
            folder: "D:/out".into(),
            written: 2,
            failed: 1,
            first_error: Some(SaveFailure::Download { url: "u".into(), error: RequestError::Captcha }),
        };
        assert_eq!(
            yandex_status(&YandexStatus::Saved(failed), ""),
            yandex_saved(
                2,
                "D:/out",
                1,
                Some(&yandex_failure(&SaveFailure::Download { url: "u".into(), error: RequestError::Captcha }))
            )
        );
        assert_eq!(yandex_status(&YandexStatus::Searched(None), ""), "", "a search with no note shows nothing");
        assert_eq!(yandex_saved(1, "D:/out", 0, None), "Saved 1 file to D:/out");
        assert_eq!(yandex_saved(3, "D:/out", 0, None), "Saved 3 files to D:/out");
        assert_eq!(yandex_saved(2, "D:/out", 1, Some("disk full")), "Saved 2, 1 failed. First failure: disk full");
        // Nothing written: not "Saved 0".
        assert_eq!(yandex_saved(0, "D:/out", 3, Some("denied")), "Nothing was saved: 3 failed. First failure: denied");
        let download = SaveFailure::Download { url: "https://a/b.jpg".into(), error: RequestError::Status(404) };
        assert_eq!(yandex_failure(&download), "https://a/b.jpg couldn't be downloaded: Yandex answered HTTP 404");
        let write = SaveFailure::Write {
            url: "https://a/b.jpg".into(),
            path: "D:/out/b.jpg".into(),
            error: "disk full".into(),
        };
        assert_eq!(yandex_failure(&write), "https://a/b.jpg couldn't be written to D:/out/b.jpg: disk full");
        assert_eq!(
            render(&Message::YandexPanicked { log: r"C:\gallery.log".into() }),
            "Ended by an internal error; the details are in C:\\gallery.log."
        );
        assert_eq!(yandex_request(&RequestError::Status(503)), "Yandex answered HTTP 503");
        assert_eq!(yandex_request(&RequestError::Transport("timed out".into())), "Yandex request failed: timed out");
        assert_eq!(
            yandex_request(&RequestError::Unexpected(pg_model::yandex::UnexpectedReason::UploadNoCbirId)),
            "Yandex's answer to the upload has no image id"
        );
        assert_eq!(
            yandex_request(&RequestError::Recorded("no such step".into())),
            "recorded Yandex session: no such step"
        );
        assert_eq!(
            render(&Message::YandexNotStill),
            "Yandex search takes still images (JPEG, PNG, WebP, GIF, BMP or TIFF)."
        );
        assert_eq!(
            render(&Message::YandexNotStarted { error: "thread limit".into() }),
            "Couldn't start the search or the save: thread limit"
        );
        assert_eq!(render(&Message::YandexNothingPicked), "Pick at least one image to save.");
        assert_eq!(render(&Message::YandexNoFolder), "No folder was chosen, so nothing was saved.");
    }

    #[test]
    fn the_start_and_crash_boxes_are_worded() {
        assert_eq!(render(&Message::CatalogUnopened { error: "locked".into() }), "Can't open the catalog: locked");
        assert_eq!(
            render(&Message::CatalogInUse { path: "D:/c/catalog.sqlite".into() }),
            "Another program has the catalog at D:/c/catalog.sqlite open, so it can't be changed until it is closed."
        );
        assert_eq!(
            render(&Message::CatalogFolderMissing { path: "E:/c".into() }),
            "The catalog folder E:/c was not found. Is its drive connected? Choose a folder that exists, or use the default."
        );
        assert_eq!(
            render(&Message::CatalogReadOnly { path: "D:/c".into() }),
            "The catalog at D:/c needs a change, but it can't be written (it, or its folder, is read-only or permission is denied). Make it writable, or open a copy of it."
        );
        assert_eq!(
            render(&Message::CatalogPathTooLong { path: "D:/c".into() }),
            "The catalog's folder path is too long for Windows: D:/c. Choose a shorter folder for the catalog (its files are kept beside it)."
        );
        assert_eq!(
            render(&Message::CatalogPredatesKeyVersion { path: "D:/c".into() }),
            "The catalog at D:/c has a schema that differs from this build's, and it predates key_version. Restore a .pre-v*.bak or start fresh."
        );
        assert_eq!(
            render(&Message::CatalogPathRules {
                path: "D:/c".into(),
                detail: "made with the windows path rules".into(),
                wanted: "posix".into(),
            }),
            "The catalog at D:/c was made with the windows path rules, but this build uses the posix path rules. Open it on the platform that made it, or move it aside and scan again."
        );
        assert_eq!(
            render(&Message::CatalogNewerSchema {
                path: "D:/c".into(),
                found: "4".into(),
                current: "3".into(),
                built_by: " by build 0.1.0-dev".into(),
            }),
            "The catalog at D:/c was made by a later build (its schema is version 4; this build's is 3) by build 0.1.0-dev. Open it with that build, or restore a backup by hand (rename it over catalog.sqlite)."
        );
        assert_eq!(
            internal_error("boom", r"C:\gallery.log", None),
            "Photo Gallery hit an internal error:\n\nboom\n\nThe details are in C:\\gallery.log."
        );
        assert_eq!(
            internal_error("boom", r"C:\gallery.log", Some("denied")),
            "Photo Gallery hit an internal error:\n\nboom\n\nWriting them to C:\\gallery.log failed (denied)."
        );
    }

    #[test]
    fn a_started_over_cache_says_why_and_where_the_old_file_is() {
        let over = |reason, path: &str| Notice::CacheStartedOver { reason, path: path.into(), detail: "x".into() };
        assert_eq!(
            worded(&over(StartOverReason::Damaged, r"C:\cat\thumbs.sqlite.bad")).1,
            format!(
                "The cache was damaged, so it was started over; its thumbnails are made again. The old file is kept as {}",
                iso(r"C:\cat\thumbs.sqlite.bad")
            )
        );
        assert_eq!(
            worded(&over(StartOverReason::OtherCatalog, r"C:\cat\thumbs.sqlite")).1,
            format!(
                "The cache {} was made for another catalog, so it was started over; its thumbnails are made again",
                iso(r"C:\cat\thumbs.sqlite")
            )
        );
    }
    /// A ledger entry back after a dismissal says how many failures came since. A ledger's files and folders are worded
    /// for the Copy, a folder by its reason's code.
    #[test]
    fn a_ledgers_counts_and_lines_are_worded() {
        let since = |count| render(&Message::LedgerSinceDismissed { count });
        assert_eq!(since(1), "1 since you dismissed it");
        assert_eq!(since(12), "12 since you dismissed it");
        let file = Message::FailedFileLine { path: r"D:\a.jpg".into(), error: "not a JPEG".into() };
        assert_eq!(render(&file), r"D:\a.jpg: not a JPEG");
        let reason = |reason: &str| render(&Message::FailedFolderReason { reason: reason.into() });
        assert_eq!(reason("unreadable"), "it couldn't be read");
        assert_eq!(reason("mount-refused"), "it is a mount the scan refuses");
        assert_eq!(reason("refused"), "the scan refuses it");
        let folder = Message::FailedFolderLine {
            path: r"D:\p\q".into(),
            why: reason("unreadable"),
            error: "Access is denied.".into(),
        };
        assert_eq!(render(&folder), r"D:\p\q: it couldn't be read (Access is denied.)");
        assert_eq!(render(&Message::FailedFolderGroup { root: r"D:\p".into() }), r"In D:\p:");
    }

    #[test]
    fn the_key_hints_are_worded() {
        use pg_client::keys::{self, Media, Shown};
        assert_eq!(
            hint(&keys::grid_hint(true)),
            "Click to select · double-click, Enter or Space to view · 0–5 rate · R / Shift+R turn · arrows move · F filters · \
             B borders · N not a border · Ctrl+wheel or Ctrl+/− resize"
        );
        let still = Shown { has_borders: false, still: true, video: false };
        let chips = |media| {
            let (keys, labels) = hint_chips(&keys::viewer_hint(media, still));
            keys.into_iter().zip(labels).collect::<Vec<(String, String)>>()
        };
        let pair = |key: &str, label: &str| (key.to_string(), label.to_string());
        assert_eq!(
            chips(Media::Still),
            vec![
                pair("← →", "step"),
                pair("0–5", "rate"),
                pair("↑ 4", ""),
                pair("↓ 1", ""),
                pair("R ⟲", ""),
                pair("Shift+R ⟳", ""),
                pair("pinch or + −", "zoom"),
                pair("double-click", "1:1"),
                pair("Z", "real size"),
                pair("L", "loupe"),
                pair("B", "borders"),
                pair("H", "hide"),
                pair("Esc", "close"),
            ]
        );
        assert_eq!(
            chips(Media::Video)[..3].to_vec(),
            vec![pair("← →", "step"), pair("K", "play/pause"), pair("M", "mute")]
        );
        assert_eq!(
            hint(&keys::viewer_hint(Media::Video, still)),
            "← → step · K play/pause · M mute · 0–5 rate · ↑ 4 · ↓ 1 · R ⟲ · Shift+R ⟳ · H hide · Esc close"
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
        // Each value as its chip words it (a shape too, which was its key), in the bundle's case; numbers and
        // separators from the bundle.
        let q = Query {
            size_mb: Range { min: Some(1.5), max: None },
            shapes: vec![Shape::Landscape],
            status: vec![Status::Imported, Status::Failed],
            borders: vec![BorderFacet::Black],
            ..Query::default()
        };
        assert_eq!(describe(&q), "1.5– MB · Landscape · Imported, Unreadable · borders: Black");
        let classes = [Class::Black, Class::Black, Class::Color];
        let borders = FoundBorders { marked: true, cut: [1, 2, 3, 4], classes: &classes };
        assert_eq!(borders_info(Some(borders)), "   borders black, colored (L 1 T 2 R 3 B 4 px) — marked not a border");
        assert_eq!(borders_info(None), "");
    }
}
