//! Two ways of cutting a file's borders off (`pg_edit::crop`), each for the borders the catalog has for the file as found
//! (not marked as not a border), and each checked to be still the file its borders were found in:
//! - [`save`]: a cut copy, at `<folder>\<library folder place>\<path inside it>`, with the original's modification time.
//!   The originals are never written; a folder whose copies would land in the library is refused up front.
//! - [`cut`]: the original itself, rewritten in place, once the user has confirmed it. It is the only write of an
//!   original this crate makes, and it refuses to run without that confirmation.

#![forbid(unsafe_code)]

use pg_model::ids::FileId;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use pg_catalog::{Catalog, CropSource};
use pg_edit::crop::{self, crop};
use pg_model::path::{PathArena, PathRef};
use pg_model::rules::Path as RulePath;

use pg_exiftool::{ExifTool, Program};
use pg_jobs::control::Control;
use pg_jobs::failure::{Class, FAILURES_KEPT, Failed, file_failure, stored_path};
use pg_jobs::pipeline::{self, Options, Read};
use pg_model::notice::FileFailure;
use pg_model::progress::Phase;
use pg_platform::file::TempFile;
use pg_platform::names::disk_path_of;
use pg_platform::names::{set_path, shown};
use pg_platform::native::NativePath;
use pg_writeback::{ReplaceError, replace};

/// What a crop couldn't do: a step of the crop that failed (`Io`), a value it can't use (`Invalid`), the catalog, the
/// edit, ExifTool or the write-back, or the pass's own pipeline failure (`Pass`).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The catalog failed (see `pg_catalog::Error`).
    #[error(transparent)]
    Catalog(#[from] pg_catalog::Error),
    /// The edit couldn't cut the file (see `pg_edit::Error`).
    #[error("{0}")]
    Edit(#[from] pg_edit::Error),
    /// ExifTool failed (see `pg_exiftool::Error`).
    #[error("{0}")]
    Exiftool(#[from] pg_exiftool::Error),
    /// The write-back failed (see `pg_writeback::Error`).
    #[error("{0}")]
    Writeback(#[from] pg_writeback::Error),
    /// A file-system step failed: its words, and the I/O error under them.
    #[error("{what}")]
    Io {
        /// The step, in words.
        what: String,
        /// The I/O error under the step.
        #[source]
        source: std::io::Error,
    },
    /// A file or a value the pass can't use, with the words that say why.
    #[error("{0}")]
    Invalid(String),
    /// The pass's pipeline failed (see `pg_jobs::failure::Failed`).
    #[error(transparent)]
    Pass(#[from] Failed),
}

/// The crop's result.
pub type Result<T> = std::result::Result<T, Error>;

impl From<Error> for Failed {
    /// A file that couldn't be cropped is an ordinary failure, stored as a failed file of the pass.
    fn from(error: Error) -> Failed {
        match error {
            Error::Pass(failed) => failed,
            other => Failed::new(Class::Other, other),
        }
    }
}

/// A step of the crop that failed on `source`, with the words of the step.
fn io(what: impl Into<String>, source: std::io::Error) -> Error {
    Error::Io { what: what.into(), source }
}

/// What a save or a cut did: the counts its report states, the failures it kept, and what a stop left to do.
pub struct Stats {
    /// The copies saved.
    pub saved: usize,
    /// Nothing to crop now (no borders, or marked): not a file that changed or went.
    pub skipped: usize,
    /// The files that changed on disk since their borders were found (their size or time differ): left as they are.
    pub changed: usize,
    /// The files that are missing from the library, or no longer in it: left as they are.
    pub missing: usize,
    /// The files that failed, the first of them kept in `failures`.
    pub failed: usize,
    /// The first failures, each with its file, up to `FAILURES_KEPT`; `failed` counts them all. A panic on a file is a
    /// failure of that file (`Cause::Panicked`), which is not queued again.
    pub failures: Vec<FileFailure>,
    /// What a stop left (to queue again).
    pub remaining: Vec<FileId>,
}

struct Job {
    id: FileId,
    source: CropSource,
    /// Where its copy goes. Both paths are in the jobs' arena.
    out: PathRef,
}

/// Save cropped copies of `ids` under `folder`, their tags copied by the ExifTool the environment names.
pub fn save(catalog: &Catalog, ids: &[FileId], folder: &Path, opts: &Options, ctl: &Control) -> Result<Stats> {
    save_with(Program::from_env().as_os_str(), catalog, ids, folder, opts, ctl)
}

/// [`save`], with the ExifTool program named (a path, or a name on PATH): the tests name a program that cannot run, to
/// see a failed tag copy.
pub fn save_with(
    program: &OsStr,
    catalog: &Catalog,
    ids: &[FileId],
    folder: &Path,
    opts: &Options,
    ctl: &Control,
) -> Result<Stats> {
    // The pass's span in the perf log: its phase is its child, and the phase records what it did.
    let _pass = pg_perf::span("pass.crop");
    let rules = catalog.rules();
    // The folder is chosen by the user. It is taken as the disk spells it, not as text: a lossy text form would write
    // the copies into a different folder from the one named.
    // The copies' folder may not exist yet: its real part is resolved, and the rest is taken as typed (names::real_path_lenient).
    let folder = pg_platform::names::real_path_lenient(folder)
        .map_err(|e| Error::Invalid(format!("the folder for the copies isn't a path: {e}")))?;
    let roots: Vec<RulePath> = catalog.roots()?.into_iter().map(|r| r.path).collect();
    let mut stats =
        Stats { saved: 0, skipped: 0, changed: 0, missing: 0, failed: 0, failures: Vec::new(), remaining: Vec::new() };
    let mut jobs = Vec::with_capacity(ids.len());
    let mut paths = PathArena::default();
    for &id in ids {
        let Some(source) = catalog.crop_source(id, &mut paths)? else {
            // No source: the file went or changed since its borders were found, or it has nothing to crop (no borders, or
            // marked). Only the last is "nothing to crop".
            let gap = catalog.crop_gap(id)?;
            if gap.missing {
                stats.missing += 1;
            } else if gap.changed {
                stats.changed += 1;
            } else {
                stats.skipped += 1;
            }
            continue;
        };
        let path = rules
            .parse_bytes(paths.get(source.path))
            .ok_or_else(|| Error::Invalid("a stored path doesn't parse".into()))?;
        // The deepest library folder holding it (library folders don't nest; a removed one can be inside another).
        let Some(root) = roots.iter().filter(|r| rules.is_under(&path, r)).max_by_key(|r| r.parts.len()) else {
            // Not in the library any more: missing.
            stats.missing += 1;
            continue;
        };
        // The copies' folder: the chosen folder, then the library folder's place (its drive as a part, so
        // `D:\Photos\2019` is `D\Photos\2019`). Two library folders can't share a place, whatever their last names.
        let tree = RulePath {
            anchor: folder.anchor.clone(),
            parts: folder.parts.iter().cloned().chain(root.drive_letter()).chain(root.parts.iter().cloned()).collect(),
        };
        if let Some(r) = roots.iter().find(|r| rules.is_under(&tree, r) || rules.is_under(r, &tree)) {
            return Err(Error::Invalid(format!(
                "The copies of {} would go into {}, in the library folder {}: choose a folder outside it",
                rules.display(root),
                rules.display(&tree),
                rules.display(r)
            )));
        }
        let mut parts = tree.parts.clone();
        let inside = path
            .parts
            .get(root.parts.len()..)
            .ok_or_else(|| Error::Invalid("a file isn't inside its library folder".into()))?;
        parts.extend(inside.iter().cloned());
        let out = rules.join(&RulePath { anchor: tree.anchor.clone(), parts });
        let out = paths.push(&out);
        jobs.push(Job { id, source, out });
    }
    if jobs.is_empty() {
        return Ok(stats);
    }

    // The phase ends when this is dropped, however the pass leaves.
    let phase = ctl.begin(Phase::Crop, u64::try_from(jobs.len()).unwrap_or(u64::MAX));
    let mut done = vec![false; jobs.len()];
    pipeline::run(
        pipeline::Pass { items: &jobs, opts, ctl },
        |job, native| {
            set_path(native, paths.get(job.source.path));
            Read::File
        },
        || Ok::<CopyWorker, Error>(CopyWorker { exiftool: None, program: program.to_os_string() }),
        |worker: &mut CopyWorker, i, bytes| {
            let job = jobs.get(i).ok_or_else(|| Error::Invalid("an item outside the pass".into()))?;
            write_copy(job, &paths, bytes, worker)
        },
        |i, result, len| {
            let (Some(job), Some(job_done)) = (jobs.get(i), done.get_mut(i)) else {
                return Err(Failed::from(Error::Invalid("an item outside the pass".into())));
            };
            *job_done = true;
            let result = result.map_err(Error::Pass);
            ctl.add(len, result.is_err());
            match result {
                Ok(Written::Saved) => stats.saved += 1,
                Ok(Written::Changed) => stats.changed += 1,
                Err(e) => {
                    stats.failed += 1;
                    if stats.failures.len() < FAILURES_KEPT {
                        let failed = Failed::from(e);
                        stats.failures.push(file_failure(stored_path(rules, paths.get(job.source.path)), &failed));
                    }
                }
            }
            Ok(true)
        },
    )?;
    drop(phase);
    stats.remaining = jobs.iter().zip(&done).filter(|(_, d)| !**d).map(|(j, _)| j.id).collect();
    Ok(stats)
}

enum Written {
    Saved,
    /// Changed on disk since its borders were found: not copied.
    Changed,
}

/// What a worker of `save` keeps from one copy to the next: its ExifTool, started for the first copy that needs its tags
/// copied over, and the program that starts it.
struct CopyWorker {
    exiftool: Option<ExifTool>,
    program: OsString,
}

/// What `cut` did: the counts for the report, the files whose originals changed (the caller forgets their thumbnails),
/// and what a stop left to do (queue it again).
#[derive(Default)]
pub struct CutStats {
    /// The originals cut and saved.
    pub saved: usize,
    /// Nothing to cut now (no borders, marked, changed since), or not in the catalog any more.
    pub skipped: usize,
    /// A format that is not cut in place. Every still format is cut now, so this stays 0; it is kept for the report.
    pub unsupported: usize,
    /// The files that failed, the first of them kept in `failures`.
    pub failed: usize,
    /// The first failures, each with its file, up to `FAILURES_KEPT`; `failed` counts them all.
    pub failures: Vec<FileFailure>,
    /// The files whose originals were rewritten, or may have been part-written (a failed write): their thumbnails are forgotten.
    pub cut: Vec<FileId>,
    /// What a stop left to do (queue it again).
    pub remaining: Vec<FileId>,
}

struct CutJob {
    id: FileId,
    source: CropSource,
}

/// What writing one original came to.
enum CutWritten {
    Saved {
        after: (u64, i64),
    },
    /// Changed since its borders were found.
    Skipped,
}

/// What a worker of `cut` keeps from one original to the next: its ExifTool, started for the first original that needs
/// its tags copied over, and the program that starts it.
struct CutWorker {
    exiftool: Option<ExifTool>,
    program: OsString,
}

/// Cut the borders off the originals of `ids`, in place: each file's borders as the catalog has them, cut exactly into
/// bytes in the file's own format (`pg_edit::crop::cut`), which replace the original (`pg_writeback::replace`). The
/// originals change, so this refuses to run unless `confirmed`, which only the user's confirmation gives. No backup is
/// kept, and no border mark is set: the cut's file has no borders left, and its detection is stored as none. Bytes that
/// come out without their metadata (JPEG, lossy WebP, TIFF, compressed BMP) get the original's tags first, by ExifTool,
/// so a failed tag copy leaves the original as it was. A file that fails is counted and the rest go on; a file whose
/// catalog row can't be updated after its cut counts as failed, though its original is cut.
///
/// `present` says whether a file is still in the library. It is asked before each original is written, since a folder
/// can be removed while the pass runs: a file that is not is skipped, and its original is left as it is.
pub fn cut(
    catalog: &Catalog,
    ids: &[FileId],
    confirmed: bool,
    opts: &Options,
    ctl: &Control,
    present: &(dyn Fn(FileId) -> bool + Sync),
) -> Result<CutStats> {
    cut_with(Program::from_env().as_os_str(), catalog, ids, confirmed, opts, ctl, present)
}

/// [`cut`], with the ExifTool program named (see [`save_with`]).
pub fn cut_with(
    program: &OsStr,
    catalog: &Catalog,
    ids: &[FileId],
    confirmed: bool,
    opts: &Options,
    ctl: &Control,
    present: &(dyn Fn(FileId) -> bool + Sync),
) -> Result<CutStats> {
    // The pass's span in the perf log: its phase is its child, and the phase records what it did.
    let _pass = pg_perf::span("pass.cut");
    if !confirmed {
        return Err(Error::Invalid("the originals are changed only once the user has confirmed it".into()));
    }
    let rules = catalog.rules();
    let mut stats = CutStats::default();
    let mut jobs = Vec::with_capacity(ids.len());
    let mut paths = PathArena::default();
    for &id in ids {
        let Some(source) = catalog.crop_source(id, &mut paths)? else {
            stats.skipped += 1;
            continue;
        };
        jobs.push(CutJob { id, source });
    }
    if jobs.is_empty() {
        return Ok(stats);
    }

    let phase = ctl.begin(Phase::Crop, u64::try_from(jobs.len()).unwrap_or(u64::MAX));
    let mut done = vec![false; jobs.len()];
    pipeline::run(
        pipeline::Pass { items: &jobs, opts, ctl },
        |job, native| {
            set_path(native, paths.get(job.source.path));
            Read::File
        },
        || Ok::<CutWorker, Error>(CutWorker { exiftool: None, program: program.to_os_string() }),
        |worker: &mut CutWorker, i, bytes| {
            let job = jobs.get(i).ok_or_else(|| Error::Invalid("an item outside the pass".into()))?;
            cut_original(job, &paths, bytes, worker, present)
        },
        |i, result, len| {
            let (Some(job), Some(job_done)) = (jobs.get(i), done.get_mut(i)) else {
                return Err(Failed::from(Error::Invalid("an item outside the pass".into())));
            };
            *job_done = true;
            // A failed write may have part-written the original (an incomplete replace, which the message names): its
            // thumbnail is forgotten too, as the cut's files are.
            if result.is_err() {
                stats.cut.push(job.id);
            }
            let result = result.map_err(Error::Pass).and_then(|written| match written {
                CutWritten::Saved { after } => {
                    // The original is cut whatever the catalog says after this, so its thumbnails are forgotten either way.
                    stats.cut.push(job.id);
                    catalog.borders_cut(job.id, after).map_err(|e| {
                        Error::Invalid(format!(
                            "the borders are cut, but the catalog wasn't updated: {}",
                            pg_model::error::chain(&e)
                        ))
                    })?;
                    Ok(CutWritten::Saved { after })
                }
                other => Ok(other),
            });
            ctl.add(len, result.is_err());
            match result {
                Ok(CutWritten::Saved { .. }) => stats.saved += 1,
                Ok(CutWritten::Skipped) => stats.skipped += 1,
                Err(e) => {
                    stats.failed += 1;
                    if stats.failures.len() < FAILURES_KEPT {
                        let failed = Failed::from(e);
                        stats.failures.push(file_failure(stored_path(rules, paths.get(job.source.path)), &failed));
                    }
                }
            }
            Ok(true)
        },
    )?;
    drop(phase);
    stats.remaining = jobs.iter().zip(&done).filter(|(_, d)| !**d).map(|(j, _)| j.id).collect();
    Ok(stats)
}

/// Cut `job`'s borders off the original as it was read (`bytes`), and replace the original with the result. Bytes that
/// come out without their metadata take the original's tags first (`with_tags`), so a failure there leaves the original
/// untouched.
fn cut_original(
    job: &CutJob,
    paths: &PathArena,
    bytes: &[u8],
    worker: &mut CutWorker,
    present: &(dyn Fn(FileId) -> bool + Sync),
) -> Result<CutWritten> {
    // Removed since the pass chose it: its original is not written (a removal's files are the user's to be rid of).
    if !present(job.id) {
        return Ok(CutWritten::Skipped);
    }
    let s = &job.source;
    let path = disk_path_of(paths.get(s.path));
    if !still_listed(&path, s, bytes.len())? {
        return Ok(CutWritten::Skipped);
    }
    let cropped = crop::cut(bytes, s.format, s.width, s.height, s.cut)?;
    let new = if cropped.metadata {
        cropped.bytes
    } else {
        with_tags(&path, &cropped.bytes, &mut worker.exiftool, &worker.program)?
    };
    let mut native = NativePath::default();
    set_path(&mut native, paths.get(s.path));
    let temp = temp_beside(&path, ".cut.pg-write").ok_or_else(|| Error::Invalid("a file without a name".into()))?;
    match replace(&path, &native, &temp, &new, (s.size, s.mtime)) {
        Ok(after) => Ok(CutWritten::Saved { after }),
        // Nothing was copied into it: the original is as it was.
        Err(ReplaceError::Untouched(e)) => Err(Error::Writeback(e)),
        // Part-written: the whole new content is in the temp, which the message names.
        Err(ReplaceError::Incomplete { temp, error }) => Err(Error::Invalid(format!(
            "{} may be incomplete: copying its cut into it failed ({}); the whole new content is in {}",
            shown(&path),
            pg_model::error::chain(&error),
            shown(&temp)
        ))),
    }
}

/// A temporary beside `path`: the file's name with a dot in front and `suffix` after it, as the repository's temporaries
/// are named (`.a.jpg.rating.pg-write`, `.a.jpg.pg-crop`). A cut's replacement is `.cut.pg-write`; a copy's is `.pg-crop`.
fn temp_beside(path: &Path, suffix: &str) -> Option<PathBuf> {
    let name = path.file_name()?;
    let mut temp = OsString::from(".");
    temp.push(name);
    temp.push(suffix);
    Some(path.with_file_name(temp))
}

/// `bytes` (the cut of `original`, which came out without its metadata) with the original's tags copied onto them by
/// ExifTool. They are written to a temporary beside the original, tagged there, and read back; the temporary is removed
/// on every path. Nothing of the original is written.
fn with_tags(original: &Path, bytes: &[u8], exiftool: &mut Option<ExifTool>, program: &OsStr) -> Result<Vec<u8>> {
    let temp = tags_temp_beside(original).ok_or_else(|| Error::Invalid("a file without a name".into()))?;
    let guard = TempFile::new(&temp);
    std::fs::write(&temp, bytes).map_err(|e| io(format!("writing {}", shown(&temp)), e))?;
    ExifTool::running(exiftool, program)?.copy_tags(original, &temp)?;
    let tagged = std::fs::read(&temp).map_err(|e| io(format!("reading {}", shown(&temp)), e))?;
    drop(guard);
    Ok(tagged)
}

/// The temporary `with_tags` tags a cut in, beside `path`: the file's stem with a dot in front and `.cut-tags` after it,
/// then its extension, which ExifTool takes the file's format from.
fn tags_temp_beside(path: &Path) -> Option<PathBuf> {
    let mut name = OsString::from(".");
    name.push(path.file_stem()?);
    name.push(".cut-tags");
    if let Some(ext) = path.extension() {
        name.push(".");
        name.push(ext);
    }
    Some(path.with_file_name(name))
}

/// Whether the file at `path` is still the source the pass listed: its size and mtime are the listed ones, and the bytes
/// read are that size. A file changed since it was listed is left as it is.
fn still_listed(path: &Path, source: &CropSource, read_len: usize) -> Result<bool> {
    let now =
        pg_platform::file::stat(path).map_err(|e| io(format!("reading {}", pg_platform::names::shown(path)), e))?;
    Ok(now == (source.size, source.mtime) && usize::try_from(source.size).ok() == Some(read_len))
}

/// Write `job`'s copy of the file read as `bytes`, through a temporary beside its output. A copy whose bytes come out
/// without their metadata (TIFF, compressed BMP) is tagged before it is published, so a failed tag copy leaves whatever an
/// earlier save left at the output.
fn write_copy(job: &Job, paths: &PathArena, bytes: &[u8], worker: &mut CopyWorker) -> Result<Written> {
    let s = &job.source;
    let (source, out) = (paths.get(s.path), paths.get(job.out));
    // Each by its disk path, which reaches it exactly: a folder whose name ends in a dot or a space is the one read and
    // written, not a sibling without it.
    let (source_path, out_path) = (disk_path_of(source), disk_path_of(out));
    if !still_listed(&source_path, s, bytes.len())? {
        return Ok(Written::Changed);
    }
    let cropped = crop(bytes, s.format, s.width, s.height, s.cut)?;
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io(format!("creating {}", shown(dir)), e))?;
    }
    let bytes = if cropped.metadata {
        cropped.bytes
    } else {
        // ExifTool takes the format from the file's extension, so the tagging temporary keeps the output's name.
        let tagged = tags_temp_beside(&out_path).ok_or_else(|| Error::Invalid("a file without a name".into()))?;
        let guard = TempFile::new(&tagged);
        std::fs::write(&tagged, &cropped.bytes).map_err(|e| io(format!("writing {}", shown(&out_path)), e))?;
        // ExifTool reads and writes by the disk paths too, so a folder whose name ends in a dot is the one it reaches.
        tag_copy(&mut worker.exiftool, &worker.program, &source_path, &tagged)?;
        let tagged_bytes = std::fs::read(&tagged).map_err(|e| io(format!("reading {}", shown(&tagged)), e))?;
        drop(guard);
        tagged_bytes
    };
    // Complete or not there: written beside it, then renamed over whatever an earlier save left.
    let temp_path = temp_beside(&out_path, ".pg-crop").ok_or_else(|| Error::Invalid("a file without a name".into()))?;
    publish(&temp_path, &out_path, &bytes, &source_path)?;
    Ok(Written::Saved)
}

/// Give the published copy at `out` the tags of `source` that the crop left out (ExifTool), and its modification time.
fn tag_copy(exiftool: &mut Option<ExifTool>, program: &OsStr, source: &Path, out: &Path) -> Result<()> {
    ExifTool::running(exiftool, program)?.copy_tags(source, out)?;
    keep_time(source, out)
}

/// Write `bytes` to the temporary `temp`, give it `source`'s modification time, and rename it to `out`, so the copy
/// at `out` is complete or not there. The temporary is removed on any failure from its first write on (a write that
/// failed part way, a time not set, a rename refused): its guard is armed before the write and disarmed only once
/// the rename is done. (Its name is fixed, `.<out>.pg-crop`, so what a crash leaves is replaced by the next save.)
fn publish(temp: &Path, out: &Path, bytes: &[u8], source: &Path) -> Result<()> {
    // Each is a disk path, reached exactly; the messages name them as the user does.
    let guard = TempFile::new(temp);
    std::fs::write(temp, bytes).map_err(|e| io(format!("writing {}", shown(out)), e))?;
    keep_time(source, temp)?;
    std::fs::rename(temp, out).map_err(|e| io(format!("writing {}", shown(out)), e))?;
    guard.persist();
    Ok(())
}

/// Give the copy at `copy` the original's modification time (both disk paths).
fn keep_time(original: &Path, copy: &Path) -> Result<()> {
    let modified = std::fs::metadata(original)
        .and_then(|meta| meta.modified())
        .map_err(|e| io(format!("reading {}", shown(original)), e))?;
    std::fs::File::options()
        .write(true)
        .open(copy)
        .and_then(|file| file.set_modified(modified))
        .map_err(|e| io(format!("writing {}", shown(copy)), e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder with a source file in it, for `publish`.
    fn folder(name: &str) -> (PathBuf, PathBuf) {
        let dir = pg_testkit::run_folder("scratch_crop", name).unwrap();
        let source = dir.join("source.jpg");
        std::fs::write(&source, b"original").unwrap();
        (dir, source)
    }

    // Published: the copy is in place with the source's time, and no temporary is left.
    #[test]
    fn publish_renames_the_temp_into_place() {
        let (dir, source) = folder("crop-publish");
        let (temp, out) = (dir.join(".out.jpg.pg-crop"), dir.join("out.jpg"));
        publish(&temp, &out, b"cropped", &source).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"cropped");
        assert!(!temp.exists());
        let time = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(time(&out), time(&source));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A rename that fails (the copy's folder is gone) leaves no temporary, and no copy.
    #[test]
    fn a_failed_rename_leaves_no_temp() {
        let (dir, source) = folder("crop-rename");
        let (temp, out) = (dir.join(".out.jpg.pg-crop"), dir.join("gone").join("out.jpg"));
        assert!(publish(&temp, &out, b"cropped", &source).is_err());
        assert!(!temp.exists(), "the temporary is removed");
        assert!(!out.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A failure after the write (the source's time can't be read: it is gone) leaves no temporary; nor does a
    // write that fails.
    #[test]
    fn a_failed_write_or_time_leaves_no_temp() {
        let (dir, source) = folder("crop-write");
        let (temp, out) = (dir.join(".out.jpg.pg-crop"), dir.join("out.jpg"));
        assert!(publish(&temp, &out, b"cropped", &dir.join("missing.jpg")).is_err());
        assert!(!temp.exists(), "written, then removed");
        assert!(!out.exists());
        // The temporary's folder is missing: the write fails, and nothing is there.
        let temp = dir.join("gone").join(".out.jpg.pg-crop");
        assert!(publish(&temp, &out, b"cropped", &source).is_err());
        assert!(!temp.exists());
        assert!(!out.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
