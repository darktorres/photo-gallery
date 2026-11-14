//! `png-decoders time [--secs S] FILE...`: median wall time per decode, one thread, every decoder.
//! `png-decoders compare FILE...`: each decoder's output against the app's fork, sample by sample.
//! `png-decoders allocs FILE...`: each decoder's heap traffic per decode in steady state: allocation calls, bytes
//! requested, and the peak above where the decode started.
//! `png-decoders dump DIR FILE...`: the app's output as DIR/NAME.raw, and DIR/NAME.txt with "width height channels
//! bytes-per-sample", for the Pillow bench (pillow_bench.py) and the C harness to check their output against.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use png_decoders::{Dec, Decoders, Dims};

/// Counts every heap call of the process: the decoders' allocations are the only ones made while `allocs` measures.
struct Counting;

static CALLS: AtomicUsize = AtomicUsize::new(0);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grow(bytes: usize) {
    CALLS.fetch_add(1, Relaxed);
    REQUESTED.fetch_add(bytes, Relaxed);
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
}

// SAFETY: every method forwards to the system allocator, with the layout the caller gave.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            CALLS.fetch_add(1, Relaxed);
            REQUESTED.fetch_add(new_size, Relaxed);
            let live = LIVE.fetch_add(new_size, Relaxed) + new_size;
            LIVE.fetch_sub(layout.size(), Relaxed);
            PEAK.fetch_max(live, Relaxed);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        bail!("usage: png-decoders time [--secs S] FILE... | compare FILE... | allocs FILE... | dump DIR FILE...");
    }
    let cmd = args.remove(0);
    if cmd == "dump" {
        return dump(&args);
    }
    let mut secs = 1.0;
    if args.first().map(String::as_str) == Some("--secs") {
        secs = args.get(1).context("--secs wants a number")?.parse()?;
        args.drain(..2);
    }
    let files = read_files(&args)?;
    let mut decoders = Decoders::new();
    match cmd.as_str() {
        "time" => time(&mut decoders, &files, Duration::from_secs_f64(secs)),
        "compare" => compare(&mut decoders, &files),
        "allocs" => allocs(&mut decoders, &files),
        _ => bail!("unknown command {cmd}"),
    }
}

fn read_files(paths: &[String]) -> Result<Vec<(String, Vec<u8>)>> {
    paths
        .iter()
        .map(|p| {
            let name = Path::new(p).file_stem().unwrap_or_default().to_string_lossy().into_owned();
            Ok((name, std::fs::read(p).with_context(|| p.clone())?))
        })
        .collect()
}

/// Bytes per sample of a decoded image: 1 for 8-bit, 2 for 16-bit (the output has no other size).
fn bytes_per_sample(dims: Dims, out: &[u8]) -> usize {
    out.len() / (dims.width * dims.height * dims.channels).max(1)
}

fn dump(args: &[String]) -> Result<()> {
    let (dir, paths) = args.split_first().context("usage: dump DIR FILE...")?;
    let mut decoders = Decoders::new();
    let mut out = Vec::new();
    for (name, bytes) in read_files(paths)? {
        let d = decoders.decode(Dec::Fork, &bytes, &mut out)?;
        let bps = bytes_per_sample(d, &out);
        std::fs::write(Path::new(dir).join(format!("{name}.raw")), &out)?;
        std::fs::write(
            Path::new(dir).join(format!("{name}.txt")),
            format!("{} {} {} {}\n", d.width, d.height, d.channels, bps),
        )?;
        println!("{name}: {}x{} {} channels, {} bits", d.width, d.height, d.channels, bps * 8);
    }
    Ok(())
}

fn label(dims: Dims, bps: usize) -> String {
    format!("{:>5.2} MP  {} ch  {:>2}-bit", (dims.width * dims.height) as f64 / 1e6, dims.channels, bps * 8)
}

fn time(decoders: &mut Decoders, files: &[(String, Vec<u8>)], budget: Duration) -> Result<()> {
    let width = files.iter().map(|f| f.0.len()).max().unwrap_or(0);
    print!("{:width$}  {:>26}", "", "");
    for dec in Dec::ALL {
        print!(" {:>15}", dec.name());
    }
    println!("   (median ms per decode)");
    let mut out = Vec::new();
    for (name, bytes) in files {
        let dims = decoders.decode(Dec::Fork, bytes, &mut out)?;
        print!("{name:width$}  {}", label(dims, bytes_per_sample(dims, &out)));
        for dec in Dec::ALL {
            if decoders.decode(dec, bytes, &mut out).is_err() {
                print!(" {:>15}", "failed");
                continue;
            }
            for _ in 0..2 {
                black_box(decoders.decode(dec, black_box(bytes), &mut out)?);
            }
            let mut samples = Vec::new();
            let start = Instant::now();
            while samples.len() < 5 || start.elapsed() < budget {
                let t = Instant::now();
                black_box(decoders.decode(dec, black_box(bytes), &mut out)?);
                samples.push(t.elapsed().as_secs_f64() * 1e3);
            }
            samples.sort_by(f64::total_cmp);
            print!(" {:>15.2}", samples[samples.len() / 2]);
        }
        println!();
    }
    Ok(())
}

/// The samples of `a` and `b` (same length, `bps` bytes each, big-endian for 16-bit): how many differ, and the
/// largest difference.
fn sample_diff(a: &[u8], b: &[u8], bps: usize) -> (usize, u32) {
    let value = |s: &[u8]| s.iter().fold(0u32, |v, &byte| (v << 8) | u32::from(byte));
    let mut differ = 0;
    let mut largest = 0;
    for (x, y) in a.chunks(bps).zip(b.chunks(bps)) {
        let d = value(x).abs_diff(value(y));
        if d != 0 {
            differ += 1;
            largest = largest.max(d);
        }
    }
    (differ, largest)
}

fn compare(decoders: &mut Decoders, files: &[(String, Vec<u8>)]) -> Result<()> {
    println!("Against the app's png fork: identical samples, or how many samples differ and the largest difference.");
    let (mut out, mut reference) = (Vec::new(), Vec::new());
    for (name, bytes) in files {
        let dims = decoders.decode(Dec::Fork, bytes, &mut reference)?;
        let bps = bytes_per_sample(dims, &reference);
        print!("{name}  {}", label(dims, bps));
        for dec in Dec::ALL.into_iter().skip(1) {
            let other = match decoders.decode(dec, bytes, &mut out) {
                Ok(other) => other,
                Err(err) => {
                    print!("  {}: failed ({err})", dec.name());
                    continue;
                }
            };
            if other != dims || out.len() != reference.len() {
                print!("  {}: {}×{} {} ch", dec.name(), other.width, other.height, other.channels);
            } else if out == reference {
                print!("  {}: identical", dec.name());
            } else {
                let (differ, max) = sample_diff(&out, &reference, bps);
                print!("  {}: {differ} of {} samples differ, largest {max}", dec.name(), out.len() / bps);
            }
        }
        println!();
    }
    Ok(())
}

/// Heap traffic of one decode after two warm-ups (so the buffers a decoder keeps are at their size): the calls, the
/// bytes they requested, and the peak above the live bytes when the decode started.
fn allocs(decoders: &mut Decoders, files: &[(String, Vec<u8>)]) -> Result<()> {
    println!("Heap traffic per decode, steady state (after two warm-ups): calls, KiB requested, peak KiB above the start.");
    let width = files.iter().map(|f| f.0.len()).max().unwrap_or(0);
    for (name, bytes) in files {
        print!("{name:width$}");
        for dec in Dec::ALL {
            let mut out = Vec::new();
            if decoders.decode(dec, bytes, &mut out).is_err() {
                print!("  {}: failed", dec.name());
                continue;
            }
            for _ in 0..2 {
                black_box(decoders.decode(dec, black_box(bytes), &mut out)?);
            }
            let start = LIVE.load(Relaxed);
            PEAK.store(start, Relaxed);
            CALLS.store(0, Relaxed);
            REQUESTED.store(0, Relaxed);
            black_box(decoders.decode(dec, black_box(bytes), &mut out)?);
            let calls = CALLS.load(Relaxed);
            let requested = REQUESTED.load(Relaxed) as f64 / 1024.0;
            let peak = (PEAK.load(Relaxed) - start) as f64 / 1024.0;
            print!("  {}: {calls} calls, {requested:.0} KiB, peak {peak:.0} KiB", dec.name());
        }
        println!();
    }
    Ok(())
}
