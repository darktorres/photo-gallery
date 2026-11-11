// The benches of ../ir.rs, included there on Linux only: the per-file work here, the catalog's in catalog.rs. Setup
// (reading the file, creating the analyzer and the thumbnail maker, copying and opening the catalog) runs outside the
// measured function. Their buffers start empty, so each bench measures a worker's first file.

use std::hint::black_box;

use gungraun::prelude::*;
use pg_analysis::analyze::Analyzer;
use pg_codec::decode::{Decoder, MinSize};
use pg_codec::pixels::Rgb;
use pg_model::analysis::Analyzed;
use pg_model::image::View;
use pg_render::display::{self, Shown};
use pg_model::turns::Turns;
use pg_render::maker::Maker;

/// A synthetic photo, 1280×1017 baseline at about 126 KB: the same size as the median file of the test set that the
/// benches used before, so the decoder and the thumbnail maker do photo-sized work. Committed, so CI has it;
/// tools/profiling/make_bench_jpeg.py makes it. `PG_BENCH_JPEG` at build time picks another file (gungraun clears the
/// environment the benchmarks run in).
const SAMPLE: &str = match option_env!("PG_BENCH_JPEG") {
    Some(path) => path,
    None => concat!(env!("CARGO_MANIFEST_DIR"), "/benches/ir/sample.jpg"),
};

/// The value of a setup step or a measured call. A benchmark whose step fails has nothing to measure, so the run stops
/// here with the error; the benches' setup and bodies have no other way to report it.
#[expect(clippy::panic, reason = "a benchmark whose step fails has nothing to measure: the run stops with the error")]
fn or_stop<T, E: std::fmt::Display>(result: Result<T, E>, what: impl std::fmt::Display) -> T {
    match result {
        Ok(value) => value,
        Err(e) => panic!("{what}: {e}"),
    }
}

fn read_sample() -> Vec<u8> {
    or_stop(std::fs::read(SAMPLE), SAMPLE)
}

fn for_analyze() -> (Vec<u8>, Analyzer) {
    (read_sample(), or_stop(Analyzer::new(), "the analyzer"))
}

fn for_thumb() -> (Vec<u8>, Maker) {
    (read_sample(), or_stop(Maker::new(), "the thumbnail maker"))
}

#[library_benchmark]
#[bench::sample(setup = for_analyze)]
fn analyze_jpeg((bytes, mut analyzer): (Vec<u8>, Analyzer)) -> Analyzed {
    black_box(or_stop(analyzer.analyze(black_box(&bytes), Turns::NONE), "analyze"))
}

#[library_benchmark]
#[bench::h256(setup = for_thumb)]
fn thumb_jpeg((bytes, mut maker): (Vec<u8>, Maker)) -> Vec<u8> {
    black_box(or_stop(maker.make(black_box(&bytes), 256, Turns::NONE), "make"))
}

/// The committed clip the video tests use: its poster frame is what the viewer shows of a video until it plays.
const CLIP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../video/tests/sample.mp4");

/// The viewer's window, in device pixels: the image is fitted into it. Its decode keeps 4× the window (the viewer's
/// margin), so the sample decodes at full size.
const WINDOW: View = View::Fit(640, 400);

fn for_viewer_jpeg() -> (Vec<u8>, Decoder) {
    (read_sample(), or_stop(Decoder::new(), "the decoder"))
}

/// The sample's pixels as a PNG, made here (outside the measured call).
fn for_viewer_png() -> (Vec<u8>, Decoder) {
    let mut decoder = or_stop(Decoder::new(), "the decoder");
    let image = or_stop(pg_render::display::decode(&mut decoder, &read_sample(), MinSize::Full, Turns::NONE), "decode");
    let width = or_stop(u32::try_from(image.image.width), "the width");
    let height = or_stop(u32::try_from(image.image.height), "the height");
    (or_stop(pg_testkit::png_rgb(width, height, &image.image.pixels), "the PNG"), decoder)
}

/// The image fitted into the window, as the viewer shows it.
fn fitted(image: &Shown) -> Option<Rgb> {
    or_stop(image.resized(WINDOW), "fit")
}

// The viewer's image of a still: the decode its window asks for, then the fit. (The path import is catalog.rs's.)
#[library_benchmark]
#[bench::jpeg(setup = for_viewer_jpeg)]
#[bench::png(setup = for_viewer_png)]
fn viewer_still((bytes, mut decoder): (Vec<u8>, Decoder)) -> Option<Rgb> {
    let decoded = or_stop(display::decode(&mut decoder, black_box(&bytes), display::min_size(WINDOW), Turns::NONE), "decode");
    black_box(fitted(&Shown::from(decoded)))
}

// The viewer's image of a video: its poster frame, then the fit.
#[library_benchmark]
#[bench::mp4(CLIP)]
fn viewer_poster(clip: &str) -> Option<Rgb> {
    let frame = or_stop(display::poster(black_box(Path::new(clip)), 0, Turns::NONE), "the poster frame");
    black_box(fitted(&frame))
}

library_benchmark_group!(name = per_file, benchmarks = [analyze_jpeg, thumb_jpeg]);
library_benchmark_group!(name = viewer, benchmarks = [viewer_still, viewer_poster]);

include!("catalog.rs");

main!(library_benchmark_groups = per_file, viewer, catalog);
