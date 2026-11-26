//! `png-decoders time [--secs S] FILE...`: median wall time per decode, one thread, every decoder.
//! `png-decoders compare FILE...`: each decoder's output against the app's fork, sample by sample.
//! `png-decoders allocs FILE...`: each decoder's heap traffic per decode in steady state: allocation calls, bytes
//! requested, and the peak above where the decode started.
//! `png-decoders dump DIR FILE...`: the app's output as DIR/NAME.raw, and DIR/NAME.txt with "width height channels
//! bytes-per-sample", for the Pillow bench (pillow_bench.py) and the C harness to check their output against.
//! `png-decoders profile [--secs S] FILE...`: the fork alone, decoding each file for S seconds, for a profiler.
//! `png-decoders inflate [--secs S] FILE...`: fdeflate alone on each file's IDAT stream, into its scanlines.
//! `png-decoders inflate-stream [--secs S] FILE...`: the same inflate, streamed in pieces into a rolled window.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use png_decoders::{Dec, Decoders, Dims};

/// Counts every heap call of the process while the `allocs` command runs. Counting is off otherwise, so the timed
/// commands make no counting calls.
struct Counting;

static COUNTING: AtomicBool = AtomicBool::new(false);
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
        if !p.is_null() && COUNTING.load(Relaxed) {
            grow(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() && COUNTING.load(Relaxed) {
            grow(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        if COUNTING.load(Relaxed) {
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() && COUNTING.load(Relaxed) {
            // The old block leaves the count first, then the block that replaces it is counted as a new one, as
            // alloc_count.cpp's realloc does, so the peak never includes both.
            LIVE.fetch_sub(layout.size(), Relaxed);
            grow(new_size);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        bail!(
            "usage: png-decoders time [--secs S] FILE... | compare FILE... | allocs FILE... | dump DIR FILE... | profile [--secs S] FILE... | inflate [--secs S] FILE... | inflate-stream [--secs S] FILE..."
        );
    }
    let cmd = args.remove(0);
    COUNTING.store(cmd == "allocs", Relaxed);
    if cmd == "dump" {
        return dump(&args);
    }
    let mut secs = 1.0;
    if args.first().map(String::as_str) == Some("--secs") {
        secs = args.get(1).context("--secs wants a number")?.parse()?;
        args.drain(..2);
    }
    // try_from keeps a negative, NaN or infinite --secs an error instead of a panic.
    let budget = Duration::try_from_secs_f64(secs).context("--secs wants a finite, non-negative number")?;
    let files = read_files(&args)?;
    let mut decoders = Decoders::new();
    match cmd.as_str() {
        "time" => time(&mut decoders, &files, budget),
        "profile" => profile(&mut decoders, &files, budget),
        "compare" => compare(&mut decoders, &files),
        "allocs" => allocs(&mut decoders, &files),
        "inflate" => inflate(&files, budget),
        "inflate-stream" => inflate_stream(&files, budget),
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

/// `profile --secs S FILE...`: the fork alone, decoding each file in turn for S seconds, for a profiler to sample. It
/// prints the decode count, not a time: the timings come from `time`.
fn profile(decoders: &mut Decoders, files: &[(String, Vec<u8>)], budget: Duration) -> Result<()> {
    let mut out = Vec::new();
    for (name, bytes) in files {
        let start = Instant::now();
        let mut decodes = 0u64;
        while start.elapsed() < budget {
            black_box(decoders.decode(Dec::Fork, black_box(bytes), &mut out)?);
            decodes += 1;
        }
        println!("{name}: {decodes} decodes of the fork in {:.1} s", budget.as_secs_f64());
    }
    Ok(())
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
                print!("  {}: {}x{} {} ch", dec.name(), other.width, other.height, other.channels);
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

/// A fresh decompressor. PNG_INFLATE_NO_CHECKSUM=1 ignores the zlib checksum, as the app's fork does by default.
fn decompressor() -> fdeflate::Decompressor {
    let mut d = fdeflate::Decompressor::new();
    if std::env::var_os("PNG_INFLATE_NO_CHECKSUM").is_some() {
        d.ignore_adler32();
    }
    d
}

/// The IDAT chunks' data, joined: the zlib stream of the image.
fn idat_stream(png: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 8;
    while let Some(head) = png.get(pos..pos + 8) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        if &head[4..8] == b"IDAT"
            && let Some(data) = png.get(pos + 8..pos + 8 + len)
        {
            out.extend_from_slice(data);
        }
        pos += 12 + len;
    }
    out
}

/// The size of the filtered scanlines as stored in the file (before EXPAND): one filter byte a row, then the row's
/// samples at the file's depth, palette indices included.
fn stored_scanline_bytes(bytes: &[u8]) -> Result<usize> {
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as usize;
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) as usize;
    let (depth, color) = (bytes[24] as usize, bytes[25]);
    let samples = match color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => bail!("unknown colour type {color}"),
    };
    Ok(h * (1 + (w * samples * depth).div_ceil(8)))
}

/// One decode of `stream` into `buf`, which must be exactly the stored scanlines' size and end the stream.
fn inflate_once(d: &mut fdeflate::Decompressor, stream: &[u8], buf: &mut [u8]) -> Result<()> {
    let (_, produced) = d.read(stream, buf, 0, true).map_err(|e| anyhow!("{e:?}"))?;
    if produced != buf.len() || !d.is_done() {
        bail!("inflated {produced} of {} bytes (done: {})", buf.len(), d.is_done());
    }
    Ok(())
}

/// The input is fed to the inflater in pieces of this size, as a streaming reader feeds it.
const IN_CHUNK: usize = 16 << 10;
/// The output window a streamed inflate writes into, and the history it keeps when the window is full (back-references
/// reach 32 KiB back, so fdeflate needs that much before it can write further; zlib keeps its own history).
const WINDOW: usize = 96 << 10;
const HISTORY: usize = 32 << 10;

/// The stream inflated through fdeflate as a streaming reader does it: input in `IN_CHUNK` pieces, output into a
/// `WINDOW` that is rolled, keeping the last `HISTORY` bytes, when it can't take another piece. The output itself is
/// discarded, as unfiltering would consume it. Returns the number of bytes inflated.
fn inflate_streamed(d: &mut fdeflate::Decompressor, stream: &[u8], window: &mut [u8]) -> Result<usize> {
    let (mut at_in, mut pos, mut total) = (0, 0, 0);
    while !d.is_done() {
        if window.len() - pos < IN_CHUNK {
            window.copy_within(pos - HISTORY..pos, 0);
            pos = HISTORY;
        }
        let end = (at_in + IN_CHUNK).min(stream.len());
        let (used, produced) = d
            .read(&stream[at_in..end], window, pos, end == stream.len())
            .map_err(|e| anyhow!("{e:?}"))?;
        if used == 0 && produced == 0 {
            bail!("the stream stalled after {total} bytes");
        }
        at_in += used;
        pos += produced;
        total += produced;
    }
    Ok(total)
}

/// The median over the same protocol as `time`: two warm-ups, at least five samples and `budget` in all.
fn median_ms(budget: Duration, mut f: impl FnMut() -> Result<()>) -> Result<f64> {
    for _ in 0..2 {
        f()?;
    }
    let mut samples = Vec::new();
    let start = Instant::now();
    while samples.len() < 5 || start.elapsed() < budget {
        let t = Instant::now();
        f()?;
        samples.push(t.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(f64::total_cmp);
    Ok(samples[samples.len() / 2])
}

/// `inflate_streamed` on each file's IDAT stream, with fdeflate: one decompressor reset between decodes (the fork's
/// way) and a new one each decode (png 0.18's). The output is checked against the stored scanline size. Interlaced
/// files are skipped, as in `inflate`.
fn inflate_stream(files: &[(String, Vec<u8>)], budget: Duration) -> Result<()> {
    println!("Inflate, streamed (fdeflate, input in 16 KiB pieces, output in a 96 KiB rolled window), median ms per decode.");
    let width = files.iter().map(|f| f.0.len()).max().unwrap_or(0);
    println!("{:width$}  {:>22}  {:>22}", "", "reset, reused", "new per decode");
    let mut window = vec![0u8; WINDOW];
    for (name, bytes) in files {
        if bytes.get(28) != Some(&0) {
            println!("{name:width$}  interlaced: skipped");
            continue;
        }
        let want = stored_scanline_bytes(bytes)?;
        let stream = idat_stream(bytes);
        let mut reused = decompressor();
        let reused_ms = median_ms(budget, || {
            reused.reset();
            check_total(inflate_streamed(&mut reused, &stream, &mut window)?, want)
        })?;
        let new_ms = median_ms(budget, || {
            check_total(inflate_streamed(&mut decompressor(), &stream, &mut window)?, want)
        })?;
        println!("{name:width$}  {reused_ms:>22.2}  {new_ms:>22.2}");
    }
    Ok(())
}

fn check_total(inflated: usize, want: usize) -> Result<()> {
    if inflated != want {
        bail!("inflated {inflated} of {want} bytes");
    }
    Ok(())
}

/// The inflate step alone, with fdeflate, on each file's IDAT stream into its filtered scanlines (filter bytes
/// included): once with one decompressor reset between decodes, as the fork keeps its own, and once with a new
/// decompressor each decode, as png 0.18 makes it. Interlaced files are skipped (their passes aren't one raster).
fn inflate(files: &[(String, Vec<u8>)], budget: Duration) -> Result<()> {
    println!("Inflate only (fdeflate), median ms per decode: the IDAT stream to its scanlines.");
    let width = files.iter().map(|f| f.0.len()).max().unwrap_or(0);
    println!("{:width$}  {:>22}  {:>22}", "", "reset, reused", "new per decode");
    for (name, bytes) in files {
        if bytes.get(28) != Some(&0) {
            println!("{name:width$}  interlaced: skipped");
            continue;
        }
        let raw = stored_scanline_bytes(bytes)?;
        let stream = idat_stream(bytes);
        let mut buf = vec![0u8; raw];
        let mut reused = decompressor();
        let reused_ms = median_ms(budget, || {
            reused.reset();
            inflate_once(&mut reused, &stream, &mut buf)
        })?;
        let new_ms = median_ms(budget, || inflate_once(&mut decompressor(), &stream, &mut buf))?;
        println!("{name:width$}  {reused_ms:>22.2}  {new_ms:>22.2}");
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
