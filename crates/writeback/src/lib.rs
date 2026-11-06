//! Writing the user's changes into the files themselves (PLAN §2): ratings as `XMP:Rating` (ExifTool), turns
//! losslessly (`pg_edit::rotate`). The catalog changed already when the user acted; this catches the files up,
//! from the `file_writes` queue, so a write survives quitting and a file that is offline waits for its drive.
//!
//! Files keep their NTFS identity, which is the catalog's identity for them (PLAN §4). Writing a temp file and
//! renaming it over the original (what `exiftool -overwrite_original` and jpegtran-based tools do) would make the
//! file new to the catalog, and lose its rating. So every write has two steps:
//! 1. the new content is written, complete, to a temp file beside the original; its path goes in the queue row;
//! 2. the temp file is copied into the original in place, and deleted.
//!
//! A crash during step 2 leaves the row with its temp set, and the next start finishes the copy.

use pg_model::ids::FileId;
use pg_model::turns::Turns;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use pg_analysis::analyze::file_hash;
use pg_catalog::Catalog;
use pg_catalog::writes::{self, Job, MAX_ATTEMPTS, Rewrite, WriteEffect};
use pg_edit::rotate::{Outcome, rotate};
use pg_exiftool::ExifTool;
use pg_jobs::events::{Bus, Watch};
use pg_jobs::service::{Service, Waker, Woke};
use pg_metadata::{self, Meta, MetaScratch, patch_orientation};
use pg_model::format::Format;
use pg_model::media::Kind;
use pg_model::rules::{Path as RulePath, PathRules};

use pg_model::write::WriteKind;
use pg_platform::names::{path_buf, text_of};
use pg_thumbcache::ThumbCache;

use pg_jobs::events::Change;

/// A rating is written once it has been left alone this long, so quick changes (triage) coalesce.
const SETTLE_SECS: i64 = 2;
/// How often the queue is looked at without being woken (retries, offline drives).
const IDLE_CHECK: Duration = Duration::from_secs(30);

/// What the app shows about the queue.
#[derive(Clone, Default)]
pub struct WriteStatus {
    pub queued: usize,
    /// The latest failure, until the next success.
    pub error: Option<String>,
    /// The latest thumbnail that couldn't follow a rewritten file (the cache busy or closing). The thumbnail is made
    /// again, and the write is done all the same.
    pub thumbnail_error: Option<String>,
}

struct Inner {
    catalog_path: PathBuf,
    thumbs: &'static ThumbCache,
    status: Watch<WriteStatus>,
    /// Files turned so far: their pixels changed, so the app reloads them and the job re-analyzes them.
    turned: Watch<u64>,
    /// Told what changed (`Change::Writes`, `Change::Turned`).
    changes: Arc<Bus<Change>>,
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
        let inner = Arc::new(Inner {
            catalog_path,
            thumbs,
            status: Watch::new(WriteStatus::default()),
            turned: Watch::new(0),
            changes,
        });
        let job = inner.clone();
        let service = Service::start("file writer", move |waker| job.run(waker)).context("start the file writer")?;
        Ok(Writer { inner, service })
    }

    /// Something was queued.
    pub fn wake(&self) {
        self.service.wake();
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
        while waker.wait(Some(IDLE_CHECK)) != Woke::Quit {
            let drained =
                catalog.take().map_or_else(|| Catalog::open(&self.catalog_path, PathRules::NATIVE), Ok).and_then(|c| {
                    self.drain(waker, &c, &mut exiftool)?;
                    catalog = Some(c);
                    Ok(())
                });
            if let Err(e) = drained {
                self.update_status(|s| s.error = Some(format!("{e:#}")));
            }
        }
    }

    /// Work through the queue until nothing is ready; then wait for what is still settling.
    fn drain(&self, waker: &Waker<'_>, catalog: &Catalog, exiftool: &mut Option<ExifTool>) -> Result<()> {
        loop {
            if waker.quitting() {
                return Ok(());
            }
            let queued = writes::remaining(catalog)?;
            self.update_status(|s| s.queued = queued);
            // Finishing interrupted copies first; then the oldest settled change (`writes::next_ready`).
            let Some(job) = writes::next_ready(catalog, SETTLE_SECS)? else {
                if writes::attemptable(catalog)? {
                    // Come back when the newest change has settled.
                    std::thread::sleep(Duration::from_secs(u64::try_from(SETTLE_SECS)?));
                    continue;
                }
                return Ok(());
            };
            // A file the catalog marks missing, or no longer lists, can't be written: its queued write is dropped, and
            // the status line says why.
            if catalog.file_gone(job.file_id)? {
                if let Some((temp, _)) = &job.temp {
                    let _ = std::fs::remove_file(temp);
                }
                writes::drop_gone(catalog, &job)?;
                let message = format!(
                    "A queued {} was dropped: file {} is not in the library",
                    job.kind.key(),
                    job.file_id.get()
                );
                self.update_status(|s| s.error = Some(message));
                continue;
            }
            match self.process(catalog, &job, exiftool) {
                Ok(()) => self.update_status(|s| s.error = None),
                Err(e) => {
                    // Offline: the drive (or the file's folder with it) is not there, and the job waits for it. A
                    // missing file on a drive that is there is a failure, counted like any other.
                    let offline = job.temp.is_none()
                        && e.downcast_ref::<std::io::Error>().is_some_and(|io| match io.kind() {
                            std::io::ErrorKind::PermissionDenied => true,
                            std::io::ErrorKind::NotFound => !self.folder_present(catalog, job.file_id),
                            _ => false,
                        });
                    let message = format!("{e:#}");
                    writes::failed(catalog, &job, &message, offline)?;
                    self.update_status(|s| s.error = Some(message));
                    if offline {
                        // Leave it for the next idle check rather than spinning on it.
                        return Ok(());
                    }
                    if job.temp.is_some() && job.attempts + 1 >= MAX_ATTEMPTS {
                        self.give_up(catalog, &job)?;
                    }
                }
            }
        }
    }

    /// Whether the folder the file is in exists, so its drive is there. A catalog error is not an answer: the folder is
    /// taken as there, so the failure counts as an attempt and the job is retried, instead of waiting as offline.
    fn folder_present(&self, catalog: &Catalog, file_id: FileId) -> bool {
        match catalog.file_folder(file_id) {
            Ok(folder) => folder.is_some_and(|p| path_buf(&catalog.rules().join(&p)).is_dir()),
            Err(_) => true,
        }
    }

    /// A finishing copy that failed at the cap: its temp is discarded, and the job no longer counts as being written
    /// (`writes::abandon_temp`). The row stays, with its attempts and error, so the failure is still in the queue.
    fn give_up(&self, catalog: &Catalog, job: &Job) -> Result<()> {
        let Some((temp, _)) = &job.temp else { return Ok(()) };
        let path = text_path(catalog, &catalog.file_path(job.file_id)?)?;
        let _ = std::fs::remove_file(temp);
        Ok(writes::abandon_temp(catalog, job, stat(&path).ok())?)
    }

    fn process(&self, catalog: &Catalog, job: &Job, exiftool: &mut Option<ExifTool>) -> Result<()> {
        let (path_rule, format, kind) = catalog.write_target(job.file_id)?;

        // Videos aren't rewritten (a copy of GBs per change): a turn stays in the catalog, and their ratings are
        // never queued (pg_catalog::writes::set_rating).
        if kind == Kind::Video.key() && job.temp.is_none() {
            match job.kind {
                WriteKind::Turns => self.keep_turn_in_catalog(catalog, job)?,
                WriteKind::Rating => writes::done(catalog, job, WriteEffect::Nothing)?,
            }
            return Ok(());
        }
        // The queue and the programs take text: a file whose name has no UTF-8 form is not written.
        let path = text_path(catalog, &path_rule)?;

        if let Some((temp, _)) = &job.temp {
            // Interrupted after the new content was complete: finish the copy. What the file held before is
            // unknown now, so its analysis is redone rather than carried over.
            copy_into(Path::new(temp), &path)?;
            let after = stat(&path)?;
            let rewrite = Rewrite { before: (-1, -1), after, metadata_only: None };
            writes::done(catalog, job, Self::rewrote(job, rewrite))?;
            if job.kind == WriteKind::Turns {
                self.pixels_changed();
            }
            return Ok(());
        }

        let bytes = std::fs::read(&path).with_context(|| format!("reading {path}"))?;
        let format = format
            .as_deref()
            .and_then(Format::from_name)
            .or_else(|| Format::sniff(&bytes))
            .context("unknown format")?;
        let before = stat(&path)?;
        let temp = temp_path(catalog.rules().sep(), &path);
        let _ = std::fs::remove_file(&temp);

        let pixels_changed = match job.kind {
            WriteKind::Rating => {
                let rating = u8::try_from(job.value).with_context(|| format!("{path}: no rating is {}", job.value))?;
                let tool = ExifTool::running(exiftool, &exiftool_program())?;
                tool.write_rating(&path, &temp, rating)?;
                let written = std::fs::read(&temp)?;
                if Meta::read(&written, format, &mut MetaScratch::default()).rating != Some(rating) {
                    let _ = std::fs::remove_file(&temp);
                    bail!("{path}: the rating didn't read back after writing it");
                }
                false
            }
            WriteKind::Turns => {
                let new = match rotate(&bytes, format, job_turns(job))? {
                    Outcome::Rewritten(new) => Some(new),
                    Outcome::Orientation(o) => match patch_orientation(&bytes, format, o) {
                        Some(new) => Some(new),
                        None => {
                            // No tag to change: ExifTool adds one. A format it can't add EXIF to keeps the turn in
                            // the catalog.
                            let tool = ExifTool::running(exiftool, &exiftool_program())?;
                            match tool.write_orientation(&path, &temp, o.get()) {
                                Ok(()) if pg_metadata::orientation(&std::fs::read(&temp)?, format) == o => None,
                                _ => {
                                    let _ = std::fs::remove_file(&temp);
                                    return self.keep_turn_in_catalog(catalog, job);
                                }
                            }
                        }
                    },
                    Outcome::Catalog => return self.keep_turn_in_catalog(catalog, job),
                };
                if let Some(new) = new {
                    std::fs::write(&temp, new).with_context(|| format!("writing {temp}"))?;
                }
                true
            }
        };

        // Step 2: the new content is complete; record it, then copy it in. From here until `done` records the write,
        // readers know the file may be part-written (pg_model::state::FileState::writing).
        writes::record_temp(catalog, job, &temp, job.value)?;
        let new_hash = if pixels_changed { None } else { Some(file_hash(&std::fs::read(&temp)?)) };
        copy_into(Path::new(&temp), &path)?;
        let after = stat(&path)?;
        let rewrite = Rewrite { before, after, metadata_only: new_hash };
        writes::done(catalog, job, Self::rewrote(job, rewrite))?;
        if job.kind == WriteKind::Rating {
            // The pixels didn't change: the thumbnails still show the file. If they can't follow it (the cache
            // busy or closing), they are only made again: the write is done all the same.
            if let Err(e) = self.thumbs.retag(job.file_id, before.1, after.1) {
                let message = format!("file {}: {e:#}", job.file_id.get());
                self.update_status(|s| s.thumbnail_error = Some(message));
            }
        }
        if pixels_changed {
            self.pixels_changed();
        }
        Ok(())
    }

    /// The effect of a job that rewrote its file: the rating it now holds, or the turns it shows.
    fn rewrote(job: &Job, rewrite: Rewrite) -> WriteEffect {
        match job.kind {
            WriteKind::Rating => {
                WriteEffect::FileRating { rewrite, rating: u8::try_from(job.value.clamp(0, 5)).unwrap_or(u8::MAX) }
            }
            WriteKind::Turns => WriteEffect::TurnedBack { rewrite, turns: job_turns(job) },
        }
    }

    /// A turn the file can't hold is kept in the catalog instead.
    fn keep_turn_in_catalog(&self, catalog: &Catalog, job: &Job) -> Result<()> {
        let turns = job_turns(job);
        writes::done(catalog, job, WriteEffect::KeptInCatalog(turns))?;
        self.pixels_changed();
        Ok(())
    }

    fn pixels_changed(&self) {
        self.turned_once();
    }
}

/// The quarter turns a turn job writes (its value, taken modulo 4).
fn job_turns(job: &Job) -> Turns {
    Turns::new(u8::try_from(job.value.rem_euclid(4)).unwrap_or(u8::MAX))
}

/// The path of a file as text: the one the queue and the programs take. A name with no UTF-8 form is refused.
fn text_path(catalog: &Catalog, path: &RulePath) -> Result<String> {
    Ok(text_of(&catalog.rules().join(path))?.to_string())
}

/// Size and mtime (FILETIME, as the scanner records it).
pub fn stat(path: impl AsRef<Path>) -> Result<(i64, i64)> {
    let path = path.as_ref();
    let m = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    Ok((i64::try_from(m.len())?, pg_platform::file::last_write_time(&m)))
}

/// Beside the original, so it is on the same drive; the scanner ignores the extension.
fn temp_path(sep: char, path: &str) -> String {
    let (dir, name) = path.rsplit_once(sep).unwrap_or((".", path));
    format!("{dir}{sep}.{name}.pg-write")
}

/// Copy `temp` into the original in place (same file, same NTFS identity), then delete `temp`.
fn copy_into(temp: &Path, path: &str) -> Result<()> {
    let data = std::fs::read(temp).with_context(|| format!("reading {}", temp.display()))?;
    let mut file =
        std::fs::OpenOptions::new().write(true).open(path).with_context(|| format!("opening {path} for writing"))?;
    file.write_all(&data)?;
    file.set_len(u64::try_from(data.len())?)?;
    file.sync_all()?;
    drop(file);
    std::fs::remove_file(temp)?;
    Ok(())
}

/// The ExifTool program: `PG_EXIFTOOL`, or `exiftool` on PATH. Read here, where the writes start it.
pub fn exiftool_program() -> std::ffi::OsString {
    std::env::var_os("PG_EXIFTOOL").unwrap_or_else(|| "exiftool".into())
}
