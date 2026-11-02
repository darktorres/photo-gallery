//! Writing the user's changes into the files themselves (PLAN §2): ratings as `XMP:Rating` (ExifTool), turns
//! losslessly (`pg_core::rotate`). The catalog changed already when the user acted; this catches the files up,
//! from the `file_writes` queue, so a write survives quitting and a file that is offline waits for its drive.
//!
//! Files keep their NTFS identity, which is the catalog's identity for them (PLAN §4). Writing a temp file and
//! renaming it over the original (what `exiftool -overwrite_original` and jpegtran-based tools do) would make the
//! file new to the catalog, and lose its rating. So every write has two steps:
//! 1. the new content is written, complete, to a temp file beside the original; its path goes in the queue row;
//! 2. the temp file is copied into the original in place, and deleted.
//!
//! A crash during step 2 leaves the row with its temp set, and the next start finishes the copy.

use std::io::{BufRead, BufReader, Write};
use std::os::windows::fs::MetadataExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use pg_core::analyze::file_hash;
use pg_core::catalog::Catalog;
use pg_core::format::Format;
use pg_core::media::Kind;
use pg_core::metadata::{self, Meta, MetaScratch, patch_orientation};
use pg_core::rotate::{Outcome, rotate};
use pg_core::thumbs::ThumbCache;
use rusqlite::{OptionalExtension, params};

/// A rating is written once it has been left alone this long, so quick changes (triage) coalesce.
const SETTLE_SECS: i64 = 2;
/// Failed writes are retried at most this many times (offline drives retry without counting).
const MAX_ATTEMPTS: i64 = 5;
/// How often the queue is looked at without being woken (retries, offline drives).
const IDLE_CHECK: Duration = Duration::from_secs(30);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// One queued change.
struct Job {
    file_id: i64,
    kind: String,
    value: i64,
    /// Set when the new content is complete: its path, and the value it carries.
    temp: Option<(String, i64)>,
}

/// What the app shows about the queue.
#[derive(Clone, Default)]
pub struct WriteStatus {
    pub queued: usize,
    /// The latest failure, until the next success.
    pub error: Option<String>,
}

struct Inner {
    catalog_path: PathBuf,
    thumbs: &'static ThumbCache,
    wake: (Mutex<bool>, Condvar),
    status: Mutex<WriteStatus>,
    /// Files whose pixels changed (turned), so the app reloads them.
    turned: AtomicU64,
    on_pixels_changed: Box<dyn Fn() + Send + Sync>,
    stop: AtomicBool,
}

pub struct Writer {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Writer {
    /// Start the writer thread. `on_pixels_changed` runs after a file was turned: its analysis is stale, so the
    /// background job should run.
    pub fn start(
        catalog_path: PathBuf,
        thumbs: &'static ThumbCache,
        on_pixels_changed: impl Fn() + Send + Sync + 'static,
    ) -> Writer {
        let inner = Arc::new(Inner {
            catalog_path,
            thumbs,
            wake: (Mutex::new(true), Condvar::new()),
            status: Mutex::new(WriteStatus::default()),
            turned: AtomicU64::new(0),
            on_pixels_changed: Box::new(on_pixels_changed),
            stop: AtomicBool::new(false),
        });
        let job = inner.clone();
        let thread =
            std::thread::Builder::new().name("file writer".into()).spawn(move || job.run()).expect("spawn writer");
        Writer { inner, thread: Mutex::new(Some(thread)) }
    }

    /// Something was queued.
    pub fn wake(&self) {
        *self.inner.wake.0.lock().unwrap() = true;
        self.inner.wake.1.notify_one();
    }

    pub fn status(&self) -> WriteStatus {
        self.inner.status.lock().unwrap().clone()
    }

    /// Counts files turned so far, for the app to notice.
    pub fn turned(&self) -> u64 {
        self.inner.turned.load(Ordering::Relaxed)
    }

    /// Stop the thread after the write in progress (a queued write waits for the next start), and wait for it.
    /// Idempotent; dropping the writer does it too.
    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        self.wake();
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Inner {
    fn run(&self) {
        let mut exiftool: Option<ExifTool> = None;
        // One connection for every drain; after a drain that failed, a new one.
        let mut catalog: Option<Catalog> = None;
        loop {
            {
                let (lock, cvar) = &self.wake;
                let mut woken = lock.lock().unwrap();
                if !*woken {
                    woken = cvar.wait_timeout(woken, IDLE_CHECK).unwrap().0;
                }
                *woken = false;
            }
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let drained = catalog.take().map_or_else(|| Catalog::open_current(&self.catalog_path), Ok).and_then(|c| {
                self.drain(&c, &mut exiftool)?;
                catalog = Some(c);
                Ok(())
            });
            if let Err(e) = drained {
                self.status.lock().unwrap().error = Some(format!("{e:#}"));
            }
        }
    }

    /// Work through the queue until nothing is ready; then wait for what is still settling.
    fn drain(&self, catalog: &Catalog, exiftool: &mut Option<ExifTool>) -> Result<()> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let queued: usize = catalog.conn.query_row("SELECT count(*) FROM file_writes", [], |r| r.get(0))?;
            self.status.lock().unwrap().queued = queued;
            // Finishing interrupted copies first; then the oldest settled change.
            let job = catalog
                .conn
                .query_row(
                    "SELECT file_id, kind, value, temp, temp_value FROM file_writes
                     WHERE temp IS NOT NULL OR (attempts < ?1 AND queued <= unixepoch() - ?2)
                     ORDER BY temp IS NULL, queued LIMIT 1",
                    params![MAX_ATTEMPTS, SETTLE_SECS],
                    |r| {
                        let temp: Option<String> = r.get(3)?;
                        let temp_value: Option<i64> = r.get(4)?;
                        Ok(Job { file_id: r.get(0)?, kind: r.get(1)?, value: r.get(2)?, temp: temp.zip(temp_value) })
                    },
                )
                .optional()?;
            let Some(job) = job else {
                let settling: bool = catalog.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM file_writes WHERE attempts < ?1)",
                    [MAX_ATTEMPTS],
                    |r| r.get(0),
                )?;
                if settling {
                    // Come back when the newest change has settled.
                    std::thread::sleep(Duration::from_secs(SETTLE_SECS as u64));
                    continue;
                }
                return Ok(());
            };
            match self.process(catalog, &job, exiftool) {
                Ok(()) => self.status.lock().unwrap().error = None,
                Err(e) => {
                    let offline = e.downcast_ref::<std::io::Error>().is_some_and(|io| {
                        matches!(io.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied)
                    }) && job.temp.is_none();
                    let message = format!("{e:#}");
                    catalog.conn.execute(
                        "UPDATE file_writes SET attempts = attempts + ?3, error = ?4, queued = unixepoch()
                         WHERE file_id = ?1 AND kind = ?2",
                        params![job.file_id, job.kind, if offline { 0 } else { 1 }, message],
                    )?;
                    self.status.lock().unwrap().error = Some(message);
                    if offline {
                        // Leave it for the next idle check rather than spinning on it.
                        return Ok(());
                    }
                }
            }
        }
    }

    fn process(&self, catalog: &Catalog, job: &Job, exiftool: &mut Option<ExifTool>) -> Result<()> {
        let conn = &catalog.conn;
        let (path, format, kind): (String, Option<String>, String) = conn.query_row(
            "SELECT fo.path || '\\' || f.name, a.format, f.kind FROM files f JOIN folders fo ON fo.id = f.folder_id
             LEFT JOIN analysis a ON a.file_id = f.id WHERE f.id = ?1",
            [job.file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

        // Videos aren't rewritten (a copy of GBs per change): a turn stays in the catalog, and their ratings are
        // never queued (browse::writes::set_rating).
        if kind == Kind::Video.key() && job.temp.is_none() {
            if job.kind == "rotation" {
                self.keep_turn_in_catalog(conn, job)?;
            } else {
                self.done(conn, job, job.value, |_| Ok(()))?;
            }
            return Ok(());
        }

        if let Some((temp, written)) = &job.temp {
            // Interrupted after the new content was complete: finish the copy. What the file held before is
            // unknown now, so its analysis is redone rather than carried over.
            copy_into(Path::new(temp), &path)?;
            let after = stat(&path)?;
            self.done(conn, job, *written, |tx| {
                Catalog::file_rewritten(tx, job.file_id, (-1, -1), after, None)?;
                if job.kind == "rating" {
                    Catalog::set_file_rating(tx, job.file_id, Some(*written as u8))?;
                }
                Ok(())
            })?;
            if job.kind == "rotation" {
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
        let temp = temp_path(&path);
        let _ = std::fs::remove_file(&temp);

        let pixels_changed = match job.kind.as_str() {
            "rating" => {
                let rating = job.value as u8;
                let tool = exiftool_for(exiftool)?;
                tool.write_rating(&path, &temp, rating)?;
                let written = std::fs::read(&temp)?;
                if Meta::read(&written, format, &mut MetaScratch::default()).rating != Some(rating) {
                    let _ = std::fs::remove_file(&temp);
                    bail!("{path}: the rating didn't read back after writing it");
                }
                false
            }
            "rotation" => {
                let turns = job.value.rem_euclid(4) as u8;
                let new = match rotate(&bytes, format, turns)? {
                    Outcome::Rewritten(new) => Some(new),
                    Outcome::Orientation(o) => match patch_orientation(&bytes, format, o) {
                        Some(new) => Some(new),
                        None => {
                            // No tag to change: ExifTool adds one. A format it can't add EXIF to keeps the turn in
                            // the catalog.
                            let tool = exiftool_for(exiftool)?;
                            match tool.write_orientation(&path, &temp, o) {
                                Ok(()) if metadata::orientation(&std::fs::read(&temp)?, format) == o => None,
                                _ => {
                                    let _ = std::fs::remove_file(&temp);
                                    return self.keep_turn_in_catalog(conn, job);
                                }
                            }
                        }
                    },
                    Outcome::Catalog => return self.keep_turn_in_catalog(conn, job),
                };
                if let Some(new) = new {
                    std::fs::write(&temp, new).with_context(|| format!("writing {temp}"))?;
                }
                true
            }
            other => bail!("unknown write kind {other}"),
        };

        // Step 2: the new content is complete; record it, then copy it in. From here until `done` records the write,
        // readers know the file may be part-written (browse::read::FileState::writing).
        conn.execute(
            "UPDATE file_writes SET temp = ?3, temp_value = ?4 WHERE file_id = ?1 AND kind = ?2",
            params![job.file_id, job.kind, temp, job.value],
        )?;
        let new_hash = if pixels_changed { None } else { Some(file_hash(&std::fs::read(&temp)?)) };
        copy_into(Path::new(&temp), &path)?;
        let after = stat(&path)?;
        self.done(conn, job, job.value, |tx| {
            Catalog::file_rewritten(tx, job.file_id, before, after, new_hash)?;
            if job.kind == "rating" {
                Catalog::set_file_rating(tx, job.file_id, Some(job.value as u8))?;
            } else {
                Catalog::analysis_turned_back(tx, job.file_id, job.value.rem_euclid(4) as u8)?;
            }
            Ok(())
        })?;
        if job.kind == "rating" {
            // The pixels didn't change: the thumbnails still show the file. If they can't follow it (the cache
            // busy or closing), they are only made again: the write is done all the same.
            if let Err(e) = self.thumbs.retag(job.file_id, before.1, after.1) {
                eprintln!("thumbnails of file {}: {e:#}", job.file_id);
            }
        }
        if pixels_changed {
            self.pixels_changed();
        }
        Ok(())
    }

    /// A turn the file can't hold is kept in the catalog instead.
    fn keep_turn_in_catalog(&self, conn: &rusqlite::Connection, job: &Job) -> Result<()> {
        let turns = job.value.rem_euclid(4) as u8;
        self.done(conn, job, job.value, |tx| Catalog::add_catalog_rotation(tx, job.file_id, turns))?;
        self.pixels_changed();
        Ok(())
    }

    /// `written` (a rating, or a number of turns) is now in the file, or the catalog: `record` records what the
    /// write changed, in the same transaction, so that no reader sees the write both done and still queued (a turn
    /// shown twice), or the file rewritten and the catalog not yet saying so. A rating changed again meanwhile, or
    /// more turns added, stays queued for the rest.
    fn done(
        &self,
        conn: &rusqlite::Connection,
        job: &Job,
        written: i64,
        record: impl FnOnce(&rusqlite::Transaction) -> Result<()>,
    ) -> Result<()> {
        let (file_id, kind) = (job.file_id, job.kind.as_str());
        let tx = conn.unchecked_transaction()?;
        record(&tx)?;
        if kind == "rotation" {
            tx.execute(
                "UPDATE file_writes SET value = (value - ?2 + 4) % 4 WHERE file_id = ?1 AND kind = 'rotation'",
                params![file_id, written],
            )?;
            tx.execute("DELETE FROM file_writes WHERE file_id = ?1 AND kind = 'rotation' AND value = 0", [file_id])?;
            // The upright size at once: the grid lays the file out turned before its analysis is redone.
            tx.execute(
                "UPDATE analysis SET width = height, height = width WHERE file_id = ?1 AND ?2 % 2 = 1",
                params![file_id, written],
            )?;
        } else {
            tx.execute(
                "DELETE FROM file_writes WHERE file_id = ?1 AND kind = ?2 AND value = ?3",
                params![file_id, kind, written],
            )?;
        }
        tx.execute(
            "UPDATE file_writes SET temp = NULL, temp_value = NULL, attempts = 0, error = NULL
             WHERE file_id = ?1 AND kind = ?2",
            params![file_id, kind],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn pixels_changed(&self) {
        self.turned.fetch_add(1, Ordering::Relaxed);
        (self.on_pixels_changed)();
    }
}

/// Size and mtime (FILETIME, as the scanner records it).
pub(crate) fn stat(path: &str) -> Result<(i64, i64)> {
    let m = std::fs::metadata(path).with_context(|| format!("reading {path}"))?;
    Ok((m.len() as i64, m.last_write_time() as i64))
}

/// Beside the original, so it is on the same drive; the scanner ignores the extension.
fn temp_path(path: &str) -> String {
    let (dir, name) = path.rsplit_once('\\').unwrap_or((".", path));
    format!("{dir}\\.{name}.pg-write")
}

/// Copy `temp` into the original in place (same file, same NTFS identity), then delete `temp`.
fn copy_into(temp: &Path, path: &str) -> Result<()> {
    let data = std::fs::read(temp).with_context(|| format!("reading {}", temp.display()))?;
    let mut file =
        std::fs::OpenOptions::new().write(true).open(path).with_context(|| format!("opening {path} for writing"))?;
    file.write_all(&data)?;
    file.set_len(data.len() as u64)?;
    file.sync_all()?;
    drop(file);
    std::fs::remove_file(temp)?;
    Ok(())
}

fn exiftool_for(slot: &mut Option<ExifTool>) -> Result<&mut ExifTool> {
    if slot.is_none() {
        *slot = Some(ExifTool::start()?);
    }
    Ok(slot.as_mut().unwrap())
}

/// One ExifTool kept running (`-stay_open`), as starting it costs ~0.3 s per call.
pub struct ExifTool {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: BufReader<ChildStderr>,
    next: u64,
}

impl ExifTool {
    /// `PG_EXIFTOOL`, or `exiftool` on PATH.
    pub fn start() -> Result<ExifTool> {
        let program = std::env::var_os("PG_EXIFTOOL").unwrap_or_else(|| "exiftool".into());
        let mut child = Command::new(&program)
            .args(["-stay_open", "True", "-@", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .with_context(|| {
                format!("starting {} (ExifTool, needed to write ratings into files)", program.to_string_lossy())
            })?;
        die_with_us(&child);
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let stderr = BufReader::new(child.stderr.take().unwrap());
        Ok(ExifTool { child, stdin, stdout, stderr, next: 1 })
    }

    /// Run one command; returns (stdout, stderr).
    fn run(&mut self, args: &[&str]) -> Result<(String, String)> {
        let n = self.next;
        self.next += 1;
        // UTF-8 file names (non-ASCII paths), one argument per line.
        let mut input = String::from("-charset\nfilename=utf8\n");
        for a in args {
            input.push_str(a);
            input.push('\n');
        }
        input.push_str(&format!("-echo4\n{{done{n}}}\n-execute{n}\n"));
        self.stdin.write_all(input.as_bytes())?;
        self.stdin.flush()?;
        let read_until = |reader: &mut dyn BufRead, marker: &str| -> Result<String> {
            let mut out = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line)? == 0 {
                    bail!("ExifTool exited");
                }
                if line.trim_end() == marker {
                    return Ok(out);
                }
                out.push_str(&line);
            }
        };
        let stdout = read_until(&mut self.stdout, &format!("{{ready{n}}}"))?;
        let stderr = read_until(&mut self.stderr, &format!("{{done{n}}}"))?;
        Ok((stdout, stderr))
    }

    /// Write `XMP:Rating` into a copy of `path` at `out`. When ExifTool won't update the file as it is (damaged
    /// metadata), the metadata is rebuilt and the write retried once, as ImageRater did.
    pub fn write_rating(&mut self, path: &str, out: &str, rating: u8) -> Result<()> {
        let tag = format!("-XMP:Rating={rating}");
        let (stdout, stderr) = self.run(&[&tag, "-o", out, path])?;
        if created(&stdout) {
            return Ok(());
        }
        let _ = std::fs::remove_file(out);
        let (stdout2, stderr2) =
            self.run(&["-all=", "-tagsfromfile", "@", "-all:all", "-unsafe", "-icc_profile", &tag, "-o", out, path])?;
        if created(&stdout2) {
            return Ok(());
        }
        bail!("ExifTool couldn't write the rating into {path}: {} {}", stderr.trim(), stderr2.trim())
    }

    /// Copy every tag `from` holds that `to` can carry (EXIF, XMP, IPTC, ICC profile) into `to`, in place.
    pub fn copy_tags(&mut self, from: &str, to: &str) -> Result<()> {
        let (stdout, stderr) =
            self.run(&["-tagsfromfile", from, "-all:all", "-unsafe", "-icc_profile", "-overwrite_original", to])?;
        if stdout.contains("1 image files updated") || stdout.contains("1 image files unchanged") {
            return Ok(());
        }
        bail!("ExifTool couldn't copy the metadata of {from}: {}", stderr.trim())
    }

    /// Write an EXIF orientation into a copy of `path` at `out` (adding EXIF if the file has none).
    pub fn write_orientation(&mut self, path: &str, out: &str, orientation: u8) -> Result<()> {
        let tag = format!("-EXIF:Orientation={orientation}");
        let (stdout, stderr) = self.run(&["-n", &tag, "-o", out, path])?;
        if created(&stdout) {
            return Ok(());
        }
        bail!("ExifTool couldn't set the orientation of {path}: {}", stderr.trim())
    }
}

/// Put a child process in a job that closes, killing it, when this process ends however it ends (the app
/// quitting runs no destructors; a stay-open ExifTool would otherwise outlive it).
fn die_with_us(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    // The job handle is never closed: the system closes it when the process exits.
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: null security attributes and name are allowed; `info` is the struct this information class
        // takes, with its size.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of_val(&info) as u32,
            );
            job as usize
        }
    });
    // SAFETY: the job handle is never closed, and the child's handle is open while `child` lives.
    unsafe { AssignProcessToJobObject(job as _, child.as_raw_handle() as _) };
}

/// "1 image files created" (with `-o`).
fn created(stdout: &str) -> bool {
    stdout.lines().any(|l| l.trim().starts_with("1 image files created"))
}

impl Drop for ExifTool {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"-stay_open\nFalse\n");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}
