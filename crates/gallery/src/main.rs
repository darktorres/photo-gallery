//! `gallery`: the photo gallery (PLAN §7). Without arguments it opens the app (crates/desktop), which imports the library
//! in the background; with a command (`gallery scan`, `gallery import`, … see `pg_cli`) it runs that pass on
//! the command line instead. Without the `desktop` feature only the commands run.

// A GUI program: no console window when started from Explorer. The command line attaches to its caller's console.
#![windows_subsystem = "windows"]
// The command line's error, and the notice of a build without the window, are printed to stderr.
#![expect(clippy::print_stderr, reason = "the command line's error and the notice of a build without a window")]

use std::ffi::OsString;
use std::process::ExitCode;

// The heap profiler's build (docs/PROFILING.md "Tool: dhat"). It sees Rust's allocations only: Qt's and FFmpeg's go
// to the C runtime's heap.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() -> ExitCode {
    // Dropping it writes dhat-heap.json, so it lives until `main` returns: runs that end in `process::exit` (an
    // error, `fatal`) write none. Backtraces are cut to 10 frames unless PG_DHAT_UNTRIMMED is set.
    #[cfg(feature = "dhat-heap")]
    let _profiler = {
        let frames = std::env::var_os("PG_DHAT_UNTRIMMED").is_none().then_some(10);
        dhat::Profiler::builder().trim_backtraces(frames).build()
    };
    let args: Vec<OsString> = std::env::args_os().collect();
    if pg_cli::wants_cli(args.get(1..).unwrap_or_default()) {
        pg_platform::console::attach_to_parent();
        if let Err(e) = pg_cli::main(args) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return ExitCode::SUCCESS;
    }
    open_window(&args)
}

/// Opens the app's window; returns when it closes. The app's exit status is the process's (a `--screenshot` run that
/// found no window fails); a status that doesn't fit an exit code (below 0 or above 255) is a failure.
#[cfg(feature = "desktop")]
fn open_window(args: &[OsString]) -> ExitCode {
    u8::try_from(pg_desktop::run(args)).map_or(ExitCode::FAILURE, ExitCode::from)
}

/// A build without the window: only the commands run.
#[cfg(not(feature = "desktop"))]
fn open_window(_args: &[OsString]) -> ExitCode {
    pg_platform::console::attach_to_parent();
    eprintln!("gallery was built without its app window (the `desktop` feature): give it a command");
    ExitCode::FAILURE
}
