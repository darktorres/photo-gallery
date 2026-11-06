#![no_main]
// The metadata reader (EXIF, XMP, ICC and the rest) on arbitrary bytes, in the format the bytes sniff as, or JPEG.
use libfuzzer_sys::fuzz_target;
use pg_metadata::{Meta, MetaScratch};
use pg_model::format::Format;

fuzz_target!(|data: &[u8]| {
    let format = Format::sniff(data).unwrap_or(Format::Jpeg);
    let mut scratch = MetaScratch::default();
    let _ = Meta::read(data, format, &mut scratch);
});
