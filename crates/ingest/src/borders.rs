//! Borders (pg_analysis::borders): the pass after the import that looks for them in the files it analyzed, and in
//! every file after a new detector (`analysis.borders_due`), each decoded at full size; reports on any files, in
//! the Python prototype's columns (`tools/borders.py`, in git history); the detector for images piped in (the eval's
//! `--trim`); and the review marks of the eval viewer's former Borders page taken into the catalog.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use pg_analysis::analyze::{Analyzer, FileBorders};
use pg_analysis::borders::{self as detector, Detector};
use pg_codec::pixels::Rgb;
use pg_core::catalog::{BordersWriter, Catalog};
use pg_model::detection::{self, Side};
use pg_model::media::{Ext, Kind};
use pg_platform::verbatim::verbatim_wide;

use crate::control::Control;
use crate::pipeline::{self, Options};

const BATCH: usize = 500;

pub struct Stats {
    pub files: usize,
    pub failed: usize,
}

/// Look for the borders that are due, committing in batches (stopping loses at most the batch).
pub fn find(catalog: &mut Catalog, opts: &Options, ctl: &Control) -> Result<Stats> {
    let work = catalog.pending_borders()?;
    let items = &work.items;
    let mut stats = Stats { files: 0, failed: 0 };
    if items.is_empty() {
        return Ok(stats);
    }
    ctl.begin("borders", String::new(), items.len() as u64);
    let mut batch: Vec<(usize, Result<FileBorders>)> = Vec::with_capacity(BATCH);
    let mut flush = |batch: &mut Vec<(usize, Result<FileBorders>)>, stats: &mut Stats| -> Result<()> {
        let tx = catalog.conn.transaction()?;
        let mut writer = BordersWriter::new(&tx)?;
        for (i, result) in batch.drain(..) {
            stats.files += 1;
            let found = match result {
                Ok(b) => b.borders,
                // Couldn't be read (a locked file, an offline drive): nothing is stored, so the file stays due and the
                // next pass tries it again. Storing "no borders" here would hide it until BORDERS_VERSION changed (R-47).
                Err(e) if e.downcast_ref::<std::io::Error>().is_some() => {
                    stats.failed += 1;
                    continue;
                }
                // It doesn't decode: no borders, for the file as it is; the import records the failure.
                Err(_) => {
                    stats.failed += 1;
                    None
                }
            };
            writer.store_borders(&items[i], found.as_ref())?;
        }
        drop(writer);
        tx.commit()?;
        Ok(())
    };
    pipeline::run(
        items,
        |item, wide| {
            let [folder, name] = work.parts(item.path);
            pg_platform::verbatim::verbatim_wide_file(folder, name, wide);
            true
        },
        opts,
        ctl,
        Analyzer::new,
        |analyzer, i, bytes| analyzer.borders(bytes, items[i].rotation),
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
        path.file_name().and_then(|n| n.to_str()).and_then(Ext::of_name).map(Ext::kind) == Some(Kind::Image)
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

/// Write `csv` with the prototype's `report.csv` columns for the image files at `paths`, as the borders pass
/// finds their borders, so the two can be compared.
pub fn report(paths: &[PathBuf], csv: &Path, opts: &Options, ctl: &Control) -> Result<ReportStats> {
    let start = Instant::now();
    let files = image_files(paths)?;
    let mut rows: Vec<(usize, String)> = Vec::with_capacity(files.len());
    let mut stats = ReportStats { files: 0, borders: 0, rejected: 0, skipped: 0, errors: 0, seconds: 0.0 };
    ctl.begin("borders", String::new(), files.len() as u64);
    pipeline::run(
        &files,
        |file, wide| {
            verbatim_wide(&[file], wide);
            true
        },
        opts,
        ctl,
        Analyzer::new,
        |analyzer, _, bytes| analyzer.borders(bytes, 0),
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

/// The most pixels one image may have: `image`'s default allocation limit (512 MiB, which the app's decoders keep to)
/// at three bytes a pixel. A header above it is refused before any pixel is read, so the pipe can't ask for a 3 GB
/// buffer.
const MAX_PIXELS: usize = (512 << 20) / 3;

/// Look for borders in images read from `input` until it ends: each one's width and height (u32, little-endian),
/// then its pixels (RGB, rows top to bottom). For each, a line on `output`: what is left without its borders, as
/// `left top right bottom` (right and bottom exclusive; the whole image when it has none, or is too small to look
/// at). Returns how many it read.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> Result<usize> {
    let mut n = 0;
    let mut finder = Detector::default();
    let mut img = Rgb { width: 0, height: 0, pixels: Vec::new() };
    loop {
        if input.fill_buf()?.is_empty() {
            return Ok(n);
        }
        let mut size = [0u8; 8];
        input.read_exact(&mut size).context("reading an image's size")?;
        let [w, h] = [&size[..4], &size[4..]].map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
        ensure!(w.checked_mul(h).is_some_and(|p| p <= MAX_PIXELS), "image {n}: {w} × {h} is too large");
        (img.width, img.height) = (w, h);
        img.pixels.resize(w * h * 3, 0);
        input.read_exact(&mut img.pixels).with_context(|| format!("reading image {n}'s pixels"))?;
        let [l, t, r, b] = if w.min(h) >= detection::MIN_SIDE {
            finder.detect(&img, &detector::Params::default()).content
        } else {
            [0, 0, w as u32, h as u32]
        };
        writeln!(output, "{l} {t} {r} {b}")?;
        output.flush()?;
        n += 1;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Photo-like content (blocks of hashed colours with fine noise) between black bars `bar` deep.
    fn letterboxed(w: u32, h: u32, bar: u32) -> Vec<u8> {
        let hash =
            |a: u32, b: u32| (a.wrapping_mul(2_654_435_761) ^ b.wrapping_mul(40_503)).wrapping_mul(2_246_822_519);
        let mut pixels = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                if !(bar..h - bar).contains(&y) {
                    pixels.extend([0, 0, 0]);
                    continue;
                }
                let (block, noise) = (hash(x / 6, y / 6), hash(x, y) % 24);
                pixels.extend([0, 8, 16].map(|s| (60 + (block >> s) % 150 + noise) as u8));
            }
        }
        pixels
    }

    fn frame(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
        [&w.to_le_bytes()[..], &h.to_le_bytes(), pixels].concat()
    }

    #[test]
    fn piped_images_get_their_content_boxes() {
        let input = [frame(320, 240, &letterboxed(320, 240, 40)), frame(32, 32, &[7; 32 * 32 * 3])].concat();
        let mut output = Vec::new();
        assert_eq!(serve(&input[..], &mut output).unwrap(), 2);
        let lines: Vec<Vec<u32>> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| l.split(' ').map(|v| v.parse().unwrap()).collect())
            .collect();
        // The bars and the detector's pixel of margin past the seam; the tiny one isn't looked at.
        let [l, t, r, b] = lines[0][..] else { panic!("{lines:?}") };
        assert!(l == 0 && r == 320 && (40..43).contains(&t) && (197..=200).contains(&b), "{lines:?}");
        assert_eq!(lines[1], [0, 0, 32, 32]);
        // A stream cut short is an error, not a silent end.
        assert!(serve(&input[..20], &mut Vec::new()).is_err());
    }

    #[test]
    fn a_header_above_the_pixel_limit_is_refused_before_its_pixels() {
        // 20000 × 20000 is 400 M pixels, over MAX_PIXELS; no pixel bytes follow, so the refusal is the header's.
        let mut input = Vec::new();
        input.extend_from_slice(&20_000u32.to_le_bytes());
        input.extend_from_slice(&20_000u32.to_le_bytes());
        let err = serve(&input[..], &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
    }
}
