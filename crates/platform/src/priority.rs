//! Thread priorities. The scan and import readers and workers run below normal, or idle, so the app keeps its processor time.

/// A thread's priority. Windows schedules by them; elsewhere a thread keeps its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Normal,
    BelowNormal,
    Idle,
}

/// Set the calling thread's priority.
#[cfg(windows)]
pub fn set_current_thread(priority: Priority) {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_IDLE, THREAD_PRIORITY_NORMAL,
    };
    let value = match priority {
        Priority::Normal => THREAD_PRIORITY_NORMAL,
        Priority::BelowNormal => THREAD_PRIORITY_BELOW_NORMAL,
        Priority::Idle => THREAD_PRIORITY_IDLE,
    };
    // SAFETY: GetCurrentThread's pseudo-handle is always valid for the calling thread.
    unsafe { SetThreadPriority(GetCurrentThread(), value) };
}

/// Elsewhere the priority is not changed.
#[cfg(not(windows))]
pub fn set_current_thread(_priority: Priority) {}
