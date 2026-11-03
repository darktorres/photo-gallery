//! Child processes. ExifTool, which writes ratings and orientations, runs without a console window and ends with this
//! process.

use std::process::{Child, Command};

/// Keep a child process from opening a console window (on Windows).
#[cfg(windows)]
pub fn hide_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// CREATE_NO_WINDOW.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Elsewhere there is no console window to keep from opening.
#[cfg(not(windows))]
pub fn hide_console_window(_cmd: &mut Command) {}

/// Put a child process in a job that kills it when this process ends, however it ends: the app quitting runs no
/// destructors, so a stay-open ExifTool would otherwise outlive it.
#[cfg(windows)]
pub fn die_with_parent(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    // The job handle is never closed: the system closes it when the process exits.
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: null security attributes and name are allowed; `info` is the struct this information class
        // takes, with its size.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of_val(&info) as u32,
            );
            job as usize
        }
    });
    // SAFETY: the job handle is never closed, and the child's handle is open while `child` lives.
    unsafe { AssignProcessToJobObject(job as _, child.as_raw_handle() as _) };
}

/// Elsewhere a child ends when this process closes its pipes to it, which the process does when it ends.
#[cfg(not(windows))]
pub fn die_with_parent(_child: &Child) {}
