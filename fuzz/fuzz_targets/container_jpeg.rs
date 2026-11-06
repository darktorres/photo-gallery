#![no_main]
// The JPEG segment walker and its EXIF lookup, on arbitrary bytes: no panic, and the walk ends.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut segments = pg_container::jpeg_segments(data);
    let mut count = 0usize;
    for segment in segments.by_ref() {
        let _ = pg_container::jpeg_exif(&segment);
        count += 1;
        // A walk over N bytes has at most N segments: a loop here would be a bug in the walker.
        assert!(count <= data.len() + 1, "the segment walk does not end");
    }
    let _ = segments.damaged();
    let _ = pg_container::jpeg_is_progressive(data);
});
