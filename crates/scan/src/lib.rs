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
//! Identity and pairing: a file is recorded by its volume and file ID; a file the catalog has no row for is
//! paired at the end of the scan (`pg_catalog::ScanLease::pair_after_scan`), which is what makes a renumbered volume or a file moved
//! while the app was closed keep its ratings.
//!
//! The catalog's write lock is never held across the file system or a wait (the write-lock section of the module docs, W2). A folder is
//! listed outside any transaction, into the batch being built, with the catalog's rows for its files as they were
//! before its listing (`Catalog::folder_rows`, `Catalog::prior_states`); a full batch is written in one short write
//! transaction, which changes only the rows still as they were read (a write-back from the listing to the write is
//! kept). A pause or a stop writes the batch built
//! first, then waits with nothing held. Two walks of overlapping folders never interleave: a walk holds the scan lease
//! of its folder (`Catalog::begin_scan`), renewed with each batch and through a pause, and a walk refused one is
//! [`ScanError::Busy`].
//!
//! ## The scan and the write lock
//!
//! A walk never holds the catalog's write lock across the file system or a wait (`pg_scan`):
//!
//! - Each folder is listed outside any transaction, into the batch being built, with the catalog's `(size, mtime)` of its
//!   files as they were before the listing: the folder's rows are read just before it (`Catalog::folder_rows`), and a
//!   file whose row is elsewhere is looked up by identity after it, a row changed since the listing began counting as
//!   none (`Catalog::prior_states`: a write-back through a hard link's other name). A batch is written at a folder
//!   boundary once it holds `BATCH`
//!   (10 000) entries, in one short write transaction (folder rows, the files seen, the new files, the folder ledger's
//!   rows), and committed at once. A folder larger than a batch is held whole.
//! - The batch changes a file's row only where it still has the size and mtime that were read and no write of the app's
//!   into it is in progress (`not_being_written!`): a write-back committed between the listing and the batch is kept, and
//!   the file is only marked seen; the next walk lists its new state. Stamps such as `files.seq` are not compared, since
//!   imports, Retry and rotations bump them.
//! - A pause or a stop first writes the batch built, then waits with no transaction open. `mark_missing` still runs only at
//!   the end of a complete walk. Pairing (`pair_after_scan`) and `mark_missing` run in batches of at most 1000 rows, each in
//!   its own transaction.
//! - **Two walks of overlapping folders never interleave** (one is inside the other): a walk holds a **scan lease**
//!   (`scan_leases`, `Catalog::begin_scan`, which refuses when an unexpired lease covers an overlapping folder). The walk
//!   renews it with each batch and every 20 s while it is held up; one with no heartbeat for `SCAN_LEASE_SECS` (60 s) is a
//!   crashed process's, and expires. It is given back in `mark_missing`'s last transaction, or when the `ScanLease` is
//!   dropped (a stop, an error, a panic). A refused walk is `ScanError::Busy`: the job puts it back on its queue and tries
//!   again after 5 s (`BUSY_RETRY`); `gallery scan` says another process is scanning and fails, or waits with `--wait`.
//! - Workers list folders ahead of the walk (`worker`), each with a read-only connection of its own that reads a folder's
//!   rows and nothing else: they hold no lock, and their listings are consumed by the walk in walk order.
//! - **Every write transaction of a walk holds the lease first**: the batches', the pairing's and the missing marks'.
//!   Removing the library folder takes the leases of the walks under it, in the transaction that marks its files missing
//!   (`Catalog::remove_root`), in any process; the walk's next write then writes nothing, and the walk ends as
//!   `ScanError::Removed`. So nothing of a removed folder is written again, by the walk's last batch or by a rebind of
//!   its pairing.
//!
//! So the walk's write lock is held for one batch write's milliseconds, and a paused scan holds nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use pg_catalog::pair::{Coverage, Pairing};
use pg_catalog::{
    Catalog, FolderFailure, FolderRowReader, FolderRows, Prior, Recorded, ScanLease, ScannedFile, Skip, SkipReason,
};
use pg_model::ids::FileIdentity;
use pg_model::media::Ext;
use pg_model::name::Name;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::dir::{Entry, Lister};
use pg_platform::native::Unit;

use pg_jobs::control::{Checkpoint, Control};
use pg_jobs::failure::{FAILURES_KEPT, keep_failure};
use pg_jobs::panics;
use pg_model::notice::FileFailure;
use pg_model::progress::Phase;
use pg_model::refusal::RefusedReason;
use pg_platform::names::{name_from_native, units_of};
use pg_platform::priority::{self, Priority};

mod refusal;
#[cfg(feature = "bench")]
pub mod synthetic;

/// The folder listing a scan reads with: the platform's [`Lister`], or a fake in the tests. A listing runs on the walk's
/// thread, and on the worker threads that list folders ahead of it, each with a lister of its own ([`Listing::fresh`]).
pub trait Listing: Send {
    type Dir;
    fn open(&mut self, path: &[Unit]) -> io::Result<Self::Dir>;
    /// The device the folder is on, as its entries number theirs (`Entry::device`): none where every folder is on one
    /// volume (Windows).
    fn device(&self, dir: &Self::Dir) -> Option<u64>;
    /// The volume the folder is on: the root's. A folder below it has its volume from its entry (`volume_of_device`).
    fn volume(&mut self, dir: &Self::Dir) -> io::Result<u64>;
    fn each(&mut self, dir: &Self::Dir, each: impl FnMut(Entry<'_>)) -> io::Result<()>;
    /// The identity of device `device`, from `probe`, a path on it. Kept, so that a second device with the same identity
    /// is told apart. The walk calls it on its own thread, in the order it lists the folders: the identities are the
    /// same as a walk with no workers gives.
    fn volume_of_device(&mut self, device: u64, probe: &[Unit]) -> u64;
    /// Whether the folder at `path`, on a device other than the root's, may be walked into: the reason it may not, if not.
    fn check_mount(&mut self, path: &[Unit]) -> Result<(), RefusedReason>;
    /// How many devices were given their device number as their identity (no UUID, or the UUID of another device).
    fn fallback_volumes(&self) -> u64;
    /// A lister of its own for a worker thread. A worker only lists, so it starts with no device identities.
    fn fresh(&self) -> Self;
}

impl Listing for Lister {
    type Dir = pg_platform::dir::Dir;

    fn open(&mut self, path: &[Unit]) -> io::Result<Self::Dir> {
        Lister::open(self, path)
    }

    fn device(&self, dir: &Self::Dir) -> Option<u64> {
        Lister::device(self, dir)
    }

    fn volume(&mut self, dir: &Self::Dir) -> io::Result<u64> {
        Lister::folder_volume(self, dir)
    }

    fn each(&mut self, dir: &Self::Dir, each: impl FnMut(Entry<'_>)) -> io::Result<()> {
        Lister::each(self, dir, each)
    }

    fn volume_of_device(&mut self, device: u64, probe: &[Unit]) -> u64 {
        Lister::map_device(self, device, probe)
    }

    fn check_mount(&mut self, path: &[Unit]) -> Result<(), RefusedReason> {
        refusal::check_mount(path)
    }

    fn fallback_volumes(&self) -> u64 {
        Lister::fallback_volumes(self)
    }

    fn fresh(&self) -> Lister {
        Lister::new()
    }
}

/// A folder name the walk does not descend into.
struct NameRule {
    /// The name, or its start when `prefix` is set (`.Trash-` matches `.Trash-1000`).
    name: &'static str,
    prefix: bool,
    /// Whether ASCII case is ignored in the comparison (NTFS names are case-insensitive; Posix names are exact).
    ignore_case: bool,
}

impl NameRule {
    fn matches(&self, name: &[u8]) -> bool {
        let want = self.name.as_bytes();
        let head = if self.prefix { name.get(..want.len()) } else { (name.len() == want.len()).then_some(name) };
        head.is_some_and(|head| if self.ignore_case { head.eq_ignore_ascii_case(want) } else { head == want })
    }
}

/// Skipped under every rule set, without case: the NTFS system folders, which a drive Windows has used carries. Mounted on
/// Linux, their deleted files would be walked as the library's.
const SKIP_ANY: &[NameRule] = &[
    NameRule { name: "$RECYCLE.BIN", prefix: false, ignore_case: true },
    NameRule { name: "System Volume Information", prefix: false, ignore_case: true },
];

/// Skipped on Windows, without case (NTFS names).
const SKIP_WINDOWS: &[NameRule] = &[NameRule { name: "_quarantine", prefix: false, ignore_case: true }];

/// Skipped on Posix, exactly: the trash, cache, thumbnail and system folders of the usual file managers and NAS systems
/// (`Pics` and `pics` are different folders there).
const SKIP_POSIX: &[NameRule] = &[
    NameRule { name: "lost+found", prefix: false, ignore_case: false },
    NameRule { name: ".Trash", prefix: false, ignore_case: false },
    NameRule { name: ".Trash-", prefix: true, ignore_case: false },
    NameRule { name: ".Trashes", prefix: false, ignore_case: false },
    NameRule { name: ".cache", prefix: false, ignore_case: false },
    NameRule { name: ".thumbnails", prefix: false, ignore_case: false },
    NameRule { name: "@eaDir", prefix: false, ignore_case: false },
    NameRule { name: "_quarantine", prefix: false, ignore_case: false },
];

/// Whether the walk does not descend into a folder named `name` under `rules`.
fn skipped_name(rules: PathRules, name: &[u8]) -> bool {
    let own = match rules {
        PathRules::Windows => SKIP_WINDOWS,
        PathRules::Posix => SKIP_POSIX,
    };
    SKIP_ANY.iter().chain(own).any(|skip| skip.matches(name))
}

/// Entries per batch: a batch is written once it holds this many, at a folder's end (a folder is never split, so a
/// folder larger than this is one batch of its own).
const BATCH: usize = 10_000;

/// How often a walk renews its lease while it is held up (a pause, a slow listing): well inside
/// `pg_catalog::SCAN_LEASE_SECS`, so a live walk's lease never expires.
const RENEW_EVERY: Duration = Duration::from_secs(20);

/// A folder or a media file in a listing. Files the library doesn't hold aren't listed.
struct DirEntry {
    name: Name,
    /// `None` for a folder.
    ext: Option<Ext>,
    file_id: u64,
    size: u64,
    mtime: i64,
    /// A reparse point (a junction, a symlink, or a cloud placeholder's reparse data): never followed.
    reparse: bool,
    /// The device the entry is on as its listing numbers it (`Entry::device`); `None` on the folder's own volume.
    device: Option<u64>,
    created: Option<i64>,
}

/// Lists a folder's media files and subfolders. The platform gives the records; the choices are here: a record needs its
/// file ID, the dot folders are skipped, and only files the library holds and folders are kept. Reparse points are kept,
/// marked, so the walk can index them without following them. `beat` runs before each record, so a listing that takes
/// long can renew the walk's lease (or show that a worker's listing is moving).
fn list<L: Listing>(lister: &mut L, dir: &L::Dir, beat: &mut dyn FnMut()) -> io::Result<Vec<DirEntry>> {
    let mut entries = Vec::new();
    let mut no_id = false;
    lister.each(dir, |e| {
        beat();
        let name = name_from_native(e.name);
        // An entry with no file ID has no identity to keep its rows by. The folder is then unreadable, so its
        // entries are not recorded and its rows are not marked missing.
        if e.file_id == 0 && !name.is_dot() {
            no_id = true;
            return;
        }
        let ext = if e.folder { None } else { Ext::of_name_bytes(name.as_bytes()) };
        let kept = (e.folder && !name.is_dot()) || ext.is_some();
        if kept {
            entries.push(DirEntry {
                name,
                ext,
                file_id: e.file_id,
                size: e.size,
                mtime: e.mtime,
                reparse: e.reparse,
                device: e.device,
                created: e.created,
            });
        }
    })?;
    if no_id {
        return Err(io::Error::other("an entry has no file ID, so the folder's identities can't be read"));
    }
    Ok(entries)
}

/// Whether an entry is on a device other than its folder's: both devices are known, and they differ. Where the platform
/// has one volume per folder (no devices), none is.
fn other_device(e: &DirEntry, folder: Option<u64>) -> bool {
    matches!((e.device, folder), (Some(device), Some(own)) if device != own)
}

/// The folder at `path`, opened for listing: by its units, as the system takes them.
fn open_folder<L: Listing>(lister: &mut L, rules: PathRules, path: &RulePath) -> io::Result<L::Dir> {
    lister.open(&units_of(&rules.join(path)))
}

/// What a scan of a root did. The counts are for the report; `pairing`'s replaced rows and dropped temp files are for the
/// caller to act on after the scan's commit (the thumbnails and the temp files of the replaced files).
#[derive(Debug, Default)]
pub struct ScanStats {
    pub folders: u64,
    pub files: u64,
    /// Files new, moved, renamed, rewritten or back from missing.
    pub changed: u64,
    /// Files marked missing.
    pub missing: usize,
    /// Folders that couldn't be read (each is in the catalog's folder ledger).
    pub errors: u64,
    /// False when stopped early; nothing is marked missing then.
    pub complete: bool,
    /// The first folders that couldn't be read (`FAILURES_KEPT`).
    pub failures: Vec<FileFailure>,
    /// Paths of the same file (a hard link, or a row of another root) that were skipped, not recorded.
    pub duplicates: u64,
    /// Reparse points indexed without following them: a file's row is kept, a folder's subtree is kept.
    pub skipped_reparse: u64,
    /// Folders not descended into because they were already on the path (a bind-mount cycle).
    pub loops: u64,
    /// Folders under the root on another device whose file system is refused (a network or pseudo file system mounted under
    /// a local root). Their files are not seen, so their rows are left as they are. Each is in the folder ledger.
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

/// Why a scan didn't run, or didn't finish: the folder is refused (before any walk; the refusal is in the folder
/// ledger), a walk of an overlapping folder holds its lease (nothing was written: try again once it ends), the folder
/// was removed from the library while it was walked (nothing more was written: there is nothing to try again), or the
/// scan failed (its batches before the error are in).
#[derive(Debug)]
pub enum ScanError {
    Refused(RefusedReason),
    /// Another walk of `root` (a folder that overlaps the one asked for) is running; `holder` says whose
    /// (`pg_catalog::Error::ScanBusy`).
    Busy {
        root: String,
        holder: String,
    },
    /// The library folder that holds `root` was removed while the walk ran, in this process or another
    /// (`pg_catalog::Error::RootRemoved`): the removal took the walk's lease, and the walk's next write found it gone
    /// and wrote nothing. Its batches before the removal are in, and are marked missing with the folder's other files.
    Removed {
        root: String,
    },
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
            ScanError::Busy { root, holder } => write!(f, "a scan of {root} is running ({holder})"),
            ScanError::Removed { root } => write!(f, "{root} was removed from the library while it was scanned"),
            ScanError::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// Whether the folder `root` may be scanned for its identities: the reason it may not, if not. The
/// scan makes this check first, and so does adding a folder to the library.
pub fn check_folder(rules: PathRules, root: &RulePath) -> Result<(), RefusedReason> {
    refusal::check_root(&units_of(&rules.join(root)))
}

/// The volume `root` (a library folder) is on, as the walk stores its files' volume (`Listing::volume` of the folder,
/// the same identity the walk gives its files). A lane that walks `root` owns this volume. An error when the folder
/// can't be opened.
pub fn root_volume(rules: PathRules, root: &RulePath) -> io::Result<u64> {
    let mut lister = pg_platform::dir::Lister::new();
    let dir = open_folder(&mut lister, rules, root)?;
    Listing::volume(&mut lister, &dir)
}

/// Record the media files under `root` (a folder of the catalog's rules); files it no longer holds are marked missing.
/// A folder that [`check_folder`] refuses is [`ScanError::Refused`], before any walk: no scans row is written, nothing is
/// marked missing, and the refusal goes in the folder ledger (`Catalog::folder_refused`), which the next complete walk
/// of the folder, or of one above it, sweeps. A walk of an overlapping folder running is [`ScanError::Busy`].
pub fn scan(catalog: &mut Catalog, root: &RulePath, ctl: &Control) -> Result<ScanStats, ScanError> {
    if let Err(reason) = check_folder(catalog.rules(), root) {
        let recorded = catalog
            .folder_refused(root, &reason.to_string())
            .with_context(|| format!("recording that {} is refused", catalog.rules().display(root)))?;
        if recorded {
            ctl.ledger_changed();
        }
        return Err(ScanError::Refused(reason));
    }
    walk(catalog, root, ctl, &mut Lister::new())
}

/// [`scan`]'s walk over any listing, without the refusal check (which reads the folder itself): for the benches, which
/// walk a synthetic tree ([`synthetic::SyntheticTree`]).
#[cfg(feature = "bench")]
pub fn scan_with<L: Listing>(
    catalog: &mut Catalog,
    root: &RulePath,
    ctl: &Control,
    lister: &mut L,
) -> Result<ScanStats, ScanError> {
    walk(catalog, root, ctl, lister)
}

/// A folder to list: its place in the walk, its path, its device and volume as the walk has them, and the folders it is
/// under (with itself last), as `(volume, file ID)`, which a folder met again is looked for in (a cycle).
///
/// The key orders the walk's folders in the order it consumes them. A folder's children are consumed after it and before
/// its earlier siblings, so comparing keys compares walk orders: a child's key is its parent's, then its place among the
/// parent's entries counted from the last (`child_job`).
struct Job {
    key: Key,
    path: RulePath,
    /// The folder's device as its listing numbers it (`None` where there is one volume per folder).
    device: Option<u64>,
    volume: u64,
    chain: Vec<(u64, u64)>,
}

/// A folder's place in the walk (see [`Job`]).
type Key = Vec<u64>;

/// A media file as a listing gave it, with the catalog's row for its identity as it was then (`prior`).
struct ListedFile {
    identity: FileIdentity,
    name: Name,
    ext: Ext,
    size: u64,
    mtime: i64,
    created: Option<i64>,
    prior: Option<Prior>,
}

/// A listed folder's files, in the batch.
struct ListedFolder {
    path: RulePath,
    files: Vec<ListedFile>,
}

/// What the walk listed and hasn't written yet: written together, in one write transaction ([`write_batch`]).
#[derive(Default)]
struct Batch {
    folders: Vec<ListedFolder>,
    /// Reparse files: their rows are only marked seen.
    reparse: Vec<FileIdentity>,
    /// The folders the walk couldn't take, for the folder ledger: why, and the error.
    failed: Vec<(RulePath, FolderFailure, String)>,
    /// The entries the batch holds (files and reparse files).
    entries: usize,
}

impl Batch {
    fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.reparse.is_empty() && self.failed.is_empty()
    }
}

/// A folder's listing: its entries in listing order, and the rows of its files read before the listing. `jobs` are the
/// folders below it on its own device, which the walk hands to the workers ([`descents`]).
struct Listed {
    rows: FolderRows,
    entries: Vec<DirEntry>,
    jobs: Vec<Job>,
}

/// What listing a folder gave: the listing, or how it failed.
enum Outcome {
    Listed(Listed),
    /// The folder can't be opened or listed: it is unreadable, and the walk records it.
    Unreadable(io::Error),
    /// The catalog couldn't be read for the folder: the walk fails.
    Failed(anyhow::Error),
    /// The listing panicked: the walk raises the panic again, with its payload.
    Panicked(Box<dyn std::any::Any + Send>),
}

impl Outcome {
    /// How much of the bound on the listing ahead a delivered outcome holds: its entries.
    fn weight(&self) -> usize {
        match self {
            Outcome::Listed(listed) => listed.entries.len() + 1,
            _ => 1,
        }
    }
}

/// A folder the walk has yet to consume: its job, its listing when it is already open (the root's), and whether a worker
/// lists it. A folder on its parent's device is handed out with the parent's listing; one on another device is listed by
/// the walk, once its mount is checked.
struct Frame<D> {
    job: Job,
    dir: Option<D>,
    pooled: bool,
}

/// The entries listed ahead of the walk, past which the workers take no more folders. A folder the walk needs that no
/// worker has taken, it lists itself ([`take`]).
const AHEAD: usize = 50_000;

/// How often the walk, waiting for a worker's listing, looks at its progress.
const POLL: Duration = Duration::from_millis(100);

/// The workers that list folders ahead of the walk, on a catalog that is a file (`walk_paced`).
const WORKERS: usize = 4;

/// The walk's pacing: its batch size, how often its lease is renewed, and how many workers list ahead of it.
#[derive(Clone, Copy)]
struct Pacing {
    batch: usize,
    renew_every: Duration,
    workers: usize,
}

/// The folders the workers list ahead of the walk, and what they have listed. Only the pool's own state is under its
/// lock: no catalog read or write is made under it.
struct Pool {
    state: Mutex<PoolState>,
    /// Signalled when a folder is added, a listing is delivered, the walk resumes, or the pool stops.
    changed: Condvar,
}

#[derive(Default)]
struct PoolState {
    /// The folders no worker has taken, in walk order.
    pending: BTreeMap<Key, Job>,
    /// The folders a worker is listing, with its progress (the entries it has listed).
    taken: BTreeMap<Key, Arc<AtomicU64>>,
    /// The listings delivered and not yet consumed by the walk.
    results: BTreeMap<Key, Outcome>,
    /// The weight of `results`.
    held: usize,
    stopped: bool,
}

impl Pool {
    fn new() -> Pool {
        Pool { state: Mutex::new(PoolState::default()), changed: Condvar::new() }
    }

    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Folders the workers may take, added by the walk.
    fn add(&self, jobs: Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        let mut st = self.lock();
        for job in jobs {
            st.pending.insert(job.key.clone(), job);
        }
        drop(st);
        self.changed.notify_all();
    }

    fn pending_len(&self) -> usize {
        self.lock().pending.len()
    }

    /// The next folder for a worker: the first in walk order that no worker has taken, once the walk isn't paused and
    /// the listings held are under [`AHEAD`]. Waits for that. `None` once the pool or the walk has stopped.
    fn claim(&self, ctl: &Control) -> Option<(Job, Arc<AtomicU64>)> {
        let mut st = self.lock();
        loop {
            if st.stopped || ctl.stop_requested() {
                return None;
            }
            if !ctl.is_paused()
                && st.held < AHEAD
                && let Some((key, job)) = st.pending.pop_first()
            {
                let progress = Arc::new(AtomicU64::new(0));
                st.taken.insert(key, Arc::clone(&progress));
                return Some((job, progress));
            }
            st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// A worker's listing of `key`, for the walk. The folders below it join the pending set. Dropped once the pool stopped.
    fn deliver(&self, key: &Key, mut outcome: Outcome) {
        let mut st = self.lock();
        st.taken.remove(key);
        if st.stopped {
            return;
        }
        if let Outcome::Listed(listed) = &mut outcome {
            for job in listed.jobs.drain(..) {
                st.pending.insert(job.key.clone(), job);
            }
        }
        st.held += outcome.weight();
        st.results.insert(key.clone(), outcome);
        drop(st);
        self.changed.notify_all();
    }

    /// Wakes the workers: the walk resumed, so they may take folders again.
    fn kick(&self) {
        self.changed.notify_all();
    }

    fn stop(&self) {
        self.lock().stopped = true;
        self.changed.notify_all();
    }
}

/// Stops the pool however the walk leaves (its end, an error, a panic): no worker takes another folder.
struct PoolStop<'a>(&'a Pool);

impl Drop for PoolStop<'_> {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// What the walk gets from the pool for a folder it comes to.
enum Taken {
    Listed(Outcome),
    /// No worker has taken the folder: the walk lists it itself.
    Inline,
    /// The walk was stopped while it waited.
    Stopped,
}

/// The listing of folder `key` as a worker makes it, waited for; or `Inline`, when no worker has taken it. While the walk
/// waits, the lease is renewed each `renew_every` in which the listing moved, so a listing that hangs lets the lease
/// expire, as the walk's own listing would (`list_here`).
fn take(
    pool: &Pool,
    key: &Key,
    ctl: &Control,
    lease: &ScanLease<'_>,
    renewed: &mut Instant,
    renew_every: Duration,
) -> Result<Taken> {
    let mut st = pool.lock();
    let mut renewed_at: Option<u64> = None;
    let mut checked = Instant::now();
    loop {
        if let Some(outcome) = st.results.remove(key) {
            st.held -= outcome.weight();
            return Ok(Taken::Listed(outcome));
        }
        if st.pending.remove(key).is_some() {
            return Ok(Taken::Inline);
        }
        let Some(progress) = st.taken.get(key).map(|p| p.load(Ordering::Relaxed)) else {
            bail!("a folder the walk needs was neither listed nor being listed");
        };
        if ctl.stop_requested() {
            return Ok(Taken::Stopped);
        }
        if checked.elapsed() >= renew_every {
            if renewed_at != Some(progress) {
                drop(st);
                renew(lease)?;
                *renewed = Instant::now();
                st = pool.lock();
                renewed_at = Some(progress);
            }
            checked = Instant::now();
        }
        st = pool.changed.wait_timeout(st, POLL).unwrap_or_else(PoisonError::into_inner).0;
    }
}

/// The job of child `e` (entry `i` of `n`) of folder `job`, on `volume`: its key (the parent's, then its place counted from
/// the last entry), its path, its device and its chain.
fn child_job(job: &Job, e: &DirEntry, i: usize, n: usize, volume: u64) -> Job {
    let mut key = job.key.clone();
    key.push(u64::try_from(n - 1 - i).unwrap_or(u64::MAX));
    let mut chain = job.chain.clone();
    chain.push((volume, e.file_id));
    Job { key, path: job.path.child(&e.name), device: e.device.or(job.device), volume, chain }
}

/// The folders of a listing a worker may list ahead of the walk: those on the folder's own device that the walk will
/// descend into. The walk makes the same choice when it consumes the listing (`walk_leased`), so each one is consumed.
fn descents(rules: PathRules, job: &Job, entries: &[DirEntry]) -> Vec<Job> {
    let n = entries.len();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.ext.is_none()
                && !e.reparse
                && !other_device(e, job.device)
                && !skipped_name(rules, e.name.as_bytes())
                && !job.chain.contains(&(job.volume, e.file_id))
        })
        .map(|(i, e)| child_job(job, e, i, n, job.volume))
        .collect()
}

/// Lists folder `job`, opened already as `dir` or opened here: its rows, read before the listing, and its entries. `beat`
/// runs with each entry. What the walk consumes.
fn unit<L: Listing>(
    lister: &mut L,
    rules: PathRules,
    job: &Job,
    dir: Option<L::Dir>,
    rows: &dyn Fn(&RulePath) -> anyhow::Result<FolderRows>,
    beat: &mut dyn FnMut(),
) -> Outcome {
    let dir = match dir {
        Some(dir) => dir,
        None => match open_folder(lister, rules, &job.path) {
            Ok(dir) => dir,
            Err(e) => return Outcome::Unreadable(e),
        },
    };
    let rows = match rows(&job.path) {
        Ok(rows) => rows,
        Err(e) => return Outcome::Failed(e),
    };
    match list(lister, &dir, beat) {
        Ok(entries) => {
            let jobs = descents(rules, job, &entries);
            Outcome::Listed(Listed { rows, entries, jobs })
        }
        Err(e) => Outcome::Unreadable(e),
    }
}

/// Hands the folders a listing holds below it to the pool, for the workers.
fn publish(pool: &Pool, outcome: &mut Outcome) {
    if let Outcome::Listed(listed) = outcome {
        pool.add(std::mem::take(&mut listed.jobs));
    }
}

/// The walk lists folder `job` itself, because no worker has taken it: renewing the lease as its entries come, and
/// publishing the folders below it. A lease that can't be renewed is the error.
#[expect(clippy::too_many_arguments, reason = "the walk's own state, passed beside the folder to list")]
fn list_here<L: Listing>(
    lease: &ScanLease<'_>,
    renewed: &mut Instant,
    renew_every: Duration,
    lister: &mut L,
    rules: PathRules,
    pool: &Pool,
    job: &Job,
    dir: Option<L::Dir>,
) -> Result<Outcome> {
    let mut lost = None;
    let mut outcome = {
        let mut beat = || {
            if lost.is_none() && renewed.elapsed() >= renew_every {
                match renew(lease) {
                    Ok(()) => *renewed = Instant::now(),
                    Err(e) => lost = Some(e),
                }
            }
        };
        let rows = |folder: &RulePath| -> anyhow::Result<FolderRows> {
            let _step = FOLDER_ROWS.timer();
            Ok(lease.folder_rows(folder)?)
        };
        unit(lister, rules, job, dir, &rows, &mut beat)
    };
    if let Some(e) = lost {
        return Err(e);
    }
    publish(pool, &mut outcome);
    Ok(outcome)
}

/// A worker thread's loop: it lists the folders the pool gives it, each with its rows read first, until the pool stops. A
/// listing that panics is delivered as such, for the walk to raise. A worker that can't open the catalog's file takes no
/// folders: the walk lists them itself.
fn worker<L: Listing>(lister: &mut L, pool: &Pool, ctl: &Control, file: &str, rules: PathRules) {
    priority::set_current_thread(Priority::BelowNormal);
    let Ok(reader) = FolderRowReader::open(file, rules) else {
        return;
    };
    while let Some((job, progress)) = pool.claim(ctl) {
        let key = job.key.clone();
        let rows = |folder: &RulePath| reader.folder_rows(folder).map_err(anyhow::Error::from);
        let mut beat = || {
            progress.fetch_add(1, Ordering::Relaxed);
        };
        let run = panics::catch(AssertUnwindSafe(|| unit(lister, rules, &job, None, &rows, &mut beat)));
        let outcome = match run {
            Ok(outcome) => outcome,
            Err(payload) => Outcome::Panicked(payload),
        };
        pool.deliver(&key, outcome);
    }
}

/// Starts a worker on the walk's scope, with its own lister.
fn spawn_worker<'scope, 'env, L: Listing + 'scope>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    pool: &'env Pool,
    ctl: &'env Control,
    file: &'env str,
    rules: PathRules,
    mut lister: L,
) {
    scope.spawn(move || worker(&mut lister, pool, ctl, file, rules));
}

/// The walk, over any listing: [`scan`] after its check. The root is opened and the lease taken; then the walk itself.
fn walk<L: Listing>(
    catalog: &mut Catalog,
    root: &RulePath,
    ctl: &Control,
    lister: &mut L,
) -> Result<ScanStats, ScanError> {
    walk_in_batches(catalog, root, ctl, lister, BATCH, RENEW_EVERY)
}

/// [`walk`], writing a batch once it holds `batch_size` entries ([`BATCH`]; the tests use small ones, to cross it), and
/// renewing the lease once `renew_every` has passed since the last renewal ([`RENEW_EVERY`]; the tests use a shorter one).
fn walk_in_batches<L: Listing>(
    catalog: &mut Catalog,
    root: &RulePath,
    ctl: &Control,
    lister: &mut L,
    batch_size: usize,
    renew_every: Duration,
) -> Result<ScanStats, ScanError> {
    walk_paced(catalog, root, ctl, lister, Pacing { batch: batch_size, renew_every, workers: WORKERS })
}

/// [`walk_in_batches`] with its pacing given whole: `pacing.workers` workers list folders ahead of the walk, each on a
/// read-only connection of its own. A catalog in memory has no file another connection can open, so its walk has none.
fn walk_paced<L: Listing>(
    catalog: &mut Catalog,
    root: &RulePath,
    ctl: &Control,
    lister: &mut L,
    pacing: Pacing,
) -> Result<ScanStats, ScanError> {
    // The scan's span in the perf log: its setup, its walk and its post-walk work (pairing, missing marks) are its
    // children, and the transaction time it caused goes on it.
    let mut pass = pg_perf::span("pass.scan");
    let tx_at = pg_catalog::tx_mark();
    let setup = pg_perf::span("scan.setup");
    let rules = catalog.rules();
    let root_dir = open_folder(lister, rules, root).with_context(|| format!("can't open {}", rules.display(root)))?;
    let root_device = lister.device(&root_dir);
    let root_volume = lister.volume(&root_dir).with_context(|| format!("the volume of {}", rules.display(root)))?;
    let mut lease = catalog.begin_scan(root).map_err(|e| match e {
        pg_catalog::Error::ScanBusy { root, holder } => ScanError::Busy { root, holder },
        e => ScanError::Failed(e.into()),
    })?;
    drop(setup);
    // The lease is given back when it is dropped, however the walk ends (a complete walk gave it back already, in its
    // last transaction).
    let walked =
        walk_leased(&mut lease, root, ctl, lister, root_dir, root_device, root_volume, pacing).map_err(
            |e| match removed(&e) {
                Some(root) => ScanError::Removed { root },
                None => ScanError::Failed(e),
            },
        );
    if let Ok(stats) = &walked {
        pass.items(stats.files);
        pass.field("changed", stats.changed);
        pass.field("missing", stats.missing);
        pass.field("folders", stats.folders);
        pass.field("errors", stats.errors);
    }
    pg_catalog::attach_tx(&mut pass, tx_at);
    walked
}

/// The time the walk's batch writes take (thread time: each is one write transaction of the walk's thread).
static WRITE_BATCH: pg_perf::Tally = pg_perf::Tally::new("scan.write_batch");
// The walk thread's steps (see `walk_leased`): the time the walk spent in each, thread time, the walk's own thread only
// (a worker's listing is its own thread's, not here).
static CHECKPOINT: pg_perf::Tally = pg_perf::Tally::new("scan.checkpoint");
static WAIT_LISTING: pg_perf::Tally = pg_perf::Tally::new("scan.wait_listing");
static LIST_HERE: pg_perf::Tally = pg_perf::Tally::new("scan.list_here");
static CONSUME: pg_perf::Tally = pg_perf::Tally::new("scan.consume");
static UNCHANGED: pg_perf::Tally = pg_perf::Tally::new("scan.unchanged");
static MARK_SEEN: pg_perf::Tally = pg_perf::Tally::new("scan.mark_seen");
static PRIOR_STATES: pg_perf::Tally = pg_perf::Tally::new("scan.prior_states");
static SPAWN: pg_perf::Tally = pg_perf::Tally::new("scan.spawn");
// Nested in those: the volume probe and the folder's rows (inside `LIST_HERE` and `CONSUME`), and the renewals (inside
// any of them). They are reported beside the steps, not summed with them.
static VOLUME_PROBE: pg_perf::Tally = pg_perf::Tally::new("scan.volume_probe");
static FOLDER_ROWS: pg_perf::Tally = pg_perf::Tally::new("scan.folder_rows");
static RENEW: pg_perf::Tally = pg_perf::Tally::new("scan.renew");

/// The folder whose removal from the library ended the walk with `e` (`pg_catalog::Error::RootRemoved`, anywhere in its
/// chain), if that is what ended it.
fn removed(e: &anyhow::Error) -> Option<String> {
    e.chain().find_map(|cause| match cause.downcast_ref::<pg_catalog::Error>() {
        Some(pg_catalog::Error::RootRemoved { root }) => Some(root.clone()),
        _ => None,
    })
}

/// The walk of `root` under its lease. Folders are consumed in walk order, depth first, as a single thread lists them.
/// The listings are made ahead by the workers (`worker`), and by this thread where none has taken a folder yet. This
/// thread alone does what is kept across folders: the identities seen, the mounts, the skipped folders, the batch, the
/// lease. Each folder's listing is consumed when it comes to the top of the stack, so the walk's result is the same
/// whatever the workers do. The lease is renewed at each checkpoint, while this thread lists a folder (each record, once
/// `renew_every` has passed), while it waits for a worker's listing (once that listing has moved), and with each batch.
/// A walk that can't renew it has lost it, and ends with the error.
#[expect(clippy::too_many_arguments, reason = "the walk's state comes from the caller: its root, listing and pacing")]
fn walk_leased<L: Listing>(
    lease: &mut ScanLease<'_>,
    root: &RulePath,
    ctl: &Control,
    lister: &mut L,
    root_dir: L::Dir,
    root_device: Option<u64>,
    root_volume: u64,
    pacing: Pacing,
) -> Result<ScanStats> {
    let rules = lease.rules();
    let scan_id = lease.id();
    let file = lease.file().map(str::to_owned);
    let pool = Pool::new();
    std::thread::scope(|scope| -> Result<ScanStats> {
        // However the walk leaves, no worker takes another folder, and a listing still running is dropped when it's
        // delivered.
        let _stop = PoolStop(&pool);
        let mut stats = ScanStats::default();
        // The folders this scan didn't list, and why: those it couldn't read, the reparse folders it doesn't follow, and
        // the mounts it refuses. The files under them weren't seen, so they aren't marked missing, and the pairing leaves
        // them out.
        let mut skipped: Vec<Skip> = Vec::new();
        // The verdict on each device a folder under the root is on, by its identity (the reason, if refused): one check
        // per mount, not per folder.
        let mut mounts: BTreeMap<u64, Result<(), RefusedReason>> = BTreeMap::new();
        // Each identity recorded or seen this scan: the second path to a file is skipped.
        // (A B-tree set: the project bans std's randomly seeded hash sets, and the walk never iterates this one.)
        let mut seen: BTreeSet<FileIdentity> = BTreeSet::new();
        let mut batch = Batch::default();
        // When the lease was last renewed (a batch's write renews it too).
        let mut renewed = Instant::now();
        let mut complete = true;
        let mut spawned = 0usize;
        // The phase ends when this is dropped, however the walk leaves.
        let _phase = ctl.begin(Phase::Scan { root: root.clone() }, 0);
        let root_job =
            Job { key: Vec::new(), path: root.clone(), device: root_device, volume: root_volume, chain: Vec::new() };
        let mut stack: Vec<Frame<L::Dir>> = vec![Frame { job: root_job, dir: Some(root_dir), pooled: false }];

        // The steps that sum to the walk's time (`steps`), and those nested in them (`nested`). A write made during a pause
        // is inside both `CHECKPOINT` and `WRITE_BATCH`, so the unaccounted time is understated by it.
        let steps = [
            &CHECKPOINT,
            &WAIT_LISTING,
            &LIST_HERE,
            &CONSUME,
            &UNCHANGED,
            &MARK_SEEN,
            &PRIOR_STATES,
            &SPAWN,
            &WRITE_BATCH,
        ];
        let nested = [&VOLUME_PROBE, &FOLDER_ROWS, &RENEW];
        let marks = steps.map(|step| step.mark());
        let nested_marks = nested.map(|step| step.mark());
        let walk_began = Instant::now();
        let mut walk = pg_perf::span("scan.walk");
        while let Some(frame) = stack.pop() {
            // A stop ends the walk here: nothing is marked missing and the scan's `finished` stays NULL. The pairing
            // still runs, on what was seen (tier 1 only: it needs no whole tree).
            let go = {
                let _step = CHECKPOINT.timer();
                checkpoint(lease, root, ctl, &mut batch, &mut stats, &mut renewed, pacing.renew_every, &pool)?
            };
            if !go {
                complete = false;
                break;
            }
            let Frame { job, dir, pooled } = frame;
            let outcome = if pooled {
                let taken = {
                    let _wait = WAIT_LISTING.timer();
                    take(&pool, &job.key, ctl, lease, &mut renewed, pacing.renew_every)?
                };
                match taken {
                    Taken::Listed(outcome) => outcome,
                    Taken::Inline => {
                        let _listing = LIST_HERE.timer();
                        list_here(lease, &mut renewed, pacing.renew_every, lister, rules, &pool, &job, None)?
                    }
                    Taken::Stopped => {
                        complete = false;
                        break;
                    }
                }
            } else {
                let _listing = LIST_HERE.timer();
                list_here(lease, &mut renewed, pacing.renew_every, lister, rules, &pool, &job, dir)?
            };
            let listed = match outcome {
                Outcome::Listed(listed) => listed,
                Outcome::Unreadable(e) => {
                    unreadable(&mut stats, &mut skipped, &mut batch, rules, job.path, &e);
                    continue;
                }
                Outcome::Failed(e) => return Err(e),
                Outcome::Panicked(payload) => std::panic::resume_unwind(payload),
            };
            let Listed { rows, entries, jobs: _ } = listed;
            stats.folders += 1;
            let mut files: Vec<ListedFile> = Vec::new();
            let n = entries.len();
            let mut consume = CONSUME.timer();
            consume.items(u64::try_from(n).unwrap_or(u64::MAX));
            // The entries are the walk's now: a file's name moves into its batch entry, not cloned.
            for (i, e) in entries.into_iter().enumerate() {
                // The identity of an entry on another device is the walk's to find, here, in walk order: the same
                // probes and the same claims a walk with no workers makes.
                let other = other_device(&e, job.device);
                let volume = match (e.device, job.device) {
                    (Some(device), Some(own)) if device != own => {
                        let probe = units_of(&rules.join(&job.path.child(&e.name)));
                        let _probe = VOLUME_PROBE.timer();
                        lister.volume_of_device(device, &probe)
                    }
                    _ => job.volume,
                };
                let identity = FileIdentity::new(volume, e.file_id);
                if e.reparse {
                    stats.skipped_reparse += 1;
                    match e.ext {
                        // A reparse folder is not descended into (it may lead anywhere, and its listing may be a download).
                        None => skipped.push(Skip { path: job.path.child(&e.name), reason: SkipReason::Reparse }),
                        // A reparse file keeps its row if it has one: its listing gives no content, so none is queued.
                        Some(_) => {
                            batch.reparse.push(identity);
                            batch.entries += 1;
                        }
                    }
                    continue;
                }
                let Some(ext) = e.ext else {
                    if skipped_name(rules, e.name.as_bytes()) {
                        continue;
                    }
                    if job.chain.contains(&(volume, e.file_id)) {
                        stats.loops += 1;
                        continue;
                    }
                    let child = job.path.child(&e.name);
                    if other {
                        let verdict = match mounts.get(&volume) {
                            Some(verdict) => verdict.clone(),
                            None => {
                                let verdict = lister.check_mount(&units_of(&rules.join(&child)));
                                mounts.insert(volume, verdict.clone());
                                verdict
                            }
                        };
                        if let Err(reason) = verdict {
                            stats.refused_mounts += 1;
                            if stats.refused_mount_list.len() < FAILURES_KEPT {
                                stats
                                    .refused_mount_list
                                    .push(RefusedMount { mount: child.clone(), reason: reason.clone() });
                            }
                            batch.failed.push((child.clone(), FolderFailure::MountRefused, reason.to_string()));
                            skipped.push(Skip { path: child, reason: SkipReason::RefusedMount });
                            continue;
                        }
                    }
                    let next = child_job(&job, &e, i, n, volume);
                    // A folder on the folder's device was handed to the workers with this listing: it is in the pool.
                    stack.push(Frame { job: next, dir: None, pooled: !other });
                    continue;
                };
                stats.files += 1;
                if !seen.insert(identity) {
                    stats.duplicates += 1;
                    continue;
                }
                files.push(ListedFile {
                    identity,
                    name: e.name,
                    ext,
                    size: e.size,
                    mtime: e.mtime,
                    created: e.created,
                    prior: None,
                });
            }
            drop(consume);
            // A file whose row the folder's read already has as it is listed needs nothing written: it is only marked seen,
            // here, in the walk's set. The others go to the batch.
            let mut key = Vec::new();
            let mut unchanged = Vec::new();
            let mut changed: Vec<ListedFile> = Vec::with_capacity(files.len());
            let mut compare = UNCHANGED.timer();
            compare.items(u64::try_from(files.len()).unwrap_or(u64::MAX));
            for f in files {
                let row = {
                    let listed = ScannedFile {
                        identity: f.identity,
                        name: &f.name,
                        ext: f.ext,
                        size: f.size,
                        mtime: f.mtime,
                        created: f.created,
                        prior: None,
                    };
                    rows.unchanged(&listed, &mut key)
                };
                match row {
                    Some(row) => unchanged.push(row),
                    None => changed.push(f),
                }
            }
            drop(compare);
            {
                let mut mark = MARK_SEEN.timer();
                mark.items(u64::try_from(unchanged.len()).unwrap_or(u64::MAX));
                lease.mark_seen(&unchanged)?;
            }
            if !changed.is_empty() {
                // The rows the batch compares with: the folder's, read before the listing, and the others' by identity, read
                // now but none changed since the listing began (`Catalog::prior_states`). The batch, written later, changes
                // only those still so, so a write-back from the folder's listing on is kept.
                let identities: Vec<FileIdentity> = changed.iter().map(|f| f.identity).collect();
                let priors = {
                    let mut prior = PRIOR_STATES.timer();
                    prior.items(u64::try_from(identities.len()).unwrap_or(u64::MAX));
                    lease.prior_states(&rows, &identities)?
                };
                ensure!(
                    priors.len() == changed.len(),
                    "the catalog gave {} rows for {} files",
                    priors.len(),
                    changed.len()
                );
                for (file, prior) in changed.iter_mut().zip(priors) {
                    file.prior = prior;
                }
                batch.entries += changed.len();
                batch.folders.push(ListedFolder { path: job.path.clone(), files: changed });
            }
            if batch.entries >= pacing.batch {
                write_batch(lease, root, &mut batch, &mut stats, ctl)?;
                renewed = Instant::now();
            }
            ctl.set_done(stats.files);
            if spawned < pacing.workers
                && pool.pending_len() > 1
                && let Some(file) = &file
            {
                spawned += 1;
                let _spawn_step = SPAWN.timer();
                spawn_worker(scope, &pool, ctl, file.as_str(), rules, lister.fresh());
            }
        }
        // The workers take no more folders: what they still list is dropped when it's delivered.
        pool.stop();
        // What is built is written, however the walk ended (a stop commits what it has).
        write_batch(lease, root, &mut batch, &mut stats, ctl)?;
        walk.items(stats.files);
        for (step, mark) in steps.iter().zip(marks) {
            walk.attach_since(step, mark);
        }
        for (step, mark) in nested.iter().zip(nested_marks) {
            walk.attach_since(step, mark);
        }
        // The walk's time that no step covers: the walk's wall time less the steps that sum to it.
        let covered: Duration = steps.iter().zip(marks).map(|(step, mark)| step.spent_since(mark)).sum();
        walk.field("unaccounted_ms", walk_began.elapsed().saturating_sub(covered).as_secs_f64() * 1e3);
        drop(walk);
        // The pairing needs the tree looked through for tiers 2 and replacement, so it runs them only on a complete walk,
        // and outside the folders it skipped. Each of its transactions, and the missing marks', holds the lease first, as
        // the batches do: after the folder's removal, none of them writes.
        stats.pairing = lease.pair_after_scan(Coverage { complete, skipped: &skipped })?;
        if complete {
            // Its last transaction sweeps the folder ledger under `root` and gives the lease back.
            stats.missing = lease.mark_missing(&skipped)?;
            lease.finish_scan(scan_id)?;
            ctl.ledger_changed();
        }
        stats.complete = complete;
        stats.volume_fallbacks = lister.fallback_volumes();
        Ok(stats)
    })
}

/// A folder the walk couldn't open or list: counted, kept for the report, skipped (its files aren't judged), and
/// recorded in the folder ledger with the next batch.
fn unreadable(
    stats: &mut ScanStats,
    skipped: &mut Vec<Skip>,
    batch: &mut Batch,
    rules: PathRules,
    path: RulePath,
    error: &io::Error,
) {
    let error = format!("{error:#}");
    keep_failure(&mut stats.failures, rules.display(&path), error.clone());
    stats.errors += 1;
    batch.failed.push((path.clone(), FolderFailure::Unreadable, error));
    skipped.push(Skip { path, reason: SkipReason::Unreadable });
}

/// The walk's checkpoint before each folder, with nothing of the catalog held across a wait: a pause or a stop first
/// writes the batch built so far, and a pause renews the lease while it lasts (so does a walk held up anywhere, every
/// `renew_every`, [`RENEW_EVERY`] in production). A resumed walk wakes the workers, which wait on a pause too. False for a
/// stop.
#[expect(clippy::too_many_arguments, reason = "the walk's state, as the walk passes it")]
fn checkpoint(
    lease: &mut ScanLease<'_>,
    root: &RulePath,
    ctl: &Control,
    batch: &mut Batch,
    stats: &mut ScanStats,
    renewed: &mut Instant,
    renew_every: Duration,
    pool: &Pool,
) -> Result<bool> {
    if renewed.elapsed() >= renew_every {
        renew(lease)?;
        *renewed = Instant::now();
    }
    if ctl.is_paused() || ctl.stop_requested() {
        write_batch(lease, root, batch, stats, ctl)?;
        *renewed = Instant::now();
    }
    loop {
        match ctl.checkpoint_within(RENEW_EVERY) {
            Checkpoint::Go => {
                pool.kick();
                return Ok(true);
            }
            Checkpoint::Stop => return Ok(false),
            Checkpoint::Paused => {
                // A pause set after the look above: what is built is written now, not held over the pause.
                write_batch(lease, root, batch, stats, ctl)?;
                renew(lease)?;
                *renewed = Instant::now();
            }
        }
    }
}

/// Renew the walk's lease. A lease that is gone ends the walk: its folder was removed from the library, which took it
/// (`pg_catalog::Error::RootRemoved`, [`ScanError::Removed`]), or it expired while the walk was held up, and another
/// walk may have begun (`pg_catalog::Error::LeaseLost`): the walk can no longer tell that it is alone.
fn renew(lease: &ScanLease<'_>) -> Result<()> {
    let _step = RENEW.timer();
    Ok(lease.renew()?)
}

/// Write the batch in one write transaction, committed at once, and empty it: the folder ledger's rows, the reparse
/// files seen, the folders and their files (each written only while its row is as the listing read it). The writer's
/// making renews the lease. Nothing is written for an empty batch.
fn write_batch(
    lease: &mut ScanLease<'_>,
    root: &RulePath,
    batch: &mut Batch,
    stats: &mut ScanStats,
    ctl: &Control,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut timer = WRITE_BATCH.timer();
    timer.items(u64::try_from(batch.entries).unwrap_or(u64::MAX));
    let scan_id = lease.id();
    // Renewed just before the batch's transaction: a walk whose lease was taken (it expired while the walk was held up)
    // writes nothing beside the other walk's writes.
    renew(&*lease)?;
    let tx = lease.tx()?;
    let mut ledger = false;
    {
        let mut writer = tx.scan_writer(scan_id, root)?;
        for (path, failure, error) in &batch.failed {
            ledger |= writer.folder_failed(path, *failure, error)?;
        }
        for identity in &batch.reparse {
            writer.seen_identity(*identity)?;
        }
        for listed in &batch.folders {
            let folder = writer.folder(&listed.path)?;
            for f in &listed.files {
                let file = ScannedFile {
                    identity: f.identity,
                    name: &f.name,
                    ext: f.ext,
                    size: f.size,
                    mtime: f.mtime,
                    created: f.created,
                    prior: f.prior,
                };
                match writer.record_file(&folder, &file)? {
                    Recorded::Same => {}
                    Recorded::Changed => stats.changed += 1,
                    Recorded::Foreign => stats.duplicates += 1,
                }
            }
        }
    }
    tx.commit()?;
    *batch = Batch::default();
    if ledger {
        ctl.ledger_changed();
    }
    Ok(())
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
        name: String,
        folder: bool,
        reparse: bool,
        file_id: u64,
        size: u64,
        mtime: i64,
        volume: Option<u64>,
    }

    /// A file entry, on the folder's volume.
    fn file(name: &str, file_id: u64, size: u64) -> FakeEntry {
        FakeEntry { name: name.to_string(), folder: false, reparse: false, file_id, size, mtime: 1, volume: None }
    }

    /// A folder entry.
    fn folder(name: &str, file_id: u64) -> FakeEntry {
        FakeEntry { name: name.to_string(), folder: true, reparse: false, file_id, size: 0, mtime: 1, volume: None }
    }

    /// What a test runs when a folder is listed: on the thread that lists it, a worker's or the walk's.
    type Hook = Arc<Mutex<dyn FnMut() + Send>>;

    /// Folders by their units, as the walk opens them: each one's path under the fake root, and its entries. A clone lists
    /// the same tree, with the same hooks: a worker's lister is one.
    type Tree = Vec<(Vec<Unit>, String, Vec<FakeEntry>)>;

    #[derive(Clone)]
    struct FakeListing {
        tree: Arc<Tree>,
        root_volume: u64,
        /// Folders (by their units) whose mount is refused.
        refused: Vec<Vec<Unit>>,
        /// Run when the folder at the path is listed, before its entries are given.
        hooks: Vec<(String, Hook)>,
        /// Run when the folder at the path has been listed, after its entries are given.
        after: Vec<(String, Hook)>,
    }

    impl FakeListing {
        fn new<S: AsRef<str>>(root_volume: u64, tree: &[(S, Vec<FakeEntry>)]) -> FakeListing {
            FakeListing::under(pg_platform::native::PATH_RULES, root_volume, tree)
        }

        /// The fake tree with its folders spelled under `rules`.
        fn under<S: AsRef<str>>(rules: PathRules, root_volume: u64, tree: &[(S, Vec<FakeEntry>)]) -> FakeListing {
            let tree = tree
                .iter()
                .map(|(rel, entries)| {
                    let rel = rel.as_ref();
                    (units_of(fake_path(rules, rel).as_bytes()), rel.to_string(), entries.clone())
                })
                .collect();
            FakeListing { tree: Arc::new(tree), root_volume, refused: Vec::new(), hooks: Vec::new(), after: Vec::new() }
        }

        /// The same tree, with the mount at `rel` (spelled under `rules`) refused by its file system.
        fn refusing(mut self, rules: PathRules, rel: &str) -> FakeListing {
            self.refused.push(units_of(fake_path(rules, rel).as_bytes()));
            self
        }

        /// The same tree, running `hook` each time the folder at `rel` is listed.
        fn on_listing(mut self, rel: &str, hook: impl FnMut() + Send + 'static) -> FakeListing {
            let hook: Hook = Arc::new(Mutex::new(hook));
            self.hooks.push((rel.to_string(), hook));
            self
        }

        /// The same tree, running `hook` once the folder at `rel` has been listed (all its entries given).
        fn after_listing(mut self, rel: &str, hook: impl FnMut() + Send + 'static) -> FakeListing {
            let hook: Hook = Arc::new(Mutex::new(hook));
            self.after.push((rel.to_string(), hook));
            self
        }

        /// Runs the hooks registered at `rel` in `hooks`.
        fn run_hooks(hooks: &[(String, Hook)], rel: &str) {
            for (at, hook) in hooks {
                if at == rel {
                    let mut run = hook.lock().unwrap_or_else(PoisonError::into_inner);
                    (*run)();
                }
            }
        }
    }

    impl Listing for FakeListing {
        type Dir = usize;

        fn open(&mut self, path: &[Unit]) -> io::Result<usize> {
            self.tree
                .iter()
                .position(|(p, _, _)| p == path)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such fake folder"))
        }

        fn device(&self, _dir: &usize) -> Option<u64> {
            Some(self.root_volume)
        }

        fn volume(&mut self, _dir: &usize) -> io::Result<u64> {
            Ok(self.root_volume)
        }

        /// The fake's devices are its identities: an entry's `volume` is its device, and its identity.
        fn volume_of_device(&mut self, device: u64, _probe: &[Unit]) -> u64 {
            device
        }

        fn fallback_volumes(&self) -> u64 {
            0
        }

        fn fresh(&self) -> FakeListing {
            self.clone()
        }

        fn check_mount(&mut self, path: &[Unit]) -> Result<(), RefusedReason> {
            if self.refused.iter().any(|p| p == path) {
                return Err(RefusedReason::Nfs);
            }
            Ok(())
        }

        fn each(&mut self, dir: &usize, mut each: impl FnMut(Entry<'_>)) -> io::Result<()> {
            let (_, rel, entries) = &self.tree[*dir];
            Self::run_hooks(&self.hooks, rel);
            for e in entries {
                let units = units_of(e.name.as_bytes());
                each(Entry {
                    name: &units,
                    folder: e.folder,
                    reparse: e.reparse,
                    file_id: e.file_id,
                    size: e.size,
                    mtime: e.mtime,
                    device: e.volume,
                    created: None,
                });
            }
            Self::run_hooks(&self.after, rel);
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

    /// One scan of the fake tree into a catalog, under the catalog's rules.
    fn scan_fake<S: AsRef<str>>(catalog: &mut Catalog, tree: &[(S, Vec<FakeEntry>)], root_volume: u64) -> ScanStats {
        let rules = catalog.rules();
        let root = fake_root_under(rules);
        catalog.add_root(&root).unwrap();
        let mut lister = FakeListing::under(rules, root_volume, tree);
        walk(catalog, &root, &Control::new(), &mut lister).unwrap()
    }

    fn catalog() -> Catalog {
        Catalog::prepare(Path::new(":memory:"), pg_catalog::Versions::current(), pg_platform::native::PATH_RULES)
            .unwrap()
    }

    #[test]
    fn an_entry_with_no_file_id_makes_its_folder_unreadable() {
        // The folder's other entries are not recorded either: the listing fails as a whole.
        let mut lister = FakeListing::new(1, &[("", vec![file("a.jpg", 7, 1), file("b.jpg", 0, 1)])]);
        assert!(list(&mut lister, &0, &mut || {}).is_err());
    }

    #[test]
    fn the_dot_entries_may_have_no_file_id() {
        let mut lister = FakeListing::new(1, &[("", vec![file(".", 0, 1), file("..", 0, 1), file("a.jpg", 7, 1)])]);
        assert_eq!(list(&mut lister, &0, &mut || {}).unwrap().len(), 1);
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
        // A reparse file is not a failure: the folder ledger has no row for it.
        assert!(c.failed_folders(10).unwrap().is_empty());
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
        // Nor is the reparse folder a failure: it is skipped, not recorded as unreadable or refused.
        assert!(c.failed_folders(10).unwrap().is_empty());
    }

    #[test]
    fn a_renumbered_volume_rebinds_every_row_and_loses_none() {
        // The same tree on volume 1, then on volume 2 (a device renumbered at a restart): every file is found again under its
        // new identity, none is ambiguous, and none is missing.
        let mut c = catalog();
        let tree = [("", vec![file("a.jpg", 7, 1), folder("sub", 20)]), ("sub", vec![file("b.jpg", 8, 2)])];
        scan_fake(&mut c, &tree, 1);
        let stats = scan_fake(&mut c, &tree, 2);
        assert_eq!(
            (stats.pairing.rebound, stats.pairing.ambiguous, stats.pairing.unpaired_with_data, stats.missing),
            (2, 0, 0, 0)
        );
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
        assert!(c.failed_folders(10).unwrap().is_empty(), "a reparse folder is not a failure");
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

    /// The keys the scan stores are the ones the grid looks folders up by: a folder ending in Σ
    /// after a letter (`ΟΔΥΣΣΕΥΣ`, whose final sigma lowercases differently from a whole-word fold) lists its files,
    /// alone and with its subfolders, under both rules.
    fn scanned_folders_list_their_files_in_the_grid(rules: PathRules) {
        use pg_model::query::{Query, Sort};

        let mut c = Catalog::prepare(Path::new(":memory:"), pg_catalog::Versions::current(), rules).unwrap();
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
        c.add_root(&fake_root()).unwrap();
        let mnt = FakeEntry { volume: Some(2), ..folder("mnt", 30) };
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

    // The walk's transactions (the write-lock section of the module docs, W2): nothing holds the catalog's write lock across a listing or a
    // pause, overlapping walks are refused while one runs, and a batch changes only the rows still as it read them.

    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Duration;

    use pg_catalog::Listed;
    use pg_catalog::fixture::{WriteHolds, state};
    use pg_model::turns::Turns;

    /// A catalog on disk (a second connection can reach it), prepared with a library folder at the fake root.
    fn on_disk(prefix: &str) -> std::path::PathBuf {
        let dir = pg_testkit::run_folder("scratch_scan", prefix).unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, pg_catalog::Versions::current(), pg_platform::native::PATH_RULES).unwrap();
        assert!(c.add_root(&fake_root()).unwrap());
        path
    }

    fn open(path: &Path) -> Catalog {
        Catalog::open(path, pg_platform::native::PATH_RULES).unwrap()
    }

    /// Whether another connection can take the write lock within `wait` (a write of its own: a library folder added
    /// beside the fake root).
    fn another_can_write(path: &Path, wait: Duration) -> bool {
        let other = open(path);
        other.set_busy_timeout(wait).unwrap();
        let rules = pg_platform::native::PATH_RULES;
        let elsewhere = match rules {
            PathRules::Windows => r"D:\elsewhere",
            PathRules::Posix => "/elsewhere",
        };
        other.add_root(&rules.parse(elsewhere).unwrap()).is_ok()
    }

    /// A root with folders `a`, `b` and `c`, one file each. The walk pops them `c`, `b`, `a`.
    fn three_folders() -> Vec<(&'static str, Vec<FakeEntry>)> {
        vec![
            ("", vec![folder("a", 20), folder("b", 21), folder("c", 22)]),
            ("a", vec![file("a.jpg", 1, 1)]),
            ("b", vec![file("b.jpg", 2, 1)]),
            ("c", vec![file("c.jpg", 3, 1)]),
        ]
    }

    #[test]
    fn a_paused_walk_holds_no_write_lock() {
        let path = on_disk("scan-paused");
        let ctl = Arc::new(Control::new());
        let (listed, at_b) = mpsc::channel();
        let pausing = ctl.clone();
        let lister = FakeListing::new(1, &three_folders()).on_listing("b", move || {
            pausing.set_paused(true);
            let _ = listed.send(());
        });
        let (walk_ctl, walk_path) = (ctl.clone(), path.clone());
        let walking = std::thread::spawn(move || {
            let mut lister = lister;
            walk(&mut open(&walk_path), &fake_root(), &walk_ctl, &mut lister).map_err(|e| e.to_string())
        });
        at_b.recv_timeout(Duration::from_secs(30)).unwrap();
        // The walk writes what it built and waits at its next checkpoint: holding nothing, whatever the pause lasts.
        assert!(another_can_write(&path, Duration::from_secs(2)), "a paused walk held the write lock");
        ctl.set_paused(false);
        let stats = walking.join().unwrap().unwrap();
        assert!(stats.complete);
        assert_eq!((stats.files, open(&path).status().unwrap().files), (3, 3));
    }

    #[test]
    fn a_slow_listing_holds_no_write_lock() {
        let path = on_disk("scan-slow");
        let (listing, at_b) = mpsc::channel();
        let lister = FakeListing::new(1, &three_folders()).on_listing("b", move || {
            let _ = listing.send(());
            std::thread::sleep(Duration::from_millis(1500));
        });
        let walk_path = path.clone();
        let walking = std::thread::spawn(move || {
            let mut lister = lister;
            walk(&mut open(&walk_path), &fake_root(), &Control::new(), &mut lister).map_err(|e| e.to_string())
        });
        at_b.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(another_can_write(&path, Duration::from_millis(500)), "a walk held the write lock while it listed");
        assert!(walking.join().unwrap().unwrap().complete);
    }

    #[test]
    fn a_stop_mid_walk_keeps_what_it_listed_and_marks_nothing_missing() {
        let mut c = catalog();
        scan_fake(&mut c, &three_folders(), 1);
        // Then every file is bigger and c.jpg is gone; the walk is stopped as it lists b, so a is never listed.
        let mut tree = three_folders();
        for (_, entries) in &mut tree {
            for e in entries.iter_mut().filter(|e| !e.folder) {
                e.size = 5;
            }
        }
        tree[3].1.clear();
        let ctl = Arc::new(Control::new());
        let stopping = ctl.clone();
        let mut lister = FakeListing::new(1, &tree).on_listing("b", move || stopping.request_stop());
        let stopped = walk(&mut c, &fake_root(), &ctl, &mut lister).unwrap();
        assert!(!stopped.complete);
        assert_eq!((stopped.changed, stopped.missing), (1, 0), "b.jpg's change is in; nothing is marked missing");
        assert_eq!(c.status().unwrap().missing, 0, "c.jpg is not missing after a stopped walk");
        // The lease went with the stopped walk: the next one runs, and finds a.jpg changed and c.jpg gone.
        let complete = scan_fake(&mut c, &tree, 1);
        assert!(complete.complete);
        assert_eq!((complete.changed, complete.missing), (1, 1));
    }

    #[test]
    fn a_walk_that_panics_gives_its_lease_back() {
        let mut c = catalog();
        c.add_root(&fake_root()).unwrap();
        let mut lister = FakeListing::new(1, &three_folders()).on_listing("b", || panic!("the listing panicked"));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            walk(&mut c, &fake_root(), &Control::new(), &mut lister).map(|_| ())
        }));
        assert!(panicked.is_err());
        assert!(scan_fake(&mut c, &three_folders(), 1).complete, "the next walk is not refused");
    }

    #[test]
    fn an_overlapping_walk_is_refused_while_one_runs_and_runs_after_it() {
        let path = on_disk("scan-overlap");
        scan_fake(&mut open(&path), &three_folders(), 1);
        let ctl = Arc::new(Control::new());
        let (listed, at_b) = mpsc::channel();
        let pausing = ctl.clone();
        let lister = FakeListing::new(1, &three_folders()).on_listing("b", move || {
            pausing.set_paused(true);
            let _ = listed.send(());
        });
        let (walk_ctl, walk_path) = (ctl.clone(), path.clone());
        let walking = std::thread::spawn(move || {
            let mut lister = lister;
            walk(&mut open(&walk_path), &fake_root(), &walk_ctl, &mut lister).map_err(|e| e.to_string())
        });
        at_b.recv_timeout(Duration::from_secs(30)).unwrap();
        // A walk of a folder inside the one walked is refused, with nothing written.
        let sub = fake_root().child(&Name::from("a"));
        let mut other = open(&path);
        let refused = walk(&mut other, &sub, &Control::new(), &mut FakeListing::new(1, &three_folders()));
        assert!(matches!(refused, Err(ScanError::Busy { .. })), "{refused:?}");
        ctl.set_paused(false);
        assert!(walking.join().unwrap().unwrap().complete);
        // Once it ended, the refused walk runs, and no file that is there is marked missing.
        let after = walk(&mut other, &sub, &Control::new(), &mut FakeListing::new(1, &three_folders())).unwrap();
        assert!(after.complete);
        assert_eq!((after.missing, other.status().unwrap().missing), (0, 0));
    }

    #[test]
    fn a_write_back_between_the_listing_and_the_batch_is_kept_and_a_real_change_is_not_blocked() {
        let path = on_disk("scan-write-back");
        let tree = [("", vec![file("f.jpg", 7, 1), folder("sub", 20)]), ("sub", vec![file("g.jpg", 8, 1)])];
        let mut c = open(&path);
        scan_fake(&mut c, &tree, 1);
        state::stub_analysis_of_all(&c).unwrap();
        let f = state::file_id(&c, "f.jpg").unwrap();
        assert_eq!(c.status().unwrap().pending, 0);
        let row = |c: &Catalog, size: i64, mtime: i64| c.is_current(f, &Listed { size, mtime, rotation: Turns::NONE });

        // The root (and f.jpg in it) is listed, then the app rewrites f.jpg's metadata as sub is listed: the batch,
        // written after, leaves the row the write-back made, and nothing is due.
        let hook_path = path.clone();
        let mut lister = FakeListing::new(1, &tree).on_listing("sub", move || {
            state::change_mtime(&open(&hook_path), f, 999).unwrap();
        });
        let stats = walk(&mut c, &fake_root(), &Control::new(), &mut lister).unwrap();
        assert_eq!((stats.changed, stats.missing), (0, 0));
        assert!(row(&c, 1, 999).unwrap(), "the write-back's row was undone by an older listing");
        assert_eq!(c.status().unwrap().pending, 0);

        // A real change on disk is written though the row's change number moved meanwhile (here, the file asked to be
        // imported again): the guard compares the row's size and time, not its stamps.
        let mut changed = tree.clone();
        changed[0].1[0].size = 5;
        let hook_path = path.clone();
        let mut lister = FakeListing::new(1, &changed).on_listing("sub", move || {
            open(&hook_path).mark_pending(&[f]).unwrap();
        });
        let stats = walk(&mut c, &fake_root(), &Control::new(), &mut lister).unwrap();
        assert_eq!(stats.changed, 1);
        assert!(row(&c, 5, 1).unwrap(), "a real change was blocked by a change number");
    }

    /// A write-back that commits once a folder's entries are given, before the walk goes on (the hook after the root's
    /// listing): the batch keeps the row the write-back made. The folder's rows were read before its listing, so the
    /// row no longer equals them, and the file is only marked seen.
    #[test]
    fn a_write_back_while_its_own_folder_is_listed_is_kept() {
        let path = on_disk("scan-write-back-own-folder");
        let tree = [("", vec![file("f.jpg", 7, 1)])];
        let mut c = open(&path);
        scan_fake(&mut c, &tree, 1);
        let f = state::file_id(&c, "f.jpg").unwrap();
        let hook_path = path.clone();
        let mut lister = FakeListing::new(1, &tree).after_listing("", move || {
            state::change_mtime(&open(&hook_path), f, 999).unwrap();
        });
        let stats = walk(&mut c, &fake_root(), &Control::new(), &mut lister).unwrap();
        assert_eq!((stats.changed, stats.missing), (0, 0));
        let rewritten = Listed { size: 1, mtime: 999, rotation: Turns::NONE };
        assert!(c.is_current(f, &rewritten).unwrap(), "the write-back's row was undone by an older listing");
    }

    /// A file with two names (hard links) in two folders, listed first by the name its row doesn't have, and rewritten
    /// by the app through the row's name once that folder is listed, before the walk looks the file up by its identity:
    /// the batch keeps the row the write-back made. The row changed after the walk began listing the folder (its change
    /// number is past the folder's mark), so the file is only marked seen.
    #[test]
    fn a_write_back_through_another_name_of_a_file_is_kept() {
        let path = on_disk("scan-write-back-hard-link");
        let first = [("", vec![folder("a", 20), folder("b", 21)]), ("a", vec![file("p.jpg", 7, 1)]), ("b", vec![])];
        let mut c = open(&path);
        scan_fake(&mut c, &first, 1);
        let f = state::file_id(&c, "p.jpg").unwrap();
        // b now holds a second name of the file, and the walk lists it first (it takes the root's last folder first).
        let linked = [
            ("", vec![folder("a", 20), folder("b", 21)]),
            ("a", vec![file("p.jpg", 7, 1)]),
            ("b", vec![file("q.jpg", 7, 1)]),
        ];
        let hook_path = path.clone();
        let mut lister = FakeListing::new(1, &linked).after_listing("b", move || {
            state::change_mtime(&open(&hook_path), f, 999).unwrap();
        });
        let stats = walk(&mut c, &fake_root(), &Control::new(), &mut lister).unwrap();
        assert_eq!((stats.changed, stats.duplicates), (0, 1));
        let rewritten = Listed { size: 1, mtime: 999, rotation: Turns::NONE };
        assert!(c.is_current(f, &rewritten).unwrap(), "the write-back's row was undone by an older listing");
    }

    /// The folder removed from the library while it is walked, by another process (which can't stop this walk): the
    /// walk ends at its next write, writing nothing, and the folder's files are gone from the catalog.
    #[test]
    fn a_folder_removed_while_it_is_walked_is_not_written_again() {
        let path = on_disk("scan-removed");
        let mut c = open(&path);
        scan_fake(&mut c, &three_folders(), 1);
        // Every file is changed on disk, so the walk has its files to write.
        let mut changed = three_folders();
        for (_, entries) in &mut changed {
            for e in entries.iter_mut().filter(|e| !e.folder) {
                e.size = 5;
            }
        }
        let root = c.root_id(&fake_root()).unwrap().unwrap();
        let hook_path = path.clone();
        let mut lister = FakeListing::new(1, &changed).after_listing("b", move || {
            assert_eq!(open(&hook_path).remove_root(root).unwrap(), Some(3));
        });
        let walked = walk(&mut c, &fake_root(), &Control::new(), &mut lister);
        assert!(matches!(walked, Err(ScanError::Removed { .. })), "{walked:?}");
        let status = c.status().unwrap();
        assert_eq!((status.files, status.missing), (0, 0), "the removed folder's files were written again");
    }

    /// A new import version between a folder's listing and the batch that writes its files: the bump makes the analysed
    /// files pending (their analyses are older than the version), and the batch, which finds them unchanged, keeps
    /// neither analysed.
    #[test]
    fn a_version_bump_between_the_listing_and_the_batch_is_not_undone() {
        let path = on_disk("scan-version-bump");
        let tree = [("", vec![file("f.jpg", 7, 1), folder("sub", 20)]), ("sub", vec![file("g.jpg", 8, 1)])];
        let mut c = open(&path);
        scan_fake(&mut c, &tree, 1);
        state::stub_analysis_of_all(&c).unwrap();
        assert_eq!(c.status().unwrap().pending, 0);

        // The bump is made while sub is listed, so both batches are written after it.
        let hook_path = path.clone();
        let mut lister = FakeListing::new(1, &tree).on_listing("sub", move || {
            // One newer than this build's: the import's version is bumped while the walk lists.
            let bumped = pg_catalog::Versions {
                import: pg_model::versions::IMPORT_VERSION + 1,
                ..pg_catalog::Versions::current()
            };
            drop(Catalog::prepare(&hook_path, bumped, pg_platform::native::PATH_RULES).unwrap());
        });
        let stats = walk(&mut c, &fake_root(), &Control::new(), &mut lister).unwrap();
        assert_eq!((stats.changed, stats.missing), (0, 0));
        let pending = c.status().unwrap().pending;
        assert_eq!(pending, 2, "the bump made both files pending, and the walk kept neither analysed");
        // The stale analyses are kept, not deleted: they stay what the grid shows until the import redoes them, and the
        // import's queue holds exactly the two files.
        assert_eq!(c.status().unwrap().analyzed, 2, "both analyses are still stored, stale");
        assert_eq!(
            c.pending_import(false, &pg_catalog::VolumeSet::All).unwrap().items.len(),
            2,
            "both files are due for the import"
        );
    }

    /// A Retry (the user's, or a pass's) between a folder's listing and the batch that writes its files. It moves the
    /// files' change numbers (`files.seq`), which the walk does not compare (only size and mtime are), so the batch
    /// keeps what it read: nothing changed on disk, nothing is due, and the Retry's own work is not undone.
    #[test]
    fn a_retry_between_the_listing_and_the_batch_is_not_undone() {
        use pg_catalog::ledger::Failure;
        use pg_model::notice::FilePass;

        let path = on_disk("scan-retry");
        let tree = [("", vec![file("f.jpg", 7, 1), folder("sub", 20)]), ("sub", vec![file("g.jpg", 8, 1)])];
        let mut c = open(&path);
        scan_fake(&mut c, &tree, 1);
        state::stub_analysis_of_all(&c).unwrap();
        let f = state::file_id(&c, "f.jpg").unwrap();
        // f.jpg is 1 byte (`file`'s third argument), mtime 1.
        let listed = Listed { size: 1, mtime: 1, rotation: Turns::NONE };
        // f.jpg's thumbnail failed; the user retries it while sub is listed.
        let failure = Failure {
            file_id: f,
            mtime: 1,
            size: 1,
            rotation: Turns::NONE,
            error: "does not decode".into(),
            retry: true,
        };
        c.record_failures(FilePass::Thumbs, &[failure], 1).unwrap();
        assert!(c.any_failures(FilePass::Thumbs).unwrap());
        let hook_path = path.clone();
        let mut lister = FakeListing::new(1, &tree).on_listing("sub", move || {
            open(&hook_path).retry_failures(FilePass::Thumbs).unwrap();
        });
        let stats = walk(&mut c, &fake_root(), &Control::new(), &mut lister).unwrap();
        assert_eq!((stats.changed, stats.missing), (0, 0), "a change number is not a change on disk");
        assert!(c.is_current(f, &listed).unwrap(), "the batch kept f.jpg as it was listed");
        assert_eq!(c.status().unwrap().pending, 0, "the Retry made nothing due");
        assert!(!c.any_failures(FilePass::Thumbs).unwrap(), "the Retry's ledger rows stay gone");
    }

    #[test]
    fn a_folder_of_25_000_entries_is_written_whole_across_the_batch_size() {
        let mut c = catalog();
        let big: Vec<FakeEntry> = (0..25_000u64).map(|i| file(&format!("{i:05}.jpg"), 100 + i, 1)).collect();
        let tree = vec![
            ("", vec![folder("big", 20), folder("small", 21)]),
            ("big", big),
            ("small", (0..10u64).map(|i| file(&format!("s{i}.jpg"), 50_000 + i, 1)).collect()),
        ];
        let first = scan_fake(&mut c, &tree, 1);
        assert_eq!((first.files, first.changed, c.status().unwrap().files), (25_010, 25_010, 25_010));
        let again = scan_fake(&mut c, &tree, 1);
        assert_eq!((again.files, again.changed, again.missing), (25_010, 0, 0));
    }

    // The pairing runs after the last batch, over the catalog, so a row and the file it pairs with in different batches
    // still pair: a volume renumbered, and one file moved to another folder and name while closed (tier 2), its old
    // row and the file written in different batches. Batches of two entries here.
    #[test]
    fn a_pairing_across_the_batch_boundary_still_finds_its_rows() {
        let mut c = catalog();
        c.add_root(&fake_root()).unwrap();
        let folders = |moved_into: &str| -> Vec<(String, Vec<FakeEntry>)> {
            let mut tree = vec![(String::new(), (0..5u64).map(|f| folder(&format!("f{f}"), 100 + f)).collect())];
            for f in 0..5u64 {
                let mut files: Vec<FakeEntry> =
                    (0..2u64).map(|i| file(&format!("{i}.jpg"), 10 + f * 2 + i, 1)).collect();
                if format!("f{f}") == moved_into {
                    files.push(file("moved.jpg", 99, 3));
                }
                tree.push((format!("f{f}"), files));
            }
            tree
        };
        let mut first = FakeListing::new(1, &folders("f0"));
        let before = walk_in_batches(&mut c, &fake_root(), &Control::new(), &mut first, 2, RENEW_EVERY).unwrap();
        assert_eq!((before.files, c.status().unwrap().files), (11, 11));
        let mut tree = folders("f4");
        tree[5].1.last_mut().unwrap().name = "renamed.jpg".to_string();
        let mut second = FakeListing::new(2, &tree);
        let stats = walk_in_batches(&mut c, &fake_root(), &Control::new(), &mut second, 2, RENEW_EVERY).unwrap();
        assert_eq!((stats.pairing.rebound, stats.pairing.ambiguous, stats.missing), (11, 0, 0));
        assert_eq!((c.status().unwrap().files, c.status().unwrap().missing), (11, 0));
    }

    // Across roots: a present row of another root keeps its identity (the file here is a second path to it,
    // skipped), and a missing one does not (its file moved here while the app was closed: the row moves with it, and
    // no second row is made).
    #[test]
    fn a_missing_row_in_another_root_does_not_claim_the_identity() {
        let mut c = catalog();
        let a = fake_root().child(&Name::from("a"));
        let b = fake_root().child(&Name::from("b"));
        c.add_root(&a).unwrap();
        c.add_root(&b).unwrap();
        let both = [("a", vec![file("p.jpg", 7, 1)]), ("b", vec![file("p.jpg", 7, 1)])];
        walk(&mut c, &a, &Control::new(), &mut FakeListing::new(1, &both)).unwrap();
        let linked = walk(&mut c, &b, &Control::new(), &mut FakeListing::new(1, &both)).unwrap();
        assert_eq!((linked.duplicates, c.status().unwrap().files), (1, 1), "a present row of a is a's");

        let moved = [("a", vec![]), ("b", vec![file("p.jpg", 7, 1)])];
        let gone = walk(&mut c, &a, &Control::new(), &mut FakeListing::new(1, &moved)).unwrap();
        assert_eq!((gone.missing, c.status().unwrap().missing), (1, 1));
        let found = walk(&mut c, &b, &Control::new(), &mut FakeListing::new(1, &moved)).unwrap();
        assert_eq!((found.duplicates, found.changed), (0, 1));
        let status = c.status().unwrap();
        assert_eq!((status.files, status.missing), (1, 0), "the row moved to b; no second row");
    }

    /// A listing that outlasts the lease keeps it: each record renews it once `renew_every` has passed. A minute passes
    /// before the folder's first record; when its listing is done the lease is fresh, so another walk of the root is
    /// refused rather than let in on the old lease.
    #[test]
    fn a_listing_that_outlasts_the_lease_keeps_it() {
        let path = on_disk("scan-long-listing");
        let tree = [("", vec![file("a.jpg", 1, 1), file("b.jpg", 2, 1), file("c.jpg", 3, 1)])];
        let (aging, checking) = (path.clone(), path.clone());
        let mut lister = FakeListing::new(1, &tree)
            .on_listing("", move || state::age_leases(&open(&aging), pg_catalog::SCAN_LEASE_SECS + 1).unwrap())
            .after_listing("", move || {
                let mut other = open(&checking);
                assert!(other.begin_scan(&fake_root()).is_err(), "the lease was renewed while the folder listed");
            });
        let mut c = open(&path);
        let stats = walk_in_batches(&mut c, &fake_root(), &Control::new(), &mut lister, BATCH, Duration::ZERO).unwrap();
        assert_eq!((stats.files, stats.complete), (3, true));
    }

    /// A walk whose lease another walk took (it expired while this one was listing, and the other walk ran to its end)
    /// ends with an error at its next renewal, and gives nothing back: the lease is not its own. Nothing of it is left
    /// behind, and the next walk runs.
    #[test]
    fn a_walk_whose_lease_was_taken_ends_with_an_error() {
        let path = on_disk("scan-lease-taken");
        let tree = [("", vec![file("a.jpg", 1, 1), file("b.jpg", 2, 1)])];
        let (aging, taking) = (path.clone(), path.clone());
        let mut lister = FakeListing::new(1, &tree).on_listing("", move || {
            state::age_leases(&open(&aging), pg_catalog::SCAN_LEASE_SECS + 1).unwrap();
            let other = [("", vec![file("z.jpg", 9, 1)])];
            let mut c = open(&taking);
            let done = walk(&mut c, &fake_root(), &Control::new(), &mut FakeListing::new(1, &other));
            assert!(done.is_ok(), "the other walk takes the expired lease and completes");
        });
        let mut c = open(&path);
        let failed = walk_in_batches(&mut c, &fake_root(), &Control::new(), &mut lister, BATCH, Duration::ZERO);
        assert!(failed.is_err(), "a walk whose lease was taken ends with an error");
        let next = walk(&mut open(&path), &fake_root(), &Control::new(), &mut FakeListing::new(1, &tree)).unwrap();
        assert!(next.complete, "the next walk is not refused");
    }

    /// A batch of `BATCH` new files is the longest hold; the bound is several times what it takes, so a listing or a
    /// pause inside a hold (seconds, or for ever) fails the test, and a slow machine doesn't.
    const HOLD_BOUND: Duration = Duration::from_secs(3);

    /// Every write transaction of a walk of 200 000 files, the pairing and the missing marks included, holds the write
    /// lock for at most this long (wall clock, in a debug build, whose SQLite is optimized: `profile.dev.package."*"`).
    #[test]
    fn no_write_transaction_of_a_200_000_file_walk_holds_the_lock_long() {
        let mut c = catalog();
        let mut tree: Vec<(String, Vec<FakeEntry>)> = Vec::with_capacity(2001);
        let folders = 2000u64;
        tree.push((String::new(), (0..folders).map(|f| folder(&format!("f{f:04}"), 1_000_000 + f)).collect()));
        for f in 0..folders {
            let files = (0..100u64).map(|i| file(&format!("{i:03}.jpg"), 1 + f * 100 + i, 1)).collect();
            tree.push((format!("f{f:04}"), files));
        }
        let holds = WriteHolds::watch();
        let stats = scan_fake(&mut c, &tree, 1);
        let held = holds.take();
        assert_eq!((stats.files, stats.complete), (200_000, true));
        assert!(held.len() >= 200_000 / BATCH, "the batches were written in {} transactions", held.len());
        let longest = held.iter().max().copied().unwrap_or_default();
        assert!(longest <= HOLD_BOUND, "a write transaction held the lock for {longest:?}");
    }

    #[test]
    fn an_unreadable_folder_is_in_the_folder_ledger_until_a_walk_reads_it() {
        let path = on_disk("scan-ledger");
        let mut c = open(&path);
        // sub has no listing: unreadable.
        let unreadable = [("", vec![file("a.jpg", 7, 1), folder("sub", 20)])];
        scan_fake(&mut c, &unreadable, 1);
        let failed = c.failed_folders(10).unwrap();
        let [one] = &failed[..] else { panic!("one failed folder: {}", failed.len()) };
        assert_eq!((one.path.parts.last(), one.failure), (Some(&Name::from("sub")), FolderFailure::Unreadable));
        // Readable again: the next complete walk sweeps it.
        scan_fake(&mut c, &[("", vec![file("a.jpg", 7, 1), folder("sub", 20)]), ("sub", vec![])], 1);
        assert!(c.failed_folders(10).unwrap().is_empty());
    }

    /// The walk's pacing with `workers` workers, and batches of 4 entries so the tree crosses them.
    fn paced(workers: usize) -> Pacing {
        Pacing { batch: 4, renew_every: RENEW_EVERY, workers }
    }

    /// A tree with what the walk has to judge: subfolders on the root's device, a reparse folder, a refused mount on
    /// another device, a loop back to an ancestor, a file seen twice, and a folder with more files than a batch.
    fn judged_tree() -> Vec<(&'static str, Vec<FakeEntry>)> {
        let mut deep: Vec<FakeEntry> = (0..9).map(|i| file(&format!("d{i}.jpg"), 100 + i, 1)).collect();
        deep.push(folder("further", 60));
        vec![
            (
                "",
                vec![
                    folder("a", 20),
                    folder("b", 21),
                    FakeEntry { reparse: true, ..folder("rp", 22) },
                    FakeEntry { volume: Some(2), ..folder("mnt", 30) },
                    file("top.jpg", 3, 1),
                ],
            ),
            ("a", vec![file("a.jpg", 1, 1), folder("back", 20), folder("deep", 23)]),
            ("a/deep", deep),
            ("a/deep/further", vec![file("f.jpg", 200, 1)]),
            ("b", vec![file("b.jpg", 2, 1), file("twin.jpg", 1, 1)]),
        ]
    }

    /// The walk's stats and the catalog's file count and missing count after one walk of `tree` in a new catalog.
    fn walk_of_tree(prefix: &str, tree: &[(&str, Vec<FakeEntry>)], workers: usize) -> (ScanStats, i64, i64) {
        let path = on_disk(prefix);
        let mut c = open(&path);
        let mut lister = FakeListing::new(1, tree).refusing(pg_platform::native::PATH_RULES, "mnt");
        let stats = walk_paced(&mut c, &fake_root(), &Control::new(), &mut lister, paced(workers)).unwrap();
        let status = c.status().unwrap();
        (stats, status.files, status.missing)
    }

    #[test]
    fn the_walk_record_names_its_steps_and_what_no_step_covers() {
        let ((_, files, _), records) = pg_perf::capture(|| walk_of_tree("walk-steps", &judged_tree(), 0));
        assert!(files > 0, "the walk saw files");
        let walk = records.iter().find(|r| r["name"] == "scan.walk").expect("the walk's record");
        let fields = &walk["fields"];
        assert!(fields["unaccounted_ms"].as_f64().is_some_and(|ms| ms >= 0.0), "{walk}");
        assert!(fields["scan.consume"]["count"].as_u64().is_some_and(|n| n > 0), "{walk}");
        assert!(fields["scan.mark_seen"]["count"].as_u64().is_some_and(|n| n > 0), "{walk}");
        assert!(fields["scan.write_batch"]["count"].as_u64().is_some_and(|n| n > 0), "{walk}");
    }

    #[test]
    fn workers_list_what_the_walk_alone_would() {
        let alone = walk_of_tree("judged-alone", &judged_tree(), 0);
        let with = walk_of_tree("judged-workers", &judged_tree(), 4);
        let (a, af, am) = &alone;
        let (w, wf, wm) = &with;
        assert!(a.complete && w.complete, "both walks complete");
        assert_eq!(
            (a.folders, a.files, a.changed, a.duplicates, a.loops, a.skipped_reparse, a.refused_mounts),
            (w.folders, w.files, w.changed, w.duplicates, w.loops, w.skipped_reparse, w.refused_mounts),
            "the same judgments"
        );
        assert_eq!((af, am), (wf, wm), "the same rows");
        assert_eq!(a.loops, 1, "the loop back to a is counted once");
        assert_eq!(a.refused_mounts, 1, "the refused mount is counted once");
        assert_eq!(a.duplicates, 1, "the twin is a second path to file 1");
    }

    #[test]
    fn a_panic_in_a_listing_is_raised_by_the_walk_with_its_payload() {
        let path = on_disk("listing-panics");
        let mut c = open(&path);
        let mut lister = FakeListing::new(1, &judged_tree()).on_listing("a", || panic!("a listing's own panic"));
        let ctl = Control::new();
        let raised = std::panic::catch_unwind(AssertUnwindSafe(|| {
            walk_paced(&mut c, &fake_root(), &ctl, &mut lister, paced(4))
        }));
        let payload = raised.expect_err("the walk raises the listing's panic");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"a listing's own panic"));
    }

    #[test]
    fn a_stop_with_workers_listing_marks_nothing_missing() {
        let mut tree = three_folders();
        let path = on_disk("stop-with-workers");
        let mut c = open(&path);
        let mut first = FakeListing::new(1, &tree);
        assert!(walk_paced(&mut c, &fake_root(), &Control::new(), &mut first, paced(4)).unwrap().complete);
        // c.jpg is gone; the walk is stopped as it lists b, with the workers listing ahead of it.
        tree[3].1.clear();
        let ctl = Arc::new(Control::new());
        let stopping = ctl.clone();
        let mut lister = FakeListing::new(1, &tree).on_listing("b", move || stopping.request_stop());
        let stopped = walk_paced(&mut c, &fake_root(), &ctl, &mut lister, paced(4)).unwrap();
        assert!(!stopped.complete);
        assert_eq!(stopped.missing, 0, "a stopped walk marks nothing missing");
        assert_eq!(c.status().unwrap().missing, 0, "c.jpg is not missing after a stopped walk");
    }

    #[test]
    fn eight_workers_judge_an_unreadable_tree_as_the_walk_alone_does() {
        let mut tree = judged_tree();
        // A folder the lister cannot open: the walk finds it, and the workers' listing of it fails too.
        tree[0].1.push(folder("gone", 40));
        let alone = walk_of_tree("unreadable-alone", &tree, 0);
        let eight = walk_of_tree("unreadable-eight", &tree, 8);
        let (a, af, am) = &alone;
        let (w, wf, wm) = &eight;
        assert!(a.complete && w.complete, "both walks complete");
        assert_eq!(
            (a.folders, a.files, a.changed, a.duplicates, a.loops, a.skipped_reparse, a.refused_mounts, a.errors),
            (w.folders, w.files, w.changed, w.duplicates, w.loops, w.skipped_reparse, w.refused_mounts, w.errors),
            "the same judgments"
        );
        assert_eq!((af, am), (wf, wm), "the same rows");
    }

    #[test]
    fn a_pause_with_workers_listing_holds_no_lock_and_resumes_with_the_same_judgments() {
        let path = on_disk("pause-workers");
        let ctl = Arc::new(Control::new());
        let (listed, at_b) = mpsc::channel();
        let pausing = ctl.clone();
        let mut lister = FakeListing::new(1, &judged_tree())
            .refusing(pg_platform::native::PATH_RULES, "mnt")
            .on_listing("b", move || {
                pausing.set_paused(true);
                let _ = listed.send(());
            });
        let (walk_ctl, walk_path) = (ctl.clone(), path.clone());
        let walking = std::thread::spawn(move || {
            walk_paced(&mut open(&walk_path), &fake_root(), &walk_ctl, &mut lister, paced(4)).map_err(|e| e.to_string())
        });
        at_b.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(another_can_write(&path, Duration::from_secs(2)), "a paused walk held the write lock");
        ctl.set_paused(false);
        let stats = walking.join().unwrap().unwrap();
        let (alone, _, _) = walk_of_tree("pause-alone", &judged_tree(), 0);
        assert!(stats.complete);
        assert_eq!(
            (stats.folders, stats.files, stats.changed, stats.duplicates, stats.loops, stats.refused_mounts),
            (alone.folders, alone.files, alone.changed, alone.duplicates, alone.loops, alone.refused_mounts),
            "the same judgments as the walk with no workers"
        );
    }

    #[test]
    fn a_stop_during_a_pause_with_workers_parked_returns_at_once() {
        let path = on_disk("stop-paused-workers");
        let ctl = Arc::new(Control::new());
        let (listed, at_b) = mpsc::channel();
        let pausing = ctl.clone();
        let mut lister = FakeListing::new(1, &judged_tree()).on_listing("b", move || {
            pausing.set_paused(true);
            let _ = listed.send(());
        });
        let (walk_ctl, walk_path) = (ctl.clone(), path.clone());
        let (done, returned) = mpsc::channel();
        std::thread::spawn(move || {
            let result = walk_paced(&mut open(&walk_path), &fake_root(), &walk_ctl, &mut lister, paced(4));
            let _ = done.send(result.map(|s| s.complete).map_err(|e| e.to_string()));
        });
        at_b.recv_timeout(Duration::from_secs(30)).unwrap();
        ctl.request_stop();
        let complete = returned.recv_timeout(Duration::from_secs(10)).expect("the stopped walk returns");
        assert_eq!(complete, Ok(false), "a stopped walk is not complete");
    }

    #[test]
    fn a_full_bound_of_held_listings_still_lets_the_walk_take_its_next_folder() {
        // One folder whose listing alone reaches the bound (AHEAD entries held), then a folder the walk takes itself.
        let wide: Vec<FakeEntry> = (0..50_001u64).map(|i| file(&format!("w{i}.jpg"), 1000 + i, 1)).collect();
        let tree: Vec<(&str, Vec<FakeEntry>)> =
            vec![("", vec![folder("wide", 20), folder("b", 21)]), ("wide", wide), ("b", vec![file("b.jpg", 2, 1)])];
        let (stats, files, _) = walk_of_tree("bound-held", &tree, 4);
        assert!(stats.complete);
        assert_eq!((stats.files, files), (50_002, 50_002), "every file of the wide folder and of b");
    }
}
