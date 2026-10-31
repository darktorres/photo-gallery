//! Instruction counts (callgrind, through gungraun) of each decoder on each input, full size and at the scale
//! `analyze` asks for. Linux only, like the app's benches; the inputs are in `JPEG_INPUTS` (default
//! /root/jpeg-inputs), named as below. Setup (reading the file, creating the decoders) isn't measured.

#[cfg(target_os = "linux")]
mod linux {
    use std::hint::black_box;

    use gungraun::prelude::*;
    use jpeg_decoders::{Dec, Decoders, Mode, Rgb};

    const DIR: &str = match option_env!("JPEG_INPUTS") {
        Some(d) => d,
        None => "/root/jpeg-inputs",
    };
    pub const FILES: [&str; 7] = [
        "pin-base444-1mp",
        "pin-base444-3mp",
        "pin-prog420-1mp",
        "pin-base420-1mp",
        "phone-base420-4mp",
        "sony-base422-7mp",
        "a6000-base422-24mp",
    ];

    pub fn load(i: usize) -> (Vec<u8>, Decoders) {
        let path = format!("{DIR}/{}.jpg", FILES[i]);
        (std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}")), Decoders::new().unwrap())
    }

    fn run(dec: Dec, mode: Mode, (bytes, mut decoders): (Vec<u8>, Decoders)) -> Rgb {
        black_box(decoders.decode(dec, mode, black_box(&bytes)).unwrap())
    }

    macro_rules! bench {
        ($name:ident, $dec:expr, $mode:expr) => {
            #[library_benchmark]
            #[benches::file(args = [0, 1, 2, 3, 4, 5, 6], setup = load)]
            pub fn $name(input: (Vec<u8>, Decoders)) -> Rgb {
                run($dec, $mode, input)
            }
        };
    }

    bench!(full_turbo, Dec::Turbo, Mode::Full);
    bench!(full_turbo_fast, Dec::TurboFast, Mode::Full);
    bench!(full_zune, Dec::Zune, Mode::Full);
    bench!(full_jpeg_decoder, Dec::JpegDecoder, Mode::Full);
    bench!(full_ffmpeg, Dec::Ffmpeg, Mode::Full);
    bench!(full_vips, Dec::Vips, Mode::Full);
    bench!(scaled_turbo, Dec::Turbo, Mode::Analyze);
    bench!(scaled_turbo_fast, Dec::TurboFast, Mode::Analyze);
    bench!(scaled_jpeg_decoder, Dec::JpegDecoder, Mode::Analyze);
    bench!(scaled_ffmpeg, Dec::Ffmpeg, Mode::Analyze);
    bench!(scaled_vips, Dec::Vips, Mode::Analyze);

    library_benchmark_group!(
        name = decoders,
        benchmarks = [
            full_turbo,
            full_turbo_fast,
            full_zune,
            full_jpeg_decoder,
            full_ffmpeg,
            full_vips,
            scaled_turbo,
            scaled_turbo_fast,
            scaled_jpeg_decoder,
            scaled_ffmpeg,
            scaled_vips
        ]
    );
}

#[cfg(target_os = "linux")]
use linux::decoders;

#[cfg(target_os = "linux")]
gungraun::main!(library_benchmark_groups = decoders);

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the callgrind benches run on Linux (docs/PROFILING.md)");
}
