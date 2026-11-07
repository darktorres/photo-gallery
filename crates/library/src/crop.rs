//! Saving cropped copies (pg_edit::crop): each file's borders as the catalog has them (found, not marked as not a
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
use pg_jobs::control::{Control, Phase};
use pg_jobs::pipeline::{self, Options};
use pg_platform::names::{path_buf, set_path};
use pg_writeback::{exiftool_program, stat};

pub struct Stats {
    pub saved: usize,
    /// Nothing to crop now (no borders, marked, changed since), or not in the library any more.
    pub skipped: usize,
    pub failed: usize,
    /// The first failure, with its file.
    pub error: Option<String>,
    /// What a stop left (to queue again).
    pub remaining: Vec<FileId>,
}

struct Job {
    id: FileId,
    source: CropSource,
    /// Where its copy goes. Both paths are in the jobs' arena.
    out: PathRef,
}

/// Save cropped copies of `ids` under `folder`.
pub fn save(catalog: &Catalog, ids: &[FileId], folder: &Path, opts: &Options, ctl: &Control) -> Result<Stats> {
    let rules = catalog.rules();
    // The folder is chosen by the user, and it is taken exactly: a lossy text form would write the copies into a
    // different folder from the one named.
    let folder = pg_platform::names::rule_path_of(folder).context("the folder for the copies isn't a path")?;
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

    ctl.begin(Phase::Crop, u64::try_from(jobs.len()).unwrap_or(u64::MAX));
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
                    let (source, out) = (path_buf(paths.get(job.source.path)), path_buf(paths.get(job.out)));
                    ExifTool::running(&mut exiftool, &exiftool_program())?.copy_tags(&source, &out)?;
                    keep_time(&source, &out)?;
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
                        format!("{}: {e:#}", String::from_utf8_lossy(paths.get(job.source.path)))
                    });
                }
            }
            Ok(true)
        },
    )?;
    ctl.finish();
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
    let (source_path, out_path) = (path_buf(source), path_buf(out));
    if stat(&source_path)? != (s.size, s.mtime) || usize::try_from(s.size).ok() != Some(bytes.len()) {
        return Ok(Written::Skipped);
    }
    let cropped = crop(bytes, s.format, s.width, s.height, s.cut)?;
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    // Complete or not there: written beside it, then renamed over whatever an earlier save left.
    temp.clear();
    temp.extend_from_slice(out);
    temp.extend_from_slice(b".pg-crop");
    let temp_path: PathBuf = path_buf(temp);
    std::fs::write(&temp_path, &cropped.bytes).with_context(|| format!("writing {}", out_path.display()))?;
    keep_time(&source_path, &temp_path)?;
    std::fs::rename(&temp_path, &out_path).with_context(|| format!("writing {}", out_path.display()))?;
    Ok(Written::Saved { metadata: cropped.metadata })
}

/// Give the copy at `copy` the original's modification time.
fn keep_time(original: &Path, copy: &Path) -> Result<()> {
    let modified = std::fs::metadata(original)?.modified()?;
    std::fs::File::options().write(true).open(copy)?.set_modified(modified)?;
    Ok(())
}
