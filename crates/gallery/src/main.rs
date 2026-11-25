//! `gallery`: the photo gallery. It opens the app's window (crates/desktop), which imports the library in the background. The
//! library's command line is gone; its developer uses are `pg-driver` (crates/driver), which runs the same job without a window.

#![forbid(unsafe_code)]
// A GUI program: no console window when started from Explorer.
#![windows_subsystem = "windows"]

use std::ffi::OsString;
use std::process::ExitCode;

// The heap profiler's build. It sees Rust's allocations only: Qt's and FFmpeg's go
// to the C runtime's heap.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() -> ExitCode {
    // Dropping it writes dhat-heap.json, so it lives until `main` returns, and every run returns from `main` (an error
    // is a status, not an exit). Backtraces are cut to 10 frames unless PG_DHAT_UNTRIMMED is set.
    #[cfg(feature = "dhat-heap")]
    let _profiler = {
        let frames = pg_platform::env::read(pg_platform::env::Var::DhatUntrimmed).is_none().then_some(10);
        dhat::Profiler::builder().trim_backtraces(frames).build()
    };
    let args: Vec<OsString> = std::env::args_os().collect();
    // The app's exit status is the process's (a `--screenshot` run that found no window fails); a status that doesn't fit
    // an exit code (below 0 or above 255) is a failure.
    u8::try_from(pg_desktop::run(&args)).map_or(ExitCode::FAILURE, ExitCode::from)
}
