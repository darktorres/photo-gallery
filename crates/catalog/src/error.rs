//! The catalog's errors. Callers branch on a few cases: [`Error::Interrupted`], a statement stopped by the cancel flag
//! (`Catalog::cancel_when`) or by SQLite's own interrupt; [`Error::ScanBusy`], a walk refused because another walks an
//! overlapping folder (`Catalog::begin_scan`); [`Error::RootRemoved`] and [`Error::LeaseLost`], a walk whose lease is
//! gone (its folder removed from the library, or the lease expired); and [`Error::is_busy`], the catalog locked by
//! another connection past the busy timeout. Every other failure is one of the sources the catalog reads or writes
//! through (SQLite, a stored value, a file), or a [`Error::Step`]: the step that failed, with the error under it.

use pg_model::rules::Path as RulePath;

/// A library folder's path as the errors show it, under this platform's path rules.
fn root_text(root: &RulePath) -> String {
    pg_platform::native::PATH_RULES.display(root)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A statement stopped before it finished: the reader gave up on it (its cancel flag was set).
    #[error("interrupted")]
    Interrupted,
    /// A value read from a row that its column can't hold (a `FromSqlError`).
    #[error(transparent)]
    Value(#[from] rusqlite::types::FromSqlError),
    /// A walk of `root` was refused: a walk of a folder that overlaps it (one is inside the other) holds an unexpired
    /// lease (`Catalog::begin_scan`), `holder` says whose. Nothing was written. It can be tried again once that walk ends,
    /// or its lease expires (`SCAN_LEASE_SECS` without a heartbeat).
    #[error("a scan of {} is running ({holder})", root_text(.root))]
    ScanBusy { root: RulePath, holder: String },
    /// A walk of `root` lost its lease: it had no heartbeat for `SCAN_LEASE_SECS` (the walk was held up), so it expired,
    /// and another walk of an overlapping folder may have begun. The write that found it gone wrote nothing: the walk
    /// can no longer tell that it is alone.
    #[error("the lease of the scan of {root} was lost (it expired while the walk was held up)")]
    LeaseLost { root: String },
    /// The folder `root` was removed from the library while a walk of it ran: the removal took its lease
    /// (`Catalog::remove_root`), and the walk's write that found it gone wrote nothing, so nothing of the folder is
    /// written again after its removal.
    #[error("{} was removed from the library while it was scanned", root_text(.root))]
    RootRemoved { root: RulePath },
    /// A library folder overlapping `path` is being removed (its marker is there, `Catalog::begin_removal`): it can't be
    /// added until the removal is finished (or discarded), since its files are still going.
    #[error("{path} is still being removed from the library: try again when it is gone")]
    RemovalPending { path: String },
    /// A file the catalog holds no row for (`id`): nothing was done to it.
    #[error("file {id} isn't in the catalog")]
    NotInCatalog { id: i64 },
    /// A file a write of the app is changing (its temp is being written): its cut is not recorded yet.
    #[error("file {id} is being written: its cut is not recorded")]
    BeingWritten { id: i64 },
    /// A file whose recorded cut is another build's: the catalog's borders are kept by a build that differs from this one.
    #[error("file {id}: its cut is not recorded, the catalog's borders are another build's")]
    OtherBuildsBorders { id: i64 },
    /// The catalog at `path` is held by another process or program, so `what` (preparing it, changing it) cannot go on.
    #[error("catalog {path}: {what}")]
    Held { path: String, what: &'static str },
    /// A migration of the catalog at `path` needs `needed` MB free on its drive (a copy aside and the migration's log), and
    /// `free` MB are.
    #[error(
        "catalog {path}: bringing it to this build's version needs {needed} MB free on its drive (a copy aside and the \
         migration's log), and {free} MB are: free some space and start again"
    )]
    MigrationRoom { path: String, needed: u64, free: u64 },
    /// A library folder whose removal was under way is a library folder again: the removal is not discarded.
    #[error("{path} is a library folder again: its removal is not discarded")]
    RootAgain { path: String },
    /// A migration that drops tables is refused while foreign keys are on: it is refused, and the catalog is unchanged.
    #[error("foreign keys are on for a migration that drops tables: it is refused, and the catalog is unchanged")]
    ForeignKeysOn,
    /// A stored value that does not read as this build's; `what` says which (a folder's path, a queued temp path).
    #[error("{what}")]
    Stored { what: &'static str },
    /// A catalog file that `prepare` refuses, and why (`reason`). Nothing was written to it. The front ends word it by
    /// `reason` (`Error::refusal`); this text is the CLI's.
    #[error("{}", .reason.message(.path))]
    Refused { path: String, reason: Box<Refusal> },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A SQLite call or statement that failed (not an interrupt, which is [`Error::Interrupted`]). Its SQLite error is the
    /// source, so [`Error::is_busy`] can find the busy code under any step.
    #[error("{0}")]
    Sqlite(#[source] rusqlite::Error),
    /// A step of a larger one (opening, migrating, copying a file, a row's write) that failed: its words, and the error
    /// under them.
    #[error("{what}")]
    Step {
        what: String,
        #[source]
        source: Source,
    },
    /// A value the catalog can't use, with the words that say which (a stored path that can't be read, a failed folder's
    /// reason that isn't one of this build's).
    #[error("{0}")]
    Invalid(String),
}

/// The error under a [`Error::Step`]: any error of the catalog's dependencies, boxed.
pub(crate) type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Why `prepare` refuses a catalog file. Typed, so a front end words it itself (`Error::refusal`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A schema after this build's: made by another build. `written_by` is the build that last changed the file, when
    /// the file says (`meta.written_by`, empty when it is not known).
    NewerSchema { found: i64, current: i64, written_by: Option<String> },
    /// A schema of an earlier version than this build's, which [`crate::Catalog::open`] does not bring forward (`prepare` does).
    OlderSchema { found: i64, current: i64 },
    /// An integer mark of the file (`mark`, a `meta` key) above this build's: a later build made or changed the file,
    /// whatever its schema. Refused like a newer schema, so no mark is set back. `written_by` as for `NewerSchema`.
    NewerMark { mark: &'static str, found: i64, current: i64, written_by: Option<String> },
    /// A schema that differs from this build's, of a catalog that may not be rebuilt (a released one). `why` is the
    /// difference.
    Drift { found: i64, why: String },
    /// A schema that differs, of a catalog without the `key_version` mark its rows' keys are known by.
    PredatesKeyVersion,
    /// Another program or connection has the catalog open, so it can't be changed.
    InUse,
    /// The path is too long for the files the catalog makes beside it to open (Windows' limit, which SQLite's own path
    /// handling does not lift). Nothing was made.
    PathTooLong,
    /// The catalog needs a change, and it, or its folder, can't be written (read-only, or permission is denied). Nothing was
    /// changed.
    ReadOnly,
    /// The catalog stores its paths under other rules than this build's: its names and paths don't read as this build's.
    /// It is a valid catalog of another platform, so it is not moved aside. `detail` is how its rules differ (or that
    /// they were not recorded), `wanted` names this build's rules.
    PathRules { detail: String, wanted: &'static str },
}

impl Refusal {
    /// The text of this refusal for the catalog at `path`.
    pub fn message(&self, path: &str) -> String {
        match self {
            Refusal::NewerSchema { found, current, written_by } => {
                let by = written_by
                    .as_deref()
                    .filter(|b| !b.is_empty())
                    .map(|b| format!(" by build {b}"))
                    .unwrap_or_default();
                format!(
                    "catalog {path}: schema version {found}, which this build doesn't know (it has {current}): made by \
                     another build{by}. Open it with that build, or restore a backup by hand (rename it over catalog.sqlite)"
                )
            }
            Refusal::OlderSchema { found, current } => {
                format!("catalog {path}: schema version {found}, not this build's {current}")
            }
            Refusal::NewerMark { mark, found, current, written_by } => {
                let by = written_by
                    .as_deref()
                    .filter(|b| !b.is_empty())
                    .map(|b| format!(" by build {b}"))
                    .unwrap_or_default();
                format!(
                    "catalog {path}: its {mark} is {found}, a later build's (this build's is {current}): made by another \
                     build{by}. Open it with that build, or restore a backup by hand (rename it over catalog.sqlite)"
                )
            }
            Refusal::Drift { found, why } => format!(
                "catalog {path}: its schema (version {found}) differs from this build's ({why}), and a released catalog is \
                 not rebuilt: restore a backup by hand (rename it over catalog.sqlite)"
            ),
            Refusal::PredatesKeyVersion => format!(
                "catalog {path}: its schema differs from this build's and it predates key_version: restore a .pre-v*.bak or \
                 start fresh"
            ),
            Refusal::InUse => format!(
                "catalog {path}: another program or connection has it open, so it can't be changed until it is closed"
            ),
            Refusal::ReadOnly => format!(
                "catalog {path}: it needs a change, and it can't be written (it, or its folder, is read-only or permission is \
                 denied): make it writable, or open a copy of it"
            ),
            Refusal::PathTooLong => format!(
                "catalog {path}: the path is too long for the files the catalog keeps beside it (Windows' limit is 259 \
                 characters): use a shorter folder, such as one set by PG_HOME or --catalog-dir"
            ),
            Refusal::PathRules { detail, wanted } => format!(
                "catalog {path}: {detail}, this build uses the {wanted} path rules: open it on the platform that made it, or \
                 move it aside and scan again"
            ),
        }
    }
}

/// The catalog's result. `E` defaults to [`Error`], so the functions are written `Result<T>`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<rusqlite::Error> for Error {
    /// SQLite's interrupt is [`Error::Interrupted`]; any other statement error is [`Error::Sqlite`].
    fn from(e: rusqlite::Error) -> Error {
        if e.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
            Error::Interrupted
        } else {
            Error::Sqlite(e)
        }
    }
}

impl Error {
    /// The step `what` failed on `source`: an [`Error::Step`] (an interrupt under it stays the interrupt).
    pub(crate) fn step<E>(what: impl Into<String>, source: E) -> Error
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Error::Step { what: what.into(), source: Box::new(source) }
    }

    /// An error of a dependency with no step of its own (an encoder's, a serializer's): the text is the error's, as it was
    /// before the step names were typed.
    pub(crate) fn bare<E>(source: E) -> Error
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        let what = source.to_string();
        Error::Step { what, source: Box::new(source) }
    }

    /// Whether this is SQLite's `SQLITE_BUSY` (another connection holds the lock, past this one's busy timeout),
    /// anywhere in its chain: a statement error is kept under whatever steps were added. It is worth trying again on
    /// the same connection. `SQLITE_LOCKED` (a conflict inside this connection) is not busy: trying it again can loop.
    pub fn is_busy(&self) -> bool {
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(self);
        while let Some(error) = cause {
            if error.downcast_ref::<rusqlite::Error>().and_then(rusqlite::Error::sqlite_error_code)
                == Some(rusqlite::ErrorCode::DatabaseBusy)
            {
                return true;
            }
            cause = error.source();
        }
        false
    }

    /// The catalog file's path and why `prepare` refused it, when this is a refusal.
    pub fn refusal(&self) -> Option<(&str, &Refusal)> {
        match self {
            Error::Refused { path, reason } => Some((path, reason.as_ref())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_is_found_under_any_context_and_locked_is_not_busy() {
        let failure = |code| {
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error { code, extended_code: 0 }, Some("locked".into()))
        };
        let busy: Error = failure(rusqlite::ErrorCode::DatabaseBusy).into();
        assert!(busy.is_busy());
        let wrapped =
            Error::step("the inbox", Error::step("posting", Error::from(failure(rusqlite::ErrorCode::DatabaseBusy))));
        assert!(wrapped.is_busy(), "{wrapped}");
        assert!(!Error::from(failure(rusqlite::ErrorCode::DatabaseLocked)).is_busy());
        assert!(!Error::Interrupted.is_busy());
    }
}
