//! The words the front end shows: the facet labels and the filter line, the grid's badge and info line, the jobs' status
//! and notice lines, the write-back line, the Yandex panel's lines, the rating symbols and the timecodes. Each line is a
//! [`Message`], a typed value with its values; its words are the Fluent entry of the same id in `locales/en/text.ftl`
//! ([`render`]). The functions below build the messages for the values they are given, so this crate needs no Qt and no
//! job type; the desktop maps the job's values in. Nothing here reads the catalog.
//!
//! The words and the bundle are in `words`, the typed lines in `message`; this module builds the lines.

mod message;
#[cfg(test)]
mod test_support;
mod words;

use pg_model::detection::Class;
use pg_model::progress::{Phase, Progress};
use pg_model::query::{
    BorderFacet, Choice, DateRange, ListFacet, MediaType, Query, Range, RatingFilter, Shape, Status,
};
use pg_model::refusal::RefusedReason;
use pg_model::rules::PathRules;
use pg_model::write::WriteKind;

use crate::input::day_text;
pub use message::{CacheStartOver, Message, render};
pub use words::{set_pseudo, ui};

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

/// The name of the pass in `phase`, as the status line and the command line's progress say it ("Importing",
/// "Scanning D:\Photos"); empty when idle. A scanned folder is shown under `rules`, the catalog's.
pub fn pass_name(phase: &Phase, rules: PathRules) -> String {
    match phase {
        Phase::Idle => String::new(),
        Phase::Scan { root } => render(&Message::PassScan { root: rules.display(root) }),
        Phase::Import => render(&Message::PassImport),
        Phase::Borders => render(&Message::PassBorders),
        Phase::Crop => render(&Message::PassCrop),
        Phase::Thumbs { height } => render(&Message::PassThumbs { height: height.to_string() }),
    }
}

/// The status line's text for a pass's progress; empty when idle. A scan counts what it found; the other passes how
/// far they are, their rate and the time left.
pub fn pass_status(p: &Progress, rules: PathRules) -> String {
    let what = pass_name(&p.phase, rules);
    match p.phase {
        Phase::Idle => what,
        Phase::Scan { .. } => render(&Message::PassFound { what, count: group_digits(p.done) }),
        _ => {
            let failed = if p.failed > 0 {
                render(&Message::PassFailed { count: group_digits(p.failed) })
            } else {
                String::new()
            };
            render(&Message::PassWorking {
                what,
                done: group_digits(p.done),
                total: group_digits(p.total),
                rate: format!("{:.0}", p.rate()),
                eta: duration(p.eta()),
                failed,
            })
        }
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
        (0, Some(e)) => render(&Message::WriteQueueFailed { error: e.to_string() }),
        (n, Some(e)) => render(&Message::ChangesUnwritten { count: n, error: e.to_string() }),
    }
}

/// A copy into a file that failed up to the cap after it may have started: the file may be incomplete, and `temp`
/// holds its whole new content (`error`: why the copy failed). The writer tries it again later.
pub fn write_incomplete(file: &str, temp: &str, error: &str) -> String {
    render(&Message::WriteIncomplete { file: file.to_string(), temp: temp.to_string(), error: error.to_string() })
}

/// Queued writes dropped because their files left the library: `count` since the last note, the latest of `kind`.
pub fn write_dropped(kind: WriteKind, count: usize) -> String {
    render(&Message::WriteDropped { kind, count })
}

/// The writer died too often in a row and isn't started again until the app is: the changes wait in the catalog.
pub fn write_stopped() -> String {
    render(&Message::WriteStopped)
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
pub fn root_refused(root: &str, reason: &RefusedReason) -> String {
    render(&Message::RootRefused { root: root.to_string(), reason: refused_reason(reason) })
}

/// A folder under a scanned root that the scanner does not walk into, because its file system is refused.
pub fn mount_refused(mount: &str, reason: &RefusedReason) -> String {
    render(&Message::MountRefused { mount: mount.to_string(), reason: refused_reason(reason) })
}

/// Why the scanner refuses a folder, as the refusal lines say it ("its drive is removable").
pub fn refused_reason(reason: &RefusedReason) -> String {
    let (file_system, error) = match reason {
        RefusedReason::NotNtfs { file_system } | RefusedReason::Pseudo { file_system } => (file_system.clone(), ""),
        RefusedReason::Unreadable { error, .. } => (String::new(), error.as_str()),
        _ => (String::new(), ""),
    };
    render(&Message::RefusedReason { code: reason.code().to_string(), file_system, error: error.to_string() })
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

/// A file a job panicked on (caught, so the job went on), with the panic's message: a crop's or a write's failure.
pub fn file_panicked(message: &str) -> String {
    render(&Message::FilePanicked { message: message.to_string() })
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

/// The photo to search couldn't be read, or (a TIFF) decoded to be sent as a JPEG.
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

/// A search or a save couldn't start: the system refused its thread, for `error`.
pub fn yandex_not_started(error: &str) -> String {
    render(&Message::YandexNotStarted { error: error.to_string() })
}

/// The folder dialog couldn't open, or didn't give a folder, for `error`: nothing was chosen.
pub fn folder_dialog_failed(error: &str) -> String {
    render(&Message::FolderDialogFailed { error: error.to_string() })
}

/// The save was asked with no image picked.
pub fn yandex_nothing_picked() -> String {
    render(&Message::YandexNothingPicked)
}

/// The save's folder dialog gave no folder: nothing is saved.
pub fn yandex_no_folder() -> String {
    render(&Message::YandexNoFolder)
}

/// The app's own folder dialog can't list a folder, for `error`.
pub fn folder_unreadable(error: &str) -> String {
    render(&Message::FolderUnreadable { error: error.to_string() })
}

/// The thumbnail cache was started over when it was opened, for `reason`: `path` is where the old file is kept (a
/// damaged one, moved aside) or the cache file made anew (one this build can't use, deleted).
pub fn thumb_cache_started_over(reason: CacheStartOver, path: &str) -> String {
    render(&Message::ThumbCacheStartedOver { reason, path: path.to_string() })
}

/// The library's status line while the background work is paused: `pass` is the running pass's line, empty when none
/// runs.
pub fn paused_status(pass: &str) -> String {
    if pass.is_empty() {
        render(&Message::WorkPaused)
    } else {
        render(&Message::PausedPass { status: pass.to_string() })
    }
}

/// The newest notice from the job, `notice`, with the `more` others that came in since the last one shown (none: the
/// notice as it is).
pub fn notice_more(notice: String, more: usize) -> String {
    if more == 0 { notice } else { render(&Message::NoticeMore { notice, count: more }) }
}

/// A folder added that was already in the library: it is rescanned.
pub fn folder_already_added(folder: &str) -> String {
    render(&Message::FolderAlreadyAdded { folder: folder.to_string() })
}

/// A folder not added, because the scanner refuses it for `reason` (not on a local NTFS disk, or on a network file
/// system).
pub fn folder_refused(folder: &str, reason: &RefusedReason) -> String {
    render(&Message::FolderRefused { folder: folder.to_string(), reason: refused_reason(reason) })
}

/// A folder that couldn't be added, for `error`.
pub fn folder_not_added(error: &str) -> String {
    render(&Message::FolderNotAdded { error: error.to_string() })
}

/// A folder that couldn't be removed from the library, for `error`.
pub fn folder_not_removed(error: &str) -> String {
    render(&Message::FolderNotRemoved { error: error.to_string() })
}

/// A rescan of the library that couldn't start, for `error`.
pub fn rescan_failed(error: &str) -> String {
    render(&Message::RescanFailed { error: error.to_string() })
}

/// The background work (the job and the writer) couldn't start, for `error`.
pub fn library_unavailable(error: &str) -> String {
    render(&Message::LibraryUnavailable { error: error.to_string() })
}

/// The library's folders couldn't be read from the catalog, for `error`.
pub fn folders_unreadable(error: &str) -> String {
    render(&Message::FoldersUnreadable { error: error.to_string() })
}

/// A round of the background job that failed, for `error`.
pub fn round_failed(error: &str) -> String {
    render(&Message::RoundFailed { error: error.to_string() })
}

/// A crop job that failed before it could save anything, for `error`.
pub fn crop_failed(error: &str) -> String {
    render(&Message::CropFailed { error: error.to_string() })
}

/// A Yandex save of `files` results about to start.
pub fn yandex_saving(files: usize) -> String {
    render(&Message::YandexSaving { count: files })
}

/// The `n`th of `of` results of a Yandex save being downloaded (counted from 1).
pub fn yandex_saving_file(n: usize, of: usize) -> String {
    render(&Message::YandexSavingFile { n: n.to_string(), of: of.to_string() })
}

/// A Yandex save that ended: `written` files into `folder`, and when some failed, how many and the first failure.
pub fn yandex_saved(written: usize, folder: &str, failed: usize, first_error: Option<&str>) -> String {
    match first_error {
        None => render(&Message::YandexSaved { count: written, folder: folder.to_string() }),
        Some(e) => render(&Message::YandexSavedFailed {
            written: written.to_string(),
            failed: failed.to_string(),
            error: e.to_string(),
        }),
    }
}

/// A Yandex result at `url` that couldn't be downloaded, for `error`.
pub fn yandex_download_failed(url: &str, error: &str) -> String {
    render(&Message::YandexDownloadFailed { url: url.to_string(), error: error.to_string() })
}

/// A Yandex result at `url`, downloaded, that couldn't be written to `path`, for `error`.
pub fn yandex_write_failed(url: &str, path: &str, error: &str) -> String {
    render(&Message::YandexWriteFailed { url: url.to_string(), path: path.to_string(), error: error.to_string() })
}

/// Yandex answered with the HTTP `status`, not a success.
pub fn yandex_http_status(status: u16) -> String {
    render(&Message::YandexHttpStatus { status: status.to_string() })
}

/// A Yandex request that failed to connect, in TLS or by a timeout, for `error`.
pub fn yandex_transport(error: &str) -> String {
    render(&Message::YandexTransport { error: error.to_string() })
}

/// A Yandex answer without the data the app reads: `detail` says what was missing.
pub fn yandex_unexpected(detail: &str) -> String {
    render(&Message::YandexUnexpected { detail: detail.to_string() })
}

/// A recorded Yandex session that can't answer, for `error`.
pub fn yandex_recorded(error: &str) -> String {
    render(&Message::YandexRecorded { error: error.to_string() })
}

/// A result's file that is not an image this app decodes, for `reason`.
pub fn yandex_undecodable(reason: &str) -> String {
    render(&Message::YandexUndecodable { reason: reason.to_string() })
}

/// The catalog couldn't be opened at start, for `error`.
pub fn catalog_unopened(error: &str) -> String {
    render(&Message::CatalogUnopened { error: error.to_string() })
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
        assert_eq!(write_status(0, Some("denied")), "A change couldn't be written into its file: denied");
    }

    #[test]
    fn a_dropped_write_and_an_incomplete_copy_are_worded() {
        assert_eq!(
            write_dropped(WriteKind::Rating, 1),
            "A queued rating was dropped: its file is no longer in the library"
        );
        assert_eq!(
            write_dropped(WriteKind::Turns, 3),
            "3 queued changes were dropped, the latest a turn: their files are no longer in the library"
        );
        assert_eq!(
            write_stopped(),
            "Writing changes into the files has stopped after repeated failures. The changes are kept, and are written when the app starts again"
        );
        assert_eq!(
            write_incomplete("D:/a.jpg", "D:/.a.jpg.rating.pg-write", "disk full"),
            "D:/a.jpg may be incomplete: copying its change into it failed (disk full). Its whole new content is in \
             D:/.a.jpg.rating.pg-write, and the copy is tried again"
        );
    }

    #[test]
    fn a_pass_line_says_how_far_it_is() {
        let rules = PathRules::Windows;
        let elapsed = std::time::Duration::from_secs(15);
        let p = Progress { phase: Phase::Import, done: 1500, total: 4000, failed: 2, bytes: 0, elapsed };
        assert_eq!(pass_status(&p, rules), "Importing 1,500 of 4,000 · 100/s · 25 s left · 2 failed");
        let thumbs = Progress { phase: Phase::Thumbs { height: 256 }, failed: 0, ..p.clone() };
        assert_eq!(pass_status(&thumbs, rules), "Thumbnails at 256 px 1,500 of 4,000 · 100/s · 25 s left");
        let root = rules.parse(r"D:\Photos").unwrap();
        let scan = Progress { phase: Phase::Scan { root }, done: 1234, total: 0, ..p.clone() };
        assert_eq!(pass_status(&scan, rules), "Scanning D:\\Photos · 1,234 images found");
        assert_eq!(pass_name(&scan.phase, rules), "Scanning D:\\Photos");
        assert_eq!(pass_status(&Progress { phase: Phase::Idle, ..p }, rules), "");
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
    fn the_library_lines_are_worded() {
        assert_eq!(paused_status(""), "Background work paused");
        assert_eq!(paused_status("Importing"), "Paused · Importing");
        assert_eq!(notice_more("D:/p: offline".to_string(), 0), "D:/p: offline");
        assert_eq!(notice_more("D:/p: offline".to_string(), 1), "D:/p: offline (and 1 more)");
        assert_eq!(notice_more("D:/p: offline".to_string(), 3), "D:/p: offline (and 3 more)");
        assert_eq!(folder_already_added(r"D:\Photos"), "D:\\Photos is already in the library: rescanning it");
        assert_eq!(
            folder_refused(r"Z:\Photos", &RefusedReason::NetworkDrive),
            "Z:\\Photos can't be added: its drive is a network drive"
        );
        assert_eq!(folder_not_added("database is locked"), "The folder couldn't be added: database is locked");
        assert_eq!(
            folder_not_removed("database is locked"),
            "The folder couldn't be removed from the library: database is locked"
        );
        assert_eq!(rescan_failed("database is locked"), "The library couldn't be scanned again: database is locked");
        assert_eq!(
            library_unavailable("no thread"),
            "The background work couldn't start, so nothing is scanned, imported or written into the files: no thread"
        );
        assert_eq!(folders_unreadable("locked"), "The library's folders couldn't be read: locked");
        assert_eq!(round_failed("disk full"), "The background job failed: disk full");
        assert_eq!(crop_failed("disk full"), "Saving cropped copies failed: disk full");
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
            RefusedReason::NetworkDrive,
            RefusedReason::RemovableDrive,
            RefusedReason::OtherDrive,
            RefusedReason::Link,
            RefusedReason::RealNotLocal,
            RefusedReason::RealOnOtherVolume,
            RefusedReason::NotNtfs { file_system: "FAT32".into() },
            RefusedReason::Nfs,
            RefusedReason::Smb,
            RefusedReason::Fat,
            RefusedReason::ExFat,
            RefusedReason::Pseudo { file_system: "proc".into() },
            RefusedReason::UnsupportedSystem,
        ];
        let fallback =
            render(&Message::RefusedReason { code: "no-such-code".into(), file_system: "".into(), error: "".into() });
        let mut worded: Vec<String> = reasons.iter().map(refused_reason).collect();
        for (reason, words) in reasons.iter().zip(&worded) {
            assert_ne!(words, &fallback, "{reason:?} falls to the default");
        }
        assert_eq!(refused_reason(&RefusedReason::RemovableDrive), "its drive is removable");
        assert_eq!(
            refused_reason(&RefusedReason::NotNtfs { file_system: "FAT32".into() }),
            "its file system is FAT32, not NTFS"
        );
        assert_eq!(refused_reason(&unreadable(Fact::Volume)), "its volume can't be read: Access is denied.");
        assert_eq!(
            root_refused(r"D:\p", &RefusedReason::Link),
            "D:\\p is not scanned, because it is a link (a junction or a symbolic link). Remove it from the library to stop this notice."
        );
        worded.sort();
        worded.dedup();
        assert_eq!(worded.len(), reasons.len(), "two reasons read the same");
    }

    #[test]
    fn the_yandex_lines_are_worded() {
        assert_eq!(render(&Message::YandexStarting), "Starting…");
        assert_eq!(yandex_saving(1), "Saving 1 file…");
        assert_eq!(yandex_saving(3), "Saving 3 files…");
        assert_eq!(yandex_saving_file(2, 5), "Saving 2 of 5…");
        assert_eq!(yandex_saved(1, "D:/out", 0, None), "Saved 1 file to D:/out");
        assert_eq!(yandex_saved(3, "D:/out", 0, None), "Saved 3 files to D:/out");
        assert_eq!(yandex_saved(2, "D:/out", 1, Some("disk full")), "Saved 2, 1 failed. First failure: disk full");
        assert_eq!(
            yandex_download_failed("https://a/b.jpg", "Yandex answered HTTP 404"),
            "https://a/b.jpg couldn't be downloaded: Yandex answered HTTP 404"
        );
        assert_eq!(
            yandex_write_failed("https://a/b.jpg", "D:/out/b.jpg", "disk full"),
            "https://a/b.jpg couldn't be written to D:/out/b.jpg: disk full"
        );
        assert_eq!(render(&Message::YandexStopped), "Stopped by an internal error; the details are in gallery.log.");
        assert_eq!(yandex_http_status(503), "Yandex answered HTTP 503");
        assert_eq!(yandex_transport("timed out"), "Yandex request failed: timed out");
        assert_eq!(yandex_unexpected("no cbir_id"), "unexpected Yandex response: no cbir_id");
        assert_eq!(yandex_recorded("no such step"), "recorded Yandex session: no such step");
        assert_eq!(
            render(&Message::YandexNotStill),
            "Yandex search takes still images (JPEG, PNG, WebP, GIF, BMP or TIFF)."
        );
        assert_eq!(yandex_not_started("thread limit"), "Couldn't start the search or the save: thread limit");
        assert_eq!(yandex_nothing_picked(), "Pick at least one image to save.");
        assert_eq!(yandex_no_folder(), "No folder was chosen, so nothing was saved.");
    }

    #[test]
    fn the_start_and_crash_boxes_are_worded() {
        assert_eq!(catalog_unopened("locked"), "Can't open the catalog: locked");
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
        assert_eq!(
            thumb_cache_started_over(CacheStartOver::Damaged, r"C:\cat\thumbs.sqlite.bad"),
            "The thumbnail cache was damaged, so it was started over; its thumbnails are made again. The old file is \
             kept as C:\\cat\\thumbs.sqlite.bad"
        );
        assert_eq!(
            thumb_cache_started_over(CacheStartOver::OtherCatalog, r"C:\cat\thumbs.sqlite"),
            "The thumbnail cache C:\\cat\\thumbs.sqlite was made for another catalog, so it was started over; its \
             thumbnails are made again"
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
