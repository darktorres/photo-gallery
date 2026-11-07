//! `jpeg-decoders time [--secs S] FILE...`: median wall time per decode, one thread, every decoder and mode.
//! `jpeg-decoders compare FILE...`: each decoder's output against libjpeg-turbo's (as the app decodes).
//! `jpeg-decoders repeat N DECODER MODE FILE`: N decodes, nothing else, for whole-process callgrind runs. The
//! difference between two N is the cost of a decode on every thread, which gungraun's per-thread collection
//! misses for decoders that work in their own threads (libvips, jpeg-decoder).

use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use jpeg_decoders::{Dec, Decoders, Mode, denom, dimensions};

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        bail!("usage: jpeg-decoders time [--secs S] FILE... | compare FILE... | repeat N DECODER MODE FILE");
    }
    let cmd = args.remove(0);
    if cmd == "repeat" {
        return repeat(&args);
    }
    let mut secs = 1.0;
    if args.first().map(String::as_str) == Some("--secs") {
        secs = args.get(1).context("--secs wants a number")?.parse()?;
        args.drain(..2);
    }
    let files: Vec<(String, Vec<u8>)> = args
        .iter()
        .map(|p| {
            let name = Path::new(p).file_stem().unwrap_or_default().to_string_lossy().into_owned();
            Ok((name, std::fs::read(p).with_context(|| p.clone())?))
        })
        .collect::<Result<_>>()?;
    let mut decoders = Decoders::new()?;
    match cmd.as_str() {
        "time" => time(&mut decoders, &files, Duration::from_secs_f64(secs)),
        "compare" => compare(&mut decoders, &files),
        _ => bail!("unknown command {cmd}"),
    }
}

fn repeat(args: &[String]) -> Result<()> {
    let [n, dec, mode, file] = args else { bail!("usage: repeat N DECODER MODE FILE") };
    let n: usize = n.parse()?;
    let dec = Dec::ALL.into_iter().find(|d| d.name() == dec.as_str()).with_context(|| format!("no decoder {dec}"))?;
    let mode = [Mode::Full, Mode::Analyze].into_iter().find(|m| m.name() == mode.as_str()).context("mode")?;
    let bytes = std::fs::read(file).with_context(|| file.clone())?;
    let mut decoders = Decoders::new()?;
    for _ in 0..n {
        black_box(decoders.decode(dec, mode, black_box(&bytes))?);
    }
    Ok(())
}

fn header(files: &[(String, Vec<u8>)], name: &str, bytes: &[u8], mode: Mode) -> Result<String> {
    let width = files.iter().map(|f| f.0.len()).max().unwrap_or(0);
    let (w, h) = dimensions(bytes)?;
    let d = denom(mode, w, h);
    Ok(format!("{name:width$}  {:>5.1} MP  {:<13} 1/{d}", (w * h) as f64 / 1e6, mode.name()))
}

fn time(decoders: &mut Decoders, files: &[(String, Vec<u8>)], budget: Duration) -> Result<()> {
    print!("{:>width$}", "", width = files.iter().map(|f| f.0.len()).max().unwrap_or(0) + 30);
    for dec in Dec::ALL {
        print!(" {:>19}", dec.name());
    }
    println!("   (median ms per decode)");
    for mode in [Mode::Full, Mode::Analyze] {
        for (name, bytes) in files {
            print!("{}", header(files, name, bytes, mode)?);
            for dec in Dec::ALL {
                if mode != Mode::Full && !dec.scales() {
                    print!(" {:>19}", "—");
                    continue;
                }
                for _ in 0..2 {
                    black_box(decoders.decode(dec, mode, bytes)?);
                }
                let mut samples = Vec::new();
                let start = Instant::now();
                while samples.len() < 5 || start.elapsed() < budget {
                    let t = Instant::now();
                    black_box(decoders.decode(dec, mode, black_box(bytes))?);
                    samples.push(t.elapsed().as_secs_f64() * 1e3);
                }
                samples.sort_by(f64::total_cmp);
                print!(" {:>19.2}", samples[samples.len() / 2]);
            }
            println!();
        }
    }
    Ok(())
}

fn compare(decoders: &mut Decoders, files: &[(String, Vec<u8>)]) -> Result<()> {
    println!("Against libjpeg-turbo (the app's decode): mean |difference| per channel value, the largest, and PSNR.");
    for mode in [Mode::Full, Mode::Analyze] {
        for (name, bytes) in files {
            let reference = decoders.decode(Dec::Turbo, mode, bytes)?;
            print!("{}", header(files, name, bytes, mode)?);
            for dec in Dec::ALL.into_iter().skip(1) {
                if mode != Mode::Full && !dec.scales() {
                    print!("  {}: —", dec.name());
                    continue;
                }
                let out = decoders.decode(dec, mode, bytes)?;
                if (out.width, out.height) != (reference.width, reference.height) {
                    print!(
                        "  {}: {}×{} vs {}×{}",
                        dec.name(),
                        out.width,
                        out.height,
                        reference.width,
                        reference.height
                    );
                    continue;
                }
                let (mut sum, mut max, mut sq) = (0u64, 0u8, 0u64);
                for (a, b) in out.pixels.iter().zip(&reference.pixels) {
                    let d = a.abs_diff(*b);
                    sum += d as u64;
                    sq += (d as u64) * (d as u64);
                    max = max.max(d);
                }
                let n = out.pixels.len() as f64;
                let psnr = if sq == 0 { f64::INFINITY } else { 10.0 * (255.0f64.powi(2) / (sq as f64 / n)).log10() };
                print!("  {}: {:.3} / {max} / {psnr:.1} dB", dec.name(), sum as f64 / n);
            }
            println!();
        }
    }
    Ok(())
}
