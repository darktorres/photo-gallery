//! `gallery`: the photo gallery (PLAN §7). Without arguments it opens the app (crates/desktop), which imports the library
//! in the background; with a command (`gallery scan`, `gallery import`, … see `pg_ingest::cli`) it runs that pass on
//! the command line instead. Without the `desktop` feature only the commands run.

// A GUI program: no console window when started from Explorer. The command line attaches to its caller's console.
#![windows_subsystem = "windows"]

use std::ffi::OsString;

use pg_ingest::cli;

// The heap profiler's build (docs/PROFILING.md "Tool: dhat"). It sees Rust's allocations only: Qt's and FFmpeg's go
// to the C runtime's heap.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    // Dropping it writes dhat-heap.json, so it lives until `main` returns: runs that end in `process::exit` (an
    // error, `fatal`) write none. Backtraces are cut to 10 frames unless PG_DHAT_UNTRIMMED is set.
    #[cfg(feature = "dhat-heap")]
    let _profiler = {
        let frames = std::env::var_os("PG_DHAT_UNTRIMMED").is_none().then_some(10);
        dhat::Profiler::builder().trim_backtraces(frames).build()
    };
    let args: Vec<OsString> = std::env::args_os().collect();
    if cli::wants_cli(&args[1..]) {
        attach_console();
        if let Err(e) = cli::main(args) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(feature = "desktop")]
    pg_desktop::run(&args);
    #[cfg(not(feature = "desktop"))]
    {
        attach_console();
        eprintln!("gallery was built without its app window (the `desktop` feature): give it a command");
        std::process::exit(1);
    }
}

/// Make stdout and stderr reach the console the command was typed in. A GUI-subsystem program starts without
/// them unless they were redirected (a pipe or a file, which then stay as they are).
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
