//! The drive's vocabulary: what a drive is, and how it is asked about. A leaf module: the probes of each platform
//! (`facts`, `windows_facts`, `win`, `linux`) import these types from here, and the drive module re-exports them, so
//! no probe needs the drive module itself (which calls the probes).

use std::io;
use std::path::Path;

/// What kind of drive a volume is on. A spinning disk is read by one stream at a time; a flash disk takes many reads
/// in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A spinning disk: it has a seek penalty.
    Rotational,
    /// A flash disk: no seek penalty.
    Solid,
    /// Not known. A USB bridge reports "no seek penalty" for a spinning disk, and a failed query says nothing; both are
    /// read as a spinning disk.
    Unknown,
}

/// The physical disks behind a volume, by identity. Volumes that share a disk share a drive.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DriveKey {
    /// The disks: Windows' disk numbers, and on Linux the whole disk's device number.
    Disks(std::collections::BTreeSet<u64>),
    /// A volume whose disks can't be seen: a virtual disk, Storage Spaces, a RAID controller, iSCSI, or an unknown bus.
    /// Such disks may share one spindle with each other, so every opaque volume is one drive.
    Opaque,
}

/// The drive a path is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    pub key: DriveKey,
    pub kind: Kind,
    /// For people: `E:` on Windows (the volume's GUID path when it has no letter), the disk's name on Linux.
    pub label: String,
}

/// The drive a path is on, as a trait so a caller is tested with a fake.
pub trait Drives {
    /// The drive `path` is on. An error when it can't be told: a network path, a file system with no block device, or a
    /// disk that can't be opened. The caller treats such a drive as unknown.
    fn of(&self, path: &Path) -> io::Result<Drive>;
}
