//! The lines as typed values: each [`Message`] names its entry (`id`) and sets its values (`args`), and
//! [`render`] words it.

use fluent_bundle::FluentArgs;
use pg_model::notice::{Notice, Source};
use pg_model::refusal::RefusedReason;
use pg_model::rules::PathRules;

use super::words::shown;

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
    FilterTaken {
        days: String,
    },
    FilterModified {
        days: String,
    },
    /// A span of days, `from` and `to` each a day or empty.
    FilterDays {
        from: String,
        to: String,
    },
    /// A range's ends, as Fluent numbers (an open end is empty).
    FilterRangeMb {
        from: Option<f64>,
        to: Option<f64>,
    },
    FilterRangeMp {
        from: Option<f64>,
        to: Option<f64>,
    },
    FilterRangeSeconds {
        from: Option<f64>,
        to: Option<f64>,
    },
    /// Between the values of one facet in the filter line.
    FilterListSeparator,
    /// Between the filter line's parts.
    FilterPartSeparator,
    FilterBorders {
        values: String,
    },
    FilterBordersMarked,
    FilterBordersNotMarked,
    FilterName {
        name: String,
    },
    BadgeVideo {
        time: String,
    },
    BadgeApng,
    InfoNotImported,
    InfoDims {
        width: String,
        height: String,
    },
    InfoSizeMb {
        size: String,
    },
    InfoSizeKb {
        size: String,
    },
    InfoKept,
    InfoLine {
        path: String,
        dims: String,
        size: String,
        kept: String,
        borders: String,
    },
    ClassBlack,
    ClassWhite,
    ClassColor,
    ClassBlur,
    BordersInfo {
        names: String,
        left: String,
        top: String,
        right: String,
        bottom: String,
        marked: String,
    },
    BordersMarked,
    PassScan {
        root: String,
    },
    /// A scan's line: its name (`what`) and the images found so far.
    PassFound {
        what: String,
        count: String,
    },
    PassImport,
    PassBorders,
    PassCrop,
    PassThumbs {
        height: String,
    },
    PassWorking {
        what: String,
        done: String,
        total: String,
        rate: String,
        eta: String,
    },
    PassFailed {
        count: String,
    },
    EtaSeconds {
        seconds: String,
    },
    EtaMinutes {
        minutes: String,
    },
    EtaHours {
        hours: String,
        minutes: String,
    },
    WritingChanges {
        count: usize,
    },
    ChangesUnwritten {
        count: usize,
        error: String,
    },
    WriteQueueFailed {
        error: String,
    },
    /// Why a folder is refused (`pg_model::refusal::RefusedReason`): its code, which selects the words, with the file
    /// system's name or the system's error where the reason has one.
    RefusedReason {
        code: String,
        file_system: String,
        error: String,
    },
    FilePanicked {
        message: String,
    },
    YandexCaptcha,
    YandexNotStill,
    YandexUploading,
    YandexOtherSizes,
    YandexSimilar,
    YandexUnreadablePhoto {
        error: String,
    },
    YandexSimilarStopped {
        shown: String,
        reason: String,
    },
    YandexSimilarFailed {
        reason: String,
    },
    YandexNotStarted {
        error: String,
    },
    /// The folder dialog couldn't open or give its folder (for `error`), so nothing was chosen.
    FolderDialogFailed {
        error: String,
    },
    /// The save was asked with nothing picked.
    YandexNothingPicked,
    /// The save's folder dialog gave no folder, so nothing is saved.
    YandexNoFolder,
    /// The app's own folder dialog can't list a folder (for `error`).
    FolderUnreadable {
        error: String,
    },
    /// The library's status line while the background work is paused and no pass is running.
    WorkPaused,
    /// The library's status line while the background work is paused in a pass (`status`: the pass's line).
    PausedPass {
        status: String,
    },
    /// The Yandex panel's line while a search or a save starts.
    YandexStarting,
    /// A save of `count` results about to start.
    YandexSaving {
        count: usize,
    },
    /// The `n`th of `of` results being downloaded.
    YandexSavingFile {
        n: String,
        of: String,
    },
    /// A save that wrote all its `count` files into `folder`.
    YandexSaved {
        count: usize,
        folder: String,
    },
    /// A save in which some files failed: how many were written, how many failed, and the first failure.
    YandexSavedFailed {
        written: String,
        failed: String,
        error: String,
    },
    /// A result that couldn't be downloaded, for `error`.
    YandexDownloadFailed {
        url: String,
        error: String,
    },
    /// A downloaded result that couldn't be written to `path`, for `error`.
    YandexWriteFailed {
        url: String,
        path: String,
        error: String,
    },
    /// A search or a save stopped by an internal error (a panic, whose details are in gallery.log).
    YandexStopped,
    /// Yandex answered with an HTTP `status` other than success.
    YandexHttpStatus {
        status: String,
    },
    /// A connection, TLS or timeout failure of a Yandex request.
    YandexTransport {
        error: String,
    },
    /// A Yandex answer without the data the app reads (`detail`: what was missing).
    YandexUnexpected {
        detail: String,
    },
    /// A recorded Yandex session (the screenshot fixture's) that can't answer, for `error`.
    YandexRecorded {
        error: String,
    },
    /// A result's file that is not an image this app decodes, for `reason`.
    YandexUndecodable {
        reason: String,
    },
    /// The catalog couldn't be opened at start, for `error`: the app ends after the box.
    CatalogUnopened {
        error: String,
    },
    /// The first line of the box for a panic that ends the app.
    InternalError,
    /// The box's last line when the panic's details were written to `log`.
    DetailsInLog {
        log: String,
    },
    /// The box's last line when writing the details to `log` failed, for `error`.
    DetailsNotLogged {
        log: String,
        error: String,
    },
    /// A notice's title (its inbox row, its toast), paths under `rules`. Its entry is its kind's (`notice-<code>-title`).
    NoticeTitle {
        notice: Notice,
        rules: PathRules,
    },
    /// A notice's details (`notice-<code>-body`).
    NoticeBody {
        notice: Notice,
        rules: PathRules,
    },
    /// A notice as one line: its title, then its details.
    NoticeLine {
        title: String,
        body: String,
    },
    /// An inbox category's name (`source-<code>`).
    SourceLabel {
        source: Source,
    },
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
            Message::FilterDays { .. } => "filter-days",
            Message::FilterRangeMb { .. } => "filter-range-mb",
            Message::FilterRangeMp { .. } => "filter-range-mp",
            Message::FilterRangeSeconds { .. } => "filter-range-seconds",
            Message::FilterListSeparator => "filter-list-separator",
            Message::FilterPartSeparator => "filter-part-separator",
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
            Message::PassFound { .. } => "pass-found",
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
            Message::WriteQueueFailed { .. } => "write-queue-failed",
            Message::RefusedReason { .. } => "refused-reason",
            Message::FilePanicked { .. } => "file-panicked",
            Message::YandexCaptcha => "yandex-captcha",
            Message::YandexNotStill => "yandex-not-still",
            Message::YandexUploading => "yandex-uploading",
            Message::YandexOtherSizes => "yandex-other-sizes",
            Message::YandexSimilar => "yandex-similar",
            Message::YandexUnreadablePhoto { .. } => "yandex-unreadable-photo",
            Message::YandexSimilarStopped { .. } => "yandex-similar-stopped",
            Message::YandexSimilarFailed { .. } => "yandex-similar-failed",
            Message::YandexNotStarted { .. } => "yandex-not-started",
            Message::FolderDialogFailed { .. } => "folder-dialog-failed",
            Message::YandexNothingPicked => "yandex-nothing-picked",
            Message::YandexNoFolder => "yandex-no-folder",
            Message::FolderUnreadable { .. } => "folder-unreadable",
            Message::WorkPaused => "work-paused",
            Message::PausedPass { .. } => "paused-pass",
            Message::YandexStarting => "yandex-starting",
            Message::YandexSaving { .. } => "yandex-saving",
            Message::YandexSavingFile { .. } => "yandex-saving-file",
            Message::YandexSaved { .. } => "yandex-saved",
            Message::YandexSavedFailed { .. } => "yandex-saved-failed",
            Message::YandexDownloadFailed { .. } => "yandex-download-failed",
            Message::YandexWriteFailed { .. } => "yandex-write-failed",
            Message::YandexStopped => "yandex-stopped",
            Message::YandexHttpStatus { .. } => "yandex-http-status",
            Message::YandexTransport { .. } => "yandex-transport",
            Message::YandexUnexpected { .. } => "yandex-unexpected",
            Message::YandexRecorded { .. } => "yandex-recorded",
            Message::YandexUndecodable { .. } => "yandex-undecodable",
            Message::CatalogUnopened { .. } => "catalog-unopened",
            Message::InternalError => "internal-error",
            Message::DetailsInLog { .. } => "details-in-log",
            Message::DetailsNotLogged { .. } => "details-not-logged",
            Message::NoticeTitle { notice, .. } => notice.title_id(),
            Message::NoticeBody { notice, .. } => notice.body_id(),
            Message::NoticeLine { .. } => "notice-line",
            Message::SourceLabel { source } => super::notice::source_id(*source),
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
            Message::FilterDays { from, to } => {
                a.set("from", from.as_str());
                a.set("to", to.as_str());
            }
            Message::FilterRangeMb { from, to }
            | Message::FilterRangeMp { from, to }
            | Message::FilterRangeSeconds { from, to } => {
                set_end(&mut a, "from", *from);
                set_end(&mut a, "to", *to);
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
            Message::PassScan { root } => {
                a.set("root", root.as_str());
            }
            Message::PassFound { what, count } => {
                a.set("what", what.as_str());
                a.set("count", count.as_str());
            }
            Message::PassThumbs { height } => {
                a.set("height", height.as_str());
            }
            Message::PassWorking { what, done, total, rate, eta } => {
                a.set("what", what.as_str());
                a.set("done", done.as_str());
                a.set("total", total.as_str());
                a.set("rate", rate.as_str());
                a.set("eta", eta.as_str());
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
            Message::WriteQueueFailed { error } => {
                a.set("error", error.as_str());
            }
            Message::RefusedReason { code, file_system, error } => {
                a.set("code", code.as_str());
                a.set("fs", file_system.as_str());
                a.set("error", error.as_str());
            }
            Message::FilePanicked { message } => {
                a.set("message", message.as_str());
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
            Message::YandexNotStarted { error }
            | Message::FolderDialogFailed { error }
            | Message::FolderUnreadable { error }
            | Message::YandexTransport { error }
            | Message::YandexRecorded { error }
            | Message::CatalogUnopened { error } => {
                a.set("error", error.as_str());
            }
            Message::PausedPass { status } => {
                a.set("status", status.as_str());
            }
            Message::YandexSaving { count } => plural(&mut a, *count),
            Message::YandexSavingFile { n, of } => {
                a.set("n", n.as_str());
                a.set("of", of.as_str());
            }
            Message::YandexSaved { count, folder } => {
                plural(&mut a, *count);
                a.set("folder", folder.as_str());
            }
            Message::YandexSavedFailed { written, failed, error } => {
                a.set("written", written.as_str());
                a.set("failed", failed.as_str());
                a.set("error", error.as_str());
            }
            Message::YandexDownloadFailed { url, error } => {
                a.set("url", url.as_str());
                a.set("error", error.as_str());
            }
            Message::YandexWriteFailed { url, path, error } => {
                a.set("url", url.as_str());
                a.set("path", path.as_str());
                a.set("error", error.as_str());
            }
            Message::YandexHttpStatus { status } => {
                a.set("status", status.as_str());
            }
            Message::YandexUnexpected { detail } => {
                a.set("detail", detail.as_str());
            }
            Message::YandexUndecodable { reason } => {
                a.set("reason", reason.as_str());
            }
            Message::DetailsInLog { log } => {
                a.set("log", log.as_str());
            }
            Message::DetailsNotLogged { log, error } => {
                a.set("log", log.as_str());
                a.set("error", error.as_str());
            }
            Message::NoticeTitle { notice, rules } | Message::NoticeBody { notice, rules } => {
                let nested = super::notice::Nested { refused: &refused_reason, panicked: &file_panicked };
                super::notice::args(notice, *rules, &nested, &mut a);
            }
            Message::NoticeLine { title, body } => {
                a.set("title", title.as_str());
                a.set("body", body.as_str());
            }
            _ => {}
        };
        a
    }

    /// Every variant, with sample values: the test that the entries and the variants match runs on this list.
    #[cfg(test)]
    pub(super) fn samples() -> Vec<Message> {
        let s = || "x".to_string();
        let mut messages = vec![
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
            Message::FilterDays { from: s(), to: s() },
            Message::FilterRangeMb { from: Some(1.5), to: None },
            Message::FilterRangeMp { from: None, to: Some(12.0) },
            Message::FilterRangeSeconds { from: Some(1.0), to: Some(2.0) },
            Message::FilterListSeparator,
            Message::FilterPartSeparator,
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
            Message::PassScan { root: s() },
            Message::PassFound { what: s(), count: s() },
            Message::PassImport,
            Message::PassBorders,
            Message::PassCrop,
            Message::PassThumbs { height: s() },
            Message::PassWorking { what: s(), done: s(), total: s(), rate: s(), eta: s() },
            Message::PassFailed { count: s() },
            Message::EtaSeconds { seconds: s() },
            Message::EtaMinutes { minutes: s() },
            Message::EtaHours { hours: s(), minutes: s() },
            Message::WritingChanges { count: 1 },
            Message::ChangesUnwritten { count: 1, error: s() },
            Message::WriteQueueFailed { error: s() },
            Message::RefusedReason { code: "link".to_string(), file_system: s(), error: s() },
            Message::FilePanicked { message: s() },
            Message::YandexCaptcha,
            Message::YandexNotStill,
            Message::YandexUploading,
            Message::YandexOtherSizes,
            Message::YandexSimilar,
            Message::YandexUnreadablePhoto { error: s() },
            Message::YandexSimilarStopped { shown: s(), reason: s() },
            Message::YandexSimilarFailed { reason: s() },
            Message::YandexNotStarted { error: s() },
            Message::FolderDialogFailed { error: s() },
            Message::YandexNothingPicked,
            Message::YandexNoFolder,
            Message::FolderUnreadable { error: s() },
            Message::WorkPaused,
            Message::PausedPass { status: s() },
            Message::YandexStarting,
            Message::YandexSaving { count: 1 },
            Message::YandexSavingFile { n: s(), of: s() },
            Message::YandexSaved { count: 1, folder: s() },
            Message::YandexSavedFailed { written: s(), failed: s(), error: s() },
            Message::YandexDownloadFailed { url: s(), error: s() },
            Message::YandexWriteFailed { url: s(), path: s(), error: s() },
            Message::YandexStopped,
            Message::YandexHttpStatus { status: s() },
            Message::YandexTransport { error: s() },
            Message::YandexUnexpected { detail: s() },
            Message::YandexRecorded { error: s() },
            Message::YandexUndecodable { reason: s() },
            Message::CatalogUnopened { error: s() },
            Message::InternalError,
            Message::DetailsInLog { log: s() },
            Message::DetailsNotLogged { log: s(), error: s() },
        ];
        let rules = PathRules::Windows;
        for notice in Notice::samples() {
            messages.push(Message::NoticeTitle { notice: notice.clone(), rules });
            messages.push(Message::NoticeBody { notice, rules });
        }
        messages.push(Message::NoticeLine { title: s(), body: s() });
        messages.extend(Source::ALL.into_iter().map(|source| Message::SourceLabel { source }));
        messages
    }
}

/// The count for a plural entry: its number selects the form, its digits are the text.
fn plural(a: &mut FluentArgs<'_>, count: usize) {
    a.set("count", pg_model::num::count(count));
    a.set("text", count.to_string());
}

/// A range's end: a Fluent number, which a locale formats (`NUMBER()`), or empty for an open end.
fn set_end(a: &mut FluentArgs<'_>, key: &'static str, end: Option<f64>) {
    match end {
        Some(value) => a.set(key, value),
        None => a.set(key, ""),
    }
}

/// The line `message` reads as in the English words.
pub fn render(message: &Message) -> String {
    shown(message.id(), &message.args())
}

/// Why the scanner refuses a folder, as the refusal lines say it ("its drive is removable").
pub(super) fn refused_reason(reason: &RefusedReason) -> String {
    let (file_system, error) = match reason {
        RefusedReason::NotNtfs { file_system } | RefusedReason::Pseudo { file_system } => (file_system.clone(), ""),
        RefusedReason::Unreadable { error, .. } => (String::new(), error.as_str()),
        _ => (String::new(), ""),
    };
    render(&Message::RefusedReason { code: reason.code().to_string(), file_system, error: error.to_string() })
}

/// A file a job panicked on (caught, so the job went on), with the panic's message.
pub(super) fn file_panicked(message: &str) -> String {
    render(&Message::FilePanicked { message: message.to_string() })
}

#[cfg(test)]
mod tests {
    use fluent_bundle::FluentResource;

    use super::*;
    use crate::text::test_support::entries_in_file;
    use crate::text::words::ENGLISH;

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
}
