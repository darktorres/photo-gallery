//! Saving cropped copies (`pg_edit::crop`): each file's borders as the catalog has them (found, not marked as not a
//! border), cut off into a copy at `<folder>\<library folder place>\<path inside it>`, with the original's
//! modification time. The originals are never written: a folder whose copies would land in the library is refused
//! up front, and each file is checked to still be the one its borders were found in.

use pg_model::ids::FileId;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use pg_catalog::{Catalog, CropSource};
use pg_edit::crop::crop;
use pg_model::name::Name;
use pg_model::path::{PathArena, PathRef};
use pg_model::rules::Path as RulePath;

use pg_exiftool::ExifTool;
use pg_jobs::control::Control;
use pg_jobs::failure::file_failure;
use pg_model::notice::FileFailure;
use pg_jobs::pipeline::{self, Options};
use pg_model::progress::Phase;
use pg_platform::file::TempFile;
use pg_platform::names::disk_path_of;
use pg_platform::names::{set_path, shown};

pub struct Stats {
    pub saved: usize,
    /// Nothing to crop now (no borders, marked, changed since), or not in the library any more.
    pub skipped: usize,
    pub failed: usize,
    /// The first failure, with its file. A panic on a file is a failure of that file (`Cause::Panicked`), which is
    /// not queued again.
    pub error: Option<FileFailure>,
    /// What a stop left (to queue again).
    pub remaining: Vec<FileId>,
}

struct Job {
    id: FileId,
    source: CropSource,
    /// Where its copy goes. Both paths are in the jobs' arena.
    out: PathRef,
}

/// The copies' folder as the disk spells it. The check that keeps the copies out of the library compares spellings, so
/// a typed 8.3 short name (`RUNNER~1`) or other case would slip past it. The folder may not exist yet: the longest part
/// that does is resolved, and the rest is taken as typed.
fn real_folder(folder: &Path) -> Result<RulePath> {
    let mut missing = Vec::new();
    let mut existing = folder;
    while !existing.exists() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else { break };
        missing.push(name);
        existing = parent;
    }
    if !existing.exists() {
        return pg_platform::names::rule_path_of(folder).context("the folder for the copies isn't a path");
    }
    let mut real = pg_platform::names::normalize_path(existing)?;
    if !missing.is_empty() {
        let tail: PathBuf = missing.iter().rev().collect();
        let tail = pg_platform::names::rule_path_of(&tail).context("the folder for the copies isn't a path")?;
        real.parts.extend(tail.parts);
    }
    Ok(real)
}

/// Save cropped copies of `ids` under `folder`.
pub fn save(catalog: &Catalog, ids: &[FileId], folder: &Path, opts: &Options, ctl: &Control) -> Result<Stats> {
    let rules = catalog.rules();
    // The folder is chosen by the user. It is taken as the disk spells it, not as text: a lossy text form would write
    // the copies into a different folder from the one named.
    let folder = real_folder(folder)?;
    let roots: Vec<RulePath> = catalog.roots()?.into_iter().map(|r| r.path).collect();
    let mut stats = Stats { saved: 0, skipped: 0, failed: 0, error: None, remaining: Vec::new() };
    let mut jobs = Vec::with_capacity(ids.len());
    let mut paths = PathArena::default();
    for &id in ids {
        let Some(source) = catalog.crop_source(id, &mut paths)? else {
            stats.skipped += 1;
            continue;
        };
        let path = rules.parse_bytes(paths.get(source.path)).context("a stored path doesn't parse")?;
        // The deepest library folder holding it (library folders don't nest; a removed one can be inside another).
        let Some(root) = roots.iter().filter(|r| rules.is_under(&path, r)).max_by_key(|r| r.parts.len()) else {
            stats.skipped += 1;
            continue;
        };
        // The copies' folder: the chosen folder, then the library folder's place (its drive as a part, so
        // `D:\Photos\2019` is `D\Photos\2019`). Two library folders can't share a place, whatever their last names.
        let tree = RulePath {
            anchor: folder.anchor.clone(),
            parts: folder.parts.iter().cloned().chain(place_of(root)).collect(),
        };
        if let Some(r) = roots.iter().find(|r| rules.is_under(&tree, r) || rules.is_under(r, &tree)) {
            bail!(
                "The copies of {} would go into {}, in the library folder {}: choose a folder outside it",
                rules.display(root),
                rules.display(&tree),
                rules.display(r)
            );
        }
        let mut parts = tree.parts.clone();
        let inside = path.parts.get(root.parts.len()..).context("a file isn't inside its library folder")?;
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
    let mut exiftool: Option<ExifTool> = None;
    pipeline::run(
        &jobs,
        |job, native| {
            set_path(native, paths.get(job.source.path));
            true
        },
        opts,
        ctl,
        || Ok(Vec::new()),
        |temp, i, bytes| write_copy(jobs.get(i).context("an item outside the pass")?, &paths, bytes, temp),
        |i, result, len| {
            let (Some(job), Some(job_done)) = (jobs.get(i), done.get_mut(i)) else {
                bail!("an item outside the pass");
            };
            *job_done = true;
            // TIFF and compressed BMP come out without their metadata.
            let result = result.and_then(|written| match written {
                Written::Skipped => Ok(false),
                Written::Saved { metadata: true } => Ok(true),
                Written::Saved { metadata: false } => {
                    // ExifTool reads and writes by the disk paths too, so a folder whose name ends in a dot is the
                    // one it reaches.
                    let (source, out) = (disk_path_of(paths.get(job.source.path)), disk_path_of(paths.get(job.out)));
                    if let Err(e) = tag_copy(&mut exiftool, &source, &out) {
                        // The copy was published before it had its tags: a failure leaves no untagged copy at the
                        // output, since the job reports a failure and the output is not what it asked for.
                        let _ = std::fs::remove_file(&out);
                        return Err(e);
                    }
                    Ok(true)
                }
            });
            ctl.add(len, result.is_err());
            match result {
                Ok(true) => stats.saved += 1,
                Ok(false) => stats.skipped += 1,
                Err(e) => {
                    stats.failed += 1;
                    stats.error.get_or_insert_with(|| {
                        file_failure(String::from_utf8_lossy(paths.get(job.source.path)).into_owned(), &e)
                    });
                }
            }
            Ok(true)
        },
    )?;
    drop(phase);
    stats.remaining = jobs.iter().zip(&done).filter(|(_, d)| !**d).map(|(j, _)| j.id).collect();
    Ok(stats)
}

/// A library folder's place in the copies' folder: its drive's letter as a part (none for a UNC or a Unix root),
/// then its parts. Two library folders can't share a place.
fn place_of(root: &RulePath) -> impl Iterator<Item = Name> + '_ {
    let letter: Vec<u8> = root.anchor.as_bytes().iter().copied().filter(|&b| b != b':' && b != b'\\').collect();
    (!letter.is_empty()).then(|| Name::from_raw(&letter)).into_iter().chain(root.parts.iter().cloned())
}

enum Written {
    Saved {
        metadata: bool,
    },
    /// Changed since its borders were found.
    Skipped,
}

/// Write `job`'s copy of the file read as `bytes`, through `temp` (a worker's buffer for the temporary path's bytes).
fn write_copy(job: &Job, paths: &PathArena, bytes: &[u8], temp: &mut Vec<u8>) -> Result<Written> {
    let s = &job.source;
    let (source, out) = (paths.get(s.path), paths.get(job.out));
    // Each by its disk path, which reaches it exactly: a folder whose name ends in a dot or a space is the one read and
    // written, not a sibling without it.
    let (source_path, out_path) = (disk_path_of(source), disk_path_of(out));
    if pg_platform::file::stat(&source_path)
        .with_context(|| format!("reading {}", pg_platform::names::shown(&source_path)))?
        != (s.size, s.mtime)
        || usize::try_from(s.size).ok() != Some(bytes.len())
    {
        return Ok(Written::Skipped);
    }
    let cropped = crop(bytes, s.format, s.width, s.height, s.cut)?;
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", shown(dir)))?;
    }
    // Complete or not there: written beside it, then renamed over whatever an earlier save left.
    temp.clear();
    temp.extend_from_slice(out);
    temp.extend_from_slice(b".pg-crop");
    let temp_path: PathBuf = disk_path_of(temp);
    publish(&temp_path, &out_path, &cropped.bytes, &source_path)?;
    Ok(Written::Saved { metadata: cropped.metadata })
}

/// Give the published copy at `out` the tags of `source` that the crop left out (ExifTool), and its modification time.
fn tag_copy(exiftool: &mut Option<ExifTool>, source: &Path, out: &Path) -> Result<()> {
    ExifTool::running(exiftool, &pg_exiftool::program())?.copy_tags(source, out)?;
    keep_time(source, out)
}

/// Write `bytes` to the temporary `temp`, give it `source`'s modification time, and rename it to `out`, so the copy
/// at `out` is complete or not there. The temporary is removed on any failure from its first write on (a write that
/// failed part way, a time not set, a rename refused): its guard is armed before the write and disarmed only once
/// the rename is done. (Its name is fixed, `<out>.pg-crop`, so what a crash leaves is replaced by the next save.)
fn publish(temp: &Path, out: &Path, bytes: &[u8], source: &Path) -> Result<()> {
    // Each is a disk path, reached exactly; the messages name them as the user does.
    let guard = TempFile::new(temp);
    std::fs::write(temp, bytes).with_context(|| format!("writing {}", shown(out)))?;
    keep_time(source, temp)?;
    std::fs::rename(temp, out).with_context(|| format!("writing {}", shown(out)))?;
    guard.persist();
    Ok(())
}

/// Give the copy at `copy` the original's modification time (both disk paths).
fn keep_time(original: &Path, copy: &Path) -> Result<()> {
    let modified = std::fs::metadata(original)?.modified()?;
    std::fs::File::options().write(true).open(copy)?.set_modified(modified)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder with a source file in it, for `publish`.
    fn folder(name: &str) -> (PathBuf, PathBuf) {
        let dir = pg_testkit::test_folder("scratch_crop", name).unwrap();
        let source = dir.join("source.jpg");
        std::fs::write(&source, b"original").unwrap();
        (dir, source)
    }

    /// Published: the copy is in place with the source's time, and no temporary is left.
    #[test]
    fn publish_renames_the_temp_into_place() {
        let (dir, source) = folder("pg-crop-publish");
        let (temp, out) = (dir.join("out.jpg.pg-crop"), dir.join("out.jpg"));
        publish(&temp, &out, b"cropped", &source).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"cropped");
        assert!(!temp.exists());
        let time = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(time(&out), time(&source));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A rename that fails (the copy's folder is gone) leaves no temporary, and no copy.
    #[test]
    fn a_failed_rename_leaves_no_temp() {
        let (dir, source) = folder("pg-crop-rename");
        let (temp, out) = (dir.join("out.jpg.pg-crop"), dir.join("gone").join("out.jpg"));
        assert!(publish(&temp, &out, b"cropped", &source).is_err());
        assert!(!temp.exists(), "the temporary is removed");
        assert!(!out.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A failure after the write (the source's time can't be read: it is gone) leaves no temporary; nor does a
    /// write that fails.
    #[test]
    fn a_failed_write_or_time_leaves_no_temp() {
        let (dir, source) = folder("pg-crop-write");
        let (temp, out) = (dir.join("out.jpg.pg-crop"), dir.join("out.jpg"));
        assert!(publish(&temp, &out, b"cropped", &dir.join("missing.jpg")).is_err());
        assert!(!temp.exists(), "written, then removed");
        assert!(!out.exists());
        // The temporary's folder is missing: the write fails, and nothing is there.
        let temp = dir.join("gone").join("out.jpg.pg-crop");
        assert!(publish(&temp, &out, b"cropped", &source).is_err());
        assert!(!temp.exists());
        assert!(!out.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
