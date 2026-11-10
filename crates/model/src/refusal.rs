//! Why the scanner refuses a folder (docs/S17_7_PLAN.md, decisions 6 and 13): as data, which `pg-scan` decides from the
//! platform's facts and `pg_client::text` words (MODULARITY_REVIEW_4, R-1 and M-34). Nothing here is English, so a
//! refusal is shown in the reader's language, whichever one the scan ran in.

/// Why a folder can't be scanned for its identities, or walked into under a scanned root. A notice stores it as
/// `{"reason": "<variant>", ...its fields}` (kebab-case); the words select on [`RefusedReason::code`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum RefusedReason {
    /// A network (UNC) path, refused by its text.
    NetworkPath,
    /// A fact the check needs couldn't be read, so the folder is refused (the checks fail closed): which fact, and the
    /// system's error.
    Unreadable { fact: Fact, error: String },
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
    NotNtfs { file_system: String },
    /// An NFS mount (Linux): its inode numbers come from the server.
    Nfs,
    /// An SMB or CIFS share (Linux).
    Smb,
    /// FAT (Linux): the kernel makes its file numbers up at each mount.
    Fat,
    /// exFAT (Linux): the kernel makes its file numbers up at each mount.
    ExFat,
    /// One of the kernel's pseudo file systems, which hold no photos (Linux): its name (`proc`, `sysfs`, ...).
    Pseudo { file_system: String },
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
}

impl RefusedReason {
    /// The reason's stored name (kebab-case, like the notices' tags), which the words select on.
    pub fn code(&self) -> &'static str {
        match self {
            RefusedReason::NetworkPath => "network-path",
            RefusedReason::Unreadable { fact: Fact::Volume, .. } => "unreadable-volume",
            RefusedReason::Unreadable { fact: Fact::Folder, .. } => "unreadable-folder",
            RefusedReason::Unreadable { fact: Fact::RealPath, .. } => "unreadable-real-path",
            RefusedReason::Unreadable { fact: Fact::RealVolume, .. } => "unreadable-real-volume",
            RefusedReason::Unreadable { fact: Fact::FileSystem, .. } => "unreadable-file-system",
            RefusedReason::NetworkDrive => "network-drive",
            RefusedReason::RemovableDrive => "removable-drive",
            RefusedReason::OtherDrive => "other-drive",
            RefusedReason::Link => "link",
            RefusedReason::RealNotLocal => "real-not-local",
            RefusedReason::RealOnOtherVolume => "real-on-other-volume",
            RefusedReason::NotNtfs { .. } => "not-ntfs",
            RefusedReason::Nfs => "nfs",
            RefusedReason::Smb => "smb",
            RefusedReason::Fat => "fat",
            RefusedReason::ExFat => "exfat",
            RefusedReason::Pseudo { .. } => "pseudo",
            RefusedReason::UnsupportedSystem => "unsupported-system",
        }
    }
}

/// Logs and errors name the reason by its code; its words are the client's.
impl std::fmt::Display for RefusedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefusedReason::Unreadable { error, .. } => write!(f, "{} ({error})", self.code()),
            RefusedReason::NotNtfs { file_system } | RefusedReason::Pseudo { file_system } => {
                write!(f, "{} ({file_system})", self.code())
            }
            _ => f.write_str(self.code()),
        }
    }
}
