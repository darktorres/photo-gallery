//! Saving cropped copies (pg_core::crop): each file's borders as the catalog has them (found, not marked as not a
//! border), cut off into a copy at `<folder>\<library folder name>\<path inside it>`, with the original's
//! modification time. The originals are never written: a folder whose copies would land in the library is refused
//! up front, and each file is checked to still be the one its borders were found in.

use std::path::Path;

use anyhow::{Context, Result, bail};
use pg_core::catalog::{Catalog, CropSource};
use pg_core::crop::crop;
use pg_core::path::is_under;
use pg_core::path::{PathArena, PathRef};
use pg_core::paths::verbatim_wide;

use crate::control::Control;
use crate::pipeline::{self, Options};
use crate::writeback::{ExifTool, stat};

pub struct Stats {
    pub saved: usize,
    /// Nothing to crop now (no borders, marked, changed since), or not in the library any more.
    pub skipped: usize,
    pub failed: usize,
    /// The first failure, with its file.
    pub error: Option<String>,
    /// What a stop left (to queue again).
    pub remaining: Vec<i64>,
}

struct Job {
    id: i64,
    source: CropSource,
    /// Where its copy goes. Both paths are in the jobs' arena.
    out: PathRef,
}

/// Save cropped copies of `ids` under `folder`.
pub fn save(catalog: &Catalog, ids: &[i64], folder: &Path, opts: &Options, ctl: &Control) -> Result<Stats> {
    let folder = folder.to_string_lossy().trim_end_matches('\\').to_string();
    let roots = catalog.roots()?;
    let mut stats = Stats { saved: 0, skipped: 0, failed: 0, error: None, remaining: Vec::new() };
    let mut jobs = Vec::with_capacity(ids.len());
    let mut paths = PathArena::default();
    // Each job's copies' folder and its copy's path, made in these and then kept in the arena.
    let (mut tree, mut out) = (String::new(), String::new());
    for &id in ids {
        let source = catalog.crop_source(id, &mut paths)?;
        // The deepest library folder holding it (library folders don't nest; a removed one can be inside another).
        let root = source.as_ref().and_then(|s| {
            let path = paths.get(s.path);
            roots.iter().filter(|r| is_under(path, r)).max_by_key(|r| r.len())
        });
        let (Some(source), Some(root)) = (source, root) else {
            stats.skipped += 1;
            continue;
        };
        tree.clear();
        tree.extend([&*folder, "\\", &root_place(root)]);
        if let Some(r) = roots.iter().find(|r| is_under(&tree, r) || is_under(r, &tree)) {
            bail!("The copies of {root} would go into {tree}, in the library folder {r}: choose a folder outside it");
        }
        out.clear();
        out.extend([&*tree, &paths.get(source.path)[root.len()..]]);
        let out = paths.push(&out);
        jobs.push(Job { id, source, out });
    }
    if jobs.is_empty() {
        return Ok(stats);
    }

    ctl.begin("crop", folder.clone(), jobs.len() as u64);
    let mut done = vec![false; jobs.len()];
    let mut exiftool: Option<ExifTool> = None;
    pipeline::run(
        &jobs,
        |job, wide| {
            verbatim_wide(&[paths.get(job.source.path)], wide);
            true
        },
        opts,
        ctl,
        || Ok(String::new()),
        |temp, i, bytes| write_copy(&jobs[i], &paths, bytes, temp),
        |i, result, len| {
            done[i] = true;
            let job = &jobs[i];
            // TIFF and compressed BMP come out without their metadata.
            let result = result.and_then(|written| match written {
                Written::Skipped => Ok(false),
                Written::Saved { metadata: true } => Ok(true),
                Written::Saved { metadata: false } => {
                    let (source, out) = (paths.get(job.source.path), paths.get(job.out));
                    ExifTool::running(&mut exiftool)?.copy_tags(source, out)?;
                    keep_time(source, out)?;
                    Ok(true)
                }
            });
            ctl.add(len, result.is_err());
            match result {
                Ok(true) => stats.saved += 1,
                Ok(false) => stats.skipped += 1,
                Err(e) => {
                    stats.failed += 1;
                    stats.error.get_or_insert_with(|| format!("{}: {e:#}", paths.get(job.source.path)));
                }
            }
            Ok(true)
        },
    )?;
    ctl.finish();
    stats.remaining = jobs.iter().zip(&done).filter(|(_, d)| !**d).map(|(j, _)| j.id).collect();
    Ok(stats)
}

/// A library folder's place in the copies' folder: its whole path, without the colon (`D:\Photos\2019` is
/// `D\Photos\2019`). Two library folders can't share a place, whatever their last names (R-48).
fn root_place(root: &str) -> String {
    root.trim_start_matches('\\').replace(':', "")
}

enum Written {
    Saved {
        metadata: bool,
    },
    /// Changed since its borders were found.
    Skipped,
}

/// Write `job`'s copy of the file read as `bytes`, through `temp` (a worker's buffer for the temporary path).
fn write_copy(job: &Job, paths: &PathArena, bytes: &[u8], temp: &mut String) -> Result<Written> {
    let s = &job.source;
    let (source, out) = (paths.get(s.path), paths.get(job.out));
    if stat(source)? != (s.size, s.mtime) || bytes.len() as i64 != s.size {
        return Ok(Written::Skipped);
    }
    let cropped = crop(bytes, s.format, s.width, s.height, s.cut)?;
    if let Some(dir) = Path::new(out).parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    // Complete or not there: written beside it, then renamed over whatever an earlier save left.
    temp.clear();
    temp.extend([out, ".pg-crop"]);
    std::fs::write(&*temp, &cropped.bytes).with_context(|| format!("writing {out}"))?;
    keep_time(source, temp)?;
    std::fs::rename(&*temp, out).with_context(|| format!("writing {out}"))?;
    Ok(Written::Saved { metadata: cropped.metadata })
}

/// Give the copy at `copy` the original's modification time.
fn keep_time(original: &str, copy: &str) -> Result<()> {
    let modified = std::fs::metadata(original)?.modified()?;
    std::fs::File::options().write(true).open(copy)?.set_modified(modified)?;
    Ok(())
}
