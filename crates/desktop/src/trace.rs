//! The startup stages as the QML sees them: Main.qml calls `Stages.firstFrame()` and `Stages.ready()`. The stages
//! themselves are `pg-trace`'s (docs/PROFILING.md "Startup"): this module is the bridge, and the app's printing of them
//! with `PG_TRACE=1` (`install`).

mod state;
use state::StagesRust;

#[cxx_qt::bridge]
pub mod qobject {
    extern "RustQt" {
        /// The stages only the QML sees.
        #[qobject]
        #[qml_element]
        type Stages = super::StagesRust;
    }

    unsafe extern "RustQt" {
        /// The window showed its first frame.
        #[qinvokable]
        #[cxx_name = "firstFrame"]
        fn first_frame(self: &Stages);

        /// The start is done: the list and the tree are in (Main.qml `started`, before it selects the last file
        /// and runs the test hooks).
        #[qinvokable]
        fn ready(self: &Stages);
    }
}

use pg_trace::{pg_stage_first_frame, pg_stage_ready};

impl qobject::Stages {
    fn first_frame(&self) {
        pg_stage_first_frame();
    }

    fn ready(&self) {
        pg_stage_ready();
    }
}

/// With `PG_TRACE=1`, each stage prints its milliseconds since the process was created on stderr (`stage 1070.4 ms
/// pg_stage_first_frame`). Called before the first stage; without it a stage costs a call.
#[expect(clippy::print_stderr, reason = "PG_TRACE=1 asks for the timings on the console")]
pub fn install() {
    if std::env::var_os("PG_TRACE").is_some_and(|v| v == "1") {
        pg_trace::set_sink(|stage, ms| eprintln!("stage {ms:>9.1} ms  {stage}"));
    }
}
