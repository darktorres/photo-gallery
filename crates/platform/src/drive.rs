//! The physical drive a path is on: the disks its volume is on, and what kind of drive they are. Two folders on
//! different drives can be read at once; two on one drive are read one after the other. The multi-drive lanes
//! key on [`DriveKey`]; this module answers, for one path, what drive it is on.
//!
//! Windows: the volume's disk extents (`IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`), then each disk's bus type and its
//! seek-penalty answer (`IOCTL_STORAGE_QUERY_PROPERTY`). The handles are opened with no access rights, so no
//! administrator is needed. Linux: the device's sysfs entry, followed through partitions and device-mapper or md
//! slaves to the whole disks, with each disk's `queue/rotational`. Anywhere else the answer is an error.
//!
//! A fact that can't be had is never guessed: a disk whose bus is unknown or hidden makes its volume
//! [`DriveKey::Opaque`], and a disk whose kind can't be told is [`Kind::Unknown`]. The caller treats an unknown kind as
//! a spinning disk, the slower assumption.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

mod types;

pub use types::{Drive, DriveKey, Drives, Kind};

/// The system's drives: Windows' volume and disk queries, or sysfs on Linux.
pub struct SystemDrives;

#[cfg(windows)]
impl Drives for SystemDrives {
    fn of(&self, path: &Path) -> io::Result<Drive> {
        win::of(path)
    }
}

#[cfg(target_os = "linux")]
impl Drives for SystemDrives {
    fn of(&self, path: &Path) -> io::Result<Drive> {
        linux::of(path)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
impl Drives for SystemDrives {
    fn of(&self, _path: &Path) -> io::Result<Drive> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Whether an error from asking a path's drive or volume says the drive is not there (unplugged, not ready, the path
/// gone), rather than that it can't be told. A root on such a drive has no lane until the drive is back: its files wait,
/// and nothing else waits for them.
pub fn unplugged(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound || unplugged_code(error.raw_os_error())
}

/// The OS error codes that say a drive is not there: Windows' `ERROR_NOT_READY`, and on Linux `ENODEV` and `ENXIO`.
#[cfg(windows)]
fn unplugged_code(code: Option<i32>) -> bool {
    code == Some(21)
}

#[cfg(target_os = "linux")]
fn unplugged_code(code: Option<i32>) -> bool {
    matches!(code, Some(19 | 6))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn unplugged_code(_code: Option<i32>) -> bool {
    false
}

/// Which keys are the same drive: for each key, the index of the first key of its drive. Two keys are one drive when
/// their disks overlap, transitively (a volume spanning disks 1 and 2 and one on disk 2 are one drive). Every opaque key
/// is one drive with the others.
pub fn same_drive(keys: &[DriveKey]) -> Vec<usize> {
    let mut parent = BTreeMap::new();
    for key in keys {
        let disks = key_disks(key);
        if let Some((&first, rest)) = disks.split_first() {
            for &disk in rest {
                union(&mut parent, first, disk);
            }
        }
    }
    let mut first_of_root: BTreeMap<u64, usize> = BTreeMap::new();
    keys.iter()
        .enumerate()
        .map(|(i, key)| match key_disks(key).first() {
            Some(&disk) => {
                let root = find(&mut parent, disk);
                *first_of_root.entry(root).or_insert(i)
            }
            None => i,
        })
        .collect()
}

/// The disk identities a key stands for. Every opaque key stands for the same one, so they are one drive.
fn key_disks(key: &DriveKey) -> Vec<u64> {
    match key {
        DriveKey::Disks(disks) => disks.iter().copied().collect(),
        DriveKey::Opaque => vec![OPAQUE_DISK],
    }
}

/// The disk identity of every opaque volume.
const OPAQUE_DISK: u64 = u64::MAX;

fn find(parent: &mut BTreeMap<u64, u64>, x: u64) -> u64 {
    let p = *parent.entry(x).or_insert(x);
    if p == x {
        return x;
    }
    let root = find(parent, p);
    parent.insert(x, root);
    root
}

fn union(parent: &mut BTreeMap<u64, u64>, a: u64, b: u64) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent.insert(rb, ra);
    }
}

/// What a platform's probe says about one physical disk, and how its volume is put together from them.
#[cfg(any(windows, target_os = "linux", test))]
mod facts {
    use super::types::{Drive, DriveKey, Kind};
    use std::collections::BTreeSet;
    use std::io;

    /// One physical disk, as its volume sees it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct DiskFact {
        /// The disk's identity (see [`DriveKey::Disks`]).
        pub(super) disk: u64,
        /// Its backing can't be seen (see [`DriveKey::Opaque`]).
        pub(super) opaque: bool,
        pub(super) kind: Kind,
    }

    /// The drive a volume is on, from the disks it is on. A disk that can't be seen makes the volume opaque. Otherwise
    /// the kind is the spinningest of its disks, so a volume with one spinning disk is read as one.
    pub(super) fn drive_of_disks(label: String, disks: &[DiskFact]) -> io::Result<Drive> {
        if disks.is_empty() {
            return Err(io::Error::other(format!("{label}: no disk is behind the volume")));
        }
        if disks.iter().any(|disk| disk.opaque) {
            return Ok(Drive { key: DriveKey::Opaque, kind: Kind::Unknown, label });
        }
        let kind = if disks.iter().any(|disk| disk.kind == Kind::Rotational) {
            Kind::Rotational
        } else if disks.iter().any(|disk| disk.kind == Kind::Unknown) {
            Kind::Unknown
        } else {
            Kind::Solid
        };
        let key = DriveKey::Disks(disks.iter().map(|disk| disk.disk).collect::<BTreeSet<u64>>());
        Ok(Drive { key, kind, label })
    }
}

/// Windows' facts as the bytes its calls return: the disk extents, a device descriptor's bus type and a seek-penalty
/// descriptor. The decisions are here, pure, so they are tested on any platform.
#[cfg(any(windows, test))]
mod windows_facts {
    use super::facts::DiskFact;
    use super::types::Kind;
    use std::io;

    // `STORAGE_BUS_TYPE` values (`windows-sys`'s `BusType*`; a test checks them against it).
    pub(super) const BUS_UNKNOWN: i32 = 0;
    #[cfg(test)]
    pub(super) const BUS_SATA: i32 = 11;
    pub(super) const BUS_USB: i32 = 7;
    pub(super) const BUS_RAID: i32 = 8;
    pub(super) const BUS_ISCSI: i32 = 9;
    pub(super) const BUS_VIRTUAL: i32 = 14;
    pub(super) const BUS_FILE_BACKED_VIRTUAL: i32 = 15;
    pub(super) const BUS_SPACES: i32 = 16;

    /// The buses whose disks can't be seen: a virtual disk is a file or part of another disk, Storage Spaces and a RAID
    /// controller span several, iSCSI is a network disk, and an unknown bus says nothing.
    const OPAQUE_BUSES: [i32; 6] = [BUS_UNKNOWN, BUS_RAID, BUS_ISCSI, BUS_VIRTUAL, BUS_FILE_BACKED_VIRTUAL, BUS_SPACES];

    /// The fact about disk `disk`, from its bus type and its seek-penalty answer (each `None` when the query failed).
    pub(super) fn disk_fact(disk: u64, bus: Option<i32>, seek_penalty: Option<bool>) -> DiskFact {
        match bus {
            Some(bus) if !OPAQUE_BUSES.contains(&bus) => {
                let kind = match seek_penalty {
                    Some(true) => Kind::Rotational,
                    // A USB bridge reports "no seek penalty" for a spinning disk, so that answer makes no flash disk.
                    Some(false) if bus != BUS_USB => Kind::Solid,
                    _ => Kind::Unknown,
                };
                DiskFact { disk, opaque: false, kind }
            }
            _ => DiskFact { disk, opaque: true, kind: Kind::Unknown },
        }
    }

    /// A little-endian `u32` at `at`, or `None` when the buffer is too short.
    fn le_u32(buf: &[u8], at: usize) -> Option<u32> {
        let end = at.checked_add(4)?;
        let bytes: [u8; 4] = buf.get(at..end)?.try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    /// The disk numbers of a `VOLUME_DISK_EXTENTS`: the count, then the extents from offset 8, 24 bytes each, whose first
    /// field is the disk number.
    pub(super) fn parse_extents(buf: &[u8]) -> io::Result<Vec<u32>> {
        let short = || io::Error::other("the volume's disk extents are cut short");
        let count = le_u32(buf, 0).ok_or_else(short)?;
        let count = usize::try_from(count).map_err(|_| short())?;
        (0..count).map(|i| le_u32(buf, 8usize.saturating_add(24usize.saturating_mul(i))).ok_or_else(short)).collect()
    }

    /// The bus type of a `STORAGE_DEVICE_DESCRIPTOR` (its `BusType` field, at offset 28).
    pub(super) fn parse_bus(buf: &[u8]) -> Option<i32> {
        i32::try_from(le_u32(buf, 28)?).ok()
    }

    /// The answer of a `DEVICE_SEEK_PENALTY_DESCRIPTOR` (`IncursSeekPenalty`, a byte at offset 8).
    pub(super) fn parse_seek_penalty(buf: &[u8]) -> Option<bool> {
        buf.get(8).map(|&byte| byte != 0)
    }
}

#[cfg(windows)]
mod win {
    use super::facts::{DiskFact, drive_of_disks};
    use super::types::Drive;
    use super::windows_facts as wf;
    use crate::dir::{RootFacts, SystemFacts};
    use crate::native::Unit;
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::Path;
    use std::ptr;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetVolumeNameForVolumeMountPointW,
        IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, StorageDeviceProperty, StorageDeviceSeekPenaltyProperty,
    };

    /// `ERROR_MORE_DATA`: the output buffer was too small for the answer.
    const ERROR_MORE_DATA: i32 = 234;

    pub(super) fn of(path: &Path) -> io::Result<Drive> {
        let units: Vec<Unit> = path.as_os_str().encode_wide().collect();
        // The volume root with its separator, as `GetVolumePathNameW` gives it (`E:\`).
        let root = SystemFacts.volume_root(&units)?;
        let guid = volume_guid(&root)?;
        // The volume by its GUID path, which the drive letter and a folder mount both resolve to.
        let volume = open(guid.trim_end_matches('\\'))?;
        let mut disks = Vec::new();
        for number in disk_numbers(&volume)? {
            disks.push(fact_of_disk(number)?);
        }
        let text = String::from_utf16_lossy(&root);
        let label =
            if text.len() == 3 && text.ends_with(":\\") { text.trim_end_matches('\\').to_string() } else { guid };
        drive_of_disks(label, &disks)
    }

    /// The volume GUID path of the volume `root` is the root of (`\\?\Volume{…}\`).
    fn volume_guid(root: &[Unit]) -> io::Result<String> {
        let mut wide: Vec<u16> = root.to_vec();
        wide.push(0);
        let mut name = vec![0u16; 64];
        let len = u32::try_from(name.len()).map_err(io::Error::other)?;
        // SAFETY: `wide` is NUL-terminated and outlives the call; `name` is writable for `len` units.
        if unsafe { GetVolumeNameForVolumeMountPointW(wide.as_ptr(), name.as_mut_ptr(), len) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        Ok(String::from_utf16_lossy(name.get(..end).unwrap_or_default()))
    }

    /// `path` opened with no access rights: enough for the queries below, and no administrator right is needed.
    fn open(path: &str) -> io::Result<OwnedHandle> {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; the security attributes and template are null, which
        // the call allows.
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a valid handle from `CreateFileW`, and this is its only owner.
        Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
    }

    /// One `DeviceIoControl`: `input`, if any, is sent, and the answer is written to `out`. Returns the bytes written.
    fn control(handle: &OwnedHandle, code: u32, input: Option<&[u8]>, out: &mut [u8]) -> io::Result<usize> {
        let (input_ptr, input_len): (*const c_void, u32) = match input {
            Some(bytes) => (bytes.as_ptr().cast(), u32::try_from(bytes.len()).map_err(io::Error::other)?),
            None => (ptr::null(), 0),
        };
        let out_len = u32::try_from(out.len()).map_err(io::Error::other)?;
        let mut returned = 0u32;
        // SAFETY: `input` is null or valid for `input_len` bytes, and `out` is valid for `out_len` bytes; no overlapped
        // I/O is requested, so no OVERLAPPED is needed.
        let ok = unsafe {
            DeviceIoControl(
                handle.as_raw_handle(),
                code,
                input_ptr,
                input_len,
                out.as_mut_ptr().cast(),
                out_len,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(usize::try_from(returned).unwrap_or(0))
    }

    /// The disk numbers of the volume `volume` is (its extents), with a bigger buffer when the first is too small.
    fn disk_numbers(volume: &OwnedHandle) -> io::Result<Vec<u32>> {
        for size in [1024usize, 64 * 1024] {
            let mut buf = vec![0u8; size];
            match control(volume, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, None, &mut buf) {
                Ok(n) => return wf::parse_extents(buf.get(..n).unwrap_or_default()),
                Err(e) if e.raw_os_error() == Some(ERROR_MORE_DATA) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("the volume has more disks than the probe's buffer holds"))
    }

    /// The fact about physical disk `number`. The disk can't be opened: an error, so the volume is unknown. A failed
    /// bus or seek-penalty query is not an error: the disk's kind is unknown, or the disk is opaque.
    fn fact_of_disk(number: u32) -> io::Result<DiskFact> {
        let disk = open(&format!(r"\\.\PhysicalDrive{number}"))?;
        let mut bus_buf = vec![0u8; 1024];
        let bus_query = query(StorageDeviceProperty);
        let bus = control(&disk, IOCTL_STORAGE_QUERY_PROPERTY, Some(bus_query.as_slice()), &mut bus_buf)
            .ok()
            .and_then(|n| wf::parse_bus(bus_buf.get(..n).unwrap_or_default()));
        let mut penalty_buf = vec![0u8; 64];
        let penalty_query = query(StorageDeviceSeekPenaltyProperty);
        let penalty = control(&disk, IOCTL_STORAGE_QUERY_PROPERTY, Some(penalty_query.as_slice()), &mut penalty_buf)
            .ok()
            .and_then(|n| wf::parse_seek_penalty(penalty_buf.get(..n).unwrap_or_default()));
        Ok(wf::disk_fact(u64::from(number), bus, penalty))
    }

    /// A `STORAGE_PROPERTY_QUERY` for `property`, as a standard query: the property id, the query type, and one byte.
    fn query(property: i32) -> [u8; 12] {
        let [p0, p1, p2, p3] = property.to_le_bytes();
        let [q0, q1, q2, q3] = PropertyStandardQuery.to_le_bytes();
        [p0, p1, p2, p3, q0, q1, q2, q3, 0, 0, 0, 0]
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::facts::{DiskFact, drive_of_disks};
    use super::types::{Drive, Kind};
    use std::fs;
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    /// The sysfs root the system's devices are looked up under.
    const SYSFS: &str = "/sys";

    /// How deep the device stack may nest (a dm device over a dm device over a disk, and so on).
    const MAX_DEPTH: u8 = 8;

    pub(super) fn of(path: &Path) -> io::Result<Drive> {
        let dev = fs::metadata(path)?.dev();
        drive_of_dev(Path::new(SYSFS), dev)
    }

    /// The drive of the block device `dev` (a `dev_t`), found under `sysfs`.
    pub(super) fn drive_of_dev(sysfs: &Path, dev: u64) -> io::Result<Drive> {
        if major(dev) == 0 {
            // Btrfs, overlayfs, ZFS and the like report no block device of their own.
            return Err(io::Error::new(io::ErrorKind::Unsupported, "the file system has no block device"));
        }
        let top = sysfs.join("dev").join("block").join(format!("{}:{}", major(dev), minor(dev)));
        let mut disks = Vec::new();
        facts_in(&top, 0, &mut disks)?;
        let label = fs::canonicalize(&top)
            .ok()
            .and_then(|real| real.file_name().map(|name| name.to_string_lossy().into_owned()))
            .unwrap_or_else(|| format!("{}:{}", major(dev), minor(dev)));
        drive_of_disks(label, &disks)
    }

    /// The whole disks under the block device at `dir`: a partition's disk, each slave of a device-mapper or md device,
    /// or the device itself, when it is a disk. A virtual device with no slaves is opaque.
    fn facts_in(dir: &Path, depth: u8, out: &mut Vec<DiskFact>) -> io::Result<()> {
        if depth > MAX_DEPTH {
            return Err(io::Error::other("the device stack is too deep"));
        }
        let real = fs::canonicalize(dir)?;
        let slaves = real.join("slaves");
        if slaves.is_dir() {
            let mut any = false;
            for entry in fs::read_dir(&slaves)? {
                any = true;
                facts_in(&entry?.path(), depth + 1, out)?;
            }
            if any {
                return Ok(());
            }
        }
        let dev = read_dev(&real)?;
        let text = real.to_string_lossy();
        if text.contains("/devices/virtual/") {
            out.push(DiskFact { disk: dev, opaque: true, kind: Kind::Unknown });
            return Ok(());
        }
        if real.join("partition").exists() {
            let parent = real.parent().ok_or_else(|| io::Error::other("a partition with no disk"))?;
            return facts_in(parent, depth + 1, out);
        }
        let rotational = fs::read_to_string(real.join("queue").join("rotational")).ok();
        // A USB bridge reports no rotation for a spinning disk, so only "rotational" is trusted behind one.
        let usb = text.contains("/usb");
        out.push(DiskFact { disk: dev, opaque: false, kind: kind_of(rotational.as_deref(), usb) });
        Ok(())
    }

    /// The kind of a whole disk from its `queue/rotational` text.
    fn kind_of(rotational: Option<&str>, usb: bool) -> Kind {
        match rotational.map(str::trim) {
            Some("1") => Kind::Rotational,
            Some("0") if !usb => Kind::Solid,
            _ => Kind::Unknown,
        }
    }

    /// The device number of the device whose sysfs directory is `dir` (its `dev` file, `major:minor`).
    fn read_dev(dir: &Path) -> io::Result<u64> {
        let text = fs::read_to_string(dir.join("dev"))?;
        let (maj, min) = text.trim().split_once(':').ok_or_else(|| io::Error::other("a dev file with no colon"))?;
        let maj: u64 = maj.parse().map_err(io::Error::other)?;
        let min: u64 = min.parse().map_err(io::Error::other)?;
        Ok(makedev(maj, min))
    }

    /// `major` of a `dev_t`, the glibc encoding `st_dev` uses.
    fn major(dev: u64) -> u64 {
        ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff)
    }

    /// `minor` of a `dev_t`, the glibc encoding `st_dev` uses.
    fn minor(dev: u64) -> u64 {
        (dev & 0xff) | ((dev >> 12) & !0xff)
    }

    /// A `dev_t` from its major and minor numbers, the glibc encoding.
    fn makedev(major: u64, minor: u64) -> u64 {
        ((major & 0xfff) << 8) | (minor & 0xff) | ((minor & !0xff) << 12) | ((major & !0xfff) << 32)
    }

    #[cfg(test)]
    mod tests {
        use super::super::{DriveKey, Drives, SystemDrives};
        use super::*;

        /// A fake sysfs: a disk `sda` with a partition `sda1`, a device-mapper device over the partition, a loop device,
        /// and a USB disk; then a solid disk `sdc` and a spinning disk `sdd`, and a device-mapper device `dm-1` over both.
        /// Its `dev/block` links point at each, as sysfs has them.
        fn sysfs() -> std::path::PathBuf {
            let root = crate::testing::run_folder("scratch_drive", "pg-drive-sysfs").unwrap();
            let write = |rel: &str, text: &str| {
                let path = root.join(rel);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            };
            write("devices/pci/block/sda/dev", "8:0\n");
            write("devices/pci/block/sda/queue/rotational", "1\n");
            write("devices/pci/block/sda/sda1/dev", "8:1\n");
            write("devices/pci/block/sda/sda1/partition", "1\n");
            write("devices/virtual/block/dm-0/dev", "253:0\n");
            fs::create_dir_all(root.join("devices/virtual/block/dm-0/slaves")).unwrap();
            std::os::unix::fs::symlink(
                root.join("devices/pci/block/sda/sda1"),
                root.join("devices/virtual/block/dm-0/slaves/sda1"),
            )
            .unwrap();
            write("devices/virtual/block/loop0/dev", "7:0\n");
            write("devices/pci/usb1/1-1/block/sdb/dev", "8:16\n");
            write("devices/pci/usb1/1-1/block/sdb/queue/rotational", "0\n");
            write("devices/pci/block/sdc/dev", "8:32\n");
            write("devices/pci/block/sdc/queue/rotational", "0\n");
            write("devices/pci/block/sdd/dev", "8:48\n");
            write("devices/pci/block/sdd/queue/rotational", "1\n");
            write("devices/virtual/block/dm-1/dev", "253:1\n");
            fs::create_dir_all(root.join("devices/virtual/block/dm-1/slaves")).unwrap();
            for disk in ["sdc", "sdd"] {
                std::os::unix::fs::symlink(
                    root.join("devices/pci/block").join(disk),
                    root.join("devices/virtual/block/dm-1/slaves").join(disk),
                )
                .unwrap();
            }
            fs::create_dir_all(root.join("dev/block")).unwrap();
            for (link, target) in [
                ("8:0", "devices/pci/block/sda"),
                ("8:1", "devices/pci/block/sda/sda1"),
                ("253:0", "devices/virtual/block/dm-0"),
                ("7:0", "devices/virtual/block/loop0"),
                ("8:16", "devices/pci/usb1/1-1/block/sdb"),
                ("8:32", "devices/pci/block/sdc"),
                ("8:48", "devices/pci/block/sdd"),
                ("253:1", "devices/virtual/block/dm-1"),
            ] {
                std::os::unix::fs::symlink(root.join(target), root.join("dev/block").join(link)).unwrap();
            }
            root
        }

        #[test]
        fn a_whole_disk_is_its_own_drive_and_its_kind_is_read() {
            let drive = drive_of_dev(&sysfs(), makedev(8, 0)).unwrap();
            assert_eq!(drive.key, DriveKey::Disks([makedev(8, 0)].into_iter().collect()));
            assert_eq!(drive.kind, Kind::Rotational);
            assert_eq!(drive.label, "sda");
        }

        #[test]
        fn a_partition_is_on_its_disk() {
            let drive = drive_of_dev(&sysfs(), makedev(8, 1)).unwrap();
            assert_eq!(drive.key, DriveKey::Disks([makedev(8, 0)].into_iter().collect()));
            assert_eq!(drive.kind, Kind::Rotational);
            assert_eq!(drive.label, "sda1");
        }

        #[test]
        fn a_device_mapper_device_is_on_the_disks_under_it() {
            let drive = drive_of_dev(&sysfs(), makedev(253, 0)).unwrap();
            assert_eq!(drive.key, DriveKey::Disks([makedev(8, 0)].into_iter().collect()));
            assert_eq!(drive.kind, Kind::Rotational);
            assert_eq!(drive.label, "dm-0");
        }

        #[test]
        fn a_virtual_device_with_no_disk_is_opaque() {
            let drive = drive_of_dev(&sysfs(), makedev(7, 0)).unwrap();
            assert_eq!(drive.key, DriveKey::Opaque);
            assert_eq!(drive.kind, Kind::Unknown);
        }

        #[test]
        fn a_usb_disk_does_not_make_its_no_rotation_a_flash_disk() {
            let drive = drive_of_dev(&sysfs(), makedev(8, 16)).unwrap();
            assert_eq!(drive.key, DriveKey::Disks([makedev(8, 16)].into_iter().collect()));
            assert_eq!(drive.kind, Kind::Unknown);
        }

        #[test]
        fn a_file_system_with_no_block_device_is_unsupported() {
            let error = drive_of_dev(&sysfs(), makedev(0, 5)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        }

        #[test]
        fn a_device_missing_from_sysfs_is_an_error() {
            assert!(drive_of_dev(&sysfs(), makedev(9, 9)).is_err());
        }

        #[test]
        fn a_device_mapper_over_two_disks_is_on_both_in_one_key() {
            let drive = drive_of_dev(&sysfs(), makedev(253, 1)).unwrap();
            assert_eq!(drive.key, DriveKey::Disks([makedev(8, 32), makedev(8, 48)].into_iter().collect()));
            assert_eq!(drive.kind, Kind::Rotational, "the spinning disk makes the volume spinning");
            assert_eq!(drive.label, "dm-1");
        }

        #[test]
        fn a_disk_with_no_rotation_is_solid_and_one_with_rotation_is_rotational() {
            assert_eq!(drive_of_dev(&sysfs(), makedev(8, 32)).unwrap().kind, Kind::Solid);
            assert_eq!(drive_of_dev(&sysfs(), makedev(8, 48)).unwrap().kind, Kind::Rotational);
        }

        #[test]
        fn queue_rotational_is_read_as_solid_or_rotational_and_a_usb_bridge_is_unknown() {
            assert_eq!(kind_of(Some("0\n"), false), Kind::Solid);
            assert_eq!(kind_of(Some("1\n"), false), Kind::Rotational);
            assert_eq!(kind_of(Some("0\n"), true), Kind::Unknown, "a bridge's no-rotation says nothing");
            assert_eq!(kind_of(Some("1\n"), true), Kind::Rotational);
            assert_eq!(kind_of(None, false), Kind::Unknown, "no file");
            assert_eq!(kind_of(Some("x"), false), Kind::Unknown, "not a number");
        }

        #[test]
        fn a_device_number_with_major_zero_has_no_block_device_whatever_its_minor() {
            // Btrfs, overlayfs and ZFS report major 0, and a minor that can be large (anonymous devices).
            for minor in [0, 5, 300] {
                let error = drive_of_dev(&sysfs(), makedev(0, minor)).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::Unsupported, "minor {minor}");
            }
            assert_eq!(major(makedev(0, 300)), 0);
        }

        #[test]
        fn device_numbers_round_trip_as_glibc_has_them() {
            for (maj, min) in [(8, 0), (8, 16), (253, 0), (259, 1000), (0, 5)] {
                assert_eq!((major(makedev(maj, min)), minor(makedev(maj, min))), (maj, min));
            }
            assert_eq!(makedev(8, 1), 0x801);
        }

        #[test]
        fn the_real_folder_is_on_a_disk_or_is_a_file_system_with_none() {
            let dir = crate::testing::run_folder("scratch_drive", "pg-drive-of").unwrap();
            match SystemDrives.of(&dir) {
                Ok(drive) => assert!(!drive.label.is_empty()),
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{error}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::facts::{DiskFact, drive_of_disks};
    use super::windows_facts as wf;
    use super::*;

    fn disk(disk: u64, kind: Kind) -> DiskFact {
        DiskFact { disk, opaque: false, kind }
    }

    fn key(disks: &[u64]) -> DriveKey {
        DriveKey::Disks(disks.iter().copied().collect())
    }

    #[test]
    fn volumes_that_share_a_disk_are_one_drive_and_an_opaque_one_is_its_own() {
        let keys = [key(&[1]), key(&[1, 2]), key(&[3]), DriveKey::Opaque, DriveKey::Opaque, key(&[2]), key(&[4])];
        assert_eq!(same_drive(&keys), vec![0, 0, 2, 3, 3, 0, 6]);
    }

    #[test]
    fn a_spanned_volume_is_as_spinning_as_its_most_spinning_disk() {
        let drive = drive_of_dev_facts(&[disk(1, Kind::Rotational), disk(2, Kind::Solid)]).unwrap();
        assert_eq!((drive.key, drive.kind), (key(&[1, 2]), Kind::Rotational));
    }

    #[test]
    fn a_solid_disk_and_an_unknown_one_are_unknown() {
        let drive = drive_of_dev_facts(&[disk(1, Kind::Solid), disk(2, Kind::Unknown)]).unwrap();
        assert_eq!(drive.kind, Kind::Unknown);
        let drive = drive_of_dev_facts(&[disk(1, Kind::Solid)]).unwrap();
        assert_eq!(drive.kind, Kind::Solid);
    }

    #[test]
    fn a_volume_with_a_hidden_disk_is_opaque() {
        let hidden = DiskFact { disk: 2, opaque: true, kind: Kind::Unknown };
        let drive = drive_of_dev_facts(&[disk(1, Kind::Solid), hidden]).unwrap();
        assert_eq!((drive.key, drive.kind), (DriveKey::Opaque, Kind::Unknown));
    }

    #[test]
    fn a_volume_with_no_disk_is_an_error() {
        assert!(drive_of_dev_facts(&[]).is_err());
    }

    fn drive_of_dev_facts(facts: &[DiskFact]) -> io::Result<Drive> {
        drive_of_disks("test".to_string(), facts)
    }

    #[test]
    fn a_disk_is_read_by_its_bus_and_its_seek_penalty() {
        use wf::*;
        assert_eq!(wf::disk_fact(1, Some(BUS_SATA), Some(true)).kind, Kind::Rotational);
        assert_eq!(wf::disk_fact(1, Some(17), Some(false)).kind, Kind::Solid, "NVMe, no seek penalty");
        assert_eq!(wf::disk_fact(1, Some(BUS_SATA), None).kind, Kind::Unknown, "the query failed");
        assert_eq!(wf::disk_fact(1, Some(BUS_USB), Some(false)).kind, Kind::Unknown, "a bridge's answer");
        assert_eq!(wf::disk_fact(1, Some(BUS_USB), Some(true)).kind, Kind::Rotational, "a spinning disk says so");
        for hidden in [
            None,
            Some(BUS_UNKNOWN),
            Some(BUS_VIRTUAL),
            Some(BUS_FILE_BACKED_VIRTUAL),
            Some(BUS_SPACES),
            Some(BUS_ISCSI),
            Some(BUS_RAID),
        ] {
            assert!(wf::disk_fact(1, hidden, Some(false)).opaque, "{hidden:?}");
        }
    }

    fn put(buf: &mut [u8], at: usize, value: u32) {
        buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn the_disks_of_a_volume_come_from_its_extents() {
        let mut buf = vec![0u8; 8 + 24 * 2];
        put(&mut buf, 0, 2);
        put(&mut buf, 8, 3);
        put(&mut buf, 8 + 24, 5);
        assert_eq!(wf::parse_extents(&buf).unwrap(), vec![3, 5]);
        assert!(wf::parse_extents(&buf[..20]).is_err(), "cut short");
    }

    #[test]
    fn the_bus_and_the_seek_penalty_are_at_their_offsets() {
        let mut descriptor = vec![0u8; 40];
        put(&mut descriptor, 28, 11);
        assert_eq!(wf::parse_bus(&descriptor), Some(11));
        assert_eq!(wf::parse_bus(&descriptor[..20]), None);
        let mut penalty = vec![0u8; 12];
        penalty[8] = 1;
        assert_eq!(wf::parse_seek_penalty(&penalty), Some(true));
        penalty[8] = 0;
        assert_eq!(wf::parse_seek_penalty(&penalty), Some(false));
        assert_eq!(wf::parse_seek_penalty(&penalty[..4]), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_bus_constants_are_windows_own() {
        use windows_sys::Win32::Storage::FileSystem as fs;
        assert_eq!(wf::BUS_USB, fs::BusTypeUsb);
        assert_eq!(wf::BUS_SATA, fs::BusTypeSata);
        assert_eq!(wf::BUS_VIRTUAL, fs::BusTypeVirtual);
        assert_eq!(wf::BUS_FILE_BACKED_VIRTUAL, fs::BusTypeFileBackedVirtual);
        assert_eq!(wf::BUS_SPACES, fs::BusTypeSpaces);
        assert_eq!(wf::BUS_ISCSI, fs::BusTypeiScsi);
        assert_eq!(wf::BUS_RAID, fs::BusTypeRAID);
    }

    #[cfg(windows)]
    #[test]
    fn a_folder_on_this_machine_is_on_a_drive_that_can_be_named() {
        let dir = crate::testing::run_folder("scratch_drive", "pg-drive-of").unwrap();
        let drive = SystemDrives.of(&dir).unwrap();
        assert!(!drive.label.is_empty());
        if let DriveKey::Disks(disks) = &drive.key {
            assert!(!disks.is_empty());
        }
    }
}
