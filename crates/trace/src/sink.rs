//! The sink the app sets, which each stage is given: its name and the milliseconds since the process was created.

use std::sync::OnceLock;

/// What a stage does: given its name and the milliseconds since the process was created.
static SINK: OnceLock<fn(&str, f64)> = OnceLock::new();

/// Set what each stage does. The first sink set stays: the app sets it once, before the first stage.
pub fn set_sink(sink: fn(&str, f64)) {
    let _ = SINK.set(sink);
}

/// Hands a stage to the sink, if one is set.
pub(crate) fn reached(stage: &str) {
    if let Some(sink) = SINK.get() {
        sink(stage, pg_platform::clock::process_age_ms());
    }
}
