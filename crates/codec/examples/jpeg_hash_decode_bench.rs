//! Times the JPEG decode that the duplicate hashes take, for the turbojpeg fork's measurement (`vendor/README.md`,
//! `## rust-turbojpeg`; `docs/reference/JPEG_DECODERS.md`). It decodes each JPEG of a folder at the size `analyze` hashes
//! at (`MinSize::Sides(512, 320)`, the thumbnail minimum), through the same `Decoder` the app uses, and prints the mean
//! time per decode over the timed rounds, after one warm-up round.
//!
//! The files are the first `count` JPEGs of the folder in depth-first, name-sorted order, so the set is the same on every
//! run and on every build. The orientation is taken as upright (no EXIF read): the decode's work depends on the size it
//! scales to, not on the rotation.
//!
//! `cargo run --release -p pg-codec --example jpeg_hash_decode_bench -- <folder> [count] [rounds]`

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use pg_codec::decode::{Decoder, MinSize, Request};
use pg_codec::pixels::Rgb;
use pg_model::format::Format;
use pg_model::orientation::Orientation;
use pg_model::turns::Turns;

/// The hashing decode's least size: `analyze`'s `DECODE_MIN_LONG` and `DECODE_MIN_SHORT`.
const MIN_LONG: usize = 512;
const MIN_SHORT: usize = 320;

/// The first `count` JPEGs under `folder`, depth-first: at each level the entries are taken in name order, folders and
/// files together, and a folder's JPEGs come before the next entry's.
fn first_jpegs(folder: &Path, count: usize, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries: Vec<PathBuf> = fs::read_dir(folder)?.map(|e| e.map(|e| e.path())).collect::<io::Result<_>>()?;
    entries.sort();
    for path in entries {
        if out.len() >= count {
            return Ok(());
        }
        if path.is_dir() {
            first_jpegs(&path, count, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let folder = args.first().ok_or("usage: jpeg_hash_decode_bench <folder> [count] [rounds]")?;
    let count: usize = args.get(1).map_or(Ok(300), |c| c.parse())?;
    let rounds: usize = args.get(2).map_or(Ok(5), |r| r.parse())?;

    let mut paths = Vec::with_capacity(count);
    first_jpegs(Path::new(folder), count, &mut paths)?;
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        files.push(fs::read(path)?);
    }
    let bytes: usize = files.iter().map(Vec::len).sum();

    let mut decoder = Decoder::new()?;
    let mut image = Rgb::default();
    let mut spare = Vec::new();
    let mut decode_all = |decoder: &mut Decoder| -> Result<(), Box<dyn Error>> {
        for file in &files {
            let format = Format::sniff(file).ok_or("a file is not a known image")?;
            let min = MinSize::Sides(MIN_LONG, MIN_SHORT);
            let request = Request { format, exif: Orientation::UPRIGHT, min, turns: Turns::NONE };
            decoder.decode_into(file, request, &mut image, &mut spare)?;
        }
        Ok(())
    };

    decode_all(&mut decoder)?; // warm-up: the first round allocates the decoder's buffers
    let started = Instant::now();
    for _ in 0..rounds {
        decode_all(&mut decoder)?;
    }
    let calls = files.len() * rounds;
    let per_call_ms = started.elapsed().as_secs_f64() * 1000.0 / pg_model::num::count(calls);

    let mut out = io::stdout().lock();
    writeln!(out, "files {}  bytes {}  rounds {}  calls {}", files.len(), bytes, rounds, calls)?;
    writeln!(out, "mean ms per decode {per_call_ms:.3}")?;
    Ok(())
}
