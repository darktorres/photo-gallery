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

/// A count held as a `u64` (a catalog's): the same form as a `usize`; a count past `usize::MAX` is not a count a
/// machine holds, so it saturates to it for the plural form.
impl Arg for u64 {
    fn set<'a>(&'a self, key: &'static str, a: &mut FluentArgs<'a>) {
        a.set(key, pg_model::num::count(usize::try_from(*self).unwrap_or(usize::MAX)));
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
    ($( $(#[$meta:meta])* $variant:ident $({ $( $(#[$fmeta:meta])* $field:ident: $ty:ty ),* })? => $id:literal, )*) => {
        /// A line of the front end's text: its entry in `locales/en/text.ftl` and the values it words. The variants are the
        /// lines the functions below make; a variant without an entry (or an entry without a variant) fails the test that
        /// lists them.
        #[derive(Clone, Debug, PartialEq)]
        pub enum Message {
            $( $(#[$meta])* $variant $({ $( $(#[$fmeta])* $field: $ty ),* })?, )*
            /// A notice's title (its inbox row, its toast), paths under `rules`. Its entry is its kind's (`notice-<code>-title`).
            NoticeTitle {
                /// The notice.
                notice: Notice,
                /// The rules the notice's paths are shown under.
                rules: PathRules,
            },
            /// A notice's details (`notice-<code>-body`).
            NoticeBody {
                /// The notice.
                notice: Notice,
                /// The rules the notice's paths are shown under.
                rules: PathRules,
            },
            /// An inbox category's name (`source-<code>`).
            SourceLabel {
                /// The category.
                source: Source,
            },
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
    /// A still image's type, in the filter line and the info line.
    TypeImage => "type-image",
    /// An animated image's type.
    TypeAnimated => "type-animated",
    /// A video's type.
    TypeVideo => "type-video",
    /// A landscape shape: wider than tall.
    ShapeLandscape => "shape-landscape",
    /// A portrait shape: taller than wide.
    ShapePortrait => "shape-portrait",
    /// A square shape.
    ShapeSquare => "shape-square",
    /// A file imported into the catalog.
    StatusImported => "status-imported",
    /// A file still waiting for its analysis.
    StatusPending => "status-pending",
    /// A file whose import failed.
    StatusFailed => "status-failed",
    /// A file with no border.
    BorderNone => "border-none",
    /// A black border.
    BorderBlack => "border-black",
    /// A white border.
    BorderWhite => "border-white",
    /// A border of a colour.
    BorderColor => "border-color",
    /// A blurred border.
    BorderBlur => "border-blur",
    /// A file with no extension.
    ExtNone => "ext-none",
    /// The filter that shows every file.
    FilterAll => "filter-all",
    /// The filter that shows the unrated files.
    FilterUnrated => "filter-unrated",
    /// The filter that shows the files with duplicates.
    FilterWithDuplicates => "filter-with-duplicates",
    /// The filter that shows the files without duplicates.
    FilterWithoutDuplicates => "filter-without-duplicates",
    /// The filter for files taken within a number of days.
    FilterTaken {
        /// The number of days, as text.
        days: String
    } => "filter-taken",
    /// The filter for files modified within a number of days.
    FilterModified {
        /// The number of days, as text.
        days: String
    } => "filter-modified",
    /// A span of days, `from` and `to` each a day or empty.
    /// A span of days, `from` and `to` each a day or empty.
    FilterDays {
        /// The first day, or empty.
        from: String,
        /// The last day, or empty.
        to: String
    } => "filter-days",
    /// A range's ends, as Fluent numbers (an open end is empty).
    FilterRangeMb {
        /// The lower end, in MB, or none.
        from: Option<f64>,
        /// The upper end, in MB, or none.
        to: Option<f64>
    } => "filter-range-mb",
    /// A range of megapixels, its ends as for the MB range.
    FilterRangeMp {
        /// The lower end, in megapixels, or none.
        from: Option<f64>,
        /// The upper end, in megapixels, or none.
        to: Option<f64>
    } => "filter-range-mp",
    /// A range of seconds, its ends as for the MB range.
    FilterRangeSeconds {
        /// The lower end, in seconds, or none.
        from: Option<f64>,
        /// The upper end, in seconds, or none.
        to: Option<f64>
    } => "filter-range-seconds",
    /// Between the values of one facet in the filter line.
    FilterListSeparator => "filter-list-separator",
    /// Between the filter line's parts.
    FilterPartSeparator => "filter-part-separator",
    /// The filter for the borders' values.
    FilterBorders {
        /// The border values, as text.
        values: String
    } => "filter-borders",
    /// The filter for the files marked as having borders.
    FilterBordersMarked => "filter-borders-marked",
    /// The filter for the files marked as not having borders.
    FilterBordersNotMarked => "filter-borders-not-marked",
    /// The filter for files whose name matches.
    FilterName {
        /// The name typed, as text.
        name: String
    } => "filter-name",
    /// The badge on a video, with its length.
    BadgeVideo {
        /// The length, as text.
        time: String
    } => "badge-video",
    /// The badge on an animated PNG.
    BadgeApng => "badge-apng",
    /// The info line of a file not imported yet.
    InfoNotImported => "info-not-imported",
    /// A file's dimensions.
    InfoDims {
        /// The width, in pixels, as text.
        width: String,
        /// The height, in pixels, as text.
        height: String
    } => "info-dims",
    /// A file's size, in MB.
    InfoSizeMb {
        /// The size, as text.
        size: String
    } => "info-size-mb",
    /// A file's size, in KB.
    InfoSizeKb {
        /// The size, as text.
        size: String
    } => "info-size-kb",
    /// The info line that says a file is kept.
    InfoKept => "info-kept",
    /// A file's info line, with its parts.
    InfoLine {
        /// The path, as text.
        path: String,
        /// Its dimensions, as text.
        dims: String,
        /// Its size, as text.
        size: String,
        /// Whether it is kept, as text.
        kept: String,
        /// Its borders, as text.
        borders: String
    } => "info-line",
    /// The class of a border: black.
    ClassBlack => "class-black",
    /// The class of a border: white.
    ClassWhite => "class-white",
    /// The class of a border: a colour.
    ClassColor => "class-color",
    /// The class of a border: blurred.
    ClassBlur => "class-blur",
    /// The borders found on a file.
    BordersInfo {
        /// The sides with borders, as text.
        names: String,
        /// The border's size on the left, as text.
        left: String,
        /// The border's size on the top, as text.
        top: String,
        /// The border's size on the right, as text.
        right: String,
        /// The border's size on the bottom, as text.
        bottom: String,
        /// Whether the borders are marked, as text.
        marked: String
    } => "borders-info",
    /// The borders marked by the user.
    BordersMarked => "borders-marked",
    /// A scan pass's line, with its folder.
    PassScan {
        /// The folder, as text.
        root: String
    } => "pass-scan",
    /// A scan's line: its name (`what`) and the images found so far.
    PassFound {
        /// The scan's name, as text.
        what: String,
        /// The images found so far, as text.
        count: String
    } => "pass-found",
    /// The import pass's name.
    PassImport => "pass-import",
    /// The hashes pass's name.
    PassHashes => "pass-hashes",
    /// The pairing pass's name.
    PassPairing => "pass-pairing",
    /// The borders pass's name.
    PassBorders => "pass-borders",
    /// The crop pass's name.
    PassCrop => "pass-crop",
    /// The thumbnails pass, at a height.
    PassThumbs {
        /// The height, as text.
        height: String
    } => "pass-thumbs",
    /// The placeholders pass's name.
    PassPlaceholders => "pass-placeholders",
    /// The removal pass's name.
    PassRemoving => "pass-removing",
    /// A pass's progress, with its totals.
    PassWorking {
        /// The pass's name, as text.
        what: String,
        /// How much is done, as text.
        done: String,
        /// The total, as text.
        total: String,
        /// The rate, as text.
        rate: String,
        /// The time left, as text.
        eta: String
    } => "pass-working",
    /// A pass's failures.
    PassFailed {
        /// How many failed, as text.
        count: String
    } => "pass-failed",
    /// A time left, in seconds.
    EtaSeconds {
        /// The seconds, as text.
        seconds: String
    } => "eta-seconds",
    /// A time left, in minutes.
    EtaMinutes {
        /// The minutes, as text.
        minutes: String
    } => "eta-minutes",
    /// A time left, in hours and minutes.
    EtaHours {
        /// The hours, as text.
        hours: String,
        /// The minutes, as text.
        minutes: String
    } => "eta-hours",
    /// The changes being written into the files.
    WritingChanges {
        /// How many, as a number.
        count: u64
    } => "writing-changes",
    /// Changes that could not be written, and why.
    ChangesUnwritten {
        /// How many, as a number.
        count: u64,
        /// The error, as text.
        error: String
    } => "changes-unwritten",
    /// The write queue failed.
    WriteQueueFailed {
        /// The error, as text.
        error: String
    } => "write-queue-failed",
    /// Why a folder is refused (`pg_model::refusal::RefusedReason`): its code, which selects the words, with the file
    /// system's name or the system's error where the reason has one.
    RefusedReason {
        /// The refusal's code, which selects its words.
        code: String,
        /// The file system's name, where the reason names one.
        file_system: String,
        /// The system's error, where the reason has one.
        error: String
    } => "refused-reason",
    /// A file's processing panicked, with its message.
    FilePanicked {
        /// The panic's message, as text.
        message: String
    } => "file-panicked",
    /// The error a job reported on a file, in the system's or the library's own words, as the detail under the failure.
    FailureDetail {
        /// The error, as text.
        error: String
    } => "failure-detail",
    /// Yandex asked for a captcha.
    YandexCaptcha => "yandex-captcha",
    /// The photo is not still: it is animated.
    YandexNotStill => "yandex-not-still",
    /// The upload to Yandex is in progress.
    YandexUploading => "yandex-uploading",
    /// The other sizes of the photo are being looked up.
    YandexOtherSizes => "yandex-other-sizes",
    /// The similar images are being looked up.
    YandexSimilar => "yandex-similar",
    /// Yandex could not read the photo.
    YandexUnreadablePhoto {
        /// The error, as text.
        error: String
    } => "yandex-unreadable-photo",
    /// The similar list stopped early.
    YandexSimilarStopped {
        /// How many were shown, as text.
        shown: String,
        /// Why it stopped, as text.
        reason: String
    } => "yandex-similar-stopped",
    /// The similar lookup failed.
    YandexSimilarFailed {
        /// Why, as text.
        reason: String
    } => "yandex-similar-failed",
    /// A search that did not start.
    YandexNotStarted {
        /// The error, as text.
        error: String
    } => "yandex-not-started",
    /// The folder dialog couldn't open or give its folder, so nothing was chosen.
    FolderDialogFailed {
        /// The error, as text.
        error: String
    } => "folder-dialog-failed",
    /// The save was asked with nothing picked.
    YandexNothingPicked => "yandex-nothing-picked",
    /// The save's folder dialog gave no folder, so nothing is saved.
    YandexNoFolder => "yandex-no-folder",
    /// The app's own folder dialog can't list a folder.
    FolderUnreadable {
        /// The error, as text.
        error: String
    } => "folder-unreadable",
    /// A path given to add (a folder, a file named on the command line) can't be opened.
    PathUnreadable {
        /// The path, as text.
        path: String,
        /// The error, as text.
        error: String
    } => "path-unreadable",
    /// A path given has no path form under the rules (a NUL).
    NotAPath {
        /// The path, as text.
        path: String
    } => "not-a-path",
    /// A lane's row while the lane is paused by itself and no pass of it runs.
    LanePaused => "lane-paused",
    /// A paused lane's row in a pass with no known total: `status` is the pass's line.
    PausedPass {
        /// The pass's line, as text.
        status: String
    } => "paused-pass",
    /// A paused lane's row in a pass with a known total: its name, and how far it is (no rate or time left).
    LanePausedPass {
        /// The pass's name, as text.
        what: String,
        /// How far it is, as text.
        done: String,
        /// The total, as text.
        total: String
    } => "lane-paused-pass",
    /// A paused lane's row in a scan: its name and the images found so far.
    LanePausedFound {
        /// The scan's name, as text.
        what: String,
        /// The images found so far, as text.
        count: String
    } => "lane-paused-found",
    /// The Yandex panel's line while a search or a save starts.
    YandexStarting => "yandex-starting",
    /// A save of `count` results about to start.
    YandexSaving {
        /// How many results, as a number.
        count: usize
    } => "yandex-saving",
    /// The `n`th of `of` results being downloaded.
    YandexSavingFile {
        /// The result's number, as text.
        n: String,
        /// The total, as text.
        of: String
    } => "yandex-saving-file",
    /// A save that wrote all its `count` files into `folder`.
    YandexSaved {
        /// How many files were written.
        count: usize,
        /// The folder they were written to.
        folder: String
    } => "yandex-saved",
    /// A save in which some files failed: how many were written, how many failed, and the first failure.
    YandexSavedFailed {
        /// How many were written, as text.
        written: String,
        /// How many failed, as text.
        failed: String,
        /// The first failure, as text.
        error: String
    } => "yandex-saved-failed",
    /// A result that couldn't be downloaded.
    YandexDownloadFailed {
        /// The result's address.
        url: String,
        /// The error, as text.
        error: String
    } => "yandex-download-failed",
    /// A downloaded result that couldn't be written to `path`.
    YandexWriteFailed {
        /// The result's address.
        url: String,
        /// The path it was to be written to.
        path: String,
        /// The error, as text.
        error: String
    } => "yandex-write-failed",
    /// A search or a save stopped by an internal error (a panic), whose details are in `log`.
    YandexPanicked {
        /// Where the details are logged.
        log: String
    } => "yandex-panicked",
    /// The folder list changed, as the panel shows it.
    FolderListChanged => "folder-list-changed",
    /// Yandex answered with an HTTP `status` other than success.
    YandexHttpStatus {
        /// The status, as text.
        status: String
    } => "yandex-http-status",
    /// A connection, TLS or timeout failure of a Yandex request.
    YandexTransport {
        /// The error, as text.
        error: String
    } => "yandex-transport",
    /// A Yandex upload answer that is not JSON.
    YandexUploadNotJson => "yandex-upload-not-json",
    /// A Yandex upload answer without the id of the upload.
    YandexUploadNoCbirId => "yandex-upload-no-cbir-id",
    /// A Yandex upload answer without the original's address.
    YandexUploadNoOriginal => "yandex-upload-no-original",
    /// A Yandex results page without its state.
    YandexResultsPageNoState => "yandex-results-page-no-state",
    /// A Yandex results state that is not JSON.
    YandexResultsStateNotJson => "yandex-results-state-not-json",
    /// A Yandex similar answer that is not JSON.
    YandexSimilarNotJson => "yandex-similar-not-json",
    /// A Yandex similar answer without results.
    YandexSimilarNoResults => "yandex-similar-no-results",
    /// The file is not an image.
    YandexNotAnImage => "yandex-not-an-image",
    /// The image is too large for Yandex.
    YandexTooLarge => "yandex-too-large",
    /// Yandex found no result.
    YandexNoResult => "yandex-no-result",
    /// There was no search to show.
    YandexNoSearch => "yandex-no-search",
    /// The search is not on the screen.
    YandexNoSearchOnScreen => "yandex-no-search-on-screen",
    /// Yandex was not called: the search did not run.
    YandexNotCalled => "yandex-not-called",
    /// A recorded Yandex session (the screenshot fixture's) that can't answer, for `error`.
    YandexRecorded {
        /// The error, as text.
        error: String
    } => "yandex-recorded",
    /// A result's file that is not an image this app decodes, for `reason`.
    YandexUndecodable {
        /// Why, as text.
        reason: String
    } => "yandex-undecodable",
    /// The catalog couldn't be opened at start, for `error`: the app ends after the box.
    CatalogUnopened {
        /// The error, as text.
        error: String
    } => "catalog-unopened",
    /// A catalog a later build made, at `path`: its schema is `found` and this build's is `current`. `built_by` is
    /// " by build X" when the file names the build, and empty when it does not.
    CatalogNewerSchema {
        /// The catalog's path.
        path: String,
        /// The schema version the file has, as text.
        found: String,
        /// The version this build expects, as text.
        current: String,
        /// " by build X" when the file names it, else empty.
        built_by: String
    } => "catalog-newer-schema",
    /// A catalog a later build marked with an integer mark this build does not know the meaning of, at `path`: `mark` is
    /// the mark's name, `found` its value and `current` the highest this build knows. `built_by` as for the schema.
    CatalogNewerMark {
        /// The catalog's path.
        path: String,
        /// The mark's name.
        mark: String,
        /// The mark's value in the file, as text.
        found: String,
        /// The highest value this build knows, as text.
        current: String,
        /// " by build X" when the file names it, else empty.
        built_by: String
    } => "catalog-newer-mark",
    /// A catalog whose schema is older than this build's, at `path`: `found` is its version and `current` this build's.
    /// A released catalog is not migrated, so it is restored from a backup.
    CatalogOlderSchema {
        /// The catalog's path.
        path: String,
        /// The schema version the file has, as text.
        found: String,
        /// The version this build expects, as text.
        current: String
    } => "catalog-older-schema",
    /// A catalog of a released version whose schema differs from this build's, at `path` (`found`: its version, `detail`:
    /// the difference).
    CatalogDrift {
        /// The catalog's path.
        path: String,
        /// The schema version the file has, as text.
        found: String,
        /// The difference from this build's schema, as text.
        detail: String
    } => "catalog-drift",
    /// A catalog without the `key_version` mark, whose schema differs from this build's, at `path`.
    CatalogPredatesKeyVersion {
        /// The catalog's path.
        path: String
    } => "catalog-predates-key-version",
    /// A catalog at `path` made under other path rules: `detail` says how (or that they were not recorded), `wanted` names
    /// this build's rules.
    CatalogPathRules {
        /// The catalog's path.
        path: String,
        /// How its rules differ, or that they were not recorded, as text.
        detail: String,
        /// This build's rules, by name, as text.
        wanted: String
    } => "catalog-path-rules",
    /// Another program has the catalog at `path` open, so it can't be changed.
    CatalogInUse {
        /// The catalog's path.
        path: String
    } => "catalog-in-use",
    /// The catalog's path at `path` is too long for the files it keeps beside it (Windows' limit).
    CatalogPathTooLong {
        /// The catalog's path.
        path: String
    } => "catalog-path-too-long",
    /// The catalog at `path` needs a change, and can't be written (read-only, or permission denied).
    CatalogReadOnly {
        /// The catalog's path.
        path: String
    } => "catalog-read-only",
    /// The catalog folder at `path` isn't there, and isn't made (most likely its drive isn't connected).
    CatalogFolderMissing {
        /// The folder's path.
        path: String
    } => "catalog-folder-missing",
    /// The first line of the box for a panic that ends the app.
    InternalError => "internal-error",
    /// The box's last line when the panic's details were written to `log`.
    DetailsInLog {
        /// Where the details are logged.
        log: String
    } => "details-in-log",
    /// The box's last line when writing the details to `log` failed, for `error`.
    DetailsNotLogged {
        /// Where the details were to be logged.
        log: String,
        /// The error, as text.
        error: String
    } => "details-not-logged",
    /// A notice as one line: its title, then its details.
    NoticeLine {
        /// The notice's title, as text.
        title: String,
        /// The notice's details, as text.
        body: String
    } => "notice-line",
    /// How many of a ledger entry's failures came after the user dismissed it.
    LedgerSinceDismissed {
        /// How many, as a number.
        count: usize
    } => "ledger-since-dismissed",
    /// A ledger's failed file, as Copy puts it: its path and its error.
    FailedFileLine {
        /// The file's path, as text.
        path: String,
        /// The error, as text.
        error: String
    } => "failed-file-line",
    /// Why a folder of the scans' ledger failed, by its reason's code.
    FailedFolderReason {
        /// The reason's code, as text.
        reason: String
    } => "failed-folder-reason",
    /// A folder of the scans' ledger, as Copy puts it: its path, why (`FailedFolderReason`'s words) and the error.
    FailedFolderLine {
        /// The folder's path, as text.
        path: String,
        /// Why it failed, in words.
        why: String,
        /// The error, as text.
        error: String
    } => "failed-folder-line",
    /// The line above a library folder's failed folders, in a Copy.
    FailedFolderGroup {
        /// The library folder, as text.
        root: String
    } => "failed-folder-group",
}

/// The line `message` reads as in the English words.
pub fn render(message: &Message) -> String {
    shown(message.id(), &message.args())
}

/// Why the scanner refuses a folder, as the refusal lines say it ("its drive is removable").
pub fn refused_reason(reason: &RefusedReason) -> String {
    let (file_system, error) = match reason {
        RefusedReason::NotNtfs { file_system } | RefusedReason::Pseudo { file_system } => (file_system.clone(), ""),
        RefusedReason::Unreadable { error, .. } => (String::new(), error.as_str()),
        _ => (String::new(), ""),
    };
    render(&Message::RefusedReason { code: reason.key().to_string(), file_system, error: error.to_string() })
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
