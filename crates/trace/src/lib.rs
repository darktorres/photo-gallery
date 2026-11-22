//! Startup stages, for measuring them. Each stage is a function with a plain C name,
//! called when the app gets there:
//!
//! - Intel SDE counts the instructions between two of them, on every thread: `sde -icount -controller_default_start
//!   0 -control start:enter_func:pg_stage_main:bcast,stop:enter_func:pg_stage_ready:bcast -- gallery.exe …`.
//! - The app's sink (`set_sink`) gets each stage with the milliseconds since the process was created; the app prints
//!   them with `PG_TRACE=1`.
//!
//! Otherwise a stage costs a call. The tree and the list are marked each time one is shown; the first of each is the
//! start's. The library does not print: what a stage does is the app's.

mod sink;
mod stages;

pub use sink::set_sink;
pub use stages::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn record(stage: &str, ms: f64) {
        assert!(ms >= 0.0, "{stage} at {ms} ms");
        SEEN.lock().unwrap().push(stage.to_string());
    }

    /// The sink is set once per process, so one test sets it: the stages it fires reach it by their names, in order.
    #[test]
    fn stages_reach_the_sink_by_their_names() {
        set_sink(record);
        pg_stage_main();
        pg_stage_ready();
        assert_eq!(*SEEN.lock().unwrap(), ["pg_stage_main", "pg_stage_ready"]);
    }
}
