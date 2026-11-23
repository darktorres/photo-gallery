//! Directory listings: each entry's name, kind, identity, size and last write time, a block at a time. On Windows
//! this is NTFS's `FileIdBothDirectoryInfo`, which gives each file's identity with the listing: no file is opened, and
//! no administrator rights are needed. On Unix it is `std::fs::read_dir`, with the identity from the metadata. The
//! library decides which entries it keeps.

#![expect(unsafe_code, reason = "FFI: the Windows directory and handle calls, and libc's statfs and ioctl on Unix")]
//!
//! The facts about the volume a folder is on are here too ([`RootFacts`], [`fs_type`]): the scanner decides from them
//! which folders it accepts, and why it refuses one (`pg_scan::refusal`).

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

/// The identity of each file in `paths`, in their order: the file ID its folder's listing gives it (NTFS's
/// `FileIdBothDirectoryInfo`, the identity the import orders its reads by; the inode on Unix). Each folder is listed
/// once for the paths in it. An error when a path names no file its folder lists, or a folder can't be read.
pub fn file_ids(paths: &[std::path::PathBuf]) -> io::Result<Vec<u64>> {
    // Ordered maps (not hashed): a clippy-banned hash map is randomly seeded, and these keys are ordered anyway.
    let mut listed: std::collections::BTreeMap<&std::path::Path, std::collections::BTreeMap<std::ffi::OsString, u64>> =
        std::collections::BTreeMap::new();
    let mut ids = Vec::with_capacity(paths.len());
    for path in paths {
        let folder = match path.parent() {
            Some(folder) if !folder.as_os_str().is_empty() => folder,
            _ => std::path::Path::new("."),
        };
        let folder_ids = match listed.entry(folder) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert(file_ids_in(folder)?),
        };
        let name = path.file_name().ok_or_else(|| io::Error::other(format!("{} names no file", path.display())))?;
        let id = folder_ids.get(name).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("{} is not a file in its folder", path.display()))
        })?;
        ids.push(*id);
    }
    Ok(ids)
}

/// The files directly in `folder`, by name, each with its file ID (see [`file_ids`]); its folders are not among them.
fn file_ids_in(folder: &std::path::Path) -> io::Result<std::collections::BTreeMap<std::ffi::OsString, u64>> {
    let mut lister = Lister::new();
    let dir = lister.open(&crate::names::units_of(&crate::names::canonical_of(folder)))?;
    let mut ids = std::collections::BTreeMap::new();
    lister.each(&dir, |e| {
        if !e.folder {
            ids.insert(crate::names::os_name(&crate::names::name_from_native(e.name)), e.file_id);
        }
    })?;
    Ok(ids)
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
    /// A removable drive, such as a memory card or a USB stick.
    Removable,
    /// A network drive.
    Remote,
    /// Anything else: an optical drive, a RAM disk, no root, or a type the system doesn't know.
    Other,
}

/// The facts the Windows checks read, one call each, behind a trait so the decision is tested with fakes. `Handle` is a
/// folder opened without following a link.
pub trait RootFacts {
    /// An open folder, as [`RootFacts::open`] gives it.
    type Handle;
    /// The volume root of `path`, with its separator (`GetVolumePathNameW`).
    fn volume_root(&self, path: &[Unit]) -> io::Result<Vec<Unit>>;
    /// The kind of drive a volume root is (`GetDriveTypeW`).
    fn drive_type(&self, volume_root: &[Unit]) -> DriveType;
    /// `path` opened for its attributes, without following a link (`FILE_FLAG_OPEN_REPARSE_POINT`).
    fn open(&self, path: &[Unit]) -> io::Result<Self::Handle>;
    /// Whether the handle's folder is a reparse point (a link, or a mount).
    fn is_reparse_point(&self, handle: &Self::Handle) -> io::Result<bool>;
    /// The path the handle really names, in the verbatim form (`GetFinalPathNameByHandleW`).
    fn final_path(&self, handle: &Self::Handle) -> io::Result<Vec<Unit>>;
    /// The file system's name (`GetVolumeInformationByHandleW`), such as `NTFS`.
    fn file_system(&self, handle: &Self::Handle) -> io::Result<String>;
}

/// A caution about where a folder is, for the log at startup. Neither stops anything: the catalog works on either, but
/// these are the places its files are at risk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caution {
    /// The folder's drive is a network drive (`GetDriveType`): SQLite's locks and write-ahead log are not reliable over
    /// one.
    NetworkDrive,
    /// The folder lies under one a sync client keeps (`OneDrive`, `Dropbox`): the client can lock or move its files while
    /// the catalog is open.
    SyncFolder(&'static str),
    /// The folder's file system is one whose inode numbers are not stable (Linux: NFS, SMB, FAT, exFAT; see [`FsKind`]).
    FileSystem(FsKind),
}

/// The caution for `folder`, if any: a folder under a sync client's root (the roots come from `env`: `OneDrive` and its
/// variants, and `%USERPROFILE%\Dropbox`) first, then a network drive. `None` for an ordinary folder, and when the folder
/// can't be read.
pub fn folder_caution(folder: &std::path::Path, env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<Caution> {
    if let Some(name) = sync_folder(folder, &env) {
        return Some(Caution::SyncFolder(name));
    }
    system_caution(&crate::names::units_of(&crate::names::canonical_of(folder)))
}

/// The system's caution for a folder's units: on Windows its drive is a network drive (`GetDriveType`).
#[cfg(windows)]
fn system_caution(units: &[Unit]) -> Option<Caution> {
    let facts = SystemFacts;
    let volume = facts.volume_root(units).ok()?;
    (facts.drive_type(&volume) == DriveType::Remote).then_some(Caution::NetworkDrive)
}

/// The system's caution for a folder's units: on Linux its file system is one the scanner refuses for its inode numbers
/// (NFS, SMB, FAT, exFAT). The pseudo file systems are not cautioned: nothing is stored under them.
#[cfg(not(windows))]
fn system_caution(units: &[Unit]) -> Option<Caution> {
    match fs_kind(fs_type(units).ok()?)? {
        FsKind::Pseudo(_) => None,
        kind => Some(Caution::FileSystem(kind)),
    }
}

/// The sync client's name when `folder` is under one of its roots, else `None`. Compared without case, with `\` and `/`
/// alike, as Windows paths are.
pub fn sync_folder(folder: &std::path::Path, env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<&'static str> {
    const ONE_DRIVE: [&str; 3] = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"];
    if ONE_DRIVE.iter().any(|var| env(var).is_some_and(|root| under(folder, &root))) {
        return Some("OneDrive");
    }
    crate::dirs::user_folder(&env)
        .map(|home| home.join("Dropbox"))
        .filter(|dropbox| under(folder, dropbox.as_os_str()))
        .map(|_| "Dropbox")
}

/// Whether `folder` is `root` or lies under it, compared as Windows compares paths.
fn under(folder: &std::path::Path, root: &std::ffi::OsStr) -> bool {
    let fold = |p: &std::ffi::OsStr| p.to_string_lossy().to_lowercase().replace('/', "\\");
    let root = fold(root);
    let root = root.trim_end_matches('\\');
    if root.is_empty() {
        return false;
    }
    let folder = fold(folder.as_os_str());
    folder == root || folder.starts_with(&format!("{root}\\"))
}

/// The system's own facts: Win32's on Windows. Elsewhere ([`ROOT_CHECK`] is not [`RootCheck::Ntfs`]) none can be read,
/// and each call says so.
pub struct SystemFacts;

/// No Win32 off Windows: every fact is unsupported, and no drive is a fixed one.
#[cfg(not(windows))]
impl RootFacts for SystemFacts {
    type Handle = ();

    fn volume_root(&self, _path: &[Unit]) -> io::Result<Vec<Unit>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not supported on this platform"))
    }

    fn drive_type(&self, _volume_root: &[Unit]) -> DriveType {
        DriveType::Other
    }

    fn open(&self, _path: &[Unit]) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not supported on this platform"))
    }

    fn is_reparse_point(&self, _handle: &()) -> io::Result<bool> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not supported on this platform"))
    }

    fn final_path(&self, _handle: &()) -> io::Result<Vec<Unit>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not supported on this platform"))
    }

    fn file_system(&self, _handle: &()) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not supported on this platform"))
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
        // `GetDriveTypeW`'s values, from windows-sys (its `Win32_System_WindowsProgramming` feature, which the workspace takes).
        use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOTE, DRIVE_REMOVABLE};
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

/// A file system the Linux checks refuse, by kind (`fs_kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsKind {
    /// A network file system (NFS).
    Nfs,
    /// A file share (SMB, CIFS).
    Smb,
    /// FAT.
    Fat,
    /// exFAT.
    ExFat,
    /// A pseudo file system, by its name: the kernel's own entries, not files.
    Pseudo(&'static str),
}

impl FsKind {
    /// The file system's name, for a message.
    pub fn name(self) -> &'static str {
        match self {
            FsKind::Nfs => "NFS",
            FsKind::Smb => "SMB",
            FsKind::Fat => "FAT",
            FsKind::ExFat => "exFAT",
            FsKind::Pseudo(name) => name,
        }
    }
}

/// The `f_type` magic numbers refused. The network file systems (kernel `uapi/linux/magic.h`) hand out inode numbers
/// from the server. FAT (`vfat` and `msdos`, which share `MSDOS_SUPER_MAGIC`) and exFAT store no inode numbers: the
/// kernel derives each from where the entry is found while the volume is mounted, so a file's number is not the same
/// after the volume is mounted again, and the catalog would see every file as new. The pseudo file systems are the
/// kernel's own entries, not files: a scan of `/proc` reads process state, and `/sys`'s entries are attributes. The
/// numbers of the pseudo ones were read with `stat -f` on the kernel this was built on (7.0). `devtmpfs` is not here:
/// it reports tmpfs's number, which is accepted.
const FS_TYPES: &[(i64, FsKind)] = &[
    (0x6969, FsKind::Nfs),
    (0x517B, FsKind::Smb),
    (0xFF53_4D42, FsKind::Smb),
    (0xFE53_4D42, FsKind::Smb),
    (0x4D44, FsKind::Fat),
    (0x2011_BAB0, FsKind::ExFat),
    (0x9FA0, FsKind::Pseudo("proc")),
    (0x6265_6572, FsKind::Pseudo("sysfs")),
    (0x1CD1, FsKind::Pseudo("devpts")),
    (0x6367_7270, FsKind::Pseudo("cgroup2")),
    (0x6462_6720, FsKind::Pseudo("debugfs")),
    (0x7472_6163, FsKind::Pseudo("tracefs")),
    (0x7363_6673, FsKind::Pseudo("securityfs")),
    (0xCAFE_4A11, FsKind::Pseudo("bpf")),
    (0x6165_676C, FsKind::Pseudo("pstore")),
    (0x6265_6570, FsKind::Pseudo("configfs")),
];

/// The refused kind of the file system with `f_type`, `None` for every other type (`9p`, WSL's drvfs, is accepted).
pub fn fs_kind(f_type: i64) -> Option<FsKind> {
    FS_TYPES.iter().find(|(magic, _)| *magic == f_type).map(|(_, kind)| *kind)
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
    Err(io::Error::new(io::ErrorKind::Unsupported, "not supported on this platform"))
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
        /// A lister with a buffer of its own.
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

        /// The device a folder is on, as a listing numbers it: none here, where each entry is on the folder's volume.
        pub fn device(&self, dir: &Dir) -> Option<u64> {
            let _ = dir;
            None
        }

        /// The identity of device `device`, from a path on it. Never called: no entry here carries a device.
        pub fn map_device(&mut self, device: u64, probe: &[Unit]) -> u64 {
            let _ = probe;
            device
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
                        device: None,
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

    /// An open folder: on Unix, its path (listed by `std::fs::read_dir` when asked) and its device number.
    pub struct Dir {
        path: PathBuf,
        dev: u64,
    }

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
        /// A lister that has seen no volumes yet.
        pub fn new() -> Lister {
            Lister::default()
        }

        /// The devices given their device number as identity (see `Lister::folder_volume`).
        pub fn fallback_volumes(&self) -> u64 {
            self.fallbacks
        }

        /// The device a folder is on, as a listing numbers it (`Entry::device`).
        pub fn device(&self, dir: &Dir) -> Option<u64> {
            Some(dir.dev)
        }

        /// The volume a folder is on: its stable identity (see `Lister::identify`).
        pub fn folder_volume(&mut self, dir: &Dir) -> io::Result<u64> {
            Ok(self.identify(dir.dev, &dir.path.join("")))
        }

        /// The stable identity of device `device`, from a path on it (a probe: an entry's path, or the folder's). A walk
        /// calls it on its own thread, in the order it lists, so that two devices with one identity are told apart as
        /// the walk meets them (`identify`).
        pub fn map_device(&mut self, device: u64, probe: &[Unit]) -> u64 {
            self.identify(device, Path::new(OsStr::from_bytes(probe)))
        }

        /// The stable identity of the volume device `dev` is, found from `probe` (a path on the volume) and kept for the
        /// listing. A device whose identity an earlier one holds (a clone of its disk) keeps its device number, and is
        /// counted, so its files stay apart from the other's.
        fn identify(&mut self, dev: u64, probe: &Path) -> u64 {
            if let Some(id) = self.volumes.get(&dev) {
                return *id;
            }
            let id = match volume_id(dev, probe) {
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

        /// Opens the folder at `path`, a folder's name as units, and reads its volume's identity.
        pub fn open(&mut self, path: &[Unit]) -> io::Result<Dir> {
            let path = PathBuf::from(OsStr::from_bytes(path));
            let meta = std::fs::metadata(&path)?;
            if !meta.is_dir() {
                return Err(io::Error::other(format!("{} is not a folder", path.display())));
            }
            Ok(Dir { path, dev: meta.dev() })
        }

        /// Each entry of the folder, until the listing ends, with its device as the platform gives it: no identity is
        /// looked up here, so a listing needs no state of the lister's and may run on any thread (see `map_device`). An
        /// entry that vanishes while listed is skipped, as a listing that doesn't see it would; any other metadata error
        /// fails the listing (the folder is unreadable).
        pub fn each(&mut self, dir: &Dir, mut each: impl FnMut(Entry<'_>)) -> io::Result<()> {
            for entry in std::fs::read_dir(&dir.path)? {
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
                    device: Some(meta.dev()),
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
        let tree = crate::testing::run_folder("scratch_platform", "folders-listing").unwrap();
        let odd = tree.join(crate::testing::odd_name("odd"));
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

    // Real calls: the test binary's folder is on a fixed NTFS disk, not a link, and really where it is.
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

    // Real calls: `/` and `/proc` have the type numbers of their file systems.
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
        assert_ne!(lister.folder_volume(&dir).unwrap(), 0, "the folder's volume has an identity");
        let wanted = units_of(b"Cargo.toml");
        let mut found = None;
        lister
            .each(&dir, |e| {
                if e.name == wanted.as_slice() {
                    found = Some((e.folder, e.file_id, e.device, e.created));
                }
            })
            .unwrap();
        let (folder, file_id, device, created) = found.expect("Cargo.toml is in the crate's folder");
        assert!(!folder);
        assert_ne!(file_id, 0, "a file has an identity");
        // Windows gives the folder's volume once, with `volume()`, so no entry carries a device; Unix gives each entry's.
        #[cfg(windows)]
        assert_eq!(device, None);
        #[cfg(not(windows))]
        assert!(device.is_some(), "Unix gives each entry its device");
        let _ = device;
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
        lister.each(&proc, |e| seen.push((e.name.to_vec(), e.device))).unwrap();
        assert!(!seen.is_empty());
        let mut by_device = BTreeMap::new();
        for (name, device) in &seen {
            let shown = OsStr::from_bytes(name);
            let probe = std::path::Path::new("/proc").join(shown);
            let own = std::fs::symlink_metadata(&probe).unwrap().dev();
            let device = device.unwrap_or_else(|| panic!("the entry {shown:?} of /proc has no device"));
            assert_eq!(device, own, "the entry {shown:?} of /proc gives its own device");
            let volume = lister.map_device(device, probe.as_os_str().as_bytes());
            assert_eq!(*by_device.entry(own).or_insert(volume), volume, "the entry {shown:?} of /proc");
            if own == proc_dev {
                assert_eq!(volume, proc_volume, "the entry {shown:?} of /proc");
            }
        }
        let identities: std::collections::BTreeSet<_> = by_device.values().collect();
        assert_eq!(identities.len(), by_device.len(), "two devices share an identity: {by_device:?}");
    }
}

#[cfg(test)]
mod caution_tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::Path;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(v))
    }

    // The refused types are NFS, SMB, FAT, exFAT and the pseudo file systems; ext4 is not refused. A kind names itself.
    #[test]
    fn a_refused_file_system_type_has_its_kind_and_name() {
        assert_eq!(fs_kind(0x6969), Some(FsKind::Nfs));
        assert_eq!(fs_kind(0x4D44), Some(FsKind::Fat));
        assert_eq!(fs_kind(0xEF53), None, "ext4 is accepted");
        assert_eq!(fs_kind(0x9FA0).map(FsKind::name), Some("proc"));
        assert_eq!(FsKind::ExFat.name(), "exFAT");
    }

    // A folder under a sync client's root is named by its client, whatever the case of either and whichever separator.
    // Drive-letter paths are paths only on Windows: elsewhere `Path` reads `C:\Users\...` as one name, so this runs there.
    #[cfg(windows)]
    #[test]
    fn a_folder_under_a_sync_root_is_named_by_its_client() {
        let roots = env(&[("OneDrive", r"C:\Users\Ana\OneDrive"), ("USERPROFILE", r"C:\Users\Ana")]);
        assert_eq!(sync_folder(Path::new(r"c:\users\ana\onedrive\Photos\catalog"), &roots), Some("OneDrive"));
        assert_eq!(sync_folder(Path::new("C:/Users/Ana/OneDrive"), &roots), Some("OneDrive"));
        assert_eq!(sync_folder(Path::new(r"C:\Users\Ana\Dropbox\catalog"), &roots), Some("Dropbox"));
    }

    // An ordinary folder, a folder that only shares a prefix with a root, and no roots at all are not sync folders.
    #[test]
    fn a_folder_that_is_not_under_a_root_is_no_caution() {
        let roots = env(&[("OneDrive", r"C:\Users\Ana\OneDrive"), ("USERPROFILE", r"C:\Users\Ana")]);
        assert_eq!(sync_folder(Path::new(r"C:\Users\Ana\OneDriveBackup\catalog"), &roots), None);
        assert_eq!(sync_folder(Path::new(r"D:\PhotoGallery\catalog"), &roots), None);
        assert_eq!(sync_folder(Path::new(r"C:\Users\Ana\Dropbox2"), &roots), None);
        assert_eq!(sync_folder(Path::new(r"D:\catalog"), env(&[])), None);
    }
}
