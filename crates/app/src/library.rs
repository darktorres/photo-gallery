//! The library: the folders the user added, and the background work that keeps the catalog and the files in step. The
//! job (`pg_library::runner`) scans, imports, makes thumbnails and saves cropped copies; the writer
//! (`pg_writeback`) puts ratings and turns into the files. Both announce what changed on one bus of
//! [`Change`]s, so a front end waits for those instead of polling; the progress and the writer's state are read here.
//!
//! The library starts on first use ([`LibraryService::start`]), at the thumbnail height the grid asks for. Its folder
//! operations are on [`LibraryService`]: they work on the catalog whether or not the job has started.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use pg_catalog::Root;
use pg_jobs::control::Progress;
use pg_jobs::events::{Bus, Change};
use pg_library::runner::{Generation, Messages, Runner};
use pg_model::ids::{FileId, RootId};
use pg_model::rules::{Path as RulePath, PathRules};
use pg_scan::normalize_path;
use pg_thumbcache::ThumbCache;
use pg_writeback::{WriteStatus, Writer};

use crate::catalogs::Catalogs;
use crate::error::{Error, Result, failed};

/// How long a stop waits for the job's file (or batch) in progress. A file that takes longer to read is left to end
/// with the process, and its last changes are merged at the next open.
const JOB_STOP_WAIT: Duration = Duration::from_secs(5);

/// A panic outside a writer job ends the writer's thread (`Writer::is_running`). The writer is started again after a
/// wait that doubles with each restart in a row, and at most `MAX_RESTARTS` times in a row: after that it is given up,
/// and the status says so. A writer that has run for `RESTART_RESET` is healthy again, so its count starts over. A
/// panic that repeats at every start is thereby stopped after a few seconds, not spun on.
const MAX_RESTARTS: u32 = 3;
const RESTART_WAIT: Duration = Duration::from_secs(5);
const RESTART_RESET: Duration = Duration::from_secs(60);

/// The job and the writer, and the bus they announce on.
pub struct Library {
    runner: Runner,
    /// The writer, restarted by [`Library::tend`] when its thread has ended.
    writer: Mutex<WriterSlot>,
    /// What a restart needs to start the writer again.
    catalog_path: PathBuf,
    thumbs: &'static ThumbCache,
    changes: Arc<Bus<Change>>,
}

/// The writer and its restarts.
struct WriterSlot {
    writer: Writer,
    /// Restarts in a row (the count of the writer's starts after its first, until it has run for `RESTART_RESET`).
    restarts: u32,
    /// When the current writer started.
    started: Instant,
    /// When the next restart may happen (set when one is due, after its wait).
    retry_at: Option<Instant>,
    /// Restarts were exhausted: the writer is not started again until the app is.
    given_up: bool,
    /// The library is stopping: the writer is not started again.
    stopping: bool,
}

/// What the library is doing now, for the status line and the progress.
pub struct Status {
    pub progress: Progress,
    pub paused: bool,
    pub writes: WriteStatus,
    /// Files turned by the writer so far (their images and sizes changed).
    pub turned: u64,
    /// The writer died too often (a panic outside its jobs, at every start) and is no longer started: the changes stay
    /// queued in the catalog, and are written when the app starts again.
    pub writer_stopped: bool,
    pub generation: Generation,
    /// Rounds of the job so far (for measuring).
    pub rounds: u64,
}

impl Library {
    /// Start the job and the writer; they first finish whatever is pending.
    pub(crate) fn start(catalog_path: PathBuf, thumbs: &'static ThumbCache, thumb_height: u32) -> Result<Library> {
        let changes = Arc::new(Bus::new(256));
        let runner = Runner::start(catalog_path.clone(), thumbs, thumb_height, changes.clone()).map_err(failed)?;
        let writer = Writer::start(catalog_path.clone(), thumbs, changes.clone()).map_err(failed)?;
        let slot = WriterSlot {
            writer,
            restarts: 0,
            started: Instant::now(),
            retry_at: None,
            given_up: false,
            stopping: false,
        };
        Ok(Library { runner, writer: Mutex::new(slot), catalog_path, thumbs, changes })
    }

    fn writer(&self) -> MutexGuard<'_, WriterSlot> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts the writer again when its thread has ended (`Writer::is_running`), once its wait is over (see
    /// `MAX_RESTARTS`). A writer that has run long enough clears the count.
    fn tend(&self, slot: &mut WriterSlot) {
        if slot.given_up || slot.stopping {
            return;
        }
        if slot.writer.is_running() {
            if slot.started.elapsed() >= RESTART_RESET {
                slot.restarts = 0;
            }
            return;
        }
        if slot.restarts >= MAX_RESTARTS {
            slot.given_up = true;
            return;
        }
        let now = Instant::now();
        let due = *slot.retry_at.get_or_insert(now + RESTART_WAIT * 2u32.pow(slot.restarts));
        if now < due {
            return;
        }
        slot.retry_at = None;
        slot.restarts += 1;
        if let Ok(writer) = Writer::start(self.catalog_path.clone(), self.thumbs, self.changes.clone()) {
            // The old writer's thread has ended, so dropping it doesn't wait.
            slot.writer = writer;
            slot.started = now;
            slot.writer.wake();
        }
    }

    /// The bus the job and the writer announce on. Subscribe to it; a slow subscriber loses the oldest changes (counted),
    /// and what they announce is read again from [`Library::status`] and [`Library::take_messages`].
    pub fn changes(&self) -> Arc<Bus<Change>> {
        self.changes.clone()
    }

    /// Stop the writer, then the job, which stops after the file (or batch) it has. Returns false when the job was still
    /// running after the wait.
    pub(crate) fn stop(&self) -> bool {
        {
            let mut slot = self.writer();
            slot.stopping = true;
            slot.writer.stop();
        }
        self.runner.stop(JOB_STOP_WAIT)
    }

    /// Ratings and turns were queued for the files.
    pub fn wake_writer(&self) {
        let mut slot = self.writer();
        self.tend(&mut slot);
        slot.writer.wake();
    }

    /// Wake the job, which re-analyzes files whose content changed (a turned file's analysis is stale).
    pub fn wake_job(&self) {
        self.runner.wake();
    }

    /// Save cropped copies of these files' borders into `folder`, first thing in the background job.
    pub fn save_crops(&self, ids: Vec<FileId>, folder: PathBuf) {
        self.runner.save_crops(ids, folder);
    }

    /// Scan these folders (the job skips folders no longer in the library).
    pub fn scan(&self, roots: Vec<RulePath>) {
        self.runner.scan(roots);
    }

    pub fn set_thumb_height(&self, height: u32) {
        self.runner.set_thumb_height(height);
    }

    pub fn set_paused(&self, paused: bool) {
        self.runner.set_paused(paused);
    }

    pub fn is_paused(&self) -> bool {
        self.runner.is_paused()
    }

    /// The job's and the writer's state now.
    pub fn status(&self) -> Status {
        let mut slot = self.writer();
        self.tend(&mut slot);
        Status {
            progress: self.runner.progress(),
            paused: self.runner.is_paused(),
            writes: slot.writer.status(),
            turned: slot.writer.turned(),
            writer_stopped: slot.given_up,
            generation: self.runner.generation(),
            rounds: self.runner.rounds(),
        }
    }

    /// The notices the job has made since the last call: the newest ones kept, and how many more were dropped.
    pub fn take_messages(&self) -> Messages {
        self.runner.take_messages()
    }
}

/// The library's service: the job and the writer, started on first use, and the folder operations, which work on the catalog
/// whether or not the job has started.
pub struct LibraryService {
    catalog_path: PathBuf,
    catalogs: Arc<Catalogs>,
    thumbs: &'static ThumbCache,
    /// The started job and writer; set once, by the first [`LibraryService::start`].
    cell: OnceLock<Library>,
}

impl LibraryService {
    pub(crate) fn new(catalog_path: PathBuf, catalogs: Arc<Catalogs>, thumbs: &'static ThumbCache) -> LibraryService {
        LibraryService { catalog_path, catalogs, thumbs, cell: OnceLock::new() }
    }

    /// The library, started at `thumb_height` device pixels on first use (later calls ignore the height: set it with
    /// [`Library::set_thumb_height`]). Fails when a thread can't start; the next call tries again.
    pub fn start(&self, thumb_height: u32) -> Result<&Library> {
        if let Some(library) = self.cell.get() {
            return Ok(library);
        }
        let library = Library::start(self.catalog_path.clone(), self.thumbs, thumb_height)?;
        Ok(self.cell.get_or_init(|| library))
    }

    /// The started library, if it has started. The folder operations scan through it, and do nothing with it until then.
    fn running(&self) -> Option<&Library> {
        self.cell.get()
    }

    /// Ratings and turns were queued for the files; a no-op until the library has started.
    pub fn wake_writer(&self) {
        if let Some(library) = self.running() {
            library.wake_writer();
        }
    }

    /// Save cropped copies into `folder`; a no-op until the library has started.
    pub fn save_crops(&self, ids: Vec<FileId>, folder: PathBuf) {
        if let Some(library) = self.running() {
            library.save_crops(ids, folder);
        }
    }

    /// The library's folders, each with its id and its path (under the catalog's rules), as the catalog has them.
    pub fn roots(&self) -> Result<Vec<Root>> {
        self.catalogs.with_catalog(|c| c.roots())
    }

    /// Add a folder to the library, at the path given (exact): normalized, and scanned. Returns the normalized folder's
    /// display path and whether it was new; a folder already in the library is rescanned.
    pub fn add_folder(&self, path: &std::path::Path) -> Result<(String, bool)> {
        let root = normalize_path(path).map_err(failed)?;
        // A folder the scanner refuses is not added: it would stay in the library with a notice on every scan.
        if let Err(refused) = pg_scan::check_folder(PathRules::NATIVE, &root) {
            return Err(Error::FolderRefused { folder: PathRules::NATIVE.display(&root), reason: refused.reason });
        }
        let added = self.catalogs.with_catalog(|c| c.add_root(&root))?;
        if let Some(library) = self.running() {
            library.scan(vec![root.clone()]);
        }
        Ok((PathRules::NATIVE.display(&root), added))
    }

    /// Remove a folder from the library, by its id (from [`LibraryService::roots`]). A scan of it in progress stops. The
    /// files it had are marked missing at once (`Catalog::remove_root`), not deleted: adding the folder back brings them
    /// back with their analyses and ratings.
    pub fn remove_folder(&self, id: RootId) -> Result<()> {
        self.catalogs.with_catalog(|c| c.remove_root(id).map(|_| ()))?;
        if let Some(library) = self.running() {
            library.scan(Vec::new());
        }
        Ok(())
    }

    /// Scan every library folder again (new, changed, moved and deleted files).
    pub fn rescan(&self) -> Result<()> {
        let roots: Vec<RulePath> =
            self.catalogs.with_catalog(|c| Ok(c.roots()?.into_iter().map(|r| r.path).collect()))?;
        if let Some(library) = self.running() {
            library.scan(roots);
        }
        Ok(())
    }

    /// Turn `file_id` by `turns` quarter turns (clockwise, as the grid shows it) and queue the rotation for the file.
    pub fn turn(&self, file_id: FileId, turns: pg_model::turns::Turns) -> Result<()> {
        self.catalogs.with_catalog(|c| pg_catalog::writes::queue_rotation(c, file_id, turns))?;
        self.wake_writer();
        Ok(())
    }

    /// Stop the library; an error when the job was still running after its wait.
    pub fn stop(&self) -> Result<()> {
        match self.running() {
            Some(library) if !library.stop() => Err(Error::Failed(
                "background job still running at exit: its last changes are merged at the next open".to_string(),
            )),
            _ => Ok(()),
        }
    }
}
