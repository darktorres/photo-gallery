#![no_main]
// The PNG, WebP and GIF block walkers, and the byte search, on arbitrary bytes.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for chunk in pg_container::png_chunks(data) {
        let _ = pg_container::chunk_data(&chunk);
    }
    for chunk in pg_container::webp_chunks(data) {
        let _ = pg_container::chunk_data(&chunk);
        let _ = pg_container::webp_exif(&chunk);
    }
    let _ = pg_container::gif_header_len(data);
    let mut blocks = pg_container::gif_blocks(data);
    for _ in blocks.by_ref() {}
    let _ = blocks.complete();
    if data.len() >= 2 {
        let _ = pg_container::find(data, &data[..2]);
    }
});
