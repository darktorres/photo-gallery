//! Checks the PNG gather against the whole file: for each PNG under a folder, the analysis the import makes of the gathered
//! bytes is the analysis it makes of the whole file. Run it with `cargo run --release -p pg-import --example png_gather_check
//! -- <folder>`; it exits with an error when any analysis differs.

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

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("png")) {
            out.push(path);
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("the folder to check"))?;
    let mut files = Vec::new();
    collect(Path::new(&root), &mut files)?;
    let mut analyzer = Analyzer::new()?;
    let (mut gathered, mut whole, mut mismatched, mut failing) = (0usize, 0usize, 0usize, 0usize);
    let started = std::time::Instant::now();
    for path in &files {
        let all = std::fs::read(path)?;
        let from_whole = analyzer.analyze(&all, Turns::NONE);
        let Some(bytes) = pg_container::png_import_bytes(&mut std::fs::File::open(path)?)? else {
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
        "{} PNG files in {:.1} s: {gathered} gathered, {whole} read whole, {mismatched} with a different analysis, {failing} that fail whole",
        files.len(),
        started.elapsed().as_secs_f64()
    );
    anyhow::ensure!(mismatched == 0, "{mismatched} files differ between the whole file and the gathered bytes");
    Ok(())
}
