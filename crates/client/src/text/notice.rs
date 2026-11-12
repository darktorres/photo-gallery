//! The values of a notice's entries (`notice-<code>-title`, `-body`): each kind's fields, its paths under the rules they
//! were stored under, its codes as selectors, and the nested words (a refusal's reason) worded first, by the caller
//! ([`Nested`]: the words are `message.rs`'s, which calls this).
//!
//! A path is set bidi-isolated ([`isolated`]): a folder's name in Arabic or Hebrew would otherwise reorder the words
//! around it. The bundle's own isolation stays off (`words.rs`): it would isolate every value, the selectors' too, and
//! the counts; only a path is text of the user's that can run the other way.

use fluent_bundle::FluentArgs;
use pg_model::notice::{Carried, FileFailure, LedgerCounts, Notice, Source};
use pg_model::refusal::RefusedReason;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_model::write::WriteKind;

/// Opens a path's isolation (U+2068 FIRST STRONG ISOLATE): the path takes its direction from its own first strong
/// character, and the words around it keep theirs.
const ISOLATE: char = '\u{2068}';
/// Closes it (U+2069 POP DIRECTIONAL ISOLATE).
const POP: char = '\u{2069}';

/// `path` as a value: bidi-isolated. An empty one (a field the notice doesn't have) stays empty.
fn isolated(path: &str) -> String {
    if path.is_empty() { String::new() } else { format!("{ISOLATE}{path}{POP}") }
}

/// A library path, displayed under `rules`, as a value.
fn shown(rules: PathRules, path: &RulePath) -> String {
    isolated(&rules.display(path))
}

/// An optional path as a value: isolated, or `none` (a selector, not a path) when there is none.
fn isolated_or_none(path: Option<&str>) -> String {
    path.map_or_else(|| "none".to_string(), isolated)
}

/// The words a notice's values nest, made by the caller: a refusal's reason, and a panicked file's error.
pub(super) struct Nested<'n> {
    pub(super) refused: &'n dyn Fn(&RefusedReason) -> String,
    pub(super) panicked: &'n dyn Fn(&str) -> String,
}

/// The id of a source's name, `source-<code>`.
pub(super) fn source_id(source: Source) -> &'static str {
    match source {
        Source::Scan => "source-scan",
        Source::Import => "source-import",
        Source::Borders => "source-borders",
        Source::Thumbs => "source-thumbs",
        Source::Crop => "source-crop",
        Source::Writeback => "source-writeback",
        Source::Prefs => "source-prefs",
        Source::Catalog => "source-catalog",
        Source::Cache => "source-cache",
        Source::Folders => "source-folders",
        Source::Library => "source-library",
        Source::Edit => "source-edit",
        Source::Update => "source-update",
        Source::App => "source-app",
    }
}

/// Set the values of `notice`'s entries in `a`. The title and the body take the same values; each uses those it needs.
pub(super) fn args(notice: &Notice, rules: PathRules, nested: &Nested<'_>, a: &mut FluentArgs<'_>) {
    match notice {
        Notice::ScanFailed { root, error } => {
            a.set("root", shown(rules, root));
            a.set("error", error.clone());
        }
        Notice::ScanNote { root, note, count } => {
            a.set("root", shown(rules, root));
            a.set("kind", note.code());
            plural(a, *count);
        }
        Notice::RoundFailed { error }
        | Notice::CropFailed { error }
        | Notice::CatalogNotClosed { error }
        | Notice::CacheTouchesNotWritten { error }
        | Notice::CacheNotStored { error }
        | Notice::CacheWalNotMerged { error }
        | Notice::CacheDisabled { error }
        | Notice::FolderNotAdded { error }
        | Notice::FolderNotRemoved { error }
        | Notice::RescanFailed { error }
        | Notice::LibraryUnavailable { error }
        | Notice::FoldersUnreadable { error }
        | Notice::PickerFailed { error }
        | Notice::ListingFailed { error }
        | Notice::RowReadFailed { error }
        | Notice::ReloadFailed { error }
        | Notice::FacetsFailed { error }
        | Notice::TreeFailed { error }
        | Notice::SessionNotSaved { error }
        | Notice::SessionNotRead { error } => {
            a.set("error", error.clone());
        }
        Notice::RoundPanicked { message } => {
            a.set("message", message.clone());
        }
        Notice::CacheFull { height, made } => {
            a.set("height", height.to_string());
            a.set("made", made.to_string());
        }
        Notice::Cropped { folder, saved, skipped, failed, first_error } => {
            a.set("folder", isolated(folder));
            a.set("saved", saved.to_string());
            a.set("skipped", skipped.to_string());
            // A number: the entry says "none failed" for 0.
            a.set("failed", pg_model::num::size(*failed));
            let (path, error) = match first_error {
                Some(FileFailure { path, error, .. }) => (isolated(path), error.clone()),
                None => (String::new(), String::new()),
            };
            a.set("path", path);
            a.set("error", error);
        }
        Notice::CatalogRebuilt { kept, why, carried } => {
            a.set("kept", isolated(kept));
            a.set("why", why.clone());
            match carried {
                Carried::Rows { files, folders } => {
                    a.set("carried", "rows");
                    a.set("files", files.to_string());
                    a.set("folders", folders.to_string());
                }
                Carried::Nothing { reason } => {
                    a.set("carried", "nothing");
                    a.set("reason", reason.clone());
                }
            }
        }
        Notice::PrefsUnreadable { path, error, kept, keep_error } => {
            a.set("path", isolated(path));
            a.set("error", error.clone());
            a.set("kept", isolated_or_none(kept.as_deref()));
            a.set("keep_error", keep_error.clone().unwrap_or_default());
        }
        Notice::PrefsNotSaved { path, error } => {
            a.set("path", isolated(path));
            a.set("error", error.clone());
        }
        Notice::CacheTotalsFixed { kept_bytes, kept_count, bytes, count } => {
            a.set("kept_bytes", kept_bytes.to_string());
            a.set("kept_count", kept_count.to_string());
            a.set("bytes", bytes.to_string());
            a.set("count", count.to_string());
        }
        Notice::CacheStartedOver { reason, path, .. } => {
            a.set("reason", reason.code());
            a.set("path", isolated(path));
        }
        Notice::WriteFailed { error, panicked, file, temp } => {
            let error = if *panicked { (nested.panicked)(error) } else { error.clone() };
            a.set("error", error);
            a.set("file", isolated_or_none(file.as_deref()));
            a.set("temp", temp.as_deref().map(isolated).unwrap_or_default());
        }
        Notice::WriteStopped | Notice::FileNoLongerInLibrary => {}
        Notice::WriteDropped { write, count } => {
            plural(a, *count);
            // A selector, not shown: the entry has the words for each kind.
            let kind = match write {
                WriteKind::Rating => "rating",
                WriteKind::Turns => "turns",
            };
            a.set("kind", kind);
        }
        Notice::FolderAlreadyAdded { folder } => {
            a.set("folder", shown(rules, folder));
        }
        Notice::FolderRefused { folder, reason } => {
            a.set("folder", shown(rules, folder));
            a.set("reason", (nested.refused)(reason));
        }
        Notice::ViewsFailed { op, error } => {
            a.set("op", op.code());
            a.set("error", error.clone());
        }
        Notice::EditFailed { op, error } => {
            a.set("op", op.code());
            a.set("error", error.clone());
        }
        Notice::ThumbsFailed { counts }
        | Notice::BordersFailed { counts }
        | Notice::ImportFailed { counts }
        | Notice::ScanFoldersFailed { counts } => match counts {
            Some(LedgerCounts { new, earlier }) => {
                a.set("counted", "yes");
                a.set("new", new.to_string());
                a.set("earlier", earlier.to_string());
            }
            None => a.set("counted", "no"),
        },
        Notice::UnreadableFiles { pass, count } => {
            a.set("pass", pass.code());
            plural(a, *count);
        }
        Notice::LedgerNotWritten { pass, error } => {
            a.set("pass", pass.code());
            a.set("error", error.clone());
        }
        Notice::NoticesLost { count } => plural(a, *count),
        Notice::UpdateAvailable { version } => {
            a.set("version", version.clone());
        }
        Notice::Unknown { code } => {
            a.set("code", code.clone());
        }
    }
}

/// A count for a plural entry: its number selects the form, its digits are the text.
fn plural(a: &mut FluentArgs<'_>, count: u64) {
    a.set("count", pg_model::num::size(count));
    a.set("text", count.to_string());
}

#[cfg(test)]
mod tests {
    use fluent_bundle::FluentValue;
    use pg_model::notice::StartOverReason;

    use super::*;

    /// The values `notice` sets, under the Windows rules.
    fn values(notice: &Notice) -> FluentArgs<'static> {
        let refused = |_: &RefusedReason| "refused".to_string();
        let panicked = |e: &str| e.to_string();
        let nested = Nested { refused: &refused, panicked: &panicked };
        let mut a = FluentArgs::new();
        args(notice, PathRules::Windows, &nested, &mut a);
        a
    }

    fn text<'a>(a: &'a FluentArgs<'_>, name: &'static str) -> &'a str {
        match a.get(name) {
            Some(FluentValue::String(s)) => s,
            other => panic!("{name} is {other:?}"),
        }
    }

    fn isolate(path: &str) -> String {
        format!("\u{2068}{path}\u{2069}")
    }

    /// Each path a notice names is set isolated, a library folder displayed under its rules; the selectors, the
    /// counts and the errors are not.
    #[test]
    fn a_notices_paths_are_bidi_isolated() {
        let folder = PathRules::Windows.parse(r"D:\صور").unwrap();
        let a = values(&Notice::FolderAlreadyAdded { folder: folder.clone() });
        assert_eq!(text(&a, "folder"), isolate(r"D:\صور"));
        let a = values(&Notice::ScanFailed { root: folder, error: "denied".into() });
        assert_eq!(text(&a, "root"), isolate(r"D:\صور"));
        assert_eq!(text(&a, "error"), "denied");

        let a = values(&Notice::WriteFailed {
            error: "disk full".into(),
            panicked: false,
            file: Some(r"D:\a.jpg".into()),
            temp: Some(r"D:\.a.jpg.pg-write".into()),
        });
        assert_eq!(text(&a, "file"), isolate(r"D:\a.jpg"));
        assert_eq!(text(&a, "temp"), isolate(r"D:\.a.jpg.pg-write"));
        // No file: `none` is a selector, and stays one.
        let a = values(&Notice::WriteFailed { error: "e".into(), panicked: false, file: None, temp: None });
        assert_eq!(text(&a, "file"), "none");
        assert_eq!(text(&a, "temp"), "");

        let a = values(&Notice::PrefsUnreadable {
            path: r"D:\Gallery\gallery.json".into(),
            error: "bad".into(),
            kept: None,
            keep_error: None,
        });
        assert_eq!(text(&a, "path"), isolate(r"D:\Gallery\gallery.json"));
        assert_eq!(text(&a, "kept"), "none");

        let a =
            values(&Notice::Cropped { folder: r"D:\out".into(), saved: 1, skipped: 0, failed: 0, first_error: None });
        assert_eq!(text(&a, "folder"), isolate(r"D:\out"));
        assert_eq!(text(&a, "path"), "", "no first failure: nothing to isolate");

        let a = values(&Notice::CacheStartedOver {
            reason: StartOverReason::Damaged,
            path: r"D:\thumbs.sqlite.bad".into(),
            detail: "d".into(),
        });
        assert_eq!(text(&a, "path"), isolate(r"D:\thumbs.sqlite.bad"));
        assert_eq!(text(&a, "reason"), "damaged");
    }
}
