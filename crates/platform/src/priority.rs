//! Thread priorities. The scan and import readers and workers run below normal, so the app keeps its processor time.

#![expect(unsafe_code, reason = "FFI: the Windows thread priority calls, and libc's setpriority and gettid on Unix")]

/// A thread's priority. Windows schedules by them; on Linux they are nice values, per thread (below); elsewhere a thread
/// keeps its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// The system's normal priority: work the user is waiting for.
    Normal,
    /// Below normal: the work yields to the interactive app. The one background class, for every pass.
    BelowNormal,
}

/// Set the calling thread's priority.
#[cfg(windows)]
pub fn set_current_thread(priority: Priority) {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_NORMAL,
    };
    let value = match priority {
        Priority::Normal => THREAD_PRIORITY_NORMAL,
        Priority::BelowNormal => THREAD_PRIORITY_BELOW_NORMAL,
    };
    // SAFETY: GetCurrentThread's pseudo-handle is always valid for the calling thread.
    unsafe { SetThreadPriority(GetCurrentThread(), value) };
}

/// On Linux a thread's nice value is its own (setpriority with a thread id names that thread). Below normal is nice 5.
/// Normal is left as it is, and a thread that can't be given a lower nice value (an unprivileged one can't) keeps its own:
/// the threads that are set here end with their run.
#[cfg(target_os = "linux")]
pub fn set_current_thread(priority: Priority) {
    let nice = match priority {
        Priority::Normal => return,
        Priority::BelowNormal => 5,
    };
    // SAFETY: setpriority takes a thread id and a nice value; a failure is a thread that keeps its priority.
    unsafe { libc::setpriority(libc::PRIO_PROCESS, current_thread_id(), nice) };
}

/// The calling thread's id, as setpriority and getpriority take it on Linux.
#[cfg(target_os = "linux")]
fn current_thread_id() -> libc::id_t {
    // SAFETY: gettid takes no arguments and cannot fail.
    let tid = unsafe { libc::syscall(libc::SYS_gettid) };
    // A thread id fits in 32 bits. 0 names the calling thread for setpriority too, so it is the fallback.
    libc::id_t::try_from(tid).unwrap_or(0)
}

/// Elsewhere the priority is not changed.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn set_current_thread(_priority: Priority) {}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// The calling thread's nice value.
    fn nice_of_current_thread() -> i32 {
        // SAFETY: getpriority takes a thread id and returns the nice value of that thread.
        unsafe { libc::getpriority(libc::PRIO_PROCESS, current_thread_id()) }
    }

    /// The nice value a new thread has after it sets `priority`. Each thread has its own, so each test gets one.
    fn nice_after(priority: Priority) -> Result<i32, Box<dyn std::error::Error>> {
        let nice = std::thread::spawn(move || {
            set_current_thread(priority);
            nice_of_current_thread()
        })
        .join()
        .map_err(|_| "a thread that set its priority panicked")?;
        Ok(nice)
    }

    #[test]
    fn below_normal_is_nice_5() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(nice_after(Priority::BelowNormal)?, 5);
        Ok(())
    }

    #[test]
    fn normal_leaves_the_thread_as_it_was() -> Result<(), Box<dyn std::error::Error>> {
        let before = std::thread::spawn(nice_of_current_thread).join().unwrap();
        assert_eq!(nice_after(Priority::Normal)?, before);
        Ok(())
    }
}
