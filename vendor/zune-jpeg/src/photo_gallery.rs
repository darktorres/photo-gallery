//! photo-gallery: decoders in a kept [`Scratch`] decode as new ones.
//!
//! The fixtures (`tests/photo_gallery`) are small JPEGs made by libjpeg (Pillow), ImageMagick and jpegtran: RGB at
//! every subsampling (4:4:4, 4:2:2, 4:2:0, 4:4:0, 4:1:1, 4:2:1 and mixed factors), gray, CMYK and YCCK, baseline and
//! progressive, restart markers by rows and by blocks, arithmetic coding (which zune-jpeg rejects), EXIF, a two-segment
//! ICC profile and XMP, sizes from 1×1 to 160×96, and damaged copies: cut in the headers, in the scans and before the
//! end marker, and with bits changed in the scans.
//!
//! Each file is decoded as pg-core's front end decodes it (a decoder for the headers, then one with the output colour
//! space), into several colour spaces, strict and not. A new decoder per file is the reference; decoders in one kept
//! scratch must give the same dimensions, colour spaces, pixels, errors and metadata, forwards and backwards, with
//! the scratch kept whole, trimmed to nothing and trimmed to 600 bytes after each file. With `set_skip_metadata`
//! they give the same, and no metadata.

extern crate std;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

use crate::{JpegDecoder, Scratch};

#[derive(Debug, PartialEq, Clone)]
struct Decoded {
    /// The dimensions and the input colour space, or the headers' error.
    headers: Result<(usize, usize, ColorSpace), String>,
    /// The pixels in each output colour space (strict, then not), or the error.
    pixels: Vec<Result<Vec<u8>, String>>,
    /// EXIF, ICC, XMP and IPTC, as the headers decoder found them.
    metadata: [Option<Vec<u8>>; 4]
}

const OUTPUTS: [ColorSpace; 4] = [ColorSpace::RGB, ColorSpace::RGBA, ColorSpace::Luma, ColorSpace::BGR];

fn options(strict: bool) -> DecoderOptions {
    DecoderOptions::default()
        .set_strict_mode(strict)
        .set_max_width(usize::MAX)
        .set_max_height(usize::MAX)
}

/// `bytes` decoded by new decoders (`scratch` `None`), or in `scratch`, given back after each decoder.
fn decode(bytes: &[u8], mut scratch: Option<&mut Scratch>, skip_metadata: bool) -> Decoded {
    let make = |options: DecoderOptions, scratch: &mut Option<&mut Scratch>| match scratch {
        None => JpegDecoder::new_with_options(ZCursor::new(bytes), options),
        Some(s) => {
            let mut d = JpegDecoder::new_with_scratch(ZCursor::new(bytes), options, core::mem::take(*s));
            d.set_skip_metadata(skip_metadata);
            d
        }
    };
    let give_back = |d: JpegDecoder<ZCursor<&[u8]>>, scratch: &mut Option<&mut Scratch>| {
        if let Some(s) = scratch {
            **s = d.into_scratch();
        }
    };

    let mut headers = make(options(false), &mut scratch);
    let read = headers.decode_headers();
    let metadata = [
        headers.exif().cloned(),
        headers.icc_profile(),
        headers.xmp().cloned(),
        headers.iptc().cloned()
    ];
    let headers_result = read.map(|()| {
        let (w, h) = headers.dimensions().unwrap();
        (w, h, headers.input_colorspace().unwrap())
    });
    give_back(headers, &mut scratch);
    let headers = match headers_result {
        Ok(h) => h,
        Err(e) => return Decoded { headers: Err(format!("{e:?}")), pixels: Vec::new(), metadata }
    };

    let mut pixels = Vec::new();
    for strict in [true, false] {
        for out in OUTPUTS {
            let mut d = make(options(strict).jpeg_set_out_colorspace(out), &mut scratch);
            pixels.push(d.decode().map_err(|e| format!("{e:?}")));
            give_back(d, &mut scratch);
        }
    }
    Decoded { headers: Ok(headers), pixels, metadata }
}

#[test]
fn kept_scratch_decodes_as_new_decoders() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/photo_gallery");
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            (path.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&path).unwrap())
        })
        .collect();
    files.sort();
    assert_eq!(files.len(), 48);

    let reference: Vec<Decoded> = files.iter().map(|(_, b)| decode(b, None, false)).collect();
    let names = |keep: &dyn Fn(&Decoded) -> bool| -> Vec<&str> {
        files.iter().zip(&reference).filter(|(_, d)| keep(d)).map(|((name, _), _)| name.as_str()).collect()
    };
    // The fixtures reach what they are meant to. The headers of the files cut there fail, and zune-jpeg rejects
    // arithmetic coding there too.
    let mut expected: Vec<&str> =
        files.iter().map(|(name, _)| name.as_str()).filter(|name| name.ends_with("_cut_headers.jpg")).collect();
    expected.push("rgb_37x23_arithmetic.jpg");
    expected.sort_unstable();
    assert_eq!(names(&|d| d.headers.is_err()), expected);
    // A file cut in its scans fails partway under strict mode, and decodes without it (the scratch is then reused
    // after a decode that stopped in the middle).
    let cut_scans = names(&|d| d.pixels.first().is_some_and(Result::is_err) && d.pixels[4].is_ok());
    for (name, _) in files.iter().filter(|(name, _)| name.ends_with("_cut_scan.jpg")) {
        assert!(cut_scans.contains(&name.as_str()), "{name}");
    }
    let with_metadata = files.iter().position(|(name, _)| name == "rgb_37x23_metadata.jpg").unwrap();
    assert_eq!(reference[with_metadata].metadata.iter().filter(|m| m.is_some()).count(), 3);
    assert_eq!(reference[with_metadata].metadata[1].as_ref().map(Vec::len), Some(70_000));

    for skip_metadata in [false, true] {
        for trim in [None, Some(0), Some(600)] {
            let mut scratch = Scratch::default();
            let order: Vec<usize> = (0..files.len()).chain((0..files.len()).rev()).collect();
            for i in order {
                let (name, bytes) = &files[i];
                let got = decode(bytes, Some(&mut scratch), skip_metadata);
                let mut want = reference[i].clone();
                if skip_metadata {
                    want.metadata = [None, None, None, None];
                }
                assert_eq!(got, want, "{name}, skip_metadata {skip_metadata}, trim {trim:?}");
                if let Some(bytes) = trim {
                    scratch.trim(bytes);
                }
            }
        }
    }
}

/// The components' vector stays in the scratch: a new decoder's empty one doesn't replace it when the
/// headers are read, so the next image's components go in the same allocation.
#[test]
fn kept_scratch_keeps_the_components_vector() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/photo_gallery");
    let bytes = std::fs::read(dir.join("rgb_37x23_420_baseline.jpg")).unwrap();
    let mut scratch = Scratch::default();
    let decode = |scratch: &mut Scratch| {
        let mut d = JpegDecoder::new_with_scratch(ZCursor::new(&bytes[..]), options(false), core::mem::take(scratch));
        d.decode().unwrap();
        *scratch = d.into_scratch();
    };
    decode(&mut scratch);
    assert!(scratch.components.capacity() >= 3);
    let kept = scratch.components.as_ptr();
    decode(&mut scratch);
    assert_eq!(scratch.components.as_ptr(), kept);
}
