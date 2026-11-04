#![no_main]
// A query as the desktop and the catalog read it from JSON: an error is fine, a panic is not.
use libfuzzer_sys::fuzz_target;
use pg_model::query::Query;

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<Query>(data);
});
