//! What the app tells the user, as data (docs/NOTIFICATIONS_PLAN.md §1, MODULARITY_REVIEW_4 R-1): a [`Notice`] is built
//! by the crate that has the outcome (the job, the app, the window), stored in the catalog's inbox by its code, recorded
//! in the log, and worded by `pg_client::text` for each front end. Nothing here is English, so a notice is read in the
//! reader's language, whichever one it was posted in; a stored notice is re-worded when the words change.
//!
//! Each kind is one variant: its code (permanent, the stored tag and the words' ids), its level, its source (the inbox's
//! category), its lifecycle (how the inbox keeps it) and its actions. A new kind is one variant here, its words, and the
//! call that posts it (the checklist is in docs/NOTIFICATIONS.md). Paths of the library are [`RulePath`]s, exact; files
//! of the app's own (the preferences, a kept catalog) are the text the system gave.
//!
//! Serialised internally tagged by `kind`, kebab-case. A field added later is `#[serde(default)]`, so an older row still
//! reads. A row of a kind this build doesn't know is [`Notice::Unknown`], which the decoder in `pg-app` makes; it is never
//! written back.

use serde::{Deserialize, Serialize};

use crate::refusal::RefusedReason;
use crate::rules::Path as RulePath;
use crate::write::WriteKind;

/// How much a notice matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// Where a notice comes from: the inbox's category, worded as `source-<code>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Scan,
    Import,
    Borders,
    Thumbs,
    Crop,
    Writeback,
    Prefs,
    Catalog,
    Cache,
    Folders,
    Library,
    Edit,
    Update,
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

    /// The source's code, as the words' ids name it (`source-<code>`).
    pub fn code(self) -> &'static str {
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

/// What a scan counted that the user may want to know (S-17.7 decision 12), by its code (`scan-note`'s selector).
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
    pub fn code(self) -> &'static str {
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
    pub const ALL: [StartOverReason; 4] = [
        StartOverReason::Damaged,
        StartOverReason::NotADatabase,
        StartOverReason::OtherCatalog,
        StartOverReason::OtherBuild,
    ];

    /// The reason's code, which the words select on.
    pub fn code(self) -> &'static str {
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
    Rows { files: u64, folders: u64 },
    /// No rows were copied (the old catalog was not one this build can read, or the copy failed), and why. The new catalog
    /// is empty; the next scan fills it.
    Nothing { reason: String },
}

/// A thumbnail that couldn't follow a rewrite of its file (the cache busy or closing): the file, and the cache's error
/// as its detail text. The thumbnail is made again, and the write is done all the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailLag {
    pub file: crate::ids::FileId,
    pub error: String,
}

/// A file a pass or the writer failed on: its path as the user sees it, and why (the error's text, or a caught panic's
/// message).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFailure {
    pub path: String,
    pub error: String,
    /// The pass panicked on the file (caught): `error` is the panic's message.
    #[serde(default)]
    pub panicked: bool,
}

/// What a failed edit was doing (`Notice::EditFailed`), one inbox entry each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EditOp {
    Rate,
    Rotate,
    MarkBorder,
}

impl EditOp {
    pub const ALL: [EditOp; 3] = [EditOp::Rate, EditOp::Rotate, EditOp::MarkBorder];

    pub fn code(self) -> &'static str {
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
    List,
    Save,
    Delete,
}

impl ViewOp {
    pub const ALL: [ViewOp; 3] = [ViewOp::List, ViewOp::Save, ViewOp::Delete];

    pub fn code(self) -> &'static str {
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
    Thumbs,
    Borders,
    Import,
}

impl FilePass {
    pub const ALL: [FilePass; 3] = [FilePass::Thumbs, FilePass::Borders, FilePass::Import];

    pub fn code(self) -> &'static str {
        match self {
            FilePass::Thumbs => "thumbs",
            FilePass::Borders => "borders",
            FilePass::Import => "import",
        }
    }
}

/// A ledger entry's counts, read only while the inbox is open: the failures since the user last saw the entry, and the
/// ones before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerCounts {
    pub new: u64,
    pub earlier: u64,
}

/// Something the app tells the user. See the module's header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Notice {
    // The scans.
    /// Folders of a scan that couldn't be read.
    ScanUnreadable {
        root: RulePath,
        count: u64,
    },
    /// A scan that failed. Its batches before the error are in.
    ScanFailed {
        root: RulePath,
        error: String,
    },
    /// A folder the scanner refuses (decisions 6 and 13); it stays in the library until it is removed.
    RootRefused {
        root: RulePath,
        reason: RefusedReason,
    },
    /// A folder under a scanned root, on another file system, that the scanner refuses: its files are not in the scan.
    MountRefused {
        root: RulePath,
        mount: RulePath,
        reason: RefusedReason,
    },
    /// What a scan of a folder counted that the user may want to know, `count` of them.
    ScanNote {
        root: RulePath,
        note: ScanNote,
        count: u64,
    },

    // The job.
    /// A round of the job that failed.
    RoundFailed {
        error: String,
    },
    /// A round that panicked (caught): the job went on with the next request.
    RoundPanicked {
        message: String,
    },
    /// The thumbnail cache is full: `made` thumbnails at `height` px weren't made.
    CacheFull {
        height: u32,
        made: u64,
    },
    /// A crop job that ran to the end of its request: the folder its copies went to, and how it went.
    Cropped {
        folder: String,
        saved: u64,
        skipped: u64,
        failed: u64,
        first_error: Option<FileFailure>,
    },
    /// A crop job that failed before it could save anything.
    CropFailed {
        error: String,
    },

    // The app's files.
    /// The catalog didn't match this build's schema, so it was moved aside (to `kept`) and a new one made, which took the
    /// old rows or none (`carried`). `why` is the first difference, as the schema check words it.
    CatalogRebuilt {
        kept: String,
        why: String,
        carried: Carried,
    },
    /// The catalog's last connection couldn't be closed at the end (logged only: the inbox has closed by then).
    CatalogNotClosed {
        error: String,
    },
    /// The preferences file didn't parse: the defaults are in use, and the file was kept aside as `kept`, or couldn't be
    /// (`keep_error`).
    PrefsUnreadable {
        path: String,
        error: String,
        kept: Option<String>,
        keep_error: Option<String>,
    },
    /// The preferences changed in memory but couldn't be saved; the change holds until the app closes.
    PrefsNotSaved {
        path: String,
        error: String,
    },

    // The thumbnail cache.
    /// The cache's totals were off when it was opened, and were fixed: what they were, and what the entries hold.
    CacheTotalsFixed {
        kept_bytes: i64,
        kept_count: i64,
        bytes: i64,
        count: i64,
    },
    /// The times entries were shown couldn't be written; they stay queued for the next try.
    CacheTouchesNotWritten {
        error: String,
    },
    /// An on-demand thumbnail couldn't be stored (it was shown anyway).
    CacheNotStored {
        error: String,
    },
    /// The cache's WAL couldn't be merged when its writer closed.
    CacheWalNotMerged {
        error: String,
    },
    /// The cache's file was started over when it was opened. `path` is where the old file is (a damaged one is moved
    /// aside) or the new file (one this build can't use is deleted); `detail` is the technical account, for the log.
    CacheStartedOver {
        reason: StartOverReason,
        path: String,
        detail: String,
    },
    /// The cache couldn't be opened: thumbnails are made each time they are shown, and none is kept.
    CacheDisabled {
        error: String,
    },

    // The write-back.
    /// The writer's latest write failed: why, and for a copy that may be incomplete, the file and its temporary copy.
    WriteFailed {
        error: String,
        panicked: bool,
        file: Option<String>,
        temp: Option<String>,
    },
    /// The writer stopped: nothing more is written into the files until the app starts again.
    WriteStopped,
    /// Queued writes were dropped (their files went), `count` of them, the last of them a `write`.
    WriteDropped {
        write: WriteKind,
        count: u64,
    },

    // The library's folders.
    /// A folder added that the library already had: it is rescanned.
    FolderAlreadyAdded {
        folder: RulePath,
    },
    /// A folder the scanner refuses, so it wasn't added.
    FolderRefused {
        folder: RulePath,
        reason: RefusedReason,
    },
    FolderNotAdded {
        error: String,
    },
    FolderNotRemoved {
        error: String,
    },
    RescanFailed {
        error: String,
    },
    /// The background work (the job and the writer) couldn't start.
    LibraryUnavailable {
        error: String,
    },
    /// The catalog's list of the library's folders couldn't be read.
    FoldersUnreadable {
        error: String,
    },
    /// A folder couldn't be picked (the system's dialog failed).
    PickerFailed {
        error: String,
    },

    // The window's reads and edits.
    /// The list of files couldn't be read.
    ListingFailed {
        error: String,
    },
    /// A file's row (its details) couldn't be read.
    RowReadFailed {
        error: String,
    },
    /// A file's row couldn't be read again after it changed.
    ReloadFailed {
        error: String,
    },
    /// The filter bar's counts couldn't be read.
    FacetsFailed {
        error: String,
    },
    /// The folder tree couldn't be read.
    TreeFailed {
        error: String,
    },
    /// The saved views couldn't be listed, saved or deleted.
    ViewsFailed {
        op: ViewOp,
        error: String,
    },
    /// The session (where the window was) couldn't be saved.
    SessionNotSaved {
        error: String,
    },
    /// A rating, a turn or a border mark couldn't be saved.
    EditFailed {
        op: EditOp,
        error: String,
    },

    // The failure ledger (built when the inbox is read).
    /// Files whose thumbnail couldn't be made.
    ThumbsFailed {
        counts: Option<LedgerCounts>,
    },
    /// Files whose borders couldn't be looked for.
    BordersFailed {
        counts: Option<LedgerCounts>,
    },
    /// Files that couldn't be imported (their analysis failed).
    ImportFailed {
        counts: Option<LedgerCounts>,
    },
    /// Files a pass couldn't read (an offline drive, a file in use), `count` of them: retried by the next pass.
    UnreadableFiles {
        pass: FilePass,
        count: u64,
    },
    /// A pass's failures couldn't be written to the failure ledger (a catalog locked for long, a full disk): its files
    /// are tried again by its next pass.
    LedgerNotWritten {
        pass: FilePass,
        error: String,
    },

    // The inbox itself, and the app.
    /// Notices that couldn't be kept (the inbox's queue was full, or its writes failed), `count` of them.
    NoticesLost {
        count: u64,
    },
    /// A file a notice named is no longer in the library (shown when it is revealed).
    FileNoLongerInLibrary,
    /// A newer version of the app is out (reserved: nothing checks yet).
    UpdateAvailable {
        version: String,
    },
    /// A stored notice of a kind this build doesn't know, by its code. Made only by the decoder; never written.
    #[serde(skip)]
    Unknown {
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
    /// One entry per pass, built from the failure ledger when the inbox is read; never stored.
    Ledger(FilePass),
    /// Kept under its key: a new post of the same key replaces it (and counts), and its producer clears it once the
    /// problem is gone.
    State(Key<'a>),
}

/// A kept notice's key: its kind's code, the library folder it is about (resolved to the folder of the library that
/// holds it when it is posted, so removing that folder clears it), and the part of that folder's notices it is.
///
/// A producer clears kept notices by a key too ([`Key::all`]): a key with no part clears every part of the kind and
/// folder (a complete scan of a folder clears its refused mounts before it posts the ones it found).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key<'a> {
    pub code: &'static str,
    pub root: Option<&'a RulePath>,
    pub part: Option<Part<'a>>,
}

impl<'a> Key<'a> {
    /// Every kept notice of the kind with `code` (a [`Notice::code`]) about `root`, whatever its part.
    pub fn all(code: &'static str, root: Option<&'a RulePath>) -> Key<'a> {
        Key { code, root, part: None }
    }

    /// The key, owning its folders: to send it to another thread.
    pub fn to_buf(&self) -> KeyBuf {
        KeyBuf {
            code: self.code,
            root: self.root.cloned(),
            part: self.part.as_ref().map(|part| match part {
                Part::Path(path) => PartBuf::Path((*path).clone()),
                Part::Code(code) => PartBuf::Code(code),
            }),
        }
    }
}

/// A [`Key`] that owns its folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBuf {
    pub code: &'static str,
    pub root: Option<RulePath>,
    pub part: Option<PartBuf>,
}

/// A [`Part`] that owns its folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PartBuf {
    Path(RulePath),
    Code(&'static str),
}

impl KeyBuf {
    pub fn key(&self) -> Key<'_> {
        Key {
            code: self.code,
            root: self.root.as_ref(),
            part: self.part.as_ref().map(|part| match part {
                PartBuf::Path(path) => Part::Path(path),
                PartBuf::Code(code) => Part::Code(code),
            }),
        }
    }
}

/// What tells two kept notices of one kind and folder apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part<'a> {
    /// A folder under the root (a refused mount).
    Path(&'a RulePath),
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

/// The ids of each kind, from one table: its code (the stored tag), and its words' ids, `notice-<code>-title` and
/// `-body`. A test checks the code against the serde tag.
macro_rules! ids {
    ($($variant:ident => $code:literal),* $(,)?) => {
        impl Notice {
            /// The kind's code: the stored tag, permanent.
            pub fn code(&self) -> &'static str {
                match self {
                    $(Notice::$variant { .. } => $code,)*
                }
            }

            /// The id of the kind's title, in the words.
            pub fn title_id(&self) -> &'static str {
                match self {
                    $(Notice::$variant { .. } => concat!("notice-", $code, "-title"),)*
                }
            }

            /// The id of the kind's body, in the words.
            pub fn body_id(&self) -> &'static str {
                match self {
                    $(Notice::$variant { .. } => concat!("notice-", $code, "-body"),)*
                }
            }
        }
    };
}

ids! {
    ScanUnreadable => "scan-unreadable",
    ScanFailed => "scan-failed",
    RootRefused => "root-refused",
    MountRefused => "mount-refused",
    ScanNote => "scan-note",
    RoundFailed => "round-failed",
    RoundPanicked => "round-panicked",
    CacheFull => "cache-full",
    Cropped => "cropped",
    CropFailed => "crop-failed",
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
    RescanFailed => "rescan-failed",
    LibraryUnavailable => "library-unavailable",
    FoldersUnreadable => "folders-unreadable",
    PickerFailed => "picker-failed",
    ListingFailed => "listing-failed",
    RowReadFailed => "row-read-failed",
    ReloadFailed => "reload-failed",
    FacetsFailed => "facets-failed",
    TreeFailed => "tree-failed",
    ViewsFailed => "views-failed",
    SessionNotSaved => "session-not-saved",
    EditFailed => "edit-failed",
    ThumbsFailed => "thumbs-failed",
    BordersFailed => "borders-failed",
    ImportFailed => "import-failed",
    UnreadableFiles => "unreadable-files",
    LedgerNotWritten => "ledger-not-written",
    NoticesLost => "notices-lost",
    FileNoLongerInLibrary => "file-no-longer-in-library",
    UpdateAvailable => "update-available",
    Unknown => "unknown",
}

/// The most bytes of an error's text a notice keeps (its words add the rest).
pub const ERROR_CAP: usize = 2048;

/// `text` as a notice keeps it: each control character (a newline, a tab) becomes a space, so it is one line, and past
/// `cap` bytes it is cut at a character's edge and ends in `…`.
pub fn tidy(text: &str, cap: usize) -> String {
    let mut out = String::with_capacity(text.len().min(cap));
    for c in text.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if out.len() + c.len_utf8() > cap {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

impl Notice {
    /// How much it matters.
    pub fn level(&self) -> Level {
        match self {
            Notice::ScanNote { note, .. } => {
                if note.is_warning() {
                    Level::Warning
                } else {
                    Level::Info
                }
            }
            Notice::Cropped { failed, .. } => {
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
            Notice::ScanUnreadable { .. }
            | Notice::RootRefused { .. }
            | Notice::MountRefused { .. }
            | Notice::CacheFull { .. }
            | Notice::CatalogRebuilt { .. }
            | Notice::CatalogNotClosed { .. }
            | Notice::PrefsUnreadable { .. }
            | Notice::PrefsNotSaved { .. }
            | Notice::CacheTouchesNotWritten { .. }
            | Notice::CacheNotStored { .. }
            | Notice::CacheWalNotMerged { .. }
            | Notice::CacheStartedOver { .. }
            | Notice::WriteDropped { .. }
            | Notice::FolderRefused { .. }
            | Notice::FoldersUnreadable { .. }
            | Notice::PickerFailed { .. }
            | Notice::ThumbsFailed { .. }
            | Notice::BordersFailed { .. }
            | Notice::ImportFailed { .. }
            | Notice::UnreadableFiles { .. }
            | Notice::LedgerNotWritten { .. }
            | Notice::NoticesLost { .. }
            | Notice::Unknown { .. } => Level::Warning,
            Notice::ScanFailed { .. }
            | Notice::RoundFailed { .. }
            | Notice::RoundPanicked { .. }
            | Notice::CropFailed { .. }
            | Notice::CacheDisabled { .. }
            | Notice::WriteFailed { .. }
            | Notice::WriteStopped
            | Notice::FolderNotAdded { .. }
            | Notice::FolderNotRemoved { .. }
            | Notice::RescanFailed { .. }
            | Notice::LibraryUnavailable { .. }
            | Notice::ListingFailed { .. }
            | Notice::RowReadFailed { .. }
            | Notice::ReloadFailed { .. }
            | Notice::FacetsFailed { .. }
            | Notice::TreeFailed { .. }
            | Notice::ViewsFailed { .. }
            | Notice::SessionNotSaved { .. }
            | Notice::EditFailed { .. } => Level::Error,
        }
    }

    /// Its category in the inbox.
    pub fn source(&self) -> Source {
        match self {
            Notice::ScanUnreadable { .. }
            | Notice::ScanFailed { .. }
            | Notice::RootRefused { .. }
            | Notice::MountRefused { .. }
            | Notice::ScanNote { .. } => Source::Scan,
            Notice::RoundFailed { .. }
            | Notice::RoundPanicked { .. }
            | Notice::RescanFailed { .. }
            | Notice::LibraryUnavailable { .. } => Source::Library,
            Notice::CacheFull { .. } | Notice::ThumbsFailed { .. } => Source::Thumbs,
            Notice::Cropped { .. } | Notice::CropFailed { .. } => Source::Crop,
            Notice::CatalogRebuilt { .. }
            | Notice::CatalogNotClosed { .. }
            | Notice::ListingFailed { .. }
            | Notice::RowReadFailed { .. }
            | Notice::ReloadFailed { .. }
            | Notice::FacetsFailed { .. }
            | Notice::TreeFailed { .. }
            | Notice::ViewsFailed { .. }
            | Notice::SessionNotSaved { .. } => Source::Catalog,
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
            | Notice::FoldersUnreadable { .. }
            | Notice::PickerFailed { .. } => Source::Folders,
            Notice::EditFailed { .. } => Source::Edit,
            Notice::BordersFailed { .. } => Source::Borders,
            Notice::ImportFailed { .. } => Source::Import,
            Notice::UnreadableFiles { pass, .. } | Notice::LedgerNotWritten { pass, .. } => match pass {
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
            Notice::ScanUnreadable { root, .. } | Notice::RootRefused { root, .. } => self.key(Some(root), None),
            Notice::MountRefused { root, mount, .. } => self.key(Some(root), Some(Part::Path(mount))),
            Notice::ScanNote { root, note, .. } => self.key(Some(root), Some(Part::Code(note.code()))),
            Notice::ViewsFailed { op, .. } => self.key(None, Some(Part::Code(op.code()))),
            Notice::EditFailed { op, .. } => self.key(None, Some(Part::Code(op.code()))),
            Notice::UnreadableFiles { pass, .. } | Notice::LedgerNotWritten { pass, .. } => {
                self.key(None, Some(Part::Code(pass.code())))
            }
            Notice::RoundFailed { .. }
            | Notice::CacheFull { .. }
            | Notice::PrefsNotSaved { .. }
            | Notice::CacheTouchesNotWritten { .. }
            | Notice::CacheNotStored { .. }
            | Notice::WriteFailed { .. }
            | Notice::WriteStopped
            | Notice::WriteDropped { .. }
            | Notice::ListingFailed { .. }
            | Notice::RowReadFailed { .. }
            | Notice::ReloadFailed { .. }
            | Notice::FacetsFailed { .. }
            | Notice::TreeFailed { .. }
            | Notice::SessionNotSaved { .. } => self.key(None, None),
            Notice::ThumbsFailed { .. } => Lifecycle::Ledger(FilePass::Thumbs),
            Notice::BordersFailed { .. } => Lifecycle::Ledger(FilePass::Borders),
            Notice::ImportFailed { .. } => Lifecycle::Ledger(FilePass::Import),
            Notice::FolderAlreadyAdded { .. } | Notice::FileNoLongerInLibrary | Notice::Unknown { .. } => {
                Lifecycle::Transient
            }
            Notice::ScanFailed { .. }
            | Notice::RoundPanicked { .. }
            | Notice::Cropped { .. }
            | Notice::CropFailed { .. }
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
            | Notice::FoldersUnreadable { .. }
            | Notice::PickerFailed { .. }
            | Notice::NoticesLost { .. }
            | Notice::UpdateAvailable { .. } => Lifecycle::Event,
        }
    }

    /// A kept notice of this kind, under `root` and `part`.
    fn key<'a>(&self, root: Option<&'a RulePath>, part: Option<Part<'a>>) -> Lifecycle<'a> {
        Lifecycle::State(Key { code: self.code(), root, part })
    }

    /// The library folder it is about, if any: the inbox ties it to the folder of the library that holds it, so it goes
    /// when that folder is removed, and a notice about a folder no longer in the library is not kept.
    pub fn root(&self) -> Option<&RulePath> {
        match self {
            Notice::ScanUnreadable { root, .. }
            | Notice::ScanFailed { root, .. }
            | Notice::RootRefused { root, .. }
            | Notice::MountRefused { root, .. }
            | Notice::ScanNote { root, .. } => Some(root),
            Notice::RoundFailed { .. }
            | Notice::RoundPanicked { .. }
            | Notice::CacheFull { .. }
            | Notice::Cropped { .. }
            | Notice::CropFailed { .. }
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
            // Not yet in the library (an added folder), so not one of its folders.
            | Notice::FolderAlreadyAdded { .. }
            | Notice::FolderRefused { .. }
            | Notice::FolderNotAdded { .. }
            | Notice::FolderNotRemoved { .. }
            | Notice::RescanFailed { .. }
            | Notice::LibraryUnavailable { .. }
            | Notice::FoldersUnreadable { .. }
            | Notice::PickerFailed { .. }
            | Notice::ListingFailed { .. }
            | Notice::RowReadFailed { .. }
            | Notice::ReloadFailed { .. }
            | Notice::FacetsFailed { .. }
            | Notice::TreeFailed { .. }
            | Notice::ViewsFailed { .. }
            | Notice::SessionNotSaved { .. }
            | Notice::EditFailed { .. }
            | Notice::ThumbsFailed { .. }
            | Notice::BordersFailed { .. }
            | Notice::ImportFailed { .. }
            | Notice::UnreadableFiles { .. }
            | Notice::LedgerNotWritten { .. }
            | Notice::NoticesLost { .. }
            | Notice::FileNoLongerInLibrary
            | Notice::UpdateAvailable { .. }
            | Notice::Unknown { .. } => None,
        }
    }

    /// What the user can do from it, besides dismissing it. A ledger entry's rows also have [`Action::Reveal`], and a toast
    /// [`Action::OpenInbox`].
    pub fn actions(&self) -> Vec<Action> {
        match self.lifecycle() {
            Lifecycle::Transient => Vec::new(),
            Lifecycle::Ledger(FilePass::Import) => vec![Action::ShowFailedFiles, Action::CopyDetails],
            Lifecycle::Ledger(FilePass::Thumbs) => vec![Action::Retry, Action::CopyDetails],
            Lifecycle::Ledger(FilePass::Borders) | Lifecycle::Event | Lifecycle::State(_) => vec![Action::CopyDetails],
        }
    }

    /// One notice of each kind, with every optional field set: for the words' tests (every kind is worded) and the
    /// stored form's (every kind reads back).
    pub fn samples() -> Vec<Notice> {
        let root = sample_path(&["Photos"]);
        let mount = sample_path(&["Photos", "card"]);
        let failure =
            FileFailure { path: r"D:\Photos\a.jpg".into(), error: "unexpected end of file".into(), panicked: false };
        let error = || "access is denied".to_string();
        vec![
            Notice::ScanUnreadable { root: root.clone(), count: 3 },
            Notice::ScanFailed { root: root.clone(), error: error() },
            Notice::RootRefused { root: root.clone(), reason: RefusedReason::RemovableDrive },
            Notice::MountRefused { root: root.clone(), mount, reason: RefusedReason::Fat },
            Notice::ScanNote { root: root.clone(), note: ScanNote::Duplicates, count: 2 },
            Notice::RoundFailed { error: error() },
            Notice::RoundPanicked { message: "index out of bounds".into() },
            Notice::CacheFull { height: 220, made: 40 },
            Notice::Cropped { folder: r"D:\Crops".into(), saved: 5, skipped: 1, failed: 1, first_error: Some(failure) },
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
                temp: Some(r"D:\Photos\a.jpg.pg-tmp".into()),
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
            Notice::UnreadableFiles { pass: FilePass::Thumbs, count: 7 },
            Notice::LedgerNotWritten { pass: FilePass::Borders, error: error() },
            Notice::NoticesLost { count: 4 },
            Notice::FileNoLongerInLibrary,
            Notice::UpdateAvailable { version: "1.2.0".into() },
            Notice::Unknown { code: "from-a-newer-build".into() },
        ]
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
            Notice::ScanUnreadable { .. } => 0,
            Notice::ScanFailed { .. } => 1,
            Notice::RootRefused { .. } => 2,
            Notice::MountRefused { .. } => 3,
            Notice::ScanNote { .. } => 4,
            Notice::RoundFailed { .. } => 5,
            Notice::RoundPanicked { .. } => 6,
            Notice::CacheFull { .. } => 7,
            Notice::Cropped { .. } => 8,
            Notice::CropFailed { .. } => 9,
            Notice::CatalogRebuilt { .. } => 10,
            Notice::CatalogNotClosed { .. } => 11,
            Notice::PrefsUnreadable { .. } => 12,
            Notice::PrefsNotSaved { .. } => 13,
            Notice::CacheTotalsFixed { .. } => 14,
            Notice::CacheTouchesNotWritten { .. } => 15,
            Notice::CacheNotStored { .. } => 16,
            Notice::CacheWalNotMerged { .. } => 17,
            Notice::CacheStartedOver { .. } => 18,
            Notice::CacheDisabled { .. } => 19,
            Notice::WriteFailed { .. } => 20,
            Notice::WriteStopped => 21,
            Notice::WriteDropped { .. } => 22,
            Notice::FolderAlreadyAdded { .. } => 23,
            Notice::FolderRefused { .. } => 24,
            Notice::FolderNotAdded { .. } => 25,
            Notice::FolderNotRemoved { .. } => 26,
            Notice::RescanFailed { .. } => 27,
            Notice::LibraryUnavailable { .. } => 28,
            Notice::FoldersUnreadable { .. } => 29,
            Notice::PickerFailed { .. } => 30,
            Notice::ListingFailed { .. } => 31,
            Notice::RowReadFailed { .. } => 32,
            Notice::ReloadFailed { .. } => 33,
            Notice::FacetsFailed { .. } => 34,
            Notice::TreeFailed { .. } => 35,
            Notice::ViewsFailed { .. } => 36,
            Notice::SessionNotSaved { .. } => 37,
            Notice::EditFailed { .. } => 38,
            Notice::ThumbsFailed { .. } => 39,
            Notice::BordersFailed { .. } => 40,
            Notice::ImportFailed { .. } => 41,
            Notice::UnreadableFiles { .. } => 42,
            Notice::LedgerNotWritten { .. } => 43,
            Notice::NoticesLost { .. } => 44,
            Notice::FileNoLongerInLibrary => 45,
            Notice::UpdateAvailable { .. } => 46,
            Notice::Unknown { .. } => 47,
        }
    }

    const KINDS: usize = 48;

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
            assert_eq!(notice.title_id(), format!("notice-{}-title", notice.code()));
            assert_eq!(notice.body_id(), format!("notice-{}-body", notice.code()));
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

    /// The stored form, pinned for a few kinds that cover each shape (a path, a typed reason, a nested value, a unit
    /// kind, an optional field): it is what the catalog's rows hold, so a change here is a migration.
    #[test]
    fn the_stored_form_is_pinned() {
        let golden = [
            (
                Notice::ScanUnreadable { root: sample_path(&["Photos"]), count: 3 },
                r#"{"kind":"scan-unreadable","root":{"anchor":"D:","parts":["Photos"]},"count":3}"#,
            ),
            (
                Notice::RootRefused {
                    root: sample_path(&[]),
                    reason: RefusedReason::NotNtfs { file_system: "FAT32".into() },
                },
                r#"{"kind":"root-refused","root":{"anchor":"D:","parts":[]},"reason":{"reason":"not-ntfs","file_system":"FAT32"}}"#,
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
            (Notice::WriteStopped, r#"{"kind":"write-stopped"}"#),
            (
                Notice::WriteDropped { write: WriteKind::Turns, count: 2 },
                r#"{"kind":"write-dropped","write":"rotation","count":2}"#,
            ),
            (Notice::ThumbsFailed { counts: None }, r#"{"kind":"thumbs-failed","counts":null}"#),
        ];
        for (notice, text) in golden {
            assert_eq!(serde_json::to_string(&notice).unwrap(), text);
        }
    }

    /// A row an older build wrote, without the fields added since, still reads.
    #[test]
    fn an_older_row_without_a_later_field_reads() {
        let old = r#"{"kind":"cropped","folder":"D:\\Crops","saved":1,"skipped":0,"failed":1,
            "first_error":{"path":"a.jpg","error":"bad"}}"#;
        let Notice::Cropped { first_error: Some(failure), .. } = serde_json::from_str::<Notice>(old).unwrap() else {
            panic!("not a crop");
        };
        assert!(!failure.panicked);
    }

    #[test]
    fn every_kind_has_its_place() {
        for notice in Notice::samples() {
            let lifecycle = notice.lifecycle();
            match (&notice, lifecycle) {
                (Notice::ThumbsFailed { .. } | Notice::BordersFailed { .. } | Notice::ImportFailed { .. }, l) => {
                    assert!(matches!(l, Lifecycle::Ledger(_)), "{notice:?}");
                }
                (_, Lifecycle::Ledger(_)) => panic!("{notice:?} is no ledger entry"),
                (_, Lifecycle::Transient) => assert!(notice.actions().is_empty(), "{notice:?}"),
                (_, Lifecycle::Event | Lifecycle::State(_)) => assert!(!notice.actions().is_empty(), "{notice:?}"),
            }
        }
    }

    /// A kept notice's key names its kind by its code, and its folder is the notice's own.
    #[test]
    fn a_key_is_the_kinds_code_and_the_notices_folder() {
        for notice in Notice::samples() {
            if let Lifecycle::State(key) = notice.lifecycle() {
                assert_eq!(key.code, notice.code());
                assert_eq!(key.root, notice.root());
            }
        }
    }

    /// Two kept notices of one kind for one folder are apart by their part: two refused mounts, two notes.
    #[test]
    fn a_folders_kept_notices_are_keyed_by_their_part() {
        let root = sample_path(&["Photos"]);
        let a = Notice::MountRefused {
            root: root.clone(),
            mount: sample_path(&["Photos", "a"]),
            reason: RefusedReason::Fat,
        };
        let b = Notice::MountRefused {
            root: root.clone(),
            mount: sample_path(&["Photos", "b"]),
            reason: RefusedReason::Fat,
        };
        assert_ne!(a.lifecycle(), b.lifecycle());
        let note = |note| Notice::ScanNote { root: root.clone(), note, count: 1 };
        assert_ne!(note(ScanNote::Duplicates).lifecycle(), note(ScanNote::Rebound).lifecycle());
        assert_eq!(
            note(ScanNote::Duplicates).lifecycle(),
            Notice::ScanNote { root, note: ScanNote::Duplicates, count: 9 }.lifecycle()
        );
    }

    #[test]
    fn every_code_list_is_complete() {
        assert_eq!(ScanNote::ALL.iter().map(|n| n.code()).collect::<BTreeSet<_>>().len(), ScanNote::ALL.len());
        assert_eq!(Source::ALL.iter().map(|s| s.code()).collect::<BTreeSet<_>>().len(), Source::ALL.len());
        for note in ScanNote::ALL {
            assert_eq!(serde_json::to_value(note).unwrap(), note.code());
        }
        for reason in StartOverReason::ALL {
            assert_eq!(serde_json::to_value(reason).unwrap(), reason.code());
        }
        for source in Source::ALL {
            assert_eq!(serde_json::to_value(source).unwrap(), source.code());
        }
        for op in EditOp::ALL {
            assert_eq!(serde_json::to_value(op).unwrap(), op.code());
        }
        for op in ViewOp::ALL {
            assert_eq!(serde_json::to_value(op).unwrap(), op.code());
        }
        for pass in FilePass::ALL {
            assert_eq!(serde_json::to_value(pass).unwrap(), pass.code());
        }
    }

    #[test]
    fn a_tidy_text_is_one_line_and_capped_at_a_character() {
        assert_eq!(tidy("a\nb\tc", 100), "a b c");
        assert_eq!(tidy("ééé", 5), "éé…");
        assert_eq!(tidy("abc", 3), "abc");
    }
}
