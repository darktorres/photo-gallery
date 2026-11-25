//! Which folders the scanner takes: the policy for Windows and for Linux, decided
//! from the facts the platform reads (`pg_platform::dir`), with the reason as data (`pg_model::refusal`),
//! which `pg_wording` words. A check that can't read a fact refuses: the checks fail closed.

use std::io;
use std::path::Path;

use pg_model::refusal::{Fact, RefusedReason};
use pg_platform::dir::{DriveType, FsKind, ROOT_CHECK, RootCheck, RootFacts, SystemFacts, fs_kind};
use pg_platform::drive::{DriveKey, Drives, SystemDrives};
use pg_platform::names::units_of;
use pg_platform::native::{SEP, Unit};

/// Whether the folder at `path` (`folder` as a path the drive probe takes) may be scanned for its identities: the
/// checks this system's folders take ([`ROOT_CHECK`]).
pub(crate) fn check_root(path: &[Unit], folder: &Path) -> Result<(), RefusedReason> {
    match ROOT_CHECK {
        RootCheck::Ntfs => check_ntfs(&SystemFacts, &SystemDrives, path, folder),
        RootCheck::FsType => check_fs(path),
        RootCheck::Unsupported => Err(RefusedReason::UnsupportedSystem),
    }
}

/// Whether a folder under a scanned root, on another device than the root's, may be walked into. On Linux a local root
/// can hold a network or pseudo file system (`/proc` under `/`), so the mount is checked as the root is. On Windows a
/// volume is a drive, which the root's check covers, and a junction is a reparse point the walk never follows; so
/// nothing is checked there, nor where nothing is scanned.
pub(crate) fn check_mount(path: &[Unit]) -> Result<(), RefusedReason> {
    match ROOT_CHECK {
        RootCheck::FsType => check_fs(path),
        RootCheck::Ntfs | RootCheck::Unsupported => Ok(()),
    }
}

fn unreadable(fact: Fact, error: &io::Error) -> RefusedReason {
    RefusedReason::Unreadable { fact, error: error.to_string() }
}

/// The Windows checks, in order, before the folder is listed: NTFS on a fixed local disk, the folder itself and
/// where it really is, and a plain physical disk under the volume. `drives` is the drive probe (`pg_platform::drive`),
/// a seam of its own: the facts are `dir`'s, and `drive` already uses `dir`, so the probe can't be one of them.
fn check_ntfs<F: RootFacts>(facts: &F, drives: &dyn Drives, path: &[Unit], folder: &Path) -> Result<(), RefusedReason> {
    // 1. A network path, refused by its text: no call is made.
    if path.starts_with(&[SEP, SEP]) {
        return Err(RefusedReason::NetworkPath);
    }
    // 2 and 3: the volume the path is on is a fixed local disk.
    let volume = facts.volume_root(path).map_err(|e| unreadable(Fact::Volume, &e))?;
    check_drive(facts.drive_type(&volume))?;
    // 4: the folder itself is not a link (a junction or symbolic link), which the volume checks cannot see.
    let handle = facts.open(path).map_err(|e| unreadable(Fact::Folder, &e))?;
    if facts.is_reparse_point(&handle).map_err(|e| unreadable(Fact::Folder, &e))? {
        return Err(RefusedReason::Link);
    }
    // 5: where the folder really is must be a local disk, on the same volume.
    let real = facts.final_path(&handle).map_err(|e| unreadable(Fact::RealPath, &e))?;
    let local = local_drive_path(&real).ok_or(RefusedReason::RealNotLocal)?;
    let real_volume = facts.volume_root(local).map_err(|e| unreadable(Fact::RealVolume, &e))?;
    check_drive(facts.drive_type(&real_volume))?;
    if !same_path(&real_volume, &volume) {
        return Err(RefusedReason::RealOnOtherVolume);
    }
    // 6: the file system is NTFS (FAT32 has no file IDs that stay put; a drive-type check alone cannot see it).
    let file_system = facts.file_system(&handle).map_err(|e| unreadable(Fact::FileSystem, &e))?;
    if file_system != "NTFS" {
        return Err(RefusedReason::NotNtfs { file_system });
    }
    // 7: the volume is on plain physical disks. A disk whose backing can't be seen (virtual or file-backed, Storage
    // Spaces, RAID, iSCSI, an unknown bus) makes it opaque, the probe's one rule (`pg_platform::drive`); a USB disk is
    // not opaque, and is taken. A probe that can't be read refuses.
    let drive = drives.of(folder).map_err(|e| unreadable(Fact::Drive, &e))?;
    if drive.key == DriveKey::Opaque {
        return Err(RefusedReason::OpaqueDisk);
    }
    Ok(())
}

/// A fixed local disk is taken; any other kind of drive is refused.
fn check_drive(drive: DriveType) -> Result<(), RefusedReason> {
    match drive {
        DriveType::Fixed => Ok(()),
        DriveType::Remote => Err(RefusedReason::NetworkDrive),
        DriveType::Removable => Err(RefusedReason::RemovableDrive),
        DriveType::Other => Err(RefusedReason::OtherDrive),
    }
}

/// The path after `\\?\` when it names a drive letter (`\\?\D:\...`); `None` for anything else, including UNC.
fn local_drive_path(real: &[Unit]) -> Option<&[Unit]> {
    let rest = real.strip_prefix(&units_of(br"\\?\")[..])?;
    let letter = char::from_u32(u32::from(*rest.first()?))?;
    let local = letter.is_ascii_alphabetic() && rest.get(1) == Some(&Unit::from(b':')) && rest.get(2) == Some(&SEP);
    local.then_some(rest)
}

/// Two volume roots are the same volume (Windows paths compare without case).
fn same_path(a: &[Unit], b: &[Unit]) -> bool {
    // ASCII upper case to lower case; any other unit is itself. `Unit` is u8 on Unix and u16 on Windows.
    fn fold(u: Unit) -> Unit {
        if (u32::from(b'A')..=u32::from(b'Z')).contains(&u32::from(u)) { u + 32 } else { u }
    }
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| fold(*x) == fold(*y))
}

/// The Linux check: the file system's type, refused by [`fs_kind`].
fn check_fs(path: &[Unit]) -> Result<(), RefusedReason> {
    let f_type = pg_platform::dir::fs_type(path).map_err(|e| unreadable(Fact::FileSystem, &e))?;
    check_fs_type(f_type)
}

/// Refuses the network, FAT and pseudo file systems by their `f_type`; every other type is accepted, including `9p`
/// (WSL's drvfs).
fn check_fs_type(f_type: i64) -> Result<(), RefusedReason> {
    let Some(kind) = fs_kind(f_type) else { return Ok(()) };
    Err(match kind {
        FsKind::Nfs => RefusedReason::Nfs,
        FsKind::Smb => RefusedReason::Smb,
        FsKind::Fat => RefusedReason::Fat,
        FsKind::ExFat => RefusedReason::ExFat,
        FsKind::Pseudo(name) => RefusedReason::Pseudo { file_system: name.to_string() },
    })
}

#[cfg(test)]
mod fs_type_tests {
    use super::*;

    #[test]
    fn network_file_systems_are_refused_and_local_ones_accepted() {
        // The magic numbers of the kernel's magic.h: NFS, SMB (smbfs), CIFS and SMB2 are refused.
        assert_eq!(check_fs_type(0x6969), Err(RefusedReason::Nfs));
        for f_type in [0x517B, 0xFF53_4D42, 0xFE53_4D42] {
            assert_eq!(check_fs_type(f_type), Err(RefusedReason::Smb), "{f_type:#x}");
        }
        // ext4, tmpfs, btrfs, and 9p (WSL's drvfs, the path to NTFS that the Linux tests use) are accepted.
        for f_type in [0xEF53, 0x0102_1994, 0x9123_683E, 0x0102_1997] {
            assert_eq!(check_fs_type(f_type), Ok(()), "{f_type:#x}");
        }
    }

    #[test]
    fn fat_and_exfat_are_refused() {
        // MSDOS_SUPER_MAGIC (vfat and msdos both report it) and EXFAT_SUPER_MAGIC, from the kernel's magic.h.
        assert_eq!(check_fs_type(0x4D44), Err(RefusedReason::Fat));
        assert_eq!(check_fs_type(0x2011_BAB0), Err(RefusedReason::ExFat));
    }

    #[test]
    fn the_kernels_pseudo_file_systems_are_refused_by_name() {
        // The magic numbers measured on the kernel this was built on (see `pg_platform::dir::fs_kind`).
        let pseudo = [
            (0x9FA0, "proc"),
            (0x6265_6572, "sysfs"),
            (0x1CD1, "devpts"),
            (0x6367_7270, "cgroup2"),
            (0x6462_6720, "debugfs"),
            (0x7472_6163, "tracefs"),
            (0x7363_6673, "securityfs"),
            (0xCAFE_4A11, "bpf"),
            (0x6165_676C, "pstore"),
            (0x6265_6570, "configfs"),
        ];
        for (f_type, name) in pseudo {
            assert_eq!(check_fs_type(f_type), Err(RefusedReason::Pseudo { file_system: name.into() }), "{f_type:#x}");
        }
    }

    // Real calls, where the system checks the file system's type: `/` is a local file system; `/proc` is procfs.
    #[test]
    fn this_systems_root_is_accepted_and_proc_is_refused() {
        if ROOT_CHECK != RootCheck::FsType {
            return;
        }
        assert_eq!(check_root(&units_of(b"/"), Path::new("/")), Ok(()));
        let proc = check_root(&units_of(b"/proc"), Path::new("/proc"));
        assert_eq!(proc, Err(RefusedReason::Pseudo { file_system: "proc".into() }));
        assert_eq!(check_mount(&units_of(b"/proc")), Err(RefusedReason::Pseudo { file_system: "proc".into() }));
    }
}

/// The NTFS decision, against fakes: Windows only, as its paths are Windows' (a drive letter, `\`).
#[cfg(all(test, windows))]
mod ntfs_tests {
    use std::cell::Cell;
    use std::collections::BTreeSet;

    use pg_platform::drive::{Drive, Kind};

    use super::*;

    fn u(s: &str) -> Vec<Unit> {
        units_of(s.as_bytes())
    }

    /// A call the fake can be told to fail.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Call {
        Volume,
        Open,
        Reparse,
        Final,
        Fs,
    }

    /// The system the checks see: the drive type of the folder's volume (D:) and of the volume it really is on
    /// (E:), what the handle reports, and how many calls were made.
    struct Fake {
        drive: DriveType,
        real_drive: DriveType,
        real: &'static str,
        reparse: bool,
        fs: &'static str,
        fail: Option<Call>,
        calls: Cell<u32>,
    }

    impl Fake {
        /// A folder on a fixed NTFS disk, not a link, really where it is.
        fn local() -> Fake {
            Fake {
                drive: DriveType::Fixed,
                real_drive: DriveType::Fixed,
                real: r"\\?\D:\photos",
                reparse: false,
                fs: "NTFS",
                fail: None,
                calls: Cell::new(0),
            }
        }

        fn call(&self, c: Call) -> io::Result<()> {
            self.calls.set(self.calls.get() + 1);
            if self.fail == Some(c) { Err(io::Error::other("injected")) } else { Ok(()) }
        }
    }

    impl RootFacts for Fake {
        type Handle = ();

        fn volume_root(&self, path: &[Unit]) -> io::Result<Vec<Unit>> {
            self.call(Call::Volume)?;
            path.get(..3).map(<[Unit]>::to_vec).ok_or_else(|| io::Error::other("the path has no drive root"))
        }

        fn drive_type(&self, volume_root: &[Unit]) -> DriveType {
            if volume_root == u(r"D:\").as_slice() { self.drive } else { self.real_drive }
        }

        fn open(&self, _path: &[Unit]) -> io::Result<()> {
            self.call(Call::Open)
        }

        fn is_reparse_point(&self, _: &()) -> io::Result<bool> {
            self.call(Call::Reparse)?;
            Ok(self.reparse)
        }

        fn final_path(&self, _: &()) -> io::Result<Vec<Unit>> {
            self.call(Call::Final)?;
            Ok(u(self.real))
        }

        fn file_system(&self, _: &()) -> io::Result<String> {
            self.call(Call::Fs)?;
            Ok(self.fs.to_string())
        }
    }

    /// The drive probe the checks see: the drive it answers, or an error.
    struct FakeDrives(Result<DriveKey, &'static str>);

    impl Drives for FakeDrives {
        fn of(&self, _path: &Path) -> io::Result<Drive> {
            let key = self.0.clone().map_err(io::Error::other)?;
            Ok(Drive { key, kind: Kind::Unknown, label: "D:".into() })
        }
    }

    /// A plain physical disk, as the probe answers for one.
    fn physical() -> FakeDrives {
        FakeDrives(Ok(DriveKey::Disks(BTreeSet::from([0]))))
    }

    fn check(f: &Fake) -> Result<(), RefusedReason> {
        check_on(f, &physical())
    }

    fn check_on(f: &Fake, drives: &FakeDrives) -> Result<(), RefusedReason> {
        check_ntfs(f, drives, &u(r"D:\photos"), Path::new(r"D:\photos"))
    }

    #[test]
    fn a_fixed_local_ntfs_folder_is_allowed() {
        assert_eq!(check(&Fake::local()), Ok(()));
    }

    // The probe's answer decides: an opaque drive (a disk whose backing can't be seen: virtual or file-backed, Storage
    // Spaces, RAID, iSCSI, an unknown bus; `pg_platform::drive` tests which bus is which) is refused, and a drive of
    // physical disks (USB, SATA, NVMe, SCSI, SAS), one or spanned, is taken.
    #[test]
    fn a_folder_on_an_opaque_disk_is_refused_and_one_on_physical_disks_is_taken() {
        assert_eq!(check_on(&Fake::local(), &FakeDrives(Ok(DriveKey::Opaque))), Err(RefusedReason::OpaqueDisk));
        assert_eq!(check_on(&Fake::local(), &physical()), Ok(()));
        let spanned = FakeDrives(Ok(DriveKey::Disks(BTreeSet::from([0, 1]))));
        assert_eq!(check_on(&Fake::local(), &spanned), Ok(()));
    }

    #[test]
    fn a_drive_probe_that_fails_refuses_with_the_drive_as_its_fact() {
        let refused = check_on(&Fake::local(), &FakeDrives(Err("injected")));
        assert_eq!(refused, Err(RefusedReason::Unreadable { fact: Fact::Drive, error: "injected".into() }));
    }

    #[test]
    fn the_earlier_checks_refuse_before_the_drive_is_asked() {
        let refused = check_on(&Fake { fs: "FAT32", ..Fake::local() }, &FakeDrives(Ok(DriveKey::Opaque)));
        assert_eq!(refused, Err(RefusedReason::NotNtfs { file_system: "FAT32".into() }));
    }

    #[test]
    fn each_drive_that_is_not_a_fixed_disk_is_refused_for_its_kind() {
        assert_eq!(check(&Fake { drive: DriveType::Remote, ..Fake::local() }), Err(RefusedReason::NetworkDrive));
        assert_eq!(check(&Fake { drive: DriveType::Removable, ..Fake::local() }), Err(RefusedReason::RemovableDrive));
        assert_eq!(check(&Fake { drive: DriveType::Other, ..Fake::local() }), Err(RefusedReason::OtherDrive));
    }

    #[test]
    fn a_fixed_fat32_drive_is_refused_by_its_file_system() {
        let refused = check(&Fake { fs: "FAT32", ..Fake::local() });
        assert_eq!(refused, Err(RefusedReason::NotNtfs { file_system: "FAT32".into() }));
    }

    #[test]
    fn a_reparse_root_is_refused() {
        assert_eq!(check(&Fake { reparse: true, ..Fake::local() }), Err(RefusedReason::Link));
    }

    #[test]
    fn a_real_location_off_a_local_drive_or_on_another_volume_is_refused() {
        let unc = Fake { real: r"\\?\UNC\localhost\D$\photos", ..Fake::local() };
        assert_eq!(check(&unc), Err(RefusedReason::RealNotLocal));
        let plain = Fake { real: r"D:\photos", ..Fake::local() };
        assert_eq!(check(&plain), Err(RefusedReason::RealNotLocal), "without the verbatim drive form");
        let other = Fake { real: r"\\?\E:\photos", ..Fake::local() };
        assert_eq!(check(&other), Err(RefusedReason::RealOnOtherVolume));
        let remote = Fake { real: r"\\?\E:\photos", real_drive: DriveType::Remote, ..Fake::local() };
        assert_eq!(check(&remote), Err(RefusedReason::NetworkDrive));
    }

    #[test]
    fn each_failed_call_refuses_with_the_fact_it_could_not_read() {
        let cases = [
            (Call::Volume, Fact::Volume),
            (Call::Open, Fact::Folder),
            (Call::Reparse, Fact::Folder),
            (Call::Final, Fact::RealPath),
            (Call::Fs, Fact::FileSystem),
        ];
        for (call, fact) in cases {
            let f = Fake { fail: Some(call), ..Fake::local() };
            assert_eq!(check(&f), Err(RefusedReason::Unreadable { fact, error: "injected".into() }), "{call:?}");
        }
    }

    #[test]
    fn a_unc_path_is_refused_before_any_call() {
        let f = Fake::local();
        let path = r"\\localhost\D$\photos";
        assert_eq!(check_ntfs(&f, &physical(), &u(path), Path::new(path)), Err(RefusedReason::NetworkPath));
        assert_eq!(f.calls.get(), 0, "the text is refused without asking the system");
    }

    // A real call: the test binary's folder is on a fixed NTFS disk, a plain physical one.
    #[test]
    fn a_folder_on_a_fixed_ntfs_disk_is_accepted() {
        let dir = std::env::current_dir().unwrap();
        let path = pg_platform::names::canonical_of(&dir);
        assert_eq!(check_root(&units_of(&path), &dir), Ok(()), "{}", dir.display());
    }

    #[test]
    fn drive_letters_compare_without_case() {
        assert!(same_path(&u(r"d:\"), &u(r"D:\")));
        assert!(!same_path(&u(r"D:\"), &u(r"E:\")));
    }
}
