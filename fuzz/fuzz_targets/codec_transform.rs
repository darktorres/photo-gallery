#![no_main]
// The decoder's orientation transform on a small picture of arbitrary pixels: its output is the size it reports.
use libfuzzer_sys::fuzz_target;
use pg_model::orientation::Orientation;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let w = usize::from(data[0] % 16) + 1;
    let h = usize::from(data[1] % 16) + 1;
    let unit = usize::from(data[2] % 4) + 1;
    let orientation = Orientation::from_tag(data[3] % 9);
    let mut pixels = data[4..].to_vec();
    pixels.resize(w * h * unit, 0);
    let (out, ow, oh) = pg_codec::decode::transform(&pixels, w, h, unit, orientation);
    assert_eq!(out.len(), ow * oh * unit, "the transformed picture is the size it reports");
});
