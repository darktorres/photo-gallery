//! Per-stage timing of the import analysis on one thread: `cargo run --release --example stage_times -- <folder> [n]`.

use std::time::{Duration, Instant};

use pg_core::decode::{Decoder, MinSize};
use pg_core::{analyze, pdq, resize};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(2).map_or(300, |s| s.parse().unwrap());
    let mut files: Vec<_> =
        std::fs::read_dir(&args[1])?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect();
    files.truncate(n);
    let bytes: Vec<Vec<u8>> = files.iter().map(std::fs::read).collect::<Result<_, _>>()?;
    let mut dec = Decoder::new()?;
    let mut t = [Duration::ZERO; 7];
    for b in &bytes {
        let s = Instant::now();
        let _ = analyze::file_hash(b);
        t[0] += s.elapsed();
        let s = Instant::now();
        let _ = pg_core::decode::exif_orientation(b);
        t[1] += s.elapsed();
        let s = Instant::now();
        let d = dec.decode(b, MinSize::Sides(512, 320))?;
        t[2] += s.elapsed();
        let s = Instant::now();
        let _ = analyze::pixel_hash(&d.image);
        t[3] += s.elapsed();
        let s = Instant::now();
        let p = resize::fit_within(&d.image, 512);
        t[4] += s.elapsed();
        let img = p.as_ref().unwrap_or(&d.image);
        let s = Instant::now();
        let _ = pdq::hash_rgb_dihedral(&img.pixels, img.width, img.height);
        t[5] += s.elapsed();
        let s = Instant::now();
        let _ = analyze::analyze(b, &mut dec)?;
        t[6] += s.elapsed();
    }
    for (name, d) in ["blake3", "exif", "decode", "pixel hash", "resize 512", "pdq", "analyze total"].iter().zip(t) {
        println!("{name:>14}: {:.2} ms", d.as_secs_f64() * 1e3 / bytes.len() as f64);
    }
    Ok(())
}
