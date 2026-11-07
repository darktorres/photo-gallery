//! The library: the folders the user added, and the background work that keeps the catalog and the files in step. The
//! job (`pg_library::runner`) scans, imports, makes thumbnails and saves cropped copies; the writer
//! (`pg_writeback`) puts ratings and turns into the files. Both announce what changed on one bus of
//! [`Change`]s, so a front end waits for those instead of polling; the progress and the writer's state are read here.
//!
//! The library starts on first use ([`App::library`]), at the thumbnail height the grid asks for. Its folder operations
//! are on [`App`]: they work on the catalog whether or not the job has started.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pg_catalog::Root;
use pg_jobs::control::Progress;
use pg_jobs::events::{Bus, Change};
use pg_library::runner::{Generation, Messages, Runner};
use pg_model::ids::{FileId, RootId};
use pg_model::rules::{Path as RulePath, PathRules};
use pg_scan::normalize_path;
use pg_writeback::{WriteStatus, Writer};

use crate::app::App;
use crate::error::{Error, Result, failed};

/// How long a stop waits for the job's file (or batch) in progress. A file that takes longer to read is left to end
/// with the process, and its last changes are merged at the next open.
const JOB_STOP_WAIT: Duration = Duration::from_secs(5);

/// The job and the writer, and the bus they announce on.
pub struct Library {
    runner: Runner,
    writer: Writer,
    changes: Arc<Bus<Change>>,
}

/// What the library is doing now, for the status line and the progress.
pub struct Status {
    pub progress: Progress,
    pub paused: bool,
    pub writes: WriteStatus,
    /// Files turned by the writer so far (their images and sizes changed).
    pub turned: u64,
    pub generation: Generation,
    /// Rounds of the job so far (for measuring).
    pub rounds: u64,
}

impl Library {
    /// Start the job and the writer; they first finish whatever is pending.
    pub(crate) fn start(app: &App, thumb_height: u32) -> Result<Library> {
        let changes = Arc::new(Bus::new(256));
        let catalog_path = app.config.catalog_path();
        let runner = Runner::start(catalog_path.clone(), app.thumbs, thumb_height, changes.clone()).map_err(failed)?;
        let writer = Writer::start(catalog_path, app.thumbs, changes.clone()).map_err(failed)?;
        Ok(Library { runner, writer, changes })
    }

    /// The bus the job and the writer announce on. Subscribe to it; a slow subscriber loses the oldest changes (counted),
    /// and what they announce is read again from [`Library::status`] and [`Library::take_messages`].
    pub fn changes(&self) -> Arc<Bus<Change>> {
        self.changes.clone()
    }

    /// Stop the writer, then the job, which stops after the file (or batch) it has. Returns false when the job was still
    /// running after the wait.
    pub(crate) fn stop(&self) -> bool {
        self.writer.stop();
        self.runner.stop(JOB_STOP_WAIT)
    }

    /// Ratings and turns were queued for the files.
    pub fn wake_writer(&self) {
        self.writer.wake();
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
        Status {
            progress: self.runner.progress(),
            paused: self.runner.is_paused(),
            writes: self.writer.status(),
            turned: self.writer.turned(),
            generation: self.runner.generation(),
            rounds: self.runner.rounds(),
        }
    }

    /// The notices the job has made since the last call: the newest ones kept, and how many more were dropped.
    pub fn take_messages(&self) -> Messages {
        self.runner.take_messages()
    }
}

impl App {
    /// The library, started at `thumb_height` device pixels on first use (later calls ignore the height: set it with
    /// [`Library::set_thumb_height`]). Fails when a thread can't start; the next call tries again.
    pub fn library(&self, thumb_height: u32) -> Result<&Library> {
        if let Some(library) = self.library_cell.get() {
            return Ok(library);
        }
        let library = Library::start(self, thumb_height)?;
        Ok(self.library_cell.get_or_init(|| library))
    }

    /// Ratings and turns were queued for the files; a no-op until the library has started.
    pub fn wake_writer(&self) {
        if let Some(library) = self.library_cell.get() {
            library.wake_writer();
        }
    }

    /// Save cropped copies into `folder`; a no-op until the library has started.
    pub fn save_crops(&self, ids: Vec<FileId>, folder: PathBuf) {
        if let Some(library) = self.library_cell.get() {
            library.save_crops(ids, folder);
        }
    }

    /// The library's folders, each with its id and its path (under the catalog's rules), as the catalog has them.
    pub fn roots(&self) -> Result<Vec<Root>> {
        self.with_catalog(|c| c.roots())
    }

    /// Add a folder to the library, at the path given (exact): normalized, and scanned. Returns the normalized folder's
    /// display path and whether it was new; a folder already in the library is rescanned.
    pub fn add_folder(&self, path: &std::path::Path) -> Result<(String, bool)> {
        let root = normalize_path(path).map_err(failed)?;
        // A folder the scanner refuses is not added: it would stay in the library with a notice on every scan.
        if let Err(refused) = pg_scan::check_folder(PathRules::NATIVE, &root) {
            return Err(failed(format!("{} can't be added: {refused}", PathRules::NATIVE.display(&root))));
        }
        let added = self.with_catalog(|c| c.add_root(&root))?;
        if let Some(library) = self.library_cell.get() {
            library.scan(vec![root.clone()]);
        }
        Ok((PathRules::NATIVE.display(&root), added))
    }

    /// Remove a folder from the library, by its id (from [`App::roots`]). A scan of it in progress stops; the files it
    /// had stay until the next scan.
    pub fn remove_folder(&self, id: RootId) -> Result<()> {
        self.with_catalog(|c| c.remove_root(id).map(|_| ()))?;
        if let Some(library) = self.library_cell.get() {
            library.scan(Vec::new());
        }
        Ok(())
    }

    /// Scan every library folder again (new, changed, moved and deleted files).
    pub fn rescan(&self) -> Result<()> {
        let roots: Vec<RulePath> = self.with_catalog(|c| Ok(c.roots()?.into_iter().map(|r| r.path).collect()))?;
        if let Some(library) = self.library_cell.get() {
            library.scan(roots);
        }
        Ok(())
    }

    /// Turn `file_id` by `turns` quarter turns (clockwise, as the grid shows it) and queue the rotation for the file.
    pub fn turn(&self, file_id: FileId, turns: pg_model::turns::Turns) -> Result<()> {
        self.with_catalog(|c| pg_catalog::writes::queue_rotation(c, file_id, turns))?;
        self.wake_writer();
        Ok(())
    }

    /// Stop the library; an error when the job was still running after its wait.
    pub(crate) fn stop_library(&self) -> Result<()> {
        match self.library_cell.get() {
            Some(library) if !library.stop() => Err(Error::Failed(
                "background job still running at exit: its last changes are merged at the next open".to_string(),
            )),
            _ => Ok(()),
        }
    }
}
