//! The startup stages as the QML sees them: Main.qml calls `Stages.firstFrame()` and `Stages.ready()`. The stages
//! themselves are `pg-trace`'s: this module is the bridge, and the app's printing of them
//! with `PG_TRACE=1` (`install`).

pub(crate) mod state;

use crate::trace_bridge::qobject;

use pg_trace::{pg_stage_first_frame, pg_stage_ready};

impl qobject::Stages {
    pub(crate) fn first_frame(&self) {
        pg_stage_first_frame();
    }

    pub(crate) fn ready(&self) {
        pg_stage_ready();
    }
}

/// With `PG_TRACE=1`, each stage prints its milliseconds since the process was created on stderr (`stage 1070.4 ms
/// pg_stage_first_frame`). Called before the first stage; without it a stage costs a call. Returns the perf log's failure
/// to open, if it failed: the window has no stderr, so the caller logs it once the log exists.
#[expect(clippy::print_stderr, reason = "PG_TRACE=1 asks for the timings on the console")]
pub(crate) fn install() -> Option<String> {
    if std::env::var_os("PG_TRACE").is_some_and(|v| v == "1") {
        pg_trace::set_sink(|stage, ms| eprintln!("stage {ms:>9.1} ms  {stage}"));
    }
    // The perf log, when `PG_PERF_LOG` names a file: the background job's rounds and their passes are appended to it as
    // they end (the app writes the file; `gallery perf` reads it). Its `-` is the command line's, not the app's.
    let file = std::env::var_os("PG_PERF_LOG").filter(|v| !v.is_empty() && v.to_str() != Some("-"))?;
    let meta = serde_json::json!({ "program": pg_perf::PROGRAM, "version": env!("CARGO_PKG_VERSION") });
    let path = std::path::Path::new(&file);
    pg_perf::open(path, meta).err().map(|e| format!("perf log: can't open {}: {e}; not recording", path.display()))
}
