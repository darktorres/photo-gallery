//! The lines as typed values: each [`Message`] names its entry (`id`) and sets its values (`args`), and
//! [`render`] words it.

use fluent_bundle::FluentArgs;
use pg_model::notice::{Notice, Source};
use pg_model::refusal::RefusedReason;
use pg_model::rules::PathRules;

use super::words::shown;

/// A value a message gives its entry (under the field's name), and a sample of it for the tests.
trait Arg {
    fn set<'a>(&'a self, key: &'static str, a: &mut FluentArgs<'a>);
    #[cfg(test)]
    fn sample() -> Self;
}

impl Arg for String {
    fn set<'a>(&'a self, key: &'static str, a: &mut FluentArgs<'a>) {
        a.set(key, self.as_str());
    }
    #[cfg(test)]
    fn sample() -> Self {
        "x".to_string()
    }
}

/// A range's end: a Fluent number, which a locale formats (`NUMBER()`), or empty for an open end.
impl Arg for Option<f64> {
    fn set<'a>(&'a self, key: &'static str, a: &mut FluentArgs<'a>) {
        match self {
            Some(value) => a.set(key, *value),
            None => a.set(key, ""),
        }
    }
    #[cfg(test)]
    fn sample() -> Self {
        Some(1.5)
    }
}

/// A count, for a plural entry: its number selects the form, and its digits are the text (`text`), so no thousands
/// separator is added.
impl Arg for usize {
    fn set<'a>(&'a self, key: &'static str, a: &mut FluentArgs<'a>) {
        a.set(key, pg_model::num::count(*self));
        a.set("text", self.to_string());
    }
    #[cfg(test)]
    fn sample() -> Self {
        1
    }
}

/// The messages, from one table: each variant, its fields and its entry. A field's value is its entry's argument of the
/// same name ([`Arg`]). The notices' title and body and the inbox's categories are added by hand: their entries are
/// their kinds'.
macro_rules! messages {
    ($( $(#[$meta:meta])* $variant:ident $({ $($field:ident: $ty:ty),* })? => $id:literal, )*) => {
        /// A line of the front end's text: its entry in `locales/en/text.ftl` and the values it words. The variants are the
        /// lines the functions below make; a variant without an entry (or an entry without a variant) fails the test that
        /// lists them.
        #[derive(Clone, Debug, PartialEq)]
        pub enum Message {
            $( $(#[$meta])* $variant $({ $($field: $ty),* })?, )*
            /// A notice's title (its inbox row, its toast), paths under `rules`. Its entry is its kind's (`notice-<code>-title`).
            NoticeTitle { notice: Notice, rules: PathRules },
            /// A notice's details (`notice-<code>-body`).
            NoticeBody { notice: Notice, rules: PathRules },
            /// An inbox category's name (`source-<code>`).
            SourceLabel { source: Source },
        }

        impl Message {
            /// The entry this message is worded by in `locales/en/text.ftl`.
            pub fn id(&self) -> &'static str {
                match self {
                    $( Message::$variant $({ $($field: _),* })? => $id, )*
                    Message::NoticeTitle { notice, .. } => notice.title_id(),
                    Message::NoticeBody { notice, .. } => notice.body_id(),
                    Message::SourceLabel { source } => super::notice::source_id(*source),
                }
            }

            /// The values the entry takes.
            fn args(&self) -> FluentArgs<'_> {
                let mut a = FluentArgs::new();
                match self {
                    $( Message::$variant $({ $($field),* })? => { $($( Arg::set($field, stringify!($field), &mut a); )*)? } )*
                    Message::NoticeTitle { notice, rules } | Message::NoticeBody { notice, rules } => {
                        let nested = super::notice::Nested { refused: &refused_reason, panicked: &file_panicked };
                        super::notice::args(notice, *rules, &nested, &mut a);
                    }
                    Message::SourceLabel { .. } => {}
                }
                a
            }

            /// Every variant, with sample values: the test that the entries and the variants match runs on this list.
            #[cfg(test)]
            pub(super) fn samples() -> Vec<Message> {
                let mut messages = vec![ $( Message::$variant $({ $($field: Arg::sample()),* })?, )* ];
                let rules = PathRules::Windows;
                for notice in Notice::samples() {
                    messages.push(Message::NoticeTitle { notice: notice.clone(), rules });
                    messages.push(Message::NoticeBody { notice, rules });
                }
                messages.extend(Source::ALL.into_iter().map(|source| Message::SourceLabel { source }));
                messages
            }
        }
    };
}

messages! {
    TypeImage => "type-image",
    TypeAnimated => "type-animated",
    TypeVideo => "type-video",
    ShapeLandscape => "shape-landscape",
    ShapePortrait => "shape-portrait",
    ShapeSquare => "shape-square",
    StatusImported => "status-imported",
    StatusPending => "status-pending",
    StatusFailed => "status-failed",
    BorderNone => "border-none",
    BorderBlack => "border-black",
    BorderWhite => "border-white",
    BorderColor => "border-color",
    BorderBlur => "border-blur",
    ExtNone => "ext-none",
    FilterAll => "filter-all",
    FilterUnrated => "filter-unrated",
    FilterWithDuplicates => "filter-with-duplicates",
    FilterWithoutDuplicates => "filter-without-duplicates",
    FilterTaken { days: String } => "filter-taken",
    FilterModified { days: String } => "filter-modified",
    /// A span of days, `from` and `to` each a day or empty.
    FilterDays { from: String, to: String } => "filter-days",
    /// A range's ends, as Fluent numbers (an open end is empty).
    FilterRangeMb { from: Option<f64>, to: Option<f64> } => "filter-range-mb",
    FilterRangeMp { from: Option<f64>, to: Option<f64> } => "filter-range-mp",
    FilterRangeSeconds { from: Option<f64>, to: Option<f64> } => "filter-range-seconds",
    /// Between the values of one facet in the filter line.
    FilterListSeparator => "filter-list-separator",
    /// Between the filter line's parts.
    FilterPartSeparator => "filter-part-separator",
    FilterBorders { values: String } => "filter-borders",
    FilterBordersMarked => "filter-borders-marked",
    FilterBordersNotMarked => "filter-borders-not-marked",
    FilterName { name: String } => "filter-name",
    BadgeVideo { time: String } => "badge-video",
    BadgeApng => "badge-apng",
    InfoNotImported => "info-not-imported",
    InfoDims { width: String, height: String } => "info-dims",
    InfoSizeMb { size: String } => "info-size-mb",
    InfoSizeKb { size: String } => "info-size-kb",
    InfoKept => "info-kept",
    InfoLine { path: String, dims: String, size: String, kept: String, borders: String } => "info-line",
    ClassBlack => "class-black",
    ClassWhite => "class-white",
    ClassColor => "class-color",
    ClassBlur => "class-blur",
    BordersInfo { names: String, left: String, top: String, right: String, bottom: String, marked: String } => "borders-info",
    BordersMarked => "borders-marked",
    PassScan { root: String } => "pass-scan",
    /// A scan's line: its name (`what`) and the images found so far.
    PassFound { what: String, count: String } => "pass-found",
    PassImport => "pass-import",
    PassBorders => "pass-borders",
    PassCrop => "pass-crop",
    PassThumbs { height: String } => "pass-thumbs",
    PassWorking { what: String, done: String, total: String, rate: String, eta: String } => "pass-working",
    PassFailed { count: String } => "pass-failed",
    EtaSeconds { seconds: String } => "eta-seconds",
    EtaMinutes { minutes: String } => "eta-minutes",
    EtaHours { hours: String, minutes: String } => "eta-hours",
    WritingChanges { count: usize } => "writing-changes",
    ChangesUnwritten { count: usize, error: String } => "changes-unwritten",
    WriteQueueFailed { error: String } => "write-queue-failed",
    /// Why a folder is refused (`pg_model::refusal::RefusedReason`): its code, which selects the words, with the file
    /// system's name or the system's error where the reason has one.
    RefusedReason { code: String, file_system: String, error: String } => "refused-reason",
    FilePanicked { message: String } => "file-panicked",
    YandexCaptcha => "yandex-captcha",
    YandexNotStill => "yandex-not-still",
    YandexUploading => "yandex-uploading",
    YandexOtherSizes => "yandex-other-sizes",
    YandexSimilar => "yandex-similar",
    YandexUnreadablePhoto { error: String } => "yandex-unreadable-photo",
    YandexSimilarStopped { shown: String, reason: String } => "yandex-similar-stopped",
    YandexSimilarFailed { reason: String } => "yandex-similar-failed",
    YandexNotStarted { error: String } => "yandex-not-started",
    /// The folder dialog couldn't open or give its folder (for `error`), so nothing was chosen.
    FolderDialogFailed { error: String } => "folder-dialog-failed",
    /// The save was asked with nothing picked.
    YandexNothingPicked => "yandex-nothing-picked",
    /// The save's folder dialog gave no folder, so nothing is saved.
    YandexNoFolder => "yandex-no-folder",
    /// The app's own folder dialog can't list a folder (for `error`).
    FolderUnreadable { error: String } => "folder-unreadable",
    /// A path given to add (a folder, a file named on the command line) can't be opened, for `error`.
    PathUnreadable { path: String, error: String } => "path-unreadable",
    /// A path given has no path form under the rules (a NUL).
    NotAPath { path: String } => "not-a-path",
    /// The library's status line while the background work is paused and no pass is running.
    WorkPaused => "work-paused",
    /// The library's status line while the background work is paused in a pass (`status`: the pass's line).
    PausedPass { status: String } => "paused-pass",
    /// The Yandex panel's line while a search or a save starts.
    YandexStarting => "yandex-starting",
    /// A save of `count` results about to start.
    YandexSaving { count: usize } => "yandex-saving",
    /// The `n`th of `of` results being downloaded.
    YandexSavingFile { n: String, of: String } => "yandex-saving-file",
    /// A save that wrote all its `count` files into `folder`.
    YandexSaved { count: usize, folder: String } => "yandex-saved",
    /// A save in which some files failed: how many were written, how many failed, and the first failure.
    YandexSavedFailed { written: String, failed: String, error: String } => "yandex-saved-failed",
    /// A result that couldn't be downloaded, for `error`.
    YandexDownloadFailed { url: String, error: String } => "yandex-download-failed",
    /// A downloaded result that couldn't be written to `path`, for `error`.
    YandexWriteFailed { url: String, path: String, error: String } => "yandex-write-failed",
    /// A search or a save stopped by an internal error (a panic, whose details are in gallery.log).
    YandexStopped => "yandex-stopped",
    /// Yandex answered with an HTTP `status` other than success.
    YandexHttpStatus { status: String } => "yandex-http-status",
    /// A connection, TLS or timeout failure of a Yandex request.
    YandexTransport { error: String } => "yandex-transport",
    /// A Yandex answer without the data the app reads (`detail`: what was missing).
    YandexUnexpected { detail: String } => "yandex-unexpected",
    /// A recorded Yandex session (the screenshot fixture's) that can't answer, for `error`.
    YandexRecorded { error: String } => "yandex-recorded",
    /// A result's file that is not an image this app decodes, for `reason`.
    YandexUndecodable { reason: String } => "yandex-undecodable",
    /// The catalog couldn't be opened at start, for `error`: the app ends after the box.
    CatalogUnopened { error: String } => "catalog-unopened",
    /// The first line of the box for a panic that ends the app.
    InternalError => "internal-error",
    /// The box's last line when the panic's details were written to `log`.
    DetailsInLog { log: String } => "details-in-log",
    /// The box's last line when writing the details to `log` failed, for `error`.
    DetailsNotLogged { log: String, error: String } => "details-not-logged",
    /// A notice as one line: its title, then its details.
    NoticeLine { title: String, body: String } => "notice-line",
    /// How many of a ledger entry's failures came after the user dismissed it.
    LedgerSinceDismissed { count: usize } => "ledger-since-dismissed",
    /// A ledger's failed file, as Copy puts it: its path and its error.
    FailedFileLine { path: String, error: String } => "failed-file-line",
    /// Why a folder of the scans' ledger failed, by its reason's code.
    FailedFolderReason { reason: String } => "failed-folder-reason",
    /// A folder of the scans' ledger, as Copy puts it: its path, why (`FailedFolderReason`'s words) and the error.
    FailedFolderLine { path: String, why: String, error: String } => "failed-folder-line",
    /// The line above a library folder's failed folders, in a Copy.
    FailedFolderGroup { root: String } => "failed-folder-group",
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
