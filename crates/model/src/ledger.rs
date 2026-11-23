//! The folder and file failures the scans and passes record, as the inbox's expanded entries list them.

use alloc::string::String;

use crate::ids::{FileId, RootId};
use crate::rules::Path as RulePath;

/// A folder of the folder ledger (`folder_failures`): why the walk couldn't take it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderFailure {
    /// It couldn't be opened or listed (`SkipReason::Unreadable`).
    Unreadable,
    /// A mount under the walked folder that the walk refuses (`SkipReason::RefusedMount`).
    MountRefused,
    /// The walked folder itself was refused, before the walk began (`Catalog::folder_refused`).
    Refused,
}

impl FolderFailure {
    /// The stored `folder_failures.reason`.
    pub fn key(self) -> &'static str {
        match self {
            FolderFailure::Unreadable => "unreadable",
            FolderFailure::MountRefused => "mount-refused",
            FolderFailure::Refused => "refused",
        }
    }

    /// The failure of a stored reason; `None` for one this build doesn't know.
    pub fn from_key(key: &str) -> Option<FolderFailure> {
        let all = [FolderFailure::Unreadable, FolderFailure::MountRefused, FolderFailure::Refused];
        all.into_iter().find(|f| f.key() == key)
    }
}

/// A folder of the scans' ledger, as the inbox's expanded entry lists it (the catalog's `failed_folders`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedFolder {
    /// The library folder that holds it.
    pub root: RootId,
    /// The library folder's path.
    pub root_path: RulePath,
    /// The folder, exact.
    pub path: RulePath,
    /// Why the folder failed.
    pub failure: FolderFailure,
    /// The error, as text.
    pub error: String,
    /// When a walk first found it (Unix seconds).
    pub first_seen: i64,
    /// Whether it came after what the user last saw of the entry.
    pub new: bool,
}

/// A failed file, as the inbox's expanded entry lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedFile {
    /// The failed file's id.
    pub file_id: FileId,
    /// The file's path, exact.
    pub path: RulePath,
    /// The error the pass recorded for the file.
    pub error: String,
    /// The pass tries it again by itself; else not until it changes, or the user retries it (the import's: by
    /// `gallery import --retry`).
    pub retry: bool,
    /// When it was first recorded (Unix seconds).
    pub first_seen: i64,
    /// Whether it came after what the user last saw of the entry.
    pub new: bool,
}
