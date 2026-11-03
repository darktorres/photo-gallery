//! Directory listings: each entry's name, kind, identity, size and last write time, a block at a time. On Windows
//! this is NTFS's `FileIdBothDirectoryInfo`, which gives each file's identity with the listing: no file is opened, and
//! no administrator rights are needed. On Unix it is `std::fs::read_dir`, with the identity from the metadata. The
//! library decides which entries it keeps.

/// A directory entry as the listing gives it. The name is borrowed from the lister's buffer until the next entry.
pub struct Entry<'a> {
    /// The name in UTF-16, as the file system stores it.
    pub name: &'a [u16],
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
}

/// What listing a folder needs, kept from one folder to the next, so a listing reuses its buffers.
pub struct Lister {
    /// The folder's path as NUL-terminated UTF-16.
    #[cfg(windows)]
    wide: Vec<u16>,
    /// The records, as u64s to keep them 8-byte aligned as they must be.
    #[cfg(windows)]
    buf: Vec<u64>,
}

#[cfg(windows)]
impl Lister {
    pub fn new() -> Lister {
        Lister { wide: Vec::new(), buf: vec![0; 8192] }
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

#[cfg(unix)]
pub use unix::Dir;
#[cfg(windows)]
pub use windows::Dir;

#[cfg(windows)]
mod windows {
    use super::{Entry, Lister};
    use crate::verbatim::verbatim_wide;
    use std::io;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileIdBothDirectoryInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        OPEN_EXISTING,
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
        /// The volume's serial number, which with a file ID identifies a file.
        pub fn volume_serial(&self) -> io::Result<u32> {
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            // SAFETY: an open handle, and `info` is the struct the call fills.
            if unsafe { GetFileInformationByHandle(self.0, &mut info) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(info.dwVolumeSerialNumber)
        }
    }

    impl Lister {
        /// Open the folder `path` for listing. A drive root (`D:`) names the drive's current folder, so the path is
        /// opened with its trailing backslash, and in its verbatim form when long, which lifts the 260-character limit.
        pub fn open(&mut self, path: &str) -> io::Result<Dir> {
            verbatim_wide(&[path, "\\"], &mut self.wide);
            // SAFETY: `wide` is NUL-terminated and outlives the call; the null pointers are optional arguments.
            let h = unsafe {
                CreateFileW(
                    self.wide.as_ptr(),
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
            loop {
                // SAFETY: an open handle, and `buf` is writable for the length given.
                let ok = unsafe {
                    GetFileInformationByHandleEx(
                        dir.0,
                        FileIdBothDirectoryInfo,
                        self.buf.as_mut_ptr().cast(),
                        std::mem::size_of_val(&self.buf[..]) as u32,
                    )
                };
                if ok == 0 {
                    let err = io::Error::last_os_error();
                    if err.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
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
                        (info, std::slice::from_raw_parts(name_ptr.cast::<u16>(), info.FileNameLength as usize / 2))
                    };
                    each(Entry {
                        name,
                        folder: info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                        reparse: info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
                        file_id: info.FileId as u64,
                        size: info.EndOfFile as u64,
                        mtime: info.LastWriteTime,
                    });
                    if info.NextEntryOffset == 0 {
                        break;
                    }
                    offset += info.NextEntryOffset as usize;
                }
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::{Entry, Lister};
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;

    /// An open folder: on Unix, its path (listed by `std::fs::read_dir` when asked).
    pub struct Dir(PathBuf);

    impl Dir {
        /// The device number, which with an inode number identifies a file.
        pub fn volume_serial(&self) -> io::Result<u32> {
            Ok(std::fs::metadata(&self.0)?.dev() as u32)
        }
    }

    impl Lister {
        pub fn open(&mut self, path: &str) -> io::Result<Dir> {
            let path = PathBuf::from(path);
            if !std::fs::metadata(&path)?.is_dir() {
                return Err(io::Error::other(format!("{} is not a folder", path.display())));
            }
            Ok(Dir(path))
        }

        /// Each entry of the folder, until the listing ends. An entry that vanishes while listed is skipped, as a
        /// listing that doesn't see it would.
        pub fn each(&mut self, dir: &Dir, mut each: impl FnMut(Entry<'_>)) -> io::Result<()> {
            for entry in std::fs::read_dir(&dir.0)? {
                let entry = entry?;
                let Ok(meta) = entry.metadata() else { continue };
                let name: Vec<u16> = entry.file_name().to_string_lossy().encode_utf16().collect();
                each(Entry {
                    name: &name,
                    folder: meta.is_dir(),
                    reparse: meta.file_type().is_symlink(),
                    file_id: meta.ino(),
                    size: meta.len(),
                    mtime: crate::file::last_write_time(&meta),
                });
            }
            Ok(())
        }
    }
}
