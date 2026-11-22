//! The command line's own lines as typed values (`locales/en/cli.ftl`): its summaries, its progress and the failures
//! it lists. Each [`Line`] names its entry and sets its values, and [`line()`] words it. Its notices are worded as the
//! window's are (`text::notice_line`). Numbers come in as the text the line shows (the command line keeps its own
//! precision), so the words hold no formatting.

use fluent_bundle::FluentArgs;

use super::words::shown;

/// A tier of identical files the `dups` report counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// 0a: identical files.
    File,
    /// 0b: identical pixels.
    Pixels,
}

/// A line of the command line: its entry in `locales/en/cli.ftl` and the values it words. A variant without an entry
/// (or an entry without a variant) fails the test that lists them.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// A pass's progress by count, with its rate and the time it has run.
    ProgressCount {
        /// The pass's name, as the line words it.
        pass: String,
        /// How many items are done, as text.
        done: String,
        /// The rate, as text.
        rate: String,
        /// The time the pass has run, in seconds, as text.
        seconds: String,
    },
    /// A pass's progress with its total, its speed and its failures.
    Progress {
        /// The pass's name, as the line words it.
        pass: String,
        /// How many items are done, as text.
        done: String,
        /// How many items there are in all, as text.
        total: String,
        /// The share done, as text.
        percent: String,
        /// The rate, as text.
        rate: String,
        /// The throughput, in megabits a second, as text.
        mbps: String,
        /// How many items failed, as text.
        failed: String,
        /// The time the pass has run, in seconds, as text.
        seconds: String,
        /// The time left, as text.
        eta: String,
    },
    /// A line with a detail after it, such as a failure's cause.
    WithDetail {
        /// The line, as worded.
        line: String,
        /// The detail that follows it.
        detail: String,
    },
    /// A file that was skipped, and why.
    Skipped {
        /// The file's path, as the command line shows it.
        path: String,
        /// Why it was skipped, as text.
        error: String,
    },
    /// A file that failed, and why.
    Failed {
        /// The file's path, as the command line shows it.
        path: String,
        /// Why it failed, as text.
        error: String,
    },
    /// Files that changed while they were read.
    ChangedWhileRead {
        /// How many changed, as text.
        count: String,
    },
    /// The summary of a folder's scan.
    ScanSummary {
        /// The folder scanned, as the library shows it.
        root: String,
        /// The files seen, as text.
        files: String,
        /// The folders walked, as text.
        folders: String,
        /// The files that changed, as text.
        changed: String,
        /// The files marked missing, as text.
        missing: String,
        /// The folders that couldn't be read, as text.
        unreadable: String,
        /// The time the scan took, in seconds, as text.
        seconds: String,
    },
    /// A mount under a scanned folder that the scan didn't walk into: its path and why.
    RefusedMount {
        /// The folder that was not walked into, as its path.
        mount: String,
        /// Why it was refused, as text.
        reason: String,
    },
    /// The refused mounts past the first few listed.
    RefusedMountsNotListed {
        /// How many more were not listed, as text.
        count: String,
    },
    /// A thumbnail that could not be forgotten, after its file was removed.
    ThumbNotForgotten {
        /// The folder of the file, as the library shows it.
        root: String,
        /// Why it could not be forgotten, as text.
        error: String,
    },
    /// A temporary file that could not be deleted.
    TempNotDeleted {
        /// The folder of the temp, as the library shows it.
        root: String,
        /// The temp file's path, as text.
        temp: String,
        /// Why it could not be deleted, as text.
        error: String,
    },
    /// A folder removed from the library.
    Removed {
        /// The folder's path, as the library shows it.
        root: String,
        /// How many files were deleted, as text.
        deleted: String,
    },
    /// A removal kept for a write that is not finished.
    RemovalKept {
        /// The folder's path, as the library shows it.
        root: String,
        /// How many files are deferred, as text.
        deferred: String,
    },
    /// A folder that is not in the library.
    NotInLibrary {
        /// The folder's path, as the library shows it.
        root: String,
    },
    /// A walk of `root` is refused: another walk of an overlapping folder is running (`holder` says whose).
    ScanBusy {
        /// The folder asked for, as its path.
        root: String,
        /// Who holds the other walk, as text.
        holder: String,
    },
    /// The library folder `root` was removed while its walk ran.
    ScanRemoved {
        /// The folder, as its path.
        root: String,
    },
    /// The files marked again, as the command line counts them.
    MarkedAgain {
        /// How many were marked, as text.
        count: String,
    },
    /// The files the import has yet to do.
    ImportPending {
        /// How many are pending, as text.
        pending: String,
    },
    /// `changed`: the [`Line::ChangedWhileRead`] words, or empty.
    ImportDone {
        /// The files imported, as text.
        files: String,
        /// The files that failed, as text.
        failed: String,
        /// The files that changed while they were read, as text.
        changed: String,
        /// The time the import took, in seconds, as text.
        seconds: String,
    },
    /// The Hashes pass: the files looked at, the hashes stored, the files that failed (and of them, those unreadable).
    HashesDone {
        /// The files looked at, as text.
        files: String,
        /// The hashes stored, as text.
        stored: String,
        /// The files that failed, as text.
        failed: String,
        /// The failed files that couldn't be read, as text.
        unreadable: String,
        /// The time the pass took, in seconds, as text.
        seconds: String,
    },
    /// Files stored without their duplicate hashes (`gallery status`): made by `gallery hash`.
    HashesOwed {
        /// How many files are owed, as text.
        count: String,
    },
    /// The Placeholders pass: the placeholders made, the files whose couldn't be made.
    PlaceholdersDone {
        /// The placeholders made, as text.
        made: String,
        /// The files whose placeholder couldn't be made, as text.
        failed: String,
        /// The time the pass took, in seconds, as text.
        seconds: String,
    },
    /// Files without a placeholder yet (`gallery status`): made by `gallery thumbs`.
    PlaceholdersOwed {
        /// How many files are owed, as text.
        count: String,
    },
    /// The pairing pass: the files paired, the pairs stored.
    PairDone {
        /// The files paired, as text.
        files: String,
        /// The pairs stored, as text.
        pairs: String,
        /// The time the pass took, in seconds, as text.
        seconds: String,
    },
    /// Where the pairing pass's time went, in seconds: reading the pictures, building the join's store, the joins (wall
    /// clock; then their threads' time in the sorts, in the scans, with nothing to take, and paused), the write
    /// statements, the commits.
    PairTimings {
        /// The time reading the pictures took, in seconds, as text.
        load: String,
        /// The time building the join's store took, in seconds, as text.
        store: String,
        /// The time the joins took, wall clock, in seconds, as text.
        join: String,
        /// The threads' time in the sorts, in seconds, as text.
        sort: String,
        /// The threads' time in the scans, in seconds, as text.
        scan: String,
        /// The threads' time with nothing to take, in seconds, as text.
        idle: String,
        /// The time the pass was paused, in seconds, as text.
        paused: String,
        /// The time in the write statements, in seconds, as text.
        write: String,
        /// The time in the commits, in seconds, as text.
        commit: String,
    },
    /// The owed pairs were marked again (`gallery pair --again`): how many analyses owe them.
    PairsAgain {
        /// How many analyses owe pairs, as text.
        count: String,
    },
    /// Analyses without orientation hashes, left out of the pairs (made by an older build).
    PairsLegacy {
        /// How many analyses, as text.
        count: String,
    },
    /// Files the pairs are still owed for (`gallery status`).
    PairsOwed {
        /// How many files are owed, as text.
        count: String,
    },
    /// A file that is not there any more.
    NoFileThere {
        /// The file's path, as the command line shows it.
        path: String,
    },
    /// The thumbnails a pass needs at a height.
    ThumbsNeeded {
        /// How many files needed a thumbnail, as text.
        needed: String,
        /// The height, in pixels, as text.
        height: String,
        /// The cache's size, in GiB, as text.
        held: String,
    },
    /// `changed`: the [`Line::ChangedWhileRead`] words, or empty; `full`: the [`Line::CacheFull`] words, or empty.
    ThumbsDone {
        /// The thumbnails made, as text.
        made: String,
        /// The height, in pixels, as text.
        height: String,
        /// The thumbnails that failed, as text.
        failed: String,
        /// The changed-while-read words, or empty.
        changed: String,
        /// The cache-full words, or empty.
        full: String,
        /// The cache's size, in GiB, as text.
        used: String,
        /// The cache's cap, in GiB, as text.
        cap: String,
        /// The time the pass took, in seconds, as text.
        seconds: String,
    },
    /// The thumbnail cache is full.
    CacheFull,
    /// The files a review imported: marked, and not in the catalog.
    ReviewImported {
        /// How many were marked as not a border, as text.
        marked: String,
        /// How many are not in the catalog, as text.
        unknown: String,
    },
    /// The borders pass's totals.
    BordersDone {
        /// The files looked at, as text.
        files: String,
        /// The files that failed, as text.
        failed: String,
        /// The borders stored, as text.
        stored: String,
        /// The time the pass took, in seconds, as text.
        seconds: String,
    },
    /// Borders served from the catalog, by the count of files.
    BordersServed {
        /// How many files, as text.
        count: String,
    },
    /// The borders report's totals.
    BordersReport {
        /// The files looked at, as text.
        files: String,
        /// The files with borders, as text.
        borders: String,
        /// The candidates rejected, as text.
        rejected: String,
        /// The files skipped, as text.
        skipped: String,
        /// The errors, as text.
        errors: String,
        /// The time the report took, in seconds, as text.
        seconds: String,
        /// The rate, in files a second, as text.
        rate: String,
    },
    /// The report's CSV file is missing.
    CsvMissing,
    /// The data folder, as the status shows it.
    StatusHome {
        /// The folder.
        folder: String,
    },
    /// The library's folder, as the status shows it.
    StatusLibrary {
        /// The folder.
        folder: String,
    },
    /// A folder being removed, and what is deferred.
    StatusRemoving {
        /// The folder being removed.
        folder: String,
        /// How many files are deferred, as text.
        deferred: String,
    },
    /// The files in the library, by state.
    StatusFiles {
        /// The files, as text.
        files: String,
        /// The files marked missing, as text.
        missing: String,
        /// The files analyzed, as text.
        analyzed: String,
        /// The files that failed, as text.
        failed: String,
        /// The files pending, as text.
        pending: String,
        /// The PDQ pairs, as text.
        pairs: String,
    },
    /// The changes queued to be written into files.
    StatusWrites {
        /// The changes queued, as text.
        queued: String,
        /// The changes failing, as text.
        failing: String,
    },
    /// The thumbnail cache's state.
    StatusCache {
        /// The entries it holds, as text.
        count: String,
        /// The size it uses, in GiB, as text.
        used: String,
        /// Its cap, in GiB, as text.
        cap: String,
    },
    /// There is no thumbnail cache yet.
    StatusNoCache,
    /// A file whose thumbnail failed, and why.
    ThumbnailsError {
        /// The file, as the command line shows it.
        file: String,
        /// Why the thumbnail failed, as text.
        error: String,
    },
    /// The writes done, and those left.
    WriteDone {
        /// The changes left to write, as text.
        left: String,
        /// The changes failing, as text.
        failing: String,
        /// The files to re-import, as text.
        pending: String,
        /// The time the writes took, in seconds, as text.
        seconds: String,
    },
    /// The writer stopped with changes still to write.
    WriterStopped {
        /// The changes still to write, as text.
        left: String,
    },
    /// The writer stopped with changes still to write, and a failure last.
    WriterStoppedFailing {
        /// The changes still to write, as text.
        left: String,
        /// The last failure, as text.
        error: String,
    },
    /// The identical files of a tier.
    DupsIdentical {
        /// The tier the groups are in.
        tier: Tier,
        /// How many groups, as text.
        groups: String,
        /// How many files, as text.
        files: String,
    },
    /// The PDQ duplicates, which the report lists by band.
    DupsPdq,
    /// A band of pictures by their distance.
    DupsBand {
        /// The band's lower distance bound, as text.
        lo: String,
        /// The band's upper distance bound, as text.
        hi: String,
        /// How many pairs are in the band, as text.
        pairs: String,
    },
    /// The groups of duplicates within a distance.
    DupsGroups {
        /// The distance the groups are within, as text.
        distance: String,
        /// How many groups, as text.
        groups: String,
        /// How many files, as text.
        files: String,
        /// The size of the largest group, as text.
        largest: String,
    },
    /// A group of duplicates.
    DupsGroup {
        /// How many files it holds, as text.
        count: String,
    },
    /// A file id that is not in the catalog.
    FileNotInCatalog {
        /// The id asked for, as text.
        id: String,
    },
    /// The build has no window, so the desktop app can't open.
    NoWindow,
    /// A command's failure, as the command line prints it: its error chain, one line.
    Error {
        /// The error's text, the chain included.
        error: String,
    },
}

impl Line {
    /// The entry this line is worded by in `locales/en/cli.ftl`.
    pub fn id(&self) -> &'static str {
        match self {
            Line::ProgressCount { .. } => "cli-progress-count",
            Line::Progress { .. } => "cli-progress",
            Line::WithDetail { .. } => "cli-with-detail",
            Line::Skipped { .. } => "cli-skipped",
            Line::Failed { .. } => "cli-failed",
            Line::ChangedWhileRead { .. } => "cli-changed-while-read",
            Line::ScanSummary { .. } => "cli-scan-summary",
            Line::RefusedMount { .. } => "cli-refused-mount",
            Line::RefusedMountsNotListed { .. } => "cli-refused-mounts-not-listed",
            Line::ThumbNotForgotten { .. } => "cli-thumb-not-forgotten",
            Line::TempNotDeleted { .. } => "cli-temp-not-deleted",
            Line::Removed { .. } => "cli-removed",
            Line::RemovalKept { .. } => "cli-removal-kept",
            Line::NotInLibrary { .. } => "cli-not-in-library",
            Line::ScanBusy { .. } => "cli-scan-busy",
            Line::ScanRemoved { .. } => "cli-scan-removed",
            Line::MarkedAgain { .. } => "cli-marked-again",
            Line::ImportPending { .. } => "cli-import-pending",
            Line::ImportDone { .. } => "cli-import-done",
            Line::HashesDone { .. } => "cli-hashes-done",
            Line::HashesOwed { .. } => "cli-hashes-owed",
            Line::PlaceholdersDone { .. } => "cli-placeholders-done",
            Line::PlaceholdersOwed { .. } => "cli-placeholders-owed",
            Line::PairDone { .. } => "cli-pair-done",
            Line::PairTimings { .. } => "cli-pair-timings",
            Line::PairsAgain { .. } => "cli-pairs-again",
            Line::PairsLegacy { .. } => "cli-pairs-legacy",
            Line::PairsOwed { .. } => "cli-pairs-owed",
            Line::NoFileThere { .. } => "cli-no-file-there",
            Line::ThumbsNeeded { .. } => "cli-thumbs-needed",
            Line::ThumbsDone { .. } => "cli-thumbs-done",
            Line::CacheFull => "cli-cache-full",
            Line::ReviewImported { .. } => "cli-review-imported",
            Line::BordersDone { .. } => "cli-borders-done",
            Line::BordersServed { .. } => "cli-borders-served",
            Line::BordersReport { .. } => "cli-borders-report",
            Line::CsvMissing => "cli-csv-missing",
            Line::StatusHome { .. } => "cli-status-home",
            Line::StatusLibrary { .. } => "cli-status-library",
            Line::StatusRemoving { .. } => "cli-status-removing",
            Line::StatusFiles { .. } => "cli-status-files",
            Line::StatusWrites { .. } => "cli-status-writes",
            Line::StatusCache { .. } => "cli-status-cache",
            Line::StatusNoCache => "cli-status-no-cache",
            Line::ThumbnailsError { .. } => "cli-thumbnails-error",
            Line::WriteDone { .. } => "cli-write-done",
            Line::WriterStopped { .. } => "cli-writer-stopped",
            Line::WriterStoppedFailing { .. } => "cli-writer-stopped-failing",
            Line::DupsIdentical { .. } => "cli-dups-identical",
            Line::DupsPdq => "cli-dups-pdq",
            Line::DupsBand { .. } => "cli-dups-band",
            Line::DupsGroups { .. } => "cli-dups-groups",
            Line::DupsGroup { .. } => "cli-dups-group",
            Line::FileNotInCatalog { .. } => "cli-file-not-in-catalog",
            Line::NoWindow => "cli-no-window",
            Line::Error { .. } => "cli-error",
        }
    }

    /// The values the entry takes, by their names in the entry.
    fn values(&self) -> Vec<(&'static str, &str)> {
        match self {
            Line::ProgressCount { pass, done, rate, seconds } => {
                vec![("pass", pass), ("done", done), ("rate", rate), ("seconds", seconds)]
            }
            Line::Progress { pass, done, total, percent, rate, mbps, failed, seconds, eta } => vec![
                ("pass", pass),
                ("done", done),
                ("total", total),
                ("percent", percent),
                ("rate", rate),
                ("mbps", mbps),
                ("failed", failed),
                ("seconds", seconds),
                ("eta", eta),
            ],
            Line::WithDetail { line, detail } => vec![("line", line), ("detail", detail)],
            Line::Skipped { path, error } | Line::Failed { path, error } => vec![("path", path), ("error", error)],
            Line::ChangedWhileRead { count } | Line::MarkedAgain { count } | Line::BordersServed { count } => {
                vec![("count", count)]
            }
            Line::DupsGroup { count } => vec![("count", count)],
            Line::ScanSummary { root, files, folders, changed, missing, unreadable, seconds } => vec![
                ("root", root),
                ("files", files),
                ("folders", folders),
                ("changed", changed),
                ("missing", missing),
                ("unreadable", unreadable),
                ("seconds", seconds),
            ],
            Line::RefusedMount { mount, reason } => vec![("mount", mount), ("reason", reason)],
            Line::RefusedMountsNotListed { count } => vec![("count", count)],
            Line::ThumbNotForgotten { root, error } => vec![("root", root), ("error", error)],
            Line::TempNotDeleted { root, temp, error } => vec![("root", root), ("temp", temp), ("error", error)],
            Line::Removed { root, deleted } => vec![("root", root), ("deleted", deleted)],
            Line::RemovalKept { root, deferred } => vec![("root", root), ("deferred", deferred)],
            Line::NotInLibrary { root } => vec![("root", root)],
            Line::ScanBusy { root, holder } => vec![("root", root), ("holder", holder)],
            Line::ScanRemoved { root } => vec![("root", root)],
            Line::ImportPending { pending } => vec![("pending", pending)],
            Line::ImportDone { files, failed, changed, seconds } => {
                vec![("files", files), ("failed", failed), ("changed", changed), ("seconds", seconds)]
            }
            Line::HashesDone { files, stored, failed, unreadable, seconds } => vec![
                ("files", files),
                ("stored", stored),
                ("failed", failed),
                ("unreadable", unreadable),
                ("seconds", seconds),
            ],
            Line::HashesOwed { count } => vec![("count", count)],
            Line::PlaceholdersDone { made, failed, seconds } => {
                vec![("made", made), ("failed", failed), ("seconds", seconds)]
            }
            Line::PlaceholdersOwed { count } => vec![("count", count)],
            Line::PairDone { files, pairs, seconds } => vec![("files", files), ("pairs", pairs), ("seconds", seconds)],
            Line::PairTimings { load, store, join, sort, scan, idle, paused, write, commit } => {
                vec![
                    ("load", load),
                    ("store", store),
                    ("join", join),
                    ("sort", sort),
                    ("scan", scan),
                    ("idle", idle),
                    ("paused", paused),
                    ("write", write),
                    ("commit", commit),
                ]
            }
            Line::PairsAgain { count } | Line::PairsLegacy { count } | Line::PairsOwed { count } => {
                vec![("count", count)]
            }
            Line::NoFileThere { path } => vec![("path", path)],
            Line::ThumbsNeeded { needed, height, held } => vec![("needed", needed), ("height", height), ("held", held)],
            Line::ThumbsDone { made, height, failed, changed, full, used, cap, seconds } => vec![
                ("made", made),
                ("height", height),
                ("failed", failed),
                ("changed", changed),
                ("full", full),
                ("used", used),
                ("cap", cap),
                ("seconds", seconds),
            ],
            Line::ReviewImported { marked, unknown } => vec![("marked", marked), ("unknown", unknown)],
            Line::BordersDone { files, failed, stored, seconds } => {
                vec![("files", files), ("failed", failed), ("stored", stored), ("seconds", seconds)]
            }
            Line::BordersReport { files, borders, rejected, skipped, errors, seconds, rate } => vec![
                ("files", files),
                ("borders", borders),
                ("rejected", rejected),
                ("skipped", skipped),
                ("errors", errors),
                ("seconds", seconds),
                ("rate", rate),
            ],
            Line::StatusHome { folder } | Line::StatusLibrary { folder } => vec![("folder", folder)],
            Line::StatusRemoving { folder, deferred } => vec![("folder", folder), ("deferred", deferred)],
            Line::StatusFiles { files, missing, analyzed, failed, pending, pairs } => vec![
                ("files", files),
                ("missing", missing),
                ("analyzed", analyzed),
                ("failed", failed),
                ("pending", pending),
                ("pairs", pairs),
            ],
            Line::StatusWrites { queued, failing } => vec![("queued", queued), ("failing", failing)],
            Line::StatusCache { count, used, cap } => vec![("count", count), ("used", used), ("cap", cap)],
            Line::ThumbnailsError { file, error } => vec![("file", file), ("error", error)],
            Line::WriteDone { left, failing, pending, seconds } => {
                vec![("left", left), ("failing", failing), ("pending", pending), ("seconds", seconds)]
            }
            Line::WriterStopped { left } => vec![("left", left)],
            Line::WriterStoppedFailing { left, error } => vec![("left", left), ("error", error)],
            Line::DupsIdentical { tier, groups, files } => {
                let tier = match tier {
                    Tier::File => "file",
                    Tier::Pixels => "pixels",
                };
                vec![("tier", tier), ("groups", groups), ("files", files)]
            }
            Line::DupsBand { lo, hi, pairs } => vec![("lo", lo), ("hi", hi), ("pairs", pairs)],
            Line::DupsGroups { distance, groups, files, largest } => {
                vec![("distance", distance), ("groups", groups), ("files", files), ("largest", largest)]
            }
            Line::FileNotInCatalog { id } => vec![("id", id)],
            Line::Error { error } => vec![("error", error)],
            Line::CacheFull | Line::CsvMissing | Line::StatusNoCache | Line::DupsPdq | Line::NoWindow => Vec::new(),
        }
    }

    /// Every variant, with sample values: the test that the entries and the variants match runs on this list.
    #[cfg(test)]
    fn samples() -> Vec<Line> {
        let s = || "x".to_string();
        vec![
            Line::ProgressCount { pass: s(), done: s(), rate: s(), seconds: s() },
            Line::Progress {
                pass: s(),
                done: s(),
                total: s(),
                percent: s(),
                rate: s(),
                mbps: s(),
                failed: s(),
                seconds: s(),
                eta: s(),
            },
            Line::WithDetail { line: s(), detail: s() },
            Line::Skipped { path: s(), error: s() },
            Line::Failed { path: s(), error: s() },
            Line::ChangedWhileRead { count: s() },
            Line::ScanSummary {
                root: s(),
                files: s(),
                folders: s(),
                changed: s(),
                missing: s(),
                unreadable: s(),
                seconds: s(),
            },
            Line::RefusedMount { mount: s(), reason: s() },
            Line::RefusedMountsNotListed { count: s() },
            Line::ThumbNotForgotten { root: s(), error: s() },
            Line::TempNotDeleted { root: s(), temp: s(), error: s() },
            Line::Removed { root: s(), deleted: s() },
            Line::RemovalKept { root: s(), deferred: s() },
            Line::NotInLibrary { root: s() },
            Line::ScanBusy { root: s(), holder: s() },
            Line::ScanRemoved { root: s() },
            Line::MarkedAgain { count: s() },
            Line::ImportPending { pending: s() },
            Line::ImportDone { files: s(), failed: s(), changed: s(), seconds: s() },
            Line::HashesDone { files: s(), stored: s(), failed: s(), unreadable: s(), seconds: s() },
            Line::HashesOwed { count: s() },
            Line::PlaceholdersDone { made: s(), failed: s(), seconds: s() },
            Line::PlaceholdersOwed { count: s() },
            Line::PairDone { files: s(), pairs: s(), seconds: s() },
            Line::PairTimings {
                load: s(),
                store: s(),
                join: s(),
                sort: s(),
                scan: s(),
                idle: s(),
                paused: s(),
                write: s(),
                commit: s(),
            },
            Line::PairsAgain { count: s() },
            Line::PairsLegacy { count: s() },
            Line::PairsOwed { count: s() },
            Line::NoFileThere { path: s() },
            Line::ThumbsNeeded { needed: s(), height: s(), held: s() },
            Line::ThumbsDone {
                made: s(),
                height: s(),
                failed: s(),
                changed: s(),
                full: s(),
                used: s(),
                cap: s(),
                seconds: s(),
            },
            Line::CacheFull,
            Line::ReviewImported { marked: s(), unknown: s() },
            Line::BordersDone { files: s(), failed: s(), stored: s(), seconds: s() },
            Line::BordersServed { count: s() },
            Line::BordersReport {
                files: s(),
                borders: s(),
                rejected: s(),
                skipped: s(),
                errors: s(),
                seconds: s(),
                rate: s(),
            },
            Line::CsvMissing,
            Line::StatusHome { folder: s() },
            Line::StatusLibrary { folder: s() },
            Line::StatusRemoving { folder: s(), deferred: s() },
            Line::StatusFiles { files: s(), missing: s(), analyzed: s(), failed: s(), pending: s(), pairs: s() },
            Line::StatusWrites { queued: s(), failing: s() },
            Line::StatusCache { count: s(), used: s(), cap: s() },
            Line::StatusNoCache,
            Line::ThumbnailsError { file: s(), error: s() },
            Line::WriteDone { left: s(), failing: s(), pending: s(), seconds: s() },
            Line::WriterStopped { left: s() },
            Line::WriterStoppedFailing { left: s(), error: s() },
            Line::DupsIdentical { tier: Tier::File, groups: s(), files: s() },
            Line::DupsPdq,
            Line::DupsBand { lo: s(), hi: s(), pairs: s() },
            Line::DupsGroups { distance: s(), groups: s(), files: s(), largest: s() },
            Line::DupsGroup { count: s() },
            Line::FileNotInCatalog { id: s() },
            Line::NoWindow,
            Line::Error { error: s() },
        ]
    }
}

/// The words of `line`, in English (or the pseudo-locale).
pub fn line(line: &Line) -> String {
    let mut args = FluentArgs::new();
    for (name, value) in line.values() {
        args.set(name, value);
    }
    shown(line.id(), &args)
}

#[cfg(test)]
mod tests {
    use fluent_bundle::FluentResource;

    use super::*;
    use crate::text::test_support::entries_of;
    use crate::text::words::ENGLISH_CLI;

    #[test]
    fn the_lines_and_the_english_entries_name_the_same_ids() {
        assert!(FluentResource::try_new(ENGLISH_CLI.to_string()).is_ok(), "cli.ftl has a syntax error");
        let mut in_file = entries_of(ENGLISH_CLI);
        in_file.sort_unstable();
        let mut in_code: Vec<&str> = Line::samples().iter().map(Line::id).collect();
        in_code.sort_unstable();
        let unique = in_code.len();
        in_code.dedup();
        assert_eq!(in_code.len(), unique, "two lines share an id");
        assert_eq!(in_file, in_code, "cli.ftl and the Line variants name different entries");
        assert!(in_file.iter().all(|id| id.starts_with("cli-")), "every cli.ftl id starts with cli-");
    }

    #[test]
    fn every_line_is_worded_by_its_entry_with_its_values() {
        for sample in Line::samples() {
            let words = line(&sample);
            assert!(!words.is_empty() && words != sample.id(), "{} has no entry", sample.id());
            assert!(!words.contains('{'), "{}: a value isn't set: {words}", sample.id());
        }
        let done = Line::ImportDone {
            files: "12".into(),
            failed: "1".into(),
            changed: line(&Line::ChangedWhileRead { count: "2".into() }),
            seconds: "0.5".into(),
        };
        assert_eq!(line(&done), "imported 12 files (1 failed, 2 changed while read, left for the next pass), 0.5 s");
        let paired = Line::PairDone { files: "12".into(), pairs: "3".into(), seconds: "0.5".into() };
        assert_eq!(line(&paired), "paired 12 files: 3 PDQ pairs, 0.5 s");
        let timings = Line::PairTimings {
            load: "210".into(),
            store: "40".into(),
            join: "620".into(),
            sort: "100".into(),
            scan: "480".into(),
            idle: "30".into(),
            paused: "0".into(),
            write: "330".into(),
            commit: "80".into(),
        };
        assert_eq!(
            line(&timings),
            "load 210 ms, store 40 ms, join 620 ms (threads' time: sorts 100 ms, scans 480 ms, idle 30 ms, \
             paused 0 ms), writes 330 ms, commits 80 ms"
        );
        let tier = Line::DupsIdentical { tier: Tier::Pixels, groups: "2".into(), files: "5".into() };
        assert_eq!(line(&tier), "tier 0b identical pixels: 2 groups, 5 files");
        let tier = Line::DupsIdentical { tier: Tier::File, groups: "3".into(), files: "7".into() };
        assert_eq!(line(&tier), "tier 0a identical files: 3 groups, 7 files");
        assert_eq!(line(&Line::Error { error: "no such file".into() }), "error: no such file");
    }
}
