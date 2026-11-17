//! Checks the import's gathered reads against the whole file: for each JPEG and PNG under a folder, the analysis the
//! import makes of the bytes it reads (`pg_container::jpeg_import_bytes`, `png_import_bytes`) is the analysis it makes
//! of the whole file. Run it with `cargo run --release -p pg-import --example gather_check -- <folder> [every] [jpeg|png]`
//! (every nth file of the folder, of one format or both); it exits with an error when any analysis differs.

#![expect(clippy::print_stdout, clippy::print_stderr, reason = "an example prints its results")]

use std::path::{Path, PathBuf};

use pg_analysis::analyze::Analyzer;
use pg_model::analysis::{Analysis, Media};
use pg_model::turns::Turns;

/// The facts an analysis stores, as text (an analysis has no `Debug`): what the catalog would keep of it, or the error.
fn facts(result: &anyhow::Result<Analysis>) -> String {
    match result {
        Err(e) => format!("error: {e:#}"),
        Ok(a) => {
            let media = match &a.media {
                Media::Image(image) => format!("image {:?} rating {:?}", image.format, image.file_rating),
                _ => "not an image".to_string(),
            };
            format!(
                "version {} orientation {:?} {}x{} frames {} taken {:?} {media}",
                a.version, a.orientation, a.width, a.height, a.frames, a.taken
            )
        }
    }
}

fn has_extension(path: &Path, exts: &[&str]) -> bool {
    path.extension().is_some_and(|ext| exts.iter().any(|x| ext.eq_ignore_ascii_case(x)))
}

const JPEG: &[&str] = &["jpg", "jpeg", "jfif"];

fn collect(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, exts, out)?;
        } else if has_extension(&path, exts) {
            out.push(path);
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("the folder to check"))?;
    let every: usize = std::env::args().nth(2).and_then(|n| n.parse().ok()).unwrap_or(1).max(1);
    let exts: Vec<&str> = match std::env::args().nth(3).as_deref() {
        Some("jpeg") => JPEG.to_vec(),
        Some("png") => vec!["png"],
        _ => [JPEG, &["png"]].concat(),
    };
    let mut files = Vec::new();
    collect(Path::new(&root), &exts, &mut files)?;
    files.sort();
    let files: Vec<PathBuf> = files.into_iter().step_by(every).collect();
    let mut analyzer = Analyzer::new()?;
    let (mut gathered, mut whole, mut mismatched, mut failing) = (0usize, 0usize, 0usize, 0usize);
    let started = std::time::Instant::now();
    for path in &files {
        let all = std::fs::read(path)?;
        let from_whole = analyzer.analyze(&all, Turns::NONE);
        let mut file = std::fs::File::open(path)?;
        let read = if has_extension(path, &["png"]) {
            pg_container::png_import_bytes(&mut file)?
        } else {
            pg_container::jpeg_import_bytes(&mut file)?
        };
        let Some(bytes) = read else {
            whole += 1;
            continue;
        };
        gathered += 1;
        let from_gathered = analyzer.analyze(&bytes, Turns::NONE);
        if from_whole.is_err() {
            failing += 1;
        }
        let (a, b) = (facts(&from_whole), facts(&from_gathered));
        if a != b {
            mismatched += 1;
            if mismatched <= 5 {
                eprintln!("MISMATCH {}\n  whole:    {a}\n  gathered: {b}", path.display());
            }
        }
    }
    println!(
        "{} files in {:.1} s: {gathered} gathered, {whole} read whole, {mismatched} with a different analysis, {failing} that fail whole",
        files.len(),
        started.elapsed().as_secs_f64()
    );
    anyhow::ensure!(mismatched == 0, "{mismatched} files differ between the whole file and the gathered bytes");
    Ok(())
}
