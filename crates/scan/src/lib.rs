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
//!
//! Identity and pairing (S-17.7): a file is recorded by its volume and file ID; a file the catalog has no row for is
//! paired at the end of the scan (`pg_catalog`'s `pair`), which is what makes a renumbered volume or a file moved
//! while the app was closed keep its ratings.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use pg_catalog::{Catalog, Recorded, ScannedFile};
use pg_model::ids::{FileId, FileIdentity};
use pg_model::media::Ext;
use pg_model::name::Name;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::dir::{Entry, Lister};
use pg_platform::native::Unit;

use pg_jobs::control::{Control, Phase};
use pg_jobs::failure::{Failure, keep_failure};
use pg_platform::names::{canonical_of, name_from_native, units_of};

/// The folder listing a scan reads with: the platform's [`Lister`], or a fake in the tests.
pub trait Listing {
    type Dir;
    fn open(&mut self, path: &[Unit]) -> io::Result<Self::Dir>;
    /// The volume the folder is on: the one its entries are on, where the platform has one volume per folder
    /// (Windows). An entry that carries its own volume (Unix) uses that instead.
    fn volume(&mut self, dir: &Self::Dir) -> io::Result<u64>;
    fn each(&mut self, dir: &Self::Dir, each: impl FnMut(Entry<'_>)) -> io::Result<()>;
}

impl Listing for Lister {
    type Dir = pg_platform::dir::Dir;

    fn open(&mut self, path: &[Unit]) -> io::Result<Self::Dir> {
        Lister::open(self, path)
    }

    fn volume(&mut self, dir: &Self::Dir) -> io::Result<u64> {
        dir.volume()
    }

    fn each(&mut self, dir: &Self::Dir, each: impl FnMut(Entry<'_>)) -> io::Result<()> {
        Lister::each(self, dir, each)
    }
}

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

/// A folder or a media file in a listing. Files the library doesn't hold aren't listed.
struct DirEntry {
    name: Name,
    /// `None` for a folder.
    ext: Option<Ext>,
    file_id: u64,
    size: u64,
    mtime: i64,
    /// A reparse point (a junction, a symlink, or a cloud placeholder's reparse data): never followed (decision 9).
    reparse: bool,
    /// The volume the entry is on (its own, or the folder's).
    volume: u64,
    created: Option<i64>,
}

/// Lists a folder's media files and subfolders into `entries`, reusing the first ones: returns how many. The
/// platform gives the records; the choices are here: a record needs its file ID, the dot folders are skipped, and
/// only files the library holds and folders are kept. Reparse points are kept, marked, so the walk can index them
/// without following them.
fn list<L: Listing>(lister: &mut L, dir: &L::Dir, root_volume: u64, entries: &mut Vec<DirEntry>) -> io::Result<usize> {
    let mut n = 0;
    let mut no_id = false;
    lister.each(dir, |e| {
        let name = name_from_native(e.name);
        // An entry with no file ID has no identity to keep its rows by. The folder is then unreadable, so its
        // entries are not recorded and its rows are not marked missing (decision 10).
        if e.file_id == 0 && !name.is_dot() {
            no_id = true;
            return;
        }
        let ext = if e.folder { None } else { Ext::of_name_bytes(name.as_bytes()) };
        let kept = (e.folder && !name.is_dot()) || ext.is_some();
        if kept {
            let entry = DirEntry {
                name,
                ext,
                file_id: e.file_id,
                size: e.size,
                mtime: e.mtime,
                reparse: e.reparse,
                volume: e.volume.unwrap_or(root_volume),
                created: e.created,
            };
            match entries.get_mut(n) {
                Some(slot) => *slot = entry,
                None => entries.push(entry),
            }
            n += 1;
        }
    })?;
    if no_id {
        return Err(io::Error::other("an entry has no file ID, so the folder's identities can't be read"));
    }
    Ok(n)
}

/// The folder at `path`, opened for listing: by its units, as the system takes them.
fn open_folder<L: Listing>(lister: &mut L, rules: PathRules, path: &RulePath) -> io::Result<L::Dir> {
    lister.open(&units_of(&rules.join(path)))
}

/// A path under `parent`, extended by one name.
fn child_path(parent: &RulePath, name: &Name) -> RulePath {
    RulePath { anchor: parent.anchor.clone(), parts: parent.parts.iter().cloned().chain([name.clone()]).collect() }
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

/// What a scan of a root did. The counts are for the report; `replaced_ids` and `temps` are for the caller to act on
/// after the scan's commit (the thumbnails and the temp files of the replaced files; decision 7).
#[derive(Debug, Default)]
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
    /// Paths of the same file (a hard link, or a row of another root) that were skipped, not recorded (decision 5).
    pub duplicates: u64,
    /// Reparse points indexed without following them: a file's row is kept, a folder's subtree is kept (decision 9).
    pub skipped_reparse: u64,
    /// Folders not descended into because they were already on the path (a bind-mount cycle, decision 11).
    pub loops: u64,
    /// Rows that took the identity of a new row on another volume (pairing tiers 1 and 2).
    pub rebound: usize,
    /// Rows that took the identity of the file that replaced them (pairing's replacement).
    pub replaced: usize,
    /// Fresh rows whose pairing candidates were several: nothing bound.
    pub ambiguous: usize,
    /// Unpaired rows with children (a rating, a rotation, an analysis) that no tier pairs.
    pub unpaired_with_data: usize,
    /// The replaced rows, whose thumbnails the caller deletes.
    pub replaced_ids: Vec<FileId>,
    /// The temp files of queued writes dropped with a replacement, which the caller deletes.
    pub temps: Vec<RulePath>,
}

/// Whether the folder `root` may be scanned for its identities (decisions 6 and 13). The scan makes this check first, and
/// so does adding a folder to the library.
pub fn check_folder(rules: PathRules, root: &RulePath) -> Result<(), pg_platform::dir::Refused> {
    pg_platform::dir::check_root(&units_of(&rules.join(root)))
}

/// Record the media files under `root` (a folder of the catalog's rules); files it no longer holds are marked missing.
/// A folder that [`check_folder`] refuses is an error that is a `Refused`, before anything is recorded.
pub fn scan(catalog: &mut Catalog, root: &RulePath, ctl: &Control) -> Result<ScanStats> {
    // A refused root writes no scans row, and nothing is marked missing.
    check_folder(catalog.rules(), root)?;
    walk(catalog, root, ctl, &mut Lister::new())
}

/// The folder being walked: its path, the folders it is under (with itself last) as `(volume, file ID)`, which a
/// folder met again is looked for in (a cycle), and its listing when it is already open.
struct Frame<D> {
    path: RulePath,
    chain: Vec<(u64, u64)>,
    dir: Option<D>,
}

/// The walk, over any listing: [`scan`] after its check.
fn walk<L: Listing>(catalog: &mut Catalog, root: &RulePath, ctl: &Control, lister: &mut L) -> Result<ScanStats> {
    let rules = catalog.rules();
    let root_name = rules.display(root);
    let root_dir = open_folder(lister, rules, root).with_context(|| format!("can't open {root_name}"))?;
    let root_volume = lister.volume(&root_dir)?;
    let scan_id = catalog.begin_scan(root)?;
    let mut stats = ScanStats::default();
    let mut stack = vec![Frame { path: root.clone(), chain: Vec::new(), dir: Some(root_dir) }];
    // The folders this scan couldn't list, or were told not to read (a reparse folder): the files under them weren't
    // seen, so they aren't marked missing.
    let mut unreadable: Vec<RulePath> = Vec::new();
    // Each identity recorded or seen this scan: the second path to a file is skipped (decision 5).
    let mut seen: BTreeSet<FileIdentity> = BTreeSet::new();
    let mut entries: Vec<DirEntry> = Vec::new();
    let mut tx = catalog.tx()?;
    let mut writer = tx.scan_writer(scan_id, root)?;
    let mut in_tx = 0;
    let mut complete = true;
    ctl.begin(Phase::Scan { root: root_name.clone() }, 0);

    while let Some(frame) = stack.pop() {
        // A stop ends the walk here: nothing is marked missing and the scan's `finished` stays NULL. The pairing
        // still runs, on what was seen (tier 1 only: it needs no whole tree).
        if !ctl.checkpoint() {
            complete = false;
            break;
        }
        let dir = match frame.dir {
            Some(dir) => dir,
            None => match open_folder(lister, rules, &frame.path) {
                Ok(dir) => dir,
                Err(e) => {
                    keep_failure(&mut stats.failures, rules.display(&frame.path), format!("{e:#}"));
                    stats.errors += 1;
                    unreadable.push(frame.path);
                    continue;
                }
            },
        };
        let listed = match list(lister, &dir, root_volume, &mut entries) {
            Ok(n) => n,
            Err(e) => {
                keep_failure(&mut stats.failures, rules.display(&frame.path), format!("{e:#}"));
                stats.errors += 1;
                unreadable.push(frame.path);
                continue;
            }
        };
        stats.folders += 1;
        let mut folder = None;
        for e in entries.iter().take(listed) {
            let identity = FileIdentity::new(e.volume, e.file_id);
            if e.reparse {
                stats.skipped_reparse += 1;
                match e.ext {
                    // A reparse folder is not descended into (it may lead anywhere, and its listing may be a download).
                    None => unreadable.push(child_path(&frame.path, &e.name)),
                    // A reparse file keeps its row if it has one: its listing gives no content, so none is queued.
                    Some(_) => {
                        writer.seen_identity(identity)?;
                        in_tx += 1;
                    }
                }
                continue;
            }
            let Some(ext) = e.ext else {
                if skip_dirs(rules).iter().any(|s| e.name.as_bytes().eq_ignore_ascii_case(s.as_bytes())) {
                    continue;
                }
                let key = (e.volume, e.file_id);
                if frame.chain.contains(&key) {
                    stats.loops += 1;
                    continue;
                }
                let mut chain = frame.chain.clone();
                chain.push(key);
                stack.push(Frame { path: child_path(&frame.path, &e.name), chain, dir: None });
                continue;
            };
            stats.files += 1;
            if !seen.insert(identity) {
                stats.duplicates += 1;
                continue;
            }
            if folder.is_none() {
                folder = Some(writer.folder(&frame.path)?);
            }
            let Some(folder) = &folder else { unreachable!("set above") };
            let recorded = writer.upsert_file(
                folder,
                &ScannedFile { identity, name: &e.name, ext, size: e.size, mtime: e.mtime, created: e.created },
            )?;
            match recorded {
                Recorded::Same => {}
                Recorded::Changed => stats.changed += 1,
                Recorded::Foreign => stats.duplicates += 1,
            }
            in_tx += 1;
        }
        if in_tx >= BATCH {
            drop(writer);
            tx.commit()?;
            tx = catalog.tx()?;
            writer = tx.scan_writer(scan_id, root)?;
            in_tx = 0;
        }
        ctl.set_done(stats.files);
    }
    drop(writer);
    tx.commit()?;
    // The pairing needs the whole tree for tiers 2 and replacement, so it runs them only when nothing was skipped.
    let clean = complete && unreadable.is_empty();
    let pairing = catalog.pair(root, scan_id, clean)?;
    stats.rebound = pairing.rebound;
    stats.replaced = pairing.replaced;
    stats.ambiguous = pairing.ambiguous;
    stats.unpaired_with_data = pairing.unpaired_with_data;
    stats.replaced_ids = pairing.replaced_ids;
    stats.temps = pairing.temps;
    if complete {
        stats.missing = catalog.mark_missing(root, scan_id, &unreadable)?;
        catalog.finish_scan(scan_id)?;
    }
    stats.complete = complete;
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

    /// One entry of a fake folder.
    #[derive(Clone)]
    struct FakeEntry {
        name: &'static str,
        folder: bool,
        reparse: bool,
        file_id: u64,
        size: u64,
        volume: Option<u64>,
    }

    /// A file entry, on the folder's volume.
    fn file(name: &'static str, file_id: u64, size: u64) -> FakeEntry {
        FakeEntry { name, folder: false, reparse: false, file_id, size, volume: None }
    }

    /// A folder entry.
    fn folder(name: &'static str, file_id: u64) -> FakeEntry {
        FakeEntry { name, folder: true, reparse: false, file_id, size: 0, volume: None }
    }

    /// Folders by their units, as the walk opens them: each one's entries.
    struct FakeListing {
        tree: Vec<(Vec<Unit>, Vec<FakeEntry>)>,
        root_volume: u64,
    }

    impl FakeListing {
        fn new(root_volume: u64, tree: &[(&str, Vec<FakeEntry>)]) -> FakeListing {
            let tree =
                tree.iter().map(|(rel, entries)| (units_of(fake_path(rel).as_bytes()), entries.clone())).collect();
            FakeListing { tree, root_volume }
        }
    }

    impl Listing for FakeListing {
        type Dir = usize;

        fn open(&mut self, path: &[Unit]) -> io::Result<usize> {
            self.tree
                .iter()
                .position(|(p, _)| p == path)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such fake folder"))
        }

        fn volume(&mut self, _dir: &usize) -> io::Result<u64> {
            Ok(self.root_volume)
        }

        fn each(&mut self, dir: &usize, mut each: impl FnMut(Entry<'_>)) -> io::Result<()> {
            for e in &self.tree[*dir].1 {
                let units = units_of(e.name.as_bytes());
                each(Entry {
                    name: &units,
                    folder: e.folder,
                    reparse: e.reparse,
                    file_id: e.file_id,
                    size: e.size,
                    mtime: 1,
                    volume: e.volume,
                    created: None,
                });
            }
            Ok(())
        }
    }

    /// The root path of the fake tree, under the rules the tests catalog uses.
    fn fake_root() -> RulePath {
        PathRules::NATIVE.parse(if cfg!(windows) { r"D:\x" } else { "/x" }).unwrap()
    }

    /// The text of a folder of the fake tree, `rel` under its root (`""` is the root), as the walk opens it.
    fn fake_path(rel: &str) -> String {
        let root = if cfg!(windows) { r"D:\x" } else { "/x" };
        match (rel.is_empty(), cfg!(windows)) {
            (true, _) => root.to_string(),
            (false, true) => format!(r"{root}\{rel}"),
            (false, false) => format!("{root}/{rel}"),
        }
    }

    fn versions() -> pg_catalog::Versions {
        pg_catalog::Versions { import: 1, borders: 1 }
    }

    /// One scan of the fake tree into a catalog.
    fn scan_fake(catalog: &mut Catalog, tree: &[(&str, Vec<FakeEntry>)], root_volume: u64) -> ScanStats {
        let root = fake_root();
        let mut lister = FakeListing::new(root_volume, tree);
        walk(catalog, &root, &Control::new(), &mut lister).unwrap()
    }

    fn catalog() -> Catalog {
        Catalog::prepare(Path::new(":memory:"), versions(), PathRules::NATIVE).unwrap()
    }

    #[test]
    fn an_entry_with_no_file_id_makes_its_folder_unreadable() {
        // The folder's other entries are not recorded either: the listing fails as a whole (decision 10).
        let mut lister = FakeListing::new(1, &[("", vec![file("a.jpg", 7, 1), file("b.jpg", 0, 1)])]);
        let mut entries = Vec::new();
        assert!(list(&mut lister, &0, 1, &mut entries).is_err());
    }

    #[test]
    fn the_dot_entries_may_have_no_file_id() {
        let mut lister = FakeListing::new(1, &[("", vec![file(".", 0, 1), file("..", 0, 1), file("a.jpg", 7, 1)])]);
        let mut entries = Vec::new();
        assert_eq!(list(&mut lister, &0, 1, &mut entries).unwrap(), 1);
    }

    #[test]
    fn dot_folders_are_not_listed_and_media_is_recognised_in_any_case() {
        assert!(name_from_native(&units_of(b".")).is_dot());
        assert!(name_from_native(&units_of(b"..")).is_dot());
        assert!(!name_from_native(&units_of(b"...")).is_dot());
        assert!(Ext::of_name_bytes(b"a.mozjpeg.JPG").is_some());
    }

    #[test]
    fn a_reparse_file_keeps_its_row_and_gets_no_content() {
        // a.jpg is recorded; then it is a cloud placeholder (a reparse point) of another size. Its row stays, is seen
        // by the second scan, and keeps the size it had (nothing is read from the placeholder). b.jpg is new and a
        // placeholder: it gets no row at all.
        let mut c = catalog();
        scan_fake(&mut c, &[("", vec![file("a.jpg", 7, 1)])], 1);
        let placeholder = FakeEntry { reparse: true, ..file("a.jpg", 7, 99) };
        let stats =
            scan_fake(&mut c, &[("", vec![placeholder, FakeEntry { reparse: true, ..file("b.jpg", 8, 5) }])], 1);
        assert_eq!(stats.skipped_reparse, 2);
        assert_eq!(stats.missing, 0);
        let status = c.status().unwrap();
        assert_eq!((status.files, status.missing), (1, 0));
    }

    #[test]
    fn a_reparse_folder_is_not_descended_and_its_rows_are_kept() {
        let mut c = catalog();
        scan_fake(&mut c, &[("", vec![folder("sub", 20)]), ("sub", vec![file("a.jpg", 9, 1)])], 1);
        let link = FakeEntry { reparse: true, ..folder("sub", 20) };
        let stats = scan_fake(&mut c, &[("", vec![link]), ("sub", vec![])], 1);
        assert_eq!(stats.skipped_reparse, 1);
        // The subtree was not listed, so its file is not missing.
        assert_eq!(stats.missing, 0);
        assert_eq!(c.status().unwrap().missing, 0);
    }

    #[test]
    fn a_folder_already_on_the_path_is_skipped_and_counted() {
        // "loop" (volume 1, file 20) holds an entry with the same identity: a bind-mount cycle.
        let mut c = catalog();
        let stats = scan_fake(&mut c, &[("", vec![folder("loop", 20)]), ("loop", vec![folder("again", 20)])], 1);
        assert_eq!(stats.loops, 1);
        assert_eq!(stats.folders, 2);
    }

    #[test]
    fn a_second_path_to_a_file_is_skipped_and_counted_and_the_row_does_not_flip() {
        // A hard link: the same identity under the root and under sub. The first path (the walk's order) keeps it.
        let mut c = catalog();
        let tree = [("", vec![file("a.jpg", 7, 1), folder("sub", 20)]), ("sub", vec![file("b.jpg", 7, 1)])];
        let first = scan_fake(&mut c, &tree, 1);
        assert_eq!(first.duplicates, 1);
        let second = scan_fake(&mut c, &tree, 1);
        assert_eq!(second.duplicates, 1);
        assert_eq!(second.changed, 0, "the same path wins on every scan");
        assert_eq!(c.status().unwrap().files, 1);
    }

    #[test]
    fn an_entry_with_no_file_id_in_the_walk_keeps_its_siblings_unchanged() {
        let mut c = catalog();
        scan_fake(&mut c, &[("", vec![file("a.jpg", 7, 1)])], 1);
        let mut lister = FakeListing::new(1, &[("", vec![file("a.jpg", 7, 9), file("b.jpg", 0, 1)])]);
        let stats = walk(&mut c, &fake_root(), &Control::new(), &mut lister).unwrap();
        assert_eq!(stats.errors, 1, "the folder is unreadable");
        assert_eq!(stats.missing, 0);
    }
}
