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
//!
//! Names and folders are exact: a listed name is a [`Name`] (its units as the system gave them), a folder is a
//! [`RulePath`] that the walk extends by its names, and a folder is opened by its units. Nothing is decoded lossily.

use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use pg_catalog::{Catalog, ScannedFile};
use pg_model::ids::FileIdentity;
use pg_model::media::Ext;
use pg_model::name::Name;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::dir::{Dir, Lister};

use pg_jobs::control::{Control, Phase};
use pg_jobs::failure::{Failure, keep_failure};
use pg_platform::names::{canonical_of, name_from_native, units_of};

/// Folders never descended into, by the rules they are under (their names compare ASCII case-insensitively). The
/// NTFS system folders exist only on Windows, where the library runs them; a Posix library keeps every folder it lists.
fn skip_dirs(rules: PathRules) -> &'static [&'static str] {
    match rules {
        PathRules::Windows => &["$RECYCLE.BIN", "System Volume Information", "_quarantine"],
        PathRules::Posix => &[],
    }
}
/// Files per catalog transaction.
const BATCH: usize = 10_000;

/// A folder or a media file in a listing. Junctions, symlinks (which would loop or leave the tree) and files the
/// library doesn't hold aren't listed.
struct DirEntry {
    name: Name,
    /// `None` for a folder.
    ext: Option<Ext>,
    file_id: u64,
    size: u64,
    mtime: i64,
}

/// Lists a folder's media files and subfolders into `entries`, reusing the first ones: returns how many. The
/// platform gives the records; the choices are here: a record needs its file ID, reparse points aren't
/// followed, the dot folders are skipped, and only files the library holds are kept.
fn list(lister: &mut Lister, dir: &Dir, entries: &mut Vec<DirEntry>) -> io::Result<usize> {
    let mut n = 0;
    lister.each(dir, |e| {
        let name = name_from_native(e.name);
        let ext = if e.folder { None } else { Ext::of_name_bytes(name.as_bytes()) };
        let kept = (e.folder && !name.is_dot()) || ext.is_some();
        if !e.reparse && e.file_id != 0 && kept {
            if n == entries.len() {
                entries.push(DirEntry { name: Name::from_raw(&[]), ext: None, file_id: 0, size: 0, mtime: 0 });
            }
            let slot = &mut entries[n];
            slot.name = name;
            (slot.ext, slot.file_id, slot.size, slot.mtime) = (ext, e.file_id, e.size, e.mtime);
            n += 1;
        }
    })?;
    Ok(n)
}

/// The folder at `path`, opened for listing: by its units, as the system takes them.
fn open_folder(lister: &mut Lister, rules: PathRules, path: &RulePath) -> io::Result<Dir> {
    lister.open(&units_of(&rules.join(path)))
}

/// The absolute path of a folder or file as the catalog has it: in its real case, without the verbatim prefix and a
/// trailing separator (a drive root is `D:`). Exact: a name with an unpaired surrogate keeps it, so a folder whose
/// real name has one (a short name such as `PROGRA~1` can resolve to it) is still the folder it is.
pub fn normalize_path(root: &Path) -> Result<RulePath> {
    // canonicalize gives the real case (a folder typed in lower case still matches the catalog's paths).
    let real = std::fs::canonicalize(root).with_context(|| format!("can't open {}", root.display()))?;
    let mut bytes = canonical_of(&real);
    if cfg!(windows) {
        if let Some(unc) = bytes.strip_prefix(br"\\?\UNC\") {
            bytes = [br"\\", unc].concat();
        } else if let Some(rest) = bytes.strip_prefix(br"\\?\") {
            bytes = rest.to_vec();
        }
    }
    // Trailing and doubled separators are dropped by the parse.
    PathRules::NATIVE.parse_bytes(&bytes).with_context(|| format!("{} is not a path", real.display()))
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
    /// The first folders that couldn't be read (`FAILURES_KEPT`).
    pub failures: Vec<Failure>,
}

/// Record the media files under `root` (a folder of the catalog's rules); files it no longer holds are marked missing.
pub fn scan(catalog: &mut Catalog, root: &RulePath, ctl: &Control) -> Result<ScanStats> {
    let rules = catalog.rules();
    let root_name = rules.display(root);
    let mut lister = Lister::new();
    let mut entries: Vec<DirEntry> = Vec::new();
    // Opened once, for its volume and its listing.
    let root_dir = open_folder(&mut lister, rules, root).with_context(|| format!("can't open {root_name}"))?;
    let volume = root_dir.volume_serial()?;
    let mut root_dir = Some(root_dir);
    let scan_id = catalog.begin_scan(root)?;
    let mut stats =
        ScanStats { folders: 0, files: 0, changed: 0, missing: 0, errors: 0, complete: false, failures: Vec::new() };
    let mut stack = vec![root.clone()];
    // The folders this scan couldn't list: the files under them weren't seen, so they aren't marked missing (R-45).
    let mut unreadable: Vec<RulePath> = Vec::new();
    let mut tx = catalog.tx()?;
    let mut writer = tx.scan_writer(scan_id)?;
    let mut in_tx = 0;
    ctl.begin(Phase::Scan { root: root_name.clone() }, 0);

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
            .map_or_else(|| open_folder(&mut lister, rules, &dir), Ok)
            .and_then(|d| list(&mut lister, &d, &mut entries))
        {
            Ok(n) => n,
            Err(e) => {
                keep_failure(&mut stats.failures, rules.display(&dir), format!("{e:#}"));
                stats.errors += 1;
                unreadable.push(dir);
                continue;
            }
        };
        stats.folders += 1;
        let mut folder = None;
        for e in &entries[..listed] {
            let Some(ext) = e.ext else {
                if !skip_dirs(rules).iter().any(|s| e.name.as_bytes().eq_ignore_ascii_case(s.as_bytes())) {
                    stack.push(RulePath {
                        anchor: dir.anchor.clone(),
                        parts: dir.parts.iter().cloned().chain([e.name.clone()]).collect(),
                    });
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
    use pg_platform::native::Unit;

    #[test]
    fn a_root_whose_real_name_is_not_unicode_keeps_it() {
        // The folder is made with the name (an unpaired surrogate, or a byte no UTF-8 sequence holds), then normalized:
        // its last part is that name, exactly, where a lossy text form would have U+FFFD.
        #[cfg(windows)]
        let (os, name): (std::ffi::OsString, Vec<Unit>) = {
            use std::os::windows::ffi::OsStringExt;
            (std::ffi::OsString::from_wide(&[0x61, 0xD800]), vec![0x61, 0xD800])
        };
        #[cfg(unix)]
        let (os, name): (std::ffi::OsString, Vec<Unit>) = {
            use std::os::unix::ffi::OsStringExt;
            (std::ffi::OsString::from_vec(vec![0x61, 0xFF]), vec![0x61, 0xFF])
        };
        let base = std::env::var_os("PG_TEST_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("pg-normalize-{}", std::process::id()));
        let folder = base.join(&os);
        std::fs::create_dir_all(&folder).unwrap();
        let path = normalize_path(&folder).unwrap();
        assert_eq!(path.parts.last(), Some(&name_from_native(&name)));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_listed_name_is_kept_exactly() {
        // An unpaired surrogate (NTFS) or a byte no UTF-8 sequence holds (Unix): the units come back as they were
        // listed, where a lossy decode would have given U+FFFD.
        #[cfg(windows)]
        let units: Vec<Unit> = vec![0x61, 0xD800, 0x62];
        #[cfg(unix)]
        let units: Vec<Unit> = vec![0x61, 0xFF, 0x62];
        let name = name_from_native(&units);
        assert_eq!(units_of(name.as_bytes()), units);
        assert!(Ext::of_name_bytes(name.as_bytes()).is_none());
    }

    #[test]
    fn dot_folders_are_not_listed_and_media_is_recognised_in_any_case() {
        assert!(name_from_native(&units_of(b".")).is_dot());
        assert!(name_from_native(&units_of(b"..")).is_dot());
        assert!(!name_from_native(&units_of(b"...")).is_dot());
        assert!(Ext::of_name_bytes(b"a.mozjpeg.JPG").is_some());
    }
}
