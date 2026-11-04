#![no_main]
// The image key text the grid and the providers exchange (`<file id>@<mtime>.<rotation>.<turns>`): read or refused.
use libfuzzer_sys::fuzz_target;
use pg_model::image::ImageKey;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    if let Some(key) = ImageKey::parse(text) {
        // What parses writes back to the same key.
        assert_eq!(ImageKey::parse(&key.to_string()), Some(key));
    }
});
