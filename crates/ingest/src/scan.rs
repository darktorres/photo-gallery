//! Scanner: walks a folder tree and records every media file's identity, size and mtime in the catalog (which
//! files are media: `media::Ext::of_name`).
//!
//! Directories are listed with `GetFileInformationByHandleEx(FileIdBothDirectoryInfo)`, which returns each
//! entry's NTFS file ID along with its size and times, without opening the files and without admin rights. The
//! file ID gives both the catalog identity (moves and renames keep their analysis) and the MFT record number the
//! import sorts by (FINDINGS "I/O": ~3× path order on the HDD).
//!
//! PLAN §8 named an Everything-based scanner: the running Everything is 1.4, whose SDK has no file IDs, so the
//! scanner lists directories itself. Everything (or the USN journal) remains the option for incremental changes.

use anyhow::{Context, Result, bail};
use pg_core::catalog::{Catalog, ScanWriter, ScannedFile};
use pg_core::media::Ext;
use pg_core::verbatim::verbatim_wide;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FileIdBothDirectoryInfo, GetFileInformationByHandle, GetFileInformationByHandleEx, OPEN_EXISTING,
};

use crate::control::Control;

/// Folders never descended into.
const SKIP_DIRS: &[&str] = &["$RECYCLE.BIN", "System Volume Information", "_quarantine"];
/// Files per catalog transaction.
const BATCH: usize = 10_000;

/// A folder or a media file in a listing. Junctions, symlinks (which would loop or leave the tree) and files the
/// library doesn't hold aren't listed.
struct DirEntry {
    name: String,
    /// `None` for a folder.
    ext: Option<Ext>,
    file_ref: u64,
    size: u64,
    mtime: i64,
}

/// What listing a folder needs, kept from one folder to the next.
struct Lister {
    /// The folder's path as NUL-terminated UTF-16.
    wide: Vec<u16>,
    /// The records, as u64s to keep them 8-byte aligned as they must be.
    buf: Vec<u64>,
    /// Entries of earlier listings keep their names' buffers: a listing fills the first ones.
    entries: Vec<DirEntry>,
}

/// An open directory handle.
struct Dir(HANDLE);

impl Drop for Dir {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful CreateFileW (`Dir::open`) and is closed only here.
        unsafe { CloseHandle(self.0) };
    }
}

impl Dir {
    fn open(path: &str, wide: &mut Vec<u16>) -> Result<Dir> {
        // With its trailing backslash (a drive root is `D:`, which alone means the drive's current folder), and
        // in its verbatim form when long, which lifts the 260-character limit.
        verbatim_wide(&[path, "\\"], wide);
        // SAFETY: `wide` is NUL-terminated and outlives the call; the null pointers are optional arguments.
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
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: an open handle, and `info` is the struct the call fills.
        if unsafe { GetFileInformationByHandle(self.0, &mut info) } == 0 {
            bail!("GetFileInformationByHandle: {}", std::io::Error::last_os_error());
        }
        Ok(info.dwVolumeSerialNumber)
    }

    /// The folder's subfolders and media files: the first `n` of `entries`, `n` returned.
    fn entries(&self, buf: &mut [u64], entries: &mut Vec<DirEntry>) -> Result<usize> {
        let mut n = 0;
        loop {
            // SAFETY: an open handle, and `buf` is writable for the length given.
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    self.0,
                    FileIdBothDirectoryInfo,
                    buf.as_mut_ptr().cast(),
                    std::mem::size_of_val(buf) as u32,
                )
            };
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                    return Ok(n);
                }
                bail!("listing directory: {err}");
            }
            let base = buf.as_ptr().cast::<u8>();
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
                let folder = info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                let reparse = info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
                const DOT: u16 = b'.' as u16;
                let ext = if folder { None } else { Ext::of_name_utf16(name) };
                if !reparse && info.FileId != 0 && (folder && name != [DOT] && name != [DOT, DOT] || ext.is_some()) {
                    if n == entries.len() {
                        entries.push(DirEntry { name: String::new(), ext: None, file_ref: 0, size: 0, mtime: 0 });
                    }
                    let e = &mut entries[n];
                    decode_name(name, &mut e.name);
                    (e.ext, e.file_ref, e.size, e.mtime) =
                        (ext, info.FileId as u64, info.EndOfFile as u64, info.LastWriteTime);
                    n += 1;
                }
                if info.NextEntryOffset == 0 {
                    break;
                }
                offset += info.NextEntryOffset as usize;
            }
        }
    }
}

/// A UTF-16 name as `String::from_utf16_lossy` gives it, into `out` (its buffer kept), sized first: `extend` would
/// reserve only the iterator's lower bound (half the UTF-16 length) and grow again.
fn decode_name(name: &[u16], out: &mut String) {
    let decoded = || char::decode_utf16(name.iter().copied());
    out.clear();
    out.reserve(decoded().map(|c| c.map_or(char::REPLACEMENT_CHARACTER.len_utf8(), char::len_utf8)).sum());
    out.extend(decoded().map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)));
}

/// The absolute path of a folder or file as the catalog has it: in its real case, without a trailing backslash (a
/// drive root is `D:`).
pub fn normalize_path(root: &str) -> Result<String> {
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
    /// Files new, moved, renamed, rewritten or back from missing.
    pub changed: u64,
    /// Files marked missing.
    pub missing: usize,
    pub errors: u64,
    /// False when stopped early; nothing is marked missing then.
    pub complete: bool,
}

/// Record the media files under `root` (a folder, from `normalize_path`); files it no longer holds are marked missing.
pub fn scan(catalog: &mut Catalog, root: &str, ctl: &Control) -> Result<ScanStats> {
    let mut lister = Lister { wide: Vec::new(), buf: vec![0; 8192], entries: Vec::new() };
    // Opened once, for its volume and its listing.
    let root_dir = Dir::open(root, &mut lister.wide)?;
    let volume = root_dir.volume_serial()?;
    let mut root_dir = Some(root_dir);
    let scan_id = catalog.begin_scan(root)?;
    let mut stats = ScanStats { folders: 0, files: 0, changed: 0, missing: 0, errors: 0, complete: false };
    let mut stack = vec![root.to_string()];
    // The folders this scan couldn't list: the files under them weren't seen, so they aren't marked missing (R-45).
    let mut unreadable: Vec<String> = Vec::new();
    let mut tx = catalog.conn.transaction()?;
    let mut writer = ScanWriter::new(&tx, scan_id)?;
    let mut in_tx = 0;
    ctl.begin("scan", root.to_string(), 0);

    while let Some(dir) = stack.pop() {
        if !ctl.checkpoint() {
            drop(writer);
            tx.commit()?;
            ctl.finish();
            return Ok(stats);
        }
        let Lister { wide, buf, entries } = &mut lister;
        let listed =
            match root_dir.take().map_or_else(|| Dir::open(&dir, wide), Ok).and_then(|d| d.entries(buf, entries)) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("skipped: {e:#}");
                    stats.errors += 1;
                    unreadable.push(dir);
                    continue;
                }
            };
        stats.folders += 1;
        let mut folder = None;
        for e in &entries[..listed] {
            let Some(ext) = e.ext else {
                if !SKIP_DIRS.iter().any(|s| s.eq_ignore_ascii_case(&e.name)) {
                    stack.push([dir.as_str(), "\\", &e.name].concat());
                }
                continue;
            };
            if folder.is_none() {
                folder = Some(writer.folder(&dir)?);
            }
            let Some(folder) = &folder else { unreachable!("set above") };
            let changed = writer.upsert_file(
                folder,
                &ScannedFile { volume, file_ref: e.file_ref, name: &e.name, ext, size: e.size, mtime: e.mtime },
            )?;
            stats.files += 1;
            stats.changed += u64::from(changed);
            in_tx += 1;
        }
        if in_tx >= BATCH {
            drop(writer);
            tx.commit()?;
            tx = catalog.conn.transaction()?;
            writer = ScanWriter::new(&tx, scan_id)?;
            in_tx = 0;
        }
        ctl.set_done(stats.files);
    }
    drop(writer);
    tx.commit()?;
    stats.missing = catalog.mark_missing(root, scan_id, &unreadable)?;
    catalog.finish_scan(scan_id)?;
    stats.complete = true;
    ctl.finish();
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As `String::from_utf16_lossy`, in a buffer of exactly its size (one allocation), reused as it is when large
    /// enough.
    #[test]
    fn names_decode_lossily_at_their_size() {
        let names: [&[u16]; 6] = [
            &[],
            &[0x61, 0x2E, 0x6A, 0x70, 0x67],
            &[0x00E9, 0x4E2D, 0x0041],
            &[0xD83D, 0xDE00, 0x2E, 0x6A],
            &[0x61, 0xD800, 0x62, 0xDC00],
            &[0xDBFF, 0xD800, 0xDFFF],
        ];
        for name in names {
            let mut out = String::new();
            decode_name(name, &mut out);
            assert_eq!(out, String::from_utf16_lossy(name));
            assert_eq!(out.capacity(), if out.is_empty() { 0 } else { out.len().max(8) }, "{name:?}");
        }
        let mut out = String::with_capacity(64);
        let ptr = out.as_ptr();
        decode_name(&[0x61; 30], &mut out);
        assert_eq!((out.len(), out.as_ptr()), (30, ptr));
    }
}
