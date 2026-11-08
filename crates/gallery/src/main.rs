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
        attach_console();
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
    attach_console();
    eprintln!("gallery was built without its app window (the `desktop` feature): give it a command");
    ExitCode::FAILURE
}

/// Nothing to attach elsewhere: a program's standard streams are its caller's from the start.
#[cfg(not(windows))]
fn attach_console() {}

/// Make stdout and stderr reach the console the command was typed in. A GUI-subsystem program starts without
/// them unless they were redirected (a pipe or a file, which then stay as they are).
#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };
    let valid = |h: HANDLE| !h.is_null() && h != INVALID_HANDLE_VALUE;
    // SAFETY: GetStdHandle and AttachConsole take no pointers.
    let missing: Vec<STD_HANDLE> =
        [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].into_iter().filter(|&s| !valid(unsafe { GetStdHandle(s) })).collect();
    // SAFETY: as above.
    if missing.is_empty() || unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
    // SAFETY: `name` is NUL-terminated and outlives the call; the null pointers are optional arguments.
    let out = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if valid(out) {
        for s in missing {
            // SAFETY: `out` is a valid console handle, kept open for the rest of the process.
            unsafe { SetStdHandle(s, out) };
        }
    }
}
