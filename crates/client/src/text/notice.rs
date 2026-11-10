//! The values of a notice's entries (`notice-<code>-title`, `-body`): each kind's fields, its paths under the rules they
//! were stored under, its codes as selectors, and the nested words (a refusal's reason) worded first, by the caller
//! ([`Nested`]: the words are `message.rs`'s, which calls this).

use fluent_bundle::FluentArgs;
use pg_model::notice::{Carried, FileFailure, LedgerCounts, Notice, Source};
use pg_model::refusal::RefusedReason;
use pg_model::rules::PathRules;
use pg_model::write::WriteKind;

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
        Notice::ScanUnreadable { root, count } => {
            a.set("root", rules.display(root));
            plural(a, *count);
        }
        Notice::ScanFailed { root, error } => {
            a.set("root", rules.display(root));
            a.set("error", error.clone());
        }
        Notice::RootRefused { root, reason } => {
            a.set("root", rules.display(root));
            a.set("reason", (nested.refused)(reason));
        }
        Notice::MountRefused { root, mount, reason } => {
            a.set("root", rules.display(root));
            a.set("mount", rules.display(mount));
            a.set("reason", (nested.refused)(reason));
        }
        Notice::ScanNote { root, note, count } => {
            a.set("root", rules.display(root));
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
        | Notice::SessionNotSaved { error } => {
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
            a.set("folder", folder.clone());
            a.set("saved", saved.to_string());
            a.set("skipped", skipped.to_string());
            // A number: the entry says "none failed" for 0.
            a.set("failed", pg_model::num::size(*failed));
            let (path, error) = match first_error {
                Some(FileFailure { path, error, .. }) => (path.clone(), error.clone()),
                None => (String::new(), String::new()),
            };
            a.set("path", path);
            a.set("error", error);
        }
        Notice::CatalogRebuilt { kept, why, carried } => {
            a.set("kept", kept.clone());
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
            a.set("path", path.clone());
            a.set("error", error.clone());
            a.set("kept", kept.clone().unwrap_or_else(|| "none".to_string()));
            a.set("keep_error", keep_error.clone().unwrap_or_default());
        }
        Notice::PrefsNotSaved { path, error } => {
            a.set("path", path.clone());
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
            a.set("path", path.clone());
        }
        Notice::WriteFailed { error, panicked, file, temp } => {
            let error = if *panicked { (nested.panicked)(error) } else { error.clone() };
            a.set("error", error);
            a.set("file", file.clone().unwrap_or_else(|| "none".to_string()));
            a.set("temp", temp.clone().unwrap_or_default());
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
            a.set("folder", rules.display(folder));
        }
        Notice::FolderRefused { folder, reason } => {
            a.set("folder", rules.display(folder));
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
        Notice::ThumbsFailed { counts } | Notice::BordersFailed { counts } | Notice::ImportFailed { counts } => {
            match counts {
                Some(LedgerCounts { new, earlier }) => {
                    a.set("counted", "yes");
                    a.set("new", new.to_string());
                    a.set("earlier", earlier.to_string());
                }
                None => a.set("counted", "no"),
            }
        }
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
