//! Why a refusal happens, as data: the scanner refuses a folder (`RefusedReason`, which `pg-scan` decides from the
//! platform's facts), and the catalog refuses a file (`CatalogRefusal`, which `prepare` decides from the file). The
//! words are `pg-wording`'s and the catalog's own, so nothing here is English: a refusal is shown in the reader's
//! language, whichever one the program ran in.

use alloc::string::String;

/// Why `prepare` refuses a catalog file. Typed, so a front end words it itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogRefusal {
    /// A schema after this build's: made by another build. `written_by` is the build that last changed the file, when
    /// the file says (`meta.written_by`, empty when it is not known).
    NewerSchema {
        /// The schema version the file has.
        found: i64,
        /// The version this build expects.
        current: i64,
        /// The build that last changed the file, when the file says.
        written_by: Option<String>,
    },
    /// A schema of an earlier version than this build's, which `Catalog::open` does not bring forward (`prepare` does).
    OlderSchema {
        /// The schema version the file has.
        found: i64,
        /// The version this build expects.
        current: i64,
    },
    /// An integer mark of the file (`mark`, a `meta` key) above this build's: a later build made or changed the file,
    /// whatever its schema. Refused like a newer schema, so no mark is set back. `written_by` as for `NewerSchema`.
    NewerMark {
        /// The mark, a `meta` key.
        mark: &'static str,
        /// The value the file has.
        found: i64,
        /// The value this build expects.
        current: i64,
        /// The build that last changed the file, when the file says.
        written_by: Option<String>,
    },
    /// A schema that differs from this build's, of a catalog that may not be rebuilt (a released one). `why` is the
    /// difference.
    Drift {
        /// The schema version the file has.
        found: i64,
        /// The difference from this build's schema.
        why: String,
    },
    /// A schema that differs, of a catalog without the `key_version` mark its rows' keys are known by.
    PredatesKeyVersion,
    /// Another program or connection has the catalog open, so it can't be changed.
    InUse,
    /// The path is too long for the files the catalog makes beside it to open (Windows' limit, which SQLite's own path
    /// handling does not lift). Nothing was made.
    PathTooLong,
    /// The catalog needs a change, and it, or its folder, can't be written (read-only, or permission is denied). Nothing
    /// was changed.
    ReadOnly,
    /// The catalog stores its paths under other rules than this build's: its names and paths don't read as this build's.
    /// It is a valid catalog of another platform, so it is not moved aside. `detail` is how its rules differ (or that
    /// they were not recorded), `wanted` names this build's rules.
    PathRules {
        /// How the file's rules differ, or that they were not recorded.
        detail: String,
        /// This build's rules, by name.
        wanted: &'static str,
    },
}

/// Why a folder can't be scanned for its identities, or walked into under a scanned root. A notice stores it as
/// `{"reason": "<variant>", ...its fields}`: the variant's name is kebab-case, and its fields keep their Rust names
/// (`file_system`), as a notice's fields do. The words select on [`RefusedReason::key`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum RefusedReason {
    /// A network (UNC) path, refused by its text.
    NetworkPath,
    /// A fact the check needs couldn't be read, so the folder is refused (the checks fail closed): which fact, and the
    /// system's error.
    Unreadable {
        /// The fact that could not be read.
        fact: Fact,
        /// The system's error, as text.
        error: String,
    },
    /// The folder's drive, or the drive it really is on, is a network drive.
    NetworkDrive,
    /// The folder's drive is removable.
    RemovableDrive,
    /// The folder's drive is neither a fixed local disk nor a network or removable drive (an optical drive, a RAM disk).
    OtherDrive,
    /// The folder is a link (a junction or a symbolic link).
    Link,
    /// Where the folder really is, is not on a drive letter (a link resolves to a share).
    RealNotLocal,
    /// Where the folder really is, is on another volume than its path.
    RealOnOtherVolume,
    /// The file system is not NTFS (FAT32 and exFAT have no file IDs that stay put): its name.
    NotNtfs {
        /// The file system's name.
        file_system: String,
    },
    /// The folder's volume is on a disk whose backing can't be seen (Windows): a virtual or file-backed disk (VHD, VHDX),
    /// Storage Spaces, a RAID controller, iSCSI, or an unknown bus. Only NTFS on plain physical disks is supported.
    OpaqueDisk,
    /// An NFS mount (Linux): its inode numbers come from the server.
    Nfs,
    /// An SMB or CIFS share (Linux).
    Smb,
    /// FAT (Linux): the kernel makes its file numbers up at each mount.
    Fat,
    /// exFAT (Linux): the kernel makes its file numbers up at each mount.
    ExFat,
    /// One of the kernel's pseudo file systems, which hold no photos (Linux): its name (`proc`, `sysfs`, ...).
    Pseudo {
        /// The pseudo file system's name.
        file_system: String,
    },
    /// A system whose identities the scanner doesn't check (neither Windows nor Linux).
    UnsupportedSystem,
}

/// A fact the refusal checks read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fact {
    /// The volume the folder's path is on.
    Volume,
    /// The folder itself, opened without following a link.
    Folder,
    /// The path the folder really names.
    RealPath,
    /// The volume of the path the folder really names.
    RealVolume,
    /// The file system's type.
    FileSystem,
    /// The disks the folder's volume is on, and their buses (`pg_platform::drive`).
    Drive,
}

impl RefusedReason {
    /// The reason's stored name (kebab-case, like the notices' tags), which the words select on.
    pub fn key(&self) -> &'static str {
        match self {
            RefusedReason::NetworkPath => "network-path",
            RefusedReason::Unreadable { fact: Fact::Volume, .. } => "unreadable-volume",
            RefusedReason::Unreadable { fact: Fact::Folder, .. } => "unreadable-folder",
            RefusedReason::Unreadable { fact: Fact::RealPath, .. } => "unreadable-real-path",
            RefusedReason::Unreadable { fact: Fact::RealVolume, .. } => "unreadable-real-volume",
            RefusedReason::Unreadable { fact: Fact::FileSystem, .. } => "unreadable-file-system",
            RefusedReason::Unreadable { fact: Fact::Drive, .. } => "unreadable-drive",
            RefusedReason::NetworkDrive => "network-drive",
            RefusedReason::RemovableDrive => "removable-drive",
            RefusedReason::OtherDrive => "other-drive",
            RefusedReason::Link => "link",
            RefusedReason::RealNotLocal => "real-not-local",
            RefusedReason::RealOnOtherVolume => "real-on-other-volume",
            RefusedReason::NotNtfs { .. } => "not-ntfs",
            RefusedReason::OpaqueDisk => "opaque-disk",
            RefusedReason::Nfs => "nfs",
            RefusedReason::Smb => "smb",
            RefusedReason::Fat => "fat",
            RefusedReason::ExFat => "ex-fat",
            RefusedReason::Pseudo { .. } => "pseudo",
            RefusedReason::UnsupportedSystem => "unsupported-system",
        }
    }
}

/// Logs and errors name the reason by its code; its words are the client's.
impl core::fmt::Display for RefusedReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RefusedReason::Unreadable { error, .. } => write!(f, "{} ({error})", self.key()),
            RefusedReason::NotNtfs { file_system } | RefusedReason::Pseudo { file_system } => {
                write!(f, "{} ({file_system})", self.key())
            }
            _ => f.write_str(self.key()),
        }
    }
}
