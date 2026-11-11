//! Directory listings: each entry's name, kind, identity, size and last write time, a block at a time. On Windows
//! this is NTFS's `FileIdBothDirectoryInfo`, which gives each file's identity with the listing: no file is opened, and
//! no administrator rights are needed. On Unix it is `std::fs::read_dir`, with the identity from the metadata. The
//! library decides which entries it keeps.
//!
//! The facts about the volume a folder is on are here too ([`RootFacts`], [`fs_type`]): the scanner decides from them
//! which folders it accepts, and why it refuses one (`pg_scan::refusal`, MODULARITY_REVIEW_4 M-34).

#[cfg(windows)]
use crate::native::NativePath;
#[cfg(windows)]
use crate::native::SEP;
use crate::native::Unit;
use std::io;

mod listing;
pub use listing::Entry;

/// The folders in `dir`, by name, sorted (a folder's name is kept exactly: the path is the listing's own): the in-app
/// folder dialog's listing. An error when `dir` can't be read, or one of its entries can't.
pub fn folders_in(dir: &std::path::Path) -> io::Result<Vec<std::path::PathBuf>> {
    let mut folders = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            folders.push(entry.path());
        }
    }
    folders.sort();
    Ok(folders)
}

/// Which checks a folder of this system takes before it is scanned for its identities: the facts there are to decide
/// from. Stated here, as [`crate::native::PATH_RULES`] is, so the scanner's policy names no platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootCheck {
    /// Windows: the drive, the link, the real location and the file system ([`RootFacts`], [`SystemFacts`]).
    Ntfs,
    /// Linux: the file system's type ([`fs_type`]), of the root and of each mount under it.
    FsType,
    /// Any other system: no identity is checked, so nothing is scanned.
    Unsupported,
}

/// The checks this system's folders take.
pub const ROOT_CHECK: RootCheck = if cfg!(windows) {
    RootCheck::Ntfs
} else if cfg!(target_os = "linux") {
    RootCheck::FsType
} else {
    RootCheck::Unsupported
};

/// What kind of drive a volume is (`GetDriveTypeW`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveType {
    /// A fixed local disk, the only kind the scanner takes.
    Fixed,
    Removable,
    /// A network drive.
    Remote,
    /// Anything else: an optical drive, a RAM disk, no root, or a type the system doesn't know.
    Other,
}

/// The facts the Windows checks read, one call each, behind a trait so the decision is tested with fakes. `Handle` is a
/// folder opened without following a link.
pub trait RootFacts {
    type Handle;
    /// The volume root of `path`, with its separator (`GetVolumePathNameW`).
    fn volume_root(&self, path: &[Unit]) -> io::Result<Vec<Unit>>;
    /// The kind of drive a volume root is (`GetDriveTypeW`).
    fn drive_type(&self, volume_root: &[Unit]) -> DriveType;
    /// `path` opened for its attributes, without following a link (`FILE_FLAG_OPEN_REPARSE_POINT`).
    fn open(&self, path: &[Unit]) -> io::Result<Self::Handle>;
    fn is_reparse_point(&self, handle: &Self::Handle) -> io::Result<bool>;
    /// The path the handle really names, in the verbatim form (`GetFinalPathNameByHandleW`).
    fn final_path(&self, handle: &Self::Handle) -> io::Result<Vec<Unit>>;
    /// The file system's name (`GetVolumeInformationByHandleW`), such as `NTFS`.
    fn file_system(&self, handle: &Self::Handle) -> io::Result<String>;
}

/// The system's own facts: Win32's on Windows. Elsewhere ([`ROOT_CHECK`] is not [`RootCheck::Ntfs`]) none can be read,
/// and each call says so.
pub struct SystemFacts;

/// No Win32 off Windows: every fact is unsupported, and no drive is a fixed one.
#[cfg(not(windows))]
impl RootFacts for SystemFacts {
    type Handle = ();

    fn volume_root(&self, _path: &[Unit]) -> io::Result<Vec<Unit>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn drive_type(&self, _volume_root: &[Unit]) -> DriveType {
        DriveType::Other
    }

    fn open(&self, _path: &[Unit]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn is_reparse_point(&self, _handle: &()) -> io::Result<bool> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn final_path(&self, _handle: &()) -> io::Result<Vec<Unit>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn file_system(&self, _handle: &()) -> io::Result<String> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(windows)]
impl RootFacts for SystemFacts {
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

    fn drive_type(&self, volume_root: &[Unit]) -> DriveType {
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        // `GetDriveTypeW`'s values (WinBase.h; windows-sys has them behind a feature this crate doesn't take).
        const DRIVE_REMOVABLE: u32 = 2;
        const DRIVE_FIXED: u32 = 3;
        const DRIVE_REMOTE: u32 = 4;
        let mut wide = NativePath::default();
        wide.set_plain(&[volume_root]);
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        match unsafe { GetDriveTypeW(wide.units().as_ptr()) } {
            DRIVE_FIXED => DriveType::Fixed,
            DRIVE_REMOVABLE => DriveType::Removable,
            DRIVE_REMOTE => DriveType::Remote,
            _ => DriveType::Other,
        }
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

/// The `f_type` of the file system holding `path` (`statfs`), the fact the Linux checks read ([`RootCheck::FsType`]).
/// Elsewhere there is none to read, and the call says so. `f_type` is `i64` on 64-bit Linux, where the conversion below
/// is the identity, so the lint is allowed rather than the conversion dropped (it is needed on 32-bit).
#[cfg(target_os = "linux")]
#[expect(clippy::useless_conversion, reason = "the identity on 64-bit Linux, where f_type is i64; needed on 32-bit")]
pub fn fs_type(path: &[Unit]) -> io::Result<i64> {
    let c = std::ffi::CString::new(path).map_err(|_| io::Error::other("the path has a NUL"))?;
    let mut st = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `c` is NUL-terminated and outlives the call; `st` is what the call fills when it returns 0.
    if unsafe { libc::statfs(c.as_ptr(), st.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: statfs returned 0, so it filled `st`.
    Ok(i64::from(unsafe { st.assume_init() }.f_type))
}

/// No `statfs` type to read off Linux.
#[cfg(not(target_os = "linux"))]
pub fn fs_type(_path: &[Unit]) -> io::Result<i64> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(unix)]
pub use unix::{Dir, Lister};
#[cfg(windows)]
pub use windows::{Dir, Lister};

#[cfg(windows)]
mod windows {
    use super::listing::Entry;
    use crate::native::{NativePath, SEP, Unit};
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

    /// What listing a folder needs, kept from one folder to the next, so a listing reuses its buffers.
    pub struct Lister {
        /// The folder's path, as the call is given it.
        wide: NativePath,
        /// The records, as u64s to keep them 8-byte aligned as they must be.
        buf: Vec<u64>,
    }

    impl Default for Lister {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Lister {
        pub fn new() -> Lister {
            Lister { wide: NativePath::default(), buf: vec![0; 8192] }
        }

        /// The devices given their device number as identity: none on Windows, where a volume's serial is always there.
        pub fn fallback_volumes(&self) -> u64 {
            0
        }

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
    use super::listing::Entry;
    use crate::native::Unit;
    use std::ffi::OsStr;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    /// An open folder: on Unix, its path (listed by `std::fs::read_dir` when asked).
    pub struct Dir(PathBuf);

    /// What listing a folder needs, kept from one folder to the next: the volume identities found so far
    /// (`std::fs::read_dir` holds its own buffer).
    #[derive(Default)]
    pub struct Lister {
        /// The stable identity of each device seen, by its device number (see `Lister::folder_volume`).
        volumes: std::collections::BTreeMap<u64, u64>,
        /// The devices that hold each identity, so a second device with the same one is found.
        claimed: std::collections::BTreeMap<u64, u64>,
        /// How many devices were given their device number as their identity (no UUID, or a clone of another's).
        fallbacks: u64,
    }

    impl Lister {
        pub fn new() -> Lister {
            Lister::default()
        }

        /// The devices given their device number as identity (see `Lister::folder_volume`).
        pub fn fallback_volumes(&self) -> u64 {
            self.fallbacks
        }

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

        pub(super) fn volume_id(dev: u64, probe: &Path) -> Option<u64> {
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
mod folders_tests {
    use std::path::Path;

    use super::folders_in;

    #[test]
    fn the_folders_listed_are_the_folders_by_their_exact_names() {
        let tree = pg_testkit::test_folder("scratch_dir", "pg-folders-listing").unwrap();
        let odd = tree.join(pg_testkit::odd_name("odd"));
        std::fs::create_dir(&odd).unwrap();
        std::fs::create_dir(tree.join("Beta")).unwrap();
        std::fs::create_dir(tree.join("alpha")).unwrap();
        std::fs::write(tree.join("a file.txt"), b"x").unwrap();
        let mut want = vec![tree.join("Beta"), tree.join("alpha"), odd];
        want.sort();
        assert_eq!(folders_in(&tree).unwrap(), want, "folders only, each by its exact path");
        let _ = std::fs::remove_dir_all(&tree);
    }

    #[test]
    fn a_folder_that_is_gone_is_an_error_not_an_empty_listing() {
        let error = folders_in(Path::new("no-such-folder-for-the-listing-test")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }
}

#[cfg(test)]
mod facts_tests {
    use super::*;
    use crate::names::units_of;

    /// Real calls: the test binary's folder is on a fixed NTFS disk, not a link, and really where it is.
    #[cfg(windows)]
    #[test]
    fn the_facts_of_a_folder_on_a_fixed_ntfs_disk() {
        let dir = std::env::current_dir().unwrap();
        let path = units_of(dir.to_str().unwrap().as_bytes());
        let facts = SystemFacts;
        let volume = facts.volume_root(&path).unwrap();
        assert_eq!(facts.drive_type(&volume), DriveType::Fixed, "{}", dir.display());
        let handle = facts.open(&path).unwrap();
        assert!(!facts.is_reparse_point(&handle).unwrap());
        assert!(facts.final_path(&handle).unwrap().starts_with(&units_of(br"\\?\")));
        assert_eq!(facts.file_system(&handle).unwrap(), "NTFS");
        assert_eq!(ROOT_CHECK, RootCheck::Ntfs);
    }

    /// Real calls: `/` and `/proc` have the type numbers of their file systems.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_file_system_type_of_proc_is_procfs() {
        assert_eq!(fs_type(&units_of(b"/proc")).unwrap(), 0x9FA0);
        assert!(fs_type(&units_of(b"/")).is_ok());
        assert!(fs_type(&units_of(b"/no/such/folder")).is_err());
        assert_eq!(ROOT_CHECK, RootCheck::FsType);
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
        // `/proc` is a mount of its own (procfs), so it is on a different device from `/`. Each entry carries the volume
        // of its own `lstat`'s device: procfs's, or, for a path a container masks with a bind mount (Docker does, for
        // some of /proc), that mount's. A volume is the device's stable identity (`Lister::folder_volume`), not its
        // device number, so the check is that the identities follow the devices: one identity per device, a different
        // one for each device, and procfs's own entries on the folder's. It fails if every entry took the folder's.
        use std::collections::BTreeMap;
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::MetadataExt;
        let mut lister = Lister::new();
        let root = lister.open(&units_of(b"/")).unwrap();
        let proc = lister.open(&units_of(b"/proc")).unwrap();
        let proc_volume = lister.folder_volume(&proc).unwrap();
        assert_ne!(lister.folder_volume(&root).unwrap(), proc_volume);
        let proc_dev = std::fs::metadata("/proc").unwrap().dev();
        let mut seen = Vec::new();
        lister.each(&proc, |e| seen.push((e.name.to_vec(), e.volume))).unwrap();
        assert!(!seen.is_empty());
        let mut by_device = BTreeMap::new();
        for (name, volume) in &seen {
            let shown = OsStr::from_bytes(name);
            let own = std::fs::symlink_metadata(std::path::Path::new("/proc").join(shown)).unwrap().dev();
            let volume = volume.unwrap_or_else(|| panic!("the entry {shown:?} of /proc has no volume"));
            assert_eq!(*by_device.entry(own).or_insert(volume), volume, "the entry {shown:?} of /proc");
            if own == proc_dev {
                assert_eq!(volume, proc_volume, "the entry {shown:?} of /proc");
            }
        }
        let identities: std::collections::BTreeSet<_> = by_device.values().collect();
        assert_eq!(identities.len(), by_device.len(), "two devices share an identity: {by_device:?}");
    }
}
