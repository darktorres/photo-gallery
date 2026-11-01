//! Borders (pg_core::borders): the pass after the import that looks for them in the files it analyzed, and in
//! every file after a new detector (`analysis.borders_due`), each decoded at full size; reports on any files, in
//! `tools/borders.py`'s columns; and the eval viewer's review marks taken into the catalog.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use pg_core::analyze::{FileBorders, borders_of};
use pg_core::borders::Side;
use pg_core::catalog::Catalog;
use pg_core::decode::Decoder;
use pg_core::media::{self, Kind};

use crate::control::Control;
use crate::pipeline::{self, Options};

const BATCH: usize = 500;

pub struct Stats {
    pub files: usize,
    pub failed: usize,
}

/// Look for the borders that are due, committing in batches (stopping loses at most the batch).
pub fn find(catalog: &mut Catalog, opts: &Options, ctl: &Control) -> Result<Stats> {
    let items = catalog.pending_borders()?;
    let mut stats = Stats { files: 0, failed: 0 };
    if items.is_empty() {
        return Ok(stats);
    }
    let paths: Vec<String> = items.iter().map(|i| i.path.clone()).collect();
    ctl.begin("borders", String::new(), items.len() as u64);
    let mut batch: Vec<(usize, Result<FileBorders>)> = Vec::with_capacity(BATCH);
    let mut flush = |batch: &mut Vec<(usize, Result<FileBorders>)>, stats: &mut Stats| -> Result<()> {
        let tx = catalog.conn.transaction()?;
        for (i, result) in batch.drain(..) {
            // A file that no longer decodes keeps no borders; if it changed, the import records the failure.
            let found = match result {
                Ok(b) => b.borders,
                Err(_) => {
                    stats.failed += 1;
                    None
                }
            };
            Catalog::store_borders(&tx, &items[i], found.as_ref())?;
            stats.files += 1;
        }
        tx.commit()?;
        Ok(())
    };
    pipeline::run(
        &paths,
        |_| true,
        opts,
        ctl,
        Decoder::new,
        |decoder, i, bytes| borders_of(bytes, items[i].rotation, decoder),
        |i, result, len| {
            ctl.add(len, result.is_err());
            batch.push((i, result));
            if batch.len() >= BATCH {
                flush(&mut batch, &mut stats)?;
            }
            Ok(true)
        },
    )?;
    flush(&mut batch, &mut stats)?;
    ctl.finish();
    Ok(stats)
}

pub struct ReportStats {
    pub files: usize,
    pub borders: usize,
    pub rejected: usize,
    pub skipped: usize,
    pub errors: usize,
    pub seconds: f64,
}

/// The image files at `paths` (files, or folders searched recursively), sorted.
fn image_files(paths: &[PathBuf]) -> Result<Vec<String>> {
    fn walk(dir: &Path, out: &mut Vec<String>) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(&path, out)?;
            } else if is_image(&path) {
                out.push(path.to_string_lossy().into_owned());
            }
        }
        Ok(())
    }
    fn is_image(path: &Path) -> bool {
        path.file_name().and_then(|n| n.to_str()).and_then(media::kind_of) == Some(Kind::Image)
    }
    let mut out = Vec::new();
    for p in paths {
        let p = std::path::absolute(p)?;
        if p.is_dir() {
            walk(&p, &mut out)?;
        } else {
            out.push(p.to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// A CSV field, quoted when it has to be.
fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Write `csv` with `tools/borders.py`'s `report.csv` columns for the image files at `paths`, as the borders pass
/// finds their borders, so the two can be compared.
pub fn report(paths: &[PathBuf], csv: &Path, opts: &Options, ctl: &Control) -> Result<ReportStats> {
    let start = Instant::now();
    let files = image_files(paths)?;
    let mut rows: Vec<(usize, String)> = Vec::with_capacity(files.len());
    let mut stats = ReportStats { files: 0, borders: 0, rejected: 0, skipped: 0, errors: 0, seconds: 0.0 };
    ctl.begin("borders", String::new(), files.len() as u64);
    pipeline::run(
        &files,
        |_| true,
        opts,
        ctl,
        Decoder::new,
        |decoder, _, bytes| borders_of(bytes, 0, decoder),
        |i, result, len| {
            ctl.add(len, result.is_err());
            let path = field(&files[i]);
            let row = match result {
                Err(e) => {
                    stats.errors += 1;
                    format!("{path},,,error,,,,,,,,{}", field(&format!("{e:#}")))
                }
                Ok(FileBorders { width, height, borders: None }) => {
                    stats.skipped += 1;
                    format!("{path},{width},{height},skipped,,,,,,,,")
                }
                Ok(FileBorders { width, height, borders: Some(d) }) => {
                    let describe = |accepted: bool| {
                        let sides: Vec<String> =
                            d.sides.iter().filter(|s| s.accepted() == accepted).map(Side::describe).collect();
                        field(&sides.join("; "))
                    };
                    let status = if d.found() {
                        stats.borders += 1;
                        "border"
                    } else if !d.sides.is_empty() {
                        stats.rejected += 1;
                        "rejected"
                    } else {
                        "none"
                    };
                    let [l, t, r, b] = d.cut();
                    let (sides, rejected) = (describe(true), describe(false));
                    format!("{path},{width},{height},{status},{l},{t},{r},{b},{sides},{rejected},,")
                }
            };
            rows.push((i, row));
            stats.files += 1;
            Ok(true)
        },
    )?;
    ctl.finish();
    rows.sort_by_key(|(i, _)| *i);
    let mut out = String::from(
        "path,width,height,status,crop_left,crop_top,crop_right,crop_bottom,sides,rejected,output,error\n",
    );
    for (_, row) in rows {
        out.push_str(&row);
        out.push('\n');
    }
    std::fs::write(csv, out).with_context(|| format!("writing {}", csv.display()))?;
    stats.seconds = start.elapsed().as_secs_f64();
    Ok(stats)
}

/// Take the false positives marked in a `review.json` of the eval viewer's Borders page (`{path: "false_positive"}`)
/// into the catalog as marks. Returns (marked, not in the catalog).
pub fn import_review(catalog: &Catalog, review: &Path) -> Result<(usize, usize)> {
    let text = std::fs::read_to_string(review).with_context(|| format!("reading {}", review.display()))?;
    let verdicts: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", review.display()))?;
    let (mut marked, mut unknown) = (0, 0);
    for (path, verdict) in verdicts {
        if verdict != "false_positive" {
            continue;
        }
        match catalog.file_by_path(&path)? {
            Some(id) => {
                Catalog::set_border_mark(&catalog.conn, id, true)?;
                marked += 1;
            }
            None => unknown += 1,
        }
    }
    Ok((marked, unknown))
}
