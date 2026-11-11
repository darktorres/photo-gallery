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
    ProgressCount {
        pass: String,
        done: String,
        rate: String,
        seconds: String,
    },
    Progress {
        pass: String,
        done: String,
        total: String,
        percent: String,
        rate: String,
        mbps: String,
        failed: String,
        seconds: String,
        eta: String,
    },
    WithDetail {
        line: String,
        detail: String,
    },
    Skipped {
        path: String,
        error: String,
    },
    Failed {
        path: String,
        error: String,
    },
    ChangedWhileRead {
        count: String,
    },
    ScanSummary {
        root: String,
        files: String,
        folders: String,
        changed: String,
        missing: String,
        unreadable: String,
        seconds: String,
    },
    ThumbNotForgotten {
        root: String,
        error: String,
    },
    TempNotDeleted {
        root: String,
        temp: String,
        error: String,
    },
    Removed {
        root: String,
        hidden: String,
    },
    NotInLibrary {
        root: String,
    },
    MarkedAgain {
        count: String,
    },
    ImportPending {
        pending: String,
        indexed: String,
    },
    /// `changed`: the [`Line::ChangedWhileRead`] words, or empty.
    ImportDone {
        files: String,
        failed: String,
        changed: String,
        pairs: String,
        seconds: String,
    },
    NoFileThere {
        path: String,
    },
    ThumbsNeeded {
        needed: String,
        height: String,
        held: String,
    },
    /// `changed`: the [`Line::ChangedWhileRead`] words, or empty; `full`: the [`Line::CacheFull`] words, or empty.
    ThumbsDone {
        made: String,
        height: String,
        failed: String,
        changed: String,
        full: String,
        used: String,
        cap: String,
        seconds: String,
    },
    CacheFull,
    ReviewImported {
        marked: String,
        unknown: String,
    },
    BordersDone {
        files: String,
        failed: String,
        stored: String,
        seconds: String,
    },
    BordersServed {
        count: String,
    },
    BordersReport {
        files: String,
        borders: String,
        rejected: String,
        skipped: String,
        errors: String,
        seconds: String,
        rate: String,
    },
    CsvMissing,
    StatusHome {
        folder: String,
    },
    StatusLibrary {
        folder: String,
    },
    StatusFiles {
        files: String,
        missing: String,
        analyzed: String,
        failed: String,
        pending: String,
        pairs: String,
    },
    StatusWrites {
        queued: String,
        failing: String,
    },
    StatusCache {
        count: String,
        used: String,
        cap: String,
    },
    StatusNoCache,
    ThumbnailsError {
        file: String,
        error: String,
    },
    WriteDone {
        left: String,
        failing: String,
        pending: String,
        seconds: String,
    },
    WriterStopped {
        left: String,
    },
    WriterStoppedFailing {
        left: String,
        error: String,
    },
    DupsIdentical {
        tier: Tier,
        groups: String,
        files: String,
    },
    DupsPdq,
    DupsBand {
        lo: String,
        hi: String,
        pairs: String,
    },
    DupsGroups {
        distance: String,
        groups: String,
        files: String,
        largest: String,
    },
    DupsGroup {
        count: String,
    },
    FileNotInCatalog {
        id: String,
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
            Line::ThumbNotForgotten { .. } => "cli-thumb-not-forgotten",
            Line::TempNotDeleted { .. } => "cli-temp-not-deleted",
            Line::Removed { .. } => "cli-removed",
            Line::NotInLibrary { .. } => "cli-not-in-library",
            Line::MarkedAgain { .. } => "cli-marked-again",
            Line::ImportPending { .. } => "cli-import-pending",
            Line::ImportDone { .. } => "cli-import-done",
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
            Line::ThumbNotForgotten { root, error } => vec![("root", root), ("error", error)],
            Line::TempNotDeleted { root, temp, error } => vec![("root", root), ("temp", temp), ("error", error)],
            Line::Removed { root, hidden } => vec![("root", root), ("hidden", hidden)],
            Line::NotInLibrary { root } => vec![("root", root)],
            Line::ImportPending { pending, indexed } => vec![("pending", pending), ("indexed", indexed)],
            Line::ImportDone { files, failed, changed, pairs, seconds } => {
                vec![("files", files), ("failed", failed), ("changed", changed), ("pairs", pairs), ("seconds", seconds)]
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
            Line::CacheFull | Line::CsvMissing | Line::StatusNoCache | Line::DupsPdq => Vec::new(),
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
            Line::ThumbNotForgotten { root: s(), error: s() },
            Line::TempNotDeleted { root: s(), temp: s(), error: s() },
            Line::Removed { root: s(), hidden: s() },
            Line::NotInLibrary { root: s() },
            Line::MarkedAgain { count: s() },
            Line::ImportPending { pending: s(), indexed: s() },
            Line::ImportDone { files: s(), failed: s(), changed: s(), pairs: s(), seconds: s() },
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
            pairs: "3".into(),
            seconds: "0.5".into(),
        };
        assert_eq!(
            line(&done),
            "imported 12 files (1 failed, 2 changed while read, left for the next pass), 3 PDQ pairs, 0.5 s"
        );
        let tier = Line::DupsIdentical { tier: Tier::Pixels, groups: "2".into(), files: "5".into() };
        assert_eq!(line(&tier), "tier 0b identical pixels: 2 groups, 5 files");
    }
}
