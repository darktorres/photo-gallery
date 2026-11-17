//! Times the import's read of PNGs (`png_import_bytes`, the file's two ends) against reading them whole, over a folder's
//! PNGs with a number of threads, and checks it against the whole file:
//! `cargo run --release -p pg-container --example png_read_bench -- <folder> <strategy> [threads] [every]`.
//!
//! Strategies: `whole` (each file read whole), `gather` (`png_import_bytes`; the file read whole when it gives none),
//! `check` (`png_import_bytes` against `png_import_bytes_of` the whole file, for every file), and `order` (the files by
//! their NTFS file index, one per line: the import's order, for `LIST`).
//!
//! Environment: `FLAGS` (a comma list of `random`, `seq`: the file's open flags, Windows), `LIST` (a file of paths to
//! read, in its order, instead of the folder's PNGs by name), `PG_LABEL` (a label for the result line). Empty the
//! system's file cache before each run for a cold read.

#![expect(clippy::print_stdout, clippy::print_stderr, reason = "a benchmark prints its results")]

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

static READ_BYTES: AtomicU64 = AtomicU64::new(0);
static READ_CALLS: AtomicU64 = AtomicU64::new(0);
static GATHERED: AtomicU64 = AtomicU64::new(0);
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
    const FILE_FLAG_RANDOM_ACCESS: u32 = 0x1000_0000;
    const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
    let flags = std::env::var("FLAGS").unwrap_or_default();
    let mut bits = 0;
    for flag in flags.split(',') {
        bits |= match flag {
            "random" => FILE_FLAG_RANDOM_ACCESS,
            "seq" => FILE_FLAG_SEQUENTIAL_SCAN,
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

/// One file by `strategy`.
fn one(strategy: &str, path: &Path) -> io::Result<()> {
    let mut file = open(path)?;
    match strategy {
        "whole" => {
            whole(&mut file)?;
        }
        "gather" => {
            if pg_container::png_import_bytes(&mut file)?.is_some() {
                GATHERED.fetch_add(1, Relaxed);
            } else {
                whole(&mut file)?;
                WHOLE.fetch_add(1, Relaxed);
            }
        }
        "check" => {
            let gathered = pg_container::png_import_bytes(&mut file)?;
            let of_whole = pg_container::png_import_bytes_of(&whole(&mut file)?);
            match gathered {
                None => {
                    WHOLE.fetch_add(1, Relaxed);
                }
                Some(bytes) if Some(&bytes) == of_whole.as_ref() => {
                    GATHERED.fetch_add(1, Relaxed);
                }
                Some(_) => return Err(io::Error::other("DIFFER: the gather is not the whole file's")),
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
        } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("png")) {
            out.push(path);
        }
    }
    Ok(())
}

/// The file's inode (what the import orders its reads by).
#[cfg(unix)]
fn file_index(path: &Path) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::metadata(path)?.ino())
}

/// The file's NTFS file index (what the import orders its reads by).
#[cfg(windows)]
fn file_index(path: &Path) -> io::Result<u64> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Info {
        attributes: u32,
        times: [u32; 6],
        serial: u32,
        size: [u32; 2],
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Info) -> i32;
    }
    let file = File::open(path)?;
    let mut info =
        Info { attributes: 0, times: [0; 6], serial: 0, size: [0; 2], links: 0, index_high: 0, index_low: 0 };
    // SAFETY: the handle is open for the call, and `info` is a BY_HANDLE_FILE_INFORMATION the call fills.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(u64::from(info.index_high) << 32 | u64::from(info.index_low))
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(root), Some(strategy)) = (args.get(1), args.get(2)) else {
        return Err(io::Error::other("usage: png_read_bench <folder> <strategy> [threads] [every]"));
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
    if strategy == "order" {
        let mut indexed = files.iter().map(|p| Ok((file_index(p)?, p))).collect::<io::Result<Vec<_>>>()?;
        indexed.sort();
        for (_, p) in indexed {
            println!("{}", p.display());
        }
        return Ok(());
    }
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
        "{strategy:6} t{threads:<2} {:16} {:6.0} files/s {secs:6.2} s  read {:6.0} MB ({:4.0} MB/s) in {:7} calls; gathered {}, read whole {}, failed {}",
        std::env::var("PG_LABEL").unwrap_or_default(),
        n / secs,
        mb(read),
        mb(read) / secs,
        READ_CALLS.load(Relaxed),
        GATHERED.load(Relaxed),
        WHOLE.load(Relaxed),
        failed.load(Relaxed),
    );
    Ok(())
}
