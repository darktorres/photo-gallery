//! Writing the user's changes into the files themselves (PLAN §2): ratings as `XMP:Rating` (ExifTool), turns
//! losslessly (`pg_edit::rotate`). The catalog changed already when the user acted; this catches the files up,
//! from the `file_writes` queue, so a write survives quitting and a file that is offline waits for its drive.
//!
//! Files keep their NTFS identity, which is the catalog's identity for them (PLAN §4). Writing a temp file and
//! renaming it over the original (what `exiftool -overwrite_original` and jpegtran-based tools do) would make the
//! file new to the catalog, and lose its rating. So every write has two steps:
//! 1. the new content is written, complete, to a temp file beside the original (one per kind of write), from the file
//!    as it was read; the temp is synced to disk;
//! 2. the original is opened for the copy, denying other writers, and checked to be as it was read (another program's
//!    save in between fails the attempt, and the next one reads the file again). Only then does the temp's path go in
//!    the queue row: from there the original may be part-written. The temp is copied into it in place, the queue
//!    records the write, and the temp is deleted.
//!
//! A crash or a failure during step 2 leaves the row with its temp set: the next attempt (and the next start)
//! finishes the copy from it. A recorded temp is never deleted before the write is recorded, nor given up: while the
//! original may be part-written, it is the only whole copy of the new content.

use pg_model::ids::FileId;
use pg_model::turns::Turns;
use std::fs::File;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use pg_model::analysis::FileHash;
use pg_catalog::{Catalog, WriteTarget};
use pg_catalog::queue::{self, Job, MAX_ATTEMPTS, Rewrite, WriteEffect};
use pg_edit::rotate::{Outcome, rotate};
use pg_exiftool::ExifTool;
use pg_jobs::events::{Bus, Watch};
use pg_jobs::failure::Cause;
use pg_jobs::panics;
use pg_jobs::service::{Service, Waker, Woke};
use pg_metadata::{self, Meta, MetaScratch, patch_orientation};
use pg_model::format::Format;
use pg_model::media::Kind;
use pg_model::rules::Path as RulePath;

use pg_model::write::{Write, WriteKind};
use pg_platform::names::disk_path;
use pg_platform::names::set_path;
use pg_platform::native::NativePath;
use pg_thumbcache::ThumbCache;

use pg_jobs::events::Change;

#[cfg(any(test, feature = "fault"))]
pub mod fault;
mod file;

pub use file::{discard_temps, temp_of};
use file::{stat, sync, unchanged, write_into};

/// A rating is written once it has been left alone this long, so quick changes (triage) coalesce.
const SETTLE_SECS: i64 = 2;
/// How often the queue is looked at without being woken (retries, offline drives).
const IDLE_CHECK: Duration = Duration::from_secs(30);

/// What the app shows about the queue.
#[derive(Clone, Default)]
pub struct WriteStatus {
    pub queued: usize,
    /// The latest failure, until the next success (or until the write it names is dropped), which the app words.
    pub error: Option<WriteError>,
    /// The file `error` is about, when it is a job's.
    pub error_file: Option<FileId>,
    /// Queued writes dropped so far because their file left the library (marked missing, or no longer listed).
    pub dropped: usize,
    /// The kind of the latest of them.
    pub last_dropped: Option<WriteKind>,
    /// The latest thumbnail that couldn't follow a rewritten file (the cache busy or closing), which the app words.
    pub thumbnail_error: Option<pg_model::notice::ThumbnailLag>,
}

/// Why a write failed, for the app to word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// A job's failure (an error, or a panic on it), or the queue's.
    Failed(Cause),
    /// The copy into the original failed up to the cap after it may have started: the original (`file`, as the user
    /// sees its path) may be incomplete, and the whole new content is in `temp`. Both are kept; the copy is tried
    /// again after the next change to the file, and at the next start.
    CopyIncomplete { file: String, temp: String, cause: Cause },
}

/// The English words, for the command line and the queue's `error` column; the app words it through `pg_client::text`.
impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Failed(cause) => write!(f, "{cause}"),
            WriteError::CopyIncomplete { file, temp, cause } => write!(
                f,
                "{file} may be incomplete: copying its change into it failed ({cause}); the whole new content is in \
                 {temp}"
            ),
        }
    }
}

/// The original a write changes, in the three forms its steps take: its disk path (std's calls), its path for the
/// system's open that denies other writers, and its path as the messages show it.
#[derive(Clone, Copy)]
struct Original<'a> {
    path: &'a Path,
    native: &'a NativePath,
    shown: &'a str,
}

/// The context of a failure once the copy into the original may have started (its temp recorded): what the drain
/// looks for to tell that the original may be part-written.
#[derive(Debug)]
struct CopyStarted {
    /// The temp, as the user sees its path.
    temp: String,
}

impl std::fmt::Display for CopyStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "copying {} into the original", self.temp)
    }
}

/// The copy of a fresh job into its original, once its temp is recorded: the original, open for the copy; the new
/// content; the file's (size, mtime) as it was read; and the BLAKE3 of the new bytes when only metadata changed.
struct CopyInto<'a> {
    file: File,
    data: &'a [u8],
    before: (i64, i64),
    new_hash: Option<FileHash>,
}

struct Inner {
    catalog_path: PathBuf,
    thumbs: &'static ThumbCache,
    status: Watch<WriteStatus>,
    /// Files turned so far: their pixels changed, so the app reloads them and the job re-analyzes them.
    turned: Watch<u64>,
    /// Told what changed (`Change::Writes`, `Change::Turned`).
    changes: Arc<Bus<Change>>,
    /// Where the tests ask it to fail.
    #[cfg(any(test, feature = "fault"))]
    fault: Option<fault::Fault>,
    /// Whether a one-time fault has happened.
    #[cfg(any(test, feature = "fault"))]
    fault_fired: std::sync::atomic::AtomicBool,
}

impl Inner {
    fn update_status(&self, change: impl FnOnce(&mut WriteStatus)) {
        self.status.update(change);
        self.changes.publish(Change::Writes);
    }

    fn turned_once(&self) {
        self.turned.update(|t| *t += 1);
        self.changes.publish(Change::Turned);
    }
}

/// The writer thread: wakes for the queue, and rechecks it every `IDLE_CHECK` (retries, offline drives).
pub struct Writer {
    inner: Arc<Inner>,
    service: Service,
}

impl Writer {
    /// Start the writer thread. What it changes is published on `changes` (`Change`). An error if the thread can't be
    /// spawned.
    pub fn start(catalog_path: PathBuf, thumbs: &'static ThumbCache, changes: Arc<Bus<Change>>) -> Result<Writer> {
        Writer::spawn(Inner {
            catalog_path,
            thumbs,
            status: Watch::new(WriteStatus::default()),
            turned: Watch::new(0),
            changes,
            #[cfg(any(test, feature = "fault"))]
            fault: None,
            #[cfg(any(test, feature = "fault"))]
            fault_fired: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// [`Writer::start`], with a writer that fails where `fault` says: for the tests of what a panic, or another
    /// program's save, does to it.
    #[cfg(any(test, feature = "fault"))]
    pub fn start_with_fault(
        catalog_path: PathBuf,
        thumbs: &'static ThumbCache,
        changes: Arc<Bus<Change>>,
        fault: fault::Fault,
    ) -> Result<Writer> {
        Writer::spawn(Inner {
            catalog_path,
            thumbs,
            status: Watch::new(WriteStatus::default()),
            turned: Watch::new(0),
            changes,
            fault: Some(fault),
            fault_fired: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn spawn(inner: Inner) -> Result<Writer> {
        let inner = Arc::new(inner);
        let job = inner.clone();
        let service = Service::start("file writer", move |waker| job.run(waker)).context("start the file writer")?;
        Ok(Writer { inner, service })
    }

    /// Something was queued.
    pub fn wake(&self) {
        self.service.wake();
    }

    /// Whether the writer thread is still running. A panic in a job is that job's failure and the writer goes on; a
    /// panic outside one (where it reads the queue) ends the thread, and whoever waits for the queue to empty stops
    /// waiting when this is `false`.
    pub fn is_running(&self) -> bool {
        self.service.is_running()
    }

    pub fn status(&self) -> WriteStatus {
        self.inner.status.get()
    }

    /// Counts files turned so far, for the app to notice.
    pub fn turned(&self) -> u64 {
        self.inner.turned.get()
    }

    /// Stop the thread after the write in progress (a queued write waits for the next start), and wait for it.
    /// Idempotent; dropping the writer does it too.
    pub fn stop(&self) {
        self.service.stop();
    }
}

impl Inner {
    fn run(&self, waker: &Waker<'_>) {
        let mut exiftool: Option<ExifTool> = None;
        // One connection for every drain; after a drain that failed, a new one.
        let mut catalog: Option<Catalog> = None;
        // Whether the stuck copies were given their attempts back (`Catalog::retry_write_copies`), once per start.
        let mut retried = false;
        while waker.wait(Some(IDLE_CHECK)) != Woke::Quit {
            if let Err(e) = self.open_and_drain(waker, &mut catalog, &mut exiftool, &mut retried) {
                self.update_status(|s| {
                    s.error = Some(WriteError::Failed(Cause::Error(format!("{e:#}"))));
                    s.error_file = None;
                });
            }
        }
    }

    /// One drain on the open catalog, or on one opened now. The first open of a start tries the stuck copies again.
    fn open_and_drain(
        &self,
        waker: &Waker<'_>,
        catalog: &mut Option<Catalog>,
        exiftool: &mut Option<ExifTool>,
        retried: &mut bool,
    ) -> Result<()> {
        let c = match catalog.take() {
            Some(c) => c,
            None => Catalog::open(&self.catalog_path, pg_platform::native::PATH_RULES)?,
        };
        if !*retried {
            c.retry_write_copies()?;
            *retried = true;
        }
        self.drain(waker, &c, exiftool)?;
        *catalog = Some(c);
        Ok(())
    }

    /// Work through the queue until nothing is ready; then wait for what is still settling.
    ///
    /// A panic in a job is that job's failure (`panics::guard`), counted as any other. A panic outside one is not
    /// caught: nothing would record it, so a writer kept running would meet it again at once. It ends the thread, which
    /// [`Writer::is_running`] reports.
    fn drain(&self, waker: &Waker<'_>, catalog: &Catalog, exiftool: &mut Option<ExifTool>) -> Result<()> {
        loop {
            if waker.quitting() {
                return Ok(());
            }
            #[cfg(any(test, feature = "fault"))]
            fault::in_drain(self.fault);
            let queued = catalog.writes_remaining()?;
            self.update_status(|s| s.queued = queued);
            // Finishing interrupted copies first; then the oldest settled change (`Catalog::next_write`).
            let Some(job) = catalog.next_write(SETTLE_SECS)? else {
                if catalog.writes_attemptable()? {
                    // Come back when the newest change has settled (or at once for a new one); a stop ends the wait.
                    if waker.wait(Some(Duration::from_secs(u64::try_from(SETTLE_SECS)?))) == Woke::Quit {
                        return Ok(());
                    }
                    continue;
                }
                return Ok(());
            };
            // A file the catalog marks missing, or no longer lists, can't be written. A fresh write is dropped, and the app
            // is told (`WriteStatus::dropped`); a failure the status still shows for it goes with it. A copy that may have
            // started is kept with its temp, the only whole copy of the new content while the original may be part-written:
            // it waits, offline (not an attempt), until the file is listed again.
            if catalog.file_gone(job.file_id)? {
                if let Some((temp, _)) = &job.temp {
                    let message = format!(
                        "{} is no longer in the library; the whole new content is kept in {} until it is back",
                        shown_file(catalog, &job),
                        catalog.rules().display(temp)
                    );
                    catalog.write_failed(&job, &message, true)?;
                    self.update_status(|s| {
                        s.error = Some(WriteError::Failed(Cause::Error(message)));
                        s.error_file = Some(job.file_id);
                    });
                    // Left for the next idle check, as an offline file is.
                    return Ok(());
                }
                catalog.drop_gone_write(&job)?;
                self.update_status(|s| {
                    s.dropped = s.dropped.saturating_add(1);
                    s.last_dropped = Some(job.kind());
                    if s.error_file == Some(job.file_id) {
                        s.error = None;
                        s.error_file = None;
                    }
                });
                continue;
            }
            match panics::guard(AssertUnwindSafe(|| self.process(catalog, &job, exiftool))) {
                Ok(()) => self.update_status(|s| {
                    s.error = None;
                    s.error_file = None;
                }),
                Err(e) => {
                    let panicked = panics::is_panic(&e);
                    if panicked {
                        // ExifTool may have been left in the middle of a command: the next job starts a new one.
                        *exiftool = None;
                    }
                    // Offline: the drive (or the file's folder with it) is not there, and the job waits for it. A
                    // missing or refused file on a drive that is there (a read-only original, a folder the user
                    // can't write) is a failure, counted like any other, and so is a sharing violation (another
                    // program has the file open for writing) and a panic.
                    let offline = !panicked
                        && e.downcast_ref::<std::io::Error>().is_some_and(|io| {
                            matches!(io.kind(), std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound)
                                && !self.folder_present(catalog, job.file_id)
                        });
                    let cause = Cause::of(&e);
                    let at_cap = !offline && job.attempts.saturating_add(1) >= MAX_ATTEMPTS;
                    // At the cap, a copy that may have started says so: the original may be incomplete, and its temp
                    // (kept, with its record) holds the whole new content.
                    let error = match copy_temp(catalog, &job, &e) {
                        Some(temp) if at_cap => {
                            WriteError::CopyIncomplete { file: shown_file(catalog, &job), temp, cause }
                        }
                        _ => WriteError::Failed(cause),
                    };
                    catalog.write_failed(&job, &error.to_string(), offline)?;
                    self.update_status(|s| {
                        s.error = Some(error);
                        s.error_file = Some(job.file_id);
                    });
                    if offline {
                        // Leave it for the next idle check rather than spinning on it.
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Whether the folder the file is in exists, so its drive is there. A catalog error is not an answer: the folder is
    /// taken as there, so the failure counts as an attempt and the job is retried, instead of waiting as offline.
    fn folder_present(&self, catalog: &Catalog, file_id: FileId) -> bool {
        match catalog.file_folder(file_id) {
            Ok(folder) => folder.is_some_and(|p| disk_path(catalog.rules(), &p).is_dir()),
            Err(_) => true,
        }
    }

    fn process(&self, catalog: &Catalog, job: &Job, exiftool: &mut Option<ExifTool>) -> Result<()> {
        #[cfg(any(test, feature = "fault"))]
        fault::in_process(self.fault, job.file_id);
        let WriteTarget { path: path_rule, format: known, kind } = catalog.write_target(job.file_id)?;

        // Videos aren't rewritten (a copy of GBs per change): a turn stays in the catalog. A rating isn't queued for
        // one (`queue::rating_in_file_possible` asks for an image first); one queued before that rule is done here.
        if kind == Kind::Video && job.temp.is_none() {
            match job.write {
                Write::Turns(turns) => self.keep_turn_in_catalog(catalog, job, turns)?,
                Write::Rating(_) => catalog.write_done(job, WriteEffect::Nothing)?,
            }
            return Ok(());
        }
        // The file's disk path (which std and ExifTool reach exactly, a folder whose name ends in a dot or a space
        // included), its path for the system's open, and its path as the messages show it.
        let rules = catalog.rules();
        let path = disk_path(rules, &path_rule);
        let mut native = NativePath::default();
        set_path(&mut native, &rules.join(&path_rule));
        let shown = rules.display(&path_rule);

        // A copy that may have started finishes.
        if let Some((temp, carried)) = &job.temp {
            let original = Original { path: &path, native: &native, shown: &shown };
            return self
                .finish_copy(catalog, job, (temp, *carried), &original)
                .with_context(|| CopyStarted { temp: rules.display(temp) });
        }

        // A rating for a format that can't hold one (known from the analysis) is done: the catalog keeps it.
        if job.kind() == WriteKind::Rating && known.is_some_and(|f| !pg_model::format::can_hold_rating(f)) {
            return Ok(catalog.write_done(job, WriteEffect::Nothing)?);
        }
        // Taken before the read: a save by another program after it shows when the original is opened for the copy.
        let before = stat(&path)?;
        let bytes = std::fs::read(&path).with_context(|| format!("reading {shown}"))?;
        let format = match known.or_else(|| Format::sniff(&bytes)) {
            // Not a format the app reads (an analysis that failed, a file that isn't what its name says), or one that
            // can't hold a rating: a rating stays in the catalog, and the write is done. A turn can't be made.
            Some(f) if job.kind() == WriteKind::Rating && !pg_model::format::can_hold_rating(f) => {
                return Ok(catalog.write_done(job, WriteEffect::Nothing)?);
            }
            Some(f) => f,
            None if job.kind() == WriteKind::Rating => return Ok(catalog.write_done(job, WriteEffect::Nothing)?),
            None => bail!("{shown}: unknown format"),
        };
        let temp_rule = temp_of(&path_rule, job.kind());
        let temp = disk_path(rules, &temp_rule);
        let shown_temp = rules.display(&temp_rule);
        // Not recorded (the job has no temp), so not a copy that may have started: what is there is a leftover (a
        // crash before it was recorded, or a removal that failed after a write), and goes.
        let _ = std::fs::remove_file(&temp);

        // Step 1: the new content, complete, in the temp.
        let (data, pixels_changed) = match job.write {
            Write::Rating(rating) => {
                let tool = ExifTool::running(exiftool, &pg_exiftool::program())?;
                tool.write_rating(&path, &temp, rating.get())?;
                let written = std::fs::read(&temp).with_context(|| format!("reading {shown_temp}"))?;
                if Meta::read(&written, format, &mut MetaScratch::default()).rating != Some(rating) {
                    let _ = std::fs::remove_file(&temp);
                    bail!("{shown}: the rating didn't read back after writing it");
                }
                (written, false)
            }
            Write::Turns(turns) => {
                let write_temp = |new: Vec<u8>| -> Result<Vec<u8>> {
                    std::fs::write(&temp, &new).with_context(|| format!("writing {shown_temp}"))?;
                    Ok(new)
                };
                let new = match rotate(&bytes, format, turns)? {
                    Outcome::Rewritten(new) => write_temp(new)?,
                    Outcome::Orientation(o) => match patch_orientation(&bytes, format, o) {
                        Some(new) => write_temp(new)?,
                        None => {
                            // No tag to change: ExifTool adds one. A format it can't add EXIF to keeps the turn in
                            // the catalog.
                            let tool = ExifTool::running(exiftool, &pg_exiftool::program())?;
                            let written = match tool.write_orientation(&path, &temp, o.get()) {
                                Ok(()) => Some(std::fs::read(&temp)?)
                                    .filter(|written| pg_metadata::orientation(written, format) == o),
                                Err(_) => None,
                            };
                            match written {
                                Some(written) => written,
                                None => {
                                    let _ = std::fs::remove_file(&temp);
                                    return self.keep_turn_in_catalog(catalog, job, turns);
                                }
                            }
                        }
                    },
                    Outcome::Catalog => return self.keep_turn_in_catalog(catalog, job, turns),
                };
                (new, true)
            }
        };
        let new_hash = if pixels_changed { None } else { Some(FileHash::of(&data)) };

        // Step 2: the original, open for the copy and as it was read, with the temp durable. Until it is recorded, a
        // failure leaves the original untouched: the temp goes, and the next attempt starts from the file.
        #[cfg(any(test, feature = "fault"))]
        fault::before_copy(self.fault, &self.fault_fired, job.file_id, &path)?;
        let file = match Self::open_for_copy(&native, &temp, &shown, &shown_temp, before) {
            Ok(file) => file,
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                return Err(e);
            }
        };
        #[cfg(any(test, feature = "fault"))]
        fault::before_record(self.fault, &self.fault_fired, job, &self.catalog_path)?;
        // Nothing is recorded (the error, or the job's row gone since it was read): the original is not touched, and the
        // temp goes. A row gone means the write was cancelled; the job is over.
        let recorded = match catalog.record_write_temp(job, &temp_rule, job.write) {
            Ok(recorded) => recorded,
            Err(e) => {
                drop(file);
                let _ = std::fs::remove_file(&temp);
                return Err(e.into());
            }
        };
        if !recorded {
            drop(file);
            let _ = std::fs::remove_file(&temp);
            return Ok(());
        }
        // From here until `done` records the write, readers know the file may be part-written
        // (pg_model::state::FileState::writing), and a failure leaves the temp for the next attempt to finish from.
        // Every failure from here on is one where the copy may have started, so it is given the temp (`CopyStarted`).
        let copy = CopyInto { file, data: &data, before, new_hash };
        let after = self
            .copy_and_done(catalog, job, &path, copy)
            .with_context(|| CopyStarted { temp: rules.display(&temp_rule) })?;
        // The write is recorded: the temp is a leftover now, and one that can't be removed is cleared by the next
        // write of the file.
        let _ = std::fs::remove_file(&temp);
        if job.kind() == WriteKind::Rating {
            // The pixels didn't change: the thumbnails still show the file. If they can't follow it (the cache
            // busy or closing), they are only made again: the write is done all the same.
            if let Err(e) = self.thumbs.retag(job.file_id, before.1, after.1) {
                let lag = pg_model::notice::ThumbnailLag { file: job.file_id, error: format!("{e:#}") };
                self.update_status(|s| s.thumbnail_error = Some(lag));
            }
        }
        if pixels_changed {
            self.pixels_changed();
        }
        Ok(())
    }

    /// The original opened for the copy, denying other writers (`pg_platform::file::open_for_copy`), once it is
    /// checked to be as it was read (`before`) and the temp is synced to disk: the new content must be durable before
    /// the original is overwritten from it.
    fn open_for_copy(
        native: &NativePath,
        temp: &Path,
        shown: &str,
        shown_temp: &str,
        before: (i64, i64),
    ) -> Result<File> {
        let file = pg_platform::file::open_for_copy(native).with_context(|| format!("opening {shown} for writing"))?;
        unchanged(shown, before, pg_platform::file::stat_of(&file)?)?;
        sync(temp).with_context(|| format!("writing {shown_temp}"))?;
        Ok(file)
    }

    /// The copy into the original, then the write recorded as done: the steps after a fresh job's temp is recorded.
    /// Every failure here is one where the copy may have started; the caller gives it `CopyStarted`. Returns the file's
    /// (size, mtime) after the copy.
    fn copy_and_done(&self, catalog: &Catalog, job: &Job, path: &Path, copy: CopyInto<'_>) -> Result<(i64, i64)> {
        #[cfg(any(test, feature = "fault"))]
        fault::in_copy(self.fault, job.file_id)?;
        write_into(copy.file, copy.data)?;
        let after = stat(path)?;
        let rewrite = Rewrite { before: Some(copy.before), after, metadata_only: copy.new_hash };
        catalog.write_done(job, Self::rewrote(job, rewrite))?;
        Ok(after)
    }

    /// Finish a copy that may have started (its temp recorded): copy the temp in, and record the write the temp
    /// carries, so a change made since stays queued. What the file held before is unknown now, so its
    /// analysis is redone rather than carried over. The original isn't compared: it may already be partly the temp's.
    /// The caller gives every failure here `CopyStarted`, since the copy may have started.
    fn finish_copy(
        &self,
        catalog: &Catalog,
        job: &Job,
        (temp, carried): (&RulePath, Write),
        original: &Original<'_>,
    ) -> Result<()> {
        #[cfg(any(test, feature = "fault"))]
        fault::in_copy(self.fault, job.file_id)?;
        let Original { path, native, shown } = *original;
        let rules = catalog.rules();
        let written = Job { write: carried, ..job.clone() };
        let temp_path = disk_path(rules, temp);
        match std::fs::read(&temp_path) {
            Ok(data) => {
                let file =
                    pg_platform::file::open_for_copy(native).with_context(|| format!("opening {shown} for writing"))?;
                write_into(file, &data)?;
            }
            // A recorded temp is deleted only once the write is recorded, so a missing one was deleted after its copy
            // by an earlier build (which deleted it before recording the write), or by another program (which can't
            // be told apart). The original is taken as holding it; the stat below fails if the drive is gone.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let after = stat(path)?;
        let rewrite = Rewrite { before: None, after, metadata_only: None };
        catalog.write_done(&written, Self::rewrote(&written, rewrite))?;
        let _ = std::fs::remove_file(&temp_path);
        if job.kind() == WriteKind::Turns {
            self.pixels_changed();
        }
        Ok(())
    }

    /// The effect of a job that rewrote its file: the rating it now holds, or the turns it shows.
    fn rewrote(job: &Job, rewrite: Rewrite) -> WriteEffect {
        match job.write {
            Write::Rating(rating) => WriteEffect::FileRating { rewrite, rating },
            Write::Turns(turns) => WriteEffect::TurnedBack { rewrite, turns },
        }
    }

    /// A turn the file can't hold (`turns`, the job's) is kept in the catalog instead.
    fn keep_turn_in_catalog(&self, catalog: &Catalog, job: &Job, turns: Turns) -> Result<()> {
        catalog.write_done(job, WriteEffect::KeptInCatalog(turns))?;
        self.pixels_changed();
        Ok(())
    }

    fn pixels_changed(&self) {
        self.turned_once();
    }
}

/// The temp of a failed job whose copy into the original may have started (as the user sees its path): the job's
/// recorded temp, or the one recorded in the attempt that failed. `None` when the original wasn't touched.
fn copy_temp(catalog: &Catalog, job: &Job, error: &anyhow::Error) -> Option<String> {
    error
        .downcast_ref::<CopyStarted>()
        .map(|started| started.temp.clone())
        .or_else(|| job.temp.as_ref().map(|(temp, _)| catalog.rules().display(temp)))
}

/// The job's file as the user sees its path, or its id when the catalog can't say.
fn shown_file(catalog: &Catalog, job: &Job) -> String {
    match catalog.file_path(job.file_id) {
        Ok(Some(path)) => catalog.rules().display(&path),
        _ => format!("file {}", job.file_id.get()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_incomplete_copy_names_the_file_and_its_temp() {
        let error = WriteError::CopyIncomplete {
            file: r"D:\p\a.jpg".into(),
            temp: r"D:\p\.a.jpg.rating.pg-write".into(),
            cause: Cause::Error("disk full".into()),
        };
        let text = error.to_string();
        assert!(text.starts_with(r"D:\p\a.jpg may be incomplete: copying its change into it failed"), "{text}");
        assert!(text.contains("(disk full)"), "{text}");
        assert!(text.ends_with(r"the whole new content is in D:\p\.a.jpg.rating.pg-write"), "{text}");
    }

    #[test]
    fn a_failed_copy_is_known_from_its_context() {
        let io = std::io::Error::other("disk full");
        let e = Err::<(), _>(io).with_context(|| CopyStarted { temp: "t".into() }).unwrap_err();
        assert_eq!(e.downcast_ref::<CopyStarted>().map(|c| c.temp.as_str()), Some("t"));
        assert!(e.downcast_ref::<std::io::Error>().is_some(), "the offline rule still sees the I/O error");
    }
}
