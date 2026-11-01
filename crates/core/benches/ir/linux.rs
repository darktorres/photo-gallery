// The benches of ../ir.rs, included there on Linux only: the per-file work here, the catalog's in catalog.rs. Setup
// (reading the file, creating the decoder and the encoder, copying and opening the catalog) runs outside the
// measured function.

use std::hint::black_box;

use gungraun::prelude::*;
use pg_core::analyze::{self, Analysis};
use pg_core::decode::Decoder;
use pg_core::thumbs;
use turbojpeg::Compressor;

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

fn for_analyze() -> (Vec<u8>, Decoder) {
    (read_sample(), Decoder::new().unwrap())
}

fn for_thumb() -> (Vec<u8>, Decoder, Compressor) {
    (read_sample(), Decoder::new().unwrap(), Compressor::new().unwrap())
}

#[library_benchmark]
#[bench::sample(setup = for_analyze)]
fn analyze_jpeg((bytes, mut decoder): (Vec<u8>, Decoder)) -> Analysis {
    black_box(analyze::analyze(black_box(&bytes), 0, &mut decoder).unwrap())
}

#[library_benchmark]
#[bench::h256(setup = for_thumb)]
fn thumb_jpeg((bytes, mut decoder, mut encoder): (Vec<u8>, Decoder, Compressor)) -> Vec<u8> {
    black_box(thumbs::make(black_box(&bytes), 256, 0, &mut decoder, &mut encoder).unwrap())
}

library_benchmark_group!(name = per_file, benchmarks = [analyze_jpeg, thumb_jpeg]);

include!("catalog.rs");

main!(library_benchmark_groups = per_file, catalog);
