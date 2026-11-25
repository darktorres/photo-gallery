//! The developer's headless driver: the library's job, started without a window, steered and read back. `Headless` runs the app's
//! own service (`pg_app::LibraryService`), so a run's passes are the passes the app's rounds run, with the set chosen
//! (`PassMask`). It holds no pass logic. The `pg-driver` command line and the tests use it; it is not shipped.
//!
//! A run's data lives in one home folder (`Layout::in_home`): the catalog, the preferences, the log. Nothing is read from the
//! environment, so a run never touches the user's library. The writer is off unless a run asks for it (`Setup::writer`), so
//! ratings and turns queued in a catalog are not written into the photos by a run that did not ask.

#![forbid(unsafe_code)]

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pg_app::{App, Config, StartConfig};
use pg_catalog::{Catalog, CatalogCounts};
use pg_library::layout::Layout;
use pg_library::mask::PassMask;
use pg_library::runner::RunnerConfig;
use pg_model::features::Features;
use pg_model::ids::RootId;
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
use pg_model::write::WriteError;
use pg_platform::native::PATH_RULES;

/// How often a wait looks at the job again.
const POLL: Duration = Duration::from_millis(20);

/// How long the job must stay quiet for a wait to end: a round that is about to start does so within it.
const QUIET_HOLD: Duration = Duration::from_millis(150);

/// What a run is set up with: its home folder, the writer, the counts pinned for it, the passes it runs, and the features on.
#[derive(Clone, Debug)]
pub struct Setup {
    /// The home folder: the catalog, the preferences and the log live under it.
    pub home: PathBuf,
    /// Whether a missing home is made. Off by default: a home that is not there is most likely a drive that is not connected,
    /// so the run is refused rather than starting an empty library.
    pub create: bool,
    /// Whether the writer runs. Off by default: a run on a real catalog writes nothing into the files unless it asks.
    pub writer: bool,
    /// The reads in flight, pinned for the run (a sweep). `None` keeps the default; a pinned run writes no learned profile.
    pub pin_readers: Option<NonZeroUsize>,
    /// The decode threads, pinned for the run. `None` keeps the default.
    pub pin_workers: Option<NonZeroUsize>,
    /// The passes the job's rounds run, unless a run names its own (`Headless::run`).
    pub passes: PassMask,
    /// The background work on: the borders and the duplicates.
    pub features: Features,
}

impl Setup {
    /// A setup of the home folder `home`: the writer off, no pins, every pass, and every feature on.
    pub fn new(home: impl Into<PathBuf>) -> Setup {
        Setup {
            home: home.into(),
            create: false,
            writer: false,
            pin_readers: None,
            pin_workers: None,
            passes: PassMask::ALL,
            features: Features::ALL,
        }
    }
}

/// The catalog's counts and its folders, as a run reads them back.
#[derive(Clone, Debug)]
pub struct Status {
    /// The counts of files, analyses, pairs, pending work and queued writes.
    pub counts: CatalogCounts,
    /// The library's folders, as the catalog displays them.
    pub roots: Vec<String>,
    /// The folders whose removal is not finished.
    pub removals: usize,
    /// The folders the scan could not walk, which the folder ledger keeps (up to 1000 are counted).
    pub folder_failures: usize,
}

/// A failure of a driver operation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The app's service failed.
    #[error(transparent)]
    App(#[from] pg_app::Error),
    /// The catalog could not be read.
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    /// The job did not become quiet in time: a folder with no drive, a removal that cannot finish, or a long run.
    #[error("the library was still busy after {seconds} s")]
    Timeout {
        /// How long the wait was, in seconds.
        seconds: u64,
    },
    /// The library's own operation (a removal's finish) failed.
    #[error(transparent)]
    Library(#[from] pg_library::Error),
    /// A folder cannot be added while its removal is pending, because the running job owns removals.
    #[error("{folder} is being removed from the library: wait for the removal to finish")]
    RemovalPending {
        /// The folder whose removal is pending.
        folder: String,
    },
    /// A folder's removal is kept for changes still waiting to be written, so it cannot be added over (`discard_removal`).
    #[error("the removal of {folder} is kept for unfinished writes: discard it, or finish the writes")]
    RemovalKept {
        /// The folder whose removal is kept.
        folder: String,
    },
    /// The home's catalog folder is missing, and the run did not ask for it to be made (`Setup::create`).
    #[error("{path} is missing: a drive that is not connected is the usual reason; pass --create to make it")]
    HomeMissing {
        /// The folder that is missing.
        path: String,
    },
    /// A write failed, and the writer's latest error is the cause. `left` counts the changes still queued (zero when the failed
    /// write was the last). `error` is the writer's own words.
    #[error("writing into the files has failed ({left} still to write): {error}")]
    WriteFailed {
        /// The changes still queued.
        left: u64,
        /// The writer's error, in its words.
        error: String,
    },
    /// The writer stopped, and changes are still queued.
    #[error("writing into the files has stopped with {left} changes still to write")]
    WriterStopped {
        /// The changes still queued.
        left: u64,
    },
    /// A write was asked for, but the run was set up without a writer (`Setup::writer`).
    #[error("this run has no writer: set Setup::writer to write the queued changes")]
    WriterOff,
    /// A folder asked for is not in the library.
    #[error("{path} is not a library folder")]
    NotAFolder {
        /// The folder, as it was asked for.
        path: String,
    },
}

/// The result of a driver operation.
pub type Result<T> = std::result::Result<T, Error>;

/// A started run: the app's service over a home folder, with its job started on the first call that needs it.
pub struct Headless {
    app: App,
    layout: Layout,
    writer: bool,
    default_passes: PassMask,
}

impl Headless {
    /// Opens the app over `setup`'s home folder. The job is not started yet: `start` does that. Creates the home's catalog
    /// folder when it is missing.
    pub fn open(setup: &Setup) -> Result<Headless> {
        let layout = Layout::in_home(&setup.home);
        if !layout.catalog_dir().is_dir() {
            if !setup.create {
                return Err(Error::HomeMissing { path: layout.catalog_dir().display().to_string() });
            }
            std::fs::create_dir_all(layout.catalog_dir())
                .map_err(|e| pg_app::Error::Failed(format!("{}: {e}", setup.home.display())))?;
        }
        let config = Config {
            start: StartConfig {
                runner: RunnerConfig {
                    pin_readers: setup.pin_readers,
                    pin_workers: setup.pin_workers,
                    passes: setup.passes,
                },
                writer: setup.writer,
            },
            ..Config::from_layout(layout.clone())
        };
        let app = App::open(config)?;
        app.library().set_features(setup.features);
        Ok(Headless { app, layout, writer: setup.writer, default_passes: setup.passes })
    }

    /// Starts the job (and the writer, when the run has one). Starting twice is harmless.
    pub fn start(&self) -> Result<()> {
        self.app.library().start()?;
        Ok(())
    }

    /// Sets the thumbnail height the grid would ask for: the thumbnail pass makes thumbnails at it.
    pub fn set_thumb_height(&self, height: ThumbHeight) {
        self.app.library().set_thumb_height(height);
    }

    /// Adds a folder to the library, as the app does, and scans it with the next `rescan` or `run`. A folder whose removal was
    /// left unfinished (an app stopped half way) is added over it: that removal is finished first, as the command line always
    /// did, then the folder is added. While the job is running, the job owns removals, so the add is refused instead
    /// (`Error::RemovalPending`), as the app refuses it. A removal kept for unfinished writes is refused the same way
    /// (`Error::RemovalKept`): nothing is added over it.
    pub fn add_folder(&self, path: &Path) -> Result<(RulePath, bool)> {
        match self.app.library().add_folder(path) {
            Err(pg_app::Error::Catalog(pg_catalog::Error::RemovalPending { path: folder }))
                if self.app.library().running().is_none() =>
            {
                let catalog = self.catalog()?;
                pg_library::removals::finish(&catalog, self.app.thumbs(), &self.layout.catalog_path(), &mut |_| true)?;
                if !catalog.removal_markers()?.is_empty() {
                    return Err(Error::RemovalKept { folder });
                }
                Ok(self.app.library().add_folder(path)?)
            }
            Err(pg_app::Error::Catalog(pg_catalog::Error::RemovalPending { path: folder })) => {
                Err(Error::RemovalPending { folder })
            }
            other => Ok(other?),
        }
    }

    /// Removes a library folder by its path, and finishes the removal (`run`'s quiet wait).
    /// `Ok(false)` when no such folder is in the library.
    pub fn remove_folder(&self, path: &str, timeout: Duration) -> Result<bool> {
        let Some(id) = self.folder_id(path)? else { return Ok(false) };
        let expected = self.folder_path(id)?;
        self.app.library().remove_folder(id, &expected)?;
        self.wait_quiet(timeout)?;
        Ok(true)
    }

    /// Gives up a removal kept for its unfinished writes, as the app's dialog does. `Ok(false)` when none is pending.
    pub fn discard_removal(&self, path: &str) -> Result<bool> {
        let rules = self.catalog()?.rules();
        let folder = rules.parse(path).ok_or_else(|| Error::NotAFolder { path: path.to_string() })?;
        Ok(self.app.library().discard_removal(&folder)?)
    }

    /// Runs the job's passes over every library folder until it is quiet, with only `passes` in each round, then restores the
    /// setup's passes. Each folder is scanned first. A pass left out owes its work, which a later run does.
    pub fn run(&self, passes: PassMask, timeout: Duration) -> Result<Duration> {
        self.start()?;
        self.app.library().set_passes(passes);
        self.app.library().rescan()?;
        let result = self.wait_quiet(timeout);
        self.app.library().set_passes(self.default_passes);
        result
    }

    /// Writes the queued changes into the files, until none is left, and returns how many were queued at the start. The run must
    /// have a writer (`Setup::writer`).
    pub fn write_pending(&self, timeout: Duration) -> Result<u64> {
        if !self.writer {
            return Err(Error::WriterOff);
        }
        self.start()?;
        let queued = self.catalog()?.left_to_write()?;
        let deadline = Instant::now() + timeout;
        loop {
            self.app.library().wake_writer();
            let left = self.catalog()?.left_to_write()?;
            // A write that fails leaves the queue too, so the writer's error is read even when nothing is left: the run
            // fails with it. A writer that stopped with changes still queued ends the run now, not at the timeout.
            if let Some(library) = self.app.library().running() {
                let activity = library.status();
                let stopped = activity.writer_stopped && left > 0;
                if let Some(error) = writer_failure(left, activity.writes.error.as_ref(), stopped) {
                    return Err(error);
                }
            }
            if left == 0 {
                return Ok(queued);
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout { seconds: timeout.as_secs() });
            }
            std::thread::sleep(POLL * 10);
        }
    }

    /// The catalog's counts and folders, read back now.
    pub fn status(&self) -> Result<Status> {
        let catalog = self.catalog()?;
        let rules = catalog.rules();
        let roots = catalog.roots()?.iter().map(|root| rules.display(&root.path)).collect();
        Ok(Status {
            counts: catalog.status()?,
            roots,
            removals: catalog.removal_markers()?.len(),
            folder_failures: catalog.failed_folders(1000)?.len(),
        })
    }

    /// Whether the job is quiet now (`LibraryService::quiescent`): no round, no queued or waiting work, no write left and no
    /// unfinished removal. A job that has not started is quiet.
    pub fn quiescent(&self) -> bool {
        self.app.library().quiescent()
    }

    /// Waits until the job is quiet: no round, no queued or waiting work and no unfinished removal, held for `QUIET_HOLD`.
    /// `Err(Timeout)` when that does not happen within `timeout`. Returns how long the wait took.
    pub fn wait_quiet(&self, timeout: Duration) -> Result<Duration> {
        self.start()?;
        let started = Instant::now();
        let deadline = started + timeout;
        let mut quiet_since: Option<Instant> = None;
        loop {
            let now = Instant::now();
            if self.app.library().quiescent() {
                let since = *quiet_since.get_or_insert(now);
                if now.duration_since(since) >= QUIET_HOLD {
                    return Ok(started.elapsed());
                }
            } else {
                quiet_since = None;
            }
            if now >= deadline {
                return Err(Error::Timeout { seconds: timeout.as_secs() });
            }
            std::thread::sleep(POLL);
        }
    }

    /// The catalog, opened for reading: a fresh connection for each call.
    fn catalog(&self) -> Result<Catalog> {
        Ok(Catalog::open(&self.layout.catalog_path(), PATH_RULES)?)
    }

    /// The id of the library folder at `path`, if it is one.
    fn folder_id(&self, path: &str) -> Result<Option<RootId>> {
        let catalog = self.catalog()?;
        let folder = catalog.rules().parse(path).ok_or_else(|| Error::NotAFolder { path: path.to_string() })?;
        let rules = catalog.rules();
        Ok(catalog.roots()?.into_iter().find(|root| rules.same(&root.path, &folder)).map(|root| root.id))
    }

    /// The path of the library folder `id`, as the catalog has it.
    fn folder_path(&self, id: RootId) -> Result<RulePath> {
        let catalog = self.catalog()?;
        catalog
            .roots()?
            .into_iter()
            .find(|root| root.id == id)
            .map(|root| root.path)
            .ok_or_else(|| Error::NotAFolder { path: format!("root {}", id.get()) })
    }
}

/// The error that ends a write run with `left` changes still queued, given the writer's latest error and whether it stopped:
/// the writer's own words for an error, else `WriterStopped`. None while the writer is working.
fn writer_failure(left: u64, error: Option<&WriteError>, stopped: bool) -> Option<Error> {
    match error {
        Some(error) => Some(Error::WriteFailed { left, error: pg_writeback::write_error_words(error) }),
        None => stopped.then_some(Error::WriterStopped { left }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::job::Cause;

    #[test]
    fn a_writer_error_ends_the_run_with_its_words_and_the_queue() {
        let error = WriteError::Failed(Cause::Error("the disk is full".to_owned()));
        assert!(matches!(
            writer_failure(3, Some(&error), false),
            Some(Error::WriteFailed { left: 3, error }) if error.contains("the disk is full")
        ));
    }

    #[test]
    fn an_error_is_reported_before_a_stop() {
        let error = WriteError::Failed(Cause::Error("the disk is full".to_owned()));
        assert!(matches!(writer_failure(1, Some(&error), true), Some(Error::WriteFailed { left: 1, .. })));
    }

    #[test]
    fn a_stopped_writer_with_no_error_ends_the_run_with_the_queue() {
        assert!(matches!(writer_failure(2, None, true), Some(Error::WriterStopped { left: 2 })));
    }

    #[test]
    fn a_working_writer_does_not_end_the_run() {
        assert!(writer_failure(2, None, false).is_none());
    }
}
