//! Saving cropped copies (pg_core::crop): each file's borders as the catalog has them (found, not marked as not a
//! border), cut off into a copy at `<folder>\<library folder name>\<path inside it>`, with the original's
//! modification time. The originals are never written: a folder whose copies would land in the library is refused
//! up front, and each file is checked to still be the one its borders were found in.

use std::path::Path;

use anyhow::{Context, Result, bail};
use pg_core::catalog::{Catalog, CropSource, is_under};
use pg_core::crop::crop;
use pg_core::decode::Format;

use crate::control::Control;
use crate::pipeline::{self, Options, long_path};
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
    out: String,
}

/// Save cropped copies of `ids` under `folder`.
pub fn save(catalog: &Catalog, ids: &[i64], folder: &Path, opts: &Options, ctl: &Control) -> Result<Stats> {
    let folder = folder.to_string_lossy().trim_end_matches('\\').to_string();
    let roots = catalog.roots()?;
    let mut stats = Stats { saved: 0, skipped: 0, failed: 0, error: None, remaining: Vec::new() };
    let mut jobs = Vec::with_capacity(ids.len());
    for &id in ids {
        let source = catalog.crop_source(id)?;
        // The deepest library folder holding it (library folders don't nest; a removed one can be inside another).
        let root = source.as_ref().and_then(|s| roots.iter().filter(|r| is_under(&s.path, r)).max_by_key(|r| r.len()));
        let (Some(source), Some(root)) = (source, root) else {
            stats.skipped += 1;
            continue;
        };
        let tree = format!("{folder}\\{}", root_name(root));
        if let Some(r) = roots.iter().find(|r| is_under(&tree, r) || is_under(r, &tree)) {
            bail!("The copies of {root} would go into {tree}, in the library folder {r}: choose a folder outside it");
        }
        let out = format!("{tree}{}", &source.path[root.len()..]);
        jobs.push(Job { id, source, out });
    }
    if jobs.is_empty() {
        return Ok(stats);
    }

    ctl.begin("crop", folder.clone(), jobs.len() as u64);
    let paths: Vec<String> = jobs.iter().map(|j| j.source.path.clone()).collect();
    let mut done = vec![false; jobs.len()];
    let mut exiftool: Option<ExifTool> = None;
    pipeline::run(
        &paths,
        |_| true,
        opts,
        ctl,
        || Ok(()),
        |_, i, bytes| write_copy(&jobs[i], bytes),
        |i, result, len| {
            done[i] = true;
            let job = &jobs[i];
            // TIFF and compressed BMP come out without their metadata.
            let result = result.and_then(|written| match written {
                Written::Skipped => Ok(false),
                Written::Saved { metadata: true } => Ok(true),
                Written::Saved { metadata: false } => {
                    if exiftool.is_none() {
                        exiftool = Some(ExifTool::start()?);
                    }
                    exiftool.as_mut().unwrap().copy_tags(&job.source.path, &job.out)?;
                    keep_time(&job.source.path, &long_path(&job.out))?;
                    Ok(true)
                }
            });
            ctl.add(len, result.is_err());
            match result {
                Ok(true) => stats.saved += 1,
                Ok(false) => stats.skipped += 1,
                Err(e) => {
                    stats.failed += 1;
                    stats.error.get_or_insert_with(|| format!("{}: {e:#}", job.source.path));
                }
            }
            Ok(true)
        },
    )?;
    ctl.finish();
    stats.remaining = jobs.iter().zip(&done).filter(|(_, d)| !**d).map(|(j, _)| j.id).collect();
    Ok(stats)
}

/// A library folder's name in the copies' folder (a drive's letter for a drive).
fn root_name(root: &str) -> &str {
    let name = root.rsplit('\\').next().unwrap_or(root);
    name.trim_end_matches(':')
}

enum Written {
    Saved {
        metadata: bool,
    },
    /// Changed since its borders were found.
    Skipped,
}

fn write_copy(job: &Job, bytes: &[u8]) -> Result<Written> {
    let s = &job.source;
    if stat(&s.path)? != (s.size, s.mtime) || bytes.len() as i64 != s.size {
        return Ok(Written::Skipped);
    }
    let format = Format::from_name(&s.format).with_context(|| format!("unknown format {}", s.format))?;
    let cropped = crop(bytes, format, s.width, s.height, s.cut)?;
    let out = long_path(&job.out);
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    // Complete or not there: written beside it, then renamed over whatever an earlier save left.
    let temp = format!("{out}.pg-crop");
    std::fs::write(&temp, &cropped.bytes).with_context(|| format!("writing {}", job.out))?;
    keep_time(&s.path, &temp)?;
    std::fs::rename(&temp, &out).with_context(|| format!("writing {}", job.out))?;
    Ok(Written::Saved { metadata: cropped.metadata })
}

/// Give the copy at `copy` (long-path form) the original's modification time.
fn keep_time(original: &str, copy: &str) -> Result<()> {
    let modified = std::fs::metadata(long_path(original))?.modified()?;
    std::fs::File::options().write(true).open(copy)?.set_modified(modified)?;
    Ok(())
}
