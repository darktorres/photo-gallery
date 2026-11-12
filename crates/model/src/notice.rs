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
use crate::size::ThumbHeight;
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

    /// How the inbox keeps the note, by what it describes (docs/NOTIFICATIONS.md): a condition of the library folder
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

/// A ledger the inbox shows as one entry, built when the inbox is read: a pass's failed files, or the folders the scans
/// couldn't read (the catalog's per-folder ledger). Its code is what its acknowledgement is stored under
/// (`ledger_ack.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LedgerKind {
    File(FilePass),
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
    pub fn code(self) -> &'static str {
        match self {
            LedgerKind::File(pass) => pass.code(),
            LedgerKind::ScanFolders => "scan-folders",
        }
    }

    /// The ledger with `code` ([`LedgerKind::code`]), if any.
    pub fn from_code(code: &str) -> Option<LedgerKind> {
        LedgerKind::ALL.into_iter().find(|kind| kind.code() == code)
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
    pub new: u64,
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
        root: RulePath,
        error: String,
    },
    /// What a scan of a folder counted that the user may want to know, `count` of them. A condition of the folder is
    /// kept; what one walk did is an event ([`ScanNote::lifecycle`]).
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
        height: ThumbHeight,
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
    /// Queued writes were dropped (their files went), `count` of them, the last of them a `write`: an event (a lost edit
    /// is a fact, not a condition), posted by the writer.
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
    /// The saved session (where the window was) couldn't be read: the window starts from the default listing, and the
    /// stored text stays as it is until the next save replaces it.
    SessionNotRead {
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
    /// Folders the scans couldn't read, or refuse (a mount under a library folder, or the library folder itself): the
    /// catalog's per-folder ledger.
    ScanFoldersFailed {
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
    /// The thumbnail cache's writer, at its first success after a failure.
    CacheRecovery,
    /// The next save of the preferences.
    NextSave,
    /// The next complete walk of the whole library folder, which replaces the folder's kept notices with the ones it
    /// found.
    RootWalk,
    /// The library folder's removal (always, for a state about a library folder).
    RootRemoval,
}

impl Resolver {
    /// The resolver's name, as the generated table in docs/NOTIFICATIONS.md writes it: `next-success(edit-rate)`.
    pub fn name(self) -> String {
        match self {
            Resolver::NextSuccess(op) => format!("next-success({})", op.code()),
            Resolver::CompletePass(pass) => format!("complete-pass({})", pass.code()),
            Resolver::StatusPoll => "status-poll".to_string(),
            Resolver::CacheRecovery => "cache-recovery".to_string(),
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
    RowRead,
    Reload,
    Facets,
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
    pub fn code(self) -> &'static str {
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
    pub code: &'static str,
    pub root: Option<&'a RulePath>,
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
    pub code: &'static str,
    pub root: Option<RulePath>,
    pub part: Option<Part>,
}

impl KeyBuf {
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

/// The ids of each kind, from one table: its code (the stored tag), and its words' ids, `notice-<code>-title` and
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
    ScanFailed => "scan-failed",
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
            Notice::CacheFull { .. }
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
            | Notice::ScanFoldersFailed { .. }
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
            Notice::ScanNote { root, note, .. } => match note.lifecycle() {
                NoteLifecycle::Condition => self.key(Some(root), Some(Part::Code(note.code())), Resolver::RootWalk),
                NoteLifecycle::Event => Lifecycle::Event,
            },
            Notice::ViewsFailed { op, .. } => {
                self.key(None, Some(Part::Code(op.code())), Resolver::NextSuccess(Op::Views(*op)))
            }
            Notice::EditFailed { op, .. } => {
                self.key(None, Some(Part::Code(op.code())), Resolver::NextSuccess(Op::Edit(*op)))
            }
            Notice::UnreadableFiles { pass, .. } => {
                self.key(None, Some(Part::Code(pass.code())), Resolver::CompletePass(*pass))
            }
            Notice::LedgerNotWritten { pass, .. } => {
                self.key(None, Some(Part::Code(pass.code())), Resolver::NextSuccess(Op::LedgerWrite(*pass)))
            }
            Notice::RoundFailed { .. } => self.key(None, None, Resolver::NextSuccess(Op::Round)),
            Notice::CacheFull { .. } => self.key(None, None, Resolver::CompletePass(FilePass::Thumbs)),
            Notice::PrefsNotSaved { .. } => self.key(None, None, Resolver::NextSave),
            Notice::CacheTouchesNotWritten { .. } | Notice::CacheNotStored { .. } => {
                self.key(None, None, Resolver::CacheRecovery)
            }
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

    /// Every kept kind and part, and what clears it: the table docs/NOTIFICATIONS.md lists (generated by
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

    /// The resolver table as docs/NOTIFICATIONS.md holds it: a Markdown table, a row per kept kind and part.
    pub fn resolver_markdown() -> String {
        let mut text = String::from("| Kind | Part | Cleared by |\n|---|---|---|\n");
        for row in Notice::resolver_table() {
            let part = row.part.map_or_else(|| "—".to_string(), |part| format!("`{part}`"));
            let resolvers: Vec<String> = row.resolvers.iter().map(|r| format!("`{}`", r.name())).collect();
            text.push_str(&format!("| `{}` | {part} | {} |\n", row.code, resolvers.join(", ")));
        }
        text
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
        let failure =
            FileFailure { path: r"D:\Photos\a.jpg".into(), error: "unexpected end of file".into(), panicked: false };
        let error = || "access is denied".to_string();
        vec![
            Notice::ScanFailed { root: root.clone(), error: error() },
            Notice::ScanNote { root: root.clone(), note: ScanNote::Duplicates, count: 2 },
            Notice::RoundFailed { error: error() },
            Notice::RoundPanicked { message: "index out of bounds".into() },
            Notice::CacheFull { height: crate::size::ThumbHeight::DEFAULT, made: 40 },
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
            Notice::ScanFoldersFailed { counts: Some(LedgerCounts { new: 2, earlier: 1 }) },
            Notice::UnreadableFiles { pass: FilePass::Thumbs, count: 7 },
            Notice::LedgerNotWritten { pass: FilePass::Borders, error: error() },
            Notice::NoticesLost { count: 4 },
            Notice::FileNoLongerInLibrary,
            Notice::UpdateAvailable { version: "1.2.0".into() },
            Notice::Unknown { code: "from-a-newer-build".into() },
            Notice::SessionNotRead { error: error() },
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
        Notice::UnreadableFiles { count, .. } => {
            FilePass::ALL.iter().map(|&pass| Notice::UnreadableFiles { pass, count }).collect()
        }
        Notice::LedgerNotWritten { error, .. } => {
            FilePass::ALL.iter().map(|&pass| Notice::LedgerNotWritten { pass, error: error.clone() }).collect()
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
        }
    }

    const KINDS: usize = 47;

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

    /// The stored form of every kind, pinned in `notice/golden.jsonl`: a line per sample, in the samples' order
    /// (`Unknown` is never written). It is what the catalog's rows hold, so a change here is a migration. Each sample
    /// is written as its line and its line reads back as the sample; a kind that differs is named, and the file as this
    /// build writes it is printed, to compare (and, for a change that is meant, to replace the file with).
    #[test]
    fn every_kind_is_stored_as_its_golden_line() {
        const GOLDEN: &str = include_str!("notice/golden.jsonl");
        let written: Vec<Notice> =
            Notice::samples().into_iter().filter(|n| !matches!(n, Notice::Unknown { .. })).collect();
        let actual: Vec<String> = written.iter().map(|n| serde_json::to_string(n).unwrap()).collect();
        let golden: Vec<&str> = GOLDEN.lines().filter(|l| !l.trim().is_empty()).collect();
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
                Notice::WriteDropped { write: WriteKind::Turns, count: 2 },
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
        let codes: BTreeSet<&str> = LedgerKind::ALL.iter().map(|k| k.code()).collect();
        assert_eq!(codes.len(), LedgerKind::ALL.len());
        for kind in LedgerKind::ALL {
            assert_eq!(LedgerKind::from_code(kind.code()), Some(kind));
            assert_eq!(kind.notice(None).lifecycle(), Lifecycle::Ledger(kind));
        }
        assert_eq!(LedgerKind::from_code("scan-folders"), Some(LedgerKind::ScanFolders));
        assert_eq!(LedgerKind::from_code("nothing"), None);
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
                    assert_eq!(key, Key { code: "scan-note", root: Some(&root), part: Some(Part::Code(note.code())) });
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

    /// The cache's failures are kept, and cleared by the cache's writer at its first success after them.
    #[test]
    fn the_caches_failures_are_kept_until_the_cache_recovers() {
        for notice in
            [Notice::CacheTouchesNotWritten { error: "e".into() }, Notice::CacheNotStored { error: "e".into() }]
        {
            assert!(
                matches!(
                    notice.lifecycle(),
                    Lifecycle::State(Key { root: None, part: None, .. }, Resolver::CacheRecovery)
                ),
                "{notice:?}"
            );
        }
    }

    /// The table in `docs/NOTIFICATIONS.md` is the generated one ([`Notice::resolver_markdown`]), so the doc says what
    /// clears each kept kind as the code does.
    #[test]
    fn the_documented_resolver_table_is_the_generated_one() {
        // Embedded at build time: the shared model reads no files.
        let doc = include_str!("../../../docs/NOTIFICATIONS.md").replace("\r\n", "\n");
        let table = Notice::resolver_markdown();
        assert!(doc.contains(&table), "docs/NOTIFICATIONS.md's resolver table is not the generated one:\n{table}");
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
        assert_eq!(row("cache-not-stored", None), Some(vec![Resolver::CacheRecovery]));
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
                Resolver::CacheRecovery => "cache-recovery",
                Resolver::NextSave => "next-save",
                Resolver::RootWalk => "root-walk",
                Resolver::RootRemoval => "root-removal",
            })
            .collect();
        assert_eq!(used.len(), 7, "{used:?}");
        let markdown = Notice::resolver_markdown();
        assert_eq!(markdown.lines().count(), table.len() + 2);
        assert!(markdown.contains("| `scan-note` | `duplicates` | `root-walk`, `root-removal` |"), "{markdown}");
        assert!(markdown.contains("| `prefs-not-saved` | — | `next-save` |"), "{markdown}");
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
