//! Per-stage timing of the import analysis on one thread: `cargo run --release --example stage_times -- <folder> [n]`.

use std::time::{Duration, Instant};

use pg_analysis::analyze::{self, Analyzer};
use pg_analysis::pdq;
use pg_codec::decode::{Decoder, MinSize};
use pg_codec::pixels::Rgb;
use pg_codec::resize::Resizer;
use pg_metadata::{Meta, MetaScratch};
use pg_model::format::Format;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(2).map_or(300, |s| s.parse().unwrap());
    let mut files: Vec<_> =
        std::fs::read_dir(&args[1])?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect();
    files.truncate(n);
    let bytes: Vec<Vec<u8>> = files.iter().map(std::fs::read).collect::<Result<_, _>>()?;
    // Each stage with its own kept buffers, as a worker's: the metadata read, the decode, the fitted copy, the hasher,
    // and the whole analysis's.
    let mut dec = Decoder::new()?;
    let (mut image, mut spare, mut fitted) = (Rgb::default(), Vec::new(), Rgb::default());
    let mut resizer = Resizer::new();
    let mut hasher = pdq::Hasher::default();
    let mut analyzer = Analyzer::new()?;
    let mut meta_scratch = MetaScratch::default();
    let mut t = [Duration::ZERO; 7];
    for b in &bytes {
        let s = Instant::now();
        let _ = analyze::file_hash(b);
        t[0] += s.elapsed();
        let format = Format::sniff(b).expect("an image");
        let s = Instant::now();
        let meta = Meta::read(b, format, &mut meta_scratch);
        t[1] += s.elapsed();
        let s = Instant::now();
        dec.decode_into(b, format, meta.orientation, MinSize::Sides(512, 320), 0, &mut image, &mut spare)?;
        t[2] += s.elapsed();
        let s = Instant::now();
        let _ = analyze::pixel_hash(&image);
        t[3] += s.elapsed();
        let s = Instant::now();
        let img = if resizer.fit_within(&image, 512, &mut fitted) { &fitted } else { &image };
        t[4] += s.elapsed();
        let s = Instant::now();
        let _ = hasher.hash_rgb_dihedral(&img.pixels, img.width, img.height);
        t[5] += s.elapsed();
        let s = Instant::now();
        let _ = analyzer.analyze(b, 0)?;
        t[6] += s.elapsed();
    }
    for (name, d) in ["blake3", "metadata", "decode", "pixel hash", "resize 512", "pdq", "analyze total"].iter().zip(t)
    {
        println!("{name:>14}: {:.2} ms", d.as_secs_f64() * 1e3 / bytes.len() as f64);
    }
    Ok(())
}
