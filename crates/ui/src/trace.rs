//! Startup stages, for measuring them (docs/PROFILING.md "Startup"). Each stage is a function with a plain C name,
//! called when the app gets there:
//!
//! - Intel SDE counts the instructions between two of them, on every thread: `sde -icount -controller_default_start
//!   0 -control start:enter_func:pg_stage_main:bcast,stop:enter_func:pg_stage_ready:bcast -- gallery.exe …`.
//! - With `PG_TRACE=1`, each prints the milliseconds since the process was created on stderr (`stage 1070.4 ms
//!   pg_stage_first_frame`).
//!
//! Otherwise a stage costs a call. The tree and the list are marked each time one is shown; the first of each is the
//! start's.

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

use std::sync::LazyLock;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

#[derive(Default)]
pub struct StagesRust;

impl qobject::Stages {
    fn first_frame(&self) {
        pg_stage_first_frame();
    }

    fn ready(&self) {
        pg_stage_ready();
    }
}

static PRINT: LazyLock<bool> = LazyLock::new(|| std::env::var_os("PG_TRACE").is_some_and(|v| v == "1"));

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
}

fn reached(stage: &str) {
    if *PRINT {
        eprintln!("stage {:>9.1} ms  {stage}", since_process_start_ms());
    }
}

fn since_process_start_ms() -> f64 {
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user, mut now) = (zero, zero, zero, zero, zero);
    // SAFETY: both only write the FILETIMEs they are given; the pseudo-handle of this process needs no closing.
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user);
        GetSystemTimePreciseAsFileTime(&mut now);
    }
    // FILETIME counts 100 ns.
    ticks(now).saturating_sub(ticks(created)) as f64 / 1e4
}
