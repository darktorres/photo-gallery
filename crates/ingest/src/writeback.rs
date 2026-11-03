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

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use pg_core::analyze::file_hash;
use pg_core::catalog::Catalog;
use pg_core::metadata::{self, Meta, MetaScratch, patch_orientation};
use pg_core::rotate::{Outcome, rotate};
use pg_core::thumbs::ThumbCache;
use pg_model::format::Format;
use pg_model::media::Kind;
use rusqlite::{OptionalExtension, params};

/// A rating is written once it has been left alone this long, so quick changes (triage) coalesce.
const SETTLE_SECS: i64 = 2;
/// Failed writes are retried at most this many times (offline drives retry without counting).
const MAX_ATTEMPTS: i64 = 5;
/// How often the queue is looked at without being woken (retries, offline drives).
const IDLE_CHECK: Duration = Duration::from_secs(30);

/// One queued change.
struct Job {
    file_id: i64,
    kind: String,
    value: i64,
    /// Set when the new content is complete: its path, and the value it carries.
    temp: Option<(String, i64)>,
    /// Failed attempts so far.
    attempts: i64,
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
            // A job is tried again once its last attempt is older than the settle time times the attempts so far: a
            // failure waits longer each time, and at MAX_ATTEMPTS it is no longer picked (see give_up for a temp).
            let job = catalog
                .conn
                .query_row(
                    "SELECT file_id, kind, value, temp, temp_value, attempts FROM file_writes
                     WHERE attempts < ?1 AND queued <= unixepoch() - ?2 * (attempts + 1)
                     ORDER BY temp IS NULL, queued LIMIT 1",
                    params![MAX_ATTEMPTS, SETTLE_SECS],
                    |r| {
                        let temp: Option<String> = r.get(3)?;
                        let temp_value: Option<i64> = r.get(4)?;
                        Ok(Job {
                            file_id: r.get(0)?,
                            kind: r.get(1)?,
                            value: r.get(2)?,
                            temp: temp.zip(temp_value),
                            attempts: r.get(5)?,
                        })
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
            // A file the catalog marks missing, or no longer lists, can't be written: its queued write is dropped, and
            // the status line says why.
            let gone: bool = catalog.conn.query_row(
                "SELECT coalesce((SELECT missing FROM files WHERE id = ?1), 1)",
                [job.file_id],
                |r| r.get(0),
            )?;
            if gone {
                if let Some((temp, _)) = &job.temp {
                    let _ = std::fs::remove_file(temp);
                }
                catalog.conn.execute(
                    "DELETE FROM file_writes WHERE file_id = ?1 AND kind = ?2",
                    params![job.file_id, job.kind],
                )?;
                self.status.lock().unwrap().error =
                    Some(format!("A queued {} was dropped: file {} is not in the library", job.kind, job.file_id));
                continue;
            }
            match self.process(catalog, &job, exiftool) {
                Ok(()) => self.status.lock().unwrap().error = None,
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
                    if job.temp.is_some() && job.attempts + 1 >= MAX_ATTEMPTS {
                        self.give_up(catalog, &job)?;
                    }
                }
            }
        }
    }

    /// Whether the folder the file is in exists, so its drive is there.
    fn folder_present(&self, catalog: &Catalog, file_id: i64) -> bool {
        let folder: rusqlite::Result<String> = catalog.conn.query_row(
            "SELECT fo.path FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
            [file_id],
            |r| r.get(0),
        );
        folder.is_ok_and(|p| Path::new(&p).is_dir())
    }

    /// A finishing copy that failed at the cap: its temp is discarded, and the file's content counts as unknown, as
    /// after an interrupted copy (`file_rewritten` with no hash, so the import redoes the analysis). The file then no
    /// longer shows as being written. The row stays, with its attempts and error, so the failure is still in the queue.
    fn give_up(&self, catalog: &Catalog, job: &Job) -> Result<()> {
        let Some((temp, _)) = &job.temp else { return Ok(()) };
        let path: String = catalog.conn.query_row(
            "SELECT fo.path || '\\' || f.name FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
            [job.file_id],
            |r| r.get(0),
        )?;
        let _ = std::fs::remove_file(temp);
        let tx = catalog.conn.unchecked_transaction()?;
        if let Ok(after) = stat(&path) {
            Catalog::file_rewritten(&tx, job.file_id, (-1, -1), after, None)?;
        }
        tx.execute(
            "UPDATE file_writes SET temp = NULL, temp_value = NULL WHERE file_id = ?1 AND kind = ?2",
            params![job.file_id, job.kind],
        )?;
        tx.commit()?;
        Ok(())
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
            self.done(conn, job, *written, |tx| record_rewrite(tx, job, *written, (-1, -1), after, None))?;
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
                let tool = ExifTool::running(exiftool)?;
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
                            let tool = ExifTool::running(exiftool)?;
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
        self.done(conn, job, job.value, |tx| record_rewrite(tx, job, job.value, before, after, new_hash))?;
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
            if pg_core::orientation::turns_swap(written.rem_euclid(4) as u8) {
                tx.execute("UPDATE analysis SET width = height, height = width WHERE file_id = ?1", [file_id])?;
            }
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
/// What a write put in the file, recorded in the catalog in `tx` (`done`'s transaction): the file's new stat and
/// its analysis (`file_rewritten`), then what the file now holds, a rating or a turn it shows upright
/// (`analysis_turned_back`). Both the finishing of an interrupted copy and the normal write record it here, so the
/// two can't disagree about a turn.
fn record_rewrite(
    tx: &rusqlite::Transaction,
    job: &Job,
    value: i64,
    before: (i64, i64),
    after: (i64, i64),
    metadata_only: Option<[u8; 32]>,
) -> Result<()> {
    Catalog::file_rewritten(tx, job.file_id, before, after, metadata_only)?;
    if job.kind == "rating" {
        Catalog::set_file_rating(tx, job.file_id, Some(value as u8))?;
    } else {
        Catalog::analysis_turned_back(tx, job.file_id, value.rem_euclid(4) as u8)?;
    }
    Ok(())
}

pub(crate) fn stat(path: &str) -> Result<(i64, i64)> {
    let m = std::fs::metadata(path).with_context(|| format!("reading {path}"))?;
    Ok((m.len() as i64, pg_platform::file::last_write_time(&m)))
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

/// One ExifTool kept running (`-stay_open`), as starting it costs ~0.3 s per call.
pub struct ExifTool {
    child: Child,
    stdin: ChildStdin,
    /// Each pipe's lines, read by a thread of its own (`pump`).
    stdout: Receiver<String>,
    stderr: Receiver<String>,
    next: u64,
    /// A command's pipe failed (the process exited or its pipes broke): this one can't be used again.
    broken: bool,
}

impl ExifTool {
    /// The running ExifTool in `slot`, starting one if the slot is empty or its process has failed (R-38). A failed
    /// command is an error for that write only: the next one gets a new process.
    pub fn running(slot: &mut Option<ExifTool>) -> Result<&mut ExifTool> {
        if slot.as_ref().is_none_or(|t| t.broken) {
            *slot = Some(ExifTool::start()?);
        }
        Ok(slot.as_mut().unwrap())
    }

    /// `PG_EXIFTOOL`, or `exiftool` on PATH.
    pub fn start() -> Result<ExifTool> {
        let program = std::env::var_os("PG_EXIFTOOL").unwrap_or_else(|| "exiftool".into());
        let mut cmd = Command::new(&program);
        cmd.args(["-stay_open", "True", "-@", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        pg_platform::process::hide_console_window(&mut cmd);
        let mut child = cmd.spawn().with_context(|| {
            format!("starting {} (ExifTool, needed to write ratings into files)", program.to_string_lossy())
        })?;
        pg_platform::process::die_with_parent(&child);
        let stdin = child.stdin.take().unwrap();
        let stdout = pump(child.stdout.take().unwrap());
        let stderr = pump(child.stderr.take().unwrap());
        Ok(ExifTool { child, stdin, stdout, stderr, next: 1, broken: false })
    }

    /// Run one command; returns (stdout, stderr). A pipe failure marks the process broken.
    fn run(&mut self, args: &[&str]) -> Result<(String, String)> {
        let result = self.exchange(args);
        if result.is_err() {
            self.broken = true;
        }
        result
    }

    fn exchange(&mut self, args: &[&str]) -> Result<(String, String)> {
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
        let stdout = read_until(&self.stdout, &format!("{{ready{n}}}"))?;
        let stderr = read_until(&self.stderr, &format!("{{done{n}}}"))?;
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

/// "1 image files created" (with `-o`).
/// A pipe's lines, read by a thread of its own. ExifTool's stdout and stderr fill as it runs: read in turn, a full
/// stderr would stop it before the stdout marker that is read first (R-39). The thread ends at the pipe's end.
fn pump(pipe: impl Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// The lines up to `marker` (not included). An ExifTool that exited ends the wait.
fn read_until(lines: &Receiver<String>, marker: &str) -> Result<String> {
    let mut out = String::new();
    loop {
        let line = lines.recv().map_err(|_| anyhow::anyhow!("ExifTool exited"))?;
        if line.trim_end() == marker {
            return Ok(out);
        }
        out.push_str(&line);
        out.push('\n');
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A process that died is replaced by the next command, instead of failing every later write (R-38). Needs
    /// ExifTool; skipped without it.
    #[test]
    fn a_dead_exiftool_is_replaced() {
        let mut slot = match ExifTool::start() {
            Ok(tool) => Some(tool),
            Err(_) => {
                eprintln!("ExifTool not found: skipped");
                return;
            }
        };
        ExifTool::running(&mut slot).unwrap();
        let tool = slot.as_mut().unwrap();
        tool.child.kill().unwrap();
        tool.child.wait().unwrap();
        assert!(tool.run(&["-ver"]).is_err());
        assert!(slot.as_ref().unwrap().broken);

        ExifTool::running(&mut slot).unwrap();
        let tool = slot.as_mut().unwrap();
        assert!(!tool.broken);
        assert!(tool.run(&["-ver"]).is_ok());
    }
}
