//! The platform lister's cost alone, as the scan lists a library: `list_probe <library folder> [repeats] [first pass
//! threads]` opens and
//! lists each folder under the library (`dir::Lister`, `FileIdBothDirectoryInfo` on Windows), first on one thread as the
//! walk does, then split across 2, 4 and 8 threads, each with a lister of its own. Each is timed `repeats` times (7 by
//! default); the median is printed, per run and per file. The first pass, which reads every folder once, is timed on
//! its own: run after emptying the file cache, it is the cold listing; the medians after it are warm.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pg_platform::dir::{Lister, folders_in};
use pg_platform::native::Unit;

fn main() -> io::Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or_else(|| io::Error::other("usage: list_probe <library folder> [repeats] [first pass threads]"))?);
    let repeats: usize = std::env::args().nth(2).and_then(|r| r.parse().ok()).unwrap_or(7);
    let first_threads: usize = std::env::args().nth(3).and_then(|t| t.parse().ok()).unwrap_or(1);
    let folders = folders_in(&root)?;
    let first = timed(&folders, first_threads)?;
    let (files, _) = list(&folders)?;
    let mut out = io::stdout();
    writeln!(out, "{files} files in {} folders under {}", folders.len(), root.display())?;
    writeln!(out, "first pass on {first_threads} thread(s), the cache as it was: {:.2} ms", first.as_secs_f64() * 1e3)?;
    for threads in [1, 2, 4, 8] {
        let mut times: Vec<Duration> = (0..repeats).map(|_| timed(&folders, threads)).collect::<io::Result<_>>()?;
        times.sort();
        let median = times.get(times.len() / 2).copied().unwrap_or_default();
        let per_file = median.as_nanos() / u128::try_from(files.max(1)).unwrap_or(1);
        writeln!(out, "{threads} thread(s): median {:.2} ms, {per_file} ns per file", median.as_secs_f64() * 1e3)?;
    }
    Ok(())
}

/// One listing of every folder, split round-robin across `threads`.
fn timed(folders: &[PathBuf], threads: usize) -> io::Result<Duration> {
    let start = Instant::now();
    std::thread::scope(|s| -> io::Result<()> {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let mine: Vec<PathBuf> = folders.iter().skip(t).step_by(threads).cloned().collect();
                s.spawn(move || list(&mine))
            })
            .collect();
        for h in handles {
            h.join().map_err(|_| io::Error::other("a listing thread panicked"))??;
        }
        Ok(())
    })?;
    Ok(start.elapsed())
}

/// Every entry of each folder: how many files, and their sizes' sum (so the reads aren't optimized away).
fn list(folders: &[PathBuf]) -> io::Result<(usize, u64)> {
    let mut lister = Lister::new();
    let (mut files, mut bytes) = (0usize, 0u64);
    for folder in folders {
        let dir = lister.open(&units(folder))?;
        lister.each(&dir, |e| {
            if !e.folder {
                files += 1;
                bytes = bytes.wrapping_add(e.size);
            }
        })?;
    }
    Ok((files, bytes))
}

fn units(path: &Path) -> Vec<Unit> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().collect()
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
}
