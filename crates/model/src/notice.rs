//! What the app tells the user, as data: a [`Notice`] is built
//! by the crate that has the outcome (the job, the app, the window), stored in the catalog's inbox by its code, recorded
//! in the log, and worded by `pg_client::text` for each front end. Nothing here is English, so a notice is read in the
//! reader's language, whichever one it was posted in; a stored notice is re-worded when the words change.
//!
//! Each kind is one variant: its code (permanent, the stored tag and the words' ids), its level, its source (the inbox's
//! category), its lifecycle (how the inbox keeps it) and its actions. A new kind is one variant here, its words, and the
//! call that posts it (the checklist is in the module docs below). Paths of the library are [`RulePath`]s, exact, and so is
//! a [`FileFailure`]'s path; files of the app's own (the preferences, a kept catalog) are the text the system gave.
//!
//! Serialised internally tagged by `kind`, kebab-case. A field added later is `#[serde(default)]`, so an older row still
//! reads. A row of a kind this build doesn't know is [`Notice::Unknown`], which the decoder in `pg-app` makes; it is never
//! written back.
//!
//! ## Notices (`pg_model::notice`)
//!
//! A `Notice` is a typed value, built by the crate that has the outcome and worded by `pg_client::text` for each front
//! end. Nothing in a notice is English, so it is read in the reader's language, and a stored one is worded again when the
//! words change. Each kind is one variant with:
//!
//! - its **code**, the stored tag and the root of its words' ids (`msg-notice-<code>-title`, `msg-notice-<code>-body`). Codes are
//!   permanent: a stored row of a code this build doesn't know reads as `Notice::Unknown`, which is never written back.
//!   The codes come from one table, the `ids!` macro, which also gives the words' ids; `Notice::RETIRED` lists the codes
//!   of kinds that were removed (`scan-unreadable`, `mount-refused`, `root-refused`, replaced by the folder ledger), which no kind
//!   may take again and whose rows read as `Unknown`.
//! - its **level** (info, warning, error) and its **source**, the inbox's category (`msg-source-<code>`).
//! - its **lifecycle** (`Lifecycle`): an *event* is kept each time it is posted; a *transient* is a toast and a log record
//!   only (feedback on what the user just did); a *state* is kept under its key, replaced by the next post of that key and
//!   cleared once the problem is gone; a *ledger* entry is built from a failure ledger when the inbox is read.
//! - for a state, its **resolver** (`Resolver`): what clears it. `Lifecycle::State(key, resolver)` carries it, so a state
//!   without one doesn't compile. A state about a library folder is also cleared when the folder is removed
//!   (`root-removal`, through the catalog's foreign key). The table is in `notice/resolver_table.golden`.
//! - its **actions** besides dismissing it: copy the details, retry, show the failed files.
//!
//! A state's key is its kind, the library folder it is about (resolved to the folder of the library that holds it, so
//! removing the folder clears it) and a part, which is always a code (a scan note, an operation, a pass), never a folder:
//! per-folder facts are the catalog's folder ledger.
//!
//! The stored form is JSON, internally tagged by `kind`, kebab-case; a field added later is `#[serde(default)]`.
//! `Notice::samples()` holds one notice of each kind. Each sample's stored form is pinned in `model/src/notice/golden.jsonl`
//! (the test checks that each writes its golden line and reads back as the sample, and names the kind that differs), and
//! the code is checked against the serde tag. The `ids!` table stays the one source of the codes: the tag is pinned by
//! `the_code_is_the_stored_kind` and the golden test, and a derive (strum) would be a third mechanism.
//!
//! Text a notice carries is tidied (`notice::tidy`): control characters become spaces, and past its cap (`ERROR_CAP`, 2 KiB,
//! for an error) it is cut at a character's edge and ends in `…`, with the mark inside the cap, so the result is never
//! more than the cap's bytes. A path in the words is bidi-isolated (U+2068 … U+2069, `client/src/text/notice.rs`), so a
//! folder named in Arabic or Hebrew doesn't reorder the words around it; the bundle's own isolation stays off.
//!
//! ### Adding a kind
//!
//! 1. A variant of `Notice`, with its code in the `ids!` table, and its arms in `level`, `source` and `lifecycle`
//!    (exhaustive matches: it doesn't compile without them). A state names its resolver there.
//! 2. A sample in `Notice::samples()`, its line in `notice/golden.jsonl`, and an arm in the test `ordinal`. A state's row
//!    in `notice/resolver_table.golden`.
//! 3. Its words: the `msg-notice-<code>-title` and `-body` entries in `client/locales/en/text.ftl`, and its values in
//!    `client/src/text/notice.rs` (`args`). The parity test checks the file's entries against the samples' ids.
//! 4. The call that posts it, from the crate that has the outcome, and the call that clears it, where its resolver is.
//!
//! A removed kind's code moves to the `retired:` list of `ids!`. A new action is a variant of `Action`, the window's
//! handler for it, and the kinds that offer it in `Notice::actions`.
//!
//! ### What clears a kept notice
//!
//! Every kept kind and part, and its resolvers, in the samples' order. The table is the output of
//! `Notice::resolver_markdown()` (from `Notice::resolver_table()`), pinned in `notice/resolver_table.golden`. The test
//! `the_resolver_table_is_the_golden_one` fails, printing the table, when the output differs from that file.
//!
//! The table lists every part the type allows. Which are posted: `unreadable-files` by the thumbnails' and the borders'
//! passes (the import doesn't post it), and `ledger-not-written` by the thumbnails' pass only (see `ledger.rs`).
//!
//! What each resolver is, in the code:
//!
//! - `next-success(op)`: the next success of the same operation clears it (the window's reads and edits through
//!   `app::resolved`, the job's round, a ledger write).
//! - `complete-pass(pass)`: the next complete run of the pass, one that read every file it had to (the runner's
//!   `files_read`, and `cache-full` after a complete thumbnail pass that didn't fill the cache).
//! - `status-poll`: the writer's status as it changes. `write-failed` is posted by `pg_writeback::Writer` at its one status
//!   choke point when the error goes from none to some (or changes) and cleared when it goes back to none; `write-stopped`
//!   means "restarts exhausted", posted where `pg_app::Library::tend` gives the writer up (a stop at shutdown, or a panic a
//!   restart recovers from, posts nothing). `Library::start` clears both before the new writer starts, since a new writer
//!   starts with no error and would never see the change that clears an earlier run's state.
//! - `touches-recovered` and `store-recovered`: the thumbnail cache's writer publishes `TouchesWritten` at its first success
//!   after `TouchesNotWritten`, and `Stored` at its first success after `NotStored`. Each clears only the notice of its own
//!   failure. `pg_thumbcache::Event::effect` turns its events into a post or a clear, which the app's forwarder applies (the
//!   command line prints the posts only).
//! - `next-save`: the next save of the preferences (`PrefStore::update`).
//! - `root-walk`: the next complete walk of the whole library folder replaces the folder's kept scan notes with the ones
//!   it found.
//!

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::refusal::RefusedReason;
use crate::rules::Path as RulePath;
use crate::size::ThumbHeight;
use crate::write::WriteKind;

/// How much a notice matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    /// Nothing to act on.
    Info,
    /// Worth the user's attention.
    Warning,
    /// Something failed.
    Error,
}

impl Level {
    /// The level's code, as the catalog stores it (`notices.level`, `baseline.sql`) and the panels colour it: 0 info,
    /// 1 warning, 2 error.
    pub fn code(self) -> i32 {
        match self {
            Level::Info => 0,
            Level::Warning => 1,
            Level::Error => 2,
        }
    }

    /// The level with `code` ([`Level::code`]). Any other code is an error, so a damaged row shows as one.
    pub fn from_code(code: i32) -> Level {
        match code {
            0 => Level::Info,
            1 => Level::Warning,
            _ => Level::Error,
        }
    }
}

/// Where a notice comes from: the inbox's category, worded as `msg-source-<code>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// The library scanner.
    Scan,
    /// The import.
    Import,
    /// Border detection.
    Borders,
    /// Thumbnail making.
    Thumbs,
    /// Cropping.
    Crop,
    /// Writing edits back into the files.
    Writeback,
    /// The preferences.
    Prefs,
    /// The catalog.
    Catalog,
    /// The thumbnail cache.
    Cache,
    /// The library's folders.
    Folders,
    /// The library as a whole.
    Library,
    /// Edits made by the user.
    Edit,
    /// The update check.
    Update,
    /// The app itself.
    App,
}

impl Source {
    /// Every source, in the inbox's order.
    pub const ALL: [Source; 14] = [
        Source::Scan,
        Source::Import,
        Source::Borders,
        Source::Thumbs,
        Source::Crop,
        Source::Writeback,
        Source::Prefs,
        Source::Catalog,
        Source::Cache,
        Source::Folders,
        Source::Library,
        Source::Edit,
        Source::Update,
        Source::App,
    ];

    /// The source's code, as the words' ids name it (`msg-source-<code>`).
    pub fn key(self) -> &'static str {
        match self {
            Source::Scan => "scan",
            Source::Import => "import",
            Source::Borders => "borders",
            Source::Thumbs => "thumbs",
            Source::Crop => "crop",
            Source::Writeback => "writeback",
            Source::Prefs => "prefs",
            Source::Catalog => "catalog",
            Source::Cache => "cache",
            Source::Folders => "folders",
            Source::Library => "library",
            Source::Edit => "edit",
            Source::Update => "update",
            Source::App => "app",
        }
    }
}

/// What a scan counted that the user may want to know, by its code (`scan-note`'s selector).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanNote {
    /// Files skipped because another path (a hard link) or another folder of the library holds them.
    #[serde(rename = "duplicates")]
    Duplicates,
    /// Reparse points (links, cloud placeholders) indexed without being followed.
    #[serde(rename = "reparse")]
    SkippedReparse,
    /// Files whose old row could not be told apart from another (nothing was bound).
    #[serde(rename = "ambiguous")]
    Ambiguous,
    /// Rows of files that went missing, kept with their ratings, whose new file wasn't found.
    #[serde(rename = "unpaired")]
    UnpairedWithData,
    /// Rows whose file was replaced by another with the same name: their content was reset.
    #[serde(rename = "replaced")]
    Replaced,
    /// Rows whose file was found again under a new identity (a renumbered drive, or a move while closed).
    #[serde(rename = "rebound")]
    Rebound,
    /// Volumes with no file system UUID: their files are identified by a device number, which a restart can renumber.
    #[serde(rename = "volume-id")]
    VolumeFallback,
}

impl ScanNote {
    /// Every note, in the inbox's order.
    pub const ALL: [ScanNote; 7] = [
        ScanNote::Duplicates,
        ScanNote::SkippedReparse,
        ScanNote::Ambiguous,
        ScanNote::UnpairedWithData,
        ScanNote::Replaced,
        ScanNote::Rebound,
        ScanNote::VolumeFallback,
    ];

    /// The note's code: what it is stored as, and what its words select on.
    pub fn key(self) -> &'static str {
        match self {
            ScanNote::Duplicates => "duplicates",
            ScanNote::SkippedReparse => "reparse",
            ScanNote::Ambiguous => "ambiguous",
            ScanNote::UnpairedWithData => "unpaired",
            ScanNote::Replaced => "replaced",
            ScanNote::Rebound => "rebound",
            ScanNote::VolumeFallback => "volume-id",
        }
    }

    /// Whether the user should act on it (a warning), not only know it.
    pub fn is_warning(self) -> bool {
        matches!(self, ScanNote::Ambiguous | ScanNote::UnpairedWithData | ScanNote::VolumeFallback)
    }

    /// How the inbox keeps the note, by what it describes (see the module docs): a condition of the library folder
    /// (hard links and reparse points come out the same on every walk) is kept under `(root, code)` and replaced only by
    /// the next walk of the whole folder; what one walk did (the rows it replaced or bound again) is an event of that
    /// walk, with its count.
    pub fn lifecycle(self) -> NoteLifecycle {
        match self {
            ScanNote::Duplicates
            | ScanNote::SkippedReparse
            | ScanNote::Ambiguous
            | ScanNote::UnpairedWithData
            | ScanNote::VolumeFallback => NoteLifecycle::Condition,
            ScanNote::Replaced | ScanNote::Rebound => NoteLifecycle::Event,
        }
    }
}

/// What a [`ScanNote`] describes ([`ScanNote::lifecycle`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteLifecycle {
    /// A condition of the library folder: a [`Lifecycle::State`] keyed `(root, code)`, resolved by [`Resolver::RootWalk`].
    Condition,
    /// What one walk did: a [`Lifecycle::Event`].
    Event,
}

/// Why the thumbnail cache's file was started over when it was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartOverReason {
    /// SQLite found the file's pages don't hold together (`DatabaseCorrupt`): moved aside.
    Damaged,
    /// The file is not a database at all (`NotADatabase`): moved aside.
    NotADatabase,
    /// A cache made for another catalog: deleted, its cap kept.
    OtherCatalog,
    /// A cache made by another build (another layout version, schema, application id or vacuum setting): deleted, its
    /// cap kept.
    OtherBuild,
}

impl StartOverReason {
    /// Every reason, in the inbox's order.
    pub const ALL: [StartOverReason; 4] = [
        StartOverReason::Damaged,
        StartOverReason::NotADatabase,
        StartOverReason::OtherCatalog,
        StartOverReason::OtherBuild,
    ];

    /// The reason's code, which the words select on.
    pub fn key(self) -> &'static str {
        match self {
            StartOverReason::Damaged => "damaged",
            StartOverReason::NotADatabase => "not-a-database",
            StartOverReason::OtherCatalog => "other-catalog",
            StartOverReason::OtherBuild => "other-build",
        }
    }
}

/// The rows a rebuilt catalog took from the old one (`pg_catalog::Rebuilt`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "carried", rename_all = "kebab-case")]
pub enum Carried {
    /// The old catalog's rows, copied as they were: its files and its library folders.
    Rows {
        /// How many files were copied.
        files: u64,
        /// How many library folders were copied.
        folders: u64,
    },
    /// No rows were copied (the old catalog was not one this build can read, or the copy failed), and why. The new catalog
    /// is empty; the next scan fills it.
    Nothing {
        /// Why no rows were copied.
        reason: String,
    },
}

/// A thumbnail that couldn't follow a rewrite of its file (the cache busy or closing): the file, and the cache's error
/// as its detail text. The thumbnail is made again, and the write is done all the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailLag {
    /// The file whose thumbnail lagged.
    pub file: crate::ids::FileId,
    /// The cache's error, as text.
    pub error: String,
}

/// A file a pass or the writer failed on: its path, and why (the error's text, or a caught panic's message).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFailure {
    /// The file that failed.
    pub path: RulePath,
    /// Why it failed, as text.
    pub error: String,
    /// The pass panicked on the file (caught): `error` is the panic's message.
    #[serde(default)]
    pub panicked: bool,
}

/// The failures of a `Cropped` notice: a list, or the one failure an older build kept (`first_error`), or none (null).
fn failures_or_one<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Vec<FileFailure>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        Many(Vec<FileFailure>),
        One(FileFailure),
    }
    Ok(match Option::<OneOrMany>::deserialize(de)? {
        None => Vec::new(),
        Some(OneOrMany::Many(failures)) => failures,
        Some(OneOrMany::One(failure)) => vec![failure],
    })
}

/// What a failed edit was doing (`Notice::EditFailed`), one inbox entry each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EditOp {
    /// A rating.
    Rate,
    /// A rotation.
    Rotate,
    /// A border mark.
    MarkBorder,
}

impl EditOp {
    /// Every edit operation, in the inbox's order.
    pub const ALL: [EditOp; 3] = [EditOp::Rate, EditOp::Rotate, EditOp::MarkBorder];

    /// The operation's code, as the words select on it.
    pub fn key(self) -> &'static str {
        match self {
            EditOp::Rate => "rate",
            EditOp::Rotate => "rotate",
            EditOp::MarkBorder => "mark-border",
        }
    }
}

/// What a failed change to the saved views was doing (`Notice::ViewsFailed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ViewOp {
    /// Listing the saved views.
    List,
    /// Saving a view.
    Save,
    /// Deleting a view.
    Delete,
}

impl ViewOp {
    /// Every view operation, in the inbox's order.
    pub const ALL: [ViewOp; 3] = [ViewOp::List, ViewOp::Save, ViewOp::Delete];

    /// The operation's code, as the words select on it.
    pub fn key(self) -> &'static str {
        match self {
            ViewOp::List => "list",
            ViewOp::Save => "save",
            ViewOp::Delete => "delete",
        }
    }
}

/// A pass whose failed files the failure ledger holds (the import's are its failed analyses): one inbox entry each,
/// built when the inbox is read, never stored as a notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FilePass {
    /// The thumbnail pass.
    Thumbs,
    /// The border detection pass.
    Borders,
    /// The import pass.
    Import,
}

impl FilePass {
    /// Every pass, in the inbox's order.
    pub const ALL: [FilePass; 3] = [FilePass::Thumbs, FilePass::Borders, FilePass::Import];

    /// The pass's code, as the catalog stores it.
    pub fn key(self) -> &'static str {
        match self {
            FilePass::Thumbs => "thumbs",
            FilePass::Borders => "borders",
            FilePass::Import => "import",
        }
    }
}

/// A ledger the inbox shows as one entry, built when the inbox is read: a pass's failed files, or the folders the scans
/// couldn't read (the catalog's per-folder ledger). Its code is what its acknowledgement is stored under
/// (`ledger_ack.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LedgerKind {
    /// A pass's failed files.
    File(FilePass),
    /// The folders the scans couldn't read.
    ScanFolders,
}

impl LedgerKind {
    /// Every ledger, in the inbox's order.
    pub const ALL: [LedgerKind; 4] = [
        LedgerKind::File(FilePass::Thumbs),
        LedgerKind::File(FilePass::Borders),
        LedgerKind::File(FilePass::Import),
        LedgerKind::ScanFolders,
    ];

    /// The ledger's code: a pass's own, or `scan-folders`.
    pub fn key(self) -> &'static str {
        match self {
            LedgerKind::File(pass) => pass.key(),
            LedgerKind::ScanFolders => "scan-folders",
        }
    }

    /// The ledger with `key` ([`LedgerKind::key`]), if any.
    pub fn from_key(key: &str) -> Option<LedgerKind> {
        LedgerKind::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// The ledger's entry, with its counts.
    pub fn notice(self, counts: Option<LedgerCounts>) -> Notice {
        match self {
            LedgerKind::File(FilePass::Thumbs) => Notice::ThumbsFailed { counts },
            LedgerKind::File(FilePass::Borders) => Notice::BordersFailed { counts },
            LedgerKind::File(FilePass::Import) => Notice::ImportFailed { counts },
            LedgerKind::ScanFolders => Notice::ScanFoldersFailed { counts },
        }
    }
}

/// A ledger entry's counts, read only while the inbox is open: the failures since the user last saw the entry, and the
/// ones before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerCounts {
    /// The failures since the user last saw the entry.
    pub new: u64,
    /// The failures before those.
    pub earlier: u64,
}

/// Something the app tells the user. See the module's header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Notice {
    // The scans. The folders a scan couldn't read, the mounts under them it refuses and a library folder it refuses are
    // not notices: they are the catalog's per-folder ledger, one entry ([`Notice::ScanFoldersFailed`]). Their old kinds'
    // codes are retired ([`Notice::RETIRED`]).
    /// A scan that failed. Its batches before the error are in.
    ScanFailed {
        /// The library folder being scanned.
        root: RulePath,
        /// Why the scan failed.
        error: String,
    },
    /// What a scan of a folder counted that the user may want to know, `count` of them. A condition of the folder is
    /// kept; what one walk did is an event ([`ScanNote::lifecycle`]).
    ScanNote {
        /// The library folder the note is about.
        root: RulePath,
        /// Which note it is.
        note: ScanNote,
        /// How many of the note the scan counted.
        count: u64,
    },

    // The job.
    /// A round of the job that failed.
    RoundFailed {
        /// Why the round failed.
        error: String,
    },
    /// A round that panicked (caught): the job went on with the next request.
    RoundPanicked {
        /// The caught panic's message.
        message: String,
    },
    /// The thumbnail cache is full: `made` thumbnails at `height` px weren't made.
    CacheFull {
        /// The thumbnail height whose thumbnails weren't made.
        height: ThumbHeight,
        /// How many thumbnails were made at that height before the cache filled.
        made: u64,
    },
    /// A crop job that ran to the end of its request: the folder its copies went to, and how it went. `failures` are the
    /// first few that failed, each with its file (`failed` counts them all).
    Cropped {
        /// The folder the copies were saved to.
        folder: String,
        /// How many files were copied and cut.
        saved: u64,
        /// How many files were left alone.
        skipped: u64,
        /// How many files failed.
        failed: u64,
        /// A row an older build wrote holds one failure as `first_error` (or null): that reads as the list of one.
        #[serde(default, alias = "first_error", deserialize_with = "failures_or_one")]
        failures: Vec<FileFailure>,
    },
    /// A crop job that failed before it could save anything.
    CropFailed {
        /// Why the crop failed before it saved anything.
        error: String,
    },
    /// The borders cut off the originals (`pg_crop::cut`): what became of each file. The first failures are kept with
    /// their files, as the copies' notice keeps them.
    BordersCut {
        /// How many originals were cut.
        saved: u64,
        /// How many were skipped.
        skipped: u64,
        /// How many were of a format that can't be cut.
        unsupported: u64,
        /// How many failed.
        failed: u64,
        /// The first failures, each with its file.
        failures: Vec<FileFailure>,
    },
    /// A cut that failed before it could change anything.
    CutFailed {
        /// Why the cut failed before it changed anything.
        error: String,
    },

    // The app's files.
    /// The catalog didn't match this build's schema, so it was moved aside (to `kept`) and a new one made, which took the
    /// old rows or none (`carried`). `why` is the first difference, as the schema check words it.
    CatalogRebuilt {
        /// Where the old catalog was moved aside to.
        kept: String,
        /// The first difference the schema check found.
        why: String,
        /// What the new catalog took from the old one.
        carried: Carried,
    },
    /// The catalog's last connection couldn't be closed at the end (logged only: the inbox has closed by then).
    CatalogNotClosed {
        /// Why the connection wouldn't close.
        error: String,
    },
    /// The preferences file didn't parse: the defaults are in use, and the file was kept aside as `kept`, or couldn't be
    /// (`keep_error`).
    PrefsUnreadable {
        /// The preferences file.
        path: String,
        /// Why it didn't parse.
        error: String,
        /// Where it was kept aside, if it was.
        kept: Option<String>,
        /// Why it couldn't be kept aside, if it wasn't.
        keep_error: Option<String>,
    },
    /// The preferences changed in memory but couldn't be saved; the change holds until the app closes.
    PrefsNotSaved {
        /// The preferences file.
        path: String,
        /// Why the save failed.
        error: String,
    },

    // The thumbnail cache.
    /// The cache's totals were off when it was opened, and were fixed: what they were, and what the entries hold.
    CacheTotalsFixed {
        /// The bytes the totals said before the fix.
        kept_bytes: i64,
        /// The entries the totals said before the fix.
        kept_count: i64,
        /// The bytes the entries hold.
        bytes: i64,
        /// The entries the cache holds.
        count: i64,
    },
    /// The times entries were shown couldn't be written; they stay queued for the next try.
    CacheTouchesNotWritten {
        /// Why the touches couldn't be written.
        error: String,
    },
    /// An on-demand thumbnail couldn't be stored (it was shown anyway).
    CacheNotStored {
        /// Why the thumbnail couldn't be stored.
        error: String,
    },
    /// The cache's WAL couldn't be merged when its writer closed.
    CacheWalNotMerged {
        /// Why the WAL couldn't be merged.
        error: String,
    },
    /// The cache's file was started over when it was opened. `path` is where the old file is (a damaged one is moved
    /// aside) or the new file (one this build can't use is deleted); `detail` is the technical account, for the log.
    CacheStartedOver {
        /// Why the cache was started over.
        reason: StartOverReason,
        /// Where the old or new file is.
        path: String,
        /// The technical account, for the log.
        detail: String,
    },
    /// The cache couldn't be opened: thumbnails are made each time they are shown, and none is kept.
    CacheDisabled {
        /// Why the cache couldn't be opened.
        error: String,
    },

    // The write-back.
    /// The writer's latest write failed: why, and for a copy that may be incomplete, the file and its temporary copy.
    WriteFailed {
        /// Why the write failed.
        error: String,
        /// Whether the writer panicked.
        panicked: bool,
        /// The file being written, if it may be incomplete.
        file: Option<String>,
        /// Its temporary copy, if it has one.
        temp: Option<String>,
    },
    /// The writer stopped: nothing more is written into the files until the app starts again.
    WriteStopped,
    /// Queued writes were dropped (their files went), `count` of them, the last of them a `write`: an event (a lost edit
    /// is a fact, not a condition), posted by the writer.
    WriteDropped {
        /// The kind of the last dropped write.
        write: WriteKind,
        /// How many writes were dropped.
        count: u64,
    },

    // The library's folders.
    /// A folder added that the library already had: it is rescanned.
    FolderAlreadyAdded {
        /// The folder the library already had.
        folder: RulePath,
    },
    /// A folder the scanner refuses, so it wasn't added.
    FolderRefused {
        /// The folder that was refused.
        folder: RulePath,
        /// Why the scanner refused it.
        reason: RefusedReason,
    },
    /// A folder couldn't be added to the library.
    FolderNotAdded {
        /// Why it couldn't be added.
        error: String,
    },
    /// A folder couldn't be removed from the library.
    FolderNotRemoved {
        /// Why the removal failed.
        error: String,
    },
    /// A copy of a file in a folder being removed from the library is kept: the write was under way when the file was
    /// deleted outside the app, so its temp (the only whole copy of the edit) stays at `temp`, and the write goes.
    /// Keyless, as an event: the folder is not in the library, so the notice is not one of its folders.
    FolderRemovalUnfinished {
        /// The folder being removed.
        folder: RulePath,
        /// The temporary copy kept for the edit.
        temp: RulePath,
    },
    /// A rescan of a folder failed.
    RescanFailed {
        /// Why the rescan failed.
        error: String,
    },
    /// The background work (the job and the writer) couldn't start.
    LibraryUnavailable {
        /// Why the background work couldn't start.
        error: String,
    },
    /// The window's thread for the library's changes couldn't start: the changes the background work makes won't show
    /// until the app is started again.
    ChangesUnseen {
        /// Why the window's thread couldn't start.
        error: String,
    },
    /// The catalog's list of the library's folders couldn't be read.
    FoldersUnreadable {
        /// Why the folders couldn't be read.
        error: String,
    },
    /// A folder couldn't be picked (the system's dialog failed).
    PickerFailed {
        /// Why the dialog failed.
        error: String,
    },

    // The window's reads and edits.
    /// The list of files couldn't be read.
    ListingFailed {
        /// Why the listing failed.
        error: String,
    },
    /// A file's row (its details) couldn't be read.
    RowReadFailed {
        /// Why the row couldn't be read.
        error: String,
    },
    /// A file's row couldn't be read again after it changed.
    ReloadFailed {
        /// Why the row couldn't be read again.
        error: String,
    },
    /// The filter bar's counts couldn't be read.
    FacetsFailed {
        /// Why the counts couldn't be read.
        error: String,
    },
    /// The folder tree couldn't be read.
    TreeFailed {
        /// Why the tree couldn't be read.
        error: String,
    },
    /// The saved views couldn't be listed, saved or deleted.
    ViewsFailed {
        /// Which change to the saved views failed.
        op: ViewOp,
        /// Why it failed.
        error: String,
    },
    /// The session (where the window was) couldn't be saved.
    SessionNotSaved {
        /// Why the session couldn't be saved.
        error: String,
    },
    /// The saved session (where the window was) couldn't be read: the window starts from the default listing, and the
    /// stored text stays as it is until the next save replaces it.
    SessionNotRead {
        /// Why the saved session couldn't be read.
        error: String,
    },
    /// A rating, a turn or a border mark couldn't be saved.
    EditFailed {
        /// Which edit failed.
        op: EditOp,
        /// Why it failed.
        error: String,
    },

    // The failure ledger (built when the inbox is read).
    /// Files whose thumbnail couldn't be made.
    ThumbsFailed {
        /// The failure counts, when they could be read.
        counts: Option<LedgerCounts>,
    },
    /// Files whose borders couldn't be looked for.
    BordersFailed {
        /// The failure counts, when they could be read.
        counts: Option<LedgerCounts>,
    },
    /// Files that couldn't be imported (their analysis failed).
    ImportFailed {
        /// The failure counts, when they could be read.
        counts: Option<LedgerCounts>,
    },
    /// Folders the scans couldn't read, or refuse (a mount under a library folder, or the library folder itself): the
    /// catalog's per-folder ledger.
    ScanFoldersFailed {
        /// The failure counts, when they could be read.
        counts: Option<LedgerCounts>,
    },
    /// Files a pass couldn't read (an offline drive, a file in use), `count` of them: retried by the next pass. `drives`
    /// names the drives those files are on, as the job's lanes know them (`E:`), when the job has more than one; empty
    /// otherwise, and not stored (a notice kept before drives were named reads as it did).
    UnreadableFiles {
        /// The pass whose files couldn't be read.
        pass: FilePass,
        /// How many files couldn't be read.
        count: u64,
        /// The drives those files are on, when the job has more than one.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        drives: Vec<String>,
    },
    /// The thumbnails' failures couldn't be written to their failure ledger (a catalog locked for long, a full disk): its
    /// files are tried again by the next pass. Thumbnails only: the borders and the import write their ledger rows in
    /// the transaction of their own work, so a failure there fails the batch instead.
    LedgerNotWritten {
        /// Why the failure ledger couldn't be written.
        error: String,
    },

    // The inbox itself, and the app.
    /// Notices that couldn't be kept (the inbox's queue was full, or its writes failed), `count` of them.
    NoticesLost {
        /// How many notices were lost.
        count: u64,
    },
    /// A file a notice named is no longer in the library (shown when it is revealed).
    FileNoLongerInLibrary,
    /// A newer version of the app is out (reserved: nothing checks yet).
    UpdateAvailable {
        /// The version that is out.
        version: String,
    },
    /// A stored notice of a kind this build doesn't know, by its code. Made only by the decoder; never written.
    #[serde(skip)]
    Unknown {
        /// The code the notice was stored under.
        code: String,
    },
}

/// How the inbox keeps a notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lifecycle<'a> {
    /// Kept once each time it is posted; never replaced.
    Event,
    /// Shown as a toast and logged, never kept: feedback on what the user just did.
    Transient,
    /// One entry per ledger, built from the catalog's ledger when the inbox is read; never stored.
    Ledger(LedgerKind),
    /// Kept under its key: a new post of the same key replaces it (and counts), and what resolves it ([`Resolver`])
    /// clears it once the problem is gone. Every state names its resolver: one that has none doesn't compile.
    State(Key<'a>, Resolver),
}

/// What clears a kept notice (a [`Lifecycle::State`]) once the problem is gone. A state about a library folder is also
/// cleared when that folder is removed ([`Resolver::RootRemoval`], through the catalog's foreign key), whatever its
/// resolver; [`Notice::resolvers`] lists both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolver {
    /// The next success of the same operation.
    NextSuccess(Op),
    /// The next complete run of the pass (one that read every file it had to).
    CompletePass(FilePass),
    /// The writer's status, as it changes: an error gone back to none, a writer started again.
    StatusPoll,
    /// The thumbnail cache's writer, at its first success after a failed write of the times entries were shown.
    TouchesRecovered,
    /// The thumbnail cache's writer, at its first success after a failed store of a thumbnail.
    StoreRecovered,
    /// The next save of the preferences.
    NextSave,
    /// The next complete walk of the whole library folder, which replaces the folder's kept notices with the ones it
    /// found.
    RootWalk,
    /// The library folder's removal (always, for a state about a library folder).
    RootRemoval,
}

impl Resolver {
    /// The resolver's token, as the golden table `notice/resolver_table.golden` writes it: `next-success(edit-rate)`.
    pub fn token(self) -> String {
        match self {
            Resolver::NextSuccess(op) => format!("next-success({})", op.key()),
            Resolver::CompletePass(pass) => format!("complete-pass({})", pass.key()),
            Resolver::StatusPoll => "status-poll".to_string(),
            Resolver::TouchesRecovered => "touches-recovered".to_string(),
            Resolver::StoreRecovered => "store-recovered".to_string(),
            Resolver::NextSave => "next-save".to_string(),
            Resolver::RootWalk => "root-walk".to_string(),
            Resolver::RootRemoval => "root-removal".to_string(),
        }
    }
}

/// An operation whose next success clears its failure's state ([`Resolver::NextSuccess`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// A round of the job.
    Round,
    /// A pass's write of its failures to the ledger.
    LedgerWrite(FilePass),
    /// The window's reads.
    Listing,
    /// The window's row reads.
    RowRead,
    /// The window's reloads of a row.
    Reload,
    /// The filter bar's counts.
    Facets,
    /// The folder tree.
    Tree,
    /// The window's save of its session.
    Session,
    /// A change to the saved views.
    Views(ViewOp),
    /// An edit.
    Edit(EditOp),
}

impl Op {
    /// The operation's code.
    pub fn key(self) -> &'static str {
        match self {
            Op::Round => "round",
            Op::LedgerWrite(FilePass::Thumbs) => "ledger-write-thumbs",
            Op::LedgerWrite(FilePass::Borders) => "ledger-write-borders",
            Op::LedgerWrite(FilePass::Import) => "ledger-write-import",
            Op::Listing => "listing",
            Op::RowRead => "row-read",
            Op::Reload => "reload",
            Op::Facets => "facets",
            Op::Tree => "tree",
            Op::Session => "session",
            Op::Views(ViewOp::List) => "views-list",
            Op::Views(ViewOp::Save) => "views-save",
            Op::Views(ViewOp::Delete) => "views-delete",
            Op::Edit(EditOp::Rate) => "edit-rate",
            Op::Edit(EditOp::Rotate) => "edit-rotate",
            Op::Edit(EditOp::MarkBorder) => "edit-mark-border",
        }
    }
}

/// What happened, for the kept notices it clears. A success site names its trigger, never a kind: [`resolves`] says
/// which kept states a trigger clears, so the rule for each kind is in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The operation succeeded.
    Op(Op),
    /// A pass ran to its end and read every file it had to.
    Pass(FilePass),
    /// The writer's status is none again: its error is gone, or the writer started again.
    Status,
    /// The thumbnail cache's writer wrote the times entries were shown, after a failed write of them.
    TouchesRecovered,
    /// The thumbnail cache's writer stored a thumbnail, after a failed store.
    StoreRecovered,
    /// The preferences were saved.
    Save,
}

/// Whether `trigger` clears a kept state whose resolver is `resolver`. [`Resolver::RootWalk`] and
/// [`Resolver::RootRemoval`] have no trigger: a complete walk replaces its folder's notes (a replace scoped to the
/// folder), and a removal goes through the catalog's foreign key. Neither matches here.
pub fn resolves(resolver: Resolver, trigger: Trigger) -> bool {
    match (resolver, trigger) {
        (Resolver::NextSuccess(op), Trigger::Op(done)) => op == done,
        (Resolver::CompletePass(pass), Trigger::Pass(done)) => pass == done,
        (Resolver::StatusPoll, Trigger::Status)
        | (Resolver::TouchesRecovered, Trigger::TouchesRecovered)
        | (Resolver::StoreRecovered, Trigger::StoreRecovered)
        | (Resolver::NextSave, Trigger::Save) => true,
        _ => false,
    }
}

/// One row of the resolver table ([`Notice::resolver_table`]): a kept kind and part, and what clears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolverRow {
    /// The kind's code.
    pub code: &'static str,
    /// The part's code, for a kind kept per part.
    pub part: Option<&'static str>,
    /// What clears it: its resolver, then [`Resolver::RootRemoval`] for a state about a library folder.
    pub resolvers: Vec<Resolver>,
}

/// A kept notice's key: its kind's code, the library folder it is about (resolved to the folder of the library that
/// holds it when it is posted, so removing that folder clears it), and the part of that folder's notices it is.
///
/// A producer clears kept notices by a key too ([`Key::all`]): a key with no part clears every part of the kind and
/// folder (a complete walk of a library folder replaces its notes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key<'a> {
    /// The notice's code ([`Notice::code`]).
    pub code: &'static str,
    /// The folder the key names, if it names one.
    pub root: Option<&'a RulePath>,
    /// The part of the folder the key names, if it names one.
    pub part: Option<Part>,
}

impl<'a> Key<'a> {
    /// Every kept notice of the kind with `code` (a [`Notice::code`]) about `root`, whatever its part.
    pub fn all(code: &'static str, root: Option<&'a RulePath>) -> Key<'a> {
        Key { code, root, part: None }
    }

    /// The key, owning its folder: to send it to another thread.
    pub fn to_buf(&self) -> KeyBuf {
        KeyBuf { code: self.code, root: self.root.cloned(), part: self.part }
    }
}

/// A [`Key`] that owns its folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBuf {
    /// The notice's code ([`Notice::code`]).
    pub code: &'static str,
    /// The folder the key names, if it names one.
    pub root: Option<RulePath>,
    /// The part of the folder the key names, if it names one.
    pub part: Option<Part>,
}

impl KeyBuf {
    /// The key, borrowing its folder.
    pub fn key(&self) -> Key<'_> {
        Key { code: self.code, root: self.root.as_ref(), part: self.part }
    }
}

/// What tells two kept notices of one kind and folder apart. A part is a code, never a folder: per-folder facts are the
/// catalog's ledger (a key matches an exact part, and a folder's bytes can't be compared as the Windows rules compare
/// names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// A code: the note, the operation, the pass.
    Code(&'static str),
}

/// What the user can do from a notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Show the files the entry is about in the grid (the import's failed files: the "Unreadable" status filter).
    ShowFailedFiles,
    /// Show one failed file in the grid (a row of a ledger entry).
    Reveal,
    /// Copy the notice's words and details.
    CopyDetails,
    /// Try the entry's files again.
    Retry,
    /// Open the inbox (a toast's action).
    OpenInbox,
}

/// The ids of each kind, from one table: its code (the stored tag), and its words' ids, `msg-notice-<code>-title` and
/// `-body`. A test checks the code against the serde tag. The retired codes are the kinds a build stored before they
/// were removed: never used again, and a row of one reads as [`Notice::Unknown`] (it has no variant to read into).
macro_rules! ids {
    ($($variant:ident => $code:literal),* $(,)?; retired: $($retired:literal),* $(,)?) => {
        impl Notice {
            /// The codes of the kinds removed: never a kind's code again.
            pub const RETIRED: &'static [&'static str] = &[$($retired),*];

            /// The codes of the kinds this build has (each kind's [`Notice::code`]).
            pub const CODES: &'static [&'static str] = &[$($code),*];

            /// The kind's code: the stored tag, permanent.
            pub fn code(&self) -> &'static str {
                match self {
                    $(Notice::$variant { .. } => $code,)*
                }
            }

            /// The id of the kind's title, in the words.
            pub fn title_id(&self) -> &'static str {
                match self {
                    $(Notice::$variant { .. } => concat!("msg-notice-", $code, "-title"),)*
                }
            }

            /// The id of the kind's body, in the words.
            pub fn body_id(&self) -> &'static str {
                match self {
                    $(Notice::$variant { .. } => concat!("msg-notice-", $code, "-body"),)*
                }
            }
        }
    };
}

ids! {
    ScanFailed => "scan-failed",
    ScanNote => "scan-note",
    RoundFailed => "round-failed",
    RoundPanicked => "round-panicked",
    CacheFull => "cache-full",
    Cropped => "cropped",
    CropFailed => "crop-failed",
    BordersCut => "borders-cut",
    CutFailed => "cut-failed",
    CatalogRebuilt => "catalog-rebuilt",
    CatalogNotClosed => "catalog-not-closed",
    PrefsUnreadable => "prefs-unreadable",
    PrefsNotSaved => "prefs-not-saved",
    CacheTotalsFixed => "cache-totals-fixed",
    CacheTouchesNotWritten => "cache-touches-not-written",
    CacheNotStored => "cache-not-stored",
    CacheWalNotMerged => "cache-wal-not-merged",
    CacheStartedOver => "cache-started-over",
    CacheDisabled => "cache-disabled",
    WriteFailed => "write-failed",
    WriteStopped => "write-stopped",
    WriteDropped => "write-dropped",
    FolderAlreadyAdded => "folder-already-added",
    FolderRefused => "folder-refused",
    FolderNotAdded => "folder-not-added",
    FolderNotRemoved => "folder-not-removed",
    FolderRemovalUnfinished => "folder-removal-unfinished",
    RescanFailed => "rescan-failed",
    LibraryUnavailable => "library-unavailable",
    ChangesUnseen => "changes-unseen",
    FoldersUnreadable => "folders-unreadable",
    PickerFailed => "picker-failed",
    ListingFailed => "listing-failed",
    RowReadFailed => "row-read-failed",
    ReloadFailed => "reload-failed",
    FacetsFailed => "facets-failed",
    TreeFailed => "tree-failed",
    ViewsFailed => "views-failed",
    SessionNotSaved => "session-not-saved",
    SessionNotRead => "session-not-read",
    EditFailed => "edit-failed",
    ThumbsFailed => "thumbs-failed",
    BordersFailed => "borders-failed",
    ImportFailed => "import-failed",
    ScanFoldersFailed => "scan-folders-failed",
    UnreadableFiles => "unreadable-files",
    LedgerNotWritten => "ledger-not-written",
    NoticesLost => "notices-lost",
    FileNoLongerInLibrary => "file-no-longer-in-library",
    UpdateAvailable => "update-available",
    Unknown => "unknown";
    // The scans' per-folder facts, now the catalog's folder ledger (`scan-folders-failed`).
    retired: "scan-unreadable", "mount-refused", "root-refused",
}

/// The most bytes of an error's text a notice keeps (its words add the rest).
pub const ERROR_CAP: usize = 2048;

/// An error's text as a notice keeps it: its causes after it (`{:#}`), on one line, capped at [`ERROR_CAP`]. Every notice
/// that carries an error's text uses this, so the rule is one.
pub fn error_text(error: &impl std::fmt::Display) -> String {
    tidy(&format!("{error:#}"), ERROR_CAP)
}

/// The mark a cut text ends in.
const ELLIPSIS: char = '…';

/// `text` as a notice keeps it: each control character (a newline, a tab) becomes a space, so it is one line, and past
/// `cap` bytes it is cut at a character's edge and ends in `…`, the mark within the cap: the result is never more than
/// `cap` bytes (under the mark's 3 bytes, a cut text is the characters that fit, with no mark).
pub fn tidy(text: &str, cap: usize) -> String {
    let mut out = String::with_capacity(text.len().min(cap));
    for c in text.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if out.len() + c.len_utf8() > cap {
            if ELLIPSIS.len_utf8() <= cap {
                // Room for the mark: the characters it would pass the cap with go (an empty text always has it).
                while out.len() + ELLIPSIS.len_utf8() > cap {
                    out.pop();
                }
                out.push(ELLIPSIS);
            }
            break;
        }
        out.push(c);
    }
    out
}

impl Notice {
    /// How much it matters, by one rule. Error: the change or action the user asked for did not take effect (it failed
    /// as a whole, was lost, or was kept aside), or a read the window needs failed. Warning: part of a batch failed and
    /// the rest went through, or the app goes on in a reduced way (no thumbnail cache, a thumbnail not kept, a list left
    /// for the next pass). Info: nothing went wrong, but the user may want to know.
    pub fn level(&self) -> Level {
        match self {
            Notice::ScanNote { note, .. } => {
                if note.is_warning() {
                    Level::Warning
                } else {
                    Level::Info
                }
            }
            Notice::Cropped { failed, .. } | Notice::BordersCut { failed, .. } => {
                if *failed == 0 {
                    Level::Info
                } else {
                    Level::Warning
                }
            }
            Notice::CacheTotalsFixed { .. }
            | Notice::FolderAlreadyAdded { .. }
            | Notice::FileNoLongerInLibrary
            | Notice::UpdateAvailable { .. } => Level::Info,
            Notice::CacheFull { .. }
            | Notice::CacheDisabled { .. }
            | Notice::CatalogRebuilt { .. }
            | Notice::CatalogNotClosed { .. }
            | Notice::PrefsUnreadable { .. }
            | Notice::PrefsNotSaved { .. }
            | Notice::CacheTouchesNotWritten { .. }
            | Notice::CacheNotStored { .. }
            | Notice::CacheWalNotMerged { .. }
            | Notice::CacheStartedOver { .. }
            | Notice::ThumbsFailed { .. }
            | Notice::BordersFailed { .. }
            | Notice::ImportFailed { .. }
            | Notice::ScanFoldersFailed { .. }
            | Notice::UnreadableFiles { .. }
            | Notice::LedgerNotWritten { .. }
            | Notice::NoticesLost { .. }
            | Notice::Unknown { .. } => Level::Warning,
            Notice::ScanFailed { .. }
            | Notice::RoundFailed { .. }
            | Notice::RoundPanicked { .. }
            | Notice::CropFailed { .. }
            | Notice::CutFailed { .. }
            | Notice::WriteFailed { .. }
            | Notice::WriteStopped
            | Notice::WriteDropped { .. }
            | Notice::FolderNotAdded { .. }
            | Notice::FolderRefused { .. }
            | Notice::FolderNotRemoved { .. }
            | Notice::FolderRemovalUnfinished { .. }
            | Notice::PickerFailed { .. }
            | Notice::FoldersUnreadable { .. }
            | Notice::RescanFailed { .. }
            | Notice::LibraryUnavailable { .. }
            | Notice::ChangesUnseen { .. }
            | Notice::ListingFailed { .. }
            | Notice::RowReadFailed { .. }
            | Notice::ReloadFailed { .. }
            | Notice::FacetsFailed { .. }
            | Notice::TreeFailed { .. }
            | Notice::ViewsFailed { .. }
            | Notice::SessionNotSaved { .. }
            | Notice::SessionNotRead { .. }
            | Notice::EditFailed { .. } => Level::Error,
        }
    }

    /// Its category in the inbox.
    pub fn source(&self) -> Source {
        match self {
            Notice::ScanFailed { .. } | Notice::ScanNote { .. } | Notice::ScanFoldersFailed { .. } => Source::Scan,
            Notice::RoundFailed { .. }
            | Notice::RoundPanicked { .. }
            | Notice::RescanFailed { .. }
            | Notice::LibraryUnavailable { .. }
            | Notice::ChangesUnseen { .. } => Source::Library,
            Notice::CacheFull { .. } | Notice::ThumbsFailed { .. } => Source::Thumbs,
            Notice::Cropped { .. }
            | Notice::CropFailed { .. }
            | Notice::BordersCut { .. }
            | Notice::CutFailed { .. } => Source::Crop,
            Notice::CatalogRebuilt { .. }
            | Notice::CatalogNotClosed { .. }
            | Notice::ListingFailed { .. }
            | Notice::RowReadFailed { .. }
            | Notice::ReloadFailed { .. }
            | Notice::FacetsFailed { .. }
            | Notice::TreeFailed { .. }
            | Notice::ViewsFailed { .. }
            | Notice::SessionNotSaved { .. }
            | Notice::SessionNotRead { .. } => Source::Catalog,
            Notice::PrefsUnreadable { .. } | Notice::PrefsNotSaved { .. } => Source::Prefs,
            Notice::CacheTotalsFixed { .. }
            | Notice::CacheTouchesNotWritten { .. }
            | Notice::CacheNotStored { .. }
            | Notice::CacheWalNotMerged { .. }
            | Notice::CacheStartedOver { .. }
            | Notice::CacheDisabled { .. } => Source::Cache,
            Notice::WriteFailed { .. } | Notice::WriteStopped | Notice::WriteDropped { .. } => Source::Writeback,
            Notice::FolderAlreadyAdded { .. }
            | Notice::FolderRefused { .. }
            | Notice::FolderNotAdded { .. }
            | Notice::FolderNotRemoved { .. }
            | Notice::FolderRemovalUnfinished { .. }
            | Notice::FoldersUnreadable { .. }
            | Notice::PickerFailed { .. } => Source::Folders,
            Notice::EditFailed { .. } => Source::Edit,
            Notice::BordersFailed { .. } => Source::Borders,
            Notice::ImportFailed { .. } => Source::Import,
            Notice::LedgerNotWritten { .. } => Source::Thumbs,
            Notice::UnreadableFiles { pass, .. } => match pass {
                FilePass::Thumbs => Source::Thumbs,
                FilePass::Borders => Source::Borders,
                FilePass::Import => Source::Import,
            },
            Notice::UpdateAvailable { .. } => Source::Update,
            Notice::NoticesLost { .. } | Notice::FileNoLongerInLibrary | Notice::Unknown { .. } => Source::App,
        }
    }

    /// How the inbox keeps it.
    pub fn lifecycle(&self) -> Lifecycle<'_> {
        match self {
            Notice::ScanNote { root, note, .. } => match note.lifecycle() {
                NoteLifecycle::Condition => self.key(Some(root), Some(Part::Code(note.key())), Resolver::RootWalk),
                NoteLifecycle::Event => Lifecycle::Event,
            },
            Notice::ViewsFailed { op, .. } => {
                self.key(None, Some(Part::Code(op.key())), Resolver::NextSuccess(Op::Views(*op)))
            }
            Notice::EditFailed { op, .. } => {
                self.key(None, Some(Part::Code(op.key())), Resolver::NextSuccess(Op::Edit(*op)))
            }
            Notice::UnreadableFiles { pass, .. } => {
                self.key(None, Some(Part::Code(pass.key())), Resolver::CompletePass(*pass))
            }
            Notice::LedgerNotWritten { .. } => {
                self.key(None, None, Resolver::NextSuccess(Op::LedgerWrite(FilePass::Thumbs)))
            }
            Notice::RoundFailed { .. } => self.key(None, None, Resolver::NextSuccess(Op::Round)),
            Notice::CacheFull { .. } => self.key(None, None, Resolver::CompletePass(FilePass::Thumbs)),
            Notice::PrefsNotSaved { .. } => self.key(None, None, Resolver::NextSave),
            Notice::CacheTouchesNotWritten { .. } => self.key(None, None, Resolver::TouchesRecovered),
            Notice::CacheNotStored { .. } => self.key(None, None, Resolver::StoreRecovered),
            Notice::WriteFailed { .. } | Notice::WriteStopped => self.key(None, None, Resolver::StatusPoll),
            Notice::ListingFailed { .. } => self.key(None, None, Resolver::NextSuccess(Op::Listing)),
            Notice::RowReadFailed { .. } => self.key(None, None, Resolver::NextSuccess(Op::RowRead)),
            Notice::ReloadFailed { .. } => self.key(None, None, Resolver::NextSuccess(Op::Reload)),
            Notice::FacetsFailed { .. } => self.key(None, None, Resolver::NextSuccess(Op::Facets)),
            Notice::TreeFailed { .. } => self.key(None, None, Resolver::NextSuccess(Op::Tree)),
            // Either is resolved by the next session save that succeeds: it replaces the row that was read or not read.
            Notice::SessionNotSaved { .. } | Notice::SessionNotRead { .. } => {
                self.key(None, None, Resolver::NextSuccess(Op::Session))
            }
            Notice::ThumbsFailed { .. } => Lifecycle::Ledger(LedgerKind::File(FilePass::Thumbs)),
            Notice::BordersFailed { .. } => Lifecycle::Ledger(LedgerKind::File(FilePass::Borders)),
            Notice::ImportFailed { .. } => Lifecycle::Ledger(LedgerKind::File(FilePass::Import)),
            Notice::ScanFoldersFailed { .. } => Lifecycle::Ledger(LedgerKind::ScanFolders),
            Notice::FolderAlreadyAdded { .. } | Notice::FileNoLongerInLibrary | Notice::Unknown { .. } => {
                Lifecycle::Transient
            }
            Notice::WriteDropped { .. }
            | Notice::ScanFailed { .. }
            | Notice::RoundPanicked { .. }
            | Notice::Cropped { .. }
            | Notice::CropFailed { .. }
            | Notice::BordersCut { .. }
            | Notice::CutFailed { .. }
            | Notice::CatalogRebuilt { .. }
            | Notice::CatalogNotClosed { .. }
            | Notice::PrefsUnreadable { .. }
            | Notice::CacheTotalsFixed { .. }
            | Notice::CacheWalNotMerged { .. }
            | Notice::CacheStartedOver { .. }
            | Notice::CacheDisabled { .. }
            | Notice::FolderRefused { .. }
            | Notice::FolderNotAdded { .. }
            | Notice::FolderNotRemoved { .. }
            | Notice::RescanFailed { .. }
            | Notice::LibraryUnavailable { .. }
            | Notice::ChangesUnseen { .. }
            | Notice::FoldersUnreadable { .. }
            | Notice::PickerFailed { .. }
            | Notice::NoticesLost { .. }
            | Notice::FolderRemovalUnfinished { .. }
            | Notice::UpdateAvailable { .. } => Lifecycle::Event,
        }
    }

    /// A kept notice of this kind, under `root` and `part`, cleared by `resolver`.
    fn key<'a>(&self, root: Option<&'a RulePath>, part: Option<Part>, resolver: Resolver) -> Lifecycle<'a> {
        Lifecycle::State(Key { code: self.code(), root, part }, resolver)
    }

    /// What clears it, for a kept notice: its resolver, then [`Resolver::RootRemoval`] when it is about a library
    /// folder. Nothing for a notice that isn't kept.
    pub fn resolvers(&self) -> Vec<Resolver> {
        match self.lifecycle() {
            Lifecycle::State(key, resolver) => {
                let mut resolvers = vec![resolver];
                if key.root.is_some() {
                    resolvers.push(Resolver::RootRemoval);
                }
                resolvers
            }
            Lifecycle::Event | Lifecycle::Transient | Lifecycle::Ledger(_) => Vec::new(),
        }
    }

    /// Every kept kind and part, and what clears it: the table `notice/resolver_table.golden` lists (generated by
    /// [`Notice::resolver_markdown`], and compared by a test). In the samples' order, each part in its list's.
    pub fn resolver_table() -> Vec<ResolverRow> {
        Notice::samples()
            .into_iter()
            .flat_map(each_part)
            .filter_map(|notice| {
                let Lifecycle::State(key, _) = notice.lifecycle() else { return None };
                let part = key.part.map(|Part::Code(code)| code);
                Some(ResolverRow { code: key.code, part, resolvers: notice.resolvers() })
            })
            .collect()
    }

    /// The resolver table as `notice/resolver_table.golden` holds it: a Markdown table, a row per kept kind and part.
    pub fn resolver_markdown() -> String {
        let mut text = String::from("| Kind | Part | Cleared by |\n|---|---|---|\n");
        for row in Notice::resolver_table() {
            let part = row.part.map_or_else(|| "—".to_string(), |part| format!("`{part}`"));
            let resolvers: Vec<String> = row.resolvers.iter().map(|r| format!("`{}`", r.token())).collect();
            text.push_str(&format!("| `{}` | {part} | {} |\n", row.code, resolvers.join(", ")));
        }
        text
    }

    /// What clears the kept kind `code` (a [`Notice::code`]) in `part` (its part's code, if it has one), as the table
    /// has it: the resolvers of the notice the table was made from, so a stored row is cleared by the same rule a
    /// posted notice is. Empty for a kind the table doesn't keep.
    pub fn resolvers_of(code: &str, part: Option<&str>) -> Vec<Resolver> {
        static TABLE: OnceLock<Vec<ResolverRow>> = OnceLock::new();
        let table = TABLE.get_or_init(Notice::resolver_table);
        table
            .iter()
            .find(|row| row.code == code && row.part == part)
            .map(|row| row.resolvers.clone())
            .unwrap_or_default()
    }

    /// The library folder it is about, if any: the inbox ties it to the folder of the library that holds it, so it goes
    /// when that folder is removed, and a notice about a folder no longer in the library is not kept.
    pub fn root(&self) -> Option<&RulePath> {
        match self {
            Notice::ScanFailed { root, .. } | Notice::ScanNote { root, .. } => Some(root),
            Notice::RoundFailed { .. }
            | Notice::RoundPanicked { .. }
            | Notice::CacheFull { .. }
            | Notice::Cropped { .. }
            | Notice::CropFailed { .. }
            | Notice::BordersCut { .. }
            | Notice::CutFailed { .. }
            | Notice::CatalogRebuilt { .. }
            | Notice::CatalogNotClosed { .. }
            | Notice::PrefsUnreadable { .. }
            | Notice::PrefsNotSaved { .. }
            | Notice::CacheTotalsFixed { .. }
            | Notice::CacheTouchesNotWritten { .. }
            | Notice::CacheNotStored { .. }
            | Notice::CacheWalNotMerged { .. }
            | Notice::CacheStartedOver { .. }
            | Notice::CacheDisabled { .. }
            | Notice::WriteFailed { .. }
            | Notice::WriteStopped
            | Notice::WriteDropped { .. }
            // Not yet in the library (an added folder), or no longer (a removed one): not one of its folders.
            | Notice::FolderRemovalUnfinished { .. }
            | Notice::FolderAlreadyAdded { .. }
            | Notice::FolderRefused { .. }
            | Notice::FolderNotAdded { .. }
            | Notice::FolderNotRemoved { .. }
            | Notice::RescanFailed { .. }
            | Notice::LibraryUnavailable { .. }
            | Notice::ChangesUnseen { .. }
            | Notice::FoldersUnreadable { .. }
            | Notice::PickerFailed { .. }
            | Notice::ListingFailed { .. }
            | Notice::RowReadFailed { .. }
            | Notice::ReloadFailed { .. }
            | Notice::FacetsFailed { .. }
            | Notice::TreeFailed { .. }
            | Notice::ViewsFailed { .. }
            | Notice::SessionNotSaved { .. }
            | Notice::SessionNotRead { .. }
            | Notice::EditFailed { .. }
            | Notice::ThumbsFailed { .. }
            | Notice::BordersFailed { .. }
            | Notice::ImportFailed { .. }
            // The folder ledger's entry is about every library folder.
            | Notice::ScanFoldersFailed { .. }
            | Notice::UnreadableFiles { .. }
            | Notice::LedgerNotWritten { .. }
            | Notice::NoticesLost { .. }
            | Notice::FileNoLongerInLibrary
            | Notice::UpdateAvailable { .. }
            | Notice::Unknown { .. } => None,
        }
    }

    /// What the user can do from it, besides dismissing it. A ledger entry's rows also have [`Action::Reveal`] (a
    /// folder's opens its parent), and a toast [`Action::OpenInbox`].
    pub fn actions(&self) -> Vec<Action> {
        match self.lifecycle() {
            Lifecycle::Transient => Vec::new(),
            Lifecycle::Ledger(LedgerKind::File(FilePass::Import)) => vec![Action::ShowFailedFiles, Action::CopyDetails],
            Lifecycle::Ledger(LedgerKind::File(FilePass::Thumbs)) => vec![Action::Retry, Action::CopyDetails],
            Lifecycle::Ledger(LedgerKind::File(FilePass::Borders) | LedgerKind::ScanFolders)
            | Lifecycle::Event
            | Lifecycle::State(..) => vec![Action::CopyDetails],
        }
    }

    /// One notice of each kind, with every optional field set: for the words' tests (every kind is worded) and the
    /// stored form's (every kind reads back).
    pub fn samples() -> Vec<Notice> {
        let root = sample_path(&["Photos"]);
        let failure = FileFailure {
            path: sample_path(&["Photos", "a.jpg"]),
            error: "unexpected end of file".into(),
            panicked: false,
        };
        let error = || "access is denied".to_string();
        vec![
            Notice::ScanFailed { root: root.clone(), error: error() },
            Notice::ScanNote { root: root.clone(), note: ScanNote::Duplicates, count: 2 },
            Notice::RoundFailed { error: error() },
            Notice::RoundPanicked { message: "index out of bounds".into() },
            Notice::CacheFull { height: crate::size::ThumbHeight::DEFAULT, made: 40 },
            Notice::Cropped { folder: r"D:\Crops".into(), saved: 5, skipped: 1, failed: 1, failures: vec![failure] },
            Notice::CropFailed { error: error() },
            Notice::CatalogRebuilt {
                kept: r"D:\Gallery\catalog.sqlite.bad".into(),
                why: "table files: column added".into(),
                carried: Carried::Rows { files: 1200, folders: 2 },
            },
            Notice::CatalogNotClosed { error: error() },
            Notice::PrefsUnreadable {
                path: r"D:\Gallery\gallery.json".into(),
                error: "expected value at line 1".into(),
                kept: Some(r"D:\Gallery\gallery.json.bad".into()),
                keep_error: None,
            },
            Notice::PrefsNotSaved { path: r"D:\Gallery\gallery.json".into(), error: error() },
            Notice::CacheTotalsFixed { kept_bytes: 10, kept_count: 1, bytes: 20, count: 2 },
            Notice::CacheTouchesNotWritten { error: error() },
            Notice::CacheNotStored { error: error() },
            Notice::CacheWalNotMerged { error: error() },
            Notice::CacheStartedOver {
                reason: StartOverReason::Damaged,
                path: r"D:\Gallery\thumbs.sqlite.bad".into(),
                detail: "database disk image is malformed".into(),
            },
            Notice::CacheDisabled { error: error() },
            Notice::WriteFailed {
                error: error(),
                panicked: false,
                file: Some(r"D:\Photos\a.jpg".into()),
                temp: Some(r"D:\Photos\a.jpg.rating.pg-write".into()),
            },
            Notice::WriteStopped,
            Notice::WriteDropped { write: WriteKind::Rating, count: 2 },
            Notice::FolderAlreadyAdded { folder: root.clone() },
            Notice::FolderRefused { folder: root, reason: RefusedReason::NetworkDrive },
            Notice::FolderNotAdded { error: error() },
            Notice::FolderNotRemoved { error: error() },
            Notice::RescanFailed { error: error() },
            Notice::LibraryUnavailable { error: error() },
            Notice::FoldersUnreadable { error: error() },
            Notice::PickerFailed { error: error() },
            Notice::ListingFailed { error: error() },
            Notice::RowReadFailed { error: error() },
            Notice::ReloadFailed { error: error() },
            Notice::FacetsFailed { error: error() },
            Notice::TreeFailed { error: error() },
            Notice::ViewsFailed { op: ViewOp::Save, error: error() },
            Notice::SessionNotSaved { error: error() },
            Notice::EditFailed { op: EditOp::Rate, error: error() },
            Notice::ThumbsFailed { counts: Some(LedgerCounts { new: 1, earlier: 5 }) },
            Notice::BordersFailed { counts: Some(LedgerCounts { new: 0, earlier: 2 }) },
            Notice::ImportFailed { counts: Some(LedgerCounts { new: 3, earlier: 0 }) },
            Notice::ScanFoldersFailed { counts: Some(LedgerCounts { new: 2, earlier: 1 }) },
            Notice::UnreadableFiles { pass: FilePass::Thumbs, count: 7, drives: Vec::new() },
            Notice::LedgerNotWritten { error: error() },
            Notice::NoticesLost { count: 4 },
            Notice::FileNoLongerInLibrary,
            Notice::UpdateAvailable { version: "1.2.0".into() },
            Notice::Unknown { code: "from-a-newer-build".into() },
            Notice::SessionNotRead { error: error() },
            Notice::BordersCut {
                saved: 4,
                skipped: 1,
                unsupported: 1,
                failed: 1,
                failures: vec![FileFailure {
                    path: sample_path(&["Photos", "c.png"]),
                    error: "may be incomplete".into(),
                    panicked: false,
                }],
            },
            Notice::CutFailed { error: error() },
            Notice::FolderRemovalUnfinished {
                folder: sample_path(&["Photos"]),
                temp: sample_path(&["Photos", "a.jpg.pg-write"]),
            },
            Notice::ChangesUnseen { error: error() },
        ]
    }
}

/// `notice` once for each part its kind is kept under (each note, operation or pass), or itself for a kind with none.
fn each_part(notice: Notice) -> Vec<Notice> {
    match notice {
        Notice::ScanNote { root, count, .. } => {
            ScanNote::ALL.iter().map(|&note| Notice::ScanNote { root: root.clone(), note, count }).collect()
        }
        Notice::ViewsFailed { error, .. } => {
            ViewOp::ALL.iter().map(|&op| Notice::ViewsFailed { op, error: error.clone() }).collect()
        }
        Notice::EditFailed { error, .. } => {
            EditOp::ALL.iter().map(|&op| Notice::EditFailed { op, error: error.clone() }).collect()
        }
        Notice::UnreadableFiles { count, drives, .. } => {
            FilePass::ALL.iter().map(|&pass| Notice::UnreadableFiles { pass, count, drives: drives.clone() }).collect()
        }
        other => vec![other],
    }
}

/// `D:\<parts>`, as the samples name a folder.
fn sample_path(parts: &[&str]) -> RulePath {
    RulePath {
        anchor: crate::name::Name::from("D:"),
        parts: parts.iter().map(|p| crate::name::Name::from(*p)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// The variant's place in the enum, by an exhaustive match: a new variant doesn't compile until it has one, and the
    /// samples are checked to hold each.
    fn ordinal(notice: &Notice) -> usize {
        match notice {
            Notice::ScanFailed { .. } => 0,
            Notice::ScanNote { .. } => 1,
            Notice::RoundFailed { .. } => 2,
            Notice::RoundPanicked { .. } => 3,
            Notice::CacheFull { .. } => 4,
            Notice::Cropped { .. } => 5,
            Notice::CropFailed { .. } => 6,
            Notice::CatalogRebuilt { .. } => 7,
            Notice::CatalogNotClosed { .. } => 8,
            Notice::PrefsUnreadable { .. } => 9,
            Notice::PrefsNotSaved { .. } => 10,
            Notice::CacheTotalsFixed { .. } => 11,
            Notice::CacheTouchesNotWritten { .. } => 12,
            Notice::CacheNotStored { .. } => 13,
            Notice::CacheWalNotMerged { .. } => 14,
            Notice::CacheStartedOver { .. } => 15,
            Notice::CacheDisabled { .. } => 16,
            Notice::WriteFailed { .. } => 17,
            Notice::WriteStopped => 18,
            Notice::WriteDropped { .. } => 19,
            Notice::FolderAlreadyAdded { .. } => 20,
            Notice::FolderRefused { .. } => 21,
            Notice::FolderNotAdded { .. } => 22,
            Notice::FolderNotRemoved { .. } => 23,
            Notice::RescanFailed { .. } => 24,
            Notice::LibraryUnavailable { .. } => 25,
            Notice::ChangesUnseen { .. } => 50,
            Notice::FoldersUnreadable { .. } => 26,
            Notice::PickerFailed { .. } => 27,
            Notice::ListingFailed { .. } => 28,
            Notice::RowReadFailed { .. } => 29,
            Notice::ReloadFailed { .. } => 30,
            Notice::FacetsFailed { .. } => 31,
            Notice::TreeFailed { .. } => 32,
            Notice::ViewsFailed { .. } => 33,
            Notice::SessionNotSaved { .. } => 34,
            Notice::EditFailed { .. } => 35,
            Notice::ThumbsFailed { .. } => 36,
            Notice::BordersFailed { .. } => 37,
            Notice::ImportFailed { .. } => 38,
            Notice::ScanFoldersFailed { .. } => 39,
            Notice::UnreadableFiles { .. } => 40,
            Notice::LedgerNotWritten { .. } => 41,
            Notice::NoticesLost { .. } => 42,
            Notice::FileNoLongerInLibrary => 43,
            Notice::UpdateAvailable { .. } => 44,
            Notice::Unknown { .. } => 45,
            Notice::SessionNotRead { .. } => 46,
            Notice::BordersCut { .. } => 47,
            Notice::CutFailed { .. } => 48,
            Notice::FolderRemovalUnfinished { .. } => 49,
        }
    }

    const KINDS: usize = 51;

    #[test]
    fn the_samples_hold_every_kind_once_in_order() {
        let ordinals: Vec<usize> = Notice::samples().iter().map(ordinal).collect();
        assert_eq!(ordinals, (0..KINDS).collect::<Vec<_>>());
    }

    #[test]
    fn the_codes_are_unique_and_name_the_ids() {
        let samples = Notice::samples();
        let codes: BTreeSet<&str> = samples.iter().map(Notice::code).collect();
        assert_eq!(codes.len(), KINDS);
        for notice in &samples {
            assert_eq!(notice.title_id(), format!("msg-notice-{}-title", notice.code()));
            assert_eq!(notice.body_id(), format!("msg-notice-{}-body", notice.code()));
        }
    }

    /// The code is the stored tag: the table above and serde's `rename_all` say the same.
    #[test]
    fn the_code_is_the_stored_kind() {
        for notice in Notice::samples().iter().filter(|n| !matches!(n, Notice::Unknown { .. })) {
            let value = serde_json::to_value(notice).unwrap();
            assert_eq!(value["kind"], notice.code(), "{notice:?}");
        }
    }

    #[test]
    fn every_kind_reads_back_as_it_was_written() {
        for notice in Notice::samples().into_iter().filter(|n| !matches!(n, Notice::Unknown { .. })) {
            let text = serde_json::to_string(&notice).unwrap();
            assert_eq!(serde_json::from_str::<Notice>(&text).unwrap(), notice, "{text}");
        }
    }

    /// An unknown kind is never written: it was read from a newer build, and stays as that build wrote it.
    #[test]
    fn an_unknown_kind_is_not_written() {
        assert!(serde_json::to_string(&Notice::Unknown { code: "x".into() }).is_err());
    }

    /// The stored form of every kind, pinned in `notice/golden.jsonl`: a line per sample, in the samples' order
    /// (`Unknown` is never written). It is what the catalog's rows hold, so a change here is a migration. Each sample
    /// is written as its line and its line reads back as the sample; a kind that differs is named, and the file as this
    /// build writes it is printed, to compare (and, for a change that is meant, to replace the file with).
    #[test]
    fn every_kind_is_stored_as_its_golden_line() {
        // Normalised as the resolver table's test normalises its file: CRLF checkouts compare as LF.
        let golden_text = include_str!("notice/golden.jsonl").replace("\r\n", "\n");
        let written: Vec<Notice> =
            Notice::samples().into_iter().filter(|n| !matches!(n, Notice::Unknown { .. })).collect();
        let actual: Vec<String> = written.iter().map(|n| serde_json::to_string(n).unwrap()).collect();
        let golden: Vec<&str> = golden_text.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut differ = Vec::new();
        for (notice, text) in written.iter().zip(&actual) {
            let line = golden
                .iter()
                .copied()
                .find(|l| serde_json::from_str::<serde_json::Value>(l).is_ok_and(|v| v["kind"] == notice.code()));
            match line {
                None => differ.push(format!("{}: no golden line", notice.code())),
                Some(line) if line != text => {
                    differ.push(format!("{}: written {text}\n    golden  {line}", notice.code()));
                }
                Some(line) => match serde_json::from_str::<Notice>(line) {
                    Ok(read) if read == *notice => {}
                    read => differ.push(format!("{}: its line reads as {read:?}", notice.code())),
                },
            }
        }
        if golden.len() != actual.len() || golden.iter().zip(&actual).any(|(g, a)| g != a) {
            differ.push(format!(
                "the file holds {} lines for {} kinds, or not in the samples' order",
                golden.len(),
                actual.len()
            ));
        }
        assert!(
            differ.is_empty(),
            "the stored form differs:\n{}\n\nnotice/golden.jsonl, as this build writes it:\n{}\n",
            differ.join("\n"),
            actual.join("\n")
        );
    }

    /// The shapes the samples don't hold (an absent count, the other write kind, a reason with a field, a nested
    /// tagged value), pinned too.
    #[test]
    fn the_other_shapes_are_pinned() {
        let golden = [
            (
                Notice::FolderRefused {
                    folder: sample_path(&[]),
                    reason: RefusedReason::NotNtfs { file_system: "FAT32".into() },
                },
                r#"{"kind":"folder-refused","folder":{"anchor":"D:","parts":[]},"reason":{"reason":"not-ntfs","file_system":"FAT32"}}"#,
            ),
            (
                Notice::ScanNote { root: sample_path(&[]), note: ScanNote::VolumeFallback, count: 1 },
                r#"{"kind":"scan-note","root":{"anchor":"D:","parts":[]},"note":"volume-id","count":1}"#,
            ),
            (
                Notice::CatalogRebuilt {
                    kept: "k".into(),
                    why: "w".into(),
                    carried: Carried::Nothing { reason: "r".into() },
                },
                r#"{"kind":"catalog-rebuilt","kept":"k","why":"w","carried":{"carried":"nothing","reason":"r"}}"#,
            ),
            (
                Notice::WriteDropped { write: WriteKind::Rotation, count: 2 },
                r#"{"kind":"write-dropped","write":"rotation","count":2}"#,
            ),
            (Notice::ThumbsFailed { counts: None }, r#"{"kind":"thumbs-failed","counts":null}"#),
            (Notice::ScanFoldersFailed { counts: None }, r#"{"kind":"scan-folders-failed","counts":null}"#),
        ];
        for (notice, text) in golden {
            assert_eq!(serde_json::to_string(&notice).unwrap(), text, "{}", notice.code());
            assert_eq!(serde_json::from_str::<Notice>(text).unwrap(), notice, "{}", notice.code());
        }
    }

    /// A retired code is no kind's, and a row of one doesn't read (the decoder makes it [`Notice::Unknown`]).
    #[test]
    fn a_retired_code_is_no_kind_and_its_rows_do_not_read() {
        let codes: BTreeSet<&str> = Notice::samples().iter().map(Notice::code).collect();
        let retired: BTreeSet<&str> = Notice::RETIRED.iter().copied().collect();
        assert_eq!(retired.len(), Notice::RETIRED.len(), "a code retired twice");
        for code in Notice::RETIRED {
            assert!(!codes.contains(code), "{code} is retired and a kind's");
        }
        let rows = [
            r#"{"kind":"scan-unreadable","root":{"anchor":"D:","parts":["Photos"]},"count":3}"#,
            r#"{"kind":"mount-refused","root":{"anchor":"D:","parts":["Photos"]},"mount":{"anchor":"D:","parts":["Photos","card"]},"reason":{"reason":"fat"}}"#,
            r#"{"kind":"root-refused","root":{"anchor":"D:","parts":["Photos"]},"reason":{"reason":"removable-drive"}}"#,
        ];
        for row in rows {
            assert!(serde_json::from_str::<Notice>(row).is_err(), "{row} still reads");
        }
    }

    /// A row an older build wrote, without the fields added since, still reads: its one `first_error` is the list of one,
    /// and a null one is none. A row with the list reads as it is.
    #[test]
    fn an_older_row_without_a_later_field_reads() {
        let old = r#"{"kind":"cropped","folder":"D:\\Crops","saved":1,"skipped":0,"failed":1,
            "first_error":{"path":{"anchor":"D:","parts":["Photos","a.jpg"]},"error":"bad"}}"#;
        let Notice::Cropped { failures, .. } = serde_json::from_str::<Notice>(old).unwrap() else {
            panic!("not a crop");
        };
        assert_eq!(failures.len(), 1);
        assert!(!failures[0].panicked);
        let none = r#"{"kind":"cropped","folder":"D:\\Crops","saved":1,"skipped":0,"failed":0,"first_error":null}"#;
        let Notice::Cropped { failures, .. } = serde_json::from_str::<Notice>(none).unwrap() else {
            panic!("not a crop");
        };
        assert!(failures.is_empty());
        let list = r#"{"kind":"cropped","folder":"D:\\Crops","saved":0,"skipped":0,"failed":2,
            "failures":[{"path":{"anchor":"D:","parts":["Photos","a.jpg"]},"error":"bad"},{"path":{"anchor":"D:","parts":["Photos","b.jpg"]},"error":"worse","panicked":true}]}"#;
        let Notice::Cropped { failures, .. } = serde_json::from_str::<Notice>(list).unwrap() else {
            panic!("not a crop");
        };
        assert_eq!(failures.len(), 2);
        assert!(failures[1].panicked);
    }

    #[test]
    fn every_kind_has_its_place() {
        for notice in Notice::samples() {
            let lifecycle = notice.lifecycle();
            match (&notice, lifecycle) {
                (
                    Notice::ThumbsFailed { .. }
                    | Notice::BordersFailed { .. }
                    | Notice::ImportFailed { .. }
                    | Notice::ScanFoldersFailed { .. },
                    l,
                ) => {
                    assert!(matches!(l, Lifecycle::Ledger(_)), "{notice:?}");
                }
                (_, Lifecycle::Ledger(_)) => panic!("{notice:?} is no ledger entry"),
                (_, Lifecycle::Transient) => assert!(notice.actions().is_empty(), "{notice:?}"),
                (_, Lifecycle::Event | Lifecycle::State(..)) => assert!(!notice.actions().is_empty(), "{notice:?}"),
            }
        }
    }

    /// Each ledger's entry is its own notice, and its code names it.
    #[test]
    fn each_ledger_has_its_entry() {
        let keys: BTreeSet<&str> = LedgerKind::ALL.iter().map(|k| k.key()).collect();
        assert_eq!(keys.len(), LedgerKind::ALL.len());
        for kind in LedgerKind::ALL {
            assert_eq!(LedgerKind::from_key(kind.key()), Some(kind));
            assert_eq!(kind.notice(None).lifecycle(), Lifecycle::Ledger(kind));
        }
        assert_eq!(LedgerKind::from_key("scan-folders"), Some(LedgerKind::ScanFolders));
        assert_eq!(LedgerKind::from_key("nothing"), None);
    }

    /// A kept notice's key names its kind by its code, and its folder is the notice's own.
    #[test]
    fn a_key_is_the_kinds_code_and_the_notices_folder() {
        for notice in Notice::samples().into_iter().flat_map(each_part) {
            if let Lifecycle::State(key, _) = notice.lifecycle() {
                assert_eq!(key.code, notice.code());
                assert_eq!(key.root, notice.root());
            }
        }
    }

    /// Two kept notices of one kind for one folder are apart by their part, and one part is one key whatever it says.
    #[test]
    fn a_folders_kept_notices_are_keyed_by_their_part() {
        let root = sample_path(&["Photos"]);
        let note = |note, count| Notice::ScanNote { root: root.clone(), note, count };
        assert_ne!(note(ScanNote::Duplicates, 1).lifecycle(), note(ScanNote::SkippedReparse, 1).lifecycle());
        assert_eq!(note(ScanNote::Duplicates, 1).lifecycle(), note(ScanNote::Duplicates, 9).lifecycle());
    }

    /// The scan notes split by what they describe: the folder's conditions are kept under `(root, code)` until the
    /// next walk of the whole folder; what one walk did is an event.
    #[test]
    fn a_scan_note_is_kept_as_a_condition_or_posted_as_an_event() {
        let root = sample_path(&["Photos"]);
        for note in ScanNote::ALL {
            let expected = match note {
                ScanNote::Duplicates
                | ScanNote::SkippedReparse
                | ScanNote::Ambiguous
                | ScanNote::UnpairedWithData
                | ScanNote::VolumeFallback => NoteLifecycle::Condition,
                ScanNote::Replaced | ScanNote::Rebound => NoteLifecycle::Event,
            };
            assert_eq!(note.lifecycle(), expected, "{note:?}");
            let notice = Notice::ScanNote { root: root.clone(), note, count: 2 };
            match (note.lifecycle(), notice.lifecycle()) {
                (NoteLifecycle::Condition, Lifecycle::State(key, Resolver::RootWalk)) => {
                    assert_eq!(key, Key { code: "scan-note", root: Some(&root), part: Some(Part::Code(note.key())) });
                }
                (NoteLifecycle::Event, Lifecycle::Event) => {}
                (_, lifecycle) => panic!("{note:?} is kept as {lifecycle:?}"),
            }
        }
    }

    /// A dropped write is a fact, not a condition: an event, with no key.
    #[test]
    fn a_dropped_write_is_an_event() {
        let dropped = Notice::WriteDropped { write: WriteKind::Rating, count: 1 };
        assert_eq!(dropped.lifecycle(), Lifecycle::Event);
        assert!(dropped.resolvers().is_empty());
    }

    /// The cache's failures are kept, each cleared by the cache's writer at its first success after that failure.
    #[test]
    fn the_caches_failures_are_kept_until_the_cache_recovers() {
        for (notice, resolver) in [
            (Notice::CacheTouchesNotWritten { error: "e".into() }, Resolver::TouchesRecovered),
            (Notice::CacheNotStored { error: "e".into() }, Resolver::StoreRecovered),
        ] {
            assert_eq!(notice.lifecycle(), Lifecycle::State(Key::all(notice.code(), None), resolver), "{notice:?}");
        }
    }

    /// A cache recovery clears its own failure's notice and no other: a touch success does not clear a not-stored
    /// notice, a store success does not clear a touch failure's, and each still clears its own.
    #[test]
    fn each_cache_recovery_clears_only_its_own_failure() {
        let pairs = [
            (Notice::CacheTouchesNotWritten { error: "e".into() }, Trigger::TouchesRecovered),
            (Notice::CacheNotStored { error: "e".into() }, Trigger::StoreRecovered),
        ];
        for (failure, own) in &pairs {
            for (_, recovery) in &pairs {
                let cleared = failure.resolvers().into_iter().any(|r| resolves(r, *recovery));
                assert_eq!(cleared, recovery == own, "{failure:?} is cleared by {recovery:?}");
            }
        }
    }

    /// The table in `notice/resolver_table.golden` is the generated one ([`Notice::resolver_markdown`]), so the doc says
    /// what clears each kept kind as the code does.
    #[test]
    fn the_resolver_table_is_the_golden_one() {
        // Embedded at build time: the shared model reads no files.
        let golden = include_str!("notice/resolver_table.golden").replace("\r\n", "\n");
        let table = Notice::resolver_markdown();
        assert_eq!(golden, table, "notice/resolver_table.golden is not the generated table");
    }

    /// Every kept kind and part is in the resolver table once, with its resolver and, for one about a library folder,
    /// the folder's removal; every resolver clears something.
    #[test]
    fn the_resolver_table_names_every_kept_kind_once() {
        let table = Notice::resolver_table();
        let keys: BTreeSet<(&str, Option<&str>)> = table.iter().map(|row| (row.code, row.part)).collect();
        assert_eq!(keys.len(), table.len(), "a kind and part twice");
        let kept = Notice::samples()
            .into_iter()
            .flat_map(each_part)
            .filter(|n| matches!(n.lifecycle(), Lifecycle::State(..)))
            .count();
        assert_eq!(table.len(), kept);
        for row in &table {
            assert!(!row.resolvers.is_empty(), "{row:?}");
            assert_eq!(row.resolvers.iter().filter(|r| **r != Resolver::RootRemoval).count(), 1, "{row:?}");
        }
        let row = |code, part| table.iter().find(|r| r.code == code && r.part == part).map(|r| r.resolvers.clone());
        assert_eq!(row("scan-note", Some("duplicates")), Some(vec![Resolver::RootWalk, Resolver::RootRemoval]));
        assert_eq!(row("scan-note", Some("replaced")), None);
        assert_eq!(row("write-dropped", None), None);
        assert_eq!(row("cache-touches-not-written", None), Some(vec![Resolver::TouchesRecovered]));
        assert_eq!(row("cache-not-stored", None), Some(vec![Resolver::StoreRecovered]));
        assert_eq!(row("edit-failed", Some("rotate")), Some(vec![Resolver::NextSuccess(Op::Edit(EditOp::Rotate))]));
        assert_eq!(row("unreadable-files", Some("import")), Some(vec![Resolver::CompletePass(FilePass::Import)]));
        // Every resolver is some kept kind's.
        let used: BTreeSet<&str> = table
            .iter()
            .flat_map(|r| r.resolvers.iter())
            .map(|r| match r {
                Resolver::NextSuccess(_) => "next-success",
                Resolver::CompletePass(_) => "complete-pass",
                Resolver::StatusPoll => "status-poll",
                Resolver::TouchesRecovered => "touches-recovered",
                Resolver::StoreRecovered => "store-recovered",
                Resolver::NextSave => "next-save",
                Resolver::RootWalk => "root-walk",
                Resolver::RootRemoval => "root-removal",
            })
            .collect();
        assert_eq!(used.len(), 8, "{used:?}");
        let markdown = Notice::resolver_markdown();
        assert_eq!(markdown.lines().count(), table.len() + 2);
        assert!(markdown.contains("| `scan-note` | `duplicates` | `root-walk`, `root-removal` |"), "{markdown}");
        assert!(markdown.contains("| `prefs-not-saved` | — | `next-save` |"), "{markdown}");
    }

    /// A trigger clears the resolver of its own family and no other: the rule the writer applies to every kept kind.
    #[test]
    fn a_trigger_clears_only_its_own_family() {
        assert!(resolves(Resolver::NextSuccess(Op::Round), Trigger::Op(Op::Round)));
        assert!(!resolves(Resolver::NextSuccess(Op::Session), Trigger::Op(Op::Round)));
        let ledger = Op::LedgerWrite(FilePass::Thumbs);
        assert!(!resolves(Resolver::NextSuccess(ledger), Trigger::Op(Op::LedgerWrite(FilePass::Borders))));
        assert!(resolves(Resolver::CompletePass(FilePass::Thumbs), Trigger::Pass(FilePass::Thumbs)));
        assert!(!resolves(Resolver::CompletePass(FilePass::Thumbs), Trigger::Pass(FilePass::Import)));
        assert!(!resolves(Resolver::CompletePass(FilePass::Thumbs), Trigger::Op(Op::Round)));
        assert!(resolves(Resolver::StatusPoll, Trigger::Status));
        assert!(!resolves(Resolver::StatusPoll, Trigger::Save));
        assert!(resolves(Resolver::TouchesRecovered, Trigger::TouchesRecovered));
        assert!(resolves(Resolver::StoreRecovered, Trigger::StoreRecovered));
        assert!(resolves(Resolver::NextSave, Trigger::Save));
        // The scoped two have no trigger.
        let triggers = [
            Trigger::Status,
            Trigger::TouchesRecovered,
            Trigger::StoreRecovered,
            Trigger::Save,
            Trigger::Pass(FilePass::Thumbs),
        ];
        for trigger in triggers {
            assert!(!resolves(Resolver::RootWalk, trigger) && !resolves(Resolver::RootRemoval, trigger), "{trigger:?}");
        }
    }

    /// The table a stored row is cleared by is the table of the notices posted: [`Notice::resolvers_of`] answers each
    /// kept kind and part as [`Notice::resolvers`] does for the notice.
    #[test]
    fn a_stored_kind_is_cleared_by_what_its_notice_is() {
        for notice in Notice::samples().into_iter().flat_map(each_part) {
            let Lifecycle::State(key, _) = notice.lifecycle() else { continue };
            let part = key.part.map(|Part::Code(code)| code);
            assert_eq!(Notice::resolvers_of(key.code, part), notice.resolvers(), "{notice:?}");
        }
        assert!(Notice::resolvers_of("write-dropped", None).is_empty());
        assert!(Notice::resolvers_of("listing-failed", Some("nope")).is_empty());
    }

    #[test]
    fn every_key_list_is_complete() {
        assert_eq!(ScanNote::ALL.iter().map(|n| n.key()).collect::<BTreeSet<_>>().len(), ScanNote::ALL.len());
        assert_eq!(Source::ALL.iter().map(|s| s.key()).collect::<BTreeSet<_>>().len(), Source::ALL.len());
        for note in ScanNote::ALL {
            assert_eq!(serde_json::to_value(note).unwrap(), note.key());
        }
        for reason in StartOverReason::ALL {
            assert_eq!(serde_json::to_value(reason).unwrap(), reason.key());
        }
        for source in Source::ALL {
            assert_eq!(serde_json::to_value(source).unwrap(), source.key());
        }
        for op in EditOp::ALL {
            assert_eq!(serde_json::to_value(op).unwrap(), op.key());
        }
        for op in ViewOp::ALL {
            assert_eq!(serde_json::to_value(op).unwrap(), op.key());
        }
        for pass in FilePass::ALL {
            assert_eq!(serde_json::to_value(pass).unwrap(), pass.key());
        }
    }

    #[test]
    fn a_tidy_text_is_one_line_and_capped_with_its_mark_inside_the_cap() {
        assert_eq!(tidy("a\nb\tc", 100), "a b c");
        assert_eq!(tidy("abc", 3), "abc");
        assert_eq!(tidy("abcd", 3), "…");
        assert_eq!(tidy("abcdef", 5), "ab…");
        // é is 2 bytes, € 3, the mark 3.
        assert_eq!(tidy("ééé", 6), "ééé");
        assert_eq!(tidy("ééé", 5), "é…");
        assert_eq!(tidy("éééé", 7), "éé…");
        assert_eq!(tidy("éééé", 6), "é…");
        assert_eq!(tidy("a€€", 7), "a€€");
        assert_eq!(tidy("a€€", 6), "a…");
        // Under the mark's bytes, the characters that fit.
        assert_eq!(tidy("ab", 1), "a");
        assert_eq!(tidy("éa", 1), "");
        assert_eq!(tidy("abc", 0), "");
    }

    /// At every cap, a tidy text is at most the cap, and is the text's start (with the mark when cut).
    #[test]
    fn a_tidy_text_is_never_past_its_cap() {
        for text in ["", "abc", "ééé€a", "a€é\nb", "€€€€", "😀a😀"] {
            let whole = tidy(text, usize::MAX);
            for cap in 0..=whole.len() + 1 {
                let out = tidy(text, cap);
                assert!(out.len() <= cap, "{text:?} at {cap}: {out:?}");
                if whole.len() <= cap {
                    assert_eq!(out, whole, "{text:?} at {cap}");
                } else {
                    let start = out.strip_suffix(ELLIPSIS).unwrap_or(&out);
                    assert!(whole.starts_with(start), "{text:?} at {cap}: {out:?}");
                    assert_eq!(out.ends_with(ELLIPSIS), cap >= ELLIPSIS.len_utf8(), "{text:?} at {cap}: {out:?}");
                }
            }
        }
    }
}
