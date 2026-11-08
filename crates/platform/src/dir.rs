//! Directory listings: each entry's name, kind, identity, size and last write time, a block at a time. On Windows
//! this is NTFS's `FileIdBothDirectoryInfo`, which gives each file's identity with the listing: no file is opened, and
//! no administrator rights are needed. On Unix it is `std::fs::read_dir`, with the identity from the metadata. The
//! library decides which entries it keeps.

use crate::names::units_of;
#[cfg(windows)]
use crate::native::NativePath;
use crate::native::{SEP, Unit};
use std::io;

/// A directory entry as the listing gives it. The name is borrowed from the lister's buffer until the next entry.
pub struct Entry<'a> {
    /// The name as the file system stores it: UTF-16 units on Windows, bytes elsewhere (exact in both).
    pub name: &'a [Unit],
    /// A folder rather than a file.
    pub folder: bool,
    /// A reparse point (a junction or a symbolic link): a listing doesn't follow it.
    pub reparse: bool,
    /// The file's identity on its volume (NTFS's file ID, which survives renames and moves). 0 when there is none.
    pub file_id: u64,
    /// The size in bytes.
    pub size: u64,
    /// The last write time in FILETIME ticks: 100 ns since 1601-01-01 UTC.
    pub mtime: i64,
    /// The file's own volume, read with the entry (Unix: its device number, which a mount can change under a folder).
    /// `None` where every entry of the folder is on the folder's volume (Windows).
    pub volume: Option<u64>,
    /// The creation time in FILETIME ticks; `None` where the file system or the platform doesn't give one (drvfs has
    /// none). Never an error: an unknown creation time is a NULL in the catalog.
    pub created: Option<i64>,
}

/// What listing a folder needs, kept from one folder to the next, so a listing reuses its buffers.
pub struct Lister {
    /// The folder's path, as the call is given it.
    #[cfg(windows)]
    wide: NativePath,
    /// The records, as u64s to keep them 8-byte aligned as they must be.
    #[cfg(windows)]
    buf: Vec<u64>,
}

#[cfg(windows)]
impl Lister {
    pub fn new() -> Lister {
        Lister { wide: NativePath::default(), buf: vec![0; 8192] }
    }
}

#[cfg(not(windows))]
impl Lister {
    pub fn new() -> Lister {
        Lister {}
    }
}

impl Default for Lister {
    fn default() -> Self {
        Self::new()
    }
}

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

/// Whether a folder may be scanned on Linux (decision 13 in docs/S17_7_PLAN.md): the file system's type is checked, and network types
/// (NFS, CIFS, SMB) are refused, because their inode numbers come from the server.
#[cfg(target_os = "linux")]
pub fn check_root(path: &[Unit]) -> Result<(), Refused> {
    let f_type = linux_fs_type(path).map_err(|e| refuse(format!("its file system can't be read: {e}")))?;
    check_fs_type(f_type)
}

/// Other systems are refused: the scanner's identities are checked on Windows and Linux only.
#[cfg(all(unix, not(target_os = "linux")))]
pub fn check_root(_path: &[Unit]) -> Result<(), Refused> {
    Err(refuse("scanning is supported on Windows and Linux only"))
}

/// The `f_type` magic numbers of the network file systems (kernel `uapi/linux/magic.h`).
const NFS_SUPER_MAGIC: i64 = 0x6969;
const SMB_SUPER_MAGIC: i64 = 0x517B;
const CIFS_SUPER_MAGIC: i64 = 0xFF53_4D42;
const SMB2_SUPER_MAGIC: i64 = 0xFE53_4D42;

/// Refuses the network file systems by their `f_type`; every other type is accepted, including `9p` (WSL's drvfs).
pub fn check_fs_type(f_type: i64) -> Result<(), Refused> {
    match f_type {
        NFS_SUPER_MAGIC => Err(refuse("it is on an NFS mount")),
        CIFS_SUPER_MAGIC | SMB2_SUPER_MAGIC | SMB_SUPER_MAGIC => Err(refuse("it is on an SMB or CIFS share")),
        _ => Ok(()),
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
    use super::{Entry, Lister};
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
    use super::{Entry, Lister};
    use crate::native::Unit;
    use std::ffi::OsStr;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;

    /// An open folder: on Unix, its path (listed by `std::fs::read_dir` when asked).
    pub struct Dir(PathBuf);

    impl Dir {
        /// The folder's device number. With an inode number it identifies a file; each entry's own device is in
        /// [`Entry::volume`], since a mount under the folder has another.
        pub fn volume(&self) -> io::Result<u64> {
            Ok(std::fs::metadata(&self.0)?.dev())
        }
    }

    impl Lister {
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
                    volume: Some(meta.dev()),
                    created: crate::file::created_time(&meta),
                });
            }
            Ok(())
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

    #[cfg(target_os = "linux")]
    #[test]
    fn this_systems_root_and_proc_are_accepted() {
        // Real calls: `/` is a local file system; `/proc` is procfs. Neither is a network file system.
        assert_eq!(check_root(&units_of(b"/")), Ok(()));
        assert_eq!(check_root(&units_of(b"/proc")), Ok(()));
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
        let folder_volume = dir.volume().unwrap();
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
        // `/proc` is a mount of its own (procfs): its entries are on its device, which the root's is not.
        let mut lister = Lister::new();
        let root = lister.open(&units_of(b"/")).unwrap();
        let proc = lister.open(&units_of(b"/proc")).unwrap();
        let (root_volume, proc_volume) = (root.volume().unwrap(), proc.volume().unwrap());
        assert_ne!(root_volume, proc_volume);
        let mut volumes = Vec::new();
        lister.each(&proc, |e| volumes.push(e.volume)).unwrap();
        assert!(!volumes.is_empty());
        assert!(volumes.iter().all(|v| *v == Some(proc_volume)));
    }
}
