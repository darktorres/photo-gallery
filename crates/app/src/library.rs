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
use pg_jobs::NoticeSink;
use pg_jobs::events::Bus;
use pg_library::folders::AddError;
use pg_library::mask::PassMask;
use pg_library::runner::{Runner, RunnerConfig};
use pg_library::shared::{FeatureCell, Generation, HeightCell};
use pg_model::features::Features;
use pg_model::ids::{FileId, RootId};
use pg_model::job::Change;
use pg_model::notice::{Notice, Trigger};
use pg_model::progress::LaneProgress;
use pg_model::progress::Progress;
use pg_model::removal::Begun;
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
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

/// How a library starts: the job's settings (`RunnerConfig`), and whether the writer runs. The app runs the writer. A headless
/// run on a real catalog does not, so the ratings and turns queued in the catalog are written into the files only when a run
/// asks for it.
#[derive(Clone, Debug)]
pub struct StartConfig {
    /// The job's settings.
    pub runner: RunnerConfig,
    /// Whether the writer runs.
    pub writer: bool,
    /// Where the writer fails, for the tests of what a failing writer does to a run (`pg-driver`'s `fault` feature). `None`
    /// is the writer as the app runs it.
    #[cfg(feature = "fault")]
    pub writer_fault: Option<pg_writeback::fault::Fault>,
}

impl Default for StartConfig {
    fn default() -> StartConfig {
        StartConfig {
            runner: RunnerConfig::default(),
            writer: true,
            #[cfg(feature = "fault")]
            writer_fault: None,
        }
    }
}

/// The writer as `start` asks for it: with the test fault it names, when there is one.
fn spawn_writer(
    catalog_path: PathBuf,
    thumbs: Arc<ThumbCache>,
    changes: Arc<Bus<Change>>,
    notices: Arc<dyn NoticeSink>,
    start: &StartConfig,
) -> pg_writeback::Result<Writer> {
    #[cfg(feature = "fault")]
    if let Some(fault) = start.writer_fault {
        return Writer::start_with_fault(catalog_path, thumbs, changes, fault, notices);
    }
    let _ = start;
    Writer::start(catalog_path, thumbs, changes, notices)
}

/// The started job, and its writer when there is one.
pub struct Library {
    runner: Runner,
    /// The writer, restarted by [`Library::tend`] when its thread has ended. `None` when the library starts without one
    /// (`StartConfig::writer`): then nothing queued in the catalog is written into the files by this run.
    writer: Option<Mutex<WriterSlot>>,
    /// What a restart needs to start the writer again.
    catalog_path: PathBuf,
    thumbs: Arc<ThumbCache>,
    changes: Arc<Bus<Change>>,
    /// Where the job's and the writer's notices go (the inbox): the writer posts its failures there, and the library its
    /// stop ([`Library::tend`]).
    notices: Arc<dyn NoticeSink>,
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

/// The job's rows now: the crop thread, and each lane (`LaneProgress`, in the lanes' order).
pub struct LaneLines {
    /// The crop thread's progress.
    pub crop: Progress,
    /// The time since the crop last had work (`LaneProgress::since_work`).
    pub crop_since_work: Option<Duration>,
    /// Each lane's progress, in the lanes' order.
    pub lanes: Vec<LaneProgress>,
}

/// What the library is doing now, for the status line and the progress: a snapshot the app's `Library::status` takes.
pub struct LibraryActivity {
    /// The library's one progress: the crop's when it works, else the first lane that works (`lane_lines` has each row's).
    pub progress: Progress,
    /// Whether the whole run is paused.
    pub paused: bool,
    /// The queue of writes into the files, as the app shows it.
    pub writes: WriteStatus,
    /// Files turned by the writer so far (their images and sizes changed).
    pub turned: u64,
    /// The writer died too often (a panic outside its jobs, at every start) and is no longer started: the changes stay
    /// queued in the catalog, and are written when the app starts again.
    pub writer_stopped: bool,
    /// The writer's thread is not running now: it ended, and until the app starts it again (after a wait) nothing is written.
    /// Set at once, where `writer_stopped` is set only once the app gives up restarting it.
    pub writer_down: bool,
    /// The generation of the library's state, which changes when its contents do.
    pub generation: Generation,
    /// Rounds of the job so far (for measuring).
    pub rounds: u64,
}

impl Library {
    /// Start the job and the writer; they first finish whatever is pending. The writer's states an earlier run kept
    /// (`write-stopped`, `write-failed`) are cleared first: a new writer starts running and with no error, so neither
    /// would otherwise see a change that clears it. Cleared before the writer starts, so a failure of its own, posted
    /// after, is kept (the inbox writes in the order posted).
    pub(crate) fn start(
        catalog_path: PathBuf,
        thumbs: Arc<ThumbCache>,
        heights: HeightCell,
        features: FeatureCell,
        notices: Arc<dyn NoticeSink>,
        start: StartConfig,
    ) -> Result<Library> {
        notices.resolve(Trigger::Status);
        let changes = Arc::new(Bus::new(256));
        let runner = Runner::start(
            catalog_path.clone(),
            thumbs.clone(),
            heights,
            features,
            changes.clone(),
            notices.clone(),
            start.runner.clone(),
        )
        .map_err(failed)?;
        let writer = if start.writer {
            let writer = spawn_writer(catalog_path.clone(), thumbs.clone(), changes.clone(), notices.clone(), &start)
                .map_err(failed)?;
            Some(Mutex::new(WriterSlot {
                writer,
                restarts: 0,
                started: Instant::now(),
                retry_at: None,
                given_up: false,
                stopping: false,
            }))
        } else {
            None
        };
        Ok(Library { runner, writer, catalog_path, thumbs, changes, notices })
    }

    /// Whether the job has nothing left to do (`Runner::quiescent`), and every write queued in the catalog is written.
    pub fn quiescent(&self) -> bool {
        self.runner.quiescent()
    }

    /// The passes each round runs (`Runner::passes`).
    pub fn passes(&self) -> PassMask {
        self.runner.passes()
    }

    /// Sets the passes each round runs (`Runner::set_passes`).
    pub fn set_passes(&self, passes: PassMask) {
        self.runner.set_passes(passes);
    }

    /// The writer's slot, or `None` when this library runs without a writer.
    fn writer(&self) -> Option<MutexGuard<'_, WriterSlot>> {
        self.writer.as_ref().map(|slot| slot.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Starts the writer again when its thread has ended (`Writer::is_running`), once its wait is over (see
    /// `MAX_RESTARTS`). A writer that has run long enough clears the count. When the restarts are exhausted the writer
    /// is given up, and `write-stopped` is posted there, once: a stop at shutdown, or a panic a restart recovers from,
    /// posts nothing.
    ///
    /// Lock order: called with the writer slot held (`slot`), and a post takes only the inbox's work lock, so the order
    /// is slot → `Work` (`crate::inbox::Inbox::push`), never the reverse.
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
            // Slot → Work (see above): the post only queues.
            self.notices.post(Notice::WriteStopped);
            return;
        }
        let now = Instant::now();
        let due = *slot.retry_at.get_or_insert(now + RESTART_WAIT * 2u32.pow(slot.restarts));
        if now < due {
            return;
        }
        slot.retry_at = None;
        slot.restarts += 1;
        if let Ok(writer) =
            Writer::start(self.catalog_path.clone(), self.thumbs.clone(), self.changes.clone(), self.notices.clone())
        {
            // The old writer's thread has ended, so dropping it doesn't wait.
            slot.writer = writer;
            slot.started = now;
            slot.writer.wake();
        }
    }

    /// The bus the job and the writer announce on. Subscribe to it; a slow subscriber loses the oldest changes (counted),
    /// and what they announce is read again from [`Library::status`]. What the job tells the user goes to the inbox.
    pub fn changes(&self) -> Arc<Bus<Change>> {
        self.changes.clone()
    }

    /// Stop the writer, then the job, which stops after the file (or batch) it has. Returns false when the job was still
    /// running after the wait.
    pub(crate) fn stop(&self) -> bool {
        if let Some(mut slot) = self.writer() {
            slot.stopping = true;
            // A writer still in a copy is finished from its temp at the next start: its wait isn't in the result.
            slot.writer.stop(JOB_STOP_WAIT);
        }
        self.runner.stop(JOB_STOP_WAIT)
    }

    /// Ratings and turns were queued for the files. Nothing is written when the library runs without a writer.
    pub fn wake_writer(&self) {
        if let Some(mut slot) = self.writer() {
            self.tend(&mut slot);
            slot.writer.wake();
        }
    }

    /// Wake the job for the files the writer turned since it was last asked: the lanes of their volumes re-analyze them.
    pub fn wake_turned(&self) {
        let turned = self.writer().map(|slot| slot.writer.take_turned()).unwrap_or_default();
        self.runner.wake_files(turned);
    }

    /// Wake the job, which re-analyzes files whose content changed (a turned file's analysis is stale). Every lane.
    pub fn wake_job(&self) {
        self.runner.wake();
    }

    /// Save cropped copies of these files' borders into `folder`, first thing in the background job.
    pub fn save_crops(&self, ids: Vec<FileId>, folder: PathBuf) {
        self.runner.save_crops(ids, folder);
    }

    /// Cut the borders off these files' originals, in place (see `Runner::cut_borders`).
    pub fn cut_borders(&self, ids: Vec<FileId>, confirmed: bool) {
        self.runner.cut_borders(ids, confirmed);
    }

    /// Scan these folders (the job skips folders no longer in the library).
    pub fn scan(&self, roots: Vec<RulePath>) {
        self.runner.scan(roots);
    }

    /// A library folder was removed: its scans are dropped, and one in progress stops (`Runner::stop_scans_under`).
    pub fn stop_scans_under(&self, root: &RulePath) {
        self.runner.stop_scans_under(root);
    }

    /// A folder's removal began: its lanes are parked on its volumes (`Runner::park_removal`).
    pub fn park_removal(&self, path: &RulePath, volumes: &pg_model::removal::Volumes) {
        self.runner.park_removal(path, volumes);
    }

    /// A folder was taken out of the library: its removal is finished by the job now (`Runner::wake_removals`).
    pub fn wake_removals(&self) {
        self.runner.wake_removals();
    }

    /// Pauses or resumes the background job (`Runner::set_paused`, the global pause).
    pub fn set_paused(&self, paused: bool) {
        self.runner.set_paused(paused);
    }

    /// Whether the library's run is paused.
    pub fn is_paused(&self) -> bool {
        self.runner.is_paused()
    }

    /// Pauses or resumes one lane (its number from `LaneProgress::number`); `false` when no lane has that number.
    pub fn set_lane_paused(&self, number: usize, paused: bool) -> bool {
        self.runner.set_lane_paused(number, paused)
    }

    /// The rows of the job now: the crop's progress and its time since work, and each lane's. Reads the runner only, so
    /// the writer is left alone (`status` also tends it).
    pub fn lane_lines(&self) -> LaneLines {
        let (crop, crop_since_work) = self.runner.crop_progress();
        LaneLines { crop, crop_since_work, lanes: self.runner.lane_progress() }
    }

    /// The job's and the writer's state now.
    pub fn status(&self) -> LibraryActivity {
        // Without a writer, nothing is written and nothing has turned: the writer's part is its idle state.
        let (writes, turned, writer_stopped, writer_down) = match self.writer() {
            Some(mut slot) => {
                self.tend(&mut slot);
                (slot.writer.status(), slot.writer.turned(), slot.given_up, !slot.writer.is_running())
            }
            None => (WriteStatus::default(), Default::default(), false, false),
        };
        LibraryActivity {
            progress: self.runner.progress(),
            paused: self.runner.is_paused(),
            writes,
            turned,
            writer_stopped,
            writer_down,
            generation: self.runner.generation(),
            rounds: self.runner.rounds(),
        }
    }
}

/// The library's service: the job and the writer, started on first use, and the folder operations, which work on the catalog
/// whether or not the job has started.
pub struct LibraryService {
    catalog_path: PathBuf,
    catalogs: Arc<Catalogs>,
    thumbs: Arc<ThumbCache>,
    /// Where the job's notices go.
    inbox: Arc<dyn NoticeSink>,
    /// The thumbnail height the grid asks for. Held from the start, so it may be set before the job starts.
    heights: HeightCell,
    /// Which background work is on (the borders, the duplicates), from the preferences. Held from the start, as the
    /// height is.
    features: FeatureCell,
    /// How the job starts: its settings and whether the writer runs. Held until the job starts; the lock is held across the
    /// start, so a `set_passes` cannot slip between reading it and the job's start.
    start: Mutex<StartConfig>,
    /// The started job and writer; set once, by the first [`LibraryService::start`].
    cell: OnceLock<Library>,
}

impl LibraryService {
    pub(crate) fn new(
        catalog_path: PathBuf,
        catalogs: Arc<Catalogs>,
        thumbs: Arc<ThumbCache>,
        inbox: Arc<dyn NoticeSink>,
        features: Features,
        start: StartConfig,
    ) -> LibraryService {
        LibraryService {
            catalog_path,
            catalogs,
            thumbs,
            inbox,
            heights: HeightCell::default(),
            features: FeatureCell::new(features),
            start: Mutex::new(start),
            cell: OnceLock::new(),
        }
    }

    /// Sets the passes each round runs. Before the job starts it is the job's first setting; after, the next round uses it.
    pub fn set_passes(&self, passes: PassMask) {
        let mut start = self.start.lock().unwrap_or_else(PoisonError::into_inner);
        start.runner.passes = passes;
        if let Some(library) = self.cell.get() {
            library.set_passes(passes);
        }
    }

    /// Whether the job has nothing left to do (`Library::quiescent`). A job that has not started has nothing to do.
    pub fn quiescent(&self) -> bool {
        self.cell.get().is_none_or(Library::quiescent)
    }

    /// The grid's thumbnail height: the thumbnails are made at it. Set before or after the job starts; a change ends the
    /// thumbnail pass in progress after its batch.
    pub fn set_thumb_height(&self, height: ThumbHeight) {
        self.heights.set(height);
    }

    /// Which background work is on: a change applies from the job's next batch (or round, for a pass not yet started).
    /// Set before or after the job starts.
    pub fn set_features(&self, features: Features) {
        self.features.set(features);
    }

    /// The library, started on first use. Fails when a thread can't start; the next call tries again.
    pub fn start(&self) -> Result<&Library> {
        if let Some(library) = self.cell.get() {
            return Ok(library);
        }
        let start = self.start.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(library) = self.cell.get() {
            return Ok(library);
        }
        let library = Library::start(
            self.catalog_path.clone(),
            self.thumbs.clone(),
            self.heights.clone(),
            self.features.clone(),
            self.inbox.clone(),
            start.clone(),
        )?;
        Ok(self.cell.get_or_init(|| library))
    }

    /// The started library, if it has started. The folder operations scan through it, and do nothing with it until then.
    pub fn running(&self) -> Option<&Library> {
        self.cell.get()
    }

    /// Ratings and turns were queued for the files; a no-op until the library has started.
    pub fn wake_writer(&self) {
        if let Some(library) = self.running() {
            library.wake_writer();
        }
    }

    /// Wake the job (the user retried a pass's failed files); a no-op until the library has started.
    pub fn wake_job(&self) {
        if let Some(library) = self.running() {
            library.wake_job();
        }
    }

    /// Save cropped copies into `folder`; a no-op until the library has started.
    pub fn save_crops(&self, ids: Vec<FileId>, folder: PathBuf) {
        if let Some(library) = self.running() {
            library.save_crops(ids, folder);
        }
    }

    /// Cut the borders off these files' originals, in place, once the user has confirmed it (`confirmed`: the only
    /// caller that passes `true` is the confirmation dialog's accept). A no-op until the library has started.
    pub fn cut_borders(&self, ids: Vec<FileId>, confirmed: bool) {
        if let Some(library) = self.running() {
            library.cut_borders(ids, confirmed);
        }
    }

    /// The library's folders, each with its id and its path (under the catalog's rules), as the catalog has them.
    pub fn roots(&self) -> Result<Vec<Root>> {
        self.catalogs.with_catalog(|c| c.roots())
    }

    /// Add a folder to the library, at the path given (exact): normalized, and scanned. Returns the normalized folder
    /// and whether it was new; a folder already in the library is rescanned.
    pub fn add_folder(&self, path: &std::path::Path) -> Result<(RulePath, bool)> {
        let (root, added) =
            self.catalogs.with_catalog(|c| Ok(pg_library::folders::add_folder(c, path)))?.map_err(|e| match e {
                AddError::Refused { folder, reason } => Error::FolderRefused { folder, reason },
                AddError::Path(e) => Error::FolderPath(e),
                AddError::Catalog(e) => Error::Catalog(e),
            })?;
        if let Some(library) = self.running() {
            library.scan(vec![root.clone()]);
        }
        Ok((root, added))
    }

    /// Remove a folder from the library for good, by its id (from [`LibraryService::roots`]): its files, their analyses,
    /// ratings and thumbnails, and its folders. `expected` is the folder the caller showed (`RulePath`, as the roots have
    /// it). The folder leaves the library at once (`Catalog::begin_removal`), and a scan of it in progress stops; its files
    /// are then finished by the library's removal job, or here when the library is not running. Returns what the catalog
    /// found: [`Begun::Changed`] when the id names another folder now (nothing was changed).
    pub fn remove_folder(&self, id: RootId, expected: &RulePath) -> Result<Begun> {
        let begun = self.catalogs.with_catalog(|c| c.begin_removal(id, expected))?;
        if let Begun::Started { path, volumes } = &begun {
            match self.running() {
                Some(library) => {
                    // The lanes that read the folder are parked before the removal's files are hidden: from now on no lane
                    // works on them (section 1, D1). The removal job takes its own holds when it hides them.
                    library.park_removal(path, volumes);
                    library.stop_scans_under(path);
                    library.wake_removals();
                }
                None => {
                    self.catalogs.with_catalog(|c| {
                        pg_library::removals::finish(c, &self.thumbs, &self.catalog_path, &mut |_| true).map_err(|e| {
                            pg_catalog::Error::Step { what: pg_model::error::chain(&e), source: Box::new(e) }
                        })
                    })?;
                }
            }
        }
        Ok(begun)
    }

    /// The folders whose removal is not finished, with the rows each keeps for an unfinished write (`Catalog::removal_markers`).
    pub fn pending_removals(&self) -> Result<Vec<(RulePath, u64)>> {
        let markers = self.catalogs.with_catalog(|c| c.removal_markers())?;
        Ok(markers.into_iter().map(|m| (m.path, m.deferred)).collect())
    }

    /// Gives up a removal kept for its unfinished writes (`Catalog::discard_removal`): the changes are dropped, their temp
    /// files are unlinked (one that can't be is logged), and the thumbnails of the files are dropped. `false` when no such
    /// removal is pending.
    pub fn discard_removal(&self, path: &RulePath) -> Result<bool> {
        let Some(discarded) = self.catalogs.with_catalog(|c| c.discard_removal(path))? else { return Ok(false) };
        let rules = pg_platform::native::PATH_RULES;
        let temps: Vec<RulePath> = discarded.temps.iter().filter_map(|bytes| rules.parse_bytes(bytes)).collect();
        for (temp, e) in pg_writeback::discard_temps(rules, &temps) {
            pg_log::warn("removal", &format!("{}: the temp file wasn't deleted: {e:#}", rules.display(&temp)));
        }
        self.thumbs.sweep_orphans(&self.catalog_path).map_err(failed)?;
        Ok(true)
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
        self.catalogs.with_catalog(|c| c.queue_rotation(file_id, turns))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::notice::KeyBuf;

    /// What the library tells its sink, in order, by code.
    #[derive(Default)]
    struct Recording {
        calls: Mutex<Vec<String>>,
    }

    impl Recording {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl NoticeSink for Recording {
        fn post(&self, notice: Notice) {
            self.calls.lock().unwrap().push(format!("post {}", notice.code()));
        }
        fn replace(&self, scope: KeyBuf, notices: Vec<Notice>) {
            self.calls.lock().unwrap().push(format!("replace {} {}", scope.code, notices.len()));
        }
        fn resolve(&self, trigger: Trigger) {
            self.calls.lock().unwrap().push(format!("resolve {trigger:?}"));
        }
        fn poke(&self) {}
    }

    fn library(name: &str) -> (Library, Arc<Recording>, PathBuf) {
        let (dir, layout) = crate::test_support::catalog_home(name);
        let thumbs = Arc::new(ThumbCache::disabled());
        let sink = Arc::new(Recording::default());
        let heights = HeightCell::default();
        heights.set(ThumbHeight::new(100).expect("a height in range"));
        let library = Library::start(
            layout.catalog_path(),
            thumbs,
            heights,
            FeatureCell::new(Features::ALL),
            sink.clone(),
            StartConfig::default(),
        )
        .unwrap();
        (library, sink, dir)
    }

    // A start clears the writer's states an earlier run kept, before anything the new writer could post.
    #[test]
    fn a_start_clears_the_writers_states_first() {
        let (library, sink, dir) = library("library-start");
        assert_eq!(sink.calls()[..1], ["resolve Status"]);
        assert!(library.stop());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // `write-stopped` is posted where the restarts run out, once; a writer stopped at shutdown posts nothing.
    #[test]
    fn write_stopped_is_posted_once_where_the_writer_is_given_up() {
        let (library, sink, dir) = library("library-given-up");
        {
            let mut slot = library.writer().expect("the library has a writer");
            slot.writer.stop(JOB_STOP_WAIT);
            slot.restarts = MAX_RESTARTS;
            library.tend(&mut slot);
            assert!(slot.given_up);
            library.tend(&mut slot);
        }
        assert!(library.status().writer_stopped);
        let stopped = |sink: &Recording| sink.calls().iter().filter(|c| *c == "post write-stopped").count();
        assert_eq!(stopped(&sink), 1);
        assert!(library.stop());
        assert_eq!(stopped(&sink), 1, "a stop posts nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A restart posts nothing: a writer that has ended is started again once its wait is over, and that is not news
    /// (only giving up is, and that has its own test).
    #[test]
    fn a_restart_posts_nothing() {
        let (library, sink, dir) = library("library-restart");
        // Only posts count: the job's own rounds resolve notices on their own schedule, while this test runs.
        let posts = |sink: &Recording| sink.calls().iter().filter(|c| c.starts_with("post")).count();
        let before = posts(&sink);
        {
            let mut slot = library.writer().expect("the library has a writer");
            slot.writer.stop(JOB_STOP_WAIT);
            // Its wait is over: the restart is due now.
            slot.retry_at = Some(Instant::now());
            library.tend(&mut slot);
            assert_eq!(slot.restarts, 1);
            assert!(!slot.given_up);
            assert!(slot.writer.is_running(), "the writer was started again");
        }
        assert_eq!(posts(&sink), before, "a restart posts nothing: {:?}", sink.calls());
        assert!(library.stop());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A writer that has run for `RESTART_RESET` is healthy: its restart count starts over, so its next failure gets the
    /// first wait again. A writer that has not run that long keeps its count.
    #[test]
    fn a_healthy_writer_starts_its_restart_count_over() {
        let (library, sink, dir) = library("library-healthy");
        let before = sink.calls().len();
        {
            let mut slot = library.writer().expect("the library has a writer");
            slot.restarts = MAX_RESTARTS - 1;
            slot.started = Instant::now();
            library.tend(&mut slot);
            assert_eq!(slot.restarts, MAX_RESTARTS - 1, "a writer that just started keeps its count");
            slot.started = Instant::now().checked_sub(RESTART_RESET).expect("the clock is older than a minute");
            library.tend(&mut slot);
            assert_eq!(slot.restarts, 0, "a writer that ran for RESTART_RESET starts its count over");
            assert!(!slot.given_up);
        }
        assert_eq!(sink.calls().len(), before, "a reset posts nothing: {:?}", sink.calls());
        assert!(library.stop());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The writer's `write-stopped` state of one run is cleared by the next run's start, which is the healthy restart:
    /// the start resolves the status triggers before its writer can post, and the writer it starts is running and not
    /// given up. (The notice's resolver is `Trigger::Status`, as `write-stopped` is in the model.)
    #[test]
    fn a_restart_after_a_given_up_writer_resolves_the_status_before_its_writer_posts() {
        let (first, first_sink, dir) = library("library-restart-given-up");
        {
            let mut slot = first.writer().expect("the library has a writer");
            slot.writer.stop(JOB_STOP_WAIT);
            slot.restarts = MAX_RESTARTS;
            first.tend(&mut slot);
            assert!(slot.given_up);
        }
        assert!(first_sink.calls().contains(&"post write-stopped".to_string()));
        assert!(first.stop());

        // The next run, on the same catalog.
        let layout = pg_library::layout::Layout::in_home(&dir);
        let thumbs = Arc::new(ThumbCache::disabled());
        let sink = Arc::new(Recording::default());
        let heights = HeightCell::default();
        heights.set(ThumbHeight::new(100).expect("a height in range"));
        let second = Library::start(
            layout.catalog_path(),
            thumbs,
            heights,
            FeatureCell::new(Features::ALL),
            sink.clone(),
            StartConfig::default(),
        )
        .unwrap();
        assert_eq!(sink.calls()[..1], ["resolve Status"]);
        assert!(!second.status().writer_stopped, "the new run's writer is not given up");
        assert!(!sink.calls().iter().any(|c| c == "post write-stopped"), "{:?}", sink.calls());
        assert!(second.stop());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
