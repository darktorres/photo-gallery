//! `png-decoders time [--secs S] FILE...`: median wall time per decode, one thread, every decoder.
//! `png-decoders compare FILE...`: each decoder's output against the app's fork, sample by sample.
//! `png-decoders dump DIR FILE...`: the app's output as DIR/NAME.raw, and DIR/NAME.txt with "width height channels",
//! for the Pillow bench (pillow_bench.py) to check its own output against.

use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use png_decoders::{Dec, Decoders, Dims};

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        bail!("usage: png-decoders time [--secs S] FILE... | compare FILE... | dump DIR FILE...");
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

fn dump(args: &[String]) -> Result<()> {
    let (dir, paths) = args.split_first().context("usage: dump DIR FILE...")?;
    let mut decoders = Decoders::new();
    let mut out = Vec::new();
    for (name, bytes) in read_files(paths)? {
        let d = decoders.decode(Dec::Fork, &bytes, &mut out)?;
        std::fs::write(Path::new(dir).join(format!("{name}.raw")), &out)?;
        std::fs::write(Path::new(dir).join(format!("{name}.txt")), format!("{} {} {}\n", d.width, d.height, d.channels))?;
        println!("{name}: {}x{} {} channels", d.width, d.height, d.channels);
    }
    Ok(())
}

fn label(dims: Dims) -> String {
    format!("{:>5.2} MP  {} ch", (dims.width * dims.height) as f64 / 1e6, dims.channels)
}

fn time(decoders: &mut Decoders, files: &[(String, Vec<u8>)], budget: Duration) -> Result<()> {
    let width = files.iter().map(|f| f.0.len()).max().unwrap_or(0);
    print!("{:width$}  {:>16}", "", "");
    for dec in Dec::ALL {
        print!(" {:>15}", dec.name());
    }
    println!("   (median ms per decode)");
    let mut out = Vec::new();
    for (name, bytes) in files {
        let dims = decoders.decode(Dec::Fork, bytes, &mut out)?;
        print!("{name:width$}  {}", label(dims));
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

fn compare(decoders: &mut Decoders, files: &[(String, Vec<u8>)]) -> Result<()> {
    println!("Against the app's png fork: identical samples, or how many samples differ and the largest difference.");
    let (mut out, mut reference) = (Vec::new(), Vec::new());
    for (name, bytes) in files {
        let dims = decoders.decode(Dec::Fork, bytes, &mut reference)?;
        print!("{name}  {}", label(dims));
        for dec in Dec::ALL.into_iter().skip(1) {
            let other = match decoders.decode(dec, bytes, &mut out) {
                Ok(other) => other,
                Err(err) => {
                    print!("  {}: failed ({err})", dec.name());
                    continue;
                }
            };
            if other != dims {
                print!("  {}: {}×{} {} ch", dec.name(), other.width, other.height, other.channels);
            } else if out == reference {
                print!("  {}: identical", dec.name());
            } else {
                let differ = out.iter().zip(&reference).filter(|(a, b)| a != b).count();
                let max = out.iter().zip(&reference).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
                print!("  {}: {differ} of {} samples differ, largest {max}", dec.name(), out.len());
            }
        }
        println!();
    }
    Ok(())
}
