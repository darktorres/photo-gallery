//! Scanner: walks a folder tree and records every media file's identity, size and mtime in the catalog (which
//! files are media: `media::Ext::of_name`).
//!
//! The listing is the platform's (`pg_platform::dir::Lister`): each entry's identity comes with its size and times,
//! without opening the files (on Windows the NTFS file ID, whose record number the import sorts by; on Linux the inode
//! on the volume's UUID). The identity is what keeps a moved or renamed file's analysis. Which folders are taken is
//! `refusal`'s, from the platform's facts.
//!
//! Names and folders are exact: a listed name is a [`Name`] (its units as the system gave them), a folder is a
//! [`RulePath`] that the walk extends by its names, and a folder is opened by its units. Nothing is decoded lossily.
//!
//! Identity and pairing (S-17.7): a file is recorded by its volume and file ID; a file the catalog has no row for is
//! paired at the end of the scan (`pg_catalog::Catalog::pair_after_scan`), which is what makes a renumbered volume or a file moved
//! while the app was closed keep its ratings.

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use anyhow::{Context, Result};
use pg_catalog::pair::{self, Coverage, Pairing};
use pg_catalog::{Catalog, Recorded, ScannedFile};
use pg_model::ids::FileIdentity;
use pg_model::media::Ext;
use pg_model::name::Name;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::dir::{Entry, Lister};
use pg_platform::native::Unit;

use pg_jobs::control::Control;
use pg_jobs::failure::{FAILURES_KEPT, keep_failure};
use pg_model::notice::FileFailure;
use pg_model::progress::Phase;
use pg_model::refusal::RefusedReason;
use pg_platform::names::{name_from_native, units_of};

mod refusal;

/// The folder listing a scan reads with: the platform's [`Lister`], or a fake in the tests.
pub trait Listing {
    type Dir;
    fn open(&mut self, path: &[Unit]) -> io::Result<Self::Dir>;
    /// The volume the folder is on: the one its entries are on, where the platform has one volume per folder
    /// (Windows). An entry that carries its own volume (Unix) uses that instead.
    fn volume(&mut self, dir: &Self::Dir) -> io::Result<u64>;
    fn each(&mut self, dir: &Self::Dir, each: impl FnMut(Entry<'_>)) -> io::Result<()>;
    /// Whether the folder at `path`, on a device other than the root's, may be walked into: the reason it may not, if not.
    fn check_mount(&mut self, path: &[Unit]) -> Result<(), RefusedReason>;
    /// How many devices were given their device number as their identity (no UUID, or the UUID of another device).
    fn fallback_volumes(&self) -> u64;
}

impl Listing for Lister {
    type Dir = pg_platform::dir::Dir;

    fn open(&mut self, path: &[Unit]) -> io::Result<Self::Dir> {
        Lister::open(self, path)
    }

    fn volume(&mut self, dir: &Self::Dir) -> io::Result<u64> {
        Lister::folder_volume(self, dir)
    }

    fn each(&mut self, dir: &Self::Dir, each: impl FnMut(Entry<'_>)) -> io::Result<()> {
        Lister::each(self, dir, each)
    }

    fn check_mount(&mut self, path: &[Unit]) -> Result<(), RefusedReason> {
        refusal::check_mount(path)
    }

    fn fallback_volumes(&self) -> u64 {
        Lister::fallback_volumes(self)
    }
}

/// A folder name the walk does not descend into.
struct Skip {
    /// The name, or its start when `prefix` is set (`.Trash-` matches `.Trash-1000`).
    name: &'static str,
    prefix: bool,
    /// Whether ASCII case is ignored in the comparison (NTFS names are case-insensitive; Posix names are exact).
    ignore_case: bool,
}

impl Skip {
    fn matches(&self, name: &[u8]) -> bool {
        let want = self.name.as_bytes();
        let head = if self.prefix { name.get(..want.len()) } else { (name.len() == want.len()).then_some(name) };
        head.is_some_and(|head| if self.ignore_case { head.eq_ignore_ascii_case(want) } else { head == want })
    }
}

/// Skipped under every rule set, without case: the NTFS system folders, which a drive Windows has used carries. Mounted on
/// Linux, their deleted files would be walked as the library's.
const SKIP_ANY: &[Skip] = &[
    Skip { name: "$RECYCLE.BIN", prefix: false, ignore_case: true },
    Skip { name: "System Volume Information", prefix: false, ignore_case: true },
];

/// Skipped on Windows, without case (NTFS names).
const SKIP_WINDOWS: &[Skip] = &[Skip { name: "_quarantine", prefix: false, ignore_case: true }];

/// Skipped on Posix, exactly: the trash, cache, thumbnail and system folders of the usual file managers and NAS systems
/// (`Pics` and `pics` are different folders there).
const SKIP_POSIX: &[Skip] = &[
    Skip { name: "lost+found", prefix: false, ignore_case: false },
    Skip { name: ".Trash", prefix: false, ignore_case: false },
    Skip { name: ".Trash-", prefix: true, ignore_case: false },
    Skip { name: ".Trashes", prefix: false, ignore_case: false },
    Skip { name: ".cache", prefix: false, ignore_case: false },
    Skip { name: ".thumbnails", prefix: false, ignore_case: false },
    Skip { name: "@eaDir", prefix: false, ignore_case: false },
    Skip { name: "_quarantine", prefix: false, ignore_case: false },
];

/// Whether the walk does not descend into a folder named `name` under `rules`.
fn skipped_name(rules: PathRules, name: &[u8]) -> bool {
    let own = match rules {
        PathRules::Windows => SKIP_WINDOWS,
        PathRules::Posix => SKIP_POSIX,
    };
    SKIP_ANY.iter().chain(own).any(|skip| skip.matches(name))
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
    pub failures: Vec<FileFailure>,
    /// Paths of the same file (a hard link, or a row of another root) that were skipped, not recorded (decision 5).
    pub duplicates: u64,
    /// Reparse points indexed without following them: a file's row is kept, a folder's subtree is kept (decision 9).
    pub skipped_reparse: u64,
    /// Folders not descended into because they were already on the path (a bind-mount cycle, decision 11).
    pub loops: u64,
    /// Folders under the root on another device whose file system is refused (a network or pseudo file system mounted under
    /// a local root). Their files are not seen, so their rows are left as they are.
    pub refused_mounts: u64,
    /// The first of those folders, with the reason (`FAILURES_KEPT`).
    pub refused_mount_list: Vec<RefusedMount>,
    /// Devices whose identity is their device number, not a file system's UUID: their identities can change at a restart.
    pub volume_fallbacks: u64,
    /// What the pairing did: its counts, and the replaced rows and dropped temp files the caller acts on.
    pub pairing: Pairing,
}

/// A folder under a scanned root that the scan didn't walk into, because its file system is refused: the folder, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedMount {
    pub mount: RulePath,
    pub reason: RefusedReason,
}

/// Why a scan didn't run: the folder is refused (before anything is recorded), or the scan failed (its batches before
/// the error are in).
#[derive(Debug)]
pub enum ScanError {
    Refused(RefusedReason),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for ScanError {
    fn from(e: anyhow::Error) -> ScanError {
        ScanError::Failed(e)
    }
}

/// A log's or the command line's text: a refusal by its code (its words are the client's), a failure with its causes.
impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::Refused(reason) => write!(f, "refused: {reason}"),
            ScanError::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// Whether the folder `root` may be scanned for its identities (decisions 6 and 13): the reason it may not, if not. The
/// scan makes this check first, and so does adding a folder to the library.
pub fn check_folder(rules: PathRules, root: &RulePath) -> Result<(), RefusedReason> {
    refusal::check_root(&units_of(&rules.join(root)))
}

/// Record the media files under `root` (a folder of the catalog's rules); files it no longer holds are marked missing.
/// A folder that [`check_folder`] refuses is [`ScanError::Refused`], before anything is recorded.
pub fn scan(catalog: &mut Catalog, root: &RulePath, ctl: &Control) -> Result<ScanStats, ScanError> {
    // A refused root writes no scans row, and nothing is marked missing.
    check_folder(catalog.rules(), root).map_err(ScanError::Refused)?;
    Ok(walk(catalog, root, ctl, &mut Lister::new())?)
}

/// The folder being walked: its path, the folders it is under (with itself last) as `(volume, file ID)`, which a
/// folder met again is looked for in (a cycle), and its listing when it is already open.
struct Frame<D> {
    path: RulePath,
    chain: Vec<(u64, u64)>,
    dir: Option<D>,
    /// The device the folder is on: a folder on another one is a mount, checked before it is walked into.
    volume: u64,
}

/// The walk, over any listing: [`scan`] after its check.
fn walk<L: Listing>(catalog: &mut Catalog, root: &RulePath, ctl: &Control, lister: &mut L) -> Result<ScanStats> {
    let rules = catalog.rules();
    let root_name = rules.display(root);
    let root_dir = open_folder(lister, rules, root).with_context(|| format!("can't open {root_name}"))?;
    let root_volume = lister.volume(&root_dir)?;
    let scan_id = catalog.begin_scan(root)?;
    let mut stats = ScanStats::default();
    let mut stack = vec![Frame { path: root.clone(), chain: Vec::new(), dir: Some(root_dir), volume: root_volume }];
    // The folders this scan didn't list: those it couldn't read, the reparse folders it doesn't follow, and the mounts it
    // refuses. The files under them weren't seen, so they aren't marked missing, and the pairing leaves them out.
    let mut skipped: Vec<RulePath> = Vec::new();
    // The verdict on each device a folder under the root is on, by its first folder (the reason, if refused): one check
    // per mount, not per folder.
    let mut mounts: BTreeMap<u64, Result<(), RefusedReason>> = BTreeMap::new();
    // Each identity recorded or seen this scan: the second path to a file is skipped (decision 5).
    let mut seen: BTreeSet<FileIdentity> = BTreeSet::new();
    let mut entries: Vec<DirEntry> = Vec::new();
    let mut tx = catalog.tx()?;
    let mut writer = tx.scan_writer(scan_id, root)?;
    let mut in_tx = 0;
    let mut complete = true;
    // The phase ends when this is dropped, however the walk leaves.
    let _phase = ctl.begin(Phase::Scan { root: root.clone() }, 0);

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
                    skipped.push(frame.path);
                    continue;
                }
            },
        };
        let listed = match list(lister, &dir, root_volume, &mut entries) {
            Ok(n) => n,
            Err(e) => {
                keep_failure(&mut stats.failures, rules.display(&frame.path), format!("{e:#}"));
                stats.errors += 1;
                skipped.push(frame.path);
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
                    None => skipped.push(frame.path.child(&e.name)),
                    // A reparse file keeps its row if it has one: its listing gives no content, so none is queued.
                    Some(_) => {
                        writer.seen_identity(identity)?;
                        in_tx += 1;
                    }
                }
                continue;
            }
            let Some(ext) = e.ext else {
                if skipped_name(rules, e.name.as_bytes()) {
                    continue;
                }
                let key = (e.volume, e.file_id);
                if frame.chain.contains(&key) {
                    stats.loops += 1;
                    continue;
                }
                let child = frame.path.child(&e.name);
                if e.volume != frame.volume {
                    let verdict = match mounts.get(&e.volume) {
                        Some(verdict) => verdict.clone(),
                        None => {
                            let verdict = lister.check_mount(&units_of(&rules.join(&child)));
                            mounts.insert(e.volume, verdict.clone());
                            verdict
                        }
                    };
                    if let Err(reason) = verdict {
                        stats.refused_mounts += 1;
                        if stats.refused_mount_list.len() < FAILURES_KEPT {
                            stats.refused_mount_list.push(RefusedMount { mount: child.clone(), reason });
                        }
                        skipped.push(child);
                        continue;
                    }
                }
                let mut chain = frame.chain.clone();
                chain.push(key);
                stack.push(Frame { path: child, chain, dir: None, volume: e.volume });
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
    // The pairing needs the tree looked through for tiers 2 and replacement, so it runs them only on a complete walk,
    // and outside the folders it skipped.
    let pairing = catalog.pair_after_scan(root, scan_id, Coverage { complete, skipped: &skipped })?;
    stats.pairing = pairing;
    if complete {
        stats.missing = catalog.mark_missing(root, scan_id, &skipped)?;
        catalog.finish_scan(scan_id)?;
    }
    stats.complete = complete;
    stats.volume_fallbacks = lister.fallback_volumes();
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_platform::native::Unit;
    use std::path::Path;

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
        /// Folders (by their units) whose mount is refused.
        refused: Vec<Vec<Unit>>,
    }

    impl FakeListing {
        fn new(root_volume: u64, tree: &[(&str, Vec<FakeEntry>)]) -> FakeListing {
            FakeListing::under(pg_platform::native::PATH_RULES, root_volume, tree)
        }

        /// The fake tree with its folders spelled under `rules`.
        fn under(rules: PathRules, root_volume: u64, tree: &[(&str, Vec<FakeEntry>)]) -> FakeListing {
            let tree = tree
                .iter()
                .map(|(rel, entries)| (units_of(fake_path(rules, rel).as_bytes()), entries.clone()))
                .collect();
            FakeListing { tree, root_volume, refused: Vec::new() }
        }

        /// The same tree, with the mount at `rel` (spelled under `rules`) refused by its file system.
        fn refusing(mut self, rules: PathRules, rel: &str) -> FakeListing {
            self.refused.push(units_of(fake_path(rules, rel).as_bytes()));
            self
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

        fn fallback_volumes(&self) -> u64 {
            0
        }

        fn check_mount(&mut self, path: &[Unit]) -> Result<(), RefusedReason> {
            if self.refused.iter().any(|p| p == path) {
                return Err(RefusedReason::Nfs);
            }
            Ok(())
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
        fake_root_under(pg_platform::native::PATH_RULES)
    }

    /// The root path of the fake tree under `rules`.
    fn fake_root_under(rules: PathRules) -> RulePath {
        rules.parse(&fake_path(rules, "")).unwrap()
    }

    /// The text of a folder of the fake tree under `rules`, `rel` under its root (`""` is the root; its parts are
    /// separated by `/`), as the walk opens it.
    fn fake_path(rules: PathRules, rel: &str) -> String {
        match (rel.is_empty(), rules) {
            (true, PathRules::Windows) => r"D:\x".to_string(),
            (true, PathRules::Posix) => "/x".to_string(),
            (false, PathRules::Windows) => format!(r"D:\x\{}", rel.replace('/', r"\")),
            (false, PathRules::Posix) => format!("/x/{rel}"),
        }
    }

    fn versions() -> pg_catalog::Versions {
        pg_catalog::Versions { import: 1, borders: 1, thumbs: 1 }
    }

    /// One scan of the fake tree into a catalog, under the catalog's rules.
    fn scan_fake(catalog: &mut Catalog, tree: &[(&str, Vec<FakeEntry>)], root_volume: u64) -> ScanStats {
        let rules = catalog.rules();
        let root = fake_root_under(rules);
        let mut lister = FakeListing::under(rules, root_volume, tree);
        walk(catalog, &root, &Control::new(), &mut lister).unwrap()
    }

    fn catalog() -> Catalog {
        Catalog::prepare(Path::new(":memory:"), versions(), pg_platform::native::PATH_RULES).unwrap()
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
    fn a_renumbered_volume_rebinds_every_row_and_loses_none() {
        // The same tree on volume 1, then on volume 2 (a device renumbered at a restart): every file is found again under its
        // new identity, none is ambiguous, and none is missing.
        let mut c = catalog();
        let tree = [("", vec![file("a.jpg", 7, 1), folder("sub", 20)]), ("sub", vec![file("b.jpg", 8, 2)])];
        scan_fake(&mut c, &tree, 1);
        let stats = scan_fake(&mut c, &tree, 2);
        assert_eq!((stats.pairing.rebound, stats.pairing.ambiguous, stats.pairing.unpaired_with_data, stats.missing), (2, 0, 0, 0));
        assert_eq!(c.status().unwrap().files, 2);
    }

    #[test]
    fn a_complete_scan_with_a_reparse_folder_still_pairs_the_rest_of_the_root() {
        // c.jpg (file 5) in the root, a.jpg in sub, on volume 1. Then the root is on volume 2, sub is a link, and c.jpg
        // is d.jpg (moved while the app was closed): tier 2 pairs it, though the walk skipped sub, and sub's file is
        // kept as it was (neither missing nor a candidate).
        let mut c = catalog();
        let before = [("", vec![file("c.jpg", 5, 3), folder("sub", 20)]), ("sub", vec![file("a.jpg", 9, 1)])];
        scan_fake(&mut c, &before, 1);
        let link = FakeEntry { reparse: true, ..folder("sub", 20) };
        let stats = scan_fake(&mut c, &[("", vec![file("d.jpg", 5, 3), link]), ("sub", vec![])], 2);
        assert!(stats.complete);
        assert_eq!((stats.skipped_reparse, stats.pairing.rebound, stats.pairing.ambiguous), (1, 1, 0));
        assert_eq!(stats.missing, 0);
        assert_eq!(c.status().unwrap().missing, 0);
    }

    #[test]
    fn an_unreadable_folder_is_skipped_and_the_rest_of_the_root_is_paired() {
        // As above, with sub unreadable (the fake has no listing for it): the same pairing, and one error.
        let mut c = catalog();
        let before = [("", vec![file("c.jpg", 5, 3), folder("sub", 20)]), ("sub", vec![file("a.jpg", 9, 1)])];
        scan_fake(&mut c, &before, 1);
        let stats = scan_fake(&mut c, &[("", vec![file("d.jpg", 5, 3), folder("sub", 20)])], 2);
        assert!(stats.complete);
        assert_eq!((stats.errors, stats.pairing.rebound), (1, 1));
        assert_eq!(c.status().unwrap().missing, 0);
    }

    /// The keys the scan stores are the ones the grid looks folders up by (CODE_REVIEW C-23): a folder ending in Σ
    /// after a letter (`ΟΔΥΣΣΕΥΣ`, whose final sigma lowercases differently from a whole-word fold) lists its files,
    /// alone and with its subfolders, under both rules.
    fn scanned_folders_list_their_files_in_the_grid(rules: PathRules) {
        use pg_model::query::{Query, Sort};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let tree = [
            ("", vec![folder("ΑΣ", 20), folder("ΟΔΥΣΣΕΥΣ", 21)]),
            ("ΑΣ", vec![file("a.jpg", 1, 1)]),
            ("ΟΔΥΣΣΕΥΣ", vec![file("b.jpg", 2, 1), folder("child", 22)]),
            ("ΟΔΥΣΣΕΥΣ/child", vec![file("c.jpg", 3, 1)]),
        ];
        let stats = scan_fake(&mut c, &tree, 1);
        assert_eq!((stats.files, stats.errors), (3, 0));
        let names = |rel: &str, recursive: bool| -> Vec<String> {
            let folder = rules.merge(&rules.parse(&fake_path(rules, rel)).unwrap());
            let entries = c.list_grid(&folder, recursive, &Query::default(), Sort::default()).unwrap();
            entries
                .iter()
                .filter_map(|e| c.details(e.id).unwrap()?.path.parts.last().map(|n| n.display().into_owned()))
                .collect()
        };
        assert_eq!(names("ΑΣ", false), ["a.jpg"]);
        assert_eq!(names("ΟΔΥΣΣΕΥΣ", false), ["b.jpg"]);
        assert_eq!(names("ΟΔΥΣΣΕΥΣ", true), ["b.jpg", "c.jpg"]);
        assert_eq!(names("ΟΔΥΣΣΕΥΣ/child", false), ["c.jpg"]);
        assert_eq!(names("", true), ["a.jpg", "b.jpg", "c.jpg"]);
    }

    #[test]
    fn scanned_folders_list_their_files_in_the_grid_windows() {
        scanned_folders_list_their_files_in_the_grid(PathRules::Windows);
    }

    #[test]
    fn scanned_folders_list_their_files_in_the_grid_posix() {
        scanned_folders_list_their_files_in_the_grid(PathRules::Posix);
    }

    #[test]
    fn the_ntfs_system_folders_are_skipped_on_every_rule_set_without_case() {
        for rules in [PathRules::Windows, PathRules::Posix] {
            assert!(skipped_name(rules, b"$RECYCLE.BIN"), "{rules:?}");
            assert!(skipped_name(rules, b"$recycle.bin"), "{rules:?}");
            assert!(skipped_name(rules, b"system volume information"), "{rules:?}");
            assert!(!skipped_name(rules, b"System Volume Information2"), "{rules:?}");
        }
    }

    #[test]
    fn each_rule_set_skips_its_own_folders_and_the_posix_ones_exactly() {
        assert!(skipped_name(PathRules::Windows, b"_QUARANTINE"));
        assert!(!skipped_name(PathRules::Windows, b".cache"));
        assert!(!skipped_name(PathRules::Windows, b".Trash-1000"));
        assert!(skipped_name(PathRules::Posix, b".cache"));
        assert!(!skipped_name(PathRules::Posix, b".CACHE"), "Posix names are exact");
        assert!(skipped_name(PathRules::Posix, b".Trash-1000"), "the trash is a prefix");
        assert!(!skipped_name(PathRules::Posix, b".Trashy"));
        assert!(skipped_name(PathRules::Posix, b"lost+found"));
        assert!(!skipped_name(PathRules::Posix, b"Pics"));
        assert!(!skipped_name(PathRules::Posix, b"pics"));
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
    fn a_mount_refused_by_its_file_system_is_not_walked_and_its_rows_stay() {
        // "mnt" is on another device (volume 2) under the root. It is walked on the first scan; the second refuses it, and
        // then its file is neither walked nor marked missing (its folder was not seen, as an unreadable one is not).
        let mut c = catalog();
        let mnt = FakeEntry { name: "mnt", folder: true, reparse: false, file_id: 30, size: 0, volume: Some(2) };
        let tree = [("", vec![file("a.jpg", 7, 1), mnt]), ("mnt", vec![file("m.jpg", 8, 1)])];
        let first = walk(&mut c, &fake_root(), &Control::new(), &mut FakeListing::new(1, &tree)).unwrap();
        assert_eq!((first.refused_mounts, c.status().unwrap().files), (0, 2));
        let mut refusing = FakeListing::new(1, &tree).refusing(pg_platform::native::PATH_RULES, "mnt");
        let second = walk(&mut c, &fake_root(), &Control::new(), &mut refusing).unwrap();
        assert_eq!(second.refused_mounts, 1);
        assert!(
            second.refused_mount_list.first().is_some_and(|f| f
                .mount
                .parts
                .last()
                .is_some_and(|n| n == &Name::from("mnt"))
                && f.reason == RefusedReason::Nfs),
            "the mount, with its reason: {:?}",
            second.refused_mount_list
        );
        assert_eq!((second.missing, c.status().unwrap().files), (0, 2));
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
