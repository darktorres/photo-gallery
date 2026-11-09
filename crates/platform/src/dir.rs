//! Directory listings: each entry's name, kind, identity, size and last write time, a block at a time. On Windows
//! this is NTFS's `FileIdBothDirectoryInfo`, which gives each file's identity with the listing: no file is opened, and
//! no administrator rights are needed. On Unix it is `std::fs::read_dir`, with the identity from the metadata. The
//! library decides which entries it keeps.

use crate::names::units_of;
#[cfg(windows)]
use crate::native::NativePath;
use crate::native::{SEP, Unit};
use std::io;

mod listing;
pub use listing::{Entry, Lister};

/// Why a folder can't be scanned: the reason, for the notice. `check_root` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub reason: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for Refused {}

fn refuse(reason: impl Into<String>) -> Refused {
    Refused { reason: reason.into() }
}

/// The calls the NTFS checks make, behind a trait so the decision (`check_ntfs`) is tested with fakes. `Handle` is a
/// folder opened without following a link.
pub trait RootFacts {
    type Handle;
    /// The volume root of `path`, with its separator (`GetVolumePathNameW`).
    fn volume_root(&self, path: &[Unit]) -> io::Result<Vec<Unit>>;
    /// The drive type of a volume root (`GetDriveTypeW`): 3 fixed, 2 removable, 4 remote.
    fn drive_type(&self, volume_root: &[Unit]) -> u32;
    /// `path` opened for its attributes, without following a link (`FILE_FLAG_OPEN_REPARSE_POINT`).
    fn open(&self, path: &[Unit]) -> io::Result<Self::Handle>;
    fn is_reparse_point(&self, handle: &Self::Handle) -> io::Result<bool>;
    /// The path the handle really names, in the verbatim form (`GetFinalPathNameByHandleW`).
    fn final_path(&self, handle: &Self::Handle) -> io::Result<Vec<Unit>>;
    /// The file system's name (`GetVolumeInformationByHandleW`), such as `NTFS`.
    fn file_system(&self, handle: &Self::Handle) -> io::Result<String>;
}

/// `GetDriveTypeW` values (WinBase.h). `DRIVE_FIXED` is what the scanner needs; the others are named in refusals.
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;

/// The checks of decision 6 in docs/S17_7_PLAN.md, in order, before the folder is listed. A call that fails refuses (fail closed).
pub fn check_ntfs<F: RootFacts>(facts: &F, path: &[Unit]) -> Result<(), Refused> {
    let unreadable = |what: &str, e: io::Error| refuse(format!("{what} can't be read: {e}"));
    // 1. A network path, refused by its text: no call is made.
    if path.starts_with(&[SEP, SEP]) {
        return Err(refuse("it is a network (UNC) path"));
    }
    // 2 and 3: the volume the path is on is a fixed local disk.
    let volume = facts.volume_root(path).map_err(|e| unreadable("its volume", e))?;
    check_drive(facts, &volume)?;
    // 4: the folder itself is not a link (a junction or symbolic link), which the volume checks cannot see.
    let handle = facts.open(path).map_err(|e| unreadable("the folder", e))?;
    if facts.is_reparse_point(&handle).map_err(|e| unreadable("the folder", e))? {
        return Err(refuse("it is a link (a junction or a symbolic link)"));
    }
    // 5: where the folder really is must be a local disk, on the same volume.
    let real = facts.final_path(&handle).map_err(|e| unreadable("the folder's real path", e))?;
    let local = local_drive_path(&real).ok_or_else(|| refuse("its real location is not on a local drive"))?;
    let real_volume = facts.volume_root(local).map_err(|e| unreadable("its real volume", e))?;
    check_drive(facts, &real_volume)?;
    if !same_path(&real_volume, &volume) {
        return Err(refuse("its real location is on another volume"));
    }
    // 6: the file system is NTFS (FAT32 has no file IDs that stay put; a drive-type check alone cannot see it).
    let fs = facts.file_system(&handle).map_err(|e| unreadable("its file system", e))?;
    if fs != "NTFS" {
        return Err(refuse(format!("its file system is {fs}, not NTFS")));
    }
    Ok(())
}

fn check_drive<F: RootFacts>(facts: &F, volume_root: &[Unit]) -> Result<(), Refused> {
    match facts.drive_type(volume_root) {
        DRIVE_FIXED => Ok(()),
        DRIVE_REMOTE => Err(refuse("its drive is a network drive")),
        DRIVE_REMOVABLE => Err(refuse("its drive is removable")),
        _ => Err(refuse("its drive is not a fixed local disk")),
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

/// The NTFS checks against Win32.
#[cfg(windows)]
struct WinFacts;

#[cfg(windows)]
impl RootFacts for WinFacts {
    type Handle = std::os::windows::io::OwnedHandle;

    fn volume_root(&self, path: &[Unit]) -> io::Result<Vec<Unit>> {
        use windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW;
        // The path as it was typed: the volume calls are given the plain form.
        let mut wide = NativePath::default();
        wide.set_plain(&[path]);
        let mut out = vec![0; 32 * 1024];
        let len = u32::try_from(out.len()).map_err(io::Error::other)?;
        // SAFETY: `wide` is NUL-terminated and outlives the call; `out` is writable for its length.
        if unsafe { GetVolumePathNameW(wide.units().as_ptr(), out.as_mut_ptr(), len) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(until_nul(&out))
    }

    fn drive_type(&self, volume_root: &[Unit]) -> u32 {
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        let mut wide = NativePath::default();
        wide.set_plain(&[volume_root]);
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        unsafe { GetDriveTypeW(wide.units().as_ptr()) }
    }

    fn open(&self, path: &[Unit]) -> io::Result<Self::Handle> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        // The drive root needs its separator, as `Lister::open` gives it.
        let mut wide = NativePath::default();
        wide.set(&[path, &[SEP]]);
        // SAFETY: `wide` is NUL-terminated and outlives the call; the null pointers are optional arguments.
        let h = unsafe {
            CreateFileW(
                wide.units().as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a handle just opened, owned by nothing else; the `OwnedHandle` closes it.
        Ok(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(h) })
    }

    fn is_reparse_point(&self, handle: &Self::Handle) -> io::Result<bool> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: an open handle; `info` is the struct the call fills.
        if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    }

    fn final_path(&self, handle: &Self::Handle) -> io::Result<Vec<Unit>> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
        let mut out: Vec<Unit> = vec![0; 512];
        loop {
            let len = u32::try_from(out.len()).map_err(io::Error::other)?;
            // SAFETY: an open handle; `out` is writable for its length.
            let n = unsafe { GetFinalPathNameByHandleW(handle.as_raw_handle(), out.as_mut_ptr(), len, 0) };
            if n == 0 {
                return Err(io::Error::last_os_error());
            }
            // The length without the NUL when it fits; otherwise the length needed, with the NUL.
            if pg_model::num::index(n) < out.len() {
                out.truncate(pg_model::num::index(n));
                return Ok(out);
            }
            out.resize(pg_model::num::index(n) + 1, 0);
        }
    }

    fn file_system(&self, handle: &Self::Handle) -> io::Result<String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;
        let mut volume = [0; 261];
        let mut fs = [0; 64];
        let (volume_len, fs_len) = (
            u32::try_from(volume.len()).map_err(io::Error::other)?,
            u32::try_from(fs.len()).map_err(io::Error::other)?,
        );
        // SAFETY: an open handle; both buffers are writable for their lengths; the other out-pointers are optional.
        let ok = unsafe {
            GetVolumeInformationByHandleW(
                handle.as_raw_handle(),
                volume.as_mut_ptr(),
                volume_len,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs_len,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(String::from_utf16_lossy(&until_nul(&fs)))
    }
}

/// The units before the first NUL.
#[cfg(windows)]
fn until_nul(units: &[Unit]) -> Vec<Unit> {
    units.iter().copied().take_while(|&u| u != 0).collect()
}

/// Whether a folder may be scanned for its identities: decision 6 in docs/S17_7_PLAN.md on Windows (NTFS on a fixed local disk).
#[cfg(windows)]
pub fn check_root(path: &[Unit]) -> Result<(), Refused> {
    check_ntfs(&WinFacts, path)
}

/// Whether a folder may be scanned on Linux (decision 13 in docs/S17_7_PLAN.md): the file system's type is checked, and the
/// network types (NFS, CIFS, SMB) are refused, because their inode numbers come from the server, and the kernel's pseudo file
/// systems (`/proc`, `/sys`, ...), which hold no photos.
#[cfg(target_os = "linux")]
pub fn check_root(path: &[Unit]) -> Result<(), Refused> {
    let f_type = linux_fs_type(path).map_err(|e| refuse(format!("its file system can't be read: {e}")))?;
    check_fs_type(f_type)
}

/// Whether a folder under a scanned root, on another device than the root's, may be walked into. On Linux a local root can
/// hold a network or pseudo file system (`/proc` under `/`), so the mount is checked as the root is. On Windows a volume is
/// a drive, which the root's check covers, and a junction is a reparse point the walk never follows.
#[cfg(target_os = "linux")]
pub fn check_mount(path: &[Unit]) -> Result<(), Refused> {
    check_root(path)
}

/// On Windows a folder under the root is on the root's volume (see [`check_mount`]), so nothing is checked.
#[cfg(not(target_os = "linux"))]
pub fn check_mount(_path: &[Unit]) -> Result<(), Refused> {
    Ok(())
}

/// Other systems are refused: the scanner's identities are checked on Windows and Linux only.
#[cfg(all(unix, not(target_os = "linux")))]
pub fn check_root(_path: &[Unit]) -> Result<(), Refused> {
    Err(refuse("scanning is supported on Windows and Linux only"))
}

/// The `f_type` magic numbers refused, with the reason. The network file systems (kernel `uapi/linux/magic.h`) hand out
/// inode numbers from the server. The pseudo file systems are the kernel's own entries, not files: a scan of `/proc`
/// reads process state, and `/sys`'s entries are attributes. The numbers of the pseudo ones were read with `stat -f` on
/// the kernel this was built on (7.0). `devtmpfs` is not here: it reports tmpfs's number, which is accepted.
const REFUSED: &[(i64, &str)] = &[
    (0x6969, "it is on an NFS mount"),
    (0x517B, "it is on an SMB or CIFS share"),
    (0xFF53_4D42, "it is on an SMB or CIFS share"),
    (0xFE53_4D42, "it is on an SMB or CIFS share"),
    (0x9FA0, "it is the kernel's proc file system, not a disk"),
    (0x6265_6572, "it is the kernel's sysfs file system, not a disk"),
    (0x1CD1, "it is the kernel's devpts file system, not a disk"),
    (0x6367_7270, "it is the kernel's cgroup2 file system, not a disk"),
    (0x6462_6720, "it is the kernel's debugfs file system, not a disk"),
    (0x7472_6163, "it is the kernel's tracefs file system, not a disk"),
    (0x7363_6673, "it is the kernel's securityfs file system, not a disk"),
    (0xCAFE_4A11, "it is the kernel's bpf file system, not a disk"),
    (0x6165_676C, "it is the kernel's pstore file system, not a disk"),
    (0x6265_6570, "it is the kernel's configfs file system, not a disk"),
];

/// Refuses the network and pseudo file systems by their `f_type`; every other type is accepted, including `9p` (WSL's drvfs).
pub fn check_fs_type(f_type: i64) -> Result<(), Refused> {
    match REFUSED.iter().find(|(magic, _)| *magic == f_type) {
        Some((_, reason)) => Err(refuse(*reason)),
        None => Ok(()),
    }
}

/// The `f_type` of the file system holding `path` (`statfs`). `f_type` is `i64` on 64-bit Linux, where the conversion
/// below is the identity, so the lint is allowed rather than the conversion dropped (it is needed on 32-bit).
#[cfg(target_os = "linux")]
#[expect(clippy::useless_conversion, reason = "the identity on 64-bit Linux, where f_type is i64; needed on 32-bit")]
fn linux_fs_type(path: &[Unit]) -> io::Result<i64> {
    let c = std::ffi::CString::new(path).map_err(|_| io::Error::other("the path has a NUL"))?;
    let mut st = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `c` is NUL-terminated and outlives the call; `st` is what the call fills when it returns 0.
    if unsafe { libc::statfs(c.as_ptr(), st.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: statfs returned 0, so it filled `st`.
    Ok(i64::from(unsafe { st.assume_init() }.f_type))
}

#[cfg(unix)]
pub use unix::Dir;
#[cfg(windows)]
pub use windows::Dir;

#[cfg(windows)]
mod windows {
    use super::listing::{Entry, Lister};
    use crate::native::{SEP, Unit};
    use std::io;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_ID_BOTH_DIR_INFO, FILE_ID_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FileIdBothDirectoryInfo, FileIdInfo, GetFileInformationByHandleEx, OPEN_EXISTING,
    };

    /// An open folder.
    pub struct Dir(HANDLE);

    impl Drop for Dir {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful CreateFileW (`Lister::open`) and is closed only here.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Dir {
        /// The volume's 64-bit serial number (`FileIdInfo`), which with a file ID identifies a file. Its low 32 bits
        /// are the old 32-bit serial, which two volumes can share; the whole value is what the catalog keys on. Read
        /// after the file-system check: `FileIdInfo` fails on FAT32.
        pub fn volume(&self) -> io::Result<u64> {
            let mut info = FILE_ID_INFO::default();
            let size = u32::try_from(std::mem::size_of::<FILE_ID_INFO>()).map_err(io::Error::other)?;
            // SAFETY: an open handle, and `info` is the struct the call fills (its size is the buffer's).
            let ok =
                unsafe { GetFileInformationByHandleEx(self.0, FileIdInfo, std::ptr::from_mut(&mut info).cast(), size) };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(info.VolumeSerialNumber)
        }
    }

    impl Lister {
        /// The volume a folder is on: NTFS's 64-bit volume serial number, which is the identity itself.
        pub fn folder_volume(&mut self, dir: &Dir) -> io::Result<u64> {
            dir.volume()
        }

        /// Open the folder `path` for listing. A drive root (`D:`) names the drive's current folder, so the path is
        /// opened with its trailing backslash, and in its verbatim form (`native`), which reaches a folder whose name
        /// ends in a dot and lifts the 260-character limit.
        pub fn open(&mut self, path: &[Unit]) -> io::Result<Dir> {
            self.wide.set(&[path, &[SEP]]);
            // SAFETY: `wide` is NUL-terminated and outlives the call; the null pointers are optional arguments.
            let h = unsafe {
                CreateFileW(
                    self.wide.units().as_ptr(),
                    FILE_LIST_DIRECTORY,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    std::ptr::null_mut(),
                )
            };
            if h == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            Ok(Dir(h))
        }

        /// Each entry of the folder, in the order the file system gives them, until the listing ends.
        pub fn each(&mut self, dir: &Dir, mut each: impl FnMut(Entry<'_>)) -> io::Result<()> {
            let len = u32::try_from(std::mem::size_of_val(&self.buf[..])).map_err(io::Error::other)?;
            loop {
                // SAFETY: an open handle, and `buf` is writable for the length given.
                let ok = unsafe {
                    GetFileInformationByHandleEx(dir.0, FileIdBothDirectoryInfo, self.buf.as_mut_ptr().cast(), len)
                };
                if ok == 0 {
                    let err = io::Error::last_os_error();
                    if err.raw_os_error() == Some(ERROR_NO_MORE_FILES.cast_signed()) {
                        return Ok(());
                    }
                    return Err(err);
                }
                let base = self.buf.as_ptr().cast::<u8>();
                let mut offset = 0usize;
                loop {
                    // SAFETY: the API filled `buf` with a chain of 8-byte-aligned records linked by NextEntryOffset,
                    // each followed by its FileNameLength bytes of name.
                    let (info, name) = unsafe {
                        let record = base.add(offset);
                        let info = &*record.cast::<FILE_ID_BOTH_DIR_INFO>();
                        let name_ptr = record.add(std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName));
                        let len = pg_model::num::index(info.FileNameLength) / 2;
                        (info, std::slice::from_raw_parts(name_ptr.cast::<Unit>(), len))
                    };
                    let size = u64::try_from(info.EndOfFile).map_err(io::Error::other)?;
                    each(Entry {
                        name,
                        folder: info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                        reparse: info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
                        file_id: info.FileId.cast_unsigned(),
                        size,
                        mtime: info.LastWriteTime,
                        volume: None,
                        created: Some(info.CreationTime),
                    });
                    if info.NextEntryOffset == 0 {
                        break;
                    }
                    offset += pg_model::num::index(info.NextEntryOffset);
                }
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::listing::{Entry, Lister};
    use crate::native::Unit;
    use std::ffi::OsStr;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    /// An open folder: on Unix, its path (listed by `std::fs::read_dir` when asked).
    pub struct Dir(PathBuf);

    impl Lister {
        /// The volume a folder is on: its stable identity (see `Lister::volume_of_device`).
        pub fn folder_volume(&mut self, dir: &Dir) -> io::Result<u64> {
            let dev = std::fs::metadata(&dir.0)?.dev();
            Ok(self.volume_of_device(dev, &dir.0, OsStr::new("")))
        }

        /// The stable identity of the volume device `dev` is, found from the path `folder/name` (a probe on that volume) and
        /// kept for the listing. A device whose identity an earlier one holds (a clone of its disk) keeps its device number,
        /// and is counted, so its files stay apart from the other's.
        fn volume_of_device(&mut self, dev: u64, folder: &Path, name: &OsStr) -> u64 {
            if let Some(id) = self.volumes.get(&dev) {
                return *id;
            }
            let id = match volume_id(dev, &folder.join(name)) {
                Some(id) if !self.claimed.contains_key(&id) => {
                    self.claimed.insert(id, dev);
                    id
                }
                _ => {
                    self.fallbacks += 1;
                    dev
                }
            };
            self.volumes.insert(dev, id);
            id
        }

        pub fn open(&mut self, path: &[Unit]) -> io::Result<Dir> {
            let path = PathBuf::from(OsStr::from_bytes(path));
            if !std::fs::metadata(&path)?.is_dir() {
                return Err(io::Error::other(format!("{} is not a folder", path.display())));
            }
            Ok(Dir(path))
        }

        /// Each entry of the folder, until the listing ends. An entry that vanishes while listed is skipped, as a
        /// listing that doesn't see it would; any other metadata error fails the listing (the folder is unreadable).
        pub fn each(&mut self, dir: &Dir, mut each: impl FnMut(Entry<'_>)) -> io::Result<()> {
            for entry in std::fs::read_dir(&dir.0)? {
                let entry = entry?;
                let meta = match entry.metadata() {
                    Ok(meta) => meta,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                };
                let name = entry.file_name();
                each(Entry {
                    name: name.as_bytes(),
                    folder: meta.is_dir(),
                    reparse: meta.file_type().is_symlink(),
                    file_id: meta.ino(),
                    size: meta.len(),
                    mtime: crate::file::last_write_time(&meta),
                    volume: Some(self.volume_of_device(meta.dev(), &dir.0, &name)),
                    created: crate::file::created_time(&meta),
                });
            }
            Ok(())
        }
    }

    /// The stable identity of the volume that device `dev` is: the file system's UUID (from the kernel, or from udev's
    /// database), or, for an ext file system, its `f_fsid` (which the kernel builds from the UUID). None when there is none
    /// (procfs, tmpfs, FAT without a UUID in udev's database): the device number is all there is then. `probe` is a path on
    /// the volume.
    #[cfg(target_os = "linux")]
    fn volume_id(dev: u64, probe: &Path) -> Option<u64> {
        linux_ids::volume_id(dev, probe)
    }

    /// Elsewhere the device number is the identity.
    #[cfg(not(target_os = "linux"))]
    fn volume_id(dev: u64, probe: &Path) -> Option<u64> {
        let _ = (dev, probe);
        None
    }

    #[cfg(target_os = "linux")]
    mod linux_ids {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;
        use std::path::Path;

        /// `FS_IOC_GETFSUUID`, `_IOR(0x15, 0, struct fsuuid2)`: the kernel's file system UUID (Linux 6.13 and later).
        const FS_IOC_GETFSUUID: u32 = 0x8011_1500;

        pub fn volume_id(dev: u64, probe: &Path) -> Option<u64> {
            uuid_from_kernel(probe).or_else(|| uuid_from_udev(dev)).or_else(|| ext_fsid(probe))
        }

        /// The UUID the kernel gives the file system holding `probe`: a length byte, then the UUID (FAT gives its 4-byte serial).
        fn uuid_from_kernel(probe: &Path) -> Option<u64> {
            // O_NONBLOCK, so a FIFO named like an entry doesn't block the open; O_NOFOLLOW, so the probe is the entry itself.
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
                .open(probe)
                .ok()?;
            let mut buf = [0u8; 17];
            // SAFETY: `file` is open, and the kernel writes at most the struct's 17 bytes (a length byte, then 16 bytes).
            let rc = unsafe { libc::ioctl(file.as_raw_fd(), libc::c_ulong::from(FS_IOC_GETFSUUID), buf.as_mut_ptr()) };
            if rc != 0 {
                return None;
            }
            let len = usize::from(buf[0]).min(16);
            uuid_id(buf.get(1..=len)?)
        }

        /// The ID_FS_UUID udev recorded for the device, in `/run/udev/data/b<major>:<minor>`, as it is written.
        fn uuid_from_udev(dev: u64) -> Option<u64> {
            let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
            let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
            let db = std::fs::read_to_string(format!("/run/udev/data/b{major}:{minor}")).ok()?;
            let value = db.lines().find_map(|line| line.strip_prefix("E:ID_FS_UUID="))?;
            let hex: String = value.chars().filter(|c| *c != '-').collect();
            if hex.len() == 32 {
                let bytes: Option<Vec<u8>> = (0..16)
                    .map(|i| hex.get(2 * i..2 * i + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
                    .collect();
                return uuid_id(&bytes?);
            }
            // A serial is written as its number in hex: vfat's `D35C-84C7` is 0xD35C84C7.
            u64::from_str_radix(&hex, 16).ok()
        }

        /// The `f_fsid` of an ext2, ext3 or ext4 file system. Other types are not used: a vfat's is built from the device
        /// number, as procfs's is.
        fn ext_fsid(probe: &Path) -> Option<u64> {
            let c = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
            let mut st = std::mem::MaybeUninit::<libc::statfs>::uninit();
            // SAFETY: `c` is NUL-terminated and outlives the call; `st` is what the call fills when it returns 0.
            if unsafe { libc::statfs(c.as_ptr(), st.as_mut_ptr()) } != 0 {
                return None;
            }
            // SAFETY: statfs returned 0, so it filled `st`.
            let st = unsafe { st.assume_init() };
            if st.f_type != 0xEF53 {
                return None;
            }
            // libc keeps fsid_t's field private: its two words, which are C ints, read as they are.
            // SAFETY: fsid_t is two C ints, the same size and layout as [i32; 2].
            let halves: [i32; 2] = unsafe { std::mem::transmute(st.f_fsid) };
            Some(u64::from(halves[0].cast_unsigned()) | (u64::from(halves[1].cast_unsigned()) << 32))
        }

        /// A file system's UUID bytes as its id: 16 bytes folded to 64 bits (FNV-1a, a fixed function, since the id is
        /// stored); a shorter serial as the little-endian integer of its bytes, which is the number udev writes in hex.
        fn uuid_id(bytes: &[u8]) -> Option<u64> {
            match bytes.len() {
                16 => Some(
                    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)),
                ),
                1..=8 => Some(bytes.iter().rev().fold(0u64, |acc, b| (acc << 8) | u64::from(*b))),
                _ => None,
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn a_serial_gets_the_number_udev_writes() {
                // The kernel gives vfat's serial as bytes c7 84 5c d3; udev writes it as D35C-84C7.
                assert_eq!(uuid_id(&[0xc7, 0x84, 0x5c, 0xd3]), Some(0xD35C_84C7));
                assert_eq!(u64::from_str_radix("D35C84C7", 16).ok(), Some(0xD35C_84C7));
            }

            #[test]
            fn the_kernel_and_udev_agree_on_the_root_volume_when_both_know_it() {
                use std::os::unix::fs::MetadataExt;
                let dev = std::fs::metadata("/").unwrap().dev();
                if let (Some(kernel), Some(udev)) = (uuid_from_kernel(Path::new("/")), uuid_from_udev(dev)) {
                    assert_eq!(kernel, udev);
                }
            }

            #[test]
            fn the_root_volume_has_the_same_identity_on_every_call() {
                use std::os::unix::fs::MetadataExt;
                let dev = std::fs::metadata("/").unwrap().dev();
                assert_eq!(volume_id(dev, Path::new("/")), volume_id(dev, Path::new("/")));
            }

            #[test]
            fn procfs_has_no_identity_of_its_own() {
                use std::os::unix::fs::MetadataExt;
                let dev = std::fs::metadata("/proc").unwrap().dev();
                assert_eq!(volume_id(dev, Path::new("/proc")), None);
            }
        }
    }
}

#[cfg(test)]
mod root_tests {
    use super::*;

    #[test]
    fn network_file_systems_are_refused_and_local_ones_accepted() {
        // The magic numbers of the kernel's magic.h: NFS, SMB (smbfs), CIFS and SMB2 are refused.
        for f_type in [0x6969, 0x517B, 0xFF53_4D42, 0xFE53_4D42] {
            assert!(check_fs_type(f_type).is_err(), "{f_type:#x}");
        }
        // ext4, tmpfs, btrfs, and 9p (WSL's drvfs, the path to NTFS that the Linux tests use) are accepted.
        for f_type in [0xEF53, 0x0102_1994, 0x9123_683E, 0x0102_1997] {
            assert_eq!(check_fs_type(f_type), Ok(()), "{f_type:#x}");
        }
    }

    #[test]
    fn the_kernels_pseudo_file_systems_are_refused() {
        // The magic numbers measured on the kernel this was built on (see REFUSED).
        let pseudo = [
            0x9FA0,
            0x6265_6572,
            0x1CD1,
            0x6367_7270,
            0x6462_6720,
            0x7472_6163,
            0x7363_6673,
            0xCAFE_4A11,
            0x6165_676C,
            0x6265_6570,
        ];
        for f_type in pseudo {
            assert!(check_fs_type(f_type).is_err(), "{f_type:#x}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_systems_root_is_accepted_and_proc_is_refused() {
        // Real calls: `/` is a local file system; `/proc` is procfs, which is refused.
        assert_eq!(check_root(&units_of(b"/")), Ok(()));
        assert!(check_root(&units_of(b"/proc")).is_err());
    }
}

#[cfg(all(test, windows))]
mod ntfs_tests {
    use super::*;
    use std::cell::Cell;

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
        drive: u32,
        real_drive: u32,
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
                drive: DRIVE_FIXED,
                real_drive: DRIVE_FIXED,
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

        fn drive_type(&self, volume_root: &[Unit]) -> u32 {
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

    fn check(f: &Fake) -> Result<(), Refused> {
        check_ntfs(f, &u(r"D:\photos"))
    }

    #[expect(clippy::panic, reason = "a test fails by panicking")]
    fn refused_for(f: &Fake, why: &str) {
        match check(f) {
            Err(e) => assert!(e.reason.contains(why), "{} (wanted: {why})", e.reason),
            Ok(()) => panic!("accepted; wanted a refusal for: {why}"),
        }
    }

    #[test]
    fn a_fixed_local_ntfs_folder_is_allowed() {
        assert_eq!(check(&Fake::local()), Ok(()));
    }

    #[test]
    fn a_remote_drive_is_refused() {
        refused_for(&Fake { drive: DRIVE_REMOTE, ..Fake::local() }, "network drive");
    }

    #[test]
    fn a_removable_drive_is_refused() {
        refused_for(&Fake { drive: DRIVE_REMOVABLE, ..Fake::local() }, "removable");
    }

    #[test]
    fn a_fixed_fat32_drive_is_refused_by_its_file_system() {
        refused_for(&Fake { fs: "FAT32", ..Fake::local() }, "FAT32");
    }

    #[test]
    fn a_reparse_root_is_refused() {
        refused_for(&Fake { reparse: true, ..Fake::local() }, "link");
    }

    #[test]
    fn a_real_location_on_a_remote_drive_is_refused() {
        refused_for(&Fake { real: r"\\?\UNC\localhost\D$\photos", ..Fake::local() }, "not on a local drive");
    }

    #[test]
    fn a_real_location_on_another_volume_is_refused() {
        refused_for(&Fake { real: r"\\?\E:\photos", ..Fake::local() }, "another volume");
    }

    #[test]
    fn a_real_location_on_a_remote_volume_is_refused() {
        refused_for(&Fake { real: r"\\?\E:\photos", real_drive: DRIVE_REMOTE, ..Fake::local() }, "network drive");
    }

    #[test]
    fn a_real_location_without_the_verbatim_drive_form_is_refused() {
        refused_for(&Fake { real: r"D:\photos", ..Fake::local() }, "not on a local drive");
    }

    #[test]
    fn each_failed_call_refuses() {
        for call in [Call::Volume, Call::Open, Call::Reparse, Call::Final, Call::Fs] {
            let f = Fake { fail: Some(call), ..Fake::local() };
            assert!(check(&f).is_err(), "{call:?} failing must refuse");
        }
    }

    #[test]
    fn a_unc_path_is_refused_before_any_call() {
        let f = Fake::local();
        let r = check_ntfs(&f, &u(r"\\localhost\D$\photos"));
        assert!(r.is_err());
        assert_eq!(f.calls.get(), 0, "the text is refused without asking the system");
    }

    #[test]
    fn a_folder_on_a_fixed_ntfs_disk_is_accepted() {
        // A real call: the test binary's folder is on a fixed NTFS disk.
        let dir = std::env::current_dir().unwrap();
        assert_eq!(check_root(&u(dir.to_str().unwrap())), Ok(()), "{}", dir.display());
    }

    #[test]
    fn drive_letters_compare_without_case() {
        assert!(same_path(&u(r"d:\"), &u(r"D:\")));
        assert!(!same_path(&u(r"D:\"), &u(r"E:\")));
    }
}

#[cfg(test)]
mod listing_tests {
    use super::*;
    use crate::names::units_of;

    #[test]
    fn a_listing_gives_each_entry_its_identity_volume_and_creation_time() {
        // The crate's own folder: read, never written.
        let mut lister = Lister::new();
        let dir = lister.open(&units_of(env!("CARGO_MANIFEST_DIR").as_bytes())).unwrap();
        let folder_volume = lister.folder_volume(&dir).unwrap();
        let wanted = units_of(b"Cargo.toml");
        let mut found = None;
        lister
            .each(&dir, |e| {
                if e.name == wanted.as_slice() {
                    found = Some((e.folder, e.file_id, e.volume, e.created));
                }
            })
            .unwrap();
        let (folder, file_id, volume, created) = found.expect("Cargo.toml is in the crate's folder");
        assert!(!folder);
        assert_ne!(file_id, 0, "a file has an identity");
        // Windows gives the folder's volume once, with `volume()`; Unix gives each entry's device.
        assert_eq!(volume.unwrap_or(folder_volume), folder_volume);
        #[cfg(windows)]
        assert!(created.is_some(), "NTFS gives the creation time");
        #[cfg(not(windows))]
        let _ = created;
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn each_entry_carries_its_own_device_across_a_mount() {
        // `/proc` is a mount of its own (procfs), so it is on a different device from `/`. Each entry carries the device
        // of its own `lstat`: procfs's device, or, for a path a container masks with a bind mount (Docker does, for some
        // of /proc), that mount's device. The check holds either way, and it fails if every entry took the folder's device.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::MetadataExt;
        let mut lister = Lister::new();
        let root = lister.open(&units_of(b"/")).unwrap();
        let proc = lister.open(&units_of(b"/proc")).unwrap();
        assert_ne!(lister.folder_volume(&root).unwrap(), lister.folder_volume(&proc).unwrap());
        let mut seen = Vec::new();
        lister.each(&proc, |e| seen.push((e.name.to_vec(), e.volume))).unwrap();
        assert!(!seen.is_empty());
        for (name, volume) in &seen {
            let own =
                std::fs::symlink_metadata(std::path::Path::new("/proc").join(OsStr::from_bytes(name))).unwrap().dev();
            assert_eq!(*volume, Some(own), "the entry {:?} of /proc", OsStr::from_bytes(name));
        }
    }
}
