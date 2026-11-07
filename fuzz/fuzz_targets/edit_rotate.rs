#![no_main]
// Rotating a file's bytes in place (the lossless writer's rewrite) on arbitrary bytes: an error is fine, a panic is not.
use libfuzzer_sys::fuzz_target;
use pg_model::format::Format;
use pg_model::turns::Turns;

fuzz_target!(|data: &[u8]| {
    let Some(format) = Format::sniff(data) else { return };
    let turns = Turns::new(data.last().copied().unwrap_or(0));
    let _ = pg_edit::rotate::rotate(data, format, turns);
});
