//! Startup stages, for measuring them (docs/PROFILING.md "Startup"). Each stage is a function with a plain C name,
//! called when the app gets there:
//!
//! - Intel SDE counts the instructions between two of them, on every thread: `sde -icount -controller_default_start
//!   0 -control start:enter_func:pg_stage_main:bcast,stop:enter_func:pg_stage_ready:bcast -- gallery.exe …`.
//! - The app's sink (`set_sink`) gets each stage with the milliseconds since the process was created; the app prints
//!   them with `PG_TRACE=1`.
//!
//! Otherwise a stage costs a call. The tree and the list are marked each time one is shown; the first of each is the
//! start's. The library does not print: what a stage does is the app's.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::sync::OnceLock;

/// What a stage does: given its name and the milliseconds since the process was created.
static SINK: OnceLock<fn(&str, f64)> = OnceLock::new();

/// Set what each stage does. The first sink set stays: the app sets it once, before the first stage.
pub fn set_sink(sink: fn(&str, f64)) {
    let _ = SINK.set(sink);
}

/// Each body names its stage, so no two are identical and the linker can't fold them into one function.
macro_rules! stages {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {
        $(
            $(#[$doc])*
            #[unsafe(no_mangle)]
            #[inline(never)]
            pub extern "C" fn $name() {
                reached(stringify!($name));
            }
        )*
    };
}

stages! {
    /// `main` runs (the DLLs are loaded).
    pg_stage_main,
    /// The catalog and the thumbnail cache are open.
    pg_stage_init_done,
    /// The QML engine starts loading the window.
    pg_stage_qml_load,
    /// It has (the window is up, not yet painted).
    pg_stage_qml_loaded,
    /// The first frame is on screen.
    pg_stage_first_frame,
    /// A folder tree is shown.
    pg_stage_tree_shown,
    /// A grid list is shown.
    pg_stage_list_shown,
    /// The start is done.
    pg_stage_ready,
    /// The library's background job finished a round (on its thread; the first is the start's import check and
    /// thumbnail pass).
    pg_stage_round_done,
}

fn reached(stage: &str) {
    if let Some(sink) = SINK.get() {
        sink(stage, pg_platform::clock::process_age_ms());
    }
}

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
