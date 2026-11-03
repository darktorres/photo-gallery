//! Scanner: walks a folder tree and records every media file's identity, size and mtime in the catalog (which
//! files are media: `media::Ext::of_name`).
//!
//! Directories are listed with `GetFileInformationByHandleEx(FileIdBothDirectoryInfo)`, which returns each
//! entry's NTFS file ID along with its size and times, without opening the files and without admin rights. The
//! file ID gives both the catalog identity (moves and renames keep their analysis) and the MFT record number the
//! import sorts by (FINDINGS "I/O": ~3× path order on the HDD).
//!
//! PLAN §8 named an Everything-based scanner: the running Everything is 1.5, whose SDK has no file IDs, so the
//! scanner lists directories itself. Everything (or the USN journal) remains the option for incremental changes.

use anyhow::{Context, Result};
use pg_catalog::{Catalog, ScannedFile};
use pg_model::ids::FileIdentity;
use pg_model::media::Ext;
use pg_platform::dir::{Dir, Lister};

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
    file_id: u64,
    size: u64,
    mtime: i64,
}

/// Lists a folder's media files and subfolders into `entries`, reusing the first ones: returns how many. The
/// platform gives the records; the choices are here: a record needs its file ID, reparse points aren't
/// followed, the dot folders are skipped, and only files the library holds are kept.
fn list(lister: &mut Lister, dir: &Dir, entries: &mut Vec<DirEntry>) -> std::io::Result<usize> {
    const DOT: u16 = b'.' as u16;
    let mut n = 0;
    lister.each(dir, |e| {
        let ext = if e.folder { None } else { Ext::of_name_utf16(e.name) };
        let kept = (e.folder && e.name != [DOT] && e.name != [DOT, DOT]) || ext.is_some();
        if !e.reparse && e.file_id != 0 && kept {
            if n == entries.len() {
                entries.push(DirEntry { name: String::new(), ext: None, file_id: 0, size: 0, mtime: 0 });
            }
            let slot = &mut entries[n];
            decode_name(e.name, &mut slot.name);
            (slot.ext, slot.file_id, slot.size, slot.mtime) = (ext, e.file_id, e.size, e.mtime);
            n += 1;
        }
    })?;
    Ok(n)
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
    let mut lister = Lister::new();
    let mut entries: Vec<DirEntry> = Vec::new();
    // Opened once, for its volume and its listing.
    let root_dir = lister.open(root).with_context(|| format!("can't open {root}"))?;
    let volume = root_dir.volume_serial()?;
    let mut root_dir = Some(root_dir);
    let scan_id = catalog.begin_scan(root)?;
    let mut stats = ScanStats { folders: 0, files: 0, changed: 0, missing: 0, errors: 0, complete: false };
    let mut stack = vec![root.to_string()];
    // The folders this scan couldn't list: the files under them weren't seen, so they aren't marked missing (R-45).
    let mut unreadable: Vec<String> = Vec::new();
    let mut tx = catalog.tx()?;
    let mut writer = tx.scan_writer(scan_id)?;
    let mut in_tx = 0;
    ctl.begin("scan", root.to_string(), 0);

    while let Some(dir) = stack.pop() {
        // A stop returns before `finish_scan` and before `mark_missing`: the walk is incomplete, so nothing is
        // marked missing, and the scan's `finished` stays NULL. No reader looks at `scans.finished`, so a stopped
        // scan and an interrupted one look the same there.
        if !ctl.checkpoint() {
            drop(writer);
            tx.commit()?;
            ctl.finish();
            return Ok(stats);
        }
        let listed = match root_dir
            .take()
            .map_or_else(|| lister.open(&dir), Ok)
            .and_then(|d| list(&mut lister, &d, &mut entries))
        {
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
                &ScannedFile {
                    identity: FileIdentity::new(volume, e.file_id),
                    name: &e.name,
                    ext,
                    size: e.size,
                    mtime: e.mtime,
                },
            )?;
            stats.files += 1;
            stats.changed += u64::from(changed);
            in_tx += 1;
        }
        if in_tx >= BATCH {
            drop(writer);
            tx.commit()?;
            tx = catalog.tx()?;
            writer = tx.scan_writer(scan_id)?;
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
