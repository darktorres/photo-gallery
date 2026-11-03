//! The borders tools of the command line: a report on the borders the detector finds in files outside the library
//! (in the Python prototype's `report.csv` columns, so the two can be compared), the detector for images piped in
//! (`--stdin`, for the eval), and the review marks of the eval viewer's former Borders page taken into the catalog
//! (`--import-review`). The pass that looks for the borders due in the library is `pg_library::borders`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use pg_analysis::analyze::{Analyzer, FileBorders};
use pg_analysis::borders::{self as detector, Detector};
use pg_catalog::Catalog;
use pg_codec::pixels::Rgb;
use pg_jobs::control::{Control, Phase};
use pg_jobs::pipeline::{self, Options};
use pg_model::detection::{self, Reason, Side};
use pg_model::media::{Ext, Kind};
use pg_model::turns::Turns;
use pg_platform::verbatim::verbatim_wide;

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
    ctl.begin(Phase::Borders, files.len() as u64);
    pipeline::run(
        &files,
        |file, wide| {
            verbatim_wide(&[file], wide);
            true
        },
        opts,
        ctl,
        Analyzer::new,
        |analyzer, _, bytes| analyzer.borders(bytes, Turns::NONE),
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
                            d.sides.iter().filter(|s| s.accepted() == accepted).map(describe_side).collect();
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
                catalog.set_border_mark(id, true)?;
                marked += 1;
            }
            None => unknown += 1,
        }
    }
    Ok((marked, unknown))
}

/// The prototype's `Side.describe()`, for reports compared with its own.
fn describe_side(side: &Side) -> String {
    let kind = match (side.kind, side.color) {
        (detection::Kind::Solid, Some([r, g, b])) => format!(
            "solid #{r:02x}{g:02x}{b:02x} dev {:.0} drift {:.0}",
            side.dev.unwrap_or(0.0),
            side.drift.unwrap_or(0.0)
        ),
        _ => "blur".to_string(),
    };
    let verdict = side.rejected.map(|r| format!(" [{}]", reason_text(r))).unwrap_or_default();
    format!("{}: {kind} {}px straight {:.2}{verdict}", side.edge.key(), side.depth, side.straight)
}

/// The prototype's words for a rejection.
fn reason_text(reason: Reason) -> &'static str {
    match reason {
        Reason::TooDeep => "too deep",
        Reason::NoSeam => "no seam",
        Reason::SeamNotStraight => "seam not straight",
        Reason::UnpairedBlur => "blurred bar without an equal opposite bar",
        Reason::NotFlat => "not flat (shading: wall or sky?)",
        Reason::TooLittleLeft => "too little content left",
    }
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
