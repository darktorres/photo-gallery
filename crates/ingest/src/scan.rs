//! Scanner: walks a folder tree and records every image file's identity, size and mtime in the catalog.
//!
//! Directories are listed with `GetFileInformationByHandleEx(FileIdBothDirectoryInfo)`, which returns each
//! entry's NTFS file ID along with its size and times, without opening the files and without admin rights. The
//! file ID gives both the catalog identity (moves and renames keep their analysis) and the MFT record number the
//! import sorts by (FINDINGS "I/O": ~3× path order on the HDD).
//!
//! PLAN §8 named an Everything-based scanner: the running Everything is 1.4, whose SDK has no file IDs, so the
//! scanner lists directories itself. Everything (or the USN journal) remains the option for incremental changes.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use anyhow::{Context, Result, bail};
use pg_core::catalog::{Catalog, ScannedFile};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FileIdBothDirectoryInfo, GetFileInformationByHandle, GetFileInformationByHandleEx, OPEN_EXISTING,
};

use crate::control::Control;

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "jfif", "png", "gif", "webp", "bmp", "tif", "tiff"];
/// Folders never descended into.
const SKIP_DIRS: &[&str] = &["$RECYCLE.BIN", "System Volume Information", "_quarantine"];
/// Files per catalog transaction.
const BATCH: usize = 10_000;

struct DirEntry {
    name: String,
    is_dir: bool,
    file_ref: u64,
    size: u64,
    mtime: i64,
}

/// An open directory handle.
struct Dir(HANDLE);

impl Drop for Dir {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Dir {
    fn open(path: &str) -> Result<Dir> {
        // The \\?\ prefix lifts the 260-character limit.
        let long = if path.starts_with(r"\\") { path.to_string() } else { format!(r"\\?\{path}\") };
        let wide: Vec<u16> = OsStr::new(&long).encode_wide().chain([0]).collect();
        let h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            bail!("can't open {path}: {}", std::io::Error::last_os_error());
        }
        Ok(Dir(h))
    }

    fn volume_serial(&self) -> Result<u32> {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(self.0, &mut info) } == 0 {
            bail!("GetFileInformationByHandle: {}", std::io::Error::last_os_error());
        }
        Ok(info.dwVolumeSerialNumber)
    }

    fn entries(&self) -> Result<Vec<DirEntry>> {
        // u64 storage keeps the buffer 8-byte aligned, as the records require.
        let mut buf = vec![0u64; 8192];
        let mut out = Vec::new();
        loop {
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    self.0,
                    FileIdBothDirectoryInfo,
                    buf.as_mut_ptr().cast(),
                    (buf.len() * 8) as u32,
                )
            };
            if ok == 0 {
                let err = unsafe { GetLastError() };
                if err == ERROR_NO_MORE_FILES {
                    return Ok(out);
                }
                bail!("listing directory: {}", std::io::Error::from_raw_os_error(err as i32));
            }
            let base = buf.as_ptr().cast::<u8>();
            let mut offset = 0usize;
            loop {
                // SAFETY: the API filled `buf` with a chain of records linked by NextEntryOffset.
                let info = unsafe { &*base.add(offset).cast::<FILE_ID_BOTH_DIR_INFO>() };
                let name_ptr = unsafe { base.add(offset + std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName)) };
                let name =
                    unsafe { std::slice::from_raw_parts(name_ptr.cast::<u16>(), info.FileNameLength as usize / 2) };
                let name = String::from_utf16_lossy(name);
                if name != "." && name != ".." {
                    out.push(DirEntry {
                        is_dir: info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                        // Junctions and symlinks would loop or leave the tree.
                        file_ref: if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                            0
                        } else {
                            info.FileId as u64
                        },
                        size: info.EndOfFile as u64,
                        mtime: info.LastWriteTime,
                        name,
                    });
                }
                if info.NextEntryOffset == 0 {
                    break;
                }
                offset += info.NextEntryOffset as usize;
            }
        }
    }
}

fn is_image(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| IMAGE_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

/// An absolute folder path without a trailing backslash (a drive root is `D:`).
pub fn normalize_root(root: &str) -> Result<String> {
    // canonicalize gives the real case (a folder typed in lower case still matches the catalog's paths).
    let real = std::fs::canonicalize(root).with_context(|| format!("can't open {root}"))?;
    let s = real.to_string_lossy();
    let s = match s.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!(r"\\{unc}"),
        None => s.strip_prefix(r"\\?\").unwrap_or(&s).to_string(),
    };
    Ok(s.trim_end_matches('\\').to_string())
}

pub struct ScanStats {
    pub folders: u64,
    pub files: u64,
    pub missing: usize,
    pub errors: u64,
    /// False when stopped early; nothing is marked missing then.
    pub complete: bool,
}

/// Record the image files under `root` (from `normalize_root`); files it no longer holds are marked missing.
pub fn scan(catalog: &mut Catalog, root: &str, ctl: &Control) -> Result<ScanStats> {
    let volume = Dir::open(root)?.volume_serial()?;
    let scan_id = catalog.begin_scan(root)?;
    let mut stats = ScanStats { folders: 0, files: 0, missing: 0, errors: 0, complete: false };
    let mut stack = vec![root.to_string()];
    let mut tx = catalog.conn.transaction()?;
    let mut in_tx = 0;
    ctl.begin("scan", root.to_string(), 0);

    while let Some(dir) = stack.pop() {
        if !ctl.checkpoint() {
            tx.commit()?;
            ctl.finish();
            return Ok(stats);
        }
        let entries = match Dir::open(&dir).and_then(|d| d.entries()) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("skipped: {e:#}");
                stats.errors += 1;
                continue;
            }
        };
        stats.folders += 1;
        let mut folder_id = None;
        for e in entries {
            if e.file_ref == 0 {
                continue; // reparse point
            }
            if e.is_dir {
                if !SKIP_DIRS.iter().any(|s| s.eq_ignore_ascii_case(&e.name)) {
                    stack.push(format!("{dir}\\{}", e.name));
                }
                continue;
            }
            if !is_image(&e.name) {
                continue;
            }
            let folder = match folder_id {
                Some(id) => id,
                None => *folder_id.insert(Catalog::folder_id(&tx, &dir)?),
            };
            Catalog::upsert_file(
                &tx,
                scan_id,
                folder,
                &ScannedFile { volume, file_ref: e.file_ref, name: &e.name, size: e.size, mtime: e.mtime },
            )?;
            stats.files += 1;
            in_tx += 1;
        }
        if in_tx >= BATCH {
            tx.commit()?;
            tx = catalog.conn.transaction()?;
            in_tx = 0;
        }
        ctl.set_done(stats.files);
    }
    tx.commit()?;
    stats.missing = catalog.mark_missing(root, scan_id)?;
    catalog.finish_scan(scan_id)?;
    stats.complete = true;
    ctl.finish();
    Ok(stats)
}
