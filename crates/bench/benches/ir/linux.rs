// The benches of ../ir.rs, included there on Linux only: the per-file work here, the catalog's in catalog.rs. Setup
// (reading the file, creating the analyzer and the thumbnail maker, copying and opening the catalog) runs outside the
// measured function. Their buffers start empty, so each bench measures a worker's first file.

use std::hint::black_box;

use gungraun::prelude::*;
use pg_core::analysis::Analysis;
use pg_core::analyze::Analyzer;
use pg_core::maker::Maker;

/// A median-size file of the test set: a 126 KB mozjpeg re-encode, 1280×1017 baseline. It's one of the library's
/// photos, so it stays out of the repo and is read from D: through WSL. `PG_BENCH_JPEG` at build time picks
/// another file (gungraun clears the environment the benchmarks run in).
const SAMPLE: &str = match option_env!("PG_BENCH_JPEG") {
    Some(path) => path,
    None => "/mnt/d/PhotoGallery/datasets/pinterest/dups2/amateur-alice-von-v-mbDfqe.jpg.mozjpeg.jpg",
};

fn read_sample() -> Vec<u8> {
    std::fs::read(SAMPLE).unwrap_or_else(|e| panic!("{SAMPLE}: {e}"))
}

fn for_analyze() -> (Vec<u8>, Analyzer) {
    (read_sample(), Analyzer::new().unwrap())
}

fn for_thumb() -> (Vec<u8>, Maker) {
    (read_sample(), Maker::new().unwrap())
}

#[library_benchmark]
#[bench::sample(setup = for_analyze)]
fn analyze_jpeg((bytes, mut analyzer): (Vec<u8>, Analyzer)) -> Analysis {
    black_box(analyzer.analyze(black_box(&bytes), 0).unwrap())
}

#[library_benchmark]
#[bench::h256(setup = for_thumb)]
fn thumb_jpeg((bytes, mut maker): (Vec<u8>, Maker)) -> Vec<u8> {
    black_box(maker.make(black_box(&bytes), 256, 0).unwrap())
}

library_benchmark_group!(name = per_file, benchmarks = [analyze_jpeg, thumb_jpeg]);

include!("catalog.rs");

main!(library_benchmark_groups = per_file, catalog);
