//! Times the import's read of JPEGs (`jpeg_import_bytes`, up to the end of the header) against a fixed prefix and the
//! whole file, over a folder's JPEGs with a number of threads:
//! `cargo run --release -p pg-container --example jpeg_read_bench -- <folder> <strategy> [threads] [every]`.
//!
//! Strategies: `whole` (the file), `prefix` (the import's read before `jpeg_import_bytes`: the first `PREFIX_KB`, 256 by
//! default, and the whole file when the header is not complete in them), `gather` (`jpeg_import_bytes`; the whole file
//! when it gives none). Environment: `FLAGS` (a comma list of `random`, `seq`: the open flags, Windows), `LIST` (a file
//! of paths, in its order, instead of the folder's JPEGs by name), `PG_LABEL`. Empty the system's file cache before each
//! run for a cold read.

#![expect(clippy::print_stdout, clippy::print_stderr, reason = "a benchmark prints its results")]

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use pg_container::JpegHeader;

static READ_BYTES: AtomicU64 = AtomicU64::new(0);
static READ_CALLS: AtomicU64 = AtomicU64::new(0);
static WHOLE: AtomicU64 = AtomicU64::new(0);

/// A file whose reads are counted.
struct Counted(File);

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.0.read(buf)?;
        READ_BYTES.fetch_add(u64::try_from(n).unwrap_or(0), Relaxed);
        READ_CALLS.fetch_add(1, Relaxed);
        Ok(n)
    }
}

impl Seek for Counted {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}

#[cfg(windows)]
fn open(path: &Path) -> io::Result<Counted> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut bits = 0;
    for flag in std::env::var("FLAGS").unwrap_or_default().split(',') {
        bits |= match flag {
            "random" => 0x1000_0000,
            "seq" => 0x0800_0000,
            _ => 0,
        };
    }
    Ok(Counted(std::fs::OpenOptions::new().read(true).custom_flags(bits).open(path)?))
}

/// Elsewhere, the plain open (`FLAGS` is Windows').
#[cfg(not(windows))]
fn open(path: &Path) -> io::Result<Counted> {
    Ok(Counted(File::open(path)?))
}

fn whole(file: &mut Counted) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn one(strategy: &str, path: &Path) -> io::Result<()> {
    let mut file = open(path)?;
    match strategy {
        "whole" => {
            whole(&mut file)?;
        }
        "prefix" => {
            let prefix = std::env::var("PREFIX_KB").ok().and_then(|v| v.parse().ok()).unwrap_or(256u64) * 1024;
            let mut bytes = Vec::new();
            (&mut file).take(prefix).read_to_end(&mut bytes)?;
            // The file goes on past the prefix: read on when the header is not complete in it.
            let cut = u64::try_from(bytes.len()).unwrap_or(0) == prefix;
            if cut && !matches!(pg_container::jpeg_header_end(&bytes), JpegHeader::Ends(_)) {
                WHOLE.fetch_add(1, Relaxed);
                file.read_to_end(&mut bytes)?;
            }
        }
        "gather" => {
            if pg_container::jpeg_import_bytes(&mut file)?.is_none() {
                WHOLE.fetch_add(1, Relaxed);
                whole(&mut file)?;
            }
        }
        other => return Err(io::Error::other(format!("no strategy {other}"))),
    }
    Ok(())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| ["jpg", "jpeg", "jfif"].iter().any(|x| e.eq_ignore_ascii_case(x))) {
            out.push(path);
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(root), Some(strategy)) = (args.get(1), args.get(2)) else {
        return Err(io::Error::other("usage: jpeg_read_bench <folder> <strategy> [threads] [every]"));
    };
    let threads: usize = args.get(3).and_then(|t| t.parse().ok()).unwrap_or(8);
    let every: usize = args.get(4).and_then(|t| t.parse().ok()).unwrap_or(1).max(1);
    let mut files = Vec::new();
    if let Ok(list) = std::env::var("LIST") {
        files = std::fs::read_to_string(list)?.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect();
    } else {
        collect(Path::new(root), &mut files)?;
        files.sort();
    }
    let files: Vec<PathBuf> = files.into_iter().step_by(every).collect();
    let next = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let started = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                while let Some(path) = files.get(next.fetch_add(1, Relaxed)) {
                    if let Err(e) = one(strategy, path) {
                        failed.fetch_add(1, Relaxed);
                        eprintln!("{}: {e}", path.display());
                    }
                }
            });
        }
    });
    let secs = started.elapsed().as_secs_f64();
    let mb = |b: u64| f64::from(u32::try_from(b >> 20).unwrap_or(u32::MAX));
    let n = f64::from(u32::try_from(files.len()).unwrap_or(u32::MAX));
    let read = READ_BYTES.load(Relaxed);
    println!(
        "{strategy:6} t{threads:<2} {:24} {:6.0} files/s {secs:6.2} s  read {:6.0} MB ({:4.0} MB/s) in {:7} calls; read whole {}, failed {}",
        std::env::var(pg_testkit::inputs::LABEL).unwrap_or_default(),
        n / secs,
        mb(read),
        mb(read) / secs,
        READ_CALLS.load(Relaxed),
        WHOLE.load(Relaxed),
        failed.load(Relaxed),
    );
    Ok(())
}
