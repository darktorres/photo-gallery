//! The startup stages as the QML sees them: Main.qml calls `Stages.firstFrame()` and `Stages.ready()`. The stages
//! themselves are `pg-trace`'s: this module is the bridge, and the app's printing of them
//! with `PG_TRACE=1` (`install`).

use crate::bridges::trace_bridge::qobject;

use pg_trace::{pg_stage_first_frame, pg_stage_ready};

impl qobject::Stages {
    pub(crate) fn first_frame(&self) {
        pg_stage_first_frame();
    }

    pub(crate) fn ready(&self) {
        pg_stage_ready();
    }
}

/// One stage's line on the console: the `PG_TRACE=1` sink of [`install`].
#[expect(clippy::print_stderr, reason = "PG_TRACE=1 asks for the timings on the console")]
fn print_stage(stage: &str, ms: f64) {
    eprintln!("stage {ms:>9.1} ms  {stage}");
}

/// With `PG_TRACE=1`, each stage prints its milliseconds since the process was created on stderr (`stage 1070.4 ms
/// pg_stage_first_frame`). Called before the first stage; without it a stage costs a call. Returns the perf log's failure
/// to open, if it failed: the window has no stderr, so the caller logs it once the log exists.
pub(crate) fn install() -> Option<String> {
    // A value that is neither on nor off turns the trace off, and is returned for the caller to log with the perf log's
    // failure: the window has no stderr to say it on.
    let mut problem = None;
    match pg_platform::env::flag(pg_platform::env::Var::Trace) {
        pg_platform::env::Parsed::On => {
            pg_trace::set_sink(print_stage);
        }
        pg_platform::env::Parsed::Invalid(text) => {
            problem = Some(format!("PG_TRACE: {text} is not on or off, so the stages are not traced"));
        }
        pg_platform::env::Parsed::Off | pg_platform::env::Parsed::Unset => {}
    }
    // The perf log, when `PG_PERF_LOG` names a file: the background job's rounds and their passes are appended to it as
    // they end (the app writes the file; `pg-driver perf` reads it).
    let perf = pg_platform::env::read(pg_platform::env::Var::PerfLog).and_then(|file| {
        let meta = serde_json::json!({ "program": pg_perf::PROGRAM, "version": env!("CARGO_PKG_VERSION") });
        let path = std::path::Path::new(&file);
        pg_perf::open(path, meta).err().map(|e| format!("perf log: can't open {}: {e}; not recording", path.display()))
    });
    problem.or(perf)
}
